//! Server configuration

use clap::{ArgAction, Parser};

#[derive(Parser, Debug, Clone)]
#[command(name = "pelago", about = "PelagoDB graph database server")]
pub struct ServerConfig {
    /// Path to optional server config file (TOML)
    #[arg(long, env = "PELAGO_CONFIG")]
    pub config: Option<String>,

    /// Disable loading config file
    #[arg(long, env = "PELAGO_NO_CONFIG", default_value_t = false)]
    pub no_config: bool,

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

    /// Default database used by background workers and replicator defaults
    #[arg(long, env = "PELAGO_DEFAULT_DATABASE", default_value = "default")]
    pub default_database: String,

    /// Default namespace used by background workers and replicator defaults
    #[arg(long, env = "PELAGO_DEFAULT_NAMESPACE", default_value = "default")]
    pub default_namespace: String,

    /// Enable RocksDB cache layer
    #[arg(
        long,
        env = "PELAGO_CACHE_ENABLED",
        default_value_t = true,
        action = ArgAction::Set
    )]
    pub cache_enabled: bool,

    /// RocksDB cache path
    #[arg(long, env = "PELAGO_CACHE_PATH", default_value = "./data/cache")]
    pub cache_path: String,

    /// RocksDB block cache size in MB
    #[arg(long, env = "PELAGO_CACHE_SIZE_MB", default_value_t = 1024)]
    pub cache_size_mb: usize,

    /// RocksDB write buffer size in MB
    #[arg(long, env = "PELAGO_CACHE_WRITE_BUFFER_MB", default_value_t = 64)]
    pub cache_write_buffer_mb: usize,

    /// RocksDB max write buffers
    #[arg(long, env = "PELAGO_CACHE_MAX_WRITE_BUFFERS", default_value_t = 3)]
    pub cache_max_write_buffers: i32,

    /// CDC projector batch size for cache projection
    #[arg(
        long,
        env = "PELAGO_CACHE_PROJECTOR_BATCH_SIZE",
        default_value_t = 1000
    )]
    pub cache_projector_batch_size: usize,

    /// Require auth on all gRPC requests
    #[arg(
        long,
        env = "PELAGO_AUTH_REQUIRED",
        default_value_t = false,
        action = ArgAction::Set
    )]
    pub auth_required: bool,

    /// Static API key map (CSV entries; optionally `key:principal`)
    #[arg(long, env = "PELAGO_API_KEYS")]
    pub api_keys: Option<String>,

    /// Enable mTLS subject-based principal extraction (usually from a trusted proxy header)
    #[arg(
        long,
        env = "PELAGO_MTLS_ENABLED",
        default_value_t = false,
        action = ArgAction::Set
    )]
    pub mtls_enabled: bool,

    /// Metadata header containing mTLS subject identity
    #[arg(
        long,
        env = "PELAGO_MTLS_SUBJECT_HEADER",
        default_value = "x-mtls-subject"
    )]
    pub mtls_subject_header: String,

    /// Optional mTLS subject->principal mappings (`subject=principal,subject2=principal2`)
    #[arg(long, env = "PELAGO_MTLS_SUBJECTS")]
    pub mtls_subjects: Option<String>,

    /// Optional mTLS certificate fingerprint->principal mappings
    /// (`sha256:hex=principal,sha256:hex2=principal2`)
    #[arg(long, env = "PELAGO_MTLS_FINGERPRINTS")]
    pub mtls_fingerprints: Option<String>,

    /// Default role assigned to mTLS-authenticated principals
    #[arg(long, env = "PELAGO_MTLS_DEFAULT_ROLE", default_value = "service")]
    pub mtls_default_role: String,

    /// Server TLS certificate (PEM). Required when TLS is enabled.
    #[arg(long, env = "PELAGO_TLS_CERT")]
    pub tls_cert: Option<String>,

    /// Server TLS private key (PEM). Required when TLS is enabled.
    #[arg(long, env = "PELAGO_TLS_KEY")]
    pub tls_key: Option<String>,

    /// Optional CA certificate bundle used for client certificate validation.
    #[arg(long, env = "PELAGO_TLS_CA")]
    pub tls_ca: Option<String>,

    /// TLS client authentication mode: `none`, `request`, or `require`.
    #[arg(long, env = "PELAGO_TLS_CLIENT_AUTH", default_value = "none")]
    pub tls_client_auth: String,

    /// Enable audit retention sweeps
    #[arg(
        long,
        env = "PELAGO_AUDIT_ENABLED",
        default_value_t = true,
        action = ArgAction::Set
    )]
    pub audit_enabled: bool,

    /// Audit retention in days
    #[arg(long, env = "PELAGO_AUDIT_RETENTION_DAYS", default_value_t = 90)]
    pub audit_retention_days: u64,

    /// Audit sweep interval in seconds
    #[arg(long, env = "PELAGO_AUDIT_RETENTION_SWEEP_SECS", default_value_t = 300)]
    pub audit_retention_sweep_secs: u64,

    /// Max records deleted per audit retention sweep
    #[arg(long, env = "PELAGO_AUDIT_RETENTION_BATCH", default_value_t = 1000)]
    pub audit_retention_batch: usize,

    /// Enable replication pull workers (when unset, defaults to enabled if peers are configured)
    #[arg(long, env = "PELAGO_REPLICATION_ENABLED")]
    pub replication_enabled: Option<bool>,

    /// Replication peers (`site_id=host:port,site_id=host:port`)
    #[arg(long, env = "PELAGO_REPLICATION_PEERS", default_value = "")]
    pub replication_peers: String,

    /// Replication source database override
    #[arg(long, env = "PELAGO_REPLICATION_DATABASE")]
    pub replication_database: Option<String>,

    /// Replication source namespace override
    #[arg(long, env = "PELAGO_REPLICATION_NAMESPACE")]
    pub replication_namespace: Option<String>,

    /// Replication pull batch size
    #[arg(long, env = "PELAGO_REPLICATION_BATCH_SIZE", default_value_t = 512)]
    pub replication_batch_size: usize,

    /// Replication poll interval in milliseconds
    #[arg(long, env = "PELAGO_REPLICATION_POLL_MS", default_value_t = 300)]
    pub replication_poll_ms: u64,

    /// API key used by pull replicators for source auth
    #[arg(long, env = "PELAGO_REPLICATION_API_KEY")]
    pub replication_api_key: Option<String>,

    /// Require active lease for replication worker execution
    #[arg(
        long,
        env = "PELAGO_REPLICATION_LEASE_ENABLED",
        default_value_t = true,
        action = ArgAction::Set
    )]
    pub replication_lease_enabled: bool,

    /// Replicator lease TTL in milliseconds
    #[arg(
        long,
        env = "PELAGO_REPLICATION_LEASE_TTL_MS",
        default_value_t = 10_000
    )]
    pub replication_lease_ttl_ms: u64,

    /// Replicator lease heartbeat period in milliseconds
    #[arg(
        long,
        env = "PELAGO_REPLICATION_LEASE_HEARTBEAT_MS",
        default_value_t = 2_000
    )]
    pub replication_lease_heartbeat_ms: u64,

    /// Enable embedded docs HTTP server
    #[arg(
        long,
        env = "PELAGO_DOCS_ENABLED",
        default_value_t = false,
        action = ArgAction::Set
    )]
    pub docs_enabled: bool,

    /// Embedded docs bind address
    #[arg(long, env = "PELAGO_DOCS_ADDR", default_value = "127.0.0.1:4070")]
    pub docs_addr: String,

    /// Embedded docs markdown directory
    #[arg(long, env = "PELAGO_DOCS_DIR", default_value = "docs")]
    pub docs_dir: String,

    /// Embedded docs site title
    #[arg(
        long,
        env = "PELAGO_DOCS_TITLE",
        default_value = "PelagoDB Documentation"
    )]
    pub docs_title: String,

    /// Max total watch subscriptions
    #[arg(long, env = "PELAGO_WATCH_MAX_SUBSCRIPTIONS")]
    pub watch_max_subscriptions: Option<usize>,

    /// Max watch subscriptions per namespace
    #[arg(long, env = "PELAGO_WATCH_MAX_NAMESPACE_SUBSCRIPTIONS")]
    pub watch_max_namespace_subscriptions: Option<usize>,

    /// Max query watches per namespace
    #[arg(long, env = "PELAGO_WATCH_MAX_QUERY_WATCHES")]
    pub watch_max_query_watches: Option<usize>,

    /// Max watch subscriptions per principal
    #[arg(long, env = "PELAGO_WATCH_MAX_PRINCIPAL_SUBSCRIPTIONS")]
    pub watch_max_principal_subscriptions: Option<usize>,

    /// Max watch TTL seconds
    #[arg(long, env = "PELAGO_WATCH_MAX_TTL_SECS")]
    pub watch_max_ttl_secs: Option<u32>,

    /// Max buffered events per watch stream
    #[arg(long, env = "PELAGO_WATCH_MAX_QUEUE_SIZE")]
    pub watch_max_queue_size: Option<u32>,

    /// Max dropped events before terminating a watch stream
    #[arg(long, env = "PELAGO_WATCH_MAX_DROPPED_EVENTS")]
    pub watch_max_dropped_events: Option<u32>,

    /// Enable retention cleanup for durable query-watch state
    #[arg(
        long,
        env = "PELAGO_WATCH_STATE_RETENTION_ENABLED",
        default_value_t = true,
        action = ArgAction::Set
    )]
    pub watch_state_retention_enabled: bool,

    /// Query-watch state retention in days
    #[arg(long, env = "PELAGO_WATCH_STATE_RETENTION_DAYS", default_value_t = 7)]
    pub watch_state_retention_days: u64,

    /// Query-watch state retention sweep interval in seconds
    #[arg(
        long,
        env = "PELAGO_WATCH_STATE_RETENTION_SWEEP_SECS",
        default_value_t = 300
    )]
    pub watch_state_retention_sweep_secs: u64,

    /// Max query-watch state rows deleted per sweep
    #[arg(
        long,
        env = "PELAGO_WATCH_STATE_RETENTION_BATCH",
        default_value_t = 1000
    )]
    pub watch_state_retention_batch: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            config: None,
            no_config: false,
            fdb_cluster: "/etc/foundationdb/fdb.cluster".to_string(),
            site_id: 1,
            site_name: "default".to_string(),
            listen_addr: "[::1]:50051".to_string(),
            log_level: "info".to_string(),
            id_batch_size: 100,
            default_database: "default".to_string(),
            default_namespace: "default".to_string(),
            cache_enabled: true,
            cache_path: "./data/cache".to_string(),
            cache_size_mb: 1024,
            cache_write_buffer_mb: 64,
            cache_max_write_buffers: 3,
            cache_projector_batch_size: 1000,
            auth_required: false,
            api_keys: None,
            mtls_enabled: false,
            mtls_subject_header: "x-mtls-subject".to_string(),
            mtls_subjects: None,
            mtls_fingerprints: None,
            mtls_default_role: "service".to_string(),
            tls_cert: None,
            tls_key: None,
            tls_ca: None,
            tls_client_auth: "none".to_string(),
            audit_enabled: true,
            audit_retention_days: 90,
            audit_retention_sweep_secs: 300,
            audit_retention_batch: 1000,
            replication_enabled: None,
            replication_peers: String::new(),
            replication_database: None,
            replication_namespace: None,
            replication_batch_size: 512,
            replication_poll_ms: 300,
            replication_api_key: None,
            replication_lease_enabled: true,
            replication_lease_ttl_ms: 10_000,
            replication_lease_heartbeat_ms: 2_000,
            docs_enabled: false,
            docs_addr: "127.0.0.1:4070".to_string(),
            docs_dir: "docs".to_string(),
            docs_title: "PelagoDB Documentation".to_string(),
            watch_max_subscriptions: None,
            watch_max_namespace_subscriptions: None,
            watch_max_query_watches: None,
            watch_max_principal_subscriptions: None,
            watch_max_ttl_secs: None,
            watch_max_queue_size: None,
            watch_max_dropped_events: None,
            watch_state_retention_enabled: true,
            watch_state_retention_days: 7,
            watch_state_retention_sweep_secs: 300,
            watch_state_retention_batch: 1000,
        }
    }
}
