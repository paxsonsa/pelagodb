//! Schema storage operations
//!
//! FDB Key Layout:
//! ```text
//! (db, ns, _schema, entity_type, "latest") → u32 version (as big-endian bytes)
//! (db, ns, _schema, entity_type, "v", version) → CBOR EntitySchema
//! ```
//!
//! Schema versioning:
//! - First registration creates version 1
//! - Each update increments version
//! - Old versions are preserved for migration support

use crate::cache::SchemaCache;
use crate::cdc::{CdcAccumulator, CdcOperation};
use crate::db::PelagoDb;
use crate::jobs::{JobStore, JobType};
use crate::namespace::enforce_namespace_schema_owner;
use crate::Subspace;
use bytes::Bytes;
use pelago_core::encoding::{decode_cbor, encode_cbor};
use pelago_core::schema::{EntitySchema, IndexType, OwnershipMode};
use pelago_core::PelagoError;
use std::sync::Arc;

/// Schema registry for managing entity type schemas
pub struct SchemaRegistry {
    db: PelagoDb,
    cache: Arc<SchemaCache>,
    site_id: String,
}

impl SchemaRegistry {
    pub fn new(db: PelagoDb, cache: Arc<SchemaCache>, site_id: String) -> Self {
        Self { db, cache, site_id }
    }

    /// Build key for the latest version pointer
    fn latest_version_key(subspace: &Subspace, entity_type: &str) -> Bytes {
        subspace
            .pack()
            .add_string(entity_type)
            .add_string("latest")
            .build()
    }

    /// Build key for a specific schema version
    fn version_key(subspace: &Subspace, entity_type: &str, version: u32) -> Bytes {
        subspace
            .pack()
            .add_string(entity_type)
            .add_string("v")
            .add_int(version as i64)
            .build()
    }

    /// Encode version number to bytes
    fn encode_version(version: u32) -> [u8; 4] {
        version.to_be_bytes()
    }

    /// Decode version number from bytes
    fn decode_version(bytes: &[u8]) -> Result<u32, PelagoError> {
        if bytes.len() != 4 {
            return Err(PelagoError::Internal(format!(
                "Invalid version bytes length: {}",
                bytes.len()
            )));
        }
        let arr: [u8; 4] = bytes.try_into().unwrap();
        Ok(u32::from_be_bytes(arr))
    }

    /// Validate a schema definition
    pub fn validate_schema(schema: &EntitySchema) -> Result<(), PelagoError> {
        // Name must not be empty
        if schema.name.is_empty() {
            return Err(PelagoError::SchemaValidation {
                message: "Schema name cannot be empty".to_string(),
            });
        }

        // Name must be alphanumeric with underscores
        if !schema.name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err(PelagoError::SchemaValidation {
                message: format!(
                    "Schema name '{}' contains invalid characters (only alphanumeric and underscore allowed)",
                    schema.name
                ),
            });
        }

        // Validate property names
        for prop_name in schema.properties.keys() {
            if prop_name.is_empty() {
                return Err(PelagoError::SchemaValidation {
                    message: "Property name cannot be empty".to_string(),
                });
            }
        }

        // Validate edge definitions
        for (edge_name, edge_def) in &schema.edges {
            if edge_name.is_empty() {
                return Err(PelagoError::SchemaValidation {
                    message: "Edge name cannot be empty".to_string(),
                });
            }

            // If sort_key is specified, it must be a valid edge property
            if let Some(ref sort_key) = edge_def.sort_key {
                if !edge_def.properties.contains_key(sort_key) {
                    return Err(PelagoError::SchemaValidation {
                        message: format!(
                            "Edge '{}' sort_key '{}' is not a defined edge property",
                            edge_name, sort_key
                        ),
                    });
                }
            }

            // Independent edge ownership is intentionally disabled in v1 to
            // keep single-owner conflict semantics deterministic.
            if edge_def.ownership == OwnershipMode::Independent {
                return Err(PelagoError::SchemaValidation {
                    message: format!(
                        "Edge '{}' uses ownership=independent, which is not supported in v1",
                        edge_name
                    ),
                });
            }
        }

