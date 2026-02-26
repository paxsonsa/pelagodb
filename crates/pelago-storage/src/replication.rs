//! Replication and site-registry metadata primitives.

use crate::cdc::Versionstamp;
use crate::db::PelagoDb;
use crate::Subspace;
use pelago_core::encoding::{decode_cbor, encode_cbor};
use pelago_core::PelagoError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteClaim {
    pub site_id: String,
    pub site_name: String,
    pub claimed_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationPosition {
    pub remote_site_id: String,
    pub last_applied_versionstamp: Option<Versionstamp>,
    pub lag_events: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationLease {
    pub site_id: String,
    pub database: String,
    pub namespace: String,
    pub holder_id: String,
    pub epoch: u64,
    pub lease_expires_at: i64,
    pub updated_at: i64,
}

pub async fn claim_site(
    db: &PelagoDb,
    site_id: &str,
    site_name: &str,
) -> Result<SiteClaim, PelagoError> {
    let key = site_claim_key(site_id);
    if let Some(bytes) = db.get(&key).await? {
        let existing: SiteClaim = decode_cbor(&bytes)?;
        if existing.site_name != site_name {
            return Err(PelagoError::Internal(format!(
                "site id '{}' already claimed by '{}'",
                site_id, existing.site_name
            )));
        }
        return Ok(existing);
    }

    let claim = SiteClaim {
        site_id: site_id.to_string(),
        site_name: site_name.to_string(),
        claimed_at: now_micros(),
    };
    db.set(&key, &encode_cbor(&claim)?).await?;
    Ok(claim)
}

pub async fn list_sites(db: &PelagoDb) -> Result<Vec<SiteClaim>, PelagoError> {
    let subspace = site_claim_subspace();
    let rows = db
        .get_range(subspace.prefix(), &subspace.range_end(), 10_000)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for (_, value) in rows {
        out.push(decode_cbor::<SiteClaim>(&value)?);
    }
    out.sort_by(|a, b| a.site_id.cmp(&b.site_id));
    Ok(out)
}

/// Resolve a site identifier to canonical site_id.
///
/// `identifier` may be:
/// - exact `site_id`
/// - exact `site_name` alias (must resolve to exactly one site)
pub async fn resolve_site_identifier(
    db: &PelagoDb,
    identifier: &str,
) -> Result<String, PelagoError> {
    let sites = list_sites(db).await?;
    resolve_site_identifier_from_claims(&sites, identifier)
}

fn resolve_site_identifier_from_claims(
    sites: &[SiteClaim],
    identifier: &str,
) -> Result<String, PelagoError> {
    let identifier = identifier.trim();
    if identifier.is_empty() {
        return Err(PelagoError::InvalidValue {
            field: "site_id".to_string(),
            reason: "site identifier cannot be empty".to_string(),
        });
    }

    if let Some(site) = sites.iter().find(|s| s.site_id == identifier) {
        return Ok(site.site_id.clone());
    }

    let alias_matches: Vec<&SiteClaim> =
        sites.iter().filter(|s| s.site_name == identifier).collect();
    match alias_matches.as_slice() {
        [single] => Ok(single.site_id.clone()),
        [] => Err(PelagoError::InvalidValue {
            field: "site_id".to_string(),
            reason: format!("unknown site identifier '{}'", identifier),
        }),
        many => {
            let ids = many
                .iter()
                .map(|s| s.site_id.as_str())
                .collect::<Vec<_>>()
                .join(",");
            Err(PelagoError::InvalidValue {
                field: "site_id".to_string(),
                reason: format!(
                    "ambiguous site alias '{}'; matches site IDs [{}]; use site ID",
                    identifier, ids
                ),
            })
        }
    }
}

pub async fn update_replication_position(
    db: &PelagoDb,
    remote_site_id: &str,
    versionstamp: Option<Versionstamp>,
    lag_events: i64,
) -> Result<ReplicationPosition, PelagoError> {
    let pos = ReplicationPosition {
        remote_site_id: remote_site_id.to_string(),
        last_applied_versionstamp: versionstamp,
        lag_events,
        updated_at: now_micros(),
    };
    db.set(
        &replication_position_key(remote_site_id),
        &encode_cbor(&pos)?,
    )
    .await?;
    Ok(pos)
}

pub async fn get_replication_positions(
    db: &PelagoDb,
) -> Result<Vec<ReplicationPosition>, PelagoError> {
    let subspace = replication_position_subspace();
    let rows = db
        .get_range(subspace.prefix(), &subspace.range_end(), 10_000)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for (_, value) in rows {
        out.push(decode_cbor::<ReplicationPosition>(&value)?);
    }
    out.sort_by(|a, b| a.remote_site_id.cmp(&b.remote_site_id));
    Ok(out)
}

pub async fn update_replication_position_scoped(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
    remote_site_id: &str,
    versionstamp: Option<Versionstamp>,
    lag_events: i64,
) -> Result<ReplicationPosition, PelagoError> {
    let pos = ReplicationPosition {
        remote_site_id: remote_site_id.to_string(),
        last_applied_versionstamp: versionstamp,
        lag_events,
        updated_at: now_micros(),
    };
    db.set(
        &scoped_replication_position_key(database, namespace, remote_site_id),
        &encode_cbor(&pos)?,
    )
    .await?;
    Ok(pos)
}

pub async fn get_replication_positions_scoped(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
) -> Result<Vec<ReplicationPosition>, PelagoError> {
    let subspace = scoped_replication_position_subspace(database, namespace);
    let rows = db
        .get_range(subspace.prefix(), &subspace.range_end(), 10_000)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for (_, value) in rows {
        out.push(decode_cbor::<ReplicationPosition>(&value)?);
    }
    out.sort_by(|a, b| a.remote_site_id.cmp(&b.remote_site_id));
    Ok(out)
}

pub async fn get_replicator_lease(
    db: &PelagoDb,
    site_id: &str,
    database: &str,
    namespace: &str,
) -> Result<Option<ReplicationLease>, PelagoError> {
    let key = replicator_lease_key(site_id, database, namespace);
    let value = db.get(&key).await?;
    value
        .map(|bytes| decode_cbor::<ReplicationLease>(&bytes))
        .transpose()
}

/// Attempt to acquire or renew a replicator lease for a site/database/namespace scope.
///
/// Returns:
/// - `Some(lease)` when acquired/renewed by `holder_id`
/// - `None` when another non-expired holder currently owns the lease
pub async fn try_acquire_replicator_lease(
    db: &PelagoDb,
    site_id: &str,
    database: &str,
    namespace: &str,
    holder_id: &str,
    lease_ttl_ms: u64,
) -> Result<Option<ReplicationLease>, PelagoError> {
    let ttl_ms = lease_ttl_ms.max(1);
    let ttl_micros = (ttl_ms as i64).saturating_mul(1_000);
    let now = now_micros();
    let key = replicator_lease_key(site_id, database, namespace);
    let trx = db.create_transaction()?;

    let existing = trx
        .get(&key, false)
        .await
        .map_err(|e| PelagoError::Internal(format!("replication lease get failed: {}", e)))?;

    let mut epoch = 1u64;
    if let Some(bytes) = existing {
        let current: ReplicationLease = decode_cbor(&bytes)?;
        if current.holder_id != holder_id && current.lease_expires_at > now {
            return Ok(None);
        }
        epoch = if current.holder_id == holder_id {
            current.epoch
        } else {
            current.epoch.saturating_add(1)
        };
    }

    let lease = ReplicationLease {
        site_id: site_id.to_string(),
        database: database.to_string(),
        namespace: namespace.to_string(),
        holder_id: holder_id.to_string(),
        epoch,
        lease_expires_at: now.saturating_add(ttl_micros),
        updated_at: now,
    };

    trx.set(&key, &encode_cbor(&lease)?);
    trx.commit()
        .await
        .map_err(|e| PelagoError::Internal(format!("replication lease commit failed: {}", e)))?;

    Ok(Some(lease))
}

fn site_claim_key(site_id: &str) -> Vec<u8> {
    site_claim_subspace()
        .pack()
        .add_string(site_id)
        .build()
        .to_vec()
}

fn replication_position_key(remote_site_id: &str) -> Vec<u8> {
    replication_position_subspace()
        .pack()
        .add_string(remote_site_id)
        .build()
        .to_vec()
}

fn scoped_replication_position_key(
    database: &str,
    namespace: &str,
    remote_site_id: &str,
) -> Vec<u8> {
    scoped_replication_position_subspace(database, namespace)
        .pack()
        .add_string(remote_site_id)
        .build()
        .to_vec()
}

fn replicator_lease_key(site_id: &str, database: &str, namespace: &str) -> Vec<u8> {
    replicator_lease_subspace(site_id, database, namespace)
        .prefix()
        .to_vec()
}

fn site_claim_subspace() -> Subspace {
    Subspace::system().child("site_claim")
}

fn replication_position_subspace() -> Subspace {
    Subspace::system().child("repl_position")
}

fn scoped_replication_position_subspace(database: &str, namespace: &str) -> Subspace {
    Subspace::system()
        .child("repl_position_scoped")
        .child(database)
        .child(namespace)
}

fn replicator_lease_subspace(site_id: &str, database: &str, namespace: &str) -> Subspace {
    Subspace::system()
        .child("repl_lease")
        .child(site_id)
        .child(database)
        .child(namespace)
}

fn now_micros() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_prefers_exact_id_match() {
        let sites = vec![
            SiteClaim {
                site_id: "1".to_string(),
                site_name: "alpha".to_string(),
                claimed_at: 0,
            },
            SiteClaim {
                site_id: "2".to_string(),
                site_name: "1".to_string(),
                claimed_at: 0,
            },
        ];
        assert_eq!(
            resolve_site_identifier_from_claims(&sites, "1").expect("id should resolve"),
            "1".to_string()
        );
    }

    #[test]
    fn resolve_by_alias() {
        let sites = vec![SiteClaim {
            site_id: "7".to_string(),
            site_name: "eu-west".to_string(),
            claimed_at: 0,
        }];
        assert_eq!(
            resolve_site_identifier_from_claims(&sites, "eu-west").expect("alias should resolve"),
            "7".to_string()
        );
    }

    #[test]
    fn resolve_rejects_ambiguous_alias() {
        let sites = vec![
            SiteClaim {
                site_id: "1".to_string(),
                site_name: "prod".to_string(),
                claimed_at: 0,
            },
            SiteClaim {
                site_id: "2".to_string(),
                site_name: "prod".to_string(),
                claimed_at: 0,
            },
        ];
        let err = resolve_site_identifier_from_claims(&sites, "prod")
            .expect_err("ambiguous alias should fail");
        assert!(matches!(err, PelagoError::InvalidValue { .. }));
    }
}
