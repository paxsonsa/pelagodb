use super::config::RocksCacheConfig;
use super::store::RocksCacheStore;
use crate::cdc::{CdcEntry, CdcOperation, Versionstamp};
use crate::consumer::{CdcConsumer, ConsumerConfig};
use crate::db::PelagoDb;
use pelago_core::PelagoError;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

const MAX_ADAPTIVE_DRAIN_WINDOWS: usize = 8;

pub struct CdcProjector {
    db: PelagoDb,
    consumer: CdcConsumer,
    cache: Arc<RocksCacheStore>,
    batch_size: usize,
    adaptive_drain_windows: usize,
    max_adaptive_drain_windows: usize,
    database: String,
    namespace: String,
}

pub struct RebuildStats {
    pub entries_processed: usize,
    pub nodes_written: usize,
    pub edges_written: usize,
}

struct FlattenedBatch {
    entry_count: usize,
    hwm: Versionstamp,
    timestamp: i64,
    operations: Vec<CdcOperation>,
    nodes_written: usize,
    edges_written: usize,
}

impl FlattenedBatch {
    fn from_entries(entries: Vec<(Versionstamp, CdcEntry)>) -> Option<Self> {
        if entries.is_empty() {
            return None;
        }

        let mut hwm = Versionstamp::zero();
        let mut timestamp = 0i64;
        let mut operations = Vec::new();
        let mut nodes_written = 0usize;
        let mut edges_written = 0usize;
        let entry_count = entries.len();

        for (vs, entry) in entries {
            hwm = vs;
            timestamp = entry.timestamp;
            for op in entry.operations {
                match &op {
                    CdcOperation::NodeCreate { .. }
                    | CdcOperation::NodeUpdate { .. }
                    | CdcOperation::NodeDelete { .. }
                    | CdcOperation::OwnershipTransfer { .. } => nodes_written += 1,
                    CdcOperation::EdgeCreate { .. } | CdcOperation::EdgeDelete { .. } => {
                        edges_written += 1
                    }
                    CdcOperation::SchemaRegister { .. } => {}
                }
                operations.push(op);
            }
        }

        Some(Self {
            entry_count,
            hwm,
            timestamp,
            operations,
            nodes_written,
            edges_written,
        })
    }
}

impl CdcProjector {
    pub async fn new(
        db: PelagoDb,
        cache: Arc<RocksCacheStore>,
        config: &RocksCacheConfig,
        database: &str,
        namespace: &str,
    ) -> Result<Self, PelagoError> {
        let batch_size = config.projector_batch_size.max(1);
        let consumer_id = format!(
            "cache_projector_{}_{}_{}_{}",
            config.site_id,
            database,
            namespace,
            projector_instance_id()
        );
        let consumer_config =
            ConsumerConfig::new(&consumer_id, database, namespace).with_batch_size(batch_size);
        let consumer = CdcConsumer::new(db.clone(), consumer_config).await?;

        Ok(Self {
            db,
            consumer,
            cache,
            batch_size,
            adaptive_drain_windows: 1,
            max_adaptive_drain_windows: MAX_ADAPTIVE_DRAIN_WINDOWS,
            database: database.to_string(),
            namespace: namespace.to_string(),
        })
    }

