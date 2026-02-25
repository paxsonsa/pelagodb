//! CDC Consumer Framework
//!
//! Consumers track a high-water mark (HWM) — the versionstamp of the last
//! processed CDC entry — and checkpoint it periodically to FDB. On restart,
//! a consumer resumes from its saved checkpoint rather than re-processing
//! the entire CDC log.
//!
//! Key layout for checkpoints:
//! ```text
//! (db, ns, _meta, "cdc_checkpoints", consumer_id) → 10-byte versionstamp
//! ```
//!
//! Multiple consumers can operate independently (e.g. replication, cache
//! invalidation, analytics) by using different `consumer_id` values.

use crate::cdc::{read_cdc_entries, CdcEntry, Versionstamp};
use crate::db::PelagoDb;
use crate::Subspace;
use pelago_core::PelagoError;
use std::time::{Duration, Instant};

/// Configuration for a CDC consumer
#[derive(Clone, Debug)]
pub struct ConsumerConfig {
    /// Unique consumer identifier
    pub consumer_id: String,
    /// Database to consume from
    pub database: String,
    /// Namespace to consume from
    pub namespace: String,
    /// Maximum entries per batch
    pub batch_size: usize,
    /// How often to save checkpoint
    pub checkpoint_interval: Duration,
    /// How long to sleep when no entries available
    pub poll_interval: Duration,
}

impl ConsumerConfig {
    pub fn new(consumer_id: &str, database: &str, namespace: &str) -> Self {
        Self {
            consumer_id: consumer_id.to_string(),
            database: database.to_string(),
            namespace: namespace.to_string(),
            batch_size: 1000,
            checkpoint_interval: Duration::from_secs(5),
            poll_interval: Duration::from_millis(100),
        }
    }

    pub fn with_batch_size(mut self, size: usize) -> Self {
        self.batch_size = size;
        self
    }

    pub fn with_checkpoint_interval(mut self, interval: Duration) -> Self {
        self.checkpoint_interval = interval;
        self
    }
}

/// CDC consumer with high-water mark tracking and checkpoint persistence.
///
/// `poll_batch()` is read-only. Callers must acknowledge successful processing
/// via `ack_through()`/`ack_batch()` to advance the HWM. The HWM is
/// periodically checkpointed to FDB so consumption can resume after a crash.
pub struct CdcConsumer {
    config: ConsumerConfig,
    db: PelagoDb,
    hwm: Versionstamp,
    last_checkpoint: Instant,
}

impl CdcConsumer {
    /// Create a new consumer, loading its HWM from FDB (or starting from zero)
    pub async fn new(db: PelagoDb, config: ConsumerConfig) -> Result<Self, PelagoError> {
        let hwm = Self::load_hwm(&db, &config).await?;
        Ok(Self {
            config,
            db,
            hwm,
            last_checkpoint: Instant::now(),
        })
    }

    /// Load high-water mark from FDB checkpoint
    async fn load_hwm(db: &PelagoDb, config: &ConsumerConfig) -> Result<Versionstamp, PelagoError> {
        let key = Self::checkpoint_key(config);
        match db.get(&key).await? {
            Some(bytes) => Versionstamp::from_bytes(&bytes)
                .ok_or_else(|| PelagoError::Internal("Invalid CDC checkpoint data".into())),
            None => Ok(Versionstamp::zero()),
        }
    }

    /// Save current high-water mark to FDB
    async fn save_hwm(&mut self) -> Result<(), PelagoError> {
        let key = Self::checkpoint_key(&self.config);
        self.db.set(&key, self.hwm.to_bytes()).await?;
        self.last_checkpoint = Instant::now();
        Ok(())
    }

    /// Build the FDB key for this consumer's checkpoint
    fn checkpoint_key(config: &ConsumerConfig) -> Vec<u8> {
        Subspace::namespace(&config.database, &config.namespace)
            .meta()
            .pack()
            .add_string("cdc_checkpoints")
            .add_string(&config.consumer_id)
            .build()
            .to_vec()
    }

    /// Poll for the next batch of CDC entries after the current HWM.
    ///
    /// This method does not advance the HWM. Callers must acknowledge the
    /// batch with `ack_batch()` or `ack_through()` after successful processing.
    pub async fn poll_batch(&mut self) -> Result<Vec<(Versionstamp, CdcEntry)>, PelagoError> {
        let after = if self.hwm.is_zero() {
            None
        } else {
            Some(&self.hwm)
        };

        let entries = read_cdc_entries(
            &self.db,
            &self.config.database,
            &self.config.namespace,
            after,
            self.config.batch_size,
        )
        .await?;
        Ok(entries)
    }

    /// Acknowledge all entries up to (and including) `versionstamp`.
    ///
    /// Periodically persists the checkpoint based on `checkpoint_interval`.
    pub async fn ack_through(&mut self, versionstamp: &Versionstamp) -> Result<(), PelagoError> {
        if versionstamp > &self.hwm {
            self.hwm = versionstamp.clone();
        }

        if self.last_checkpoint.elapsed() >= self.config.checkpoint_interval {
            self.save_hwm().await?;
        }

        Ok(())
    }

