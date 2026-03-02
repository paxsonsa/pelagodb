use pelago_api::schema_service::{proto_to_core_properties, proto_to_core_schema};
use pelago_core::PelagoError;
use pelago_core::ServerConfig;
use pelago_proto::cdc_operation_proto::Operation;
use pelago_proto::replication_service_client::ReplicationServiceClient;
use pelago_proto::{IndexDefaultMode, PullCdcEventsRequest, RequestContext};
use pelago_storage::{
    append_audit_record, append_cdc_entry, get_replication_positions_scoped, get_replicator_lease,
    try_acquire_replicator_lease, update_replication_position_scoped, AuditRecord, CdcEntry,
    CdcOperation, EdgeStore, IdAllocator, NodeRef, NodeStore, PelagoDb, SchemaRegistry,
    Versionstamp,
};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::watch;
use tonic::metadata::MetadataValue;
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint, Identity};
use tonic::Request;
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Debug, Clone)]
struct ReplicationPeer {
    remote_site_id: String,
    endpoint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ReplicationScope {
    database: String,
    namespace: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplicationTlsMode {
    Disabled,
    SystemRoots,
    CustomCa,
    MutualTls,
}

impl ReplicationTlsMode {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "disabled" | "off" | "none" => Ok(Self::Disabled),
            "system-roots" | "system_roots" | "system" => Ok(Self::SystemRoots),
            "custom-ca" | "custom_ca" => Ok(Self::CustomCa),
            "mutual-tls" | "mutual_tls" | "mtls" => Ok(Self::MutualTls),
            other => Err(format!(
                "invalid PELAGO_REPLICATION_TLS_MODE '{}'; expected one of: disabled, system-roots, custom-ca, mutual-tls",
                other
            )),
        }
    }

    fn tls_required(self) -> bool {
        self != Self::Disabled
    }
}

#[derive(Debug, Clone)]
struct ReplicationTlsConfig {
    mode: ReplicationTlsMode,
    ca_path: Option<String>,
    cert_path: Option<String>,
    key_path: Option<String>,
    server_name: Option<String>,
}

impl Default for ReplicationTlsConfig {
    fn default() -> Self {
        Self {
            mode: ReplicationTlsMode::Disabled,
            ca_path: None,
            cert_path: None,
            key_path: None,
            server_name: None,
        }
    }
}

impl ReplicationTlsConfig {
    fn from_server_config(config: &ServerConfig) -> Result<Self, String> {
        let mode = ReplicationTlsMode::parse(&config.replication_tls_mode)?;
        let ca_path = normalize_opt(config.replication_tls_ca.clone());
        let cert_path = normalize_opt(config.replication_tls_cert.clone());
        let key_path = normalize_opt(config.replication_tls_key.clone());
        let server_name = normalize_opt(config.replication_tls_server_name.clone());

        match mode {
            ReplicationTlsMode::Disabled | ReplicationTlsMode::SystemRoots => {}
            ReplicationTlsMode::CustomCa => {
                if ca_path.is_none() {
                    return Err(
                        "PELAGO_REPLICATION_TLS_CA is required when replication TLS mode is custom-ca"
                            .to_string(),
                    );
                }
            }
            ReplicationTlsMode::MutualTls => {
                if ca_path.is_none() || cert_path.is_none() || key_path.is_none() {
                    return Err(
                        "PELAGO_REPLICATION_TLS_CA, PELAGO_REPLICATION_TLS_CERT, and PELAGO_REPLICATION_TLS_KEY are required when replication TLS mode is mutual-tls"
                            .to_string(),
                    );
                }
            }
        }

        Ok(Self {
            mode,
            ca_path,
            cert_path,
            key_path,
            server_name,
        })
    }

    fn mode(&self) -> ReplicationTlsMode {
        self.mode
    }
}

#[derive(Debug, Clone)]
pub struct ReplicatorConfig {
    pub enabled: bool,
    pub batch_size: usize,
    pub poll_ms: u64,
    pub api_key: Option<String>,
    tls: ReplicationTlsConfig,
    pub lease_enabled: bool,
    pub lease_ttl_ms: u64,
    pub lease_heartbeat_ms: u64,
    peers: Vec<ReplicationPeer>,
    scopes: Vec<ReplicationScope>,
}

