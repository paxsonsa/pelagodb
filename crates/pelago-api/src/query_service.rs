//! QueryService gRPC handler
//!
//! Handles query operations:
//! - FindNodes: Stream nodes matching a CEL expression
//! - Traverse: Graph traversal with multi-hop paths
//! - Explain: Query plan explanation

use crate::authz::{authorize, principal_from_request};
use crate::error::ToStatus;
use crate::schema_service::core_to_proto_properties;
use pelago_proto::{
    query_service_server::QueryService, Edge, EdgeDirection, ExecutePqlRequest, ExplainRequest,
    ExplainResponse, FindNodesRequest, IndexOperation, Node, NodeRef as ProtoNodeRef, NodeResult,
    PqlResult, QueryPlan, TraverseRequest, TraverseResult,
};
use pelago_query::plan::QueryExplanation;
use pelago_query::planner::QueryPlanner;
use pelago_query::pql::{
    explain_query, parse_pql, InMemorySchemaProvider, PqlCompiler, PqlResolver, SchemaInfo,
};
use pelago_query::traversal::{TraversalConfig, TraversalDirection, TraversalEngine, TraversalHop};
use pelago_query::QueryExecutor;
use pelago_storage::{
    IdAllocator, NodeRef, NodeStore, PelagoDb, SchemaRegistry, StoredEdge, StoredNode,
};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

const OFFSET_CURSOR_LEN: usize = 8;
const NODE_ID_CURSOR_LEN: usize = 9;
const KEYSET_CURSOR_VERSION: u8 = 1;

enum FindNodesCursor {
    Offset(usize),
    Keyset(Option<Vec<u8>>),
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

        let (page_nodes, next_cursor) = match cursor_mode {
            // Legacy compatibility path for older offset cursors.
            FindNodesCursor::Offset(cursor_offset) => {
                let fetch_limit = cursor_offset
                    .saturating_add(page_size)
                    .saturating_add(1)
                    .min(u32::MAX as usize) as u32;
                let limit = Some(fetch_limit);

                let maybe_term_results = self
                    .query_executor
                    .execute_term_expression(
                        &ctx.database,
                        &ctx.namespace,
                        &entity_type,
                        &cel_expression,
                        projection.clone(),
                        limit,
                        None,
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
                        .execute(&ctx.database, &ctx.namespace, &plan)
                        .await
                        .map_err(|e| e.into_status())?
                };

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
                (page_nodes, next_cursor)
            }
            // Default path: keyset cursor by node id bytes.
            FindNodesCursor::Keyset(cursor_node_id) => {
                let fetch_limit = page_size.saturating_add(1).min(u32::MAX as usize) as u32;
                let limit = Some(fetch_limit);

                let maybe_term_results = self
                    .query_executor
                    .execute_term_expression(
                        &ctx.database,
                        &ctx.namespace,
                        &entity_type,
                        &cel_expression,
                        projection.clone(),
                        limit,
                        cursor_node_id.as_deref(),
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
                        .execute(&ctx.database, &ctx.namespace, &plan)
                        .await
                        .map_err(|e| e.into_status())?
                };

                let next_cursor = if results.has_more {
                    results
                        .cursor
                        .as_ref()
                        .map(|raw| encode_keyset_cursor(raw))
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                (results.nodes, next_cursor)
            }
        };
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
            .traverse(
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
            )
            .await
            .map_err(|e| e.into_status())?;

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
            });
        }

        if out.is_empty() && results.continuation_token.is_some() {
            out.push(TraverseResult {
                depth: 0,
                path: Vec::new(),
                node: None,
                edge: None,
                next_cursor: results.continuation_token.unwrap_or_default(),
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
        let input = apply_params(&req.pql, &req.params);

        let ast = parse_pql(&input).map_err(|e| Status::invalid_argument(e.to_string()))?;
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
            let stream = tokio_stream::iter(vec![Ok(PqlResult {
                block_name: "explain".to_string(),
                node: None,
                edge: None,
                next_cursor: vec![],
                explain: explanation,
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
                        .get_node(&ctx.database, &ctx.namespace, entity_type, node_id)
                        .await
                        .map_err(|e| e.into_status())?
                    {
                        if !fields.is_empty() {
                            node.properties.retain(|k, _| fields.contains(k));
                        }
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
                    let mut nodes = self
                        .query_executor
                        .execute(&ctx.database, &ctx.namespace, &plan)
                        .await
                        .map_err(|e| e.into_status())?
                        .nodes;
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
                        .traverse(
                            &ctx.database,
                            &ctx.namespace,
                            start_entity_type,
                            start_node_id,
                            &hops,
                            None,
                        )
                        .await
                        .map_err(|e| e.into_status())?;
                    for path in results.paths {
                        let node = path.end_node().clone();
                        if *cascade && node.id.is_empty() {
                            continue;
                        }
                        block_nodes.push(node);
                    }
                }
                pelago_query::pql::CompiledBlock::VariableRef {
                    variable, fields, ..
                } => {
                    if let Some(nodes) = variables.get(variable) {
                        block_nodes = nodes
                            .iter()
                            .map(|n| {
                                let mut n2 = n.clone();
                                if !fields.is_empty() {
                                    n2.properties.retain(|k, _| fields.contains(k));
                                }
                                n2
                            })
                            .collect();
                    }
                }
            }

            if let Some(ref capture) = resolved_block.block.capture_as {
                variables.insert(capture.clone(), block_nodes.clone());
            }

            out.extend(block_nodes.into_iter().map(|node| PqlResult {
                block_name: resolved_block.name.clone(),
                node: Some(stored_node_to_proto(&node)),
                edge: None,
                next_cursor: vec![],
                explain: String::new(),
            }));
        }

        let stream = tokio_stream::iter(out.into_iter().map(Ok));
        Ok(Response::new(Box::pin(stream)))
    }
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

fn apply_params(input: &str, params: &std::collections::HashMap<String, String>) -> String {
    let mut replaced = input.to_string();
    for (key, value) in params {
        replaced = replaced.replace(&format!("${}", key), value);
    }
    replaced
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
}
