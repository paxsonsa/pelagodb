//! Index operations
//!
//! FDB Key Layout:
//! ```text
//! Unique:   (db, ns, idx, entity_type, property, "u", value) → node_id
//! Equality: (db, ns, idx, entity_type, property, "e", value, node_id) → empty
//! Range:    (db, ns, idx, entity_type, property, "r", encoded_value, node_id) → empty
//! ```
//!
//! Index Semantics:
//! - Unique index: check-then-write pattern (fail if key exists)
//! - Equality index: append node_id to value for multiple matches
//! - Range index: sort-order encoded values for range queries
//! - Null handling: no index entry for null values

use crate::Subspace;
use bytes::Bytes;
use pelago_core::encoding::encode_value_for_index;
use pelago_core::schema::{EntitySchema, IndexType, PropertyDef};
use pelago_core::{PelagoError, Value};
use std::collections::HashMap;

/// Index key markers
pub mod markers {
    pub const UNIQUE: u8 = b'u';
    pub const EQUALITY: u8 = b'e';
    pub const RANGE: u8 = b'r';
}

/// Compute index entries that need to be written for a node
pub fn compute_index_entries(
    subspace: &Subspace,
    entity_type: &str,
    node_id: &[u8],
    schema: &EntitySchema,
    properties: &HashMap<String, Value>,
) -> Result<Vec<IndexEntry>, PelagoError> {
    let mut entries = Vec::new();
    let idx_subspace = subspace.index();

    for (prop_name, prop_def) in &schema.properties {
        if prop_def.index == IndexType::None {
            continue;
        }

        // Get the property value, skip if null or missing
        let value = match properties.get(prop_name) {
            Some(v) if !v.is_null() => v,
            _ => continue, // No index entry for null/missing values
        };

        let entry = build_index_entry(
            &idx_subspace,
            entity_type,
            prop_name,
            node_id,
            prop_def,
            value,
        )?;
        entries.push(entry);
    }

    Ok(entries)
}

/// Compute index entries to remove for old property values
pub fn compute_index_removals(
    subspace: &Subspace,
    entity_type: &str,
    node_id: &[u8],
    schema: &EntitySchema,
    old_properties: &HashMap<String, Value>,
    new_properties: &HashMap<String, Value>,
) -> Result<Vec<IndexEntry>, PelagoError> {
    let mut removals = Vec::new();
    let idx_subspace = subspace.index();

    for (prop_name, prop_def) in &schema.properties {
        if prop_def.index == IndexType::None {
            continue;
        }

        let old_value = old_properties.get(prop_name);
        let new_value = new_properties.get(prop_name);

        // Only remove if old value exists, is non-null, and differs from new
        let old_val = match old_value {
            Some(v) if !v.is_null() => v,
            _ => continue,
        };

        let needs_removal = match new_value {
            Some(new_val) if !new_val.is_null() => old_val != new_val,
            _ => true, // Removing or nulling the property
        };

        if needs_removal {
            let entry = build_index_entry(
                &idx_subspace,
                entity_type,
                prop_name,
                node_id,
                prop_def,
                old_val,
            )?;
            removals.push(entry);
        }
    }

    Ok(removals)
}

/// Build a single index entry
fn build_index_entry(
    idx_subspace: &Subspace,
    entity_type: &str,
    prop_name: &str,
    node_id: &[u8],
    prop_def: &PropertyDef,
    value: &Value,
) -> Result<IndexEntry, PelagoError> {
    let encoded_value = encode_value_for_index(value)?;

    let (key, entry_type) = match prop_def.index {
        IndexType::Unique => {
            let key = idx_subspace
                .pack()
                .add_string(entity_type)
                .add_string(prop_name)
                .add_marker(markers::UNIQUE)
                .add_raw_bytes(&encoded_value)
                .build();
            (key, IndexEntryType::Unique)
        }
        IndexType::Equality => {
            let key = idx_subspace
                .pack()
                .add_string(entity_type)
                .add_string(prop_name)
                .add_marker(markers::EQUALITY)
                .add_raw_bytes(&encoded_value)
                .add_raw_bytes(node_id)
                .build();
            (key, IndexEntryType::Equality)
        }
        IndexType::Range => {
            let key = idx_subspace
                .pack()
                .add_string(entity_type)
                .add_string(prop_name)
                .add_marker(markers::RANGE)
                .add_raw_bytes(&encoded_value)
                .add_raw_bytes(node_id)
                .build();
            (key, IndexEntryType::Range)
        }
        IndexType::None => unreachable!(),
    };

    Ok(IndexEntry {
        key,
        entry_type,
        value_bytes: if matches!(prop_def.index, IndexType::Unique) {
            Some(Bytes::copy_from_slice(node_id))
        } else {
            None
        },
    })
}

