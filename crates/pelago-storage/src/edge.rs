//! Edge CRUD operations
//!
//! FDB Key Layout:
//! ```text
//! Forward:  (db, ns, edge, f, src_type, src_id, label, [sort_key], tgt_db, tgt_ns, tgt_type, tgt_id)
//! Metadata: (db, ns, edge, m, src_type, src_id, label, [sort_key], tgt_db, tgt_ns, tgt_type, tgt_id) → CBOR props
//! Reverse:  (db, ns, edge, r, tgt_db, tgt_ns, tgt_type, tgt_id, label, src_type, src_id)
//! ```
//!
//! Operations:
//! - create_edge: Validate nodes exist, write forward/reverse pairs
//! - delete_edge: Remove all paired entries
//! - list_edges: Range scan with optional label filter

use crate::subspace::edge_markers;
use crate::Subspace;
use bytes::Bytes;
use pelago_core::encoding::encode_value_for_index;
use pelago_core::schema::{EdgeDirection, EntitySchema};
use pelago_core::{PelagoError, Value};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Reference to a node (for edge endpoints)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeRef {
    pub database: String,
    pub namespace: String,
    pub entity_type: String,
    pub node_id: String,
}

impl NodeRef {
    pub fn new(database: &str, namespace: &str, entity_type: &str, node_id: &str) -> Self {
        Self {
            database: database.to_string(),
            namespace: namespace.to_string(),
            entity_type: entity_type.to_string(),
            node_id: node_id.to_string(),
        }
    }

    /// Check if this is a cross-database/namespace reference
    pub fn is_cross_namespace(&self, current_db: &str, current_ns: &str) -> bool {
        self.database != current_db || self.namespace != current_ns
    }
}

/// Stored edge data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredEdge {
    pub edge_id: String,
    pub source: NodeRef,
    pub target: NodeRef,
    pub label: String,
    pub properties: HashMap<String, Value>,
    pub created_at: i64,
}

impl StoredEdge {
    pub fn new(
        edge_id: String,
        source: NodeRef,
        target: NodeRef,
        label: String,
        properties: HashMap<String, Value>,
    ) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as i64;

        Self {
            edge_id,
            source,
            target,
            label,
            properties,
            created_at: now,
        }
    }
}

/// Compute the FDB keys for an edge
pub fn compute_edge_keys(
    subspace: &Subspace,
    source: &NodeRef,
    target: &NodeRef,
    label: &str,
    sort_key_value: Option<&Value>,
    is_bidirectional: bool,
) -> Result<EdgeKeys, PelagoError> {
    let edge_subspace = subspace.edge();

    // Encode sort key if present
    let sort_key_bytes = match sort_key_value {
        Some(v) if !v.is_null() => Some(encode_value_for_index(v)?),
        _ => None,
    };

    // Forward key: (edge, f, src_type, src_id, label, [sort_key], tgt_db, tgt_ns, tgt_type, tgt_id)
    let mut forward_builder = edge_subspace
        .pack()
        .add_marker(edge_markers::FORWARD)
        .add_string(&source.entity_type)
        .add_string(&source.node_id)
        .add_string(label);

    if let Some(ref sk) = sort_key_bytes {
        forward_builder = forward_builder.add_raw_bytes(sk);
    }

    let forward_key = forward_builder
        .add_string(&target.database)
        .add_string(&target.namespace)
        .add_string(&target.entity_type)
        .add_string(&target.node_id)
        .build();

    // Metadata key: (edge, m, src_type, src_id, label, [sort_key], tgt_db, tgt_ns, tgt_type, tgt_id)
    let mut meta_builder = edge_subspace
        .pack()
        .add_marker(edge_markers::FORWARD_META)
        .add_string(&source.entity_type)
        .add_string(&source.node_id)
        .add_string(label);

    if let Some(ref sk) = sort_key_bytes {
        meta_builder = meta_builder.add_raw_bytes(sk);
    }

    let meta_key = meta_builder
        .add_string(&target.database)
        .add_string(&target.namespace)
        .add_string(&target.entity_type)
        .add_string(&target.node_id)
        .build();

    // Reverse key: (edge, r, tgt_db, tgt_ns, tgt_type, tgt_id, label, src_type, src_id)
    let reverse_key = edge_subspace
        .pack()
        .add_marker(edge_markers::REVERSE)
        .add_string(&target.database)
        .add_string(&target.namespace)
        .add_string(&target.entity_type)
        .add_string(&target.node_id)
        .add_string(label)
        .add_string(&source.entity_type)
        .add_string(&source.node_id)
        .build();

    // For bidirectional edges, also compute the reverse direction keys
    let reverse_direction_keys = if is_bidirectional {
        // Swap source and target for the reverse direction
        let mut rev_forward_builder = edge_subspace
            .pack()
            .add_marker(edge_markers::FORWARD)
            .add_string(&target.entity_type)
            .add_string(&target.node_id)
            .add_string(label);

        if let Some(ref sk) = sort_key_bytes {
            rev_forward_builder = rev_forward_builder.add_raw_bytes(sk);
        }

        let rev_forward_key = rev_forward_builder
            .add_string(&source.database)
            .add_string(&source.namespace)
            .add_string(&source.entity_type)
            .add_string(&source.node_id)
            .build();

        let rev_reverse_key = edge_subspace
            .pack()
            .add_marker(edge_markers::REVERSE)
            .add_string(&source.database)
            .add_string(&source.namespace)
            .add_string(&source.entity_type)
            .add_string(&source.node_id)
            .add_string(label)
            .add_string(&target.entity_type)
            .add_string(&target.node_id)
            .build();

        Some((rev_forward_key, rev_reverse_key))
    } else {
        None
    };

    Ok(EdgeKeys {
        forward_key,
        meta_key,
        reverse_key,
        reverse_direction_keys,
    })
}

