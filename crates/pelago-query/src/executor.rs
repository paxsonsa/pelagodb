//! Query execution
//!
//! - Execute primary plan against FDB
//! - Apply residual filter to candidates
//! - Stream results with backpressure
//! - Pagination via cursor
//!
//! The executor takes an ExecutionPlan and produces a stream of matching nodes.

use crate::cel::CelEnvironment;
use crate::plan::{ExecutionPlan, IndexPredicate, PredicateValue, QueryPlan};
use pelago_core::encoding::encode_value_for_index;
use pelago_core::{PelagoError, Value};
use pelago_storage::index::markers;
use pelago_storage::{CdcWriter, IdAllocator, NodeStore, PelagoDb, SchemaRegistry, StoredNode, Subspace};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Query executor configuration
pub struct ExecutorConfig {
    /// Maximum results to buffer before applying backpressure
    pub buffer_size: usize,
    /// Default limit if none specified
    pub default_limit: u32,
    /// Maximum limit allowed
    pub max_limit: u32,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            buffer_size: 100,
            default_limit: 1000,
            max_limit: 10000,
        }
    }
}

/// Query executor that runs plans against FDB
pub struct QueryExecutor {
    db: PelagoDb,
    schema_registry: Arc<SchemaRegistry>,
    id_allocator: Arc<IdAllocator>,
    cdc_writer: Arc<CdcWriter>,
    config: ExecutorConfig,
}