impl ReplicatorConfig {
    pub fn from_server_config(config: &ServerConfig) -> Self {
        let peers = parse_peers(&config.replication_peers);
        let scopes = parse_replication_scopes(config);
        let enabled = config.replication_enabled.unwrap_or(!peers.is_empty());
        let tls = match ReplicationTlsConfig::from_server_config(config) {
            Ok(cfg) => cfg,
            Err(err) => {
                warn!("{err}; falling back to replication TLS disabled");
                ReplicationTlsConfig::default()
            }
        };
        let lease_heartbeat_ms = config.replication_lease_heartbeat_ms.max(200);
        let lease_ttl_ms = config
            .replication_lease_ttl_ms
            .max(1_000)
            .max(lease_heartbeat_ms.saturating_mul(2));

        Self {
            enabled,
            batch_size: config.replication_batch_size.max(1),
            poll_ms: config.replication_poll_ms.max(50),
            api_key: config.replication_api_key.clone(),
            tls,
            lease_enabled: config.replication_lease_enabled,
            lease_ttl_ms,
            lease_heartbeat_ms,
            peers,
            scopes,
        }
    }

    fn peers(&self) -> &[ReplicationPeer] {
        &self.peers
    }

    fn scopes(&self) -> &[ReplicationScope] {
        &self.scopes
    }
}

pub fn start_replicators(
    db: PelagoDb,
    schema_registry: Arc<SchemaRegistry>,
    id_allocator: Arc<IdAllocator>,
    local_site_id: String,
    config: ReplicatorConfig,
    shutdown_rx: watch::Receiver<bool>,
) {
    if !config.enabled {
        info!("replicator disabled");
        return;
    }
    if config.peers.is_empty() {
        info!("replicator enabled but no peers configured");
        return;
    }

    let holder_id = build_replicator_holder_id();

    if config.scopes().is_empty() {
        warn!("replicator enabled but no valid replication scopes configured");
        return;
    }

    for scope in config.scopes().iter().cloned() {
        for peer in config.peers().iter().cloned() {
            let db = db.clone();
            let schema_registry = Arc::clone(&schema_registry);
            let id_allocator = Arc::clone(&id_allocator);
            let cfg = config.clone();
            let shutdown_rx = shutdown_rx.clone();
            let local_site_id = local_site_id.clone();
            let holder_id = holder_id.clone();
            let scope = scope.clone();

            tokio::spawn(async move {
                run_peer_replicator(
                    db,
                    schema_registry,
                    id_allocator,
                    local_site_id,
                    holder_id,
                    cfg,
                    scope,
                    peer,
                    shutdown_rx,
                )
                .await;
            });
        }
    }
}

