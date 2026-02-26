//! PelagoDB Server
//!
//! Main entry point for the PelagoDB gRPC server.
//!
//! Initializes the FDB connection, sets up shared components (schema cache, ID allocator),
//! and starts the gRPC server with all service handlers.

mod auth_identity;
mod docs_server;
mod replicator;
mod ui_server;

use anyhow::{Context, Result};
use clap::Parser;
use metrics_exporter_prometheus::PrometheusBuilder;
use pelago_api::{
    AdminServiceImpl, AuthRuntime, AuthServiceImpl, EdgeServiceImpl, HealthServiceImpl,
    NodeServiceImpl, QueryServiceImpl, ReplicationServiceImpl, SchemaServiceImpl, WatchRegistry,
    WatchServiceImpl,
};
use pelago_core::ServerConfig;
use pelago_proto::{
    admin_service_server::AdminServiceServer, auth_service_server::AuthServiceServer,
    edge_service_server::EdgeServiceServer, health_service_server::HealthServiceServer,
    node_service_server::NodeServiceServer, query_service_server::QueryServiceServer,
    replication_service_server::ReplicationServiceServer,
    schema_service_server::SchemaServiceServer, watch_service_server::WatchServiceServer,
};
use pelago_storage::{
    claim_site, cleanup_audit_records, cleanup_query_watch_states, CacheFreshnessBudgets,
    CachedReadPath, CdcProjector, IdAllocator, JobWorker, NodeStore, PelagoDb, RocksCacheConfig,
    RocksCacheStore, SchemaCache, SchemaRegistry,
};
use replicator::{start_replicators, ReplicatorConfig};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};
use tonic::{Request, Status};
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use ui_server::{UiServerConfig, UiServerDeps};

use crate::auth_identity::resolve_principal_from_tonic_request;