    /// Acknowledge an entire batch returned by `poll_batch()`.
    pub async fn ack_batch(
        &mut self,
        entries: &[(Versionstamp, CdcEntry)],
    ) -> Result<(), PelagoError> {
        if let Some((last_vs, _)) = entries.last() {
            self.ack_through(last_vs).await?;
        }
        Ok(())
    }

    /// Force a checkpoint save (e.g. for graceful shutdown)
    pub async fn checkpoint(&mut self) -> Result<(), PelagoError> {
        self.save_hwm().await
    }

    /// Get current high-water mark
    pub fn hwm(&self) -> &Versionstamp {
        &self.hwm
    }

    /// Get the consumer configuration
    pub fn config(&self) -> &ConsumerConfig {
        &self.config
    }
}

/// Fetch all CDC consumer checkpoints for a namespace.
///
/// This is used by the CDC retention job to determine which entries are
/// safe to delete — only entries that all consumers have processed (i.e.
/// entries before the minimum checkpoint) can be removed.
pub async fn fetch_all_checkpoints(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
) -> Result<Vec<(String, Versionstamp)>, PelagoError> {
    let prefix = Subspace::namespace(database, namespace)
        .meta()
        .pack()
        .add_string("cdc_checkpoints")
        .build();

    let mut range_end = prefix.to_vec();
    range_end.push(0xFF);

    let results = db.get_range(prefix.as_ref(), &range_end, 1000).await?;

    let prefix_len = prefix.len();
    let mut checkpoints = Vec::new();
    for (key, value) in results {
        let consumer_id = extract_tuple_string(&key[prefix_len..]);
        if let Some(vs) = Versionstamp::from_bytes(&value) {
            checkpoints.push((consumer_id, vs));
        }
    }

    Ok(checkpoints)
}

/// Extract a tuple-encoded string from raw bytes.
///
/// Tuple format: 0x02 | bytes | 0x00 (with 0x00 0xFF escape for embedded nulls)
fn extract_tuple_string(data: &[u8]) -> String {
    if data.is_empty() || data[0] != 0x02 {
        return String::new();
    }

    let mut result = Vec::new();
    let mut i = 1;
    while i < data.len() {
        if data[i] == 0x00 {
            if i + 1 < data.len() && data[i + 1] == 0xFF {
                // Escaped null byte
                result.push(0x00);
                i += 2;
            } else {
                // End of string
                break;
            }
        } else {
            result.push(data[i]);
            i += 1;
        }
    }

    String::from_utf8(result).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_tuple_string_simple() {
        // 0x02 "hello" 0x00
        let mut data = vec![0x02];
        data.extend_from_slice(b"hello");
        data.push(0x00);
        assert_eq!(extract_tuple_string(&data), "hello");
    }

    #[test]
    fn test_extract_tuple_string_empty() {
        assert_eq!(extract_tuple_string(&[]), "");
        assert_eq!(extract_tuple_string(&[0x02, 0x00]), "");
    }

    #[test]
    fn test_extract_tuple_string_with_null_escape() {
        // 0x02 "a" 0x00 0xFF "b" 0x00
        let data = vec![0x02, b'a', 0x00, 0xFF, b'b', 0x00];
        assert_eq!(extract_tuple_string(&data), "a\0b");
    }

    #[test]
    fn test_consumer_config_defaults() {
        let config = ConsumerConfig::new("test_consumer", "mydb", "default");
        assert_eq!(config.consumer_id, "test_consumer");
        assert_eq!(config.batch_size, 1000);
        assert_eq!(config.checkpoint_interval, Duration::from_secs(5));
        assert_eq!(config.poll_interval, Duration::from_millis(100));
    }

    #[test]
    fn test_consumer_config_builder() {
        let config = ConsumerConfig::new("c1", "db", "ns")
            .with_batch_size(500)
            .with_checkpoint_interval(Duration::from_secs(10));
        assert_eq!(config.batch_size, 500);
        assert_eq!(config.checkpoint_interval, Duration::from_secs(10));
    }

    #[test]
    fn test_checkpoint_key_is_deterministic() {
        let config = ConsumerConfig::new("replication", "mydb", "default");
        let key1 = CdcConsumer::checkpoint_key(&config);
        let key2 = CdcConsumer::checkpoint_key(&config);
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_different_consumers_have_different_keys() {
        let config_a = ConsumerConfig::new("replication", "mydb", "default");
        let config_b = ConsumerConfig::new("analytics", "mydb", "default");
        let key_a = CdcConsumer::checkpoint_key(&config_a);
        let key_b = CdcConsumer::checkpoint_key(&config_b);
        assert_ne!(key_a, key_b);
    }
}
