use super::store::RocksCacheStore;
use crate::cdc::Versionstamp;
use crate::db::PelagoDb;
use crate::edge::StoredEdge;
use crate::node::StoredNode;
use metrics::{counter, histogram};
use pelago_core::PelagoError;
use std::sync::Arc;
use std::time::Instant;
use tracing::warn;

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

impl ReadConsistency {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Strong => "strong",
            Self::Session => "session",
            Self::Eventual => "eventual",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheFallbackReason {
    StrongForcedFdb,
    CacheCold,
    StaleHwm,
    StaleFreshnessBudget,
    KeyAbsent,
    DecodeError,
}

impl CacheFallbackReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StrongForcedFdb => "strong_forced_fdb",
            Self::CacheCold => "cold",
            Self::StaleHwm => "stale_hwm",
            Self::StaleFreshnessBudget => "stale_freshness_budget",
            Self::KeyAbsent => "key_absent",
            Self::DecodeError => "decode_error",
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CacheFreshnessBudgets {
    pub eventual_max_lag_ms: Option<u64>,
    pub session_max_lag_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct CacheLookup<T> {
    value: Option<T>,
    fallback_reason: Option<CacheFallbackReason>,
}

impl<T> CacheLookup<T> {
    pub fn hit(value: T) -> Self {
        Self {
            value: Some(value),
            fallback_reason: None,
        }
    }

    pub fn miss(reason: CacheFallbackReason) -> Self {
        Self {
            value: None,
            fallback_reason: Some(reason),
        }
    }

    pub fn value(self) -> Option<T> {
        self.value
    }

    pub fn fallback_reason(&self) -> Option<CacheFallbackReason> {
        self.fallback_reason
    }

    pub fn is_hit(&self) -> bool {
        self.value.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionCacheState {
    Fresh,
    Cold,
    StaleHwm,
}

impl SessionCacheState {
    fn fallback_reason(self) -> CacheFallbackReason {
        match self {
            Self::Fresh => CacheFallbackReason::KeyAbsent,
            Self::Cold => CacheFallbackReason::CacheCold,
            Self::StaleHwm => CacheFallbackReason::StaleHwm,
        }
    }
}

pub struct CachedReadPath {
    #[allow(dead_code)]
    fdb: PelagoDb,
    cache: Arc<RocksCacheStore>,
    freshness_budgets: CacheFreshnessBudgets,
}

impl CachedReadPath {
    pub fn new(
        fdb: PelagoDb,
        cache: Arc<RocksCacheStore>,
        freshness_budgets: CacheFreshnessBudgets,
    ) -> Self {
        Self {
            fdb,
            cache,
            freshness_budgets,
        }
    }

    pub async fn get_node(
        &self,
        db: &str,
        ns: &str,
        entity_type: &str,
        node_id: &str,
        consistency: ReadConsistency,
        required_read_version: Option<i64>,
    ) -> Result<CacheLookup<StoredNode>, PelagoError> {
        let started = Instant::now();
        let lookup = match consistency {
            ReadConsistency::Strong => {
                // Always go to FDB - delegate to caller's normal FDB path
                CacheLookup::miss(CacheFallbackReason::StrongForcedFdb)
            }
            ReadConsistency::Session => {
                // Session consistency requires the cache to be at least as fresh as the
                // current FDB read version.
                let state = self
                    .session_cache_state_for_read_version(required_read_version)
                    .await?;
                if state != SessionCacheState::Fresh {
                    return Ok(self.observe_lookup(
                        "get_node",
                        consistency,
                        CacheLookup::miss(state.fallback_reason()),
                        started,
                    ));
                }
                if let Some(reason) = self.freshness_budget_violation(consistency)? {
                    return Ok(self.observe_lookup(
                        "get_node",
                        consistency,
                        CacheLookup::miss(reason),
                        started,
                    ));
                }
                match self.cache.get_node(db, ns, entity_type, node_id) {
                    Ok(Some(node)) => CacheLookup::hit(node),
                    Ok(None) => CacheLookup::miss(CacheFallbackReason::KeyAbsent),
                    Err(err) => {
                        warn!(
                            error = %err,
                            db,
                            ns,
                            entity_type,
                            node_id,
                            "cache decode/read failed in get_node; falling back to fdb"
                        );
                        CacheLookup::miss(CacheFallbackReason::DecodeError)
                    }
                }
            }
            ReadConsistency::Eventual => {
                if let Some(reason) = self.freshness_budget_violation(consistency)? {
                    return Ok(self.observe_lookup(
                        "get_node",
                        consistency,
                        CacheLookup::miss(reason),
                        started,
                    ));
                }
                // Cache-first, fall through on miss.
                match self.cache.get_node(db, ns, entity_type, node_id) {
                    Ok(Some(node)) => CacheLookup::hit(node),
                    Ok(None) => CacheLookup::miss(CacheFallbackReason::KeyAbsent),
                    Err(err) => {
                        warn!(
                            error = %err,
                            db,
                            ns,
                            entity_type,
                            node_id,
                            "cache decode/read failed in get_node; falling back to fdb"
                        );
                        CacheLookup::miss(CacheFallbackReason::DecodeError)
                    }
                }
            }
        };

        Ok(self.observe_lookup("get_node", consistency, lookup, started))
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
        Ok(self.session_cache_state_for_read_version(None).await? == SessionCacheState::Fresh)
    }

    pub async fn session_cache_fresh_for_read_version(
        &self,
        required_read_version: Option<i64>,
    ) -> Result<bool, PelagoError> {
        Ok(self
            .session_cache_state_for_read_version(required_read_version)
            .await?
            == SessionCacheState::Fresh)
    }

    async fn session_cache_state_for_read_version(
        &self,
        required_read_version: Option<i64>,
    ) -> Result<SessionCacheState, PelagoError> {
        let read_version = if let Some(v) = required_read_version {
            v
        } else {
            self.fdb.get_read_version().await?
        };
        let cache_hwm = self.cache.get_hwm()?;
        if cache_hwm.is_zero() {
            return Ok(SessionCacheState::Cold);
        }

        let cache_tx_version = versionstamp_tx_version(&cache_hwm);
        if cache_tx_version >= read_version as u64 {
            Ok(SessionCacheState::Fresh)
        } else {
            Ok(SessionCacheState::StaleHwm)
        }
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
        required_read_version: Option<i64>,
    ) -> Result<CacheLookup<Vec<StoredEdge>>, PelagoError> {
        let started = Instant::now();
        let lookup = match consistency {
            ReadConsistency::Strong => CacheLookup::miss(CacheFallbackReason::StrongForcedFdb),
            ReadConsistency::Session => {
                let state = self
                    .session_cache_state_for_read_version(required_read_version)
                    .await?;
                if state != SessionCacheState::Fresh {
                    return Ok(self.observe_lookup(
                        "list_edges_outgoing",
                        consistency,
                        CacheLookup::miss(state.fallback_reason()),
                        started,
                    ));
                }
                if let Some(reason) = self.freshness_budget_violation(consistency)? {
                    return Ok(self.observe_lookup(
                        "list_edges_outgoing",
                        consistency,
                        CacheLookup::miss(reason),
                        started,
                    ));
                }
                match self
                    .cache
                    .list_edges_cached(db, ns, entity_type, node_id, label)
                {
                    Ok(edges) if !edges.is_empty() => CacheLookup::hit(edges),
                    Ok(_) => CacheLookup::miss(CacheFallbackReason::KeyAbsent),
                    Err(err) => {
                        warn!(
                            error = %err,
                            db,
                            ns,
                            entity_type,
                            node_id,
                            label = label.unwrap_or(""),
                            "cache decode/read failed in list_edges_cached; falling back to fdb"
                        );
                        CacheLookup::miss(CacheFallbackReason::DecodeError)
                    }
                }
            }
            ReadConsistency::Eventual => {
                if let Some(reason) = self.freshness_budget_violation(consistency)? {
                    return Ok(self.observe_lookup(
                        "list_edges_outgoing",
                        consistency,
                        CacheLookup::miss(reason),
                        started,
                    ));
                }
                match self
                    .cache
                    .list_edges_cached(db, ns, entity_type, node_id, label)
                {
                    Ok(edges) if !edges.is_empty() => CacheLookup::hit(edges),
                    Ok(_) => CacheLookup::miss(CacheFallbackReason::KeyAbsent),
                    Err(err) => {
                        warn!(
                            error = %err,
                            db,
                            ns,
                            entity_type,
                            node_id,
                            label = label.unwrap_or(""),
                            "cache decode/read failed in list_edges_cached; falling back to fdb"
                        );
                        CacheLookup::miss(CacheFallbackReason::DecodeError)
                    }
                }
            }
        };

        Ok(self.observe_lookup("list_edges_outgoing", consistency, lookup, started))
    }

    pub async fn list_incoming_edges_cached(
        &self,
        db: &str,
        ns: &str,
        entity_type: &str,
        node_id: &str,
        label: Option<&str>,
        consistency: ReadConsistency,
        required_read_version: Option<i64>,
    ) -> Result<CacheLookup<Vec<StoredEdge>>, PelagoError> {
        let started = Instant::now();
        let lookup = match consistency {
            ReadConsistency::Strong => CacheLookup::miss(CacheFallbackReason::StrongForcedFdb),
            ReadConsistency::Session => {
                let state = self
                    .session_cache_state_for_read_version(required_read_version)
                    .await?;
                if state != SessionCacheState::Fresh {
                    return Ok(self.observe_lookup(
                        "list_edges_incoming",
                        consistency,
                        CacheLookup::miss(state.fallback_reason()),
                        started,
                    ));
                }
                if let Some(reason) = self.freshness_budget_violation(consistency)? {
                    return Ok(self.observe_lookup(
                        "list_edges_incoming",
                        consistency,
                        CacheLookup::miss(reason),
                        started,
                    ));
                }
                match self
                    .cache
                    .list_incoming_edges_cached(db, ns, entity_type, node_id, label)
                {
                    Ok(edges) if !edges.is_empty() => CacheLookup::hit(edges),
                    Ok(_) => CacheLookup::miss(CacheFallbackReason::KeyAbsent),
                    Err(err) => {
                        warn!(
                            error = %err,
                            db,
                            ns,
                            entity_type,
                            node_id,
                            label = label.unwrap_or(""),
                            "cache decode/read failed in list_incoming_edges_cached; falling back to fdb"
                        );
                        CacheLookup::miss(CacheFallbackReason::DecodeError)
                    }
                }
            }
            ReadConsistency::Eventual => {
                if let Some(reason) = self.freshness_budget_violation(consistency)? {
                    return Ok(self.observe_lookup(
                        "list_edges_incoming",
                        consistency,
                        CacheLookup::miss(reason),
                        started,
                    ));
                }
                match self
                    .cache
                    .list_incoming_edges_cached(db, ns, entity_type, node_id, label)
                {
                    Ok(edges) if !edges.is_empty() => CacheLookup::hit(edges),
                    Ok(_) => CacheLookup::miss(CacheFallbackReason::KeyAbsent),
                    Err(err) => {
                        warn!(
                            error = %err,
                            db,
                            ns,
                            entity_type,
                            node_id,
                            label = label.unwrap_or(""),
                            "cache decode/read failed in list_incoming_edges_cached; falling back to fdb"
                        );
                        CacheLookup::miss(CacheFallbackReason::DecodeError)
                    }
                }
            }
        };

        Ok(self.observe_lookup("list_edges_incoming", consistency, lookup, started))
    }

    pub fn cache(&self) -> &RocksCacheStore {
        &self.cache
    }

    fn freshness_budget_violation(
        &self,
        consistency: ReadConsistency,
    ) -> Result<Option<CacheFallbackReason>, PelagoError> {
        let max_lag_ms = self.max_lag_budget_ms(consistency);
        if max_lag_ms.is_none() {
            return Ok(None);
        }

        let cache_hwm = self.cache.get_hwm()?;
        let cache_updated_at_ms = self.cache.get_hwm_updated_at_ms()?;
        Ok(freshness_budget_violation_reason(
            max_lag_ms,
            cache_hwm.is_zero(),
            cache_updated_at_ms,
            now_unix_ms(),
        ))
    }

    fn max_lag_budget_ms(&self, consistency: ReadConsistency) -> Option<u64> {
        match consistency {
            ReadConsistency::Strong => None,
            ReadConsistency::Session => self.freshness_budgets.session_max_lag_ms,
            ReadConsistency::Eventual => self.freshness_budgets.eventual_max_lag_ms,
        }
    }

    fn observe_lookup<T>(
        &self,
        operation: &'static str,
        consistency: ReadConsistency,
        lookup: CacheLookup<T>,
        started: Instant,
    ) -> CacheLookup<T> {
        let result = if lookup.is_hit() { "hit" } else { "miss" };
        counter!(
            "pelago.cache.lookup_total",
            "operation" => operation,
            "consistency" => consistency.as_str(),
            "result" => result
        )
        .increment(1);
        histogram!(
            "pelago.cache.lookup_latency_ms",
            "operation" => operation,
            "consistency" => consistency.as_str(),
            "result" => result
        )
        .record(started.elapsed().as_secs_f64() * 1000.0);

        if let Some(reason) = lookup.fallback_reason() {
            counter!(
                "pelago.cache.fallback_total",
                "operation" => operation,
                "consistency" => consistency.as_str(),
                "reason" => reason.as_str()
            )
            .increment(1);
        }

        lookup
    }
}

fn versionstamp_tx_version(vs: &Versionstamp) -> u64 {
    let bytes = vs.to_bytes();
    let mut tx = [0u8; 8];
    tx.copy_from_slice(&bytes[..8]);
    u64::from_be_bytes(tx)
}

fn freshness_budget_violation_reason(
    max_lag_ms: Option<u64>,
    cache_hwm_is_zero: bool,
    cache_updated_at_ms: Option<u64>,
    now_ms: u64,
) -> Option<CacheFallbackReason> {
    let max_lag_ms = max_lag_ms?;
    if cache_hwm_is_zero {
        return Some(CacheFallbackReason::CacheCold);
    }

    let Some(cache_updated_at_ms) = cache_updated_at_ms else {
        return Some(CacheFallbackReason::StaleFreshnessBudget);
    };
    if now_ms.saturating_sub(cache_updated_at_ms) > max_lag_ms {
        Some(CacheFallbackReason::StaleFreshnessBudget)
    } else {
        None
    }
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{freshness_budget_violation_reason, CacheFallbackReason};

    #[test]
    fn freshness_budget_disabled_never_forces_fallback() {
        let reason = freshness_budget_violation_reason(None, false, Some(10), 25);
        assert!(reason.is_none());
    }

    #[test]
    fn freshness_budget_marks_zero_hwm_as_cold() {
        let reason = freshness_budget_violation_reason(Some(500), true, Some(10), 25);
        assert_eq!(reason, Some(CacheFallbackReason::CacheCold));
    }

    #[test]
    fn freshness_budget_marks_stale_cache() {
        let reason = freshness_budget_violation_reason(Some(500), false, Some(1_000), 1_800);
        assert_eq!(reason, Some(CacheFallbackReason::StaleFreshnessBudget));
    }

    #[test]
    fn freshness_budget_requires_projection_timestamp_when_enabled() {
        let reason = freshness_budget_violation_reason(Some(500), false, None, 1_800);
        assert_eq!(reason, Some(CacheFallbackReason::StaleFreshnessBudget));
    }
}
