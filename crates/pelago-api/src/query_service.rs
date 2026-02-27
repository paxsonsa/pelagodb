//! QueryService gRPC handler
//!
//! Handles query operations:
//! - FindNodes: Stream nodes matching a CEL expression
//! - Traverse: Graph traversal with multi-hop paths
//! - Explain: Query plan explanation

use crate::authz::{authorize, principal_from_request};
use crate::error::ToStatus;
use crate::schema_service::core_to_proto_properties;
use cel_interpreter::{Context as CelContext, Program as CelProgram, Value as CelValue};
use pelago_core::PelagoError;
use pelago_core::Value as CoreValue;
use pelago_proto::{
    query_service_server::QueryService, Edge, EdgeDirection, ExecutePqlRequest, ExplainRequest,
    ExplainResponse, FindNodesRequest, IndexOperation, Node, NodeRef as ProtoNodeRef, NodeResult,
    PqlResult, QueryPlan, SnapshotConsistencyApplied, SnapshotMode, TraverseRequest,
    TraverseResult,
};
use pelago_query::plan::QueryExplanation;
use pelago_query::planner::QueryPlanner;
use pelago_query::pql::{
    explain_query, parse_pql, InMemorySchemaProvider, PqlCompiler, PqlParseError, PqlResolver,
    SchemaInfo, SetOp,
};
use pelago_query::traversal::{
    TraversalConfig, TraversalDirection, TraversalEngine, TraversalHop, TraversalReadOptions,
};
use pelago_query::{QueryExecutor, ReadExecutionOptions};
use pelago_storage::{
    IdAllocator, NodeRef, NodeStore, PelagoDb, SchemaRegistry, StoredEdge, StoredNode,
};
use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::Arc;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

const OFFSET_CURSOR_LEN: usize = 8;
const NODE_ID_CURSOR_LEN: usize = 9;
const KEYSET_CURSOR_VERSION: u8 = 1;
const C5_STRICT_MAX_SNAPSHOT_MS: u64 = 2_000;
const C5_STRICT_MAX_SCAN_KEYS: usize = 50_000;
const C5_STRICT_MAX_RESULT_BYTES: usize = 8 * 1024 * 1024;

enum FindNodesCursor {
    Offset(usize),
    Keyset(Option<Vec<u8>>),
}

#[derive(Clone)]
struct SnapshotResponseMeta {
    consistency_applied: SnapshotConsistencyApplied,
    snapshot_read_version: Option<i64>,
    degraded: bool,
    degraded_reason: String,
}

/// Query service implementation
pub struct QueryServiceImpl {
    query_executor: QueryExecutor,
    schema_registry: Arc<SchemaRegistry>,
    db: PelagoDb,
    id_allocator: Arc<IdAllocator>,
    site_id: String,
}

impl QueryServiceImpl {
    pub fn new(
        db: PelagoDb,
        schema_registry: Arc<SchemaRegistry>,
        id_allocator: Arc<IdAllocator>,
        site_id: String,
    ) -> Self {
        Self {
            query_executor: QueryExecutor::new(
                db.clone(),
                Arc::clone(&schema_registry),
                Arc::clone(&id_allocator),
                site_id.clone(),
            ),
            schema_registry,
            db,
            id_allocator,
            site_id,
        }
    }
}

#[tonic::async_trait]
impl QueryService for QueryServiceImpl {
    type FindNodesStream = Pin<Box<dyn Stream<Item = Result<NodeResult, Status>> + Send>>;

    async fn find_nodes(
        &self,
        request: Request<FindNodesRequest>,
    ) -> Result<Response<Self::FindNodesStream>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        let entity_type = req.entity_type;
        authorize(
            &self.db,
            principal.as_ref(),
            "query.find",
            &ctx.database,
            &ctx.namespace,
            &entity_type,
        )
        .await?;
        let cel_expression = req.cel_expression;
        let snapshot_mode = default_find_nodes_snapshot_mode(req.snapshot_mode);
        let allow_degrade = req.allow_degrade_to_best_effort;
        let strict_read_version = if snapshot_mode == SnapshotMode::Strict {
            Some(
                self.db
                    .get_read_version()
                    .await
                    .map_err(|e| e.into_status())?,
            )
        } else {
            None
        };
        let read_options = ReadExecutionOptions {
            read_version: strict_read_version,
        };

        let page_size = if req.limit > 0 {
            req.limit as usize
        } else {
            1000
        };
        let cursor_mode = decode_find_nodes_cursor(&req.cursor)?;

        // Build execution plan
        let projection = if req.fields.is_empty() {
            None
        } else {
            Some(req.fields.into_iter().collect())
        };

