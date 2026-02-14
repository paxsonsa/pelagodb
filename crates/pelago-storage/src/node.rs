//! Node CRUD operations
//!
//! FDB Key Layout:
//! ```text
//! Data:     (db, ns, data, entity_type, node_id) → CBOR properties
//! Locality: (db, ns, loc, entity_type, site_id, node_id) → empty
//! ```
//!
//! Operations:
//! - create_node: Validate, allocate ID, write data + indexes + CDC
//! - get_node: Point lookup by ID
//! - update_node: Read-modify-write with index diff
//! - delete_node: Remove data, indexes, cascade edges, emit CDC

use crate::cdc::CdcWriter;
use crate::db::PelagoDb;
use crate::ids::IdAllocator;
use crate::index::{compute_index_entries, compute_index_removals, IndexEntry, IndexEntryType};
use crate::schema::SchemaRegistry;
use crate::Subspace;
use bytes::Bytes;
use pelago_core::encoding::{decode_cbor, encode_cbor};
use pelago_core::schema::{EntitySchema, ExtrasPolicy, IndexType};
use pelago_core::{NodeId, PelagoError, Value};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Node storage operations
pub struct NodeStore {
    db: PelagoDb,
    schema_registry: Arc<SchemaRegistry>,
    id_allocator: Arc<IdAllocator>,
    cdc_writer: Arc<CdcWriter>,
}

impl NodeStore {
    pub fn new(
        db: PelagoDb,
        schema_registry: Arc<SchemaRegistry>,
        id_allocator: Arc<IdAllocator>,
        cdc_writer: Arc<CdcWriter>,
    ) -> Self {
        Self {
            db,
            schema_registry,
            id_allocator,
            cdc_writer,
        }
    }

    /// Build the data key for a node
    fn data_key(subspace: &Subspace, entity_type: &str, node_id: &[u8]) -> Bytes {
        subspace
            .pack()
            .add_string(entity_type)
            .add_raw_bytes(node_id)
            .build()
    }

    /// Build the locality key for a node
    #[allow(dead_code)]
    fn locality_key(subspace: &Subspace, entity_type: &str, site_id: u8, node_id: &[u8]) -> Bytes {
        // Build using the same subspace base but with loc marker
        subspace
            .pack()
            .add_string(entity_type)
            .add_int(site_id as i64)
            .add_raw_bytes(node_id)
            .build()
    }

    /// Create a new node
    pub async fn create_node(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        mut properties: HashMap<String, Value>,
    ) -> Result<StoredNode, PelagoError> {
        // Get schema
        let schema = self
            .schema_registry
            .get_schema(database, namespace, entity_type)
            .await?
            .ok_or_else(|| PelagoError::UnregisteredType {
                entity_type: entity_type.to_string(),
            })?;

        // Apply defaults before validation
        apply_defaults(&schema, &mut properties);

        // Validate properties
        validate_properties(entity_type, &schema, &properties)?;

        // Check unique constraints before creating
        for (prop_name, prop_def) in &schema.properties {
            if prop_def.index == IndexType::Unique {
                if let Some(value) = properties.get(prop_name) {
                    if let Some(_existing_id) = self
                        .check_unique_constraint(database, namespace, entity_type, prop_name, value)
                        .await?
                    {
                        return Err(PelagoError::UniqueConstraintViolation {
                            entity_type: entity_type.to_string(),
                            field: prop_name.clone(),
                            value: format!("{:?}", value),
                        });
                    }
                }
            }
        }

        // Allocate ID
        let (site_id, seq) = self
            .id_allocator
            .allocate(database, namespace, entity_type)
            .await?;
        let node_id = NodeId::new(site_id, seq);
        let node_id_bytes = node_id.to_bytes();

        // Create stored node
        let stored_node = StoredNode::new(
            node_id.to_string(),
            entity_type.to_string(),
            properties.clone(),
            site_id,
        );

        // Compute index entries
        let subspace = Subspace::namespace(database, namespace);
        let index_entries =
            compute_index_entries(&subspace, entity_type, &node_id_bytes, &schema, &properties)?;

        // Encode node data
        let node_data = encode_cbor(&NodeData {
            properties: properties.clone(),
            locality: site_id,
            created_at: stored_node.created_at,
            updated_at: stored_node.updated_at,
        })?;

        // Write everything in a transaction
        let trx = self.db.create_transaction()?;

        // Write data key
        let data_subspace = subspace.data();
        let data_key = Self::data_key(&data_subspace, entity_type, &node_id_bytes);
        trx.set(data_key.as_ref(), &node_data);

        // Write index entries
        for entry in &index_entries {
            Self::write_index_entry(&trx, entry)?;
        }

        // Commit
        trx.commit()
            .await
            .map_err(|e| PelagoError::Internal(format!("Failed to create node: {}", e)))?;

        // Write CDC entry (after commit for ordering)
        self.cdc_writer
            .write_create(database, namespace, entity_type, &node_id.to_string(), &properties)
            .await?;

        Ok(stored_node)
    }

