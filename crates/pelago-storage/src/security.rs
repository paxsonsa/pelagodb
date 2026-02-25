//! Security metadata: policies and audit events.

use crate::db::PelagoDb;
use crate::Subspace;
use pelago_core::encoding::{decode_cbor, encode_cbor};
use pelago_core::PelagoError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyPermission {
    pub database: String,
    pub namespace: String,
    pub entity_type: String,
    pub actions: Vec<String>,
    #[serde(default)]
    pub path: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthTokenSession {
    pub access_token: String,
    pub refresh_token: String,
    pub principal_id: String,
    pub principal_type: String,
    pub roles: Vec<String>,
    pub issued_at: i64,
    pub expires_at: i64,
    pub revoked_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RefreshTokenBinding {
    access_token: String,
    expires_at: i64,
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
    let subspace = policy_subspace();
    let rows = db
        .get_range(subspace.prefix(), &subspace.range_end(), 10_000)
        .await?;

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
    let resource_path = build_resource_path(action, database, namespace, entity_type);
    let legacy_resource = format!("{}/{}/{}", database, namespace, entity_type);
    let policies = list_policies(db, Some(principal_id)).await?;
    Ok(policies.iter().any(|p| {
        p.permissions.iter().any(|perm| {
            let resource_match = if perm.path.trim().is_empty() {
                field_match(&perm.database, database)
                    && field_match(&perm.namespace, namespace)
                    && field_match(&perm.entity_type, entity_type)
            } else {
                path_match(&perm.path, &resource_path) || path_match(&perm.path, &legacy_resource)
            };
            resource_match && perm.actions.iter().any(|a| action_match(a, action))
        })
    }))
}

pub async fn upsert_token_session(
    db: &PelagoDb,
    session: &AuthTokenSession,
) -> Result<(), PelagoError> {
    let refresh = RefreshTokenBinding {
        access_token: session.access_token.clone(),
        expires_at: session.expires_at,
    };
    let trx = db.create_transaction()?;
    trx.set(
        &access_token_key(&session.access_token),
        &encode_cbor(session)?,
    );
    trx.set(
        &refresh_token_key(&session.refresh_token),
        &encode_cbor(&refresh)?,
    );
    trx.commit()
        .await
        .map(|_| ())
        .map_err(|e| PelagoError::Internal(format!("auth token commit failed: {}", e)))
}

pub async fn get_token_session_by_access(
    db: &PelagoDb,
    access_token: &str,
) -> Result<Option<AuthTokenSession>, PelagoError> {
    match db.get(&access_token_key(access_token)).await? {
        Some(bytes) => Ok(Some(decode_cbor(&bytes)?)),
        None => Ok(None),
    }
}

pub async fn get_token_session_by_refresh(
    db: &PelagoDb,
    refresh_token: &str,
) -> Result<Option<AuthTokenSession>, PelagoError> {
    let binding = match db.get(&refresh_token_key(refresh_token)).await? {
        Some(bytes) => decode_cbor::<RefreshTokenBinding>(&bytes)?,
        None => return Ok(None),
    };
    let Some(session) = get_token_session_by_access(db, &binding.access_token).await? else {
        return Ok(None);
    };
    if session.refresh_token != refresh_token {
        return Ok(None);
    }
    Ok(Some(session))
}

pub async fn revoke_access_token(db: &PelagoDb, access_token: &str) -> Result<bool, PelagoError> {
    let Some(mut session) = get_token_session_by_access(db, access_token).await? else {
        return Ok(false);
    };
    session.revoked_at = Some(now_secs());
    let trx = db.create_transaction()?;
    trx.set(
        &access_token_key(&session.access_token),
        &encode_cbor(&session)?,
    );
    trx.clear(&refresh_token_key(&session.refresh_token));
    trx.commit()
        .await
        .map_err(|e| PelagoError::Internal(format!("auth revoke commit failed: {}", e)))?;
    Ok(true)
}

pub async fn revoke_refresh_token(db: &PelagoDb, refresh_token: &str) -> Result<bool, PelagoError> {
    let key = refresh_token_key(refresh_token);
    let exists = db.get(&key).await?.is_some();
    if exists {
        db.clear(&key).await?;
    }
    Ok(exists)
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
    let start = audit_range_start(from_ts);
    let end = audit_range_end_inclusive(to_ts);

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
    let start = audit_subspace().prefix().to_vec();
    let end = audit_range_end_inclusive(Some(cutoff));
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
    policy_subspace()
        .pack()
        .add_string(policy_id)
        .build()
        .to_vec()
}

fn access_token_key(token: &str) -> Vec<u8> {
    auth_access_subspace()
        .pack()
        .add_string(token)
        .build()
        .to_vec()
}

fn refresh_token_key(token: &str) -> Vec<u8> {
    auth_refresh_subspace()
        .pack()
        .add_string(token)
        .build()
        .to_vec()
}

fn audit_key(ts: i64, event_id: &str) -> Vec<u8> {
    audit_subspace()
        .pack()
        .add_int(ts)
        .add_string(event_id)
        .build()
        .to_vec()
}

fn policy_subspace() -> Subspace {
    Subspace::system().child("policies")
}

fn auth_access_subspace() -> Subspace {
    Subspace::system().child("auth").child("access")
}

fn auth_refresh_subspace() -> Subspace {
    Subspace::system().child("auth").child("refresh")
}

fn audit_subspace() -> Subspace {
    Subspace::system().child("audit")
}

fn audit_range_start(from_ts: Option<i64>) -> Vec<u8> {
    let subspace = audit_subspace();
    match from_ts {
        Some(ts) => subspace.pack().add_int(ts).build().to_vec(),
        None => subspace.prefix().to_vec(),
    }
}

fn audit_range_end_inclusive(to_ts: Option<i64>) -> Vec<u8> {
    let subspace = audit_subspace();
    match to_ts {
        Some(ts) => {
            if ts == i64::MAX {
                subspace.range_end().to_vec()
            } else {
                subspace
                    .pack()
                    .add_int(ts.saturating_add(1))
                    .build()
                    .to_vec()
            }
        }
        None => subspace.range_end().to_vec(),
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn now_micros() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64
}

fn field_match(rule: &str, value: &str) -> bool {
    rule == "*" || rule == "+" || rule.is_empty() || rule == value
}

fn action_match(rule: &str, action: &str) -> bool {
    rule == "*" || rule.eq_ignore_ascii_case(action)
}

fn build_resource_path(action: &str, database: &str, namespace: &str, entity_type: &str) -> String {
    let subspace = action_subspace(action);
    if entity_type.is_empty() {
        format!("{}/{}/{}", database, namespace, subspace)
    } else {
        format!("{}/{}/{}/{}", database, namespace, subspace, entity_type)
    }
}

fn action_subspace(action: &str) -> &'static str {
    if action.starts_with("schema.") {
        "_schema"
    } else if action.starts_with("edge.") {
        "edge"
    } else if action.starts_with("auth.policy.") {
        "_sys/policies"
    } else if action.starts_with("auth.token.") {
        "_sys/tokens"
    } else if action.starts_with("replication.") {
        "_replication"
    } else {
        "data"
    }
}

fn path_match(rule: &str, value: &str) -> bool {
    let rule_parts: Vec<&str> = rule
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let value_parts: Vec<&str> = value
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    path_match_segments(&rule_parts, &value_parts)
}

fn path_match_segments(rule: &[&str], value: &[&str]) -> bool {
    if rule.is_empty() {
        return value.is_empty();
    }

    match rule[0] {
        "*" => {
            if value.is_empty() {
                false
            } else {
                path_match_segments(&rule[1..], &value[1..])
            }
        }
        "+" => {
            if value.is_empty() {
                return false;
            }
            for consumed in 1..=value.len() {
                if path_match_segments(&rule[1..], &value[consumed..]) {
                    return true;
                }
            }
            false
        }
        segment => {
            if value.first().copied() == Some(segment) {
                path_match_segments(&rule[1..], &value[1..])
            } else {
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_match_star() {
        assert!(path_match("prod/*/data/*", "prod/core/data/Person"));
        assert!(!path_match("prod/*/data/*", "prod/core/data"));
    }

    #[test]
    fn test_path_match_plus() {
        assert!(path_match("prod/+", "prod/core/data/Person/1_42"));
        assert!(!path_match("prod/+", "prod"));
    }

    #[test]
    fn test_path_match_with_sys_subspace() {
        assert!(path_match(
            "prod/core/_sys/policies/+",
            "prod/core/_sys/policies/policy-1"
        ));
    }
}