        let (page_nodes, next_cursor, scanned_keys, result_bytes, elapsed_ms) = match cursor_mode {
            // Legacy compatibility path for older offset cursors.
            FindNodesCursor::Offset(cursor_offset) => {
                let fetch_limit = cursor_offset
                    .saturating_add(page_size)
                    .saturating_add(1)
                    .min(u32::MAX as usize) as u32;
                let limit = Some(fetch_limit);

                let maybe_term_results = self
                    .query_executor
                    .execute_term_expression_with_options(
                        &ctx.database,
                        &ctx.namespace,
                        &entity_type,
                        &cel_expression,
                        projection.clone(),
                        limit,
                        None,
                        read_options,
                    )
                    .await
                    .map_err(|e| e.into_status())?;

                let results = if let Some(results) = maybe_term_results {
                    results
                } else {
                    let schema = self
                        .schema_registry
                        .get_schema(&ctx.database, &ctx.namespace, &entity_type)
                        .await
                        .map_err(|e| e.into_status())?
                        .ok_or_else(|| {
                            Status::not_found(format!("schema '{}' not found", entity_type))
                        })?;

                    let plan = QueryPlanner::plan(
                        &entity_type,
                        &cel_expression,
                        &schema,
                        projection.clone(),
                        limit,
                    )
                    .map_err(|e| e.into_status())?;

                    self.query_executor
                        .execute_with_options(&ctx.database, &ctx.namespace, &plan, read_options)
                        .await
                        .map_err(|e| e.into_status())?
                };
                let scanned_keys = results.scanned_keys;
                let result_bytes = results.result_bytes;
                let elapsed_ms = results.elapsed_ms;

                let total = results.nodes.len();
                let start = cursor_offset.min(total);
                let end = start.saturating_add(page_size).min(total);
                let has_more = total > end;
                let next_cursor = if has_more {
                    encode_offset_cursor(end)
                } else {
                    Vec::new()
                };
                let page_nodes: Vec<_> = results
                    .nodes
                    .into_iter()
                    .skip(start)
                    .take(page_size)
                    .collect();
                (
                    page_nodes,
                    next_cursor,
                    scanned_keys,
                    result_bytes,
                    elapsed_ms,
                )
            }
            // Default path: keyset cursor by node id bytes.
            FindNodesCursor::Keyset(cursor_node_id) => {
                // QueryExecutor already over-fetches by one to determine has_more/cursor.
                // Passing the client page size here preserves exact limit semantics.
                let limit = Some(page_size.min(u32::MAX as usize) as u32);

                let maybe_term_results = self
                    .query_executor
                    .execute_term_expression_with_options(
                        &ctx.database,
                        &ctx.namespace,
                        &entity_type,
                        &cel_expression,
                        projection.clone(),
                        limit,
                        cursor_node_id.as_deref(),
                        read_options,
                    )
                    .await
                    .map_err(|e| e.into_status())?;

                let results = if let Some(results) = maybe_term_results {
                    results
                } else {
                    let schema = self
                        .schema_registry
                        .get_schema(&ctx.database, &ctx.namespace, &entity_type)
                        .await
                        .map_err(|e| e.into_status())?
                        .ok_or_else(|| {
                            Status::not_found(format!("schema '{}' not found", entity_type))
                        })?;

                    let mut plan = QueryPlanner::plan(
                        &entity_type,
                        &cel_expression,
                        &schema,
                        projection.clone(),
                        limit,
                    )
                    .map_err(|e| e.into_status())?;
                    if let Some(cursor) = cursor_node_id {
                        plan = plan.with_cursor(cursor);
                    }

                    self.query_executor
                        .execute_with_options(&ctx.database, &ctx.namespace, &plan, read_options)
                        .await
                        .map_err(|e| e.into_status())?
                };
                let scanned_keys = results.scanned_keys;
                let result_bytes = results.result_bytes;
                let elapsed_ms = results.elapsed_ms;

                let next_cursor = if results.has_more {
                    results
                        .cursor
                        .as_ref()
                        .map(|raw| encode_keyset_cursor(raw))
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                (
                    results.nodes,
                    next_cursor,
                    scanned_keys,
                    result_bytes,
                    elapsed_ms,
                )
            }
        };
        let snapshot_meta = validate_snapshot_guardrail(
            "find_nodes",
            snapshot_mode,
            allow_degrade,
            strict_read_version,
            scanned_keys,
            result_bytes,
            elapsed_ms,
        )?;
        let last_idx = page_nodes.len().saturating_sub(1);

        let stream =
            tokio_stream::iter(page_nodes.into_iter().enumerate().map(move |(idx, node)| {
                let item_cursor = if idx == last_idx {
                    next_cursor.clone()
                } else {
                    Vec::new()
                };
                Ok(NodeResult {
                    node: Some(Node {
                        id: node.id,
                        entity_type: node.entity_type,
                        properties: core_to_proto_properties(&node.properties),
                        locality: node.locality.to_string(),
                        created_at: node.created_at,
                        updated_at: node.updated_at,
                    }),
                    next_cursor: item_cursor,
                    consistency_applied: snapshot_meta.consistency_applied as i32,
                    snapshot_read_version: snapshot_meta.snapshot_read_version.unwrap_or_default(),
                    degraded: snapshot_meta.degraded,
                    degraded_reason: snapshot_meta.degraded_reason.clone(),
                })
            }));

        Ok(Response::new(Box::pin(stream)))
    }

    type TraverseStream = Pin<Box<dyn Stream<Item = Result<TraverseResult, Status>> + Send>>;

    async fn traverse(
        &self,
        request: Request<TraverseRequest>,
    ) -> Result<Response<Self::TraverseStream>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        authorize(
            &self.db,
            principal.as_ref(),
            "query.traverse",
            &ctx.database,
            &ctx.namespace,
            "*",
        )
        .await?;
        let start = req
            .start
            .ok_or_else(|| Status::invalid_argument("missing start node"))?;
        let snapshot_mode = default_traverse_snapshot_mode(req.snapshot_mode);
        let allow_degrade = req.allow_degrade_to_best_effort;
        let strict_read_version = if snapshot_mode == SnapshotMode::Strict {
            Some(
                self.db
                    .get_read_version()
                    .await
                    .map_err(|e| e.into_status())?,
            )
        } else {
            None
        };

        let hops: Vec<TraversalHop> = req
            .steps
            .into_iter()
            .map(|step| {
                let direction = match EdgeDirection::try_from(step.direction)
                    .unwrap_or(EdgeDirection::Unspecified)
                {
                    EdgeDirection::Outgoing => TraversalDirection::Outbound,
                    EdgeDirection::Incoming => TraversalDirection::Inbound,
                    EdgeDirection::Both => TraversalDirection::Both,
                    EdgeDirection::Unspecified => TraversalDirection::Outbound,
                };

                let mut hop = TraversalHop::new(direction);
                if !step.edge_type.is_empty() {
                    hop = hop.with_labels(vec![step.edge_type]);
                }
                if !step.edge_filter.is_empty() {
                    hop = hop.with_edge_filter(step.edge_filter);
                }
                if !step.node_filter.is_empty() {
                    hop = hop.with_node_filter(step.node_filter);
                }
                hop
            })
            .collect();