/// FDB keys for an edge
#[derive(Debug)]
pub struct EdgeKeys {
    /// Forward edge key
    pub forward_key: Bytes,
    /// Metadata key (for edge properties)
    pub meta_key: Bytes,
    /// Reverse edge key
    pub reverse_key: Bytes,
    /// For bidirectional edges: (forward_key, reverse_key) in reverse direction
    pub reverse_direction_keys: Option<(Bytes, Bytes)>,
}

impl EdgeKeys {
    /// Get all keys that need to be written
    pub fn all_keys(&self) -> Vec<&Bytes> {
        let mut keys = vec![&self.forward_key, &self.meta_key, &self.reverse_key];
        if let Some((ref rev_fwd, ref rev_rev)) = self.reverse_direction_keys {
            keys.push(rev_fwd);
            keys.push(rev_rev);
        }
        keys
    }

    /// Number of FDB entries for this edge
    pub fn entry_count(&self) -> usize {
        if self.reverse_direction_keys.is_some() {
            5 // forward, meta, reverse + rev_forward, rev_reverse
        } else {
            3 // forward, meta, reverse
        }
    }
}

/// Validate edge type against schema
pub fn validate_edge_type(
    schema: &EntitySchema,
    label: &str,
    target_type: &str,
) -> Result<EdgeDirection, PelagoError> {
    if let Some(edge_def) = schema.edges.get(label) {
        // Check target type constraint
        match &edge_def.target {
            pelago_core::schema::EdgeTarget::Specific(expected_type) => {
                if expected_type != target_type {
                    return Err(PelagoError::TargetTypeMismatch {
                        edge_type: label.to_string(),
                        expected: expected_type.clone(),
                        actual: target_type.to_string(),
                    });
                }
            }
            pelago_core::schema::EdgeTarget::Polymorphic => {
                // Any target type is allowed
            }
        }
        Ok(edge_def.direction)
    } else if schema.meta.allow_undeclared_edges {
        // Undeclared but allowed - default to outgoing
        Ok(EdgeDirection::Outgoing)
    } else {
        Err(PelagoError::UndeclaredEdgeType {
            entity_type: schema.name.clone(),
            edge_type: label.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pelago_core::schema::{EdgeDef, EdgeTarget, SchemaMeta};

    fn make_person_schema() -> EntitySchema {
        EntitySchema::new("Person")
            .with_edge("KNOWS", EdgeDef::new(EdgeTarget::specific("Person")).bidirectional())
            .with_edge("WORKS_AT", EdgeDef::new(EdgeTarget::specific("Company")))
            .with_edge("LIKES", EdgeDef::new(EdgeTarget::polymorphic()))
            .with_meta(SchemaMeta::strict())
    }

    #[test]
    fn test_compute_edge_keys_unidirectional() {
        let subspace = Subspace::namespace("db", "ns");
        let source = NodeRef::new("db", "ns", "Person", "1_100");
        let target = NodeRef::new("db", "ns", "Company", "1_200");

        let keys = compute_edge_keys(&subspace, &source, &target, "WORKS_AT", None, false).unwrap();

        assert!(keys.reverse_direction_keys.is_none());
        assert_eq!(keys.entry_count(), 3);
        assert_eq!(keys.all_keys().len(), 3);
    }

    #[test]
    fn test_compute_edge_keys_bidirectional() {
        let subspace = Subspace::namespace("db", "ns");
        let source = NodeRef::new("db", "ns", "Person", "1_100");
        let target = NodeRef::new("db", "ns", "Person", "1_200");

        let keys = compute_edge_keys(&subspace, &source, &target, "KNOWS", None, true).unwrap();

        assert!(keys.reverse_direction_keys.is_some());
        assert_eq!(keys.entry_count(), 5);
        assert_eq!(keys.all_keys().len(), 5);
    }

    #[test]
    fn test_compute_edge_keys_with_sort_key() {
        let subspace = Subspace::namespace("db", "ns");
        let source = NodeRef::new("db", "ns", "Person", "1_100");
        let target = NodeRef::new("db", "ns", "Company", "1_200");
        let sort_value = Value::Timestamp(1234567890);

        let keys_with_sort =
            compute_edge_keys(&subspace, &source, &target, "WORKS_AT", Some(&sort_value), false)
                .unwrap();
        let keys_without_sort =
            compute_edge_keys(&subspace, &source, &target, "WORKS_AT", None, false).unwrap();

        // Keys with sort key should be different (longer)
        assert_ne!(keys_with_sort.forward_key, keys_without_sort.forward_key);
    }

    #[test]
    fn test_validate_edge_type_valid() {
        let schema = make_person_schema();

        // Valid specific target
        let dir = validate_edge_type(&schema, "WORKS_AT", "Company").unwrap();
        assert_eq!(dir, EdgeDirection::Outgoing);

        // Valid bidirectional
        let dir = validate_edge_type(&schema, "KNOWS", "Person").unwrap();
        assert_eq!(dir, EdgeDirection::Bidirectional);

        // Valid polymorphic
        let dir = validate_edge_type(&schema, "LIKES", "Anything").unwrap();
        assert_eq!(dir, EdgeDirection::Outgoing);
    }

    #[test]
    fn test_validate_edge_type_mismatch() {
        let schema = make_person_schema();

        let result = validate_edge_type(&schema, "WORKS_AT", "WrongType");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PelagoError::TargetTypeMismatch { .. }
        ));
    }

    #[test]
    fn test_validate_edge_type_undeclared() {
        let schema = make_person_schema(); // strict mode

        let result = validate_edge_type(&schema, "UNKNOWN_EDGE", "Company");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PelagoError::UndeclaredEdgeType { .. }
        ));
    }

    #[test]
    fn test_validate_edge_type_undeclared_allowed() {
        let schema = EntitySchema::new("Person").with_meta(SchemaMeta::permissive());

        let result = validate_edge_type(&schema, "UNKNOWN_EDGE", "Company");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), EdgeDirection::Outgoing);
    }

    #[test]
    fn test_node_ref_cross_namespace() {
        let ref1 = NodeRef::new("db", "ns", "Person", "1_100");
        let ref2 = NodeRef::new("other_db", "ns", "Person", "1_100");
        let ref3 = NodeRef::new("db", "other_ns", "Person", "1_100");

        assert!(!ref1.is_cross_namespace("db", "ns"));
        assert!(ref2.is_cross_namespace("db", "ns"));
        assert!(ref3.is_cross_namespace("db", "ns"));
    }
}
