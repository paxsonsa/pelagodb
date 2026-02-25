//! Durable watch query state metadata.

use crate::cdc::Versionstamp;
use crate::db::PelagoDb;
use crate::Subspace;
use pelago_core::encoding::{decode_cbor, encode_cbor};
use pelago_core::PelagoError;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryWatchState {
    pub state_id: String,
    pub last_position: Versionstamp,
    pub matched_nodes: Vec<String>,
    pub updated_at: i64,
}

pub async fn upsert_query_watch_state(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
    state_id: &str,
    last_position: &Versionstamp,
    matched_nodes: &HashSet<String>,
) -> Result<(), PelagoError> {
    let mut matched_nodes_vec: Vec<String> = matched_nodes.iter().cloned().collect();
    matched_nodes_vec.sort();

    let state = QueryWatchState {
        state_id: state_id.to_string(),
        last_position: last_position.clone(),
        matched_nodes: matched_nodes_vec,
        updated_at: now_micros(),
    };
    db.set(
        &query_watch_state_key(database, namespace, state_id),
        &encode_cbor(&state)?,
    )
    .await
}

pub async fn get_query_watch_state(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
    state_id: &str,
) -> Result<Option<QueryWatchState>, PelagoError> {
    match db
        .get(&query_watch_state_key(database, namespace, state_id))
        .await?
    {
        Some(bytes) => Ok(Some(decode_cbor::<QueryWatchState>(&bytes)?)),
        None => Ok(None),
    }
}

pub async fn delete_query_watch_state(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
    state_id: &str,
) -> Result<bool, PelagoError> {
    let key = query_watch_state_key(database, namespace, state_id);
    let exists = db.get(&key).await?.is_some();
    if exists {
        db.clear(&key).await?;
    }
    Ok(exists)
}

/// Delete query-watch states older than `retention_secs`.
///
/// Returns the number of deleted records in this sweep.
pub async fn cleanup_query_watch_states(
    db: &PelagoDb,
    retention_secs: u64,
    batch_limit: usize,
) -> Result<usize, PelagoError> {
    let retention_micros = (retention_secs as i64).saturating_mul(1_000_000);
    let cutoff = now_micros().saturating_sub(retention_micros);
    let batch_limit = batch_limit.max(1);
    let page_size = batch_limit.saturating_mul(8).max(64);
    let subspace = query_watch_state_root_subspace();
    let start = subspace.prefix().to_vec();
    let end = subspace.range_end().to_vec();
    let mut scan_start = start;
    let mut to_delete = Vec::with_capacity(batch_limit);

    loop {
        let rows = db.get_range(&scan_start, &end, page_size).await?;
        if rows.is_empty() {
            break;
        }

        let mut next_start = None;
        for (key, value) in rows {
            next_start = Some(next_scan_start(&key));

            let delete = match decode_cbor::<QueryWatchState>(&value) {
                Ok(state) => is_expired(state.updated_at, cutoff),
                Err(_) => true, // Corrupt/unreadable state is cleaned up opportunistically.
            };

            if delete {
                to_delete.push(key);
                if to_delete.len() >= batch_limit {
                    break;
                }
            }
        }

        if to_delete.len() >= batch_limit {
            break;
        }

        let Some(next_start) = next_start else {
            break;
        };
        if next_start >= end {
            break;
        }
        scan_start = next_start;
    }

    if to_delete.is_empty() {
        return Ok(0);
    }

    let trx = db.create_transaction()?;
    let mut deleted = 0usize;
    for key in to_delete {
        trx.clear(&key);
        deleted += 1;
    }
    trx.commit().await.map_err(|e| {
        PelagoError::Internal(format!("watch state retention commit failed: {}", e))
    })?;
    Ok(deleted)
}

fn query_watch_state_key(database: &str, namespace: &str, state_id: &str) -> Vec<u8> {
    query_watch_state_subspace(database, namespace)
        .pack()
        .add_string(state_id)
        .build()
        .to_vec()
}

fn query_watch_state_root_subspace() -> Subspace {
    Subspace::system().child("watch").child("qstate")
}

fn query_watch_state_subspace(database: &str, namespace: &str) -> Subspace {
    query_watch_state_root_subspace()
        .child(database)
        .child(namespace)
}

fn now_micros() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64
}

fn is_expired(updated_at: i64, cutoff: i64) -> bool {
    updated_at <= cutoff
}

fn next_scan_start(key: &[u8]) -> Vec<u8> {
    let mut next = Vec::with_capacity(key.len() + 1);
    next.extend_from_slice(key);
    next.push(0);
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_watch_state_key_shape() {
        let key = query_watch_state_key("db", "ns", "state-a");
        let prefix = query_watch_state_subspace("db", "ns").prefix().to_vec();
        assert!(key.starts_with(&prefix));
    }

    #[test]
    fn test_is_expired_cutoff_inclusive() {
        assert!(is_expired(10, 10));
        assert!(is_expired(9, 10));
        assert!(!is_expired(11, 10));
    }

    #[test]
    fn test_next_scan_start_appends_nul_byte() {
        assert_eq!(next_scan_start(b"abc"), b"abc\0");
    }
}
