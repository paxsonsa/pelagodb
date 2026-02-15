//! QueryService gRPC handler
//!
//! Handles query operations:
//! - FindNodes: Stream nodes matching a CEL expression
//! - Traverse: Graph traversal with multi-hop paths
//! - Explain: Query plan explanation

use crate::error::ToStatus;
use crate::schema_service::core_to_proto_properties;
use pelago_proto::{
    query_service_server::QueryService, ExplainRequest, ExplainResponse, FindNodesRequest,
    IndexOperation, Node, NodeResult, QueryPlan, TraverseRequest, TraverseResult,
};
use pelago_query::planner::QueryPlanner;
use pelago_query::plan::QueryExplanation;
use pelago_query::QueryExecutor;
use pelago_storage::{IdAllocator, PelagoDb, SchemaRegistry};
use std::pin::Pin;
use std::sync::Arc;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

/// Query service implementation
pub struct QueryServiceImpl {
    query_executor: QueryExecutor,
    schema_registry: Arc<SchemaRegistry>,
}

impl QueryServiceImpl {
    pub fn new(
        db: PelagoDb,
        schema_registry: Arc<SchemaRegistry>,
        id_allocator: Arc<IdAllocator>,
        site_id: String,
    ) -> Self {
        Self {
            query_executor: QueryExecutor::new(db, Arc::clone(&schema_registry), id_allocator, site_id),
            schema_registry,
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
        let _ctx = req.context.ok_or_else(|| Status::invalid_argument("missing context"))?;
        let _start = req.start.ok_or_else(|| Status::invalid_argument("missing start node"))?;

        // TODO: Implement actual traversal when TraversalEngine is integrated
        // For now, return empty stream
        let stream = tokio_stream::empty();
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