    /// Get a node by ID
    pub async fn get_node(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        node_id: &str,
    ) -> Result<Option<StoredNode>, PelagoError> {
        let parsed_id: NodeId = node_id
            .parse()
            .map_err(|_| PelagoError::InvalidId {
                value: node_id.to_string(),
            })?;
        let node_id_bytes = parsed_id.to_bytes();

        let subspace = Subspace::namespace(database, namespace).data();
        let data_key = Self::data_key(&subspace, entity_type, &node_id_bytes);

        match self.db.get(data_key.as_ref()).await? {
            Some(bytes) => {
                let data: NodeData = decode_cbor(&bytes)?;
                Ok(Some(StoredNode {
                    id: node_id.to_string(),
                    entity_type: entity_type.to_string(),
                    properties: data.properties,
                    locality: data.locality,
                    created_at: data.created_at,
                    updated_at: data.updated_at,
                }))
            }
            None => Ok(None),
        }
    }

    /// Update a node's properties
    pub async fn update_node(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        node_id: &str,
        new_properties: HashMap<String, Value>,
    ) -> Result<StoredNode, PelagoError> {
        // Get schema
        let schema = self
            .schema_registry
            .get_schema(database, namespace, entity_type)
            .await?
            .ok_or_else(|| PelagoError::UnregisteredType {
                entity_type: entity_type.to_string(),
            })?;

        // Parse node ID
        let parsed_id: NodeId = node_id
            .parse()
            .map_err(|_| PelagoError::InvalidId {
                value: node_id.to_string(),
            })?;
        let node_id_bytes = parsed_id.to_bytes();

        let subspace = Subspace::namespace(database, namespace);
        let data_subspace = subspace.data();
        let data_key = Self::data_key(&data_subspace, entity_type, &node_id_bytes);

        // Read existing node
        let existing_bytes = self.db.get(data_key.as_ref()).await?.ok_or_else(|| {
            PelagoError::NodeNotFound {
                entity_type: entity_type.to_string(),
                node_id: node_id.to_string(),
            }
        })?;

        let existing_data: NodeData = decode_cbor(&existing_bytes)?;
        let old_properties = existing_data.properties.clone();

        // Merge: start with old properties, overlay new properties
        let mut merged_properties = old_properties.clone();
        for (key, value) in &new_properties {
            merged_properties.insert(key.clone(), value.clone());
        }

        // Validate merged properties (not just new ones)
        validate_properties(entity_type, &schema, &merged_properties)?;

        // Compute index changes between old and merged
        let removals = compute_index_removals(
            &subspace,
            entity_type,
            &node_id_bytes,
            &schema,
            &old_properties,
            &merged_properties,
        )?;

        let additions =
            compute_index_entries(&subspace, entity_type, &node_id_bytes, &schema, &merged_properties)?;

        // Update timestamp
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as i64;

        // Encode merged data
        let updated_data = NodeData {
            properties: merged_properties.clone(),
            locality: existing_data.locality,
            created_at: existing_data.created_at,
            updated_at: now,
        };
        let node_data = encode_cbor(&updated_data)?;

        // Write in transaction
        let trx = self.db.create_transaction()?;

        // Remove old index entries
        for entry in &removals {
            trx.clear(entry.key.as_ref());
        }

        // Add new index entries
        for entry in &additions {
            Self::write_index_entry(&trx, entry)?;
        }

        // Update data
        trx.set(data_key.as_ref(), &node_data);

        trx.commit()
            .await
            .map_err(|e| PelagoError::Internal(format!("Failed to update node: {}", e)))?;

        // Write CDC entry
        self.cdc_writer
            .write_update(
                database,
                namespace,
                entity_type,
                node_id,
                &old_properties,
                &merged_properties,
            )
            .await?;

        Ok(StoredNode {
            id: node_id.to_string(),
            entity_type: entity_type.to_string(),
            properties: merged_properties,
            locality: existing_data.locality,
            created_at: existing_data.created_at,
            updated_at: now,
        })
    }