/// Represents an index entry to write or remove
#[derive(Debug)]
pub struct IndexEntry {
    pub key: Bytes,
    pub entry_type: IndexEntryType,
    /// For unique indexes, the value is the node_id
    pub value_bytes: Option<Bytes>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexEntryType {
    Unique,
    Equality,
    Range,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pelago_core::NodeId;

    fn make_schema_with_indexes() -> EntitySchema {
        use pelago_core::PropertyType;

        EntitySchema::new("Person")
            .with_property(
                "email",
                PropertyDef::new(PropertyType::String).with_index(IndexType::Unique),
            )
            .with_property(
                "status",
                PropertyDef::new(PropertyType::String).with_index(IndexType::Equality),
            )
            .with_property(
                "age",
                PropertyDef::new(PropertyType::Int).with_index(IndexType::Range),
            )
            .with_property(
                "name",
                PropertyDef::new(PropertyType::String), // No index
            )
    }

    #[test]
    fn test_compute_index_entries() {
        let schema = make_schema_with_indexes();
        let subspace = Subspace::namespace("db", "ns");
        let node_id = NodeId::new(1, 100).to_bytes();

        let mut props = HashMap::new();
        props.insert(
            "email".to_string(),
            Value::String("alice@example.com".into()),
        );
        props.insert("status".to_string(), Value::String("active".into()));
        props.insert("age".to_string(), Value::Int(30));
        props.insert("name".to_string(), Value::String("Alice".into()));

        let entries = compute_index_entries(&subspace, "Person", &node_id, &schema, &props).unwrap();

        // Should have 3 index entries (email=unique, status=equality, age=range)
        // name has no index
        assert_eq!(entries.len(), 3);

        let unique_entry = entries
            .iter()
            .find(|e| e.entry_type == IndexEntryType::Unique)
            .unwrap();
        assert!(unique_entry.value_bytes.is_some());

        let equality_entry = entries
            .iter()
            .find(|e| e.entry_type == IndexEntryType::Equality)
            .unwrap();
        assert!(equality_entry.value_bytes.is_none());
    }

    #[test]
    fn test_null_values_not_indexed() {
        let schema = make_schema_with_indexes();
        let subspace = Subspace::namespace("db", "ns");
        let node_id = NodeId::new(1, 100).to_bytes();

        let mut props = HashMap::new();
        props.insert("email".to_string(), Value::Null);
        props.insert("status".to_string(), Value::Null);

        let entries = compute_index_entries(&subspace, "Person", &node_id, &schema, &props).unwrap();

        // No entries for null values
        assert_eq!(entries.len(), 0);
    }

    #[test]
    fn test_missing_values_not_indexed() {
        let schema = make_schema_with_indexes();
        let subspace = Subspace::namespace("db", "ns");
        let node_id = NodeId::new(1, 100).to_bytes();

        let props = HashMap::new(); // Empty

        let entries = compute_index_entries(&subspace, "Person", &node_id, &schema, &props).unwrap();

        // No entries for missing values
        assert_eq!(entries.len(), 0);
    }

    #[test]
    fn test_compute_index_removals() {
        let schema = make_schema_with_indexes();
        let subspace = Subspace::namespace("db", "ns");
        let node_id = NodeId::new(1, 100).to_bytes();

        let mut old_props = HashMap::new();
        old_props.insert(
            "email".to_string(),
            Value::String("old@example.com".into()),
        );
        old_props.insert("age".to_string(), Value::Int(25));

        let mut new_props = HashMap::new();
        new_props.insert(
            "email".to_string(),
            Value::String("new@example.com".into()),
        );
        new_props.insert("age".to_string(), Value::Int(25)); // Same value

        let removals = compute_index_removals(
            &subspace,
            "Person",
            &node_id,
            &schema,
            &old_props,
            &new_props,
        )
        .unwrap();

        // Only email changed, age stayed the same
        assert_eq!(removals.len(), 1);
        assert_eq!(removals[0].entry_type, IndexEntryType::Unique);
    }
}
