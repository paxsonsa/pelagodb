//! Security metadata: policies and audit events.

use crate::db::PelagoDb;
use pelago_core::encoding::{decode_cbor, encode_cbor};
use pelago_core::PelagoError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const POLICY_PREFIX: &str = "_sys:policies:";
const AUDIT_PREFIX: &str = "_sys:audit:";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyPermission {
    pub database: String,
    pub namespace: String,
    pub entity_type: String,
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthPolicy {
    pub policy_id: String,
    pub principal_id: String,
    pub permissions: Vec<PolicyPermission>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRecord {
    pub event_id: String,
    pub timestamp: i64,
    pub principal_id: String,
    pub action: String,
    pub resource: String,
    pub allowed: bool,
    pub reason: String,
    pub metadata: std::collections::HashMap<String, String>,
}

pub async fn upsert_policy(db: &PelagoDb, policy: &AuthPolicy) -> Result<(), PelagoError> {
    db.set(&policy_key(&policy.policy_id), &encode_cbor(policy)?)
        .await
}

pub async fn get_policy(db: &PelagoDb, policy_id: &str) -> Result<Option<AuthPolicy>, PelagoError> {
    match db.get(&policy_key(policy_id)).await? {
        Some(bytes) => Ok(Some(decode_cbor(&bytes)?)),
        None => Ok(None),
    }
}

pub async fn delete_policy(db: &PelagoDb, policy_id: &str) -> Result<bool, PelagoError> {
    let key = policy_key(policy_id);
    let exists = db.get(&key).await?.is_some();
    if exists {
        db.clear(&key).await?;
    }
    Ok(exists)
}

pub async fn list_policies(
    db: &PelagoDb,
    principal_filter: Option<&str>,
) -> Result<Vec<AuthPolicy>, PelagoError> {
    let start = POLICY_PREFIX.as_bytes().to_vec();
    let mut end = start.clone();
    end.push(0xFF);
    let rows = db.get_range(&start, &end, 10_000).await?;

    let mut out = Vec::new();
    for (_, value) in rows {
        let policy: AuthPolicy = decode_cbor(&value)?;
        if principal_filter
            .map(|p| p == policy.principal_id)
            .unwrap_or(true)
        {
            out.push(policy);
        }
    }
    out.sort_by(|a, b| a.policy_id.cmp(&b.policy_id));
    Ok(out)
}

pub async fn check_permission(
    db: &PelagoDb,
    principal_id: &str,
    action: &str,
    database: &str,
    namespace: &str,
    entity_type: &str,
) -> Result<bool, PelagoError> {
    let policies = list_policies(db, Some(principal_id)).await?;
    Ok(policies.iter().any(|p| {
        p.permissions.iter().any(|perm| {
            field_match(&perm.database, database)
                && field_match(&perm.namespace, namespace)
                && field_match(&perm.entity_type, entity_type)
                && perm.actions.iter().any(|a| action_match(a, action))
        })
    }))
}

pub async fn append_audit_record(
    db: &PelagoDb,
    mut record: AuditRecord,
) -> Result<AuditRecord, PelagoError> {
    if record.event_id.is_empty() {
        record.event_id = Uuid::now_v7().to_string();
    }
    if record.timestamp == 0 {
        record.timestamp = now_micros();
    }
    db.set(
        &audit_key(record.timestamp, &record.event_id),
        &encode_cbor(&record)?,
    )
    .await?;
    Ok(record)
}

pub async fn query_audit_records(
    db: &PelagoDb,
    principal_id: Option<&str>,
    action: Option<&str>,
    from_ts: Option<i64>,
    to_ts: Option<i64>,
    limit: usize,
) -> Result<Vec<AuditRecord>, PelagoError> {
    let start = if let Some(ts) = from_ts {
        format!("{}{:020}:", AUDIT_PREFIX, ts).into_bytes()
    } else {
        AUDIT_PREFIX.as_bytes().to_vec()
    };

    let end = if let Some(ts) = to_ts {
        format!("{}{:020}:\u{10FFFF}", AUDIT_PREFIX, ts).into_bytes()
    } else {
        let mut end = AUDIT_PREFIX.as_bytes().to_vec();
        end.push(0xFF);
        end
    };

    let rows = db.get_range(&start, &end, limit.max(1) * 4).await?;
    let mut out = Vec::new();
    for (_, value) in rows {
        let record: AuditRecord = decode_cbor(&value)?;
        if principal_id
            .map(|p| p == record.principal_id.as_str())
            .unwrap_or(true)
            && action.map(|a| a == record.action.as_str()).unwrap_or(true)
        {
            out.push(record);
            if out.len() >= limit.max(1) {
                break;
            }
        }
    }
    out.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    Ok(out)
}

/// Delete audit records older than `retention_secs`.
///
/// Returns the number of deleted records in this sweep.
pub async fn cleanup_audit_records(
    db: &PelagoDb,
    retention_secs: u64,
    batch_limit: usize,
) -> Result<usize, PelagoError> {
    let now = now_micros();
    let cutoff = now - (retention_secs as i64 * 1_000_000);
    let start = AUDIT_PREFIX.as_bytes().to_vec();
    let end = format!("{}{:020}:\u{10FFFF}", AUDIT_PREFIX, cutoff).into_bytes();
    let rows = db.get_range(&start, &end, batch_limit.max(1)).await?;
    if rows.is_empty() {
        return Ok(0);
    }

    let trx = db.create_transaction()?;
    let mut deleted = 0usize;
    for (key, _value) in rows {
        trx.clear(&key);
        deleted += 1;
    }
    trx.commit()
        .await
        .map_err(|e| PelagoError::Internal(format!("audit retention commit failed: {}", e)))?;
    Ok(deleted)
}

fn policy_key(policy_id: &str) -> Vec<u8> {
    format!("{}{}", POLICY_PREFIX, policy_id).into_bytes()
}

fn audit_key(ts: i64, event_id: &str) -> Vec<u8> {
    format!("{}{:020}:{}", AUDIT_PREFIX, ts, event_id).into_bytes()
}

fn now_micros() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64
}

fn field_match(rule: &str, value: &str) -> bool {
    rule == "*" || rule.is_empty() || rule == value
}

fn action_match(rule: &str, action: &str) -> bool {
    rule == "*" || rule.eq_ignore_ascii_case(action)
}