        let config = TraversalConfig {
            max_depth: req.max_depth.max(1),
            max_results: req.max_results.max(1),
            timeout: std::time::Duration::from_millis(req.timeout_ms.max(1) as u64),
            buffer_size: 100,
        };
        let engine = TraversalEngine::with_config(
            self.db.clone(),
            Arc::clone(&self.schema_registry),
            Arc::clone(&self.id_allocator),
            self.site_id.clone(),
            config,
        );

        let results = engine
            .traverse_with_options(
                &ctx.database,
                &ctx.namespace,
                &start.entity_type,
                &start.node_id,
                &hops,
                if req.cursor.is_empty() {
                    None
                } else {
                    Some(req.cursor.as_slice())
                },
                TraversalReadOptions {
                    read_version: strict_read_version,
                },
            )
            .await
            .map_err(|e| e.into_status())?;
        let snapshot_meta = validate_snapshot_guardrail(
            "traverse",
            snapshot_mode,
            allow_degrade,
            strict_read_version,
            results.scanned_keys,
            results.result_bytes,
            results.elapsed_ms,
        )?;

        let mut out = Vec::new();
        let last_idx = results.paths.len().saturating_sub(1);
        for (idx, path) in results.paths.iter().enumerate() {
            let mut proto_path = Vec::with_capacity(path.hops.len() + 1);
            proto_path.push(stored_node_ref_to_proto(&path.start));
            for hop in &path.hops {
                proto_path.push(stored_node_ref_to_proto(&hop.node));
            }

            let node = Some(stored_node_to_proto(path.end_node()));
            let edge = path.hops.last().map(|h| stored_edge_to_proto(&h.edge));
            let next_cursor = if idx == last_idx {
                results.continuation_token.clone().unwrap_or_default()
            } else {
                Vec::new()
            };

            out.push(TraverseResult {
                depth: path.depth() as i32,
                path: proto_path,
                node,
                edge,
                next_cursor,
                consistency_applied: snapshot_meta.consistency_applied as i32,
                snapshot_read_version: snapshot_meta.snapshot_read_version.unwrap_or_default(),
                degraded: snapshot_meta.degraded,
                degraded_reason: snapshot_meta.degraded_reason.clone(),
            });
        }

        if out.is_empty() && results.continuation_token.is_some() {
            out.push(TraverseResult {
                depth: 0,
                path: Vec::new(),
                node: None,
                edge: None,
                next_cursor: results.continuation_token.unwrap_or_default(),
                consistency_applied: snapshot_meta.consistency_applied as i32,
                snapshot_read_version: snapshot_meta.snapshot_read_version.unwrap_or_default(),
                degraded: snapshot_meta.degraded,
                degraded_reason: snapshot_meta.degraded_reason.clone(),
            });
        }

