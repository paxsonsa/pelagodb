use super::config::RocksCacheConfig;
use crate::cdc::{CdcOperation, Versionstamp};
use crate::edge::{NodeRef, StoredEdge};
use crate::node::StoredNode;
use pelago_core::encoding::{decode_cbor, encode_cbor};
use pelago_core::PelagoError;
use std::collections::HashMap;

const CF_DEFAULT: &str = "default";
const CF_NODE: &str = "node";
const CF_EDGE: &str = "edge";
const CF_META: &str = "meta";

pub struct RocksCacheStore {
    db: rocksdb::DB,
    use_column_families: bool,
}

impl RocksCacheStore {
    pub fn open(config: &RocksCacheConfig) -> Result<Self, PelagoError> {
        let mut db_opts = rocksdb::Options::default();
        db_opts.create_if_missing(true);
        db_opts.create_missing_column_families(true);

        let cf_opts = Self::cf_options(config)?;
        let db = if config.use_column_families {
            let cf_descriptors = vec![
                rocksdb::ColumnFamilyDescriptor::new(CF_DEFAULT, cf_opts.clone()),
                rocksdb::ColumnFamilyDescriptor::new(CF_NODE, cf_opts.clone()),
                rocksdb::ColumnFamilyDescriptor::new(CF_EDGE, cf_opts.clone()),
                rocksdb::ColumnFamilyDescriptor::new(CF_META, cf_opts),
            ];
            rocksdb::DB::open_cf_descriptors(&db_opts, &config.path, cf_descriptors)
                .map_err(|e| PelagoError::Internal(format!("RocksDB open failed: {}", e)))?
        } else {
            let mut fallback_opts = Self::cf_options(config)?;
            fallback_opts.create_if_missing(true);
            rocksdb::DB::open(&fallback_opts, &config.path)
                .map_err(|e| PelagoError::Internal(format!("RocksDB open failed: {}", e)))?
        };

        Ok(Self {
            db,
            use_column_families: config.use_column_families,
        })
    }

    pub fn open_temp() -> Result<Self, PelagoError> {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = format!(
            "/tmp/pelago-cache-test-{}-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("t"),
            ts
        );
        let mut config = RocksCacheConfig::default();
        config.path = path;
        Self::open(&config)
    }

    fn cf_options(config: &RocksCacheConfig) -> Result<rocksdb::Options, PelagoError> {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.set_write_buffer_size(config.write_buffer_mb * 1024 * 1024);
        opts.set_max_write_buffer_number(config.max_write_buffers);

        let mut block_opts = rocksdb::BlockBasedOptions::default();
        let block_cache = rocksdb::Cache::new_lru_cache(config.cache_size_mb * 1024 * 1024);
        block_opts.set_block_cache(&block_cache);
        block_opts.set_bloom_filter(config.bloom_bits_per_key as f64, false);
        opts.set_block_based_table_factory(&block_opts);
        opts.set_optimize_filters_for_hits(true);
        if config.prefix_extractor_bytes > 0 {
            opts.set_prefix_extractor(rocksdb::SliceTransform::create_fixed_prefix(
                config.prefix_extractor_bytes,
            ));
            opts.set_memtable_prefix_bloom_ratio(0.2);
        }
        Ok(opts)
    }

    fn cf_handle(&self, cf_name: &'static str) -> Result<&rocksdb::ColumnFamily, PelagoError> {
        self.db.cf_handle(cf_name).ok_or_else(|| {
            PelagoError::Internal(format!("RocksDB column family missing: {}", cf_name))
        })
    }

