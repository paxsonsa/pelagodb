//! Schema cache for avoiding repeated FDB reads
//!
//! In-memory cache with version checking and invalidation.

use pelago_core::schema::EntitySchema;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Cache key: (database, namespace, entity_type)
type CacheKey = (String, String, String);

/// Thread-safe schema cache
pub struct SchemaCache {
    cache: RwLock<HashMap<CacheKey, Arc<EntitySchema>>>,
}

impl SchemaCache {
    pub fn new() -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Get a cached schema
    pub async fn get(&self, db: &str, ns: &str, entity_type: &str) -> Option<Arc<EntitySchema>> {
        let cache = self.cache.read().await;
        cache
            .get(&(db.to_string(), ns.to_string(), entity_type.to_string()))
            .cloned()
    }

    /// Insert a schema into the cache
    pub async fn insert(&self, db: &str, ns: &str, schema: EntitySchema) {
        let mut cache = self.cache.write().await;
        cache.insert(
            (db.to_string(), ns.to_string(), schema.name.clone()),
            Arc::new(schema),
        );
    }

    /// Invalidate a cached schema
    pub async fn invalidate(&self, db: &str, ns: &str, entity_type: &str) {
        let mut cache = self.cache.write().await;
        cache.remove(&(db.to_string(), ns.to_string(), entity_type.to_string()));
    }

    /// Clear all cached schemas
    pub async fn clear(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
    }
}

impl Default for SchemaCache {
    fn default() -> Self {
        Self::new()
    }
}