#[tokio::main]
async fn main() -> Result<()> {
    load_server_config_file_into_env()?;

    // Parse configuration
    let config = ServerConfig::parse();
    apply_cli_env_overrides(&config);

    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| config.log_level.clone().into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
    install_metrics_recorder(&config)?;

    info!("PelagoDB server starting");
    info!("Site ID: {}", config.site_id);
    info!("Listen address: {}", config.listen_addr);

    let fdb_cluster = resolve_fdb_cluster_file(&config.fdb_cluster);
    if fdb_cluster != config.fdb_cluster {
        warn!(
            "Configured FDB cluster file '{}' not found; using '{}' instead",
            config.fdb_cluster, fdb_cluster
        );
    }
    // Keep FDB bindings and subprocesses aligned on the same cluster file.
    std::env::set_var("FDB_CLUSTER_FILE", &fdb_cluster);

    // Connect to FDB cluster (this also initializes the FDB network)
    info!("Connecting to FDB cluster: {}", fdb_cluster);
    let db = PelagoDb::connect(&fdb_cluster).await?;
    // Force an early connectivity check so startup fails fast with a clear message.
    if let Err(e) = db.get(b"pelago.startup.probe").await {
        return Err(anyhow::anyhow!(
            "FoundationDB connectivity check failed using '{}': {}. \
             Set PELAGO_FDB_CLUSTER (or FDB_CLUSTER_FILE), or run ./scripts/start-fdb.sh",
            fdb_cluster,
            e
        ));
    }
    claim_site(&db, &config.site_id.to_string(), &config.site_name).await?;

    // Initialize shared components
    let site_id_str = config.site_id.to_string();
    let schema_cache = Arc::new(SchemaCache::new());
    let schema_registry = Arc::new(SchemaRegistry::new(
        db.clone(),
        Arc::clone(&schema_cache),
        site_id_str.clone(),
    ));
    let id_allocator = Arc::new(IdAllocator::new(
        db.clone(),
        config.site_id,
        config.id_batch_size,
    ));
    let node_store = Arc::new(NodeStore::new(
        db.clone(),
        Arc::clone(&schema_registry),
        Arc::clone(&id_allocator),
        site_id_str.clone(),
    ));

    // Cache layer configuration
    let mut cache_cfg = RocksCacheConfig::default();
    cache_cfg.enabled = config.cache_enabled;
    cache_cfg.path = config.cache_path.clone();
    cache_cfg.cache_size_mb = config.cache_size_mb;
    cache_cfg.write_buffer_mb = config.cache_write_buffer_mb;
    cache_cfg.max_write_buffers = config.cache_max_write_buffers;
    cache_cfg.bloom_bits_per_key = config.cache_bloom_bits_per_key;
    cache_cfg.prefix_extractor_bytes = config.cache_prefix_extractor_bytes;
    cache_cfg.use_column_families = config.cache_use_column_families;
    cache_cfg.projector_batch_size = config.cache_projector_batch_size;
    cache_cfg.eventual_max_lag_ms = config.cache_eventual_max_lag_ms;
    cache_cfg.session_max_lag_ms = config.cache_session_max_lag_ms;
    cache_cfg.site_id = config.site_id.to_string();

    let mut cache_store_for_projector: Option<Arc<RocksCacheStore>> = None;
    let cached_read_path: Option<Arc<CachedReadPath>> = if cache_cfg.enabled {
        info!("RocksDB cache enabled at {}", cache_cfg.path);
        let cache_store = Arc::new(RocksCacheStore::open(&cache_cfg)?);
        let freshness_budgets = CacheFreshnessBudgets {
            eventual_max_lag_ms: cache_cfg.eventual_max_lag_ms,
            session_max_lag_ms: cache_cfg.session_max_lag_ms,
        };
        cache_store_for_projector = Some(cache_store.clone());
        Some(Arc::new(CachedReadPath::new(
            db.clone(),
            cache_store,
            freshness_budgets,
        )))
    } else {
        info!("RocksDB cache disabled (--cache-enabled=false)");
        None
    };

    // Create service implementations
    let schema_service = SchemaServiceImpl::new(db.clone(), Arc::clone(&schema_registry));
    let node_service = NodeServiceImpl::new(
        db.clone(),
        Arc::clone(&schema_registry),
        Arc::clone(&id_allocator),
        site_id_str.clone(),
        cached_read_path.clone(),
    );
    let edge_service = EdgeServiceImpl::new(
        db.clone(),
        Arc::clone(&schema_registry),
        Arc::clone(&id_allocator),
        Arc::clone(&node_store),
        site_id_str.clone(),
        cached_read_path.clone(),
    );
    let query_service = QueryServiceImpl::new(
        db.clone(),
        Arc::clone(&schema_registry),
        Arc::clone(&id_allocator),
        site_id_str,
    );
    let replication_service = ReplicationServiceImpl::new(db.clone());
    let admin_service = AdminServiceImpl::new(Arc::new(db.clone()));
    let health_service = HealthServiceImpl::new();
    let watch_registry = WatchRegistry::default();
    let watch_service = WatchServiceImpl::new(
        db.clone(),
        Arc::clone(&schema_registry),
        watch_registry.clone(),
    );
    let auth_runtime = AuthRuntime::with_db(db.clone());
    let auth_service = AuthServiceImpl::new(db.clone(), auth_runtime.clone());
    let auth_required = config.auth_required;
    let interceptor = auth_interceptor(
        auth_required,
        auth_runtime.clone(),
        config.mtls_subject_header.clone(),
    );

    // Start background job worker
    let job_worker = JobWorker::new(db.clone(), Arc::clone(&schema_registry));
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let job_db_name = config.default_database.clone();
    let job_ns_name = config.default_namespace.clone();
    let projector_scopes = resolve_cache_projector_scopes(&config);
    let job_db_name_for_worker = job_db_name.clone();
    let job_ns_name_for_worker = job_ns_name.clone();
    let shutdown_for_jobs = shutdown_rx.clone();
    tokio::spawn(async move {
        if let Err(e) = job_worker
            .run(
                &job_db_name_for_worker,
                &job_ns_name_for_worker,
                shutdown_for_jobs,
            )
            .await
        {
            tracing::error!("Job worker exited with error: {}", e);
        }
    });

    if let Some(cache_store) = cache_store_for_projector {
        info!(
            "Starting {} cache projector scope(s)",
            projector_scopes.len()
        );
        for scope in projector_scopes {
            let projector_db = db.clone();
            let projector_cfg = cache_cfg.clone();
            let projector_db_name = scope.database;
            let projector_ns_name = scope.namespace;
            let projector_cache_store = Arc::clone(&cache_store);
            let shutdown_for_projector = shutdown_rx.clone();
            tokio::spawn(async move {
                match CdcProjector::new(
                    projector_db,
                    projector_cache_store,
                    &projector_cfg,
                    &projector_db_name,
                    &projector_ns_name,
                )
                .await
                {
                    Ok(mut projector) => {
                        if let Err(e) = projector.run(shutdown_for_projector).await {
                            tracing::error!(
                                "Cache projector exited for {}/{} with error: {}",
                                projector_db_name,
                                projector_ns_name,
                                e
                            );
                        }
                    }
                    Err(e) => tracing::error!(
                        "Failed to initialize cache projector for {}/{}: {}",
                        projector_db_name,
                        projector_ns_name,
                        e
                    ),
                }
            });
        }
    }

    if config.audit_enabled {
        let audit_db = db.clone();
        let retention_days = config.audit_retention_days.max(1);
        let sweep_secs = config.audit_retention_sweep_secs.max(10);
        let batch_limit = config.audit_retention_batch.max(1);
        let shutdown_for_audit = shutdown_rx.clone();
        tokio::spawn(async move {
            loop {
                if *shutdown_for_audit.borrow() {
                    break;
                }
                if shutdown_for_audit.has_changed().unwrap_or(false) && *shutdown_for_audit.borrow()
                {
                    break;
                }
                let retention_secs = retention_days.saturating_mul(86_400);
                match cleanup_audit_records(&audit_db, retention_secs, batch_limit).await {
                    Ok(deleted) if deleted > 0 => {
                        info!("audit retention removed {} records", deleted);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        warn!("audit retention sweep failed: {}", e);
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(sweep_secs)).await;
            }
        });
    }

    if config.watch_state_retention_enabled {
        let watch_state_db = db.clone();
        let retention_days = config.watch_state_retention_days.max(1);
        let sweep_secs = config.watch_state_retention_sweep_secs.max(10);
        let batch_limit = config.watch_state_retention_batch.max(1);
        let shutdown_for_watch_state = shutdown_rx.clone();
        tokio::spawn(async move {
            loop {
                if *shutdown_for_watch_state.borrow() {
                    break;
                }
                if shutdown_for_watch_state.has_changed().unwrap_or(false)
                    && *shutdown_for_watch_state.borrow()
                {
                    break;
                }
                let retention_secs = retention_days.saturating_mul(86_400);
                match cleanup_query_watch_states(&watch_state_db, retention_secs, batch_limit).await
                {
                    Ok(deleted) if deleted > 0 => {
                        info!("watch state retention removed {} records", deleted);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        warn!("watch state retention sweep failed: {}", e);
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(sweep_secs)).await;
            }
        });
    }

    start_replicators(
        db.clone(),
        Arc::clone(&schema_registry),
        Arc::clone(&id_allocator),
        config.site_id.to_string(),
        ReplicatorConfig::from_server_config(&config),
        shutdown_rx.clone(),
    );

    if config.docs_enabled {
        let docs_addr = config.docs_addr.clone();
        let docs_dir = config.docs_dir.clone();
        let docs_title = config.docs_title.clone();
        let shutdown_for_docs = shutdown_rx.clone();

        tokio::spawn(async move {
            if let Err(e) = docs_server::serve_docs(
                docs_addr.as_str(),
                std::path::PathBuf::from(docs_dir),
                docs_title,
                shutdown_for_docs,
            )
            .await
            {
                warn!("Docs server exited with error: {}", e);
            }
        });
    } else {
        info!("Docs server disabled (--docs-enabled=true to enable)");
    }

    if config.ui_enabled {
        let ui_config = UiServerConfig {
            addr: config.ui_addr.clone(),
            assets_dir: config.ui_assets_dir.clone(),
            title: config.ui_title.clone(),
        };
        let ui_deps = UiServerDeps {
            db: db.clone(),
            schema_registry: Arc::clone(&schema_registry),
            id_allocator: Arc::clone(&id_allocator),
            node_store: Arc::clone(&node_store),
            cached_read_path: cached_read_path.clone(),
            watch_registry: watch_registry.clone(),
            auth_runtime: auth_runtime.clone(),
            auth_required: config.auth_required,
            mtls_subject_header: config.mtls_subject_header.clone(),
            default_database: config.default_database.clone(),
            default_namespace: config.default_namespace.clone(),
            site_id: config.site_id,
            metrics_enabled: config.metrics_enabled,
            metrics_addr: config.metrics_addr.clone(),
        };
        let shutdown_for_ui = shutdown_rx.clone();
        tokio::spawn(async move {
            if let Err(e) = ui_server::serve_ui(ui_config, ui_deps, shutdown_for_ui).await {
                warn!("UI server exited with error: {}", e);
            }
        });
    } else {
        info!("UI server disabled (--ui-enabled=true to enable)");
    }

    // Parse listen address
    let addr = config.listen_addr.parse()?;

    info!("Starting gRPC server on {}", addr);

    let mut server = Server::builder();
    if let Some(tls_config) = build_server_tls_config(&config)? {
        info!(
            client_auth = %config.tls_client_auth,
            "gRPC TLS transport enabled"
        );
        server = server
            .tls_config(tls_config)
            .context("failed to apply gRPC TLS configuration")?;
    }

    // Build and start the gRPC server
    server
        .add_service(SchemaServiceServer::with_interceptor(
            schema_service,
            interceptor.clone(),
        ))
        .add_service(NodeServiceServer::with_interceptor(
            node_service,
            interceptor.clone(),
        ))
        .add_service(EdgeServiceServer::with_interceptor(
            edge_service,
            interceptor.clone(),
        ))
        .add_service(QueryServiceServer::with_interceptor(
            query_service,
            interceptor.clone(),
        ))
        .add_service(ReplicationServiceServer::with_interceptor(
            replication_service,
            interceptor.clone(),
        ))
        .add_service(AdminServiceServer::with_interceptor(
            admin_service,
            interceptor.clone(),
        ))
        .add_service(HealthServiceServer::with_interceptor(
            health_service,
            interceptor.clone(),
        ))
        .add_service(WatchServiceServer::with_interceptor(
            watch_service,
            interceptor.clone(),
        ))
        .add_service(AuthServiceServer::new(auth_service))
        .serve(addr)
        .await?;

    // Signal job worker to shut down
    let _ = shutdown_tx.send(true);
    info!("PelagoDB server shutdown complete");

    Ok(())
}

fn apply_cli_env_overrides(config: &ServerConfig) {
    // These subsystems still read env directly; map parsed CLI/env config into env.
    set_env_if_some("PELAGO_API_KEYS", config.api_keys.clone());
    set_env_if_some("PELAGO_MTLS_ENABLED", Some(config.mtls_enabled));
    set_env_if_some(
        "PELAGO_MTLS_SUBJECT_HEADER",
        Some(config.mtls_subject_header.clone()),
    );
    set_env_if_some("PELAGO_MTLS_SUBJECTS", config.mtls_subjects.clone());
    set_env_if_some("PELAGO_MTLS_FINGERPRINTS", config.mtls_fingerprints.clone());
    set_env_if_some(
        "PELAGO_MTLS_DEFAULT_ROLE",
        Some(config.mtls_default_role.clone()),
    );
    set_env_if_some("PELAGO_TLS_CERT", config.tls_cert.clone());
    set_env_if_some("PELAGO_TLS_KEY", config.tls_key.clone());
    set_env_if_some("PELAGO_TLS_CA", config.tls_ca.clone());
    set_env_if_some(
        "PELAGO_TLS_CLIENT_AUTH",
        Some(config.tls_client_auth.clone()),
    );
    set_env_if_some(
        "PELAGO_WATCH_MAX_SUBSCRIPTIONS",
        config.watch_max_subscriptions,
    );
    set_env_if_some(
        "PELAGO_WATCH_MAX_NAMESPACE_SUBSCRIPTIONS",
        config.watch_max_namespace_subscriptions,
    );
    set_env_if_some(
        "PELAGO_WATCH_MAX_QUERY_WATCHES",
        config.watch_max_query_watches,
    );
    set_env_if_some(
        "PELAGO_WATCH_MAX_PRINCIPAL_SUBSCRIPTIONS",
        config.watch_max_principal_subscriptions,
    );
    set_env_if_some("PELAGO_WATCH_MAX_TTL_SECS", config.watch_max_ttl_secs);
    set_env_if_some("PELAGO_WATCH_MAX_QUEUE_SIZE", config.watch_max_queue_size);
    set_env_if_some(
        "PELAGO_WATCH_MAX_DROPPED_EVENTS",
        config.watch_max_dropped_events,
    );
}

fn set_env_if_some<T: ToString>(name: &str, value: Option<T>) {
    if let Some(value) = value {
        std::env::set_var(name, value.to_string());
    }
}

fn load_server_config_file_into_env() -> Result<()> {
    let Some(config_path) = bootstrap_config_path() else {
        return Ok(());
    };

    let raw = std::fs::read_to_string(&config_path).with_context(|| {
        format!(
            "failed to read server config file '{}'",
            config_path.display()
        )
    })?;

    let parsed: toml::Value = toml::from_str(&raw).with_context(|| {
        format!(
            "failed to parse TOML config file '{}'",
            config_path.display()
        )
    })?;
    let table = parsed.as_table().ok_or_else(|| {
        anyhow::anyhow!(
            "server config file '{}' must be a TOML table",
            config_path.display()
        )
    })?;

    for (key, env_name) in config_key_env_pairs() {
        if std::env::var_os(env_name).is_some() {
            continue;
        }
        let Some(value) = table.get(*key) else {
            continue;
        };
        let rendered = toml_value_to_string(value).ok_or_else(|| {
            anyhow::anyhow!(
                "config key '{}' in '{}' has unsupported type (expected string/number/bool)",
                key,
                config_path.display()
            )
        })?;
        std::env::set_var(env_name, rendered);
    }

    Ok(())
}

fn bootstrap_config_path() -> Option<PathBuf> {
    let mut config = std::env::var("PELAGO_CONFIG").ok();
    let mut no_config = std::env::var("PELAGO_NO_CONFIG")
        .ok()
        .and_then(|v| parse_bool(&v))
        .unwrap_or(false);

    let mut args = std::env::args().skip(1).peekable();
    while let Some(arg) = args.next() {
        if arg == "--config" {
            if let Some(path) = args.next() {
                config = Some(path);
            }
            continue;
        }
        if let Some(path) = arg.strip_prefix("--config=") {
            config = Some(path.to_string());
            continue;
        }
        if arg == "--no-config" {
            if let Some(next) = args.peek() {
                if !next.starts_with("--") {
                    let value = args.next().unwrap_or_default();
                    no_config = parse_bool(&value).unwrap_or(true);
                    continue;
                }
            }
            no_config = true;
            continue;
        }
        if let Some(v) = arg.strip_prefix("--no-config=") {
            no_config = parse_bool(v).unwrap_or(true);
        }
    }

    if no_config {
        return None;
    }

    if let Some(path) = config {
        return Some(PathBuf::from(path));
    }

    let default = PathBuf::from("pelago-server.toml");
    if default.exists() {
        Some(default)
    } else {
        None
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn toml_value_to_string(value: &toml::Value) -> Option<String> {
    match value {
        toml::Value::String(v) => Some(v.clone()),
        toml::Value::Integer(v) => Some(v.to_string()),
        toml::Value::Float(v) => Some(v.to_string()),
        toml::Value::Boolean(v) => Some(v.to_string()),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ScopeSpec {
    database: String,
    namespace: String,
}

fn parse_scope_specs(raw: &str) -> Vec<ScopeSpec> {
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
            scopes.push(ScopeSpec {
                database: database.to_string(),
                namespace: namespace.to_string(),
            });
        }
    }

    scopes
}

fn resolve_cache_projector_scopes(config: &ServerConfig) -> Vec<ScopeSpec> {
    if let Some(raw) = config.cache_projector_scopes.as_deref() {
        let scopes = parse_scope_specs(raw);
        if !scopes.is_empty() {
            return scopes;
        }
        warn!(
            "Ignoring empty/invalid PELAGO_CACHE_PROJECTOR_SCOPES='{}'; falling back to defaults",
            raw
        );
    }

    let mut scopes = Vec::new();
    let mut seen = HashSet::new();

    let mut push_scope = |database: &str, namespace: &str| {
        if seen.insert((database.to_string(), namespace.to_string())) {
            scopes.push(ScopeSpec {
                database: database.to_string(),
                namespace: namespace.to_string(),
            });
        }
    };

    push_scope(&config.default_database, &config.default_namespace);
    if let Some(replication_scopes_raw) = config.replication_scopes.as_deref() {
        let replication_scopes = parse_scope_specs(replication_scopes_raw);
        if replication_scopes.is_empty() {
            warn!(
                "Ignoring empty/invalid PELAGO_REPLICATION_SCOPES='{}' while deriving cache projector defaults",
                replication_scopes_raw
            );
        } else {
            for scope in &replication_scopes {
                push_scope(&scope.database, &scope.namespace);
            }
            return scopes;
        }
    }

    let replication_database = config
        .replication_database
        .as_deref()
        .unwrap_or(&config.default_database);
    let replication_namespace = config
        .replication_namespace
        .as_deref()
        .unwrap_or(&config.default_namespace);
    push_scope(replication_database, replication_namespace);

    scopes
}

fn config_key_env_pairs() -> &'static [(&'static str, &'static str)] {
    &[
        ("fdb_cluster", "PELAGO_FDB_CLUSTER"),
        ("site_id", "PELAGO_SITE_ID"),
        ("site_name", "PELAGO_SITE_NAME"),
        ("listen_addr", "PELAGO_LISTEN_ADDR"),
        ("log_level", "RUST_LOG"),
        ("metrics_enabled", "PELAGO_METRICS_ENABLED"),
        ("metrics_addr", "PELAGO_METRICS_ADDR"),
        ("id_batch_size", "PELAGO_ID_BATCH_SIZE"),
        ("default_database", "PELAGO_DEFAULT_DATABASE"),
        ("default_namespace", "PELAGO_DEFAULT_NAMESPACE"),
        ("cache_enabled", "PELAGO_CACHE_ENABLED"),
        ("cache_path", "PELAGO_CACHE_PATH"),
        ("cache_size_mb", "PELAGO_CACHE_SIZE_MB"),
        ("cache_write_buffer_mb", "PELAGO_CACHE_WRITE_BUFFER_MB"),
        ("cache_max_write_buffers", "PELAGO_CACHE_MAX_WRITE_BUFFERS"),
        (
            "cache_bloom_bits_per_key",
            "PELAGO_CACHE_BLOOM_BITS_PER_KEY",
        ),
        (
            "cache_prefix_extractor_bytes",
            "PELAGO_CACHE_PREFIX_EXTRACTOR_BYTES",
        ),
        (
            "cache_use_column_families",
            "PELAGO_CACHE_USE_COLUMN_FAMILIES",
        ),
        (
            "cache_projector_batch_size",
            "PELAGO_CACHE_PROJECTOR_BATCH_SIZE",
        ),
        ("cache_projector_scopes", "PELAGO_CACHE_PROJECTOR_SCOPES"),
        (
            "cache_eventual_max_lag_ms",
            "PELAGO_CACHE_EVENTUAL_MAX_LAG_MS",
        ),
        (
            "cache_session_max_lag_ms",
            "PELAGO_CACHE_SESSION_MAX_LAG_MS",
        ),
        ("auth_required", "PELAGO_AUTH_REQUIRED"),
        ("api_keys", "PELAGO_API_KEYS"),
        ("mtls_enabled", "PELAGO_MTLS_ENABLED"),
        ("mtls_subject_header", "PELAGO_MTLS_SUBJECT_HEADER"),
        ("mtls_subjects", "PELAGO_MTLS_SUBJECTS"),
        ("mtls_fingerprints", "PELAGO_MTLS_FINGERPRINTS"),
        ("mtls_default_role", "PELAGO_MTLS_DEFAULT_ROLE"),
        ("tls_cert", "PELAGO_TLS_CERT"),
        ("tls_key", "PELAGO_TLS_KEY"),
        ("tls_ca", "PELAGO_TLS_CA"),
        ("tls_client_auth", "PELAGO_TLS_CLIENT_AUTH"),
        ("audit_enabled", "PELAGO_AUDIT_ENABLED"),
        ("audit_retention_days", "PELAGO_AUDIT_RETENTION_DAYS"),
        (
            "audit_retention_sweep_secs",
            "PELAGO_AUDIT_RETENTION_SWEEP_SECS",
        ),
        ("audit_retention_batch", "PELAGO_AUDIT_RETENTION_BATCH"),
        ("replication_enabled", "PELAGO_REPLICATION_ENABLED"),
        ("replication_peers", "PELAGO_REPLICATION_PEERS"),
        ("replication_database", "PELAGO_REPLICATION_DATABASE"),
        ("replication_namespace", "PELAGO_REPLICATION_NAMESPACE"),
        ("replication_scopes", "PELAGO_REPLICATION_SCOPES"),
        ("replication_batch_size", "PELAGO_REPLICATION_BATCH_SIZE"),
        ("replication_poll_ms", "PELAGO_REPLICATION_POLL_MS"),
        ("replication_api_key", "PELAGO_REPLICATION_API_KEY"),
        (
            "replication_lease_enabled",
            "PELAGO_REPLICATION_LEASE_ENABLED",
        ),
        (
            "replication_lease_ttl_ms",
            "PELAGO_REPLICATION_LEASE_TTL_MS",
        ),
        (
            "replication_lease_heartbeat_ms",
            "PELAGO_REPLICATION_LEASE_HEARTBEAT_MS",
        ),
        ("docs_enabled", "PELAGO_DOCS_ENABLED"),
        ("docs_addr", "PELAGO_DOCS_ADDR"),
        ("docs_dir", "PELAGO_DOCS_DIR"),
        ("docs_title", "PELAGO_DOCS_TITLE"),
        ("ui_enabled", "PELAGO_UI_ENABLED"),
        ("ui_addr", "PELAGO_UI_ADDR"),
        ("ui_assets_dir", "PELAGO_UI_ASSETS_DIR"),
        ("ui_title", "PELAGO_UI_TITLE"),
        ("watch_max_subscriptions", "PELAGO_WATCH_MAX_SUBSCRIPTIONS"),
        (
            "watch_max_namespace_subscriptions",
            "PELAGO_WATCH_MAX_NAMESPACE_SUBSCRIPTIONS",
        ),
        ("watch_max_query_watches", "PELAGO_WATCH_MAX_QUERY_WATCHES"),
        (
            "watch_max_principal_subscriptions",
            "PELAGO_WATCH_MAX_PRINCIPAL_SUBSCRIPTIONS",
        ),
        ("watch_max_ttl_secs", "PELAGO_WATCH_MAX_TTL_SECS"),
        ("watch_max_queue_size", "PELAGO_WATCH_MAX_QUEUE_SIZE"),
        (
            "watch_max_dropped_events",
            "PELAGO_WATCH_MAX_DROPPED_EVENTS",
        ),
        (
            "watch_state_retention_enabled",
            "PELAGO_WATCH_STATE_RETENTION_ENABLED",
        ),
        (
            "watch_state_retention_days",
            "PELAGO_WATCH_STATE_RETENTION_DAYS",
        ),
        (
            "watch_state_retention_sweep_secs",
            "PELAGO_WATCH_STATE_RETENTION_SWEEP_SECS",
        ),
        (
            "watch_state_retention_batch",
            "PELAGO_WATCH_STATE_RETENTION_BATCH",
        ),
    ]
}

fn install_metrics_recorder(config: &ServerConfig) -> Result<()> {
    if !config.metrics_enabled {
        return Ok(());
    }

    let addr: SocketAddr = config.metrics_addr.parse().with_context(|| {
        format!(
            "invalid PELAGO_METRICS_ADDR '{}': expected host:port",
            config.metrics_addr
        )
    })?;

    PrometheusBuilder::new()
        .with_http_listener(addr)
        .install_recorder()
        .map_err(|e| anyhow::anyhow!("failed to install metrics recorder: {}", e))?;
    info!("Prometheus metrics exporter listening on {}", addr);

    Ok(())
}

fn resolve_fdb_cluster_file(configured: &str) -> String {
    if std::path::Path::new(configured).exists() {
        return configured.to_string();
    }

    if let Ok(path) = std::env::var("FDB_CLUSTER_FILE") {
        if std::path::Path::new(&path).exists() {
            return path;
        }
    }

    if std::path::Path::new("fdb.cluster").exists() {
        return "fdb.cluster".to_string();
    }

    configured.to_string()
}

fn auth_interceptor(
    auth_required: bool,
    runtime: AuthRuntime,
    mtls_subject_header: String,
) -> impl FnMut(Request<()>) -> Result<Request<()>, Status> + Clone {
    let mtls_subject_header = mtls_subject_header.to_ascii_lowercase();
    move |mut req: Request<()>| {
        if !auth_required {
            return Ok(req);
        }

        if let Some(principal) =
            resolve_principal_from_tonic_request(&runtime, &mtls_subject_header, &req)
        {
            req.extensions_mut().insert(principal);
            return Ok(req);
        }

        Err(Status::unauthenticated(
            "authentication required: provide mTLS client cert, bearer token, or x-api-key",
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TlsClientAuthMode {
    None,
    Request,
    Require,
}

impl TlsClientAuthMode {
    fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "none" => Ok(Self::None),
            "request" => Ok(Self::Request),
            "require" => Ok(Self::Require),
            other => Err(anyhow::anyhow!(
                "invalid PELAGO_TLS_CLIENT_AUTH value '{}'; expected one of: none, request, require",
                other
            )),
        }
    }
}

fn build_server_tls_config(config: &ServerConfig) -> Result<Option<ServerTlsConfig>> {
    let tls_cert = config
        .tls_cert
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let tls_key = config
        .tls_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let tls_ca = config
        .tls_ca
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    if tls_cert.is_none() && tls_key.is_none() && tls_ca.is_none() {
        return Ok(None);
    }

    let cert_path = tls_cert.ok_or_else(|| {
        anyhow::anyhow!("PELAGO_TLS_CERT is required when TLS configuration is enabled")
    })?;
    let key_path = tls_key.ok_or_else(|| {
        anyhow::anyhow!("PELAGO_TLS_KEY is required when TLS configuration is enabled")
    })?;

    let cert_pem = std::fs::read(cert_path)
        .with_context(|| format!("failed to read TLS cert '{}'", cert_path))?;
    let key_pem = std::fs::read(key_path)
        .with_context(|| format!("failed to read TLS key '{}'", key_path))?;

    let mode = TlsClientAuthMode::parse(&config.tls_client_auth)?;
    let mut tls = ServerTlsConfig::new().identity(Identity::from_pem(cert_pem, key_pem));
    match mode {
        TlsClientAuthMode::None => {
            if tls_ca.is_some() {
                warn!(
                    "PELAGO_TLS_CA is set but PELAGO_TLS_CLIENT_AUTH=none; client certificate auth is disabled"
                );
            }
        }
        TlsClientAuthMode::Request | TlsClientAuthMode::Require => {
            let ca_path = tls_ca.ok_or_else(|| {
                anyhow::anyhow!(
                    "PELAGO_TLS_CA is required when PELAGO_TLS_CLIENT_AUTH is '{}'",
                    config.tls_client_auth
                )
            })?;
            let ca_pem = std::fs::read(ca_path)
                .with_context(|| format!("failed to read TLS CA cert '{}'", ca_path))?;
            tls = tls.client_ca_root(Certificate::from_pem(ca_pem));
            if mode == TlsClientAuthMode::Request {
                tls = tls.client_auth_optional(true);
            }
        }
    }

    Ok(Some(tls))
}
