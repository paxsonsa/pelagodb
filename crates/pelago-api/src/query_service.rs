//! QueryService gRPC handler
//!
//! Handles query operations:
//! - FindNodes: Stream nodes matching a CEL expression
//! - Traverse: Graph traversal with multi-hop paths
//! - Explain: Query plan explanation

use pelago_proto::{
    query_service_server::QueryService, ExplainRequest, ExplainResponse, FindNodesRequest,
    IndexOperation, NodeResult, QueryPlan, TraverseRequest, TraverseResult,
};
use pelago_query::planner::QueryPlanner;
use pelago_query::plan::{ExecutionPlan, QueryPlan as CoreQueryPlan};
use pelago_storage::PelagoDb;
use std::pin::Pin;
use std::sync::Arc;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

/// Query service implementation
pub struct QueryServiceImpl {
    db: Arc<PelagoDb>,
}

impl QueryServiceImpl {
    pub fn new(db: Arc<PelagoDb>) -> Self {
        Self { db }
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
        let _ctx = req.context.ok_or_else(|| Status::invalid_argument("missing context"))?;
        let _entity_type = req.entity_type;
        let _cel_expression = req.cel_expression;
        let _consistency = req.consistency;
        let _fields = req.fields;
        let _limit = req.limit;
        let _cursor = req.cursor;

        // TODO: Implement actual query execution with FDB
        // 1. Load schema for entity type
        // 2. Plan query using QueryPlanner
        // 3. Execute plan against FDB
        // 4. Stream results with filtering

        // Return empty stream for now
        let stream = tokio_stream::empty();
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
        let _steps = req.steps;
        let _max_depth = req.max_depth;
        let _timeout_ms = req.timeout_ms;
        let _max_results = req.max_results;
        let _consistency = req.consistency;
        let _cascade = req.cascade;

        // TODO: Implement actual graph traversal with FDB
        // 1. Start from given node
        // 2. Execute each step with filters
        // 3. Respect depth, timeout, and result limits
        // 4. Stream results

        // Return empty stream for now
        let stream = tokio_stream::empty();
        Ok(Response::new(Box::pin(stream)))
    }

    async fn explain(
        &self,
        request: Request<ExplainRequest>,
    ) -> Result<Response<ExplainResponse>, Status> {
        let req = request.into_inner();
        let _ctx = req.context.ok_or_else(|| Status::invalid_argument("missing context"))?;
        let entity_type = req.entity_type;
        let cel_expression = req.cel_expression;

        // For explain, we don't need FDB - just use the planner
        // We need a schema though - for now return a basic explanation

        // Build explanation
        let mut explanation = vec![
            format!("Query: {} WHERE {}", entity_type, cel_expression),
        ];

        // Without schema, we can only do basic parsing
        let predicates = cel_expression.split("&&").collect::<Vec<_>>();

        if predicates.is_empty() || cel_expression.is_empty() {
            explanation.push("Strategy: FULL_SCAN (no predicates)".to_string());
        } else {
            explanation.push(format!("Extracted {} predicate(s)", predicates.len()));
            explanation.push("Strategy: Requires schema to determine index usage".to_string());
        }

        Ok(Response::new(ExplainResponse {
            plan: Some(QueryPlan {
                strategy: if cel_expression.is_empty() {
                    "full_scan".to_string()
                } else {
                    "unknown".to_string()
                },
                indexes: vec![],
                residual_filter: if cel_expression.is_empty() {
                    String::new()
                } else {
                    cel_expression
                },
            }),
            explanation,
            estimated_cost: 1.0,
            estimated_rows: 1000.0,
        }))
    }
}