    pub async fn run(
        &mut self,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), PelagoError> {
        info!(
            "CDC projector started for {}/{}",
            self.database, self.namespace
        );
        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("CDC projector shutting down");
                        self.consumer.checkpoint().await?;
                        return Ok(());
                    }
                }
                result = self.project_batch() => {
                    match result {
                        Ok(0) => {
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                        Ok(n) => {
                            debug!("Projected {} CDC entries", n);
                        }
                        Err(e) => {
                            warn!("CDC projection error: {}", e);
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                    }
                }
            }
        }
    }

    pub async fn project_batch(&mut self) -> Result<usize, PelagoError> {
        let mut drained_entries = self.consumer.poll_batch().await?;
        if drained_entries.is_empty() {
            self.adaptive_drain_windows = next_adaptive_drain_window(
                self.adaptive_drain_windows,
                self.max_adaptive_drain_windows,
                self.batch_size,
                0,
                false,
            );
            return Ok(0);
        }

        let mut total_entries = drained_entries.len();
        let mut drained_windows = 1usize;
        let mut last_batch_full = total_entries >= self.batch_size;

        while last_batch_full && drained_windows < self.adaptive_drain_windows {
            let next_entries = self.consumer.poll_batch().await?;
            if next_entries.is_empty() {
                last_batch_full = false;
                break;
            }

            last_batch_full = next_entries.len() >= self.batch_size;
            total_entries = total_entries.saturating_add(next_entries.len());
            drained_entries.extend(next_entries);
            drained_windows += 1;
        }

        if let Some(flattened) = FlattenedBatch::from_entries(drained_entries) {
            self.cache.apply_cdc_operations(
                &self.database,
                &self.namespace,
                &flattened.operations,
                flattened.timestamp,
                &flattened.hwm,
            )?;
            self.consumer.ack_through(&flattened.hwm).await?;
        }

        let exhausted_with_full_batch =
            drained_windows == self.adaptive_drain_windows && last_batch_full;
        self.adaptive_drain_windows = next_adaptive_drain_window(
            self.adaptive_drain_windows,
            self.max_adaptive_drain_windows,
            self.batch_size,
            total_entries,
            exhausted_with_full_batch,
        );

        Ok(total_entries)
    }

    pub async fn rebuild(&mut self) -> Result<RebuildStats, PelagoError> {
        info!("Starting cache rebuild from CDC");
        self.cache.clear()?;

        let replay_consumer_id = format!(
            "cache_projector_rebuild_{}_{}",
            self.database,
            std::process::id()
        );
        let replay_config =
            ConsumerConfig::new(&replay_consumer_id, &self.database, &self.namespace)
                .with_batch_size(self.batch_size);
        let mut replay_consumer = CdcConsumer::new(self.db.clone(), replay_config).await?;

        let mut stats = RebuildStats {
            entries_processed: 0,
            nodes_written: 0,
            edges_written: 0,
        };

        loop {
            let entries = replay_consumer.poll_batch().await?;
            let Some(flattened) = FlattenedBatch::from_entries(entries) else {
                break;
            };

            self.cache.apply_cdc_operations(
                &self.database,
                &self.namespace,
                &flattened.operations,
                flattened.timestamp,
                &flattened.hwm,
            )?;
            replay_consumer.ack_through(&flattened.hwm).await?;
            stats.entries_processed = stats
                .entries_processed
                .saturating_add(flattened.entry_count);
            stats.nodes_written = stats.nodes_written.saturating_add(flattened.nodes_written);
            stats.edges_written = stats.edges_written.saturating_add(flattened.edges_written);
        }

        info!(
            "Cache rebuild complete: {} entries processed",
            stats.entries_processed
        );
        Ok(stats)
    }

    pub fn hwm(&self) -> &Versionstamp {
        self.consumer.hwm()
    }
}

fn projector_instance_id() -> String {
    std::env::var("POD_NAME")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| format!("pid{}", std::process::id()))
}

fn next_adaptive_drain_window(
    current_window: usize,
    max_window: usize,
    base_batch_size: usize,
    entries_processed: usize,
    exhausted_with_full_batch: bool,
) -> usize {
    let current_window = current_window.max(1);
    let max_window = max_window.max(1);
    let base_batch_size = base_batch_size.max(1);

    if exhausted_with_full_batch {
        return current_window.saturating_mul(2).min(max_window);
    }

    if entries_processed < base_batch_size {
        return current_window.saturating_sub(1).max(1);
    }

    current_window
}

#[cfg(test)]
mod tests {
    use super::{next_adaptive_drain_window, FlattenedBatch};
    use crate::cdc::{CdcEntry, CdcOperation, Versionstamp};
    use std::collections::HashMap;

    #[test]
    fn flattened_batch_keeps_last_hwm_and_op_counts() {
        let entries = vec![
            (
                Versionstamp::from_bytes(&[0, 0, 0, 0, 0, 0, 0, 1, 0, 1]).expect("hwm1"),
                CdcEntry {
                    site: "1".to_string(),
                    timestamp: 1,
                    batch_id: None,
                    operations: vec![CdcOperation::NodeCreate {
                        entity_type: "Person".to_string(),
                        node_id: "1_1".to_string(),
                        properties: HashMap::new(),
                        home_site: "1".to_string(),
                    }],
                },
            ),
            (
                Versionstamp::from_bytes(&[0, 0, 0, 0, 0, 0, 0, 2, 0, 1]).expect("hwm2"),
                CdcEntry {
                    site: "1".to_string(),
                    timestamp: 2,
                    batch_id: None,
                    operations: vec![CdcOperation::EdgeDelete {
                        source_type: "Person".to_string(),
                        source_id: "1_1".to_string(),
                        target_type: "Person".to_string(),
                        target_id: "1_2".to_string(),
                        edge_type: "KNOWS".to_string(),
                    }],
                },
            ),
        ];

        let flattened = FlattenedBatch::from_entries(entries).expect("flattened");
        assert_eq!(flattened.entry_count, 2);
        assert_eq!(flattened.operations.len(), 2);
        assert_eq!(flattened.nodes_written, 1);
        assert_eq!(flattened.edges_written, 1);
        assert_eq!(flattened.timestamp, 2);
        assert_eq!(
            flattened.hwm,
            Versionstamp::from_bytes(&[0, 0, 0, 0, 0, 0, 0, 2, 0, 1]).expect("expected hwm")
        );
    }

    #[test]
    fn adaptive_window_grows_when_backlog_exhausts_window() {
        let next = next_adaptive_drain_window(2, 8, 1000, 2000, true);
        assert_eq!(next, 4);
    }

    #[test]
    fn adaptive_window_shrinks_when_batch_not_full() {
        let next = next_adaptive_drain_window(4, 8, 1000, 120, false);
        assert_eq!(next, 3);
    }
}
