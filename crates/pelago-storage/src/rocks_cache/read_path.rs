use super::store::RocksCacheStore;
use crate::cdc::Versionstamp;
use crate::db::PelagoDb;
use crate::edge::StoredEdge;
use crate::node::StoredNode;
use pelago_core::PelagoError;
use std::sync::Arc;

/// Read consistency levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadConsistency {
    /// Always read from FDB (strongest guarantee)
    Strong,
    /// Read from cache if HWM >= required version, else fall back to FDB
    Session,
    /// Read from cache, fall back to FDB on miss
    Eventual,
}

pub struct CachedReadPath {
    #[allow(dead_code)]
    fdb: PelagoDb,
    cache: Arc<RocksCacheStore>,
}

impl CachedReadPath {
    pub fn new(fdb: PelagoDb, cache: Arc<RocksCacheStore>) -> Self {
        Self { fdb, cache }
    }

    pub async fn get_node(
        &self,
        db: &str,
        ns: &str,
        entity_type: &str,
        node_id: &str,
        consistency: ReadConsistency,
    ) -> Result<Option<StoredNode>, PelagoError> {
        match consistency {
            ReadConsistency::Strong => {
                // Always go to FDB - delegate to caller's normal FDB path
                Ok(None) // Signal: use FDB
            }
            ReadConsistency::Session => {
                // Session consistency requires the cache to be at least as fresh as the
                // current FDB read version.
                if !self.session_cache_fresh_for_current_read().await? {
                    return Ok(None);
                }
                self.cache.get_node(db, ns, entity_type, node_id)
            }
            ReadConsistency::Eventual => {
                // Cache-first, fall through on miss.
                self.cache.get_node(db, ns, entity_type, node_id)
            }
        }
    }

    /// Session cache freshness check.
    ///
    /// If `required_hwm` is `Some`, the cache must be at least that versionstamp.
    /// If `required_hwm` is `None`, a non-zero cache HWM is required.
    pub fn session_cache_fresh(
        &self,
        required_hwm: Option<&Versionstamp>,
    ) -> Result<bool, PelagoError> {
        let cache_hwm = self.cache.get_hwm()?;
        if cache_hwm.is_zero() {
            return Ok(false);
        }

        if let Some(required) = required_hwm {
            Ok(cache_hwm >= *required)
        } else {
            Ok(true)
        }
    }

    pub async fn session_cache_fresh_for_current_read(&self) -> Result<bool, PelagoError> {
        let read_version = self.fdb.get_read_version().await?;
        let cache_hwm = self.cache.get_hwm()?;
        if cache_hwm.is_zero() {
            return Ok(false);
        }

        let cache_tx_version = versionstamp_tx_version(&cache_hwm);
        Ok(cache_tx_version >= read_version as u64)
    }

    pub fn get_hwm(&self) -> Result<Versionstamp, PelagoError> {
        self.cache.get_hwm()
    }

    pub fn set_hwm(&self, vs: &Versionstamp) -> Result<(), PelagoError> {
        self.cache.set_hwm(vs)
    }

    pub async fn list_edges_cached(
        &self,
        db: &str,
        ns: &str,
        entity_type: &str,
        node_id: &str,
        label: Option<&str>,
        consistency: ReadConsistency,
    ) -> Result<Vec<StoredEdge>, PelagoError> {
        match consistency {
            ReadConsistency::Strong => Ok(Vec::new()), // Signal: fallback to FDB caller path
            ReadConsistency::Session => {
                if !self.session_cache_fresh_for_current_read().await? {
                    return Ok(Vec::new());
                }
                self.cache
                    .list_edges_cached(db, ns, entity_type, node_id, label)
            }
            ReadConsistency::Eventual => {
                self.cache
                    .list_edges_cached(db, ns, entity_type, node_id, label)
            }
        }
    }

    pub fn cache(&self) -> &RocksCacheStore {
        &self.cache
    }
}

fn versionstamp_tx_version(vs: &Versionstamp) -> u64 {
    let bytes = vs.to_bytes();
    let mut tx = [0u8; 8];
    tx.copy_from_slice(&bytes[..8]);
    u64::from_be_bytes(tx)
}
