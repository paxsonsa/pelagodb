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

use crate::cdc::{CdcAccumulator, CdcOperation};
use crate::db::PelagoDb;
use crate::ids::IdAllocator;
use crate::node::NodeStore;
use crate::schema::SchemaRegistry;
use crate::subspace::edge_markers;
use crate::Subspace;
use bytes::Bytes;
use pelago_core::encoding::{decode_cbor, encode_cbor, encode_value_for_index};
use pelago_core::schema::{EdgeDirection, EntitySchema, OwnershipMode};
use pelago_core::{EdgeId, PelagoError, Value};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

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

/// Edge storage operations
pub struct EdgeStore {
    db: PelagoDb,
    schema_registry: Arc<SchemaRegistry>,
    id_allocator: Arc<IdAllocator>,
    node_store: Arc<NodeStore>,
    site_id: String,
}

impl EdgeStore {
    pub fn new(
        db: PelagoDb,
        schema_registry: Arc<SchemaRegistry>,
        id_allocator: Arc<IdAllocator>,
        node_store: Arc<NodeStore>,
        site_id: String,
    ) -> Self {
        Self {
            db,
            schema_registry,
            id_allocator,
            node_store,
            site_id,
        }
    }

    fn local_site_id(&self) -> Option<u8> {
        self.site_id.parse::<u8>().ok()
    }

    fn parse_site_id(value: &str, field: &str) -> Result<u8, PelagoError> {
        value.parse::<u8>().map_err(|_| PelagoError::InvalidValue {
            field: field.to_string(),
            reason: format!("'{}' is not a valid site id", value),
        })
    }

    fn enforce_source_ownership(
        &self,
        source_type: &str,
        source_id: &str,
        source_locality: u8,
    ) -> Result<(), PelagoError> {
        if let Some(local_site) = self.local_site_id() {
            if local_site != source_locality {
                return Err(PelagoError::VersionConflict {
                    entity_type: source_type.to_string(),
                    node_id: source_id.to_string(),
                });
            }
        }
        Ok(())
    }

    /// Apply replicated edge creation without emitting CDC.
    pub async fn apply_replica_edge_create(
        &self,
        database: &str,
        namespace: &str,
        source: NodeRef,
        target: NodeRef,
        label: &str,
        edge_id: &str,
        properties: HashMap<String, Value>,
        event_timestamp: i64,
        origin_site: &str,
    ) -> Result<bool, PelagoError> {
        let actor_site = Self::parse_site_id(origin_site, "origin_site")?;
        let source_node = self
            .node_store
            .get_node(
                &source.database,
                &source.namespace,
                &source.entity_type,
                &source.node_id,
            )
            .await?
            .ok_or_else(|| PelagoError::SourceNotFound {
                entity_type: source.entity_type.clone(),
                node_id: source.node_id.clone(),
            })?;
        let _target_node = self
            .node_store
            .get_node(
                &target.database,
                &target.namespace,
                &target.entity_type,
                &target.node_id,
            )
            .await?
            .ok_or_else(|| PelagoError::TargetNotFound {
                entity_type: target.entity_type.clone(),
                node_id: target.node_id.clone(),
            })?;

        let schema = self
            .schema_registry
            .get_schema(&source.database, &source.namespace, &source.entity_type)
            .await?
            .ok_or_else(|| PelagoError::UnregisteredType {
                entity_type: source.entity_type.clone(),
            })?;
        let direction = validate_edge_type(&schema, label, &target.entity_type)?;
        let is_bidirectional = direction == EdgeDirection::Bidirectional;
        let ownership = schema
            .edges
            .get(label)
            .map(|e| e.ownership)
            .unwrap_or(OwnershipMode::SourceSite);
        if ownership == OwnershipMode::SourceSite {
            if source_node.locality != actor_site && event_timestamp <= source_node.updated_at {
                return Ok(false);
            }
        }

        let sort_key_value = schema
            .edges
            .get(label)
            .and_then(|edge_def| edge_def.sort_key.as_ref())
            .and_then(|sk| properties.get(sk));

        let subspace = Subspace::namespace(database, namespace);
        let keys = compute_edge_keys(
            &subspace,
            &source,
            &target,
            label,
            sort_key_value,
            is_bidirectional,
        )?;
        if self.db.get(keys.forward_key.as_ref()).await?.is_some() {
            return Ok(false);
        }

        let edge_data = EdgeData {
            edge_id: edge_id.to_string(),
            properties: properties.clone(),
            created_at: event_timestamp,
            source: Some(source.clone()),
            target: Some(target.clone()),
            label: Some(label.to_string()),
        };
        let edge_bytes = encode_cbor(&edge_data)?;

        let trx = self.db.create_transaction()?;
        trx.set(keys.forward_key.as_ref(), &[]);
        trx.set(keys.meta_key.as_ref(), &edge_bytes);
        trx.set(keys.reverse_key.as_ref(), &[]);
        if let Some((rev_forward, rev_reverse)) = &keys.reverse_direction_keys {
            trx.set(rev_forward.as_ref(), &[]);
            trx.set(rev_reverse.as_ref(), &[]);
        }
        trx.commit().await.map_err(|e| {
            PelagoError::Internal(format!("Failed to apply replicated edge create: {}", e))
        })?;
        Ok(true)
    }