async fn run_peer_replicator(
    db: PelagoDb,
    schema_registry: Arc<SchemaRegistry>,
    id_allocator: Arc<IdAllocator>,
    local_site_id: String,
    holder_id: String,
    config: ReplicatorConfig,
    scope: ReplicationScope,
    peer: ReplicationPeer,
    shutdown_rx: watch::Receiver<bool>,
) {
    let peer_endpoint = normalize_endpoint(&peer.endpoint, config.tls.mode().tls_required());
    info!(
        "starting replicator for remote site {} at {} for {}/{}",
        peer.remote_site_id, peer_endpoint, scope.database, scope.namespace
    );

    let mut after =
        load_checkpoint(&db, &scope.database, &scope.namespace, &peer.remote_site_id).await;
    let mut conflict_count: i64 = 0;
    let mut has_lease = !config.lease_enabled;
    let lease_refresh_interval = Duration::from_millis(config.lease_heartbeat_ms);
    let mut last_lease_refresh = Instant::now()
        .checked_sub(lease_refresh_interval)
        .unwrap_or_else(Instant::now);

    loop {
        if *shutdown_rx.borrow() {
            info!("replicator for {} shutting down", peer.remote_site_id);
            break;
        }
        if shutdown_rx.has_changed().unwrap_or(false) && *shutdown_rx.borrow() {
            info!("replicator for {} shutting down", peer.remote_site_id);
            break;
        }

        if config.lease_enabled && last_lease_refresh.elapsed() >= lease_refresh_interval {
            match try_acquire_replicator_lease(
                &db,
                &local_site_id,
                &scope.database,
                &scope.namespace,
                &holder_id,
                config.lease_ttl_ms,
            )
            .await
            {
                Ok(Some(lease)) => {
                    if !has_lease {
                        info!(
                            "replicator lease acquired for {}/{} (epoch {})",
                            scope.database, scope.namespace, lease.epoch
                        );
                    }
                    has_lease = true;
                }
                Ok(None) => {
                    if has_lease {
                        info!(
                            "replicator lease lost for {}/{}; waiting",
                            scope.database, scope.namespace
                        );
                    }
                    has_lease = false;
                }
                Err(err) => {
                    warn!(
                        "replicator lease check failed for {}/{}: {}",
                        scope.database, scope.namespace, err
                    );
                    has_lease = match get_replicator_lease(
                        &db,
                        &local_site_id,
                        &scope.database,
                        &scope.namespace,
                    )
                    .await
                    {
                        Ok(Some(lease)) => {
                            lease.holder_id == holder_id && lease.lease_expires_at > now_micros()
                        }
                        _ => false,
                    };
                }
            }
            last_lease_refresh = Instant::now();
        }

        if !has_lease {
            tokio::time::sleep(Duration::from_millis(config.poll_ms)).await;
            continue;
        }

        let mut client = match connect_replication_client(&config.tls, &peer_endpoint).await {
            Ok(client) => client,
            Err(err) => {
                warn!(
                    "replicator connect failed for {} ({}): {}",
                    peer.remote_site_id, peer_endpoint, err
                );
                tokio::time::sleep(std::time::Duration::from_millis(config.poll_ms)).await;
                continue;
            }
        };

        let mut req = Request::new(PullCdcEventsRequest {
            context: Some(RequestContext {
                database: scope.database.clone(),
                namespace: scope.namespace.clone(),
                site_id: local_site_id.clone(),
                request_id: Uuid::now_v7().to_string(),
            }),
            after_versionstamp: after
                .as_ref()
                .map(|vs| vs.to_bytes().to_vec())
                .unwrap_or_default(),
            limit: config.batch_size as u32,
            source_site: peer.remote_site_id.clone(),
        });
        if let Some(api_key) = &config.api_key {
            if let Ok(value) = MetadataValue::try_from(api_key.as_str()) {
                req.metadata_mut().insert("x-api-key", value);
            }
        }

        let response = match client.pull_cdc_events(req).await {
            Ok(response) => response,
            Err(err) => {
                warn!(
                    "replicator pull failed for {}: {}",
                    peer.remote_site_id, err
                );
                tokio::time::sleep(std::time::Duration::from_millis(config.poll_ms)).await;
                continue;
            }
        };

        let mut stream = response.into_inner();
        let mut seen_events = 0usize;

        loop {
            let next = match stream.message().await {
                Ok(next) => next,
                Err(err) => {
                    warn!(
                        "replicator stream error for {}: {}",
                        peer.remote_site_id, err
                    );
                    break;
                }
            };
            let Some(event) = next else {
                break;
            };

            let Some(vs) = Versionstamp::from_bytes(&event.versionstamp) else {
                conflict_count += 1;
                record_replication_conflict(
                    &db,
                    &peer.remote_site_id,
                    &scope.database,
                    &scope.namespace,
                    "invalid versionstamp in replication event",
                )
                .await;
                continue;
            };
            let Some(entry) = event.entry else {
                conflict_count += 1;
                record_replication_conflict(
                    &db,
                    &peer.remote_site_id,
                    &scope.database,
                    &scope.namespace,
                    "missing CDC entry payload",
                )
                .await;
                continue;
            };

            if entry.site != peer.remote_site_id {
                conflict_count += 1;
                record_replication_conflict(
                    &db,
                    &peer.remote_site_id,
                    &scope.database,
                    &scope.namespace,
                    &format!(
                        "source-site mismatch: expected {}, got {}",
                        peer.remote_site_id, entry.site
                    ),
                )
                .await;
                after = Some(vs);
                continue;
            }

            let mut mirrored_ops = Vec::new();
            let entry_timestamp = entry.timestamp;
            let entry_batch_id = if entry.batch_id.is_empty() {
                None
            } else {
                Some(entry.batch_id.clone())
            };

            let node_store = Arc::new(NodeStore::new(
                db.clone(),
                Arc::clone(&schema_registry),
                Arc::clone(&id_allocator),
                peer.remote_site_id.clone(),
            ));
            let edge_store = EdgeStore::new(
                db.clone(),
                Arc::clone(&schema_registry),
                Arc::clone(&id_allocator),
                Arc::clone(&node_store),
                peer.remote_site_id.clone(),
            );

            for op in entry.operations {
                let applied = apply_replication_op(
                    &node_store,
                    &edge_store,
                    &schema_registry,
                    &peer.remote_site_id,
                    &scope.database,
                    &scope.namespace,
                    entry.timestamp,
                    op.operation,
                )
                .await;
                match applied {
                    Ok(Some(op)) => {
                        mirrored_ops.push(op);
                    }
                    Ok(None) => {
                        conflict_count += 1;
                        record_replication_conflict(
                            &db,
                            &peer.remote_site_id,
                            &scope.database,
                            &scope.namespace,
                            "operation skipped by ownership/LWW filter",
                        )
                        .await;
                    }
                    Err(err) => {
                        conflict_count += 1;
                        record_replication_conflict(
                            &db,
                            &peer.remote_site_id,
                            &scope.database,
                            &scope.namespace,
                            &format!("operation apply failed: {}", err),
                        )
                        .await;
                    }
                }
            }

            if !mirrored_ops.is_empty() {
                let mirror_entry = CdcEntry {
                    site: peer.remote_site_id.clone(),
                    timestamp: entry_timestamp,
                    batch_id: entry_batch_id,
                    operations: mirrored_ops,
                };
                if let Err(err) =
                    append_cdc_entry(&db, &scope.database, &scope.namespace, mirror_entry).await
                {
                    conflict_count += 1;
                    record_replication_conflict(
                        &db,
                        &peer.remote_site_id,
                        &scope.database,
                        &scope.namespace,
                        &format!("failed to append mirrored CDC entry: {}", err),
                    )
                    .await;
                }
            }

            after = Some(vs);
            seen_events += 1;
        }

        if let Err(err) = update_replication_position_scoped(
            &db,
            &scope.database,
            &scope.namespace,
            &peer.remote_site_id,
            after.clone(),
            conflict_count,
        )
        .await
        {
            warn!(
                "failed to persist replication position for {}: {}",
                peer.remote_site_id, err
            );
        }

        if seen_events == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(config.poll_ms)).await;
        }
    }
}

