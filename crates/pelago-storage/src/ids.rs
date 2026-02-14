//! ID allocation from atomic counters
//!
//! Each site allocates IDs in batches from FDB atomic counters.
//! Key: (db, ns, _ids, entity_type, site_id) -> u64 counter

use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Mutex;

/// ID allocator that batches allocation from FDB
pub struct IdAllocator {
    site_id: u8,
    batch_size: u64,
    /// Current allocation state per entity type
    allocations: Mutex<std::collections::HashMap<String, AllocationState>>,
}

struct AllocationState {
    /// Next ID to allocate
    next: AtomicU64,
    /// End of current batch (exclusive)
    batch_end: AtomicU64,
}

impl IdAllocator {
    pub fn new(site_id: u8, batch_size: u64) -> Self {
        Self {
            site_id,
            batch_size,
            allocations: Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Get the site ID
    pub fn site_id(&self) -> u8 {
        self.site_id
    }

    /// Allocate a new ID for an entity type
    /// Returns (site_id, sequence_number)
    pub async fn allocate(&self, entity_type: &str) -> (u8, u64) {
        let mut allocations = self.allocations.lock().await;

        let state = allocations
            .entry(entity_type.to_string())
            .or_insert_with(|| AllocationState {
                next: AtomicU64::new(0),
                batch_end: AtomicU64::new(0),
            });

        let next = state.next.load(Ordering::SeqCst);
        let batch_end = state.batch_end.load(Ordering::SeqCst);

        if next >= batch_end {
            // Need to allocate a new batch from FDB
            // In the real implementation, this would do an atomic add in FDB
            // For now, we just extend the batch
            let new_batch_start = batch_end;
            let new_batch_end = new_batch_start + self.batch_size;
            state.next.store(new_batch_start, Ordering::SeqCst);
            state.batch_end.store(new_batch_end, Ordering::SeqCst);
        }

        let id = state.next.fetch_add(1, Ordering::SeqCst);
        (self.site_id, id)
    }
}

impl Default for IdAllocator {
    fn default() -> Self {
        Self::new(1, 100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_id_allocation() {
        let allocator = IdAllocator::new(1, 10);

        let (site, seq1) = allocator.allocate("Person").await;
        let (_, seq2) = allocator.allocate("Person").await;
        let (_, seq3) = allocator.allocate("Person").await;

        assert_eq!(site, 1);
        assert_eq!(seq1, 0);
        assert_eq!(seq2, 1);
        assert_eq!(seq3, 2);
    }

    #[tokio::test]
    async fn test_separate_entity_types() {
        let allocator = IdAllocator::new(1, 10);

        let (_, person_id) = allocator.allocate("Person").await;
        let (_, company_id) = allocator.allocate("Company").await;

        // Both start at 0 since they're different entity types
        assert_eq!(person_id, 0);
        assert_eq!(company_id, 0);
    }
}