    /// Apply replicated edge deletion without emitting CDC.
    pub async fn apply_replica_edge_delete(
        &self,
        database: &str,
        namespace: &str,
        source: NodeRef,
        target: NodeRef,
        label: &str,
        event_timestamp: i64,
        origin_site: &str,
    ) -> Result<bool, PelagoError> {
        let actor_site = Self::parse_site_id(origin_site, "origin_site")?;
        let source_node = self
            .node_store
            .get_node(
                &source.database,
                &source.namespace,
                &source.entity_type,
                &source.node_id,
            )
            .await?
            .ok_or_else(|| PelagoError::SourceNotFound {
                entity_type: source.entity_type.clone(),
                node_id: source.node_id.clone(),
            })?;
        let schema = self
            .schema_registry
            .get_schema(&source.database, &source.namespace, &source.entity_type)
            .await?;
        let ownership = schema
            .as_ref()
            .and_then(|s| s.edges.get(label))
            .map(|e| e.ownership)
            .unwrap_or(OwnershipMode::SourceSite);
        if ownership == OwnershipMode::SourceSite
            && source_node.locality != actor_site
            && event_timestamp <= source_node.updated_at
        {
            return Ok(false);
        }

        let is_bidirectional = schema
            .as_ref()
            .and_then(|s| s.edges.get(label))
            .map(|e| e.direction == EdgeDirection::Bidirectional)
            .unwrap_or(false);

        let subspace = Subspace::namespace(database, namespace);
        let keys = compute_edge_keys(&subspace, &source, &target, label, None, is_bidirectional)?;
        let exists = self.db.get(keys.forward_key.as_ref()).await?.is_some();
        if !exists {
            return Ok(false);
        }

        let trx = self.db.create_transaction()?;
        trx.clear(keys.forward_key.as_ref());
        trx.clear(keys.meta_key.as_ref());
        trx.clear(keys.reverse_key.as_ref());
        if let Some((rev_forward, rev_reverse)) = &keys.reverse_direction_keys {
            trx.clear(rev_forward.as_ref());
            trx.clear(rev_reverse.as_ref());
        }
        trx.commit().await.map_err(|e| {
            PelagoError::Internal(format!("Failed to apply replicated edge delete: {}", e))
        })?;
        Ok(true)
    }

