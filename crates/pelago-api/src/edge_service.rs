//! EdgeService gRPC handler
//!
//! Handles edge operations:
//! - CreateEdge: Create an edge between two nodes
//! - DeleteEdge: Delete an edge
//! - ListEdges: Stream edges for a given node

use crate::schema_service::{core_to_proto_properties, proto_to_core_properties};
use pelago_proto::{
    edge_service_server::EdgeService, CreateEdgeRequest, CreateEdgeResponse, DeleteEdgeRequest,
    DeleteEdgeResponse, Edge, EdgeResult, ListEdgesRequest, NodeRef,
};
use pelago_storage::PelagoDb;
use std::pin::Pin;
use std::sync::Arc;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

/// Edge service implementation
pub struct EdgeServiceImpl {
    db: Arc<PelagoDb>,
}

impl EdgeServiceImpl {
    pub fn new(db: Arc<PelagoDb>) -> Self {
        Self { db }
    }
}

#[tonic::async_trait]
impl EdgeService for EdgeServiceImpl {
    async fn create_edge(
        &self,
        request: Request<CreateEdgeRequest>,
    ) -> Result<Response<CreateEdgeResponse>, Status> {
        let req = request.into_inner();
        let _ctx = req.context.ok_or_else(|| Status::invalid_argument("missing context"))?;
        let source = req.source.ok_or_else(|| Status::invalid_argument("missing source"))?;
        let target = req.target.ok_or_else(|| Status::invalid_argument("missing target"))?;
        let label = req.label;
        let properties = proto_to_core_properties(&req.properties);

        // TODO: Implement actual edge creation with FDB
        // 1. Verify source and target nodes exist
        // 2. Load schema and validate edge type
        // 3. Compute edge keys (forward, meta, reverse if bidirectional)
        // 4. Write all edge entries in transaction

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as i64;

        Ok(Response::new(CreateEdgeResponse {
            edge: Some(Edge {
                edge_id: "placeholder_edge_id".to_string(),
                source: Some(source),
                target: Some(target),
                label,
                properties: req.properties,
                created_at: now,
            }),
        }))
    }

    async fn delete_edge(
        &self,
        request: Request<DeleteEdgeRequest>,
    ) -> Result<Response<DeleteEdgeResponse>, Status> {
        let req = request.into_inner();
        let _ctx = req.context.ok_or_else(|| Status::invalid_argument("missing context"))?;
        let _source = req.source.ok_or_else(|| Status::invalid_argument("missing source"))?;
        let _target = req.target.ok_or_else(|| Status::invalid_argument("missing target"))?;
        let _label = req.label;

        // TODO: Implement actual edge deletion with FDB
        // 1. Compute edge keys
        // 2. Delete all edge entries in transaction

        Ok(Response::new(DeleteEdgeResponse { deleted: false }))
    }

    type ListEdgesStream = Pin<Box<dyn Stream<Item = Result<EdgeResult, Status>> + Send>>;

    async fn list_edges(
        &self,
        request: Request<ListEdgesRequest>,
    ) -> Result<Response<Self::ListEdgesStream>, Status> {
        let req = request.into_inner();
        let _ctx = req.context.ok_or_else(|| Status::invalid_argument("missing context"))?;
        let _entity_type = req.entity_type;
        let _node_id = req.node_id;
        let _label = req.label;
        let _direction = req.direction;
        let _consistency = req.consistency;
        let _limit = req.limit;
        let _cursor = req.cursor;

        // TODO: Implement actual edge listing with FDB
        // 1. Build key range based on direction and label
        // 2. Iterate with cursor and limit
        // 3. Stream results

        // Return empty stream for now
        let stream = tokio_stream::empty();
        Ok(Response::new(Box::pin(stream)))
    }
}