        let stream = tokio_stream::iter(out.into_iter().map(Ok));
        Ok(Response::new(Box::pin(stream)))
    }

    async fn explain(
        &self,
        request: Request<ExplainRequest>,
    ) -> Result<Response<ExplainResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        let entity_type = req.entity_type;
        authorize(
            &self.db,
            principal.as_ref(),
            "query.explain",
            &ctx.database,
            &ctx.namespace,
            &entity_type,
        )
        .await?;
        let cel_expression = req.cel_expression;

        // Get schema for planning
        let schema = self
            .schema_registry
            .get_schema(&ctx.database, &ctx.namespace, &entity_type)
            .await
            .map_err(|e| e.into_status())?
            .ok_or_else(|| Status::not_found(format!("schema '{}' not found", entity_type)))?;

        // Build execution plan
        let plan = QueryPlanner::plan(&entity_type, &cel_expression, &schema, None, None)
            .map_err(|e| e.into_status())?;

        // Generate explanation
        let explanation = QueryExplanation::from_plan(&plan, 10000.0); // Assume 10k rows

        // Build proto response
        let indexes = if let Some(ref idx) = explanation.index_used {
            vec![IndexOperation {
                field: idx.property.clone(),
                operator: idx.operator.clone(),
                value: idx.value.clone(),
                estimated_rows: explanation.estimated_rows,
            }]
        } else {
            vec![]
        };

        Ok(Response::new(ExplainResponse {
            plan: Some(QueryPlan {
                strategy: explanation.strategy,
                indexes,
                residual_filter: explanation.residual_filter.unwrap_or_default(),
            }),
            explanation: explanation.steps,
            estimated_cost: explanation.estimated_cost,
            estimated_rows: explanation.estimated_rows,
        }))
    }

    type ExecutePQLStream = Pin<Box<dyn Stream<Item = Result<PqlResult, Status>> + Send>>;

    async fn execute_pql(
        &self,
        request: Request<ExecutePqlRequest>,
    ) -> Result<Response<Self::ExecutePQLStream>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        authorize(
            &self.db,
            principal.as_ref(),
            "query.pql",
            &ctx.database,
            &ctx.namespace,
            "*",
        )
        .await?;
        let snapshot_mode = default_execute_pql_snapshot_mode(req.snapshot_mode);
        let allow_degrade = req.allow_degrade_to_best_effort;
        let strict_read_version = if snapshot_mode == SnapshotMode::Strict {
            Some(
                self.db
                    .get_read_version()
                    .await
                    .map_err(|e| e.into_status())?,
            )
        } else {
            None
        };
        let input = apply_params(&req.pql, &req.params);

        let ast = parse_pql(&input).map_err(pql_parse_error_to_status)?;
        let schemas = self
            .build_pql_schema_provider(&ctx.database, &ctx.namespace)
            .await
            .map_err(|e| e.into_status())?;

        let resolver = PqlResolver::new();
        let resolved = resolver
            .resolve(&ast, &schemas)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        let compiler = PqlCompiler::new();
        let compiled = compiler
            .compile(&resolved)
            .map_err(|e| Status::failed_precondition(e.to_string()))?;

        if req.explain {
            let explanation = explain_query(&compiled);
            let snapshot_meta = validate_snapshot_guardrail(
                "execute_pql",
                snapshot_mode,
                allow_degrade,
                strict_read_version,
                0,
                0,
                0,
            )?;
            let stream = tokio_stream::iter(vec![Ok(PqlResult {
                block_name: "explain".to_string(),
                node: None,
                edge: None,
                next_cursor: vec![],
                explain: explanation,
                consistency_applied: snapshot_meta.consistency_applied as i32,
                snapshot_read_version: snapshot_meta.snapshot_read_version.unwrap_or_default(),
                degraded: snapshot_meta.degraded,
                degraded_reason: snapshot_meta.degraded_reason,
            })]);
            return Ok(Response::new(Box::pin(stream)));
        }

        let node_store = NodeStore::new(
            self.db.clone(),
            Arc::clone(&self.schema_registry),
            Arc::clone(&self.id_allocator),
            self.site_id.clone(),
        );
        let mut variables: HashMap<String, Vec<StoredNode>> = HashMap::new();
        let mut out = Vec::new();
        let mut total_scanned_keys = 0usize;
        let mut total_result_bytes = 0usize;
        let started_at = std::time::Instant::now();
        let read_options = ReadExecutionOptions {
            read_version: strict_read_version,
        };

        for (compiled_idx, block) in compiled.iter().enumerate() {
            let resolved_idx = resolved.execution_order[compiled_idx];
            let resolved_block = &resolved.blocks[resolved_idx];

            let mut block_nodes: Vec<StoredNode> = Vec::new();
            match block {
                pelago_query::pql::CompiledBlock::PointLookup {
                    entity_type,
                    node_id,
                    fields,
                    ..
                } => {
                    if let Some(mut node) = node_store
                        .get_node_at_read_version(
                            &ctx.database,
                            &ctx.namespace,
                            entity_type,
                            node_id,
                            strict_read_version,
                        )
                        .await
                        .map_err(|e| e.into_status())?
                    {
                        if !fields.is_empty() {
                            node.properties.retain(|k, _| fields.contains(k));
                        }
                        total_scanned_keys = total_scanned_keys.saturating_add(1);
                        total_result_bytes =
                            total_result_bytes.saturating_add(estimate_stored_node_bytes(&node));
                        block_nodes.push(node);
                    }
                }
                pelago_query::pql::CompiledBlock::FindNodes {
                    entity_type,
                    cel_expression,
                    fields,
                    limit,
                    offset,
                    ..
                } => {
                    let schema = self
                        .schema_registry
                        .get_schema(&ctx.database, &ctx.namespace, entity_type)
                        .await
                        .map_err(|e| e.into_status())?
                        .ok_or_else(|| {
                            Status::not_found(format!("schema '{}' not found", entity_type))
                        })?;
                    let projection = if fields.is_empty() {
                        None
                    } else {
                        Some(fields.iter().cloned().collect())
                    };
                    let plan = QueryPlanner::plan(
                        entity_type,
                        cel_expression.as_deref().unwrap_or(""),
                        &schema,
                        projection,
                        *limit,
                    )
                    .map_err(|e| e.into_status())?;
                    let query_results = self
                        .query_executor
                        .execute_with_options(&ctx.database, &ctx.namespace, &plan, read_options)
                        .await
                        .map_err(|e| e.into_status())?;
                    total_scanned_keys =
                        total_scanned_keys.saturating_add(query_results.scanned_keys);
                    total_result_bytes =
                        total_result_bytes.saturating_add(query_results.result_bytes);
                    let mut nodes = query_results.nodes;
                    if let Some(skip) = offset {
                        nodes = nodes.into_iter().skip(*skip as usize).collect();
                    }
                    block_nodes = nodes;
                }
                pelago_query::pql::CompiledBlock::Traverse {
                    start_entity_type,
                    start_node_id,
                    steps,
                    max_depth,
                    cascade,
                    max_results,
                    ..
                } => {
                    let hops: Vec<TraversalHop> = steps
                        .iter()
                        .map(|step| {
                            let mut hop = TraversalHop::new(match step.direction {
                                pelago_query::pql::PqlEdgeDirection::Outgoing => {
                                    TraversalDirection::Outbound
                                }
                                pelago_query::pql::PqlEdgeDirection::Incoming => {
                                    TraversalDirection::Inbound
                                }
                                pelago_query::pql::PqlEdgeDirection::Both => {
                                    TraversalDirection::Both
                                }
                            })
                            .with_labels(vec![step.edge_type.clone()]);
                            if let Some(ref f) = step.edge_filter {
                                hop = hop.with_edge_filter(f.clone());
                            }
                            if let Some(ref f) = step.node_filter {
                                hop = hop.with_node_filter(f.clone());
                            }
                            hop
                        })
                        .collect();
                    let engine = TraversalEngine::with_config(
                        self.db.clone(),
                        Arc::clone(&self.schema_registry),
                        Arc::clone(&self.id_allocator),
                        self.site_id.clone(),
                        TraversalConfig {
                            max_depth: *max_depth,
                            max_results: *max_results,
                            timeout: std::time::Duration::from_secs(5),
                            buffer_size: 100,
                        },
                    );
                    let results = engine
                        .traverse_with_options(
                            &ctx.database,
                            &ctx.namespace,
                            start_entity_type,
                            start_node_id,
                            &hops,
                            None,
                            TraversalReadOptions {
                                read_version: strict_read_version,
                            },
                        )
                        .await
                        .map_err(|e| e.into_status())?;
                    total_scanned_keys = total_scanned_keys.saturating_add(results.scanned_keys);
                    total_result_bytes = total_result_bytes.saturating_add(results.result_bytes);
                    for path in results.paths {
                        let node = path.end_node().clone();
                        if *cascade && node.id.is_empty() {
                            continue;
                        }
                        block_nodes.push(node);
                    }
                }
                pelago_query::pql::CompiledBlock::VariableRef {
                    variable,
                    filter,
                    fields,
                    limit,
                    offset,
                    ..
                } => {
                    if let Some(nodes) = variables.get(variable) {
                        block_nodes = apply_variable_nodes_pipeline(
                            nodes.clone(),
                            filter.as_deref(),
                            fields,
                            *limit,
                            *offset,
                        )?;
                    }
                }
                pelago_query::pql::CompiledBlock::VariableSet {
                    variables: variable_names,
                    set_op,
                    filter,
                    fields,
                    limit,
                    offset,
                    ..
                } => {
                    let merged = merge_variable_sets(&variables, variable_names, set_op);
                    block_nodes = apply_variable_nodes_pipeline(
                        merged,
                        filter.as_deref(),
                        fields,
                        *limit,
                        *offset,
                    )?;
                }
            }

            if let Some(ref capture) = resolved_block.block.capture_as {
                variables.insert(capture.clone(), block_nodes.clone());
            }

            let elapsed_ms = started_at.elapsed().as_millis().min(u64::MAX as u128) as u64;
            let snapshot_meta = validate_snapshot_guardrail(
                "execute_pql",
                snapshot_mode,
                allow_degrade,
                strict_read_version,
                total_scanned_keys,
                total_result_bytes,
                elapsed_ms,
            )?;
            out.extend(block_nodes.into_iter().map(|node| PqlResult {
                block_name: resolved_block.name.clone(),
                node: Some(stored_node_to_proto(&node)),
                edge: None,
                next_cursor: vec![],
                explain: String::new(),
                consistency_applied: snapshot_meta.consistency_applied as i32,
                snapshot_read_version: snapshot_meta.snapshot_read_version.unwrap_or_default(),
                degraded: snapshot_meta.degraded,
                degraded_reason: snapshot_meta.degraded_reason.clone(),
            }));
        }

        let stream = tokio_stream::iter(out.into_iter().map(Ok));
        Ok(Response::new(Box::pin(stream)))
    }
}