    /// Create a new edge
    pub async fn create_edge(
        &self,
        database: &str,
        namespace: &str,
        source: NodeRef,
        target: NodeRef,
        label: &str,
        properties: HashMap<String, Value>,
    ) -> Result<StoredEdge, PelagoError> {
        // Verify source node exists
        let source_node = self
            .node_store
            .get_node(
                &source.database,
                &source.namespace,
                &source.entity_type,
                &source.node_id,
            )
            .await?
            .ok_or_else(|| PelagoError::SourceNotFound {
                entity_type: source.entity_type.clone(),
                node_id: source.node_id.clone(),
            })?;
        self.enforce_source_ownership(&source.entity_type, &source.node_id, source_node.locality)?;

        // Verify target node exists
        let _target_node = self
            .node_store
            .get_node(
                &target.database,
                &target.namespace,
                &target.entity_type,
                &target.node_id,
            )
            .await?
            .ok_or_else(|| PelagoError::TargetNotFound {
                entity_type: target.entity_type.clone(),
                node_id: target.node_id.clone(),
            })?;

        // Get source schema and validate edge type
        let schema = self
            .schema_registry
            .get_schema(&source.database, &source.namespace, &source.entity_type)
            .await?
            .ok_or_else(|| PelagoError::UnregisteredType {
                entity_type: source.entity_type.clone(),
            })?;

        let direction = validate_edge_type(&schema, label, &target.entity_type)?;
        let is_bidirectional = direction == EdgeDirection::Bidirectional;

        // Get sort key value if defined
        let sort_key_value = if let Some(edge_def) = schema.edges.get(label) {
            edge_def.sort_key.as_ref().and_then(|sk| properties.get(sk))
        } else {
            None
        };

        // Allocate edge ID
        let (site_id, seq) = self
            .id_allocator
            .allocate(database, namespace, &format!("edge_{}", label))
            .await?;
        let edge_id = EdgeId::new(site_id, seq);

        // Compute keys
        let subspace = Subspace::namespace(database, namespace);
        let keys = compute_edge_keys(
            &subspace,
            &source,
            &target,
            label,
            sort_key_value,
            is_bidirectional,
        )?;

        // Encode edge data for metadata
        let edge_data = EdgeData {
            edge_id: edge_id.to_string(),
            properties: properties.clone(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_micros() as i64,
            source: Some(source.clone()),
            target: Some(target.clone()),
            label: Some(label.to_string()),
        };
        let edge_bytes = encode_cbor(&edge_data)?;

        // Write in a single transaction (edge keys + CDC)
        let trx = self.db.create_transaction()?;
        let mut cdc = CdcAccumulator::new(&self.site_id);

        // Write forward key (empty value - just existence)
        trx.set(keys.forward_key.as_ref(), &[]);

        // Write metadata key (edge properties)
        trx.set(keys.meta_key.as_ref(), &edge_bytes);

        // Write reverse key (empty value)
        trx.set(keys.reverse_key.as_ref(), &[]);

        // Write bidirectional keys if needed
        if let Some((rev_forward, rev_reverse)) = &keys.reverse_direction_keys {
            trx.set(rev_forward.as_ref(), &[]);
            trx.set(rev_reverse.as_ref(), &[]);
        }

        // Accumulate CDC operation
        cdc.push(CdcOperation::EdgeCreate {
            source_type: source.entity_type.clone(),
            source_id: source.node_id.clone(),
            target_type: target.entity_type.clone(),
            target_id: target.node_id.clone(),
            edge_type: label.to_string(),
            edge_id: edge_id.to_string(),
            properties: properties.clone(),
        });

        // Flush CDC into same transaction
        cdc.flush(&trx, database, namespace)?;

        // Single atomic commit
        trx.commit()
            .await
            .map_err(|e| PelagoError::Internal(format!("Failed to create edge: {}", e)))?;

        Ok(StoredEdge::new(
            edge_id.to_string(),
            source,
            target,
            label.to_string(),
            properties,
        ))
    }

    /// Delete an edge
    pub async fn delete_edge(
        &self,
        database: &str,
        namespace: &str,
        source: NodeRef,
        target: NodeRef,
        label: &str,
    ) -> Result<bool, PelagoError> {
        let source_node = self
            .node_store
            .get_node(
                &source.database,
                &source.namespace,
                &source.entity_type,
                &source.node_id,
            )
            .await?
            .ok_or_else(|| PelagoError::SourceNotFound {
                entity_type: source.entity_type.clone(),
                node_id: source.node_id.clone(),
            })?;
        self.enforce_source_ownership(&source.entity_type, &source.node_id, source_node.locality)?;

        // Get source schema for bidirectional check
        let schema = self
            .schema_registry
            .get_schema(&source.database, &source.namespace, &source.entity_type)
            .await?;

        let is_bidirectional = schema
            .as_ref()
            .and_then(|s| s.edges.get(label))
            .map(|e| e.direction == EdgeDirection::Bidirectional)
            .unwrap_or(false);

        let subspace = Subspace::namespace(database, namespace);
        let keys = compute_edge_keys(&subspace, &source, &target, label, None, is_bidirectional)?;

        // Check if edge exists
        let exists = self.db.get(keys.forward_key.as_ref()).await?.is_some();
        if !exists {
            return Ok(false);
        }

        // Delete in a single transaction (edge keys + CDC)
        let trx = self.db.create_transaction()?;
        let mut cdc = CdcAccumulator::new(&self.site_id);

        trx.clear(keys.forward_key.as_ref());
        trx.clear(keys.meta_key.as_ref());
        trx.clear(keys.reverse_key.as_ref());

        if let Some((rev_forward, rev_reverse)) = &keys.reverse_direction_keys {
            trx.clear(rev_forward.as_ref());
            trx.clear(rev_reverse.as_ref());
        }

        // Accumulate CDC operation
        cdc.push(CdcOperation::EdgeDelete {
            source_type: source.entity_type.clone(),
            source_id: source.node_id.clone(),
            target_type: target.entity_type.clone(),
            target_id: target.node_id.clone(),
            edge_type: label.to_string(),
        });

        // Flush CDC into same transaction
        cdc.flush(&trx, database, namespace)?;

        // Single atomic commit
        trx.commit()
            .await
            .map_err(|e| PelagoError::Internal(format!("Failed to delete edge: {}", e)))?;

        Ok(true)
    }

    /// List outgoing edges from a node
    pub async fn list_edges(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        node_id: &str,
        label_filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StoredEdge>, PelagoError> {
        let subspace = Subspace::namespace(database, namespace).edge();

        // Build range prefix for forward edges
        let mut prefix_builder = subspace
            .pack()
            .add_marker(edge_markers::FORWARD)
            .add_string(entity_type)
            .add_string(node_id);

        if let Some(label) = label_filter {
            prefix_builder = prefix_builder.add_string(label);
        }

        let range_start = prefix_builder.build();
        let range_end = {
            let mut end = range_start.to_vec();
            end.push(0xFF);
            end
        };

        let results = self
            .db
            .get_range(range_start.as_ref(), &range_end, limit)
            .await?;

        let mut edges = Vec::new();

        // The marker position is right after the edge subspace prefix
        let marker_pos = subspace.prefix().len();

        // For each forward key, fetch the corresponding metadata
        for (key, _) in results {
            // Convert forward key to metadata key (change marker from 'f' to 'm')
            let mut meta_key = key.clone();
            // Replace the marker at the known position (not searching - 'f' may appear in strings!)
            if meta_key.len() > marker_pos {
                meta_key[marker_pos] = edge_markers::FORWARD_META;
            }

            if let Some(meta_bytes) = self.db.get(&meta_key).await? {
                let edge_data: EdgeData = decode_cbor(&meta_bytes)?;
                let label = edge_data
                    .label
                    .clone()
                    .unwrap_or_else(|| label_filter.unwrap_or("unknown").to_string());
                let source = edge_data
                    .source
                    .clone()
                    .unwrap_or_else(|| NodeRef::new(database, namespace, entity_type, node_id));
                let target = edge_data
                    .target
                    .clone()
                    .unwrap_or_else(|| NodeRef::new(database, namespace, "Unknown", "Unknown"));

                edges.push(StoredEdge {
                    edge_id: edge_data.edge_id,
                    source,
                    target,
                    label,
                    properties: edge_data.properties,
                    created_at: edge_data.created_at,
                });
            }
        }

        Ok(edges)
    }
}

/// Internal edge data for storage
#[derive(Debug, Clone, Serialize, Deserialize)]
struct EdgeData {
    edge_id: String,
    properties: HashMap<String, Value>,
    created_at: i64,
    #[serde(default)]
    source: Option<NodeRef>,
    #[serde(default)]
    target: Option<NodeRef>,
    #[serde(default)]
    label: Option<String>,
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
            .with_edge(
                "KNOWS",
                EdgeDef::new(EdgeTarget::specific("Person")).bidirectional(),
            )
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

        let keys_with_sort = compute_edge_keys(
            &subspace,
            &source,
            &target,
            "WORKS_AT",
            Some(&sort_value),
            false,
        )
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