async fn apply_replication_op(
    node_store: &Arc<NodeStore>,
    edge_store: &EdgeStore,
    schema_registry: &Arc<SchemaRegistry>,
    remote_site_id: &str,
    database: &str,
    namespace: &str,
    timestamp: i64,
    operation: Option<Operation>,
) -> Result<Option<CdcOperation>, pelago_core::PelagoError> {
    let Some(op) = operation else {
        return Ok(None);
    };

    match op {
        Operation::NodeCreate(op) => {
            let props = proto_to_core_properties(&op.properties);
            let applied = node_store
                .apply_replica_node_create(
                    database,
                    namespace,
                    &op.entity_type,
                    &op.node_id,
                    props.clone(),
                    &op.home_site,
                    timestamp,
                    remote_site_id,
                )
                .await?;
            if applied {
                Ok(Some(CdcOperation::NodeCreate {
                    entity_type: op.entity_type,
                    node_id: op.node_id,
                    properties: props,
                    home_site: op.home_site,
                }))
            } else {
                Ok(None)
            }
        }
        Operation::NodeUpdate(op) => {
            let changed = proto_to_core_properties(&op.changed_properties);
            let old = proto_to_core_properties(&op.old_properties);
            let applied = node_store
                .apply_replica_node_update(
                    database,
                    namespace,
                    &op.entity_type,
                    &op.node_id,
                    changed.clone(),
                    old.clone(),
                    timestamp,
                    remote_site_id,
                )
                .await?;
            if applied {
                Ok(Some(CdcOperation::NodeUpdate {
                    entity_type: op.entity_type,
                    node_id: op.node_id,
                    changed_properties: changed,
                    old_properties: old,
                }))
            } else {
                Ok(None)
            }
        }
        Operation::NodeDelete(op) => {
            let applied = node_store
                .apply_replica_node_delete(
                    database,
                    namespace,
                    &op.entity_type,
                    &op.node_id,
                    timestamp,
                    remote_site_id,
                )
                .await?;
            if applied {
                Ok(Some(CdcOperation::NodeDelete {
                    entity_type: op.entity_type,
                    node_id: op.node_id,
                }))
            } else {
                Ok(None)
            }
        }
        Operation::EdgeCreate(op) => {
            let props = proto_to_core_properties(&op.properties);
            let applied = edge_store
                .apply_replica_edge_create(
                    database,
                    namespace,
                    NodeRef::new(database, namespace, &op.source_type, &op.source_id),
                    NodeRef::new(database, namespace, &op.target_type, &op.target_id),
                    &op.edge_type,
                    &op.edge_id,
                    props.clone(),
                    timestamp,
                    remote_site_id,
                )
                .await?;
            if applied {
                Ok(Some(CdcOperation::EdgeCreate {
                    source_type: op.source_type,
                    source_id: op.source_id,
                    target_type: op.target_type,
                    target_id: op.target_id,
                    edge_type: op.edge_type,
                    edge_id: op.edge_id,
                    properties: props,
                }))
            } else {
                Ok(None)
            }
        }
        Operation::EdgeDelete(op) => {
            let applied = edge_store
                .apply_replica_edge_delete(
                    database,
                    namespace,
                    NodeRef::new(database, namespace, &op.source_type, &op.source_id),
                    NodeRef::new(database, namespace, &op.target_type, &op.target_id),
                    &op.edge_type,
                    timestamp,
                    remote_site_id,
                )
                .await?;
            if applied {
                Ok(Some(CdcOperation::EdgeDelete {
                    source_type: op.source_type,
                    source_id: op.source_id,
                    target_type: op.target_type,
                    target_id: op.target_id,
                    edge_type: op.edge_type,
                }))
            } else {
                Ok(None)
            }
        }
        Operation::SchemaRegister(op) => {
            let proto_schema = op.schema.ok_or_else(|| PelagoError::SchemaValidation {
                message: "replicated schema register missing schema payload".to_string(),
            })?;
            let schema = proto_to_core_schema(&proto_schema, IndexDefaultMode::AutoByTypeV1)
                .map_err(|e| PelagoError::SchemaValidation {
                    message: format!("invalid replicated schema payload: {}", e.message()),
                })?;
            if schema.name != op.entity_type {
                return Err(PelagoError::SchemaValidation {
                    message: format!(
                        "schema payload name '{}' does not match op entity_type '{}'",
                        schema.name, op.entity_type
                    ),
                });
            }

            let applied = schema_registry
                .apply_replica_schema_register(
                    database,
                    namespace,
                    schema.clone(),
                    op.version,
                    remote_site_id,
                )
                .await?;
            if applied {
                Ok(Some(CdcOperation::SchemaRegister {
                    entity_type: op.entity_type,
                    version: op.version,
                    schema,
                }))
            } else {
                Ok(None)
            }
        }
        Operation::OwnershipTransfer(op) => {
            let applied = node_store
                .apply_replica_ownership_transfer(
                    database,
                    namespace,
                    &op.entity_type,
                    &op.node_id,
                    &op.previous_site_id,
                    &op.current_site_id,
                    timestamp,
                    remote_site_id,
                )
                .await?;
            if applied {
                Ok(Some(CdcOperation::OwnershipTransfer {
                    entity_type: op.entity_type,
                    node_id: op.node_id,
                    previous_site_id: op.previous_site_id,
                    current_site_id: op.current_site_id,
                }))
            } else {
                Ok(None)
            }
        }
    }
}