fn default_find_nodes_snapshot_mode(raw: i32) -> SnapshotMode {
    match SnapshotMode::try_from(raw).unwrap_or(SnapshotMode::Unspecified) {
        SnapshotMode::Unspecified => SnapshotMode::Strict,
        other => other,
    }
}

fn default_traverse_snapshot_mode(raw: i32) -> SnapshotMode {
    match SnapshotMode::try_from(raw).unwrap_or(SnapshotMode::Unspecified) {
        SnapshotMode::Unspecified => SnapshotMode::BestEffort,
        other => other,
    }
}

fn default_execute_pql_snapshot_mode(raw: i32) -> SnapshotMode {
    match SnapshotMode::try_from(raw).unwrap_or(SnapshotMode::Unspecified) {
        SnapshotMode::Unspecified => SnapshotMode::BestEffort,
        other => other,
    }
}

fn validate_snapshot_guardrail(
    operation: &str,
    mode: SnapshotMode,
    allow_degrade: bool,
    strict_read_version: Option<i64>,
    scanned_keys: usize,
    result_bytes: usize,
    elapsed_ms: u64,
) -> Result<SnapshotResponseMeta, Status> {
    let mut meta = SnapshotResponseMeta {
        consistency_applied: if mode == SnapshotMode::Strict {
            SnapshotConsistencyApplied::Strict
        } else {
            SnapshotConsistencyApplied::BestEffort
        },
        snapshot_read_version: strict_read_version,
        degraded: false,
        degraded_reason: String::new(),
    };

    if mode != SnapshotMode::Strict {
        return Ok(meta);
    }

    let exceeded = scanned_keys > C5_STRICT_MAX_SCAN_KEYS
        || result_bytes > C5_STRICT_MAX_RESULT_BYTES
        || elapsed_ms > C5_STRICT_MAX_SNAPSHOT_MS;
    if !exceeded {
        return Ok(meta);
    }

    if allow_degrade {
        meta.consistency_applied = SnapshotConsistencyApplied::BestEffort;
        meta.snapshot_read_version = None;
        meta.degraded = true;
        meta.degraded_reason = "SNAPSHOT_BUDGET_EXCEEDED".to_string();
        return Ok(meta);
    }

    Err(PelagoError::SnapshotBudgetExceeded {
        operation: operation.to_string(),
        scanned_keys,
        result_bytes,
        elapsed_ms,
        budget_ms: C5_STRICT_MAX_SNAPSHOT_MS,
    }
    .into_status())
}

fn stored_node_ref_to_proto(node: &StoredNode) -> ProtoNodeRef {
    ProtoNodeRef {
        entity_type: node.entity_type.clone(),
        node_id: node.id.clone(),
        database: String::new(),
        namespace: String::new(),
    }
}

fn core_node_ref_to_proto(node: &NodeRef) -> ProtoNodeRef {
    ProtoNodeRef {
        entity_type: node.entity_type.clone(),
        node_id: node.node_id.clone(),
        database: node.database.clone(),
        namespace: node.namespace.clone(),
    }
}

