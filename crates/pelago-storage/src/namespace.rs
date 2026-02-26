//! Namespace settings and policy metadata.
//!
//! Stores control-plane settings for a namespace under:
//! ```text
//! (db, ns, _ns, settings) -> CBOR NamespaceSettings
//! ```

use crate::db::PelagoDb;
use crate::subspace::markers;
use crate::Subspace;
use pelago_core::encoding::{decode_cbor, encode_cbor};
use pelago_core::PelagoError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NamespaceSettings {
    pub database: String,
    pub namespace: String,
    pub schema_owner_site_id: Option<String>,
    pub epoch: u64,
    pub updated_at: i64,
}

impl NamespaceSettings {
    pub fn unrestricted(database: &str, namespace: &str) -> Self {
        Self {
            database: database.to_string(),
            namespace: namespace.to_string(),
            schema_owner_site_id: None,
            epoch: 0,
            updated_at: 0,
        }
    }

    pub fn enforce_schema_owner(&self, actor_site_id: &str) -> Result<(), PelagoError> {
        if let Some(owner_site_id) = self.schema_owner_site_id.as_deref() {
            if owner_site_id != actor_site_id {
                return Err(PelagoError::NamespaceSchemaOwnershipConflict {
                    database: self.database.clone(),
                    namespace: self.namespace.clone(),
                    owner_site_id: owner_site_id.to_string(),
                    actor_site_id: actor_site_id.to_string(),
                });
            }
        }
        Ok(())
    }
}

pub async fn get_namespace_settings(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
) -> Result<Option<NamespaceSettings>, PelagoError> {
    let key = namespace_settings_key(database, namespace);
    match db.get(&key).await? {
        Some(bytes) => Ok(Some(decode_cbor::<NamespaceSettings>(&bytes)?)),
        None => Ok(None),
    }
}

pub async fn enforce_namespace_schema_owner(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
    actor_site_id: &str,
) -> Result<(), PelagoError> {
    if let Some(settings) = get_namespace_settings(db, database, namespace).await? {
        settings.enforce_schema_owner(actor_site_id)?;
    }
    Ok(())
}

pub async fn set_namespace_schema_owner(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
    owner_site_id: Option<&str>,
) -> Result<NamespaceSettings, PelagoError> {
    let key = namespace_settings_key(database, namespace);
    let trx = db.create_transaction()?;
    let existing = trx
        .get(&key, false)
        .await
        .map_err(|e| PelagoError::Internal(format!("namespace settings get failed: {}", e)))?;

    let mut settings = match existing {
        Some(bytes) => decode_cbor::<NamespaceSettings>(&bytes)?,
        None => NamespaceSettings::unrestricted(database, namespace),
    };

    let normalized_owner = normalize_owner_site_id(owner_site_id);
    if settings.schema_owner_site_id != normalized_owner {
        settings.schema_owner_site_id = normalized_owner;
        settings.epoch = settings.epoch.saturating_add(1);
    }
    settings.updated_at = now_micros();

    trx.set(&key, &encode_cbor(&settings)?);
    trx.commit()
        .await
        .map_err(|e| PelagoError::Internal(format!("namespace settings commit failed: {}", e)))?;
    Ok(settings)
}

pub async fn transfer_namespace_schema_owner(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
    expected_owner_site_id: &str,
    target_owner_site_id: &str,
) -> Result<NamespaceSettings, PelagoError> {
    let expected = normalize_required_site_id(expected_owner_site_id, "expected_owner_site_id")?;
    let target = normalize_required_site_id(target_owner_site_id, "target_owner_site_id")?;

    let key = namespace_settings_key(database, namespace);
    let trx = db.create_transaction()?;
    let existing = trx
        .get(&key, false)
        .await
        .map_err(|e| PelagoError::Internal(format!("namespace settings get failed: {}", e)))?;

    let mut settings = match existing {
        Some(bytes) => decode_cbor::<NamespaceSettings>(&bytes)?,
        None => NamespaceSettings::unrestricted(database, namespace),
    };

    let owner = settings.schema_owner_site_id.clone().unwrap_or_default();
    if owner != expected {
        return Err(PelagoError::NamespaceSchemaOwnershipConflict {
            database: database.to_string(),
            namespace: namespace.to_string(),
            owner_site_id: if owner.is_empty() {
                "<unassigned>".to_string()
            } else {
                owner
            },
            actor_site_id: expected.to_string(),
        });
    }

    if expected != target {
        settings.schema_owner_site_id = Some(target.to_string());
        settings.epoch = settings.epoch.saturating_add(1);
        settings.updated_at = now_micros();
        trx.set(&key, &encode_cbor(&settings)?);
        trx.commit().await.map_err(|e| {
            PelagoError::Internal(format!("namespace transfer ownership commit failed: {}", e))
        })?;
    }

    Ok(settings)
}

fn namespace_settings_key(database: &str, namespace: &str) -> Vec<u8> {
    namespace_settings_subspace(database, namespace)
        .prefix()
        .to_vec()
}

fn namespace_settings_subspace(database: &str, namespace: &str) -> Subspace {
    Subspace::namespace(database, namespace)
        .child(markers::NS)
        .child("settings")
}

fn normalize_owner_site_id(owner_site_id: Option<&str>) -> Option<String> {
    owner_site_id
        .and_then(|site| {
            let trimmed = site.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
        .map(|site| site.to_string())
}

fn normalize_required_site_id(value: &str, field: &str) -> Result<String, PelagoError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(PelagoError::InvalidValue {
            field: field.to_string(),
            reason: "site id cannot be empty".to_string(),
        });
    }
    Ok(trimmed.to_string())
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
    fn test_enforce_schema_owner_allows_unrestricted_namespace() {
        let settings = NamespaceSettings::unrestricted("db", "ns");
        assert!(settings.enforce_schema_owner("2").is_ok());
    }

    #[test]
    fn test_enforce_schema_owner_rejects_non_owner() {
        let mut settings = NamespaceSettings::unrestricted("db", "ns");
        settings.schema_owner_site_id = Some("1".to_string());
        let err = settings
            .enforce_schema_owner("2")
            .expect_err("owner mismatch should fail");
        assert!(matches!(
            err,
            PelagoError::NamespaceSchemaOwnershipConflict { .. }
        ));
    }

    #[test]
    fn test_normalize_owner_site_id() {
        assert_eq!(normalize_owner_site_id(None), None);
        assert_eq!(normalize_owner_site_id(Some("   ")), None);
        assert_eq!(normalize_owner_site_id(Some("  7 ")), Some("7".to_string()));
    }
}