    fn get_cf(&self, cf_name: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, PelagoError> {
        if self.use_column_families {
            let cf = self.cf_handle(cf_name)?;
            self.db
                .get_cf(cf, key)
                .map_err(|e| PelagoError::Internal(format!("RocksDB get: {}", e)))
        } else {
            self.db
                .get(key)
                .map_err(|e| PelagoError::Internal(format!("RocksDB get: {}", e)))
        }
    }

    fn put_cf(&self, cf_name: &'static str, key: &[u8], value: &[u8]) -> Result<(), PelagoError> {
        if self.use_column_families {
            let cf = self.cf_handle(cf_name)?;
            self.db
                .put_cf(cf, key, value)
                .map_err(|e| PelagoError::Internal(format!("RocksDB put: {}", e)))
        } else {
            self.db
                .put(key, value)
                .map_err(|e| PelagoError::Internal(format!("RocksDB put: {}", e)))
        }
    }

    fn delete_cf(&self, cf_name: &'static str, key: &[u8]) -> Result<(), PelagoError> {
        if self.use_column_families {
            let cf = self.cf_handle(cf_name)?;
            self.db
                .delete_cf(cf, key)
                .map_err(|e| PelagoError::Internal(format!("RocksDB delete: {}", e)))
        } else {
            self.db
                .delete(key)
                .map_err(|e| PelagoError::Internal(format!("RocksDB delete: {}", e)))
        }
    }

    fn batch_put_cf(
        &self,
        batch: &mut rocksdb::WriteBatch,
        cf_name: &'static str,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), PelagoError> {
        if self.use_column_families {
            let cf = self.cf_handle(cf_name)?;
            batch.put_cf(cf, key, value);
        } else {
            batch.put(key, value);
        }
        Ok(())
    }

    fn batch_delete_cf(
        &self,
        batch: &mut rocksdb::WriteBatch,
        cf_name: &'static str,
        key: &[u8],
    ) -> Result<(), PelagoError> {
        if self.use_column_families {
            let cf = self.cf_handle(cf_name)?;
            batch.delete_cf(cf, key);
        } else {
            batch.delete(key);
        }
        Ok(())
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

    // Incoming edge key: i:<db>:<ns>:<tgt_type>:<tgt_id>:<label>:<src_type>:<src_id>
    fn incoming_edge_key(
        db: &str,
        ns: &str,
        source: &NodeRef,
        label: &str,
        target: &NodeRef,
    ) -> Vec<u8> {
        format!(
            "i:{}:{}:{}:{}:{}:{}:{}",
            db, ns, target.entity_type, target.node_id, label, source.entity_type, source.node_id
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

    // Incoming edge prefix for listing: i:<db>:<ns>:<tgt_type>:<tgt_id>: or with label
    fn incoming_edge_prefix(
        db: &str,
        ns: &str,
        entity_type: &str,
        node_id: &str,
        label: Option<&str>,
    ) -> Vec<u8> {
        match label {
            Some(l) => format!("i:{}:{}:{}:{}:{}:", db, ns, entity_type, node_id, l).into_bytes(),
            None => format!("i:{}:{}:{}:{}:", db, ns, entity_type, node_id).into_bytes(),
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
        match self.get_cf(CF_NODE, &key)? {
            Some(bytes) => Ok(Some(decode_cbor(&bytes)?)),
            None => Ok(None),
        }
    }

    pub fn put_node(&self, db: &str, ns: &str, node: &StoredNode) -> Result<(), PelagoError> {
        let key = Self::node_key(db, ns, &node.entity_type, &node.id);
        let value = encode_cbor(node)?;
        self.put_cf(CF_NODE, &key, &value)
    }

    pub fn delete_node(
        &self,
        db: &str,
        ns: &str,
        entity_type: &str,
        node_id: &str,
    ) -> Result<(), PelagoError> {
        let key = Self::node_key(db, ns, entity_type, node_id);
        self.delete_cf(CF_NODE, &key)
    }

    pub fn put_edge(&self, db: &str, ns: &str, edge: &StoredEdge) -> Result<(), PelagoError> {
        let key = Self::edge_key(db, ns, &edge.source, &edge.label, &edge.target);
        let incoming_key = Self::incoming_edge_key(db, ns, &edge.source, &edge.label, &edge.target);
        let value = encode_cbor(edge)?;
        let mut batch = rocksdb::WriteBatch::default();
        self.batch_put_cf(&mut batch, CF_EDGE, &key, &value)?;
        self.batch_put_cf(&mut batch, CF_EDGE, &incoming_key, &value)?;
        self.db
            .write(batch)
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
        let incoming_key = Self::incoming_edge_key(db, ns, source, label, target);
        let mut batch = rocksdb::WriteBatch::default();
        self.batch_delete_cf(&mut batch, CF_EDGE, &key)?;
        self.batch_delete_cf(&mut batch, CF_EDGE, &incoming_key)?;
        self.db
            .write(batch)
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
        if self.use_column_families {
            let cf = self.cf_handle(CF_EDGE)?;
            let iter = self.db.prefix_iterator_cf(cf, &prefix);
            for item in iter {
                let (key, value) =
                    item.map_err(|e| PelagoError::Internal(format!("RocksDB iter: {}", e)))?;
                if !key.starts_with(&prefix) {
                    break;
                }
                let edge: StoredEdge = decode_cbor(&value)?;
                edges.push(edge);
            }
        } else {
            let iter = self.db.prefix_iterator(&prefix);
            for item in iter {
                let (key, value) =
                    item.map_err(|e| PelagoError::Internal(format!("RocksDB iter: {}", e)))?;
                if !key.starts_with(&prefix) {
                    break;
                }
                let edge: StoredEdge = decode_cbor(&value)?;
                edges.push(edge);
            }
        }
        Ok(edges)
    }

    pub fn list_incoming_edges_cached(
        &self,
        db: &str,
        ns: &str,
        entity_type: &str,
        node_id: &str,
        label: Option<&str>,
    ) -> Result<Vec<StoredEdge>, PelagoError> {
        let prefix = Self::incoming_edge_prefix(db, ns, entity_type, node_id, label);
        let mut edges = Vec::new();
        if self.use_column_families {
            let cf = self.cf_handle(CF_EDGE)?;
            let iter = self.db.prefix_iterator_cf(cf, &prefix);
            for item in iter {
                let (key, value) =
                    item.map_err(|e| PelagoError::Internal(format!("RocksDB iter: {}", e)))?;
                if !key.starts_with(&prefix) {
                    break;
                }
                let edge: StoredEdge = decode_cbor(&value)?;
                edges.push(edge);
            }
        } else {
            let iter = self.db.prefix_iterator(&prefix);
            for item in iter {
                let (key, value) =
                    item.map_err(|e| PelagoError::Internal(format!("RocksDB iter: {}", e)))?;
                if !key.starts_with(&prefix) {
                    break;
                }
                let edge: StoredEdge = decode_cbor(&value)?;
                edges.push(edge);
            }
        }
        Ok(edges)
    }

    pub fn get_hwm(&self) -> Result<Versionstamp, PelagoError> {
        match self.get_cf(CF_META, b"_hwm")? {
            Some(bytes) => Versionstamp::from_bytes(&bytes)
                .ok_or_else(|| PelagoError::Internal("Invalid HWM".into())),
            None => Ok(Versionstamp::zero()),
        }
    }

    pub fn set_hwm(&self, vs: &Versionstamp) -> Result<(), PelagoError> {
        let hwm_bytes = vs.to_bytes();
        self.put_cf(CF_META, b"_hwm", hwm_bytes)
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
                    let incoming_key =
                        Self::incoming_edge_key(db, ns, &edge.source, &edge.label, &edge.target);
                    let value = encode_cbor(&edge)?;
                    self.batch_put_cf(&mut batch, CF_EDGE, &key, &value)?;
                    self.batch_put_cf(&mut batch, CF_EDGE, &incoming_key, &value)?;
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
                    let incoming_key = Self::incoming_edge_key(db, ns, &source, edge_type, &target);
                    self.batch_delete_cf(&mut batch, CF_EDGE, &key)?;
                    self.batch_delete_cf(&mut batch, CF_EDGE, &incoming_key)?;
                }
                CdcOperation::SchemaRegister { .. } => {
                    // No cache action for schema registrations.
                }
                CdcOperation::OwnershipTransfer {
                    entity_type,
                    node_id,
                    current_site_id,
                    ..
                } => {
                    // Best-effort locality update for cached node.
                    if let Some(mut node) = self.get_node(db, ns, entity_type, node_id)? {
                        node.locality = current_site_id.parse::<u8>().unwrap_or(node.locality);
                        let key = Self::node_key(db, ns, &node.entity_type, &node.id);
                        let value = encode_cbor(&node)?;
                        self.batch_put_cf(&mut batch, CF_NODE, &key, &value)?;
                    }
                }
            }
        }

        for ((entity_type, node_id), maybe_node) in pending_nodes {
            let key = Self::node_key(db, ns, &entity_type, &node_id);
            match maybe_node {
                Some(node) => {
                    let value = encode_cbor(&node)?;
                    self.batch_put_cf(&mut batch, CF_NODE, &key, &value)?;
                }
                None => self.batch_delete_cf(&mut batch, CF_NODE, &key)?,
            }
        }

        let hwm_bytes = hwm.to_bytes();
        self.batch_put_cf(&mut batch, CF_META, b"_hwm", hwm_bytes)?;
        self.db
            .write(batch)
            .map_err(|e| PelagoError::Internal(format!("RocksDB batch write: {}", e)))
    }

    /// Clear all data (for testing/rebuild)
    pub fn clear(&self) -> Result<(), PelagoError> {
        if self.use_column_families {
            for cf_name in [CF_NODE, CF_EDGE, CF_META] {
                let cf = self.cf_handle(cf_name)?;
                let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);
                for item in iter {
                    let (key, _) =
                        item.map_err(|e| PelagoError::Internal(format!("RocksDB iter: {}", e)))?;
                    self.delete_cf(cf_name, &key)?;
                }
            }
        } else {
            let iter = self.db.iterator(rocksdb::IteratorMode::Start);
            for item in iter {
                let (key, _) =
                    item.map_err(|e| PelagoError::Internal(format!("RocksDB iter: {}", e)))?;
                self.db
                    .delete(&*key)
                    .map_err(|e| PelagoError::Internal(format!("RocksDB delete: {}", e)))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::RocksCacheStore;
    use crate::edge::{NodeRef, StoredEdge};
    use pelago_core::Value;
    use std::collections::HashMap;

    #[test]
    fn incoming_edge_listing_uses_target_index() {
        let store = RocksCacheStore::open_temp().expect("open temp rocks cache");
        let source = NodeRef::new("db", "ns", "Person", "1_1");
        let target = NodeRef::new("db", "ns", "Person", "1_2");
        let edge = StoredEdge::new(
            "e1".to_string(),
            source.clone(),
            target.clone(),
            "KNOWS".to_string(),
            HashMap::from([(String::from("weight"), Value::Int(7))]),
        );

        store
            .put_edge("db", "ns", &edge)
            .expect("write outgoing+incoming edge");

        let outgoing = store
            .list_edges_cached("db", "ns", "Person", "1_1", Some("KNOWS"))
            .expect("list outgoing");
        let incoming = store
            .list_incoming_edges_cached("db", "ns", "Person", "1_2", Some("KNOWS"))
            .expect("list incoming");

        assert_eq!(outgoing.len(), 1);
        assert_eq!(incoming.len(), 1);
        assert_eq!(outgoing[0].edge_id, "e1");
        assert_eq!(incoming[0].edge_id, "e1");
    }

    #[test]
    fn deleting_edge_clears_incoming_projection() {
        let store = RocksCacheStore::open_temp().expect("open temp rocks cache");
        let source = NodeRef::new("db", "ns", "Person", "1_1");
        let target = NodeRef::new("db", "ns", "Person", "1_2");
        let edge = StoredEdge::new(
            "e1".to_string(),
            source.clone(),
            target.clone(),
            "KNOWS".to_string(),
            HashMap::new(),
        );

        store.put_edge("db", "ns", &edge).expect("seed edge");
        store
            .delete_edge("db", "ns", &source, "KNOWS", &target)
            .expect("delete edge");

        let outgoing = store
            .list_edges_cached("db", "ns", "Person", "1_1", Some("KNOWS"))
            .expect("list outgoing");
        let incoming = store
            .list_incoming_edges_cached("db", "ns", "Person", "1_2", Some("KNOWS"))
            .expect("list incoming");

        assert!(outgoing.is_empty());
        assert!(incoming.is_empty());
    }
}