fn stored_node_to_proto(node: &StoredNode) -> Node {
    Node {
        id: node.id.clone(),
        entity_type: node.entity_type.clone(),
        properties: core_to_proto_properties(&node.properties),
        locality: node.locality.to_string(),
        created_at: node.created_at,
        updated_at: node.updated_at,
    }
}

fn stored_edge_to_proto(edge: &StoredEdge) -> Edge {
    Edge {
        edge_id: edge.edge_id.clone(),
        source: Some(core_node_ref_to_proto(&edge.source)),
        target: Some(core_node_ref_to_proto(&edge.target)),
        label: edge.label.clone(),
        properties: core_to_proto_properties(&edge.properties),
        created_at: edge.created_at,
    }
}

impl QueryServiceImpl {
    async fn build_pql_schema_provider(
        &self,
        database: &str,
        namespace: &str,
    ) -> Result<InMemorySchemaProvider, pelago_core::PelagoError> {
        let mut provider = InMemorySchemaProvider::new();
        let names = self
            .schema_registry
            .list_schemas(database, namespace)
            .await?;
        for name in names {
            if let Some(schema) = self
                .schema_registry
                .get_schema(database, namespace, &name)
                .await?
            {
                provider.add_schema(SchemaInfo {
                    entity_type: schema.name.clone(),
                    fields: schema.properties.keys().cloned().collect(),
                    edges: schema.edges.keys().cloned().collect(),
                    allow_undeclared_edges: schema.meta.allow_undeclared_edges,
                });
            }
        }
        Ok(provider)
    }
}

fn apply_params(input: &str, params: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        if ch == '$' && i + 1 < chars.len() && is_ident_start(chars[i + 1]) {
            let start = i + 1;
            let mut j = start + 1;
            while j < chars.len() && is_ident_continue(chars[j]) {
                j += 1;
            }
            let key: String = chars[start..j].iter().collect();
            if let Some(value) = params.get(&key) {
                out.push_str(value);
            } else {
                out.push('$');
                out.push_str(&key);
            }
            i = j;
            continue;
        }

        out.push(ch);
        i += 1;
    }

    out
}

fn estimate_stored_node_bytes(node: &StoredNode) -> usize {
    let mut total = node.id.len().saturating_add(node.entity_type.len());
    total = total.saturating_add(16);
    for (key, value) in &node.properties {
        total = total.saturating_add(key.len());
        total = total.saturating_add(match value {
            CoreValue::String(s) => s.len(),
            CoreValue::Int(_) => 8,
            CoreValue::Float(_) => 8,
            CoreValue::Bool(_) => 1,
            CoreValue::Timestamp(_) => 8,
            CoreValue::Bytes(b) => b.len(),
            CoreValue::Null => 1,
        });
    }
    total
}

fn apply_variable_nodes_pipeline(
    mut nodes: Vec<StoredNode>,
    filter: Option<&str>,
    fields: &[String],
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<Vec<StoredNode>, Status> {
    if let Some(filter_expr) = filter {
        let program = CelProgram::compile(filter_expr).map_err(|e| {
            Status::invalid_argument(format!("invalid variable filter '{}': {}", filter_expr, e))
        })?;
        nodes.retain(|node| evaluate_node_filter(&program, node));
    }

    if !fields.is_empty() {
        for node in &mut nodes {
            node.properties.retain(|k, _| fields.contains(k));
        }
    }

    let skip = offset.unwrap_or(0) as usize;
    if skip > 0 {
        nodes = nodes.into_iter().skip(skip).collect();
    }
    if let Some(max) = limit {
        nodes = nodes.into_iter().take(max as usize).collect();
    }

    Ok(nodes)
}

fn evaluate_node_filter(program: &CelProgram, node: &StoredNode) -> bool {
    let mut context = CelContext::default();
    for (key, value) in &node.properties {
        context.add_variable(key, core_value_to_cel(value)).ok();
    }

    match program.execute(&context) {
        Ok(CelValue::Bool(matched)) => matched,
        Ok(CelValue::Null) => false,
        Ok(_) => false,
        Err(_) => false,
    }
}

fn core_value_to_cel(value: &CoreValue) -> CelValue {
    match value {
        CoreValue::String(s) => CelValue::String(Arc::new(s.clone())),
        CoreValue::Int(n) => CelValue::Int(*n),
        CoreValue::Float(f) => CelValue::Float(*f),
        CoreValue::Bool(b) => CelValue::Bool(*b),
        CoreValue::Timestamp(t) => CelValue::Int(*t),
        CoreValue::Bytes(b) => CelValue::Bytes(Arc::new(b.clone())),
        CoreValue::Null => CelValue::Null,
    }
}

fn merge_variable_sets(
    variables: &HashMap<String, Vec<StoredNode>>,
    variable_names: &[String],
    set_op: &SetOp,
) -> Vec<StoredNode> {
    if variable_names.is_empty() {
        return Vec::new();
    }

    let first = variables
        .get(&variable_names[0])
        .cloned()
        .unwrap_or_default();

    match set_op {
        SetOp::Union => {
            let mut seen: HashSet<(String, String)> = HashSet::new();
            let mut merged = Vec::new();
            for name in variable_names {
                if let Some(nodes) = variables.get(name) {
                    for node in nodes {
                        let key = node_identity(node);
                        if seen.insert(key) {
                            merged.push(node.clone());
                        }
                    }
                }
            }
            merged
        }
        SetOp::Intersect => {
            if variable_names.len() == 1 {
                return first;
            }
            let other_sets: Vec<HashSet<(String, String)>> = variable_names[1..]
                .iter()
                .map(|name| {
                    variables
                        .get(name)
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|node| node_identity(&node))
                        .collect()
                })
                .collect();

            first
                .into_iter()
                .filter(|node| {
                    let key = node_identity(node);
                    other_sets.iter().all(|s| s.contains(&key))
                })
                .collect()
        }
        SetOp::Difference => {
            if variable_names.len() == 1 {
                return first;
            }
            let mut excluded: HashSet<(String, String)> = HashSet::new();
            for name in &variable_names[1..] {
                if let Some(nodes) = variables.get(name) {
                    for node in nodes {
                        excluded.insert(node_identity(node));
                    }
                }
            }
            first
                .into_iter()
                .filter(|node| !excluded.contains(&node_identity(node)))
                .collect()
        }
    }
}

