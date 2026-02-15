//! QueryService gRPC handler
//!
//! Handles query operations:
//! - FindNodes: Stream nodes matching a CEL expression
//! - Traverse: Graph traversal with multi-hop paths
//! - Explain: Query plan explanation

use crate::error::ToStatus;
use crate::schema_service::core_to_proto_properties;
use pelago_proto::{
    query_service_server::QueryService, Edge, EdgeDirection, ExplainRequest, ExplainResponse,
    FindNodesRequest, IndexOperation, Node, NodeRef as ProtoNodeRef, NodeResult, QueryPlan,
    TraverseRequest, TraverseResult,
};
use pelago_query::planner::QueryPlanner;
use pelago_query::plan::QueryExplanation;
use pelago_query::traversal::{TraversalConfig, TraversalDirection, TraversalEngine, TraversalHop};
use pelago_query::QueryExecutor;
use pelago_storage::{IdAllocator, NodeRef, PelagoDb, SchemaRegistry, StoredEdge, StoredNode};
use std::pin::Pin;
use std::sync::Arc;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

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
        let req = request.into_inner();
        let ctx = req.context.ok_or_else(|| Status::invalid_argument("missing context"))?;
        let entity_type = req.entity_type;
        let cel_expression = req.cel_expression;
        let limit = if req.limit > 0 { Some(req.limit) } else { None };

        // Get schema for planning
        let schema = self.schema_registry
            .get_schema(&ctx.database, &ctx.namespace, &entity_type)
            .await
            .map_err(|e| e.into_status())?
            .ok_or_else(|| Status::not_found(format!("schema '{}' not found", entity_type)))?;

        // Build execution plan
        let projection = if req.fields.is_empty() {
            None
        } else {
            Some(req.fields.into_iter().collect())
        };

        let plan = QueryPlanner::plan(&entity_type, &cel_expression, &schema, projection, limit)
            .map_err(|e| e.into_status())?;

        // Execute query
        let results = self.query_executor
            .execute(&ctx.database, &ctx.namespace, &plan)
            .await
            .map_err(|e| e.into_status())?;

        // Convert to stream
        let stream = tokio_stream::iter(results.nodes.into_iter().map(|node| {
            Ok(NodeResult {
                node: Some(Node {
                    id: node.id,
                    entity_type: node.entity_type,
                    properties: core_to_proto_properties(&node.properties),
                    locality: node.locality.to_string(),
                    created_at: node.created_at,
                    updated_at: node.updated_at,
                }),
                next_cursor: vec![], // Pagination cursor not implemented yet
            })
        }));

        Ok(Response::new(Box::pin(stream)))
    }

    type TraverseStream = Pin<Box<dyn Stream<Item = Result<TraverseResult, Status>> + Send>>;

    async fn traverse(
        &self,
        request: Request<TraverseRequest>,
    ) -> Result<Response<Self::TraverseStream>, Status> {
        let req = request.into_inner();
        let ctx = req.context.ok_or_else(|| Status::invalid_argument("missing context"))?;
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
        let req = request.into_inner();
        let ctx = req.context.ok_or_else(|| Status::invalid_argument("missing context"))?;
        let entity_type = req.entity_type;
        let cel_expression = req.cel_expression;

        // Get schema for planning
        let schema = self.schema_registry
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