    /// Delete a node
    pub async fn delete_node(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        node_id: &str,
    ) -> Result<bool, PelagoError> {
        // Get schema for index removal
        let schema = self
            .schema_registry
            .get_schema(database, namespace, entity_type)
            .await?
            .ok_or_else(|| PelagoError::UnregisteredType {
                entity_type: entity_type.to_string(),
            })?;

        // Parse node ID
        let parsed_id: NodeId = node_id
            .parse()
            .map_err(|_| PelagoError::InvalidId {
                value: node_id.to_string(),
            })?;
        let node_id_bytes = parsed_id.to_bytes();

        let subspace = Subspace::namespace(database, namespace);
        let data_subspace = subspace.data();
        let data_key = Self::data_key(&data_subspace, entity_type, &node_id_bytes);

        // Read existing node for CDC and index removal
        let existing_bytes = match self.db.get(data_key.as_ref()).await? {
            Some(bytes) => bytes,
            None => return Ok(false), // Node doesn't exist
        };

        let existing_data: NodeData = decode_cbor(&existing_bytes)?;

        // Compute index entries to remove
        let removals = compute_index_entries(
            &subspace,
            entity_type,
            &node_id_bytes,
            &schema,
            &existing_data.properties,
        )?;

        // Delete in transaction
        let trx = self.db.create_transaction()?;

        // Remove data
        trx.clear(data_key.as_ref());

        // Remove index entries
        for entry in &removals {
            trx.clear(entry.key.as_ref());
        }

        trx.commit()
            .await
            .map_err(|e| PelagoError::Internal(format!("Failed to delete node: {}", e)))?;

        // Write CDC entry
        self.cdc_writer
            .write_delete(
                database,
                namespace,
                entity_type,
                node_id,
                &existing_data.properties,
            )
            .await?;

        Ok(true)
    }

    /// Write an index entry to FDB
    fn write_index_entry(
        trx: &foundationdb::Transaction,
        entry: &IndexEntry,
    ) -> Result<(), PelagoError> {
        match entry.entry_type {
            IndexEntryType::Unique => {
                // For unique indexes, value is the node_id
                let value = entry.value_bytes.as_ref().ok_or_else(|| {
                    PelagoError::Internal("Unique index entry missing value".to_string())
                })?;
                trx.set(entry.key.as_ref(), value.as_ref());
            }
            IndexEntryType::Equality | IndexEntryType::Range => {
                // For equality/range indexes, value is empty (node_id is in key)
                trx.set(entry.key.as_ref(), &[]);
            }
        }
        Ok(())
    }

    /// Check if a unique index value already exists
    pub async fn check_unique_constraint(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        property: &str,
        value: &Value,
    ) -> Result<Option<String>, PelagoError> {
        let subspace = Subspace::namespace(database, namespace).index();
        let encoded_value = pelago_core::encoding::encode_value_for_index(value)?;

        let key = subspace
            .pack()
            .add_string(entity_type)
            .add_string(property)
            .add_marker(crate::index::markers::UNIQUE)
            .add_raw_bytes(&encoded_value)
            .build();

        match self.db.get(key.as_ref()).await? {
            Some(node_id_bytes) => {
                if node_id_bytes.len() == 9 {
                    let arr: [u8; 9] = node_id_bytes.try_into().unwrap();
                    let node_id = NodeId::from_bytes(&arr);
                    Ok(Some(node_id.to_string()))
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        }
    }
}

/// Validate node properties against a schema
pub fn validate_properties(
    entity_type: &str,
    schema: &EntitySchema,
    properties: &HashMap<String, Value>,
) -> Result<(), PelagoError> {
    // Check required properties
    for (prop_name, prop_def) in &schema.properties {
        if prop_def.required {
            match properties.get(prop_name) {
                None => {
                    return Err(PelagoError::MissingRequired {
                        entity_type: entity_type.to_string(),
                        field: prop_name.clone(),
                    });
                }
                Some(Value::Null) => {
                    return Err(PelagoError::MissingRequired {
                        entity_type: entity_type.to_string(),
                        field: prop_name.clone(),
                    });
                }
                _ => {}
            }
        }
    }

    // Check property types
    for (prop_name, value) in properties {
        if let Some(prop_def) = schema.properties.get(prop_name) {
            if !prop_def.property_type.matches(value) {
                return Err(PelagoError::TypeMismatch {
                    field: prop_name.clone(),
                    expected: prop_def.property_type.to_string(),
                    actual: value.type_name().to_string(),
                });
            }
        } else {
            // Extra property handling
            match schema.meta.extras_policy {
                ExtrasPolicy::Reject => {
                    return Err(PelagoError::ExtraProperty {
                        field: prop_name.clone(),
                    });
                }
                ExtrasPolicy::Warn => {
                    // In a real implementation, we'd log a warning
                    tracing::warn!(
                        entity_type = entity_type,
                        field = prop_name,
                        "Extra property not in schema"
                    );
                }
                ExtrasPolicy::Allow => {
                    // Silently allow
                }
            }
        }
    }

    Ok(())
}

/// Apply default values from schema to properties
pub fn apply_defaults(schema: &EntitySchema, properties: &mut HashMap<String, Value>) {
    for (prop_name, prop_def) in &schema.properties {
        if !properties.contains_key(prop_name) {
            if let Some(ref default) = prop_def.default_value {
                properties.insert(prop_name.clone(), default.clone());
            }
        }
    }
}

/// Internal node data structure for storage
#[derive(Debug, Clone, Serialize, Deserialize)]
struct NodeData {
    properties: HashMap<String, Value>,
    locality: u8,
    created_at: i64,
    updated_at: i64,
}

/// Node data structure for API responses
#[derive(Debug, Clone)]
pub struct StoredNode {
    pub id: String,
    pub entity_type: String,
    pub properties: HashMap<String, Value>,
    pub locality: u8,
    pub created_at: i64,
    pub updated_at: i64,
}

impl StoredNode {
    pub fn new(
        id: String,
        entity_type: String,
        properties: HashMap<String, Value>,
        site_id: u8,
    ) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as i64;

        Self {
            id,
            entity_type,
            properties,
            locality: site_id,
            created_at: now,
            updated_at: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pelago_core::schema::{IndexType, PropertyDef, SchemaMeta};
    use pelago_core::PropertyType;

    fn make_person_schema() -> EntitySchema {
        EntitySchema::new("Person")
            .with_property("name", PropertyDef::new(PropertyType::String).required())
            .with_property("age", PropertyDef::new(PropertyType::Int))
            .with_property(
                "email",
                PropertyDef::new(PropertyType::String).with_index(IndexType::Unique),
            )
            .with_meta(SchemaMeta::strict())
    }

    #[test]
    fn test_validate_valid_properties() {
        let schema = make_person_schema();
        let mut props = HashMap::new();
        props.insert("name".to_string(), Value::String("Alice".into()));
        props.insert("age".to_string(), Value::Int(30));
        props.insert(
            "email".to_string(),
            Value::String("alice@example.com".into()),
        );

        let result = validate_properties("Person", &schema, &props);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_missing_required() {
        let schema = make_person_schema();
        let props = HashMap::new(); // Missing "name"

        let result = validate_properties("Person", &schema, &props);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, PelagoError::MissingRequired { field, .. } if field == "name"));
    }

    #[test]
    fn test_validate_null_required() {
        let schema = make_person_schema();
        let mut props = HashMap::new();
        props.insert("name".to_string(), Value::Null);

        let result = validate_properties("Person", &schema, &props);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PelagoError::MissingRequired { .. }
        ));
    }

