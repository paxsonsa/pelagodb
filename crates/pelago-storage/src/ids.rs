//! ID allocation from atomic counters
//!
//! Each site allocates IDs in batches from FDB atomic counters.
//! Key: (db, ns, _ids, entity_type, site_id) -> u64 counter

use crate::db::PelagoDb;
use crate::Subspace;
use pelago_core::PelagoError;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Mutex;

/// ID allocator that batches allocation from FDB atomic counters
pub struct IdAllocator {
    db: PelagoDb,
    site_id: u8,
    batch_size: u64,
    /// Current allocation state per entity type
    allocations: Mutex<std::collections::HashMap<String, AllocationState>>,
}

struct AllocationState {
    /// Next ID to allocate from local batch
    next: AtomicU64,
    /// End of current batch (exclusive)
    batch_end: AtomicU64,
}

impl IdAllocator {
    pub fn new(db: PelagoDb, site_id: u8, batch_size: u64) -> Self {
        Self {
            db,
            site_id,
            batch_size,
            allocations: Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Get the site ID
    pub fn site_id(&self) -> u8 {
        self.site_id
    }

    /// Build the FDB key for the ID counter
    fn counter_key(subspace: &Subspace, entity_type: &str, site_id: u8) -> Vec<u8> {
        subspace
            .pack()
            .add_string(entity_type)
            .add_int(site_id as i64)
            .build()
            .to_vec()
    }

    /// Allocate a new ID for an entity type
    /// Returns (site_id, sequence_number)
    pub async fn allocate(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
    ) -> Result<(u8, u64), PelagoError> {
        let cache_key = format!("{}:{}:{}", database, namespace, entity_type);
        let mut allocations = self.allocations.lock().await;

        let state = allocations
            .entry(cache_key.clone())
            .or_insert_with(|| AllocationState {
                next: AtomicU64::new(0),
                batch_end: AtomicU64::new(0),
            });

        let next = state.next.load(Ordering::SeqCst);
        let batch_end = state.batch_end.load(Ordering::SeqCst);

        if next >= batch_end {
            // Need to allocate a new batch from FDB
            let new_batch_start = self
                .allocate_batch_from_fdb(database, namespace, entity_type)
                .await?;
            let new_batch_end = new_batch_start + self.batch_size;
            state.next.store(new_batch_start, Ordering::SeqCst);
            state.batch_end.store(new_batch_end, Ordering::SeqCst);
        }

        let id = state.next.fetch_add(1, Ordering::SeqCst);
        Ok((self.site_id, id))
    }

    /// Allocate a batch of IDs from FDB using atomic add
    async fn allocate_batch_from_fdb(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
    ) -> Result<u64, PelagoError> {
        let subspace = Subspace::namespace(database, namespace).ids();
        let key = Self::counter_key(&subspace, entity_type, self.site_id);

        // Create a transaction for atomic add
        let trx = self.db.create_transaction()?;

        // Read current value
        let current = match trx
            .get(&key, false)
            .await
            .map_err(|e| PelagoError::Internal(format!("Failed to read ID counter: {}", e)))?
        {
            Some(bytes) => {
                if bytes.len() != 8 {
                    return Err(PelagoError::Internal(format!(
                        "Invalid ID counter bytes length: {}",
                        bytes.len()
                    )));
                }
                let arr: [u8; 8] = bytes.as_ref().try_into().unwrap();
                u64::from_be_bytes(arr)
            }
            None => 0,
        };

        // Write new value (current + batch_size)
        let new_value = current + self.batch_size;
        trx.set(&key, &new_value.to_be_bytes());

        trx.commit()
            .await
            .map_err(|e| PelagoError::Internal(format!("Failed to allocate ID batch: {}", e)))?;

        Ok(current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests require FDB to be running
    // Run with: cargo test --test fdb_integration -- --ignored

    #[test]
    fn test_counter_key_generation() {
        let subspace = Subspace::namespace("db", "ns").ids();
        let key = IdAllocator::counter_key(&subspace, "Person", 1);

        // Key should contain the subspace prefix
        assert!(!key.is_empty());
        assert!(key.len() > subspace.prefix().len());
    }
}