impl QueryExecutor {
    /// Create a new query executor
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
            config: ExecutorConfig::default(),
        }
    }

    /// Create with custom config
    pub fn with_config(
        db: PelagoDb,
        schema_registry: Arc<SchemaRegistry>,
        id_allocator: Arc<IdAllocator>,
        cdc_writer: Arc<CdcWriter>,
        config: ExecutorConfig,
    ) -> Self {
        Self {
            db,
            schema_registry,
            id_allocator,
            cdc_writer,
            config,
        }
    }

    /// Execute a query plan and return results
    pub async fn execute(
        &self,
        database: &str,
        namespace: &str,
        plan: &ExecutionPlan,
    ) -> Result<QueryResults, PelagoError> {
        // Get schema for the entity type
        let schema = self
            .schema_registry
            .get_schema(database, namespace, &plan.entity_type)
            .await?
            .ok_or_else(|| PelagoError::UnregisteredType {
                entity_type: plan.entity_type.clone(),
            })?;

        let schema = Arc::new(schema);

        // Compile residual filter if present
        let residual_filter = if let Some(ref filter_expr) = plan.residual_filter {
            let env = CelEnvironment::new(Arc::clone(&schema));
            Some(env.compile(filter_expr)?)
        } else {
            None
        };

        // Determine effective limit
        let limit = plan
            .limit
            .unwrap_or(self.config.default_limit)
            .min(self.config.max_limit) as usize;

        // Execute based on primary plan
        let candidates = match &plan.primary_plan {
            QueryPlan::PointLookup { node_id } => {
                self.execute_point_lookup(database, namespace, &plan.entity_type, node_id)
                    .await?
            }
            QueryPlan::IndexScan {
                property,
                index_type,
                predicate,
            } => {
                self.execute_index_scan(
                    database,
                    namespace,
                    &plan.entity_type,
                    property,
                    *index_type,
                    predicate,
                    limit,
                )
                .await?
            }
            QueryPlan::FullScan => {
                self.execute_full_scan(database, namespace, &plan.entity_type, limit)
                    .await?
            }
        };

        // Apply residual filter
        let mut results = Vec::new();
        for node in candidates {
            if results.len() >= limit {
                break;
            }

            // Apply residual filter if present
            if let Some(ref filter) = residual_filter {
                if !filter.evaluate(&node.properties)? {
                    continue;
                }
            }

            // Apply projection if present
            let node = if let Some(ref fields) = plan.projection {
                project_node(node, fields)
            } else {
                node
            };

            results.push(node);
        }

        // Build cursor for pagination
        let has_more = results.len() >= limit;
        let cursor = if has_more && !results.is_empty() {
            Some(build_cursor(&results.last().unwrap().id))
        } else {
            None
        };

        Ok(QueryResults {
            nodes: results,
            cursor,
            has_more,
        })
    }

    /// Execute a point lookup by node ID
    async fn execute_point_lookup(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        node_id: &str,
    ) -> Result<Vec<StoredNode>, PelagoError> {
        let node_store = NodeStore::new(
            self.db.clone(),
            Arc::clone(&self.schema_registry),
            Arc::clone(&self.id_allocator),
            Arc::clone(&self.cdc_writer),
        );

        match node_store.get_node(database, namespace, entity_type, node_id).await? {
            Some(node) => Ok(vec![node]),
            None => Ok(vec![]),
        }
    }

    /// Execute an index scan
    async fn execute_index_scan(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        property: &str,
        index_type: pelago_core::schema::IndexType,
        predicate: &IndexPredicate,
        limit: usize,
    ) -> Result<Vec<StoredNode>, PelagoError> {
        let subspace = Subspace::namespace(database, namespace);
        let idx_subspace = subspace.index();

        // Build index key prefix
        let marker = match index_type {
            pelago_core::schema::IndexType::Unique => markers::UNIQUE,
            pelago_core::schema::IndexType::Equality => markers::EQUALITY,
            pelago_core::schema::IndexType::Range => markers::RANGE,
            pelago_core::schema::IndexType::None => {
                return Err(PelagoError::Internal("Cannot scan non-indexed property".into()))
            }
        };

        let (range_start, range_end) = match predicate {
            IndexPredicate::Equals(value) => {
                let encoded_value = encode_predicate_value(value)?;
                let key_prefix = idx_subspace
                    .pack()
                    .add_string(entity_type)
                    .add_string(property)
                    .add_marker(marker)
                    .add_raw_bytes(&encoded_value)
                    .build();

                // For equality, scan the exact prefix
                let mut end = key_prefix.to_vec();
                end.push(0xFF);
                (key_prefix.to_vec(), end)
            }
            IndexPredicate::Range { lower, upper } => {
                let start = match lower {
                    Some(bound) => {
                        let encoded = encode_predicate_value(&bound.value)?;
                        let key_builder = idx_subspace
                            .pack()
                            .add_string(entity_type)
                            .add_string(property)
                            .add_marker(marker)
                            .add_raw_bytes(&encoded);
                        let mut key = key_builder.build().to_vec();
                        if !bound.inclusive {
                            key.push(0xFF); // Skip the exact value
                        }
                        key
                    }
                    None => idx_subspace
                        .pack()
                        .add_string(entity_type)
                        .add_string(property)
                        .add_marker(marker)
                        .build()
                        .to_vec(),
                };

                let end = match upper {
                    Some(bound) => {
                        let encoded = encode_predicate_value(&bound.value)?;
                        let key_builder = idx_subspace
                            .pack()
                            .add_string(entity_type)
                            .add_string(property)
                            .add_marker(marker)
                            .add_raw_bytes(&encoded);
                        let mut key = key_builder.build().to_vec();
                        if bound.inclusive {
                            key.push(0xFF); // Include the exact value
                        }
                        key
                    }
                    None => {
                        let mut key = idx_subspace
                            .pack()
                            .add_string(entity_type)
                            .add_string(property)
                            .add_marker(marker)
                            .build()
                            .to_vec();
                        key.push(0xFF);
                        key
                    }
                };

                (start, end)
            }
            IndexPredicate::In(values) => {
                // For IN, we need to do multiple scans
                // For simplicity, just do the first value and filter the rest
                if values.is_empty() {
                    return Ok(vec![]);
                }
                let encoded_value = encode_predicate_value(&values[0])?;
                let key_prefix = idx_subspace
                    .pack()
                    .add_string(entity_type)
                    .add_string(property)
                    .add_marker(marker)
                    .add_raw_bytes(&encoded_value)
                    .build();

                let mut end = key_prefix.to_vec();
                end.push(0xFF);
                (key_prefix.to_vec(), end)
            }
        };

        // Scan index keys
        let index_results = self
            .db
            .get_range(&range_start, &range_end, limit * 2) // Over-fetch to account for filtering
            .await?;

        // Extract node IDs from index entries
        let mut node_ids = Vec::new();
        for (key, value) in index_results {
            let node_id = match index_type {
                pelago_core::schema::IndexType::Unique => {
                    // Value is the node_id
                    value
                }
                pelago_core::schema::IndexType::Equality | pelago_core::schema::IndexType::Range => {
                    // Node ID is the last component of the key
                    // This is a simplification - proper implementation would parse the tuple
                    extract_node_id_from_index_key(&key)?
                }
                pelago_core::schema::IndexType::None => unreachable!(),
            };
            node_ids.push(node_id);
        }

        // Fetch full nodes
        let node_store = NodeStore::new(
            self.db.clone(),
            Arc::clone(&self.schema_registry),
            Arc::clone(&self.id_allocator),
            Arc::clone(&self.cdc_writer),
        );
        let mut nodes = Vec::new();
        for node_id_bytes in node_ids {
            // Parse node ID from bytes (must be exactly 9 bytes)
            if node_id_bytes.len() == 9 {
                let arr: [u8; 9] = node_id_bytes.try_into().unwrap();
                let node_id = pelago_core::NodeId::from_bytes(&arr);
                let id_str = node_id.to_string();
                if let Ok(Some(node)) = node_store
                    .get_node(database, namespace, entity_type, &id_str)
                    .await
                {
                    nodes.push(node);
                }
            }
        }

        Ok(nodes)
    }

    /// Execute a full scan of all nodes of a type
    async fn execute_full_scan(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        limit: usize,
    ) -> Result<Vec<StoredNode>, PelagoError> {
        let subspace = Subspace::namespace(database, namespace);
        let data_subspace = subspace.data();

        // Build key range for all nodes of this type
        let prefix = data_subspace
            .pack()
            .add_string(entity_type)
            .build();

        let mut range_end = prefix.to_vec();
        range_end.push(0xFF);

        // Scan all node keys
        let results = self
            .db
            .get_range(prefix.as_ref(), &range_end, limit * 2)
            .await?;

        // Create NodeStore to decode results
        let node_store = NodeStore::new(
            self.db.clone(),
            Arc::clone(&self.schema_registry),
            Arc::clone(&self.id_allocator),
            Arc::clone(&self.cdc_writer),
        );

        // Decode nodes - need to fetch them individually since results are raw KV
        let mut nodes = Vec::new();
        for (key, _value) in results {
            if nodes.len() >= limit {
                break;
            }

            // Extract node ID from key
            if let Ok(node_id_bytes) = extract_node_id_from_data_key(&key) {
                if node_id_bytes.len() == 9 {
                    let arr: [u8; 9] = node_id_bytes.try_into().unwrap();
                    let node_id = pelago_core::NodeId::from_bytes(&arr);
                    let id_str = node_id.to_string();
                    if let Ok(Some(node)) = node_store
                        .get_node(database, namespace, entity_type, &id_str)
                        .await
                    {
                        nodes.push(node);
                    }
                }
            }
        }

        Ok(nodes)
    }

    /// Execute a streaming query that yields results via channel
    pub async fn execute_streaming(
        &self,
        database: &str,
        namespace: &str,
        plan: &ExecutionPlan,
    ) -> Result<QueryResultStream, PelagoError> {
        let (tx, rx) = mpsc::channel(self.config.buffer_size);

        // Clone what we need for the spawned task
        let db = self.db.clone();
        let schema_registry = Arc::clone(&self.schema_registry);
        let id_allocator = Arc::clone(&self.id_allocator);
        let cdc_writer = Arc::clone(&self.cdc_writer);
        let plan = plan.clone();
        let database = database.to_string();
        let namespace = namespace.to_string();
        let config = ExecutorConfig::default();

        tokio::spawn(async move {
            let executor = QueryExecutor::with_config(db, schema_registry, id_allocator, cdc_writer, config);

            match executor.execute(&database, &namespace, &plan).await {
                Ok(results) => {
                    for node in results.nodes {
                        if tx.send(Ok(node)).await.is_err() {
                            break; // Receiver dropped
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                }
            }
        });

        Ok(QueryResultStream { rx })
    }
}

/// Results from a query execution
pub struct QueryResults {
    /// Matching nodes
    pub nodes: Vec<StoredNode>,
    /// Cursor for next page (if any)
    pub cursor: Option<Vec<u8>>,
    /// Whether there are more results
    pub has_more: bool,
}

/// Streaming query results
pub struct QueryResultStream {
    rx: mpsc::Receiver<Result<StoredNode, PelagoError>>,
}

impl QueryResultStream {
    /// Get the next result from the stream
    pub async fn next(&mut self) -> Option<Result<StoredNode, PelagoError>> {
        self.rx.recv().await
    }
}

/// Project a node to only include specified fields
fn project_node(mut node: StoredNode, fields: &std::collections::HashSet<String>) -> StoredNode {
    node.properties.retain(|k, _| fields.contains(k));
    node
}

/// Build a pagination cursor from a node ID
fn build_cursor(node_id: &str) -> Vec<u8> {
    node_id.as_bytes().to_vec()
}

/// Encode a predicate value for index key construction
fn encode_predicate_value(value: &PredicateValue) -> Result<Vec<u8>, PelagoError> {
    let pelago_value = match value {
        PredicateValue::String(s) => Value::String(s.clone()),
        PredicateValue::Int(n) => Value::Int(*n),
        PredicateValue::Float(f) => Value::Float(*f),
        PredicateValue::Bool(b) => Value::Bool(*b),
        PredicateValue::Timestamp(t) => Value::Timestamp(*t),
    };
    encode_value_for_index(&pelago_value)
}

/// Extract node ID from an index key (for equality/range indexes)
fn extract_node_id_from_index_key(key: &[u8]) -> Result<Vec<u8>, PelagoError> {
    // The node ID is the last 9 bytes of the key (site_id:u8 + seq:u64)
    // This is a simplification - proper implementation would parse the tuple structure
    if key.len() < 9 {
        return Err(PelagoError::Internal("Index key too short".into()));
    }
    Ok(key[key.len() - 9..].to_vec())
}

/// Extract node ID from a data key
fn extract_node_id_from_data_key(key: &[u8]) -> Result<Vec<u8>, PelagoError> {
    // The node ID is the last 9 bytes of the key
    if key.len() < 9 {
        return Err(PelagoError::Internal("Data key too short".into()));
    }
    Ok(key[key.len() - 9..].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_cursor() {
        let cursor = build_cursor("1_100");
        assert_eq!(cursor, b"1_100");
    }

    #[test]
    fn test_encode_predicate_value() {
        let result = encode_predicate_value(&PredicateValue::String("test".into())).unwrap();
        assert_eq!(result, b"test");

        let result = encode_predicate_value(&PredicateValue::Int(42)).unwrap();
        assert_eq!(result.len(), 8); // i64 encoded as 8 bytes
    }
}