    #[test]
    fn test_validate_type_mismatch() {
        let schema = make_person_schema();
        let mut props = HashMap::new();
        props.insert("name".to_string(), Value::String("Alice".into()));
        props.insert("age".to_string(), Value::String("thirty".into())); // Should be Int

        let result = validate_properties("Person", &schema, &props);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, PelagoError::TypeMismatch { field, .. } if field == "age"));
    }

    #[test]
    fn test_validate_extra_property_rejected() {
        let schema = make_person_schema();
        let mut props = HashMap::new();
        props.insert("name".to_string(), Value::String("Alice".into()));
        props.insert("unknown".to_string(), Value::String("value".into()));

        let result = validate_properties("Person", &schema, &props);
        assert!(result.is_err());
        assert!(
            matches!(result.unwrap_err(), PelagoError::ExtraProperty { field } if field == "unknown")
        );
    }

    #[test]
    fn test_validate_extra_property_allowed() {
        let schema = EntitySchema::new("Person")
            .with_property("name", PropertyDef::new(PropertyType::String).required())
            .with_meta(SchemaMeta::permissive());

        let mut props = HashMap::new();
        props.insert("name".to_string(), Value::String("Alice".into()));
        props.insert("unknown".to_string(), Value::String("value".into()));

        let result = validate_properties("Person", &schema, &props);
        assert!(result.is_ok());
    }

    #[test]
    fn test_apply_defaults() {
        let schema = EntitySchema::new("Person")
            .with_property("name", PropertyDef::new(PropertyType::String).required())
            .with_property(
                "status",
                PropertyDef::new(PropertyType::String).with_default(Value::String("active".into())),
            );

        let mut props = HashMap::new();
        props.insert("name".to_string(), Value::String("Alice".into()));

        apply_defaults(&schema, &mut props);

        assert_eq!(props.get("status"), Some(&Value::String("active".into())));
    }

    #[test]
    fn test_apply_defaults_does_not_override() {
        let schema = EntitySchema::new("Person").with_property(
            "status",
            PropertyDef::new(PropertyType::String).with_default(Value::String("active".into())),
        );

        let mut props = HashMap::new();
        props.insert("status".to_string(), Value::String("inactive".into()));

        apply_defaults(&schema, &mut props);

        // Should NOT override existing value
        assert_eq!(
            props.get("status"),
            Some(&Value::String("inactive".into()))
        );
    }
}