async fn load_checkpoint(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
    remote_site_id: &str,
) -> Option<Versionstamp> {
    let positions = get_replication_positions_scoped(db, database, namespace)
        .await
        .ok()?;
    positions
        .into_iter()
        .find(|p| p.remote_site_id == remote_site_id)
        .and_then(|p| p.last_applied_versionstamp)
}

async fn record_replication_conflict(
    db: &PelagoDb,
    remote_site_id: &str,
    database: &str,
    namespace: &str,
    reason: &str,
) {
    let _ = append_audit_record(
        db,
        AuditRecord {
            event_id: String::new(),
            timestamp: 0,
            principal_id: format!("replicator:{}", remote_site_id),
            action: "replication.conflict".to_string(),
            resource: format!("{}/{}", database, namespace),
            allowed: false,
            reason: reason.to_string(),
            metadata: std::collections::HashMap::from([(
                "remote_site_id".to_string(),
                remote_site_id.to_string(),
            )]),
        },
    )
    .await;
}

fn parse_peers(raw: &str) -> Vec<ReplicationPeer> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|entry| {
            let (site, endpoint) = entry.split_once('=')?;
            let site = site.trim();
            let endpoint = endpoint.trim();
            if site.is_empty() || endpoint.is_empty() {
                return None;
            }
            Some(ReplicationPeer {
                remote_site_id: site.to_string(),
                endpoint: endpoint.to_string(),
            })
        })
        .collect()
}

