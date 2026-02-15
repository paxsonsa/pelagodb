/// Configuration for the RocksDB cache layer
#[derive(Clone, Debug)]
pub struct RocksCacheConfig {
    pub enabled: bool,
    pub path: String,
    pub cache_size_mb: usize,
    pub write_buffer_mb: usize,
    pub max_write_buffers: i32,
    pub projector_batch_size: usize,
    pub warm_on_start: bool,
    pub warm_types: Vec<String>,
    pub site_id: String,
}

impl Default for RocksCacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            path: "./data/cache".to_string(),
            cache_size_mb: 1024,
            write_buffer_mb: 64,
            max_write_buffers: 3,
            projector_batch_size: 1000,
            warm_on_start: false,
            warm_types: Vec::new(),
            site_id: "1".to_string(),
        }
    }
}
