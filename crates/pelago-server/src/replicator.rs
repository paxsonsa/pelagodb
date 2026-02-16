use pelago_api::schema_service::proto_to_core_properties;
use pelago_core::ServerConfig;
use pelago_proto::cdc_operation_proto::Operation;
use pelago_proto::replication_service_client::ReplicationServiceClient;
use pelago_proto::{PullCdcEventsRequest, RequestContext};
use pelago_storage::{
    append_audit_record, get_replication_positions, update_replication_position, AuditRecord,
    EdgeStore, IdAllocator, NodeRef, NodeStore, PelagoDb, SchemaRegistry, Versionstamp,
};
use std::sync::Arc;
use tokio::sync::watch;
use tonic::metadata::MetadataValue;
use tonic::Request;
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Debug, Clone)]
struct ReplicationPeer {
    remote_site_id: String,
    endpoint: String,
}

#[derive(Debug, Clone)]
pub struct ReplicatorConfig {
    pub enabled: bool,
    pub database: String,
    pub namespace: String,
    pub batch_size: usize,
    pub poll_ms: u64,
    pub api_key: Option<String>,
    peers: Vec<ReplicationPeer>,
}

impl ReplicatorConfig {
    pub fn from_server_config(config: &ServerConfig) -> Self {
        let peers = parse_peers(&config.replication_peers);
        let enabled = config.replication_enabled.unwrap_or(!peers.is_empty());

        Self {
            enabled,
            database: config
                .replication_database
                .clone()
                .unwrap_or_else(|| config.default_database.clone()),
            namespace: config
                .replication_namespace
                .clone()
                .unwrap_or_else(|| config.default_namespace.clone()),
            batch_size: config.replication_batch_size.max(1),
            poll_ms: config.replication_poll_ms.max(50),
            api_key: config.replication_api_key.clone(),
            peers,
        }
    }

