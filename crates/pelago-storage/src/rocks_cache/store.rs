use crate::cdc::{CdcOperation, Versionstamp};
use crate::edge::{NodeRef, StoredEdge};
use crate::node::StoredNode;
use pelago_core::encoding::{decode_cbor, encode_cbor};
use pelago_core::PelagoError;
use super::config::RocksCacheConfig;
use std::collections::HashMap;

pub struct RocksCacheStore {
    db: rocksdb::DB,
}

impl RocksCacheStore {
    pub fn open(config: &RocksCacheConfig) -> Result<Self, PelagoError> {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.set_write_buffer_size(config.write_buffer_mb * 1024 * 1024);
        opts.set_max_write_buffer_number(config.max_write_buffers);

        let db = rocksdb::DB::open(&opts, &config.path)
            .map_err(|e| PelagoError::Internal(format!("RocksDB open failed: {}", e)))?;

        Ok(Self { db })
    }

    pub fn open_temp() -> Result<Self, PelagoError> {
        let path = format!("/tmp/pelago-cache-test-{}", std::process::id());
        let mut config = RocksCacheConfig::default();
        config.path = path;
        Self::open(&config)
    }

    // Node key: n:<db>:<ns>:<type>:<node_id>
    fn node_key(db: &str, ns: &str, entity_type: &str, node_id: &str) -> Vec<u8> {
        format!("n:{}:{}:{}:{}", db, ns, entity_type, node_id).into_bytes()
    }

    // Edge key: e:<db>:<ns>:<src_type>:<src_id>:<label>:<tgt_type>:<tgt_id>
    fn edge_key(db: &str, ns: &str, source: &NodeRef, label: &str, target: &NodeRef) -> Vec<u8> {
        format!(
            "e:{}:{}:{}:{}:{}:{}:{}",
            db, ns, source.entity_type, source.node_id, label, target.entity_type, target.node_id
        )
        .into_bytes()
    }

    // Edge prefix for listing: e:<db>:<ns>:<src_type>:<src_id>: or with label
    fn edge_prefix(
        db: &str,
        ns: &str,
        entity_type: &str,
        node_id: &str,
        label: Option<&str>,
    ) -> Vec<u8> {
        match label {
            Some(l) => format!("e:{}:{}:{}:{}:{}:", db, ns, entity_type, node_id, l).into_bytes(),
            None => format!("e:{}:{}:{}:{}:", db, ns, entity_type, node_id).into_bytes(),
        }
    }

    pub fn get_node(
        &self,
        db: &str,
        ns: &str,
        entity_type: &str,
        node_id: &str,
    ) -> Result<Option<StoredNode>, PelagoError> {
        let key = Self::node_key(db, ns, entity_type, node_id);
        match self
            .db
            .get(&key)
            .map_err(|e| PelagoError::Internal(format!("RocksDB get: {}", e)))?
        {
            Some(bytes) => Ok(Some(decode_cbor(&bytes)?)),
            None => Ok(None),
        }
    }

    pub fn put_node(
        &self,
        db: &str,
        ns: &str,
        node: &StoredNode,
    ) -> Result<(), PelagoError> {
        let key = Self::node_key(db, ns, &node.entity_type, &node.id);
        let value = encode_cbor(node)?;
        self.db
            .put(&key, &value)
            .map_err(|e| PelagoError::Internal(format!("RocksDB put: {}", e)))
    }

    pub fn delete_node(
        &self,
        db: &str,
        ns: &str,
        entity_type: &str,
        node_id: &str,
    ) -> Result<(), PelagoError> {
        let key = Self::node_key(db, ns, entity_type, node_id);
        self.db
            .delete(&key)
            .map_err(|e| PelagoError::Internal(format!("RocksDB delete: {}", e)))
    }

    pub fn put_edge(
        &self,
        db: &str,
        ns: &str,
        edge: &StoredEdge,
    ) -> Result<(), PelagoError> {
        let key = Self::edge_key(db, ns, &edge.source, &edge.label, &edge.target);
        let value = encode_cbor(edge)?;
        self.db
            .put(&key, &value)
            .map_err(|e| PelagoError::Internal(format!("RocksDB put edge: {}", e)))
    }

    pub fn delete_edge(
        &self,
        db: &str,
        ns: &str,
        source: &NodeRef,
        label: &str,
        target: &NodeRef,
    ) -> Result<(), PelagoError> {
        let key = Self::edge_key(db, ns, source, label, target);
        self.db
            .delete(&key)
            .map_err(|e| PelagoError::Internal(format!("RocksDB delete edge: {}", e)))
    }

    pub fn list_edges_cached(
        &self,
        db: &str,
        ns: &str,
        entity_type: &str,
        node_id: &str,
        label: Option<&str>,
    ) -> Result<Vec<StoredEdge>, PelagoError> {
        let prefix = Self::edge_prefix(db, ns, entity_type, node_id, label);
        let mut edges = Vec::new();
        let iter = self.db.prefix_iterator(&prefix);
        for item in iter {
            let (key, value) = item
                .map_err(|e| PelagoError::Internal(format!("RocksDB iter: {}", e)))?;
            if !key.starts_with(&prefix) {
                break;
            }
            let edge: StoredEdge = decode_cbor(&value)?;
            edges.push(edge);
        }
        Ok(edges)
    }