fn parse_scopes(raw: &str) -> Vec<ReplicationScope> {
    let mut scopes = Vec::new();
    let mut seen = HashSet::new();

    for entry in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let Some((database, namespace)) = entry.split_once('/') else {
            continue;
        };
        let database = database.trim();
        let namespace = namespace.trim();
        if database.is_empty() || namespace.is_empty() {
            continue;
        }

        if seen.insert((database.to_string(), namespace.to_string())) {
            scopes.push(ReplicationScope {
                database: database.to_string(),
                namespace: namespace.to_string(),
            });
        }
    }

    scopes
}

fn parse_replication_scopes(config: &ServerConfig) -> Vec<ReplicationScope> {
    if let Some(raw) = config.replication_scopes.as_deref() {
        let scopes = parse_scopes(raw);
        if !scopes.is_empty() {
            return scopes;
        }
        warn!(
            "Ignoring empty/invalid PELAGO_REPLICATION_SCOPES='{}'; falling back to single-scope config",
            raw
        );
    }

    vec![ReplicationScope {
        database: config
            .replication_database
            .clone()
            .unwrap_or_else(|| config.default_database.clone()),
        namespace: config
            .replication_namespace
            .clone()
            .unwrap_or_else(|| config.default_namespace.clone()),
    }]
}

fn normalize_endpoint(endpoint: &str, tls_required: bool) -> String {
    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        endpoint.to_string()
    } else if tls_required {
        format!("https://{}", endpoint)
    } else {
        format!("http://{}", endpoint)
    }
}

fn normalize_opt(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

async fn connect_replication_client(
    tls: &ReplicationTlsConfig,
    endpoint: &str,
) -> Result<ReplicationServiceClient<tonic::transport::Channel>, String> {
    if endpoint.starts_with("https://") && !tls.mode.tls_required() {
        return Err(
            "HTTPS replication endpoint requires replication TLS mode != disabled".to_string(),
        );
    }
    if endpoint.starts_with("http://") && tls.mode.tls_required() {
        return Err("replication TLS mode requires HTTPS endpoint".to_string());
    }

    let mut transport = Endpoint::from_shared(endpoint.to_string())
        .map_err(|e| format!("invalid replication endpoint '{}': {}", endpoint, e))?;

    if tls.mode.tls_required() {
        let mut tls_config = ClientTlsConfig::new();
        if let Some(server_name) = tls.server_name.as_deref() {
            tls_config = tls_config.domain_name(server_name.to_string());
        }

        match tls.mode {
            ReplicationTlsMode::Disabled => {}
            ReplicationTlsMode::SystemRoots => {}
            ReplicationTlsMode::CustomCa | ReplicationTlsMode::MutualTls => {
                let ca_path = tls.ca_path.as_deref().ok_or_else(|| {
                    "replication TLS mode requires PELAGO_REPLICATION_TLS_CA".to_string()
                })?;
                let ca_pem = std::fs::read(ca_path).map_err(|e| {
                    format!("failed to read replication TLS CA '{}': {}", ca_path, e)
                })?;
                tls_config = tls_config.ca_certificate(Certificate::from_pem(ca_pem));
            }
        }

        if tls.mode == ReplicationTlsMode::MutualTls {
            let cert_path = tls.cert_path.as_deref().ok_or_else(|| {
                "replication mutual TLS requires PELAGO_REPLICATION_TLS_CERT".to_string()
            })?;
            let key_path = tls.key_path.as_deref().ok_or_else(|| {
                "replication mutual TLS requires PELAGO_REPLICATION_TLS_KEY".to_string()
            })?;
            let cert_pem = std::fs::read(cert_path).map_err(|e| {
                format!("failed to read replication TLS cert '{}': {}", cert_path, e)
            })?;
            let key_pem = std::fs::read(key_path)
                .map_err(|e| format!("failed to read replication TLS key '{}': {}", key_path, e))?;
            tls_config = tls_config.identity(Identity::from_pem(cert_pem, key_pem));
        }

        transport = transport
            .tls_config(tls_config)
            .map_err(|e| format!("invalid replication TLS configuration: {}", e))?;
    }

    let channel = transport
        .connect()
        .await
        .map_err(|e| format!("connection error: {}", e))?;
    Ok(ReplicationServiceClient::new(channel))
}

fn build_replicator_holder_id() -> String {
    let host = std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::env::var("HOST").ok())
        .unwrap_or_else(|| "localhost".to_string());
    format!("{}:{}:{}", host, std::process::id(), Uuid::now_v7())
}

