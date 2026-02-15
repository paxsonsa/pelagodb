//! PelagoDB Server
//!
//! Main entry point for the PelagoDB gRPC server.
//!
//! Initializes the FDB connection, sets up shared components (schema cache, ID allocator),
//! and starts the gRPC server with all service handlers.

use anyhow::Result;
use clap::Parser;
use pelago_api::{
    AdminServiceImpl, EdgeServiceImpl, HealthServiceImpl, NodeServiceImpl, QueryServiceImpl,
    ReplicationServiceImpl, SchemaServiceImpl, WatchServiceImpl,
};
use pelago_core::ServerConfig;
use pelago_proto::{
    admin_service_server::AdminServiceServer, edge_service_server::EdgeServiceServer,
    health_service_server::HealthServiceServer, node_service_server::NodeServiceServer,
    query_service_server::QueryServiceServer,
    replication_service_server::ReplicationServiceServer,
    schema_service_server::SchemaServiceServer,
    watch_service_server::WatchServiceServer,
};
use pelago_storage::{
    CachedReadPath, CdcProjector, IdAllocator, JobWorker, NodeStore, PelagoDb, RegistryConfig,
    RocksCacheConfig, RocksCacheStore, SchemaCache, SchemaRegistry, SubscriptionRegistry,
    TtlManager,
};
use std::sync::Arc;
use tonic::transport::Server;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    // Parse configuration
    let config = ServerConfig::parse();

    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| config.log_level.clone().into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("PelagoDB server starting");
    info!("Site ID: {}", config.site_id);
    info!("Listen address: {}", config.listen_addr);

    // Connect to FDB cluster (this also initializes the FDB network)
    info!("Connecting to FDB cluster: {}", config.fdb_cluster);
    let db = PelagoDb::connect(&config.fdb_cluster).await?;

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

    // Cache layer configuration from env (`PELAGO_CACHE_*`)
    let mut cache_cfg = RocksCacheConfig::default();
    cache_cfg.enabled = env_bool("PELAGO_CACHE_ENABLED", true);
    if let Ok(v) = std::env::var("PELAGO_CACHE_PATH") {
        cache_cfg.path = v;
    }
    if let Ok(v) = std::env::var("PELAGO_CACHE_SIZE_MB") {
        if let Ok(n) = v.parse::<usize>() {
            cache_cfg.cache_size_mb = n;
        }
    }
    if let Ok(v) = std::env::var("PELAGO_CACHE_WRITE_BUFFER_MB") {
        if let Ok(n) = v.parse::<usize>() {
            cache_cfg.write_buffer_mb = n;
        }
    }
    if let Ok(v) = std::env::var("PELAGO_CACHE_MAX_WRITE_BUFFERS") {
        if let Ok(n) = v.parse::<i32>() {
            cache_cfg.max_write_buffers = n;
        }
    }
    if let Ok(v) = std::env::var("PELAGO_CACHE_PROJECTOR_BATCH_SIZE") {
        if let Ok(n) = v.parse::<usize>() {
            cache_cfg.projector_batch_size = n;
        }
    }
    cache_cfg.site_id = config.site_id.to_string();

    let mut cache_store_for_projector: Option<Arc<RocksCacheStore>> = None;
    let cached_read_path: Option<Arc<CachedReadPath>> = if cache_cfg.enabled {
        info!("RocksDB cache enabled at {}", cache_cfg.path);
        let cache_store = Arc::new(RocksCacheStore::open(&cache_cfg)?);
        cache_store_for_projector = Some(cache_store.clone());
        Some(Arc::new(CachedReadPath::new(db.clone(), cache_store)))
    } else {
        info!("RocksDB cache disabled via PELAGO_CACHE_ENABLED");
        None
    };

    // Create service implementations
    let schema_service = SchemaServiceImpl::new(Arc::clone(&schema_registry));
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

    // Start background job worker
    let job_worker = JobWorker::new(db.clone(), Arc::clone(&schema_registry));
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let job_db_name = std::env::var("PELAGO_DEFAULT_DATABASE").unwrap_or_else(|_| "default".to_string());
    let job_ns_name = std::env::var("PELAGO_DEFAULT_NAMESPACE").unwrap_or_else(|_| "default".to_string());
    let job_db_name_for_worker = job_db_name.clone();
    let job_ns_name_for_worker = job_ns_name.clone();
    let shutdown_for_jobs = shutdown_rx.clone();
    tokio::spawn(async move {
        if let Err(e) = job_worker
            .run(&job_db_name_for_worker, &job_ns_name_for_worker, shutdown_for_jobs)
            .await
        {
            tracing::error!("Job worker exited with error: {}", e);
        }
    });

    if let Some(cache_store) = cache_store_for_projector {
        let projector_db = db.clone();
        let projector_cfg = cache_cfg.clone();
        let projector_db_name = job_db_name.clone();
        let projector_ns_name = job_ns_name.clone();
        let shutdown_for_projector = shutdown_rx.clone();
        tokio::spawn(async move {
            match CdcProjector::new(
                projector_db,
                cache_store,
                &projector_cfg,
                &projector_db_name,
                &projector_ns_name,
            )
            .await
            {
                Ok(mut projector) => {
                    if let Err(e) = projector.run(shutdown_for_projector).await {
                        tracing::error!("Cache projector exited with error: {}", e);
                    }
                }
                Err(e) => tracing::error!("Failed to initialize cache projector: {}", e),
            }
        });
    }

    // Watch system: subscription registry and TTL manager
    let watch_config = RegistryConfig::default();
    let subscription_registry = Arc::new(SubscriptionRegistry::new(db.clone(), watch_config));
    let watch_service = WatchServiceImpl::new(
        Arc::clone(&subscription_registry),
        Arc::clone(&schema_registry),
    );

    // Spawn TTL manager to expire stale subscriptions
    let ttl_registry = Arc::clone(&subscription_registry);
    let shutdown_for_ttl = shutdown_rx.clone();
    tokio::spawn(async move {
        let ttl_manager = TtlManager::new(ttl_registry);
        ttl_manager.run(shutdown_for_ttl).await;
    });

    // Parse listen address
    let addr = config.listen_addr.parse()?;

    info!("Starting gRPC server on {}", addr);

    // Build and start the gRPC server
    Server::builder()
        .add_service(SchemaServiceServer::new(schema_service))
        .add_service(NodeServiceServer::new(node_service))
        .add_service(EdgeServiceServer::new(edge_service))
        .add_service(QueryServiceServer::new(query_service))
        .add_service(ReplicationServiceServer::new(replication_service))
        .add_service(AdminServiceServer::new(admin_service))
        .add_service(HealthServiceServer::new(health_service))
        .add_service(WatchServiceServer::new(watch_service))
        .serve(addr)
        .await?;

    // Signal background workers to shut down
    let _ = shutdown_tx.send(true);
    subscription_registry.shutdown().await;
    info!("PelagoDB server shutdown complete");

    Ok(())
}

fn env_bool(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .and_then(|v| match v.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or(default)
}
