//! PelagoDB Server
//!
//! Main entry point for the PelagoDB gRPC server.

use anyhow::Result;
use clap::Parser;
use pelago_core::ServerConfig;
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

    // TODO: Initialize FDB connection
    // TODO: Start gRPC server

    info!("PelagoDB server ready");

    // Keep server running
    tokio::signal::ctrl_c().await?;
    info!("Shutting down");

    Ok(())
}
