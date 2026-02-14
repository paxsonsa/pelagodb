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

#[allow(unused_imports)] // Will be used when FDB operations are implemented
use crate::cache::SchemaCache;
use crate::Subspace;
#[allow(unused_imports)]
use bytes::Bytes;
#[allow(unused_imports)] // Will be used when FDB operations are implemented
use pelago_core::encoding::{decode_cbor, encode_cbor};
use pelago_core::schema::EntitySchema;
use pelago_core::PelagoError;
#[allow(unused_imports)]
use std::sync::Arc;

/// Schema registry for managing entity type schemas
pub struct SchemaRegistry {
    #[allow(dead_code)] // Will be used when FDB operations are implemented
    cache: Arc<SchemaCache>,
}

impl SchemaRegistry {
    pub fn new(cache: Arc<SchemaCache>) -> Self {
        Self { cache }
    }

    /// Build key for the latest version pointer
    #[allow(dead_code)] // Will be used when FDB operations are implemented
    fn latest_version_key(subspace: &Subspace, entity_type: &str) -> Bytes {
        subspace
            .pack()
            .add_string(entity_type)
            .add_string("latest")
            .build()
    }

    /// Build key for a specific schema version
    #[allow(dead_code)] // Will be used when FDB operations are implemented
    fn version_key(subspace: &Subspace, entity_type: &str, version: u32) -> Bytes {
        subspace
            .pack()
            .add_string(entity_type)
            .add_string("v")
            .add_int(version as i64)
            .build()
    }

    /// Encode version number to bytes
    #[allow(dead_code)] // Will be used when FDB operations are implemented
    fn encode_version(version: u32) -> [u8; 4] {
        version.to_be_bytes()
    }

    /// Decode version number from bytes
    #[allow(dead_code)] // Will be used when FDB operations are implemented
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
        if !schema
            .name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_')
        {
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
        }

        Ok(())
    }
}

impl Default for SchemaRegistry {
    fn default() -> Self {
        Self::new(Arc::new(SchemaCache::new()))
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