fn now_micros() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_peers() {
        let peers = parse_peers("2=127.0.0.1:50052,3=http://example.com:50053");
        assert_eq!(peers.len(), 2);
        assert_eq!(peers[0].remote_site_id, "2");
        assert_eq!(peers[0].endpoint, "127.0.0.1:50052");
    }

    #[test]
    fn test_normalize_endpoint() {
        assert_eq!(
            normalize_endpoint("127.0.0.1:5001", false),
            "http://127.0.0.1:5001"
        );
        assert_eq!(
            normalize_endpoint("http://127.0.0.1:5001", false),
            "http://127.0.0.1:5001"
        );
        assert_eq!(
            normalize_endpoint("127.0.0.1:5001", true),
            "https://127.0.0.1:5001"
        );
    }

    #[test]
    fn test_parse_scopes() {
        let scopes = parse_scopes("db1/ns1, db2/ns2, db1/ns1");
        assert_eq!(scopes.len(), 2);
        assert_eq!(scopes[0].database, "db1");
        assert_eq!(scopes[0].namespace, "ns1");
        assert_eq!(scopes[1].database, "db2");
        assert_eq!(scopes[1].namespace, "ns2");
    }

    #[test]
    fn test_parse_replication_scopes_override() {
        let mut config = ServerConfig::default();
        config.replication_scopes = Some("a/x,b/y".to_string());

        let scopes = parse_replication_scopes(&config);
        assert_eq!(scopes.len(), 2);
        assert_eq!(scopes[0].database, "a");
        assert_eq!(scopes[0].namespace, "x");
        assert_eq!(scopes[1].database, "b");
        assert_eq!(scopes[1].namespace, "y");
    }

    #[test]
    fn test_parse_replication_scopes_fallback_single_scope() {
        let mut config = ServerConfig::default();
        config.replication_database = Some("tenant".to_string());
        config.replication_namespace = Some("graph".to_string());

        let scopes = parse_replication_scopes(&config);
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].database, "tenant");
        assert_eq!(scopes[0].namespace, "graph");
    }

    #[test]
    fn test_parse_replication_tls_mode() {
        assert_eq!(
            ReplicationTlsMode::parse("disabled").expect("disabled"),
            ReplicationTlsMode::Disabled
        );
        assert_eq!(
            ReplicationTlsMode::parse("system-roots").expect("system-roots"),
            ReplicationTlsMode::SystemRoots
        );
        assert_eq!(
            ReplicationTlsMode::parse("custom-ca").expect("custom-ca"),
            ReplicationTlsMode::CustomCa
        );
        assert_eq!(
            ReplicationTlsMode::parse("mutual-tls").expect("mutual-tls"),
            ReplicationTlsMode::MutualTls
        );
        assert!(ReplicationTlsMode::parse("bogus").is_err());
    }

    #[test]
    fn test_replication_tls_config_validation() {
        let mut cfg = ServerConfig::default();
        cfg.replication_tls_mode = "custom-ca".to_string();
        assert!(ReplicationTlsConfig::from_server_config(&cfg).is_err());

        cfg.replication_tls_ca = Some("/tmp/ca.pem".to_string());
        assert!(ReplicationTlsConfig::from_server_config(&cfg).is_ok());

        cfg.replication_tls_mode = "mutual-tls".to_string();
        assert!(ReplicationTlsConfig::from_server_config(&cfg).is_err());
        cfg.replication_tls_cert = Some("/tmp/cert.pem".to_string());
        cfg.replication_tls_key = Some("/tmp/key.pem".to_string());
        assert!(ReplicationTlsConfig::from_server_config(&cfg).is_ok());
    }
}