    fn peers(&self) -> &[ReplicationPeer] {
        &self.peers
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

    for peer in config.peers().iter().cloned() {
        let db = db.clone();
        let schema_registry = Arc::clone(&schema_registry);
        let id_allocator = Arc::clone(&id_allocator);
        let cfg = config.clone();
        let shutdown_rx = shutdown_rx.clone();
        let local_site_id = local_site_id.clone();

        tokio::spawn(async move {
            run_peer_replicator(
                db,
                schema_registry,
                id_allocator,
                local_site_id,
                cfg,
                peer,
                shutdown_rx,
            )
            .await;
        });
    }
}

async fn run_peer_replicator(
    db: PelagoDb,
    schema_registry: Arc<SchemaRegistry>,
    id_allocator: Arc<IdAllocator>,
    local_site_id: String,
    config: ReplicatorConfig,
    peer: ReplicationPeer,
    shutdown_rx: watch::Receiver<bool>,
) {
    let peer_endpoint = normalize_endpoint(&peer.endpoint);
    info!(
        "starting replicator for remote site {} at {}",
        peer.remote_site_id, peer_endpoint
    );

    let mut after = load_checkpoint(&db, &peer.remote_site_id).await;
    let mut conflict_count: i64 = 0;

    loop {
        if *shutdown_rx.borrow() {
            info!("replicator for {} shutting down", peer.remote_site_id);
            break;
        }
        if shutdown_rx.has_changed().unwrap_or(false) && *shutdown_rx.borrow() {
            info!("replicator for {} shutting down", peer.remote_site_id);
            break;
        }

        let mut client = match ReplicationServiceClient::connect(peer_endpoint.clone()).await {
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
                database: config.database.clone(),
                namespace: config.namespace.clone(),
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
                    &config.database,
                    &config.namespace,
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
                    &config.database,
                    &config.namespace,
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
                    &config.database,
                    &config.namespace,
                    &format!(
                        "source-site mismatch: expected {}, got {}",
                        peer.remote_site_id, entry.site
                    ),
                )
                .await;
                after = Some(vs);
                continue;
            }

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
                    &peer.remote_site_id,
                    &config.database,
                    &config.namespace,
                    entry.timestamp,
                    op.operation,
                )
                .await;
                match applied {
                    Ok(true) => {}
                    Ok(false) => {
                        conflict_count += 1;
                        record_replication_conflict(
                            &db,
                            &peer.remote_site_id,
                            &config.database,
                            &config.namespace,
                            "operation skipped by ownership/LWW filter",
                        )
                        .await;
                    }
                    Err(err) => {
                        conflict_count += 1;
                        record_replication_conflict(
                            &db,
                            &peer.remote_site_id,
                            &config.database,
                            &config.namespace,
                            &format!("operation apply failed: {}", err),
                        )
                        .await;
                    }
                }
            }

            after = Some(vs);
            seen_events += 1;
        }

        if let Err(err) =
            update_replication_position(&db, &peer.remote_site_id, after.clone(), conflict_count)
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
    remote_site_id: &str,
    database: &str,
    namespace: &str,
    timestamp: i64,
    operation: Option<Operation>,
) -> Result<bool, pelago_core::PelagoError> {
    let Some(op) = operation else {
        return Ok(false);
    };

    match op {
        Operation::NodeCreate(op) => {
            let props = proto_to_core_properties(&op.properties);
            node_store
                .apply_replica_node_create(
                    database,
                    namespace,
                    &op.entity_type,
                    &op.node_id,
                    props,
                    &op.home_site,
                    timestamp,
                    remote_site_id,
                )
                .await
        }
        Operation::NodeUpdate(op) => {
            let changed = proto_to_core_properties(&op.changed_properties);
            let old = proto_to_core_properties(&op.old_properties);
            node_store
                .apply_replica_node_update(
                    database,
                    namespace,
                    &op.entity_type,
                    &op.node_id,
                    changed,
                    old,
                    timestamp,
                    remote_site_id,
                )
                .await
        }
        Operation::NodeDelete(op) => {
            node_store
                .apply_replica_node_delete(
                    database,
                    namespace,
                    &op.entity_type,
                    &op.node_id,
                    timestamp,
                    remote_site_id,
                )
                .await
        }
        Operation::EdgeCreate(op) => {
            edge_store
                .apply_replica_edge_create(
                    database,
                    namespace,
                    NodeRef::new(database, namespace, &op.source_type, &op.source_id),
                    NodeRef::new(database, namespace, &op.target_type, &op.target_id),
                    &op.edge_type,
                    &op.edge_id,
                    proto_to_core_properties(&op.properties),
                    timestamp,
                    remote_site_id,
                )
                .await
        }
        Operation::EdgeDelete(op) => {
            edge_store
                .apply_replica_edge_delete(
                    database,
                    namespace,
                    NodeRef::new(database, namespace, &op.source_type, &op.source_id),
                    NodeRef::new(database, namespace, &op.target_type, &op.target_id),
                    &op.edge_type,
                    timestamp,
                    remote_site_id,
                )
                .await
        }
        Operation::SchemaRegister(_op) => {
            // Schema body is not included in CDC operations yet.
            Ok(false)
        }
        Operation::OwnershipTransfer(op) => {
            node_store
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
                .await
        }
    }
}

async fn load_checkpoint(db: &PelagoDb, remote_site_id: &str) -> Option<Versionstamp> {
    let positions = get_replication_positions(db).await.ok()?;
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

fn normalize_endpoint(endpoint: &str) -> String {
    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        endpoint.to_string()
    } else {
        format!("http://{}", endpoint)
    }
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
            normalize_endpoint("127.0.0.1:5001"),
            "http://127.0.0.1:5001"
        );
        assert_eq!(
            normalize_endpoint("http://127.0.0.1:5001"),
            "http://127.0.0.1:5001"
        );
    }
}