fn node_identity(node: &StoredNode) -> (String, String) {
    (node.entity_type.clone(), node.id.clone())
}

fn pql_parse_error_to_status(err: PqlParseError) -> Status {
    match err {
        PqlParseError::UnsupportedFeature(msg) => Status::failed_precondition(msg),
        other => Status::invalid_argument(other.to_string()),
    }
}

fn is_ident_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_ident_continue(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn decode_find_nodes_cursor(cursor: &[u8]) -> Result<FindNodesCursor, Status> {
    if cursor.is_empty() {
        return Ok(FindNodesCursor::Keyset(None));
    }

    if cursor.first() == Some(&KEYSET_CURSOR_VERSION) {
        if cursor.len() < 2 {
            return Err(Status::invalid_argument(format!(
                "invalid keyset cursor: expected non-empty payload, got {} bytes",
                cursor.len().saturating_sub(1),
            )));
        }
        return Ok(FindNodesCursor::Keyset(Some(cursor[1..].to_vec())));
    }

    if cursor.len() == OFFSET_CURSOR_LEN {
        return Ok(FindNodesCursor::Offset(decode_offset_cursor(cursor)?));
    }

    if cursor.len() == NODE_ID_CURSOR_LEN {
        return Ok(FindNodesCursor::Keyset(Some(cursor.to_vec())));
    }

    Err(Status::invalid_argument(format!(
        "invalid cursor: unsupported length {}",
        cursor.len()
    )))
}

fn decode_offset_cursor(cursor: &[u8]) -> Result<usize, Status> {
    if cursor.is_empty() {
        return Ok(0);
    }
    if cursor.len() != OFFSET_CURSOR_LEN {
        return Err(Status::invalid_argument(format!(
            "invalid cursor: expected {} bytes, got {}",
            OFFSET_CURSOR_LEN,
            cursor.len()
        )));
    }

    let mut bytes = [0u8; OFFSET_CURSOR_LEN];
    bytes.copy_from_slice(cursor);
    Ok(u64::from_be_bytes(bytes) as usize)
}

fn encode_offset_cursor(offset: usize) -> Vec<u8> {
    let offset = u64::try_from(offset).unwrap_or(u64::MAX);
    offset.to_be_bytes().to_vec()
}

fn encode_keyset_cursor(cursor: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(cursor.len() + 1);
    out.push(KEYSET_CURSOR_VERSION);
    out.extend_from_slice(cursor);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pelago_core::Value;
    use pelago_query::pql::CompiledBlock;
    use tonic::Code;

    #[test]
    fn test_offset_cursor_roundtrip() {
        let cursor = encode_offset_cursor(42);
        assert_eq!(decode_offset_cursor(&cursor).unwrap(), 42);
    }

    #[test]
    fn test_offset_cursor_empty_is_zero() {
        assert_eq!(decode_offset_cursor(&[]).unwrap(), 0);
    }

    #[test]
    fn test_offset_cursor_invalid_length() {
        let err = decode_offset_cursor(&[1, 2, 3]).unwrap_err();
        assert_eq!(err.code(), Code::InvalidArgument);
    }

    #[test]
    fn test_decode_find_nodes_cursor_defaults_to_keyset() {
        match decode_find_nodes_cursor(&[]).unwrap() {
            FindNodesCursor::Keyset(None) => {}
            _ => panic!("expected keyset-none for empty cursor"),
        }
    }

    #[test]
    fn test_decode_find_nodes_cursor_legacy_offset() {
        let cursor = encode_offset_cursor(7);
        match decode_find_nodes_cursor(&cursor).unwrap() {
            FindNodesCursor::Offset(7) => {}
            _ => panic!("expected legacy offset cursor"),
        }
    }

    #[test]
    fn test_keyset_cursor_roundtrip() {
        let node_cursor = [1, 0, 0, 0, 0, 0, 0, 0, 9];
        let encoded = encode_keyset_cursor(&node_cursor);
        match decode_find_nodes_cursor(&encoded).unwrap() {
            FindNodesCursor::Keyset(Some(raw)) => assert_eq!(raw, node_cursor.to_vec()),
            _ => panic!("expected keyset cursor"),
        }
    }

    #[test]
    fn test_keyset_cursor_supports_variable_payload_length() {
        let index_cursor = b"index:key:cursor".to_vec();
        let encoded = encode_keyset_cursor(&index_cursor);
        match decode_find_nodes_cursor(&encoded).unwrap() {
            FindNodesCursor::Keyset(Some(raw)) => assert_eq!(raw, index_cursor),
            _ => panic!("expected variable-length keyset cursor"),
        }
    }

    #[test]
    fn test_keyset_cursor_prefix_takes_precedence_over_offset_shape() {
        let encoded = vec![KEYSET_CURSOR_VERSION, 1, 2, 3, 4, 5, 6, 7];
        match decode_find_nodes_cursor(&encoded).unwrap() {
            FindNodesCursor::Keyset(Some(raw)) => assert_eq!(raw, vec![1, 2, 3, 4, 5, 6, 7]),
            _ => panic!("expected keyset cursor when version prefix is present"),
        }
    }

    #[test]
    fn test_apply_params_token_aware_replacement() {
        let mut params = HashMap::new();
        params.insert("a".to_string(), "1".to_string());
        params.insert("age".to_string(), "30".to_string());
        params.insert("name".to_string(), "\"Alice\"".to_string());

        let input = r#"query { q(func: type(Person)) @filter(age >= $age && score >= $a && name == $name && other == $missing) { name } }"#;
        let output = apply_params(input, &params);

        assert!(output.contains("age >= 30"));
        assert!(output.contains("score >= 1"));
        assert!(output.contains("name == \"Alice\""));
        assert!(output.contains("other == $missing"));
    }

    #[test]
    fn test_pql_parse_error_status_mapping() {
        let unsupported = pql_parse_error_to_status(PqlParseError::UnsupportedFeature(
            "upsert not implemented".to_string(),
        ));
        assert_eq!(unsupported.code(), Code::FailedPrecondition);

        let syntax = pql_parse_error_to_status(PqlParseError::Syntax {
            line: 1,
            col: 1,
            message: "bad token".to_string(),
        });
        assert_eq!(syntax.code(), Code::InvalidArgument);
    }

    #[test]
    fn test_merge_variable_sets_operations() {
        let mut vars = HashMap::new();
        vars.insert(
            "a".to_string(),
            vec![test_node("1", 20), test_node("2", 30)],
        );
        vars.insert(
            "b".to_string(),
            vec![test_node("2", 30), test_node("3", 40)],
        );

        let union = merge_variable_sets(&vars, &["a".into(), "b".into()], &SetOp::Union);
        assert_eq!(
            union.iter().map(|n| n.id.clone()).collect::<Vec<_>>(),
            vec!["1", "2", "3"]
        );

        let intersect = merge_variable_sets(&vars, &["a".into(), "b".into()], &SetOp::Intersect);
        assert_eq!(
            intersect.iter().map(|n| n.id.clone()).collect::<Vec<_>>(),
            vec!["2"]
        );

        let difference = merge_variable_sets(&vars, &["a".into(), "b".into()], &SetOp::Difference);
        assert_eq!(
            difference.iter().map(|n| n.id.clone()).collect::<Vec<_>>(),
            vec!["1"]
        );
    }

    #[test]
    fn test_variable_pipeline_filter_projection_and_paging() {
        let nodes = vec![test_node("1", 20), test_node("2", 30), test_node("3", 40)];
        let out = apply_variable_nodes_pipeline(
            nodes,
            Some("age >= 30"),
            &["age".to_string()],
            Some(1),
            Some(1),
        )
        .unwrap();

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "3");
        assert_eq!(out[0].properties.len(), 1);
        assert_eq!(out[0].properties.get("age"), Some(&Value::Int(40)));
    }

    #[test]
    fn test_multi_block_setop_runtime_pipeline_from_compiled_query() {
        let pql = r#"query {
            friends as friends(func: type(Person)) {
                name
                age
            }
            coworkers as coworkers(func: type(Person)) {
                name
                age
            }
            mutual as mutual(func: uid(friends, coworkers, intersect)) @filter(age >= 30) @limit(first: 10) {
                name
                age
            }
            narrowed(func: uid(mutual)) @filter(name == "p2") {
                name
            }
        }"#;

        let ast = parse_pql(pql).unwrap();
        let mut schemas = InMemorySchemaProvider::new();
        schemas.add_schema(SchemaInfo {
            entity_type: "Person".to_string(),
            fields: vec!["name".to_string(), "age".to_string()],
            edges: vec![],
            allow_undeclared_edges: false,
        });
        let resolved = PqlResolver::new().resolve(&ast, &schemas).unwrap();
        let compiled = PqlCompiler::new().compile(&resolved).unwrap();

        let mut variables: HashMap<String, Vec<StoredNode>> = HashMap::new();
        variables.insert(
            "friends".to_string(),
            vec![test_node("1", 25), test_node("2", 35), test_node("3", 40)],
        );
        variables.insert(
            "coworkers".to_string(),
            vec![test_node("2", 35), test_node("3", 28), test_node("4", 50)],
        );

        let mutual_nodes = match &compiled[2] {
            CompiledBlock::VariableSet {
                variables: names,
                set_op,
                filter,
                fields,
                limit,
                offset,
                ..
            } => {
                let merged = merge_variable_sets(&variables, names, set_op);
                apply_variable_nodes_pipeline(merged, filter.as_deref(), fields, *limit, *offset)
                    .unwrap()
            }
            other => panic!("expected VariableSet at block 3, got {:?}", other),
        };
        assert_eq!(
            mutual_nodes
                .iter()
                .map(|n| n.id.as_str())
                .collect::<Vec<_>>(),
            vec!["2", "3"]
        );
        variables.insert("mutual".to_string(), mutual_nodes);

        let narrowed_nodes = match &compiled[3] {
            CompiledBlock::VariableRef {
                variable,
                filter,
                fields,
                limit,
                offset,
                ..
            } => {
                let seed = variables.get(variable).cloned().unwrap_or_default();
                apply_variable_nodes_pipeline(seed, filter.as_deref(), fields, *limit, *offset)
                    .unwrap()
            }
            other => panic!("expected VariableRef at block 4, got {:?}", other),
        };

        assert_eq!(narrowed_nodes.len(), 1);
        assert_eq!(narrowed_nodes[0].id, "2");
        assert_eq!(narrowed_nodes[0].properties.len(), 1);
        assert_eq!(
            narrowed_nodes[0].properties.get("name"),
            Some(&Value::String("p2".to_string()))
        );
    }

    fn test_node(id: &str, age: i64) -> StoredNode {
        let mut props = HashMap::new();
        props.insert("name".to_string(), Value::String(format!("p{}", id)));
        props.insert("age".to_string(), Value::Int(age));
        StoredNode::new(id.to_string(), "Person".to_string(), props, 1)
    }
}