    pub fn get_hwm(&self) -> Result<Versionstamp, PelagoError> {
        match self
            .db
            .get(b"_hwm")
            .map_err(|e| PelagoError::Internal(format!("RocksDB get hwm: {}", e)))?
        {
            Some(bytes) => Versionstamp::from_bytes(&bytes)
                .ok_or_else(|| PelagoError::Internal("Invalid HWM".into())),
            None => Ok(Versionstamp::zero()),
        }
    }

    pub fn set_hwm(&self, vs: &Versionstamp) -> Result<(), PelagoError> {
        self.db
            .put(b"_hwm", vs.to_bytes())
            .map_err(|e| PelagoError::Internal(format!("RocksDB set hwm: {}", e)))
    }

    /// Apply a batch of CDC operations atomically with HWM advancement.
    pub fn apply_cdc_operations(
        &self,
        db: &str,
        ns: &str,
        operations: &[CdcOperation],
        hwm: &Versionstamp,
    ) -> Result<(), PelagoError> {
        let mut batch = rocksdb::WriteBatch::default();

        // Track node mutations within the same CDC entry so update/delete ordering is preserved.
        let mut pending_nodes: HashMap<(String, String), Option<StoredNode>> = HashMap::new();

        for op in operations {
            match op {
                CdcOperation::NodeCreate {
                    entity_type,
                    node_id,
                    properties,
                    home_site,
                } => {
                    let node = StoredNode {
                        id: node_id.clone(),
                        entity_type: entity_type.clone(),
                        properties: properties.clone(),
                        locality: home_site.parse::<u8>().unwrap_or(1),
                        created_at: 0,
                        updated_at: 0,
                    };
                    pending_nodes.insert((entity_type.clone(), node_id.clone()), Some(node));
                }
                CdcOperation::NodeUpdate {
                    entity_type,
                    node_id,
                    changed_properties,
                    ..
                } => {
                    let key = (entity_type.clone(), node_id.clone());
                    let existing = if let Some(staged) = pending_nodes.get(&key) {
                        staged.clone()
                    } else {
                        self.get_node(db, ns, entity_type, node_id)?
                    };

                    if let Some(mut node) = existing {
                        for (k, v) in changed_properties {
                            if v.is_null() {
                                node.properties.remove(k);
                            } else {
                                node.properties.insert(k.clone(), v.clone());
                            }
                        }
                        pending_nodes.insert(key, Some(node));
                    }
                }
                CdcOperation::NodeDelete {
                    entity_type,
                    node_id,
                } => {
                    pending_nodes.insert((entity_type.clone(), node_id.clone()), None);
                }
                CdcOperation::EdgeCreate {
                    source_type,
                    source_id,
                    target_type,
                    target_id,
                    edge_type,
                    edge_id,
                    properties,
                } => {
                    let edge = StoredEdge {
                        source: NodeRef::new(db, ns, source_type, source_id),
                        target: NodeRef::new(db, ns, target_type, target_id),
                        label: edge_type.clone(),
                        edge_id: edge_id.clone(),
                        properties: properties.clone(),
                        created_at: 0,
                    };
                    let key = Self::edge_key(db, ns, &edge.source, &edge.label, &edge.target);
                    let value = encode_cbor(&edge)?;
                    batch.put(key, value);
                }
                CdcOperation::EdgeDelete {
                    source_type,
                    source_id,
                    target_type,
                    target_id,
                    edge_type,
                } => {
                    let source = NodeRef::new(db, ns, source_type, source_id);
                    let target = NodeRef::new(db, ns, target_type, target_id);
                    let key = Self::edge_key(db, ns, &source, edge_type, &target);
                    batch.delete(key);
                }
                CdcOperation::SchemaRegister { .. } => {
                    // No cache action for schema registrations.
                }
            }
        }

        for ((entity_type, node_id), maybe_node) in pending_nodes {
            let key = Self::node_key(db, ns, &entity_type, &node_id);
            match maybe_node {
                Some(node) => {
                    let value = encode_cbor(&node)?;
                    batch.put(key, value);
                }
                None => batch.delete(key),
            }
        }

        batch.put(b"_hwm", hwm.to_bytes());
        self.db
            .write(batch)
            .map_err(|e| PelagoError::Internal(format!("RocksDB batch write: {}", e)))
    }

    /// Clear all data (for testing/rebuild)
    pub fn clear(&self) -> Result<(), PelagoError> {
        let iter = self.db.iterator(rocksdb::IteratorMode::Start);
        for item in iter {
            let (key, _) = item
                .map_err(|e| PelagoError::Internal(format!("RocksDB iter: {}", e)))?;
            self.db
                .delete(&*key)
                .map_err(|e| PelagoError::Internal(format!("RocksDB delete: {}", e)))?;
        }
        Ok(())
    }
}
