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
    ReplicationServiceImpl, SchemaServiceImpl,
};
use pelago_core::ServerConfig;
use pelago_proto::{
    admin_service_server::AdminServiceServer, edge_service_server::EdgeServiceServer,
    health_service_server::HealthServiceServer, node_service_server::NodeServiceServer,
    query_service_server::QueryServiceServer,
    replication_service_server::ReplicationServiceServer,
    schema_service_server::SchemaServiceServer,
};
use pelago_storage::{IdAllocator, NodeStore, PelagoDb, SchemaCache, SchemaRegistry};
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

    // Create service implementations
    let schema_service = SchemaServiceImpl::new(Arc::clone(&schema_registry));
    let node_service = NodeServiceImpl::new(
        db.clone(),
        Arc::clone(&schema_registry),
        Arc::clone(&id_allocator),
        site_id_str.clone(),
    );
    let edge_service = EdgeServiceImpl::new(
        db.clone(),
        Arc::clone(&schema_registry),
        Arc::clone(&id_allocator),
        Arc::clone(&node_store),
        site_id_str.clone(),
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
        .serve(addr)
        .await?;

    info!("PelagoDB server shutdown complete");

    Ok(())
}
