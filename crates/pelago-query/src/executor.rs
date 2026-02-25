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
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::mpsc;

const NODE_BATCH_FETCH_SIZE: usize = 256;
const TEMPLATE_FIELD_SHOW_SCHEME_SHOT_SEQUENCE: &str = "__tpl_show_scheme_shot_sequence";
const TEMPLATE_FIELD_SHOW_SCHEME_SHOT_SEQUENCE_TASK_LABEL: &str =
    "__tpl_show_scheme_shot_sequence_task_label";

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
        let groups = rewrite_template_term_groups(parsed.groups);

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
                &groups,
                cursor_node_id.as_ref(),
                fetch_limit,
            )
            .await?;
        let candidate_count = node_ids.len();

        let mut ordered_node_ids = Vec::with_capacity(node_ids.len());
        for node_id_bytes in node_ids {
            if node_id_bytes.len() != 9 {
                continue;
            }
            if let Some(cursor) = cursor_node_id.as_ref() {
                if node_id_bytes.as_slice() <= cursor.as_slice() {
                    continue;
                }
            }
            let arr: [u8; 9] = node_id_bytes.try_into().unwrap();
            ordered_node_ids.push(NodeId::from_bytes(&arr).to_string());
            if ordered_node_ids.len() >= fetch_limit {
                break;
            }
        }

        let mut nodes = self
            .fetch_nodes_in_order(database, namespace, entity_type, &ordered_node_ids)
            .await?;
        if let Some(ref fields) = projection {
            nodes = nodes
                .into_iter()
                .map(|node| project_node(node, fields))
                .collect();
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
            groups = groups.len(),
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
        let mut keyed_node_ids: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        match predicate {
            IndexPredicate::Equals(value) => {
                let encoded_value = encode_predicate_value(value)?;
                let key_prefix = idx_subspace
                    .pack()
                    .add_string(entity_type)
                    .add_string(property)
                    .add_marker(marker)
                    .add_raw_bytes(&encoded_value)
                    .build();

                let mut range_start = key_prefix.to_vec();
                let mut range_end = key_prefix.to_vec();
                range_end.push(0xFF);
                if let Some(cursor) = cursor {
                    range_start = make_exclusive_start(cursor);
                }
                let index_results = self
                    .db
                    .get_range(&range_start, &range_end, limit.max(1))
                    .await?;
                for (key, value) in index_results {
                    let node_id = match index_type {
                        pelago_core::schema::IndexType::Unique => value,
                        pelago_core::schema::IndexType::Equality
                        | pelago_core::schema::IndexType::Range => {
                            extract_node_id_from_index_key(&key)?
                        }
                        pelago_core::schema::IndexType::None => unreachable!(),
                    };
                    keyed_node_ids.push((key, node_id));
                }
            }
            IndexPredicate::Range { lower, upper } => {
                let mut range_start = match lower {
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
                let range_end = match upper {
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

                if let Some(cursor) = cursor {
                    range_start = make_exclusive_start(cursor);
                }
                let index_results = self
                    .db
                    .get_range(&range_start, &range_end, limit.max(1))
                    .await?;
                for (key, value) in index_results {
                    let node_id = match index_type {
                        pelago_core::schema::IndexType::Unique => value,
                        pelago_core::schema::IndexType::Equality
                        | pelago_core::schema::IndexType::Range => {
                            extract_node_id_from_index_key(&key)?
                        }
                        pelago_core::schema::IndexType::None => unreachable!(),
                    };
                    keyed_node_ids.push((key, node_id));
                }
            }
            IndexPredicate::In(values) => {
                if values.is_empty() {
                    return Ok(Vec::new());
                }

                let cursor_node_id = cursor.map(parse_node_cursor).transpose()?;
                let hard_cap = limit.max(1).saturating_mul(8).max(512);
                let per_value_limit = (hard_cap / values.len().max(1)).max(limit.max(1));
                let mut merged_by_node_id: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();

                for value in values {
                    let encoded_value = encode_predicate_value(value)?;
                    let key_prefix = idx_subspace
                        .pack()
                        .add_string(entity_type)
                        .add_string(property)
                        .add_marker(marker)
                        .add_raw_bytes(&encoded_value)
                        .build();
                    let mut end = key_prefix.to_vec();
                    end.push(0xFF);

                    let rows = self
                        .db
                        .get_range(&key_prefix, &end, per_value_limit.max(1))
                        .await?;
                    for (key, value_bytes) in rows {
                        let node_id = match index_type {
                            pelago_core::schema::IndexType::Unique => value_bytes,
                            pelago_core::schema::IndexType::Equality
                            | pelago_core::schema::IndexType::Range => {
                                extract_node_id_from_index_key(&key)?
                            }
                            pelago_core::schema::IndexType::None => unreachable!(),
                        };
                        if node_id.len() != 9 {
                            continue;
                        }
                        if let Some(cursor_id) = cursor_node_id.as_ref() {
                            if node_id.as_slice() <= cursor_id.as_slice() {
                                continue;
                            }
                        }
                        // For IN scans we keyset-page by node_id bytes to keep
                        // ordering stable across multiple per-value scans.
                        merged_by_node_id
                            .entry(node_id.clone())
                            .or_insert_with(|| node_id.clone());
                        if merged_by_node_id.len() >= hard_cap {
                            break;
                        }
                    }
                    if merged_by_node_id.len() >= hard_cap {
                        break;
                    }
                }

                keyed_node_ids.extend(
                    merged_by_node_id
                        .into_iter()
                        .map(|(node_id, cursor_key)| (cursor_key, node_id)),
                );
            }
        }

        let mut keyed_node_ids_text = Vec::with_capacity(keyed_node_ids.len());
        for (cursor_key, node_id_bytes) in keyed_node_ids {
            if node_id_bytes.len() != 9 {
                continue;
            }
            let arr: [u8; 9] = node_id_bytes.try_into().unwrap();
            keyed_node_ids_text.push((cursor_key, NodeId::from_bytes(&arr).to_string()));
        }
        self.fetch_scan_candidates_in_order(database, namespace, entity_type, &keyed_node_ids_text)
            .await
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

        let mut keyed_node_ids = Vec::with_capacity(results.len());
        for (key, _value) in results {
            if let Ok(node_id_bytes) = extract_node_id_from_data_key(&key) {
                if node_id_bytes.len() == 9 {
                    let arr: [u8; 9] = node_id_bytes.try_into().unwrap();
                    keyed_node_ids.push((key, NodeId::from_bytes(&arr).to_string()));
                }
            }
        }

        self.fetch_scan_candidates_in_order(database, namespace, entity_type, &keyed_node_ids)
            .await
    }

    async fn fetch_nodes_in_order(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        node_ids: &[String],
    ) -> Result<Vec<StoredNode>, PelagoError> {
        if node_ids.is_empty() {
            return Ok(Vec::new());
        }

        let node_store = NodeStore::new(
            self.db.clone(),
            Arc::clone(&self.schema_registry),
            Arc::clone(&self.id_allocator),
            self.site_id.clone(),
        );

        let mut out = Vec::with_capacity(node_ids.len());
        for chunk in node_ids.chunks(NODE_BATCH_FETCH_SIZE) {
            let ids: Vec<String> = chunk.to_vec();
            let rows = node_store
                .get_nodes_batch(database, namespace, entity_type, &ids)
                .await?;
            for row in rows {
                if let Some(node) = row {
                    out.push(node);
                }
            }
        }

        Ok(out)
    }

    async fn fetch_scan_candidates_in_order(
        &self,
        database: &str,
        namespace: &str,
        entity_type: &str,
        keyed_node_ids: &[(Vec<u8>, String)],
    ) -> Result<Vec<ScanCandidate>, PelagoError> {
        if keyed_node_ids.is_empty() {
            return Ok(Vec::new());
        }

        let node_store = NodeStore::new(
            self.db.clone(),
            Arc::clone(&self.schema_registry),
            Arc::clone(&self.id_allocator),
            self.site_id.clone(),
        );

        let mut out = Vec::with_capacity(keyed_node_ids.len());
        for chunk in keyed_node_ids.chunks(NODE_BATCH_FETCH_SIZE) {
            let ids: Vec<String> = chunk.iter().map(|(_, id)| id.clone()).collect();
            let rows = node_store
                .get_nodes_batch(database, namespace, entity_type, &ids)
                .await?;

            for ((cursor_key, _), row) in chunk.iter().zip(rows.into_iter()) {
                if let Some(node) = row {
                    out.push(ScanCandidate {
                        node,
                        cursor_key: Some(cursor_key.clone()),
                    });
                }
            }
        }

        Ok(out)
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

fn rewrite_template_term_groups(groups: Vec<Vec<TermPredicate>>) -> Vec<Vec<TermPredicate>> {
    groups
        .into_iter()
        .map(rewrite_template_term_group)
        .collect()
}

fn rewrite_template_term_group(group: Vec<TermPredicate>) -> Vec<TermPredicate> {
    let mut string_values: HashMap<String, String> = HashMap::new();
    for term in &group {
        let Value::String(s) = &term.value else {
            continue;
        };
        if let Some(existing) = string_values.get(&term.field) {
            if existing != s {
                // Conflicting equality terms; keep original group unchanged.
                return group;
            }
        } else {
            string_values.insert(term.field.clone(), s.clone());
        }
    }

    let show = string_values.get("show");
    let scheme = string_values.get("scheme");
    let shot = string_values.get("shot");
    let sequence = string_values.get("sequence");

    let mut consumed = HashSet::new();
    let mut synthetic = Vec::new();
    if let (Some(show), Some(scheme), Some(shot), Some(sequence)) = (show, scheme, shot, sequence) {
        if let (Some(task), Some(label)) = (string_values.get("task"), string_values.get("label")) {
            synthetic.push(TermPredicate {
                field: TEMPLATE_FIELD_SHOW_SCHEME_SHOT_SEQUENCE_TASK_LABEL.to_string(),
                value: Value::String(format!("{show}|{scheme}|{shot}|{sequence}|{task}|{label}")),
            });
            consumed.extend([
                "show".to_string(),
                "scheme".to_string(),
                "shot".to_string(),
                "sequence".to_string(),
                "task".to_string(),
                "label".to_string(),
            ]);
        } else {
            synthetic.push(TermPredicate {
                field: TEMPLATE_FIELD_SHOW_SCHEME_SHOT_SEQUENCE.to_string(),
                value: Value::String(format!("{show}|{scheme}|{shot}|{sequence}")),
            });
            consumed.extend([
                "show".to_string(),
                "scheme".to_string(),
                "shot".to_string(),
                "sequence".to_string(),
            ]);
        }
    }

    if synthetic.is_empty() {
        return group;
    }

    let mut out = Vec::with_capacity(group.len() + synthetic.len());
    out.extend(synthetic);
    out.extend(
        group
            .into_iter()
            .filter(|term| !consumed.contains(&term.field)),
    );
    out
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

    #[test]
    fn test_parse_term_expression_accepts_context_template_shape() {
        let parsed = parse_term_expression(
            "show == 'show_001' && scheme == 'main' && shot == 'shot_0001' && task == 'fx' && label == 'default'",
        )
        .unwrap();

        assert_eq!(parsed.groups.len(), 1);
        assert_eq!(parsed.groups[0].len(), 5);
        assert_eq!(parsed.groups[0][0].field, "show");
        assert_eq!(parsed.groups[0][4].field, "label");
    }

    #[test]
    fn test_rewrite_template_term_group_collapses_context_template() {
        let parsed = parse_term_expression(
            "show == 'show_001' && scheme == 'main' && shot == 'shot_0001' && sequence == 'seq_001' && task == 'fx' && label == 'default' && status == 'active'",
        )
        .unwrap();
        let rewritten = rewrite_template_term_groups(parsed.groups);
        assert_eq!(rewritten.len(), 1);
        assert_eq!(rewritten[0].len(), 2);
        assert_eq!(
            rewritten[0][0].field,
            TEMPLATE_FIELD_SHOW_SCHEME_SHOT_SEQUENCE_TASK_LABEL
        );
        assert_eq!(rewritten[0][1].field, "status");
    }

    #[test]
    fn test_estimate_posting_fetch_limit_respects_hard_cap() {
        let limit = estimate_posting_fetch_limit(10_000_000, 100, 4096);
        assert_eq!(limit, 4096);

        let minimum = estimate_posting_fetch_limit(1, 0, 128);
        assert_eq!(minimum, 128);
    }
}