        Ok(())
    }

    /// Register a new schema or update an existing one
    ///
    /// Returns the version number assigned to this schema
    pub async fn register_schema(
        &self,
        database: &str,
        namespace: &str,
        mut schema: EntitySchema,
    ) -> Result<u32, PelagoError> {
        // Validate schema first
        Self::validate_schema(&schema)?;
        enforce_namespace_schema_owner(&self.db, database, namespace, &self.site_id).await?;

        let subspace = Subspace::namespace(database, namespace).schema();
        let latest_key = Self::latest_version_key(&subspace, &schema.name);

        // Get current version or start at 0
        let current_version = match self.db.get(latest_key.as_ref()).await? {
            Some(bytes) => Self::decode_version(&bytes)?,
            None => 0,
        };

        let new_version = current_version + 1;
        schema.version = new_version;

        // Encode schema to CBOR
        let schema_bytes = encode_cbor(&schema)?;

        // Write in a single transaction (schema + CDC)
        let trx = self.db.create_transaction()?;
        let mut cdc = CdcAccumulator::new(&self.site_id);

        // Write version pointer
        trx.set(latest_key.as_ref(), &Self::encode_version(new_version));

        // Write versioned schema
        let version_key = Self::version_key(&subspace, &schema.name, new_version);
        trx.set(version_key.as_ref(), &schema_bytes);

        // Accumulate CDC operation
        cdc.push(CdcOperation::SchemaRegister {
            entity_type: schema.name.clone(),
            version: new_version,
            schema: schema.clone(),
        });

        // Flush CDC into same transaction
        cdc.flush(&trx, database, namespace)?;

        // Single atomic commit
        trx.commit()
            .await
            .map_err(|e| PelagoError::Internal(format!("Schema registration failed: {}", e)))?;

        // Update cache
        self.cache.insert(database, namespace, schema.clone()).await;

        // Auto-create IndexBackfill jobs for newly indexed properties on schema updates
        if current_version > 0 {
            if let Ok(Some(old_schema)) = self
                .get_schema_version(database, namespace, &schema.name, current_version)
                .await
            {
                let job_store = JobStore::new(self.db.clone());
                for (prop_name, new_def) in &schema.properties {
                    if new_def.index != IndexType::None {
                        let old_index = old_schema
                            .properties
                            .get(prop_name)
                            .map(|d| d.index)
                            .unwrap_or(IndexType::None);
                        if old_index == IndexType::None {
                            // New index on this property — create backfill job
                            let _ = job_store
                                .create_job(
                                    database,
                                    namespace,
                                    JobType::IndexBackfill {
                                        entity_type: schema.name.clone(),
                                        property_name: prop_name.clone(),
                                        index_type: new_def.index,
                                    },
                                )
                                .await;
                        }
                    }
                }
            }
        }

        Ok(new_version)
    }

    /// Apply a replicated schema registration without emitting CDC.
    ///
    /// Returns `true` when the local schema state changed.
    pub async fn apply_replica_schema_register(
        &self,
        database: &str,
        namespace: &str,
        mut schema: EntitySchema,
        expected_version: u32,
        origin_site_id: &str,
    ) -> Result<bool, PelagoError> {
        Self::validate_schema(&schema)?;
        enforce_namespace_schema_owner(&self.db, database, namespace, origin_site_id).await?;

        let subspace = Subspace::namespace(database, namespace).schema();
        let latest_key = Self::latest_version_key(&subspace, &schema.name);
        let current_version = match self.db.get(latest_key.as_ref()).await? {
            Some(bytes) => Self::decode_version(&bytes)?,
            None => 0,
        };

        if current_version >= expected_version {
            // Idempotent replay or stale event.
            return Ok(false);
        }

        if current_version + 1 != expected_version {
            return Err(PelagoError::SchemaMismatch {
                expected: current_version + 1,
                actual: expected_version,
            });
        }

        schema.version = expected_version;
        let schema_bytes = encode_cbor(&schema)?;

        let trx = self.db.create_transaction()?;
        trx.set(latest_key.as_ref(), &Self::encode_version(expected_version));
        let version_key = Self::version_key(&subspace, &schema.name, expected_version);
        trx.set(version_key.as_ref(), &schema_bytes);
        trx.commit().await.map_err(|e| {
            PelagoError::Internal(format!("Replicated schema register failed: {}", e))
        })?;

        self.cache.insert(database, namespace, schema.clone()).await;

        // Mirror local register behavior: create backfill jobs for newly-indexed fields.
        if current_version > 0 {
            if let Ok(Some(old_schema)) = self
                .get_schema_version(database, namespace, &schema.name, current_version)
                .await
            {
                let job_store = JobStore::new(self.db.clone());
                for (prop_name, new_def) in &schema.properties {
                    if new_def.index != IndexType::None {
                        let old_index = old_schema
                            .properties
                            .get(prop_name)
                            .map(|d| d.index)
                            .unwrap_or(IndexType::None);
                        if old_index == IndexType::None {
                            let _ = job_store
                                .create_job(
                                    database,
                                    namespace,
                                    JobType::IndexBackfill {
                                        entity_type: schema.name.clone(),
                                        property_name: prop_name.clone(),
                                        index_type: new_def.index,
                                    },
                                )
                                .await;
                        }
                    }
                }
            }
        }

        Ok(true)
    }

    /// Get the current version of a schema
    pub async fn get_schema(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
    ) -> Result<Option<Arc<EntitySchema>>, PelagoError> {
        // Check cache first
        if let Some(schema) = self.cache.get(database, namespace, entity_type).await {
            return Ok(Some(schema));
        }

        // Not in cache, fetch from FDB
        let subspace = Subspace::namespace(database, namespace).schema();
        let latest_key = Self::latest_version_key(&subspace, entity_type);

        let version = match self.db.get(latest_key.as_ref()).await? {
            Some(bytes) => Self::decode_version(&bytes)?,
            None => return Ok(None),
        };

        // Fetch the versioned schema
        let version_key = Self::version_key(&subspace, entity_type, version);
        let schema_bytes = match self.db.get(version_key.as_ref()).await? {
            Some(bytes) => bytes,
            None => {
                return Err(PelagoError::Internal(format!(
                    "Schema version {} for {} exists but data is missing",
                    version, entity_type
                )))
            }
        };

        let schema: EntitySchema = decode_cbor(&schema_bytes)?;

        // Cache it
        self.cache.insert(database, namespace, schema.clone()).await;

        Ok(Some(Arc::new(schema)))
    }

    /// Get a specific version of a schema
    pub async fn get_schema_version(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        version: u32,
    ) -> Result<Option<EntitySchema>, PelagoError> {
        let subspace = Subspace::namespace(database, namespace).schema();
        let version_key = Self::version_key(&subspace, entity_type, version);

        match self.db.get(version_key.as_ref()).await? {
            Some(bytes) => {
                let schema: EntitySchema = decode_cbor(&bytes)?;
                Ok(Some(schema))
            }
            None => Ok(None),
        }
    }

    /// List all schemas in a namespace
    pub async fn list_schemas(
        &self,
        database: &str,
        namespace: &str,
    ) -> Result<Vec<String>, PelagoError> {
        let subspace = Subspace::namespace(database, namespace).schema();

        // Scan all keys in the schema subspace
        let range_start = subspace.prefix().to_vec();
        let range_end = subspace.range_end().to_vec();

        let results = self.db.get_range(&range_start, &range_end, 1000).await?;

        // Extract unique entity types from "latest" keys
        // Key format: (prefix)(entity_type)(0x00)(0x02)latest(0x00)
        let mut entity_types = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for (key, _value) in results {
            // Check if this is a "latest" key by looking for the pattern
            if let Some(entity_type) = Self::extract_entity_type_from_latest_key(&subspace, &key) {
                if seen.insert(entity_type.clone()) {
                    entity_types.push(entity_type);
                }
            }
        }

        Ok(entity_types)
    }

    /// Extract entity type from a "latest" key
    fn extract_entity_type_from_latest_key(subspace: &Subspace, key: &[u8]) -> Option<String> {
        let prefix = subspace.prefix();
        if !key.starts_with(prefix) {
            return None;
        }

        // After prefix, we have: (0x02)(entity_type bytes)(0x00)(0x02)latest(0x00)
        let remainder = &key[prefix.len()..];

        // Check for string type marker
        if remainder.is_empty() || remainder[0] != 0x02 {
            return None;
        }

        // Find the null terminator for entity_type
        let entity_bytes = &remainder[1..];
        let null_pos = entity_bytes.iter().position(|&b| b == 0x00)?;
        let entity_type = std::str::from_utf8(&entity_bytes[..null_pos]).ok()?;

        // Check if the next part is "latest"
        let after_entity = &entity_bytes[null_pos + 1..];
        if after_entity.len() < 2 || after_entity[0] != 0x02 {
            return None;
        }

        let latest_start = &after_entity[1..];
        if latest_start.starts_with(b"latest\x00") {
            Some(entity_type.to_string())
        } else {
            None
        }
    }

    /// Get the latest version number for a schema
    pub async fn get_latest_version(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
    ) -> Result<Option<u32>, PelagoError> {
        let subspace = Subspace::namespace(database, namespace).schema();
        let latest_key = Self::latest_version_key(&subspace, entity_type);

        match self.db.get(latest_key.as_ref()).await? {
            Some(bytes) => Ok(Some(Self::decode_version(&bytes)?)),
            None => Ok(None),
        }
    }

    /// Invalidate a cached schema (force reload on next access)
    pub async fn invalidate_cache(&self, database: &str, namespace: &str, entity_type: &str) {
        self.cache
            .invalidate(database, namespace, entity_type)
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pelago_core::schema::{EdgeDef, EdgeTarget, PropertyDef};
    use pelago_core::PropertyType;

    #[test]
    fn test_validate_valid_schema() {
        let schema = EntitySchema::new("Person")
            .with_property("name", PropertyDef::new(PropertyType::String).required())
            .with_property("age", PropertyDef::new(PropertyType::Int))
            .with_edge("WORKS_AT", EdgeDef::new(EdgeTarget::specific("Company")));

        assert!(SchemaRegistry::validate_schema(&schema).is_ok());
    }

    #[test]
    fn test_validate_empty_name() {
        let schema = EntitySchema::new("");
        let result = SchemaRegistry::validate_schema(&schema);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot be empty"));
    }

    #[test]
    fn test_validate_invalid_name_chars() {
        let schema = EntitySchema::new("Person-Type");
        let result = SchemaRegistry::validate_schema(&schema);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("invalid characters"));
    }

    #[test]
    fn test_validate_invalid_sort_key() {
        let mut schema = EntitySchema::new("Person");
        let mut edge = EdgeDef::new(EdgeTarget::specific("Company"));
        edge.sort_key = Some("nonexistent".to_string());
        schema.edges.insert("WORKS_AT".to_string(), edge);

        let result = SchemaRegistry::validate_schema(&schema);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not a defined edge property"));
    }

    #[test]
    fn test_validate_valid_sort_key() {
        let schema = EntitySchema::new("Person").with_edge(
            "WORKS_AT",
            EdgeDef::new(EdgeTarget::specific("Company"))
                .with_sort_key("since")
                .with_property("since", PropertyDef::new(PropertyType::Timestamp)),
        );

        assert!(SchemaRegistry::validate_schema(&schema).is_ok());
    }

    #[test]
    fn test_key_generation() {
        let subspace = Subspace::namespace("db", "ns").schema();

        let latest_key = SchemaRegistry::latest_version_key(&subspace, "Person");
        let version_key = SchemaRegistry::version_key(&subspace, "Person", 1);

        // Keys should be different
        assert_ne!(latest_key, version_key);

        // Both should start with the schema subspace prefix
        assert!(latest_key.starts_with(subspace.prefix()));
        assert!(version_key.starts_with(subspace.prefix()));
    }

    #[test]
    fn test_version_encoding() {
        for v in [0u32, 1, 100, 1000, u32::MAX] {
            let encoded = SchemaRegistry::encode_version(v);
            let decoded = SchemaRegistry::decode_version(&encoded).unwrap();
            assert_eq!(v, decoded);
        }
    }
}
