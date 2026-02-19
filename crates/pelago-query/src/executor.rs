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
use pelago_core::{NodeId, PelagoError, Value};
use pelago_storage::index::markers;
use pelago_storage::term_index;
use pelago_storage::{IdAllocator, NodeStore, PelagoDb, SchemaRegistry, StoredNode, Subspace};
use std::collections::HashSet;
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
    site_id: String,
    config: ExecutorConfig,
}

struct ScanCandidate {
    node: StoredNode,
    cursor_key: Option<Vec<u8>>,
}

impl QueryExecutor {
    /// Create a new query executor
    pub fn new(
        db: PelagoDb,
        schema_registry: Arc<SchemaRegistry>,
        id_allocator: Arc<IdAllocator>,
        site_id: String,
    ) -> Self {
        Self {
            db,
            schema_registry,
            id_allocator,
            site_id,
            config: ExecutorConfig::default(),
        }
    }

    /// Create with custom config
    pub fn with_config(
        db: PelagoDb,
        schema_registry: Arc<SchemaRegistry>,
        id_allocator: Arc<IdAllocator>,
        site_id: String,
        config: ExecutorConfig,
    ) -> Self {
        Self {
            db,
            schema_registry,
            id_allocator,
            site_id,
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
        let fetch_limit = limit.saturating_add(1);
        let scan_limit = fetch_limit
            .saturating_mul(4)
            .min(self.config.max_limit as usize);
        let point_cursor_node_id = if matches!(plan.primary_plan, QueryPlan::PointLookup { .. }) {
            plan.cursor.as_deref().map(parse_node_cursor).transpose()?
        } else {
            None
        };

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
                    plan.cursor.as_deref(),
                    scan_limit.max(1),
                )
                .await?
            }
            QueryPlan::FullScan => {
                self.execute_full_scan(
                    database,
                    namespace,
                    &plan.entity_type,
                    plan.cursor.as_deref(),
                    scan_limit.max(1),
                )
                .await?
            }
        };
        let candidate_count = candidates.len();

        // Apply residual filter
        let mut results = Vec::new();
        let mut last_cursor: Option<Vec<u8>> = None;
        for candidate in candidates {
            let mut node = candidate.node;

            // Apply residual filter if present
            if let Some(ref filter) = residual_filter {
                if !filter.evaluate(&node.properties)? {
                    continue;
                }
            }

            // Point lookup fallback cursor behavior remains node-id based.
            if let Some(cursor) = point_cursor_node_id.as_ref() {
                if !node_id_is_after_cursor(&node.id, cursor) {
                    continue;
                }
            }

            // Apply projection if present
            if let Some(ref fields) = plan.projection {
                node = project_node(node, fields);
            }

            last_cursor = Some(
                candidate
                    .cursor_key
                    .unwrap_or_else(|| build_cursor(&node.id)),
            );
            results.push(node);
            if results.len() >= fetch_limit {
                break;
            }
        }

        // Build cursor for pagination
        let has_more = results.len() > limit;
        if has_more {
            results.truncate(limit);
        }
        let cursor = if has_more && !results.is_empty() {
            last_cursor
        } else {
            None
        };

        tracing::debug!(
            entity_type = %plan.entity_type,
            strategy = %plan.primary_plan.strategy_name(),
            candidates_scanned = candidate_count,
            rows_returned = results.len(),
            has_more,
            cursor_used = plan.cursor.is_some(),
            "query execution summary"
        );

        Ok(QueryResults {
            nodes: results,
            cursor,
            has_more,
        })
    }

    /// Execute a simple equality-based boolean expression using term postings.
    ///
    /// Supports expressions composed of:
    /// - `field == value`
    /// - conjunctions with `&&`
    /// - disjunctions with `||`
    ///
    /// Parentheses and non-equality operators are intentionally not handled here.
    /// Returns `Ok(None)` when the expression isn't supported by this fast path.
    pub async fn execute_term_expression(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        expression: &str,
        projection: Option<HashSet<String>>,
        limit: Option<u32>,
        cursor: Option<&[u8]>,
    ) -> Result<Option<QueryResults>, PelagoError> {
        let parsed = match parse_term_expression(expression) {
            Some(p) => p,
            None => return Ok(None),
        };

        let effective_limit = limit
            .unwrap_or(self.config.default_limit)
            .min(self.config.max_limit) as usize;
        let fetch_limit = effective_limit.saturating_add(1);
        let cursor_node_id = cursor.map(parse_node_cursor).transpose()?;

        let node_ids = self
            .execute_term_groups(
                database,
                namespace,
                entity_type,
                &parsed.groups,
                cursor_node_id.as_ref(),
                fetch_limit,
            )
            .await?;
        let candidate_count = node_ids.len();

        let node_store = NodeStore::new(
            self.db.clone(),
            Arc::clone(&self.schema_registry),
            Arc::clone(&self.id_allocator),
            self.site_id.clone(),
        );

        let mut nodes = Vec::new();
        for node_id_bytes in node_ids {
            if nodes.len() >= fetch_limit {
                break;
            }
            if node_id_bytes.len() != 9 {
                continue;
            }
            if let Some(cursor) = cursor_node_id.as_ref() {
                if node_id_bytes.as_slice() <= cursor.as_slice() {
                    continue;
                }
            }
            let arr: [u8; 9] = node_id_bytes.try_into().unwrap();
            let node_id = NodeId::from_bytes(&arr).to_string();
            if let Some(mut node) = node_store
                .get_node(database, namespace, entity_type, &node_id)
                .await?
            {
                if let Some(ref fields) = projection {
                    node = project_node(node, fields);
                }
                nodes.push(node);
            }
        }

        let has_more = nodes.len() > effective_limit;
        if has_more {
            nodes.truncate(effective_limit);
        }
        let cursor = if has_more && !nodes.is_empty() {
            Some(build_cursor(&nodes.last().unwrap().id))
        } else {
            None
        };

        tracing::debug!(
            entity_type,
            expression,
            groups = parsed.groups.len(),
            candidates_scanned = candidate_count,
            rows_returned = nodes.len(),
            has_more,
            cursor_used = cursor_node_id.is_some(),
            "term-expression query summary"
        );

        Ok(Some(QueryResults {
            nodes,
            cursor,
            has_more,
        }))
    }

    /// Execute a point lookup by node ID
    async fn execute_point_lookup(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        node_id: &str,
    ) -> Result<Vec<ScanCandidate>, PelagoError> {
        let node_store = NodeStore::new(
            self.db.clone(),
            Arc::clone(&self.schema_registry),
            Arc::clone(&self.id_allocator),
            self.site_id.clone(),
        );

        match node_store
            .get_node(database, namespace, entity_type, node_id)
            .await?
        {
            Some(node) => Ok(vec![ScanCandidate {
                node,
                cursor_key: None,
            }]),
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
        cursor: Option<&[u8]>,
        limit: usize,
    ) -> Result<Vec<ScanCandidate>, PelagoError> {
        let subspace = Subspace::namespace(database, namespace);
        let idx_subspace = subspace.index();

        // Build index key prefix
        let marker = match index_type {
            pelago_core::schema::IndexType::Unique => markers::UNIQUE,
            pelago_core::schema::IndexType::Equality => markers::EQUALITY,
            pelago_core::schema::IndexType::Range => markers::RANGE,
            pelago_core::schema::IndexType::None => {
                return Err(PelagoError::Internal(
                    "Cannot scan non-indexed property".into(),
                ))
            }
        };

        let (mut range_start, range_end) = match predicate {
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
                    return Ok(Vec::new());
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

        if let Some(cursor) = cursor {
            range_start = make_exclusive_start(cursor);
        }

        // Scan index keys
        let index_results = self
            .db
            .get_range(&range_start, &range_end, limit.max(1))
            .await?;

        // Extract node IDs from index entries
        let mut keyed_node_ids: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        for (key, value) in index_results {
            let node_id = match index_type {
                pelago_core::schema::IndexType::Unique => {
                    // Value is the node_id
                    value
                }
                pelago_core::schema::IndexType::Equality
                | pelago_core::schema::IndexType::Range => {
                    // Node ID is the last component of the key
                    // This is a simplification - proper implementation would parse the tuple
                    extract_node_id_from_index_key(&key)?
                }
                pelago_core::schema::IndexType::None => unreachable!(),
            };
            keyed_node_ids.push((key, node_id));
        }

        // Fetch full nodes
        let node_store = NodeStore::new(
            self.db.clone(),
            Arc::clone(&self.schema_registry),
            Arc::clone(&self.id_allocator),
            self.site_id.clone(),
        );
        let mut nodes = Vec::new();
        for (index_key, node_id_bytes) in keyed_node_ids {
            // Parse node ID from bytes (must be exactly 9 bytes)
            if node_id_bytes.len() == 9 {
                let arr: [u8; 9] = node_id_bytes.try_into().unwrap();
                let node_id = NodeId::from_bytes(&arr);
                let id_str = node_id.to_string();
                if let Ok(Some(node)) = node_store
                    .get_node(database, namespace, entity_type, &id_str)
                    .await
                {
                    nodes.push(ScanCandidate {
                        node,
                        cursor_key: Some(index_key),
                    });
                }
            }
        }

        Ok(nodes)
    }

    async fn execute_term_groups(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        groups: &[Vec<TermPredicate>],
        after_node_id: Option<&[u8; 9]>,
        target_limit: usize,
    ) -> Result<Vec<Vec<u8>>, PelagoError> {
        // Keep bounded memory for OR unions while still allowing broad result sets.
        let hard_cap = (self.config.max_limit as usize)
            .saturating_mul(20)
            .max(5_000);
        let mut union: HashSet<Vec<u8>> = HashSet::new();

        for group in groups {
            let group_ids = self
                .execute_term_group(
                    database,
                    namespace,
                    entity_type,
                    group,
                    after_node_id,
                    target_limit,
                    hard_cap,
                )
                .await?;
            for id in group_ids {
                if union.len() >= hard_cap {
                    break;
                }
                union.insert(id);
            }
            if union.len() >= hard_cap {
                break;
            }
        }

        let mut out: Vec<Vec<u8>> = union.into_iter().collect();
        out.sort();
        Ok(out)
    }

    async fn execute_term_group(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        group: &[TermPredicate],
        after_node_id: Option<&[u8; 9]>,
        target_limit: usize,
        hard_cap: usize,
    ) -> Result<Vec<Vec<u8>>, PelagoError> {
        if group.is_empty() {
            return Ok(Vec::new());
        }

        // Selectivity-first ordering using DF counters when present.
        let mut ordered_terms = Vec::with_capacity(group.len());
        for term in group {
            let df = term_index::get_document_frequency(
                &self.db,
                database,
                namespace,
                entity_type,
                &term.field,
                &term.value,
            )
            .await?
            .unwrap_or(u64::MAX / 4);
            ordered_terms.push((df, term));
        }
        ordered_terms.sort_by_key(|(df, _)| *df);

        let mut running: Option<HashSet<Vec<u8>>> = None;
        for (df, term) in ordered_terms {
            let fetch_limit = estimate_posting_fetch_limit(df, target_limit, hard_cap);
            let posting_ids = term_index::list_posting_node_ids_after(
                &self.db,
                database,
                namespace,
                entity_type,
                &term.field,
                &term.value,
                after_node_id.map(|id| id.as_slice()),
                fetch_limit,
            )
            .await?;
            let posting_set: HashSet<Vec<u8>> = posting_ids.into_iter().collect();

            running = Some(match running {
                None => posting_set,
                Some(prev) => prev.intersection(&posting_set).cloned().collect(),
            });

            if running.as_ref().is_some_and(|s| s.is_empty()) {
                break;
            }
        }

        Ok(running.unwrap_or_default().into_iter().collect())
    }

    /// Execute a full scan of all nodes of a type
    async fn execute_full_scan(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        cursor: Option<&[u8]>,
        limit: usize,
    ) -> Result<Vec<ScanCandidate>, PelagoError> {
        let subspace = Subspace::namespace(database, namespace);
        let data_subspace = subspace.data();

        // Build key range for all nodes of this type
        let prefix = data_subspace.pack().add_string(entity_type).build();
        let range_start = match cursor {
            Some(c) => make_exclusive_start(c),
            None => prefix.to_vec(),
        };

        let mut range_end = prefix.to_vec();
        range_end.push(0xFF);

        // Scan all node keys
        let results = self
            .db
            .get_range(&range_start, &range_end, limit.max(1))
            .await?;

        // Create NodeStore to decode results
        let node_store = NodeStore::new(
            self.db.clone(),
            Arc::clone(&self.schema_registry),
            Arc::clone(&self.id_allocator),
            self.site_id.clone(),
        );

        // Decode nodes - need to fetch them individually since results are raw KV
        let mut nodes = Vec::new();
        for (key, _value) in results {
            // Extract node ID from key
            if let Ok(node_id_bytes) = extract_node_id_from_data_key(&key) {
                if node_id_bytes.len() == 9 {
                    let arr: [u8; 9] = node_id_bytes.try_into().unwrap();
                    let node_id = NodeId::from_bytes(&arr);
                    let id_str = node_id.to_string();
                    if let Ok(Some(node)) = node_store
                        .get_node(database, namespace, entity_type, &id_str)
                        .await
                    {
                        nodes.push(ScanCandidate {
                            node,
                            cursor_key: Some(key),
                        });
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
        let site_id = self.site_id.clone();
        let plan = plan.clone();
        let database = database.to_string();
        let namespace = namespace.to_string();
        let config = ExecutorConfig::default();

        tokio::spawn(async move {
            let executor =
                QueryExecutor::with_config(db, schema_registry, id_allocator, site_id, config);

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
    node_id
        .parse::<NodeId>()
        .map(|id| id.to_bytes().to_vec())
        .unwrap_or_else(|_| node_id.as_bytes().to_vec())
}

fn parse_node_cursor(cursor: &[u8]) -> Result<[u8; 9], PelagoError> {
    if cursor.len() == 9 {
        let mut out = [0u8; 9];
        out.copy_from_slice(cursor);
        return Ok(out);
    }

    let as_str = std::str::from_utf8(cursor).map_err(|_| PelagoError::InvalidValue {
        field: "cursor".to_string(),
        reason: "cursor must be a 9-byte node id or node id string".to_string(),
    })?;
    let parsed = as_str
        .parse::<NodeId>()
        .map_err(|_| PelagoError::InvalidValue {
            field: "cursor".to_string(),
            reason: "cursor is not a valid node id".to_string(),
        })?;
    Ok(parsed.to_bytes())
}

fn node_id_is_after_cursor(node_id: &str, cursor: &[u8; 9]) -> bool {
    node_id
        .parse::<NodeId>()
        .map(|id| id.to_bytes().as_slice() > cursor.as_slice())
        .unwrap_or(true)
}

fn make_exclusive_start(key: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(key.len() + 1);
    out.extend_from_slice(key);
    out.push(0x00);
    out
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

#[derive(Debug, Clone)]
struct TermPredicate {
    field: String,
    value: Value,
}

#[derive(Debug, Clone)]
struct ParsedTermExpression {
    // OR groups of AND predicates
    groups: Vec<Vec<TermPredicate>>,
}

fn parse_term_expression(expression: &str) -> Option<ParsedTermExpression> {
    let expression = expression.trim();
    if expression.is_empty() {
        return None;
    }

    // Parenthesized expressions should go through the normal planner/executor path.
    if expression.contains('(') || expression.contains(')') {
        return None;
    }

    let mut groups = Vec::new();
    for or_part in expression.split("||") {
        let or_part = or_part.trim();
        if or_part.is_empty() {
            return None;
        }

        let mut group = Vec::new();
        for and_part in or_part.split("&&") {
            let and_part = and_part.trim();
            if and_part.is_empty() {
                return None;
            }
            let predicate = parse_term_predicate(and_part)?;
            group.push(predicate);
        }
        groups.push(group);
    }

    if groups.is_empty() {
        None
    } else {
        Some(ParsedTermExpression { groups })
    }
}

fn parse_term_predicate(expr: &str) -> Option<TermPredicate> {
    if expr.contains("!=")
        || expr.contains(">=")
        || expr.contains("<=")
        || expr.contains('>')
        || expr.contains('<')
    {
        return None;
    }

    let pos = expr.find("==")?;
    let field = expr[..pos].trim();
    let value_str = expr[pos + 2..].trim();

    if field.is_empty() || field.contains(' ') {
        return None;
    }

    let value = parse_term_value(value_str)?;
    Some(TermPredicate {
        field: field.to_string(),
        value,
    })
}

fn parse_term_value(raw: &str) -> Option<Value> {
    let raw = raw.trim();

    if raw.len() < 2 {
        if let Ok(i) = raw.parse::<i64>() {
            return Some(Value::Int(i));
        }
        return None;
    }

    if (raw.starts_with('"') && raw.ends_with('"'))
        || (raw.starts_with('\'') && raw.ends_with('\''))
    {
        return Some(Value::String(raw[1..raw.len() - 1].to_string()));
    }
    if raw == "true" {
        return Some(Value::Bool(true));
    }
    if raw == "false" {
        return Some(Value::Bool(false));
    }
    if raw.contains('.') {
        if let Ok(f) = raw.parse::<f64>() {
            return Some(Value::Float(f));
        }
    }
    if let Ok(i) = raw.parse::<i64>() {
        return Some(Value::Int(i));
    }

    None
}

fn estimate_posting_fetch_limit(df: u64, target_limit: usize, hard_cap: usize) -> usize {
    let baseline = target_limit.saturating_mul(16).max(1024);
    let estimated = if df >= (u64::MAX / 8) {
        baseline
    } else {
        (df as usize).max(baseline)
    };
    estimated.min(hard_cap).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_cursor() {
        let cursor = build_cursor("1_100");
        assert_eq!(cursor.len(), 9);
        assert_eq!(cursor[0], 1);
        assert_eq!(cursor[8], 100);
    }

    #[test]
    fn test_parse_node_cursor_string_and_raw() {
        let from_string = parse_node_cursor(b"2_55").unwrap();
        assert_eq!(from_string[0], 2);
        assert_eq!(u64::from_be_bytes(from_string[1..].try_into().unwrap()), 55);

        let raw = [3, 0, 0, 0, 0, 0, 0, 0, 9];
        let from_raw = parse_node_cursor(&raw).unwrap();
        assert_eq!(from_raw, raw);
    }

    #[test]
    fn test_encode_predicate_value() {
        let result = encode_predicate_value(&PredicateValue::String("test".into())).unwrap();
        assert_eq!(result, b"test");

        let result = encode_predicate_value(&PredicateValue::Int(42)).unwrap();
        assert_eq!(result.len(), 8); // i64 encoded as 8 bytes
    }

    #[test]
    fn test_parse_term_expression_and_or() {
        let parsed = parse_term_expression("shot == 's1' && task == 'fx' || shot == 's2'").unwrap();
        assert_eq!(parsed.groups.len(), 2);
        assert_eq!(parsed.groups[0].len(), 2);
        assert_eq!(parsed.groups[1].len(), 1);
    }

    #[test]
    fn test_parse_term_expression_rejects_parentheses() {
        assert!(parse_term_expression("(shot == 's1') && task == 'fx'").is_none());
    }

    #[test]
    fn test_parse_term_expression_rejects_non_equality() {
        assert!(parse_term_expression("age >= 30").is_none());
    }
}
