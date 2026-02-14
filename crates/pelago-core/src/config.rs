//! Server configuration

use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(name = "pelago", about = "PelagoDB graph database server")]
pub struct ServerConfig {
    /// FDB cluster file path
    #[arg(
        long,
        env = "PELAGO_FDB_CLUSTER",
        default_value = "/etc/foundationdb/fdb.cluster"
    )]
    pub fdb_cluster: String,

    /// Site ID for this server instance (0-255)
    #[arg(long, env = "PELAGO_SITE_ID")]
    pub site_id: u8,

    /// Site name for collision detection
    #[arg(long, env = "PELAGO_SITE_NAME", default_value = "default")]
    pub site_name: String,

    /// gRPC listen address
    #[arg(long, env = "PELAGO_LISTEN_ADDR", default_value = "[::1]:50051")]
    pub listen_addr: String,

    /// Log level filter
    #[arg(long, env = "RUST_LOG", default_value = "info")]
    pub log_level: String,

    /// ID allocation batch size
    #[arg(long, env = "PELAGO_ID_BATCH_SIZE", default_value = "100")]
    pub id_batch_size: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            fdb_cluster: "/etc/foundationdb/fdb.cluster".to_string(),
            site_id: 1,
            site_name: "default".to_string(),
            listen_addr: "[::1]:50051".to_string(),
            log_level: "info".to_string(),
            id_batch_size: 100,
        }
    }
}
