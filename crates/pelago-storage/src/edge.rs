//! Edge CRUD operations
//!
//! FDB Key Layout:
//! ```text
//! Forward:  (db, ns, edge, f, src_type, src_id, label, [sort_key], tgt_db, tgt_ns, tgt_type, tgt_id) → CBOR edge data
//! Metadata: (db, ns, edge, m, src_type, src_id, label, [sort_key], tgt_db, tgt_ns, tgt_type, tgt_id) → CBOR edge data (compat)
//! Reverse:  (db, ns, edge, r, tgt_db, tgt_ns, tgt_type, tgt_id, label, src_type, src_id) → CBOR edge data
//! ```
//!
//! Operations:
//! - create_edge: Validate nodes exist, write forward/reverse pairs
//! - delete_edge: Remove all paired entries
//! - list_edges: Range scan with optional label filter

use crate::cdc::CdcOperation;
use crate::db::PelagoDb;
use crate::failpoints;
use crate::ids::IdAllocator;
use crate::mutation;
use crate::node::NodeStore;
use crate::schema::SchemaRegistry;
use crate::subspace::edge_markers;
use crate::Subspace;
use bytes::Bytes;
use pelago_core::encoding::{decode_cbor, encode_cbor, encode_value_for_index};
use pelago_core::schema::{EdgeDirection, EntitySchema, OwnershipMode};
use pelago_core::{EdgeId, NodeId, PelagoError, Value};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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

    /// Delete all edges connected to a node and emit edge-delete CDC operations.
    pub async fn delete_edges_for_node(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        node_id: &str,
    ) -> Result<usize, PelagoError> {
        self.delete_edges_for_node_internal(database, namespace, entity_type, node_id, true)
            .await
    }

    /// Delete all edges connected to a node without emitting CDC.
    ///
    /// Used by replication apply paths where CDC has already been emitted at source.
    pub async fn delete_edges_for_node_replica(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        node_id: &str,
    ) -> Result<usize, PelagoError> {
        self.delete_edges_for_node_internal(database, namespace, entity_type, node_id, false)
            .await
    }

    async fn delete_edges_for_node_internal(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        node_id: &str,
        emit_cdc: bool,
    ) -> Result<usize, PelagoError> {
        const CASCADE_FETCH_LIMIT: usize = 2048;
        const CASCADE_TX_EDGE_LIMIT: usize = 256;

        let mut total_deleted = 0usize;

        loop {
            let mut candidates = self
                .list_edges(
                    database,
                    namespace,
                    entity_type,
                    node_id,
                    None,
                    CASCADE_FETCH_LIMIT,
                )
                .await?;
            candidates.extend(
                self.list_incoming_edges(
                    database,
                    namespace,
                    entity_type,
                    node_id,
                    None,
                    CASCADE_FETCH_LIMIT,
                )
                .await?,
            );

            if candidates.is_empty() {
                break;
            }

            let mut seen = HashSet::new();
            let mut planned: Vec<(EdgeKeys, NodeRef, NodeRef, String)> = Vec::new();

            for edge in candidates {
                let dedupe_key = format!(
                    "{}|{}|{}|{}|{}|{}|{}",
                    edge.edge_id,
                    edge.source.entity_type,
                    edge.source.node_id,
                    edge.label,
                    edge.target.entity_type,
                    edge.target.node_id,
                    edge.created_at
                );
                if !seen.insert(dedupe_key) {
                    continue;
                }

                let schema = self
                    .schema_registry
                    .get_schema(
                        &edge.source.database,
                        &edge.source.namespace,
                        &edge.source.entity_type,
                    )
                    .await?;
                let is_bidirectional = schema
                    .as_ref()
                    .and_then(|s| s.edges.get(&edge.label))
                    .map(|e| e.direction == EdgeDirection::Bidirectional)
                    .unwrap_or(false);
                let edge_data = match self
                    .find_edge_data_by_endpoints(
                        &edge.source.database,
                        &edge.source.namespace,
                        &edge.source,
                        &edge.target,
                        &edge.label,
                    )
                    .await?
                {
                    Some(data) => data,
                    None => continue,
                };
                let sort_key_value = sort_key_for_label(
                    schema.as_ref().map(|s| s.as_ref()),
                    &edge.label,
                    &edge_data.properties,
                );
                let subspace = Subspace::namespace(&edge.source.database, &edge.source.namespace);
                let keys = compute_edge_keys(
                    &subspace,
                    &edge.source,
                    &edge.target,
                    &edge.label,
                    sort_key_value,
                    is_bidirectional,
                )?;
                planned.push((keys, edge.source, edge.target, edge.label));
                if planned.len() >= CASCADE_TX_EDGE_LIMIT {
                    break;
                }
            }

            if planned.is_empty() {
                return Err(PelagoError::Internal(
                    "Unable to resolve edge keys during node cascade delete".to_string(),
                ));
            }

            let trx = self.db.create_transaction()?;
            let mut cdc = mutation::cdc_for_site(&self.site_id);

            for (keys, source, target, label) in &planned {
                trx.clear(keys.forward_key.as_ref());
                trx.clear(keys.meta_key.as_ref());
                trx.clear(keys.reverse_key.as_ref());
                if let Some((rev_forward, rev_reverse)) = &keys.reverse_direction_keys {
                    trx.clear(rev_forward.as_ref());
                    trx.clear(rev_reverse.as_ref());
                }

                if emit_cdc {
                    cdc.push(CdcOperation::EdgeDelete {
                        source_type: source.entity_type.clone(),
                        source_id: source.node_id.clone(),
                        target_type: target.entity_type.clone(),
                        target_id: target.node_id.clone(),
                        edge_type: label.clone(),
                    });
                }
            }

            if emit_cdc {
                cdc.flush(&trx, database, namespace)?;
                failpoints::inject("edge.cascade_delete.after_cdc_flush")?;
            }

            mutation::commit_or_internal(trx, "cascade-delete edges for node").await?;

            total_deleted = total_deleted.saturating_add(planned.len());
        }

        Ok(total_deleted)
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
        trx.set(keys.forward_key.as_ref(), &edge_bytes);
        trx.set(keys.meta_key.as_ref(), &edge_bytes);
        trx.set(keys.reverse_key.as_ref(), &edge_bytes);
        if let Some((rev_forward, rev_reverse)) = &keys.reverse_direction_keys {
            trx.set(rev_forward.as_ref(), &edge_bytes);
            trx.set(rev_reverse.as_ref(), &edge_bytes);
        }
        mutation::commit_or_internal(trx, "apply replicated edge create").await?;
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
        let edge_data = match self
            .find_edge_data_by_endpoints(database, namespace, &source, &target, label)
            .await?
        {
            Some(data) => data,
            None => return Ok(false),
        };
        let sort_key_value = sort_key_for_label(
            schema.as_ref().map(|s| s.as_ref()),
            label,
            &edge_data.properties,
        );
        let keys = compute_edge_keys(
            &subspace,
            &source,
            &target,
            label,
            sort_key_value,
            is_bidirectional,
        )?;
        if self.db.get(keys.forward_key.as_ref()).await?.is_none() {
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
        mutation::commit_or_internal(trx, "apply replicated edge delete").await?;
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
        let mut cdc = mutation::cdc_for_site(&self.site_id);

        if self
            .node_delete_guard_exists_in_txn(
                &trx,
                &source.database,
                &source.namespace,
                &source.entity_type,
                &source.node_id,
            )
            .await?
        {
            return Err(PelagoError::VersionConflict {
                entity_type: source.entity_type.clone(),
                node_id: source.node_id.clone(),
            });
        }

        if self
            .node_delete_guard_exists_in_txn(
                &trx,
                &target.database,
                &target.namespace,
                &target.entity_type,
                &target.node_id,
            )
            .await?
        {
            return Err(PelagoError::VersionConflict {
                entity_type: target.entity_type.clone(),
                node_id: target.node_id.clone(),
            });
        }

        let source_locality = self
            .read_node_locality_in_txn(
                &trx,
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
        self.enforce_source_ownership(&source.entity_type, &source.node_id, source_locality)?;

        if self
            .read_node_locality_in_txn(
                &trx,
                &target.database,
                &target.namespace,
                &target.entity_type,
                &target.node_id,
            )
            .await?
            .is_none()
        {
            return Err(PelagoError::TargetNotFound {
                entity_type: target.entity_type.clone(),
                node_id: target.node_id.clone(),
            });
        }

        // Write forward key with edge data so scans avoid N+1 metadata fetches.
        trx.set(keys.forward_key.as_ref(), &edge_bytes);

        // Write metadata key (edge properties)
        trx.set(keys.meta_key.as_ref(), &edge_bytes);

        // Write reverse key (stores edge data for incoming scans)
        trx.set(keys.reverse_key.as_ref(), &edge_bytes);

        // Write bidirectional keys if needed
        if let Some((rev_forward, rev_reverse)) = &keys.reverse_direction_keys {
            trx.set(rev_forward.as_ref(), &edge_bytes);
            trx.set(rev_reverse.as_ref(), &edge_bytes);
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
        failpoints::inject("edge.create.after_cdc_flush")?;

        // Single atomic commit
        mutation::commit_or_internal(trx, "create edge").await?;

        Ok(StoredEdge::new(
            edge_id.to_string(),
            source,
            target,
            label.to_string(),
            properties,
        ))
    }

    async fn read_node_locality_in_txn(
        &self,
        trx: &foundationdb::Transaction,
        database: &str,
        namespace: &str,
        entity_type: &str,
        node_id: &str,
    ) -> Result<Option<u8>, PelagoError> {
        let parsed: NodeId = node_id.parse().map_err(|_| PelagoError::InvalidId {
            value: node_id.to_string(),
        })?;
        let node_id_bytes = parsed.to_bytes();
        let data_key = Subspace::namespace(database, namespace)
            .data()
            .pack()
            .add_string(entity_type)
            .add_raw_bytes(&node_id_bytes)
            .build();
        let bytes = trx
            .get(data_key.as_ref(), false)
            .await
            .map_err(|e| PelagoError::Internal(format!("Failed to read node in txn: {}", e)))?;
        let Some(bytes) = bytes else {
            return Ok(None);
        };

        let node_data: EdgeNodeData = decode_cbor(&bytes)?;
        Ok(Some(node_data.locality))
    }

    async fn node_delete_guard_exists_in_txn(
        &self,
        trx: &foundationdb::Transaction,
        database: &str,
        namespace: &str,
        entity_type: &str,
        node_id: &str,
    ) -> Result<bool, PelagoError> {
        let parsed: NodeId = node_id.parse().map_err(|_| PelagoError::InvalidId {
            value: node_id.to_string(),
        })?;
        let node_id_bytes = parsed.to_bytes();
        let guard_key = Subspace::namespace(database, namespace)
            .data()
            .pack()
            .add_string("__deleting")
            .add_string(entity_type)
            .add_raw_bytes(&node_id_bytes)
            .build();
        let value = trx
            .get(guard_key.as_ref(), false)
            .await
            .map_err(|e| PelagoError::Internal(format!("Failed to read delete guard: {}", e)))?;
        Ok(value.is_some())
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
        let edge_data = match self
            .find_edge_data_by_endpoints(database, namespace, &source, &target, label)
            .await?
        {
            Some(data) => data,
            None => return Ok(false),
        };
        let sort_key_value = sort_key_for_label(
            schema.as_ref().map(|s| s.as_ref()),
            label,
            &edge_data.properties,
        );
        let keys = compute_edge_keys(
            &subspace,
            &source,
            &target,
            label,
            sort_key_value,
            is_bidirectional,
        )?;

        if self.db.get(keys.forward_key.as_ref()).await?.is_none() {
            return Ok(false);
        }

        // Delete in a single transaction (edge keys + CDC)
        let trx = self.db.create_transaction()?;
        let mut cdc = mutation::cdc_for_site(&self.site_id);

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
        failpoints::inject("edge.delete.after_cdc_flush")?;

        // Single atomic commit
        mutation::commit_or_internal(trx, "delete edge").await?;

        Ok(true)
    }

    async fn find_edge_data_by_endpoints(
        &self,
        database: &str,
        namespace: &str,
        source: &NodeRef,
        target: &NodeRef,
        label: &str,
    ) -> Result<Option<EdgeData>, PelagoError> {
        self.find_edge_data_by_endpoints_at_read_version(
            database, namespace, source, target, label, None,
        )
        .await
    }

    async fn find_edge_data_by_endpoints_at_read_version(
        &self,
        database: &str,
        namespace: &str,
        source: &NodeRef,
        target: &NodeRef,
        label: &str,
        read_version: Option<i64>,
    ) -> Result<Option<EdgeData>, PelagoError> {
        let edge_subspace = Subspace::namespace(database, namespace).edge();
        let range_start = edge_subspace
            .pack()
            .add_marker(edge_markers::FORWARD)
            .add_string(&source.entity_type)
            .add_string(&source.node_id)
            .add_string(label)
            .build();
        let range_end = {
            let mut end = range_start.to_vec();
            end.push(0xFF);
            end
        };

        let rows = self
            .db
            .get_range_at_read_version(range_start.as_ref(), &range_end, 2048, read_version)
            .await?;
        let marker_pos = edge_subspace.prefix().len();

        for (key, forward_value) in rows {
            let edge_data = if !forward_value.is_empty() {
                decode_cbor::<EdgeData>(&forward_value).ok()
            } else {
                let mut meta_key = key.clone();
                if meta_key.len() > marker_pos {
                    meta_key[marker_pos] = edge_markers::FORWARD_META;
                }
                self.db
                    .get_at_read_version(meta_key.as_ref(), read_version)
                    .await?
                    .map(|bytes| decode_cbor::<EdgeData>(&bytes))
                    .transpose()?
            };

            let Some(edge_data) = edge_data else {
                continue;
            };

            let edge_label = edge_data.label.clone().unwrap_or_else(|| label.to_string());
            let edge_source = edge_data.source.clone().unwrap_or_else(|| source.clone());
            let edge_target = edge_data.target.clone().unwrap_or_else(|| target.clone());

            if edge_label == label && edge_source == *source && edge_target == *target {
                return Ok(Some(edge_data));
            }
        }

        Ok(None)
    }

    /// List outgoing edges from a node.
    pub async fn list_edges(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        node_id: &str,
        label_filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StoredEdge>, PelagoError> {
        self.list_edges_at_read_version(
            database,
            namespace,
            entity_type,
            node_id,
            label_filter,
            limit,
            None,
        )
        .await
    }

    /// List outgoing edges from a node at an optional pinned read version.
    pub async fn list_edges_at_read_version(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        node_id: &str,
        label_filter: Option<&str>,
        limit: usize,
        read_version: Option<i64>,
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
            .get_range_at_read_version(range_start.as_ref(), &range_end, limit, read_version)
            .await?;

        let mut edges = Vec::new();

        // The marker position is right after the edge subspace prefix
        let marker_pos = subspace.prefix().len();
        let needs_legacy_meta = results.iter().any(|(_, value)| value.is_empty());
        let mut legacy_meta_by_key: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();

        if needs_legacy_meta {
            let mut meta_start = range_start.to_vec();
            if meta_start.len() > marker_pos {
                meta_start[marker_pos] = edge_markers::FORWARD_META;
            }
            let mut meta_end = meta_start.clone();
            meta_end.push(0xFF);

            let meta_rows = self
                .db
                .get_range_at_read_version(meta_start.as_ref(), &meta_end, limit, read_version)
                .await?;
            legacy_meta_by_key.extend(meta_rows);
        }

        // For each forward key, fetch the corresponding metadata
        for (key, forward_value) in results {
            // Convert forward key to metadata key (change marker from 'f' to 'm')
            let mut meta_key = key.clone();
            // Replace the marker at the known position (not searching - 'f' may appear in strings!)
            if meta_key.len() > marker_pos {
                meta_key[marker_pos] = edge_markers::FORWARD_META;
            }

            let edge_data = if !forward_value.is_empty() {
                decode_cbor::<EdgeData>(&forward_value).ok()
            } else {
                None
            };

            let edge_data = match edge_data {
                Some(data) => Some(data),
                None => legacy_meta_by_key
                    .get(&meta_key)
                    .map(|meta_bytes| decode_cbor::<EdgeData>(meta_bytes))
                    .transpose()?,
            };

            let Some(edge_data) = edge_data else {
                continue;
            };

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

        Ok(edges)
    }

    /// List incoming edges to a node.
    pub async fn list_incoming_edges(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        node_id: &str,
        label_filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StoredEdge>, PelagoError> {
        self.list_incoming_edges_at_read_version(
            database,
            namespace,
            entity_type,
            node_id,
            label_filter,
            limit,
            None,
        )
        .await
    }

    /// List incoming edges to a node at an optional pinned read version.
    pub async fn list_incoming_edges_at_read_version(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        node_id: &str,
        label_filter: Option<&str>,
        limit: usize,
        read_version: Option<i64>,
    ) -> Result<Vec<StoredEdge>, PelagoError> {
        let edge_subspace = Subspace::namespace(database, namespace).edge();
        let mut prefix_builder = edge_subspace
            .pack()
            .add_marker(edge_markers::REVERSE)
            .add_string(database)
            .add_string(namespace)
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

        let rows = self
            .db
            .get_range_at_read_version(range_start.as_ref(), &range_end, limit, read_version)
            .await?;
        let marker_pos = edge_subspace.prefix().len();
        let mut edges = Vec::new();

        for (key, reverse_value) in rows {
            let edge_data = if !reverse_value.is_empty() {
                decode_cbor::<EdgeData>(&reverse_value).ok()
            } else {
                let Some(parsed) = parse_reverse_key_parts(&key, marker_pos) else {
                    continue;
                };
                let source = NodeRef::new(
                    database,
                    namespace,
                    &parsed.source_entity_type,
                    &parsed.source_node_id,
                );
                let target = NodeRef::new(
                    &parsed.target_database,
                    &parsed.target_namespace,
                    &parsed.target_entity_type,
                    &parsed.target_node_id,
                );
                self.find_edge_data_by_endpoints_at_read_version(
                    database,
                    namespace,
                    &source,
                    &target,
                    &parsed.label,
                    read_version,
                )
                .await?
            };

            let Some(edge_data) = edge_data else {
                continue;
            };

            let label = edge_data
                .label
                .clone()
                .unwrap_or_else(|| label_filter.unwrap_or("unknown").to_string());
            let target = edge_data
                .target
                .clone()
                .unwrap_or_else(|| NodeRef::new(database, namespace, entity_type, node_id));
            let source = edge_data
                .source
                .clone()
                .unwrap_or_else(|| NodeRef::new(database, namespace, "Unknown", "Unknown"));

            if target.entity_type == entity_type && target.node_id == node_id {
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

#[derive(Debug, Deserialize)]
struct EdgeNodeData {
    locality: u8,
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

fn sort_key_for_label<'a>(
    schema: Option<&EntitySchema>,
    label: &str,
    properties: &'a HashMap<String, Value>,
) -> Option<&'a Value> {
    schema
        .and_then(|s| s.edges.get(label))
        .and_then(|def| def.sort_key.as_ref())
        .and_then(|sort_key| properties.get(sort_key))
}

#[derive(Debug)]
struct ReverseKeyParts {
    target_database: String,
    target_namespace: String,
    target_entity_type: String,
    target_node_id: String,
    label: String,
    source_entity_type: String,
    source_node_id: String,
}

fn parse_reverse_key_parts(key: &[u8], marker_pos: usize) -> Option<ReverseKeyParts> {
    if key.get(marker_pos).copied()? != edge_markers::REVERSE {
        return None;
    }

    let mut idx = marker_pos + 1;
    let target_database = decode_tuple_string(key, &mut idx)?;
    let target_namespace = decode_tuple_string(key, &mut idx)?;
    let target_entity_type = decode_tuple_string(key, &mut idx)?;
    let target_node_id = decode_tuple_string(key, &mut idx)?;
    let label = decode_tuple_string(key, &mut idx)?;
    let source_entity_type = decode_tuple_string(key, &mut idx)?;
    let source_node_id = decode_tuple_string(key, &mut idx)?;

    Some(ReverseKeyParts {
        target_database,
        target_namespace,
        target_entity_type,
        target_node_id,
        label,
        source_entity_type,
        source_node_id,
    })
}

fn decode_tuple_string(bytes: &[u8], idx: &mut usize) -> Option<String> {
    if bytes.get(*idx).copied()? != 0x02 {
        return None;
    }
    *idx += 1;

    let mut out = Vec::new();
    while *idx < bytes.len() {
        let byte = bytes[*idx];
        *idx += 1;

        if byte == 0x00 {
            if *idx < bytes.len() && bytes[*idx] == 0xFF {
                out.push(0x00);
                *idx += 1;
                continue;
            }
            return String::from_utf8(out).ok();
        }

        out.push(byte);
    }

    None
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

    #[test]
    fn test_parse_reverse_key_parts_round_trip() {
        let subspace = Subspace::namespace("db", "ns");
        let source = NodeRef::new("db", "ns", "Person", "1_100");
        let target = NodeRef::new("db", "ns", "Company", "1_200");
        let keys = compute_edge_keys(&subspace, &source, &target, "WORKS_AT", None, false)
            .expect("compute edge keys");

        let marker_pos = subspace.edge().prefix().len();
        let parsed = parse_reverse_key_parts(keys.reverse_key.as_ref(), marker_pos)
            .expect("parse reverse key");

        assert_eq!(parsed.target_database, "db");
        assert_eq!(parsed.target_namespace, "ns");
        assert_eq!(parsed.target_entity_type, "Company");
        assert_eq!(parsed.target_node_id, "1_200");
        assert_eq!(parsed.label, "WORKS_AT");
        assert_eq!(parsed.source_entity_type, "Person");
        assert_eq!(parsed.source_node_id, "1_100");
    }

    #[test]
    fn test_decode_tuple_string_rejects_invalid_element_type() {
        let bytes = vec![0x01, b'a', 0x00];
        let mut idx = 0usize;
        assert!(decode_tuple_string(&bytes, &mut idx).is_none());
    }
}
