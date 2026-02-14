//! NodeService gRPC handler
//!
//! Handles node CRUD operations:
//! - CreateNode: Create a new node with validated properties
//! - GetNode: Retrieve a node by ID
//! - UpdateNode: Update node properties (merge semantics)
//! - DeleteNode: Delete a node and its edges

use crate::schema_service::{core_to_proto_properties, proto_to_core_properties};
use pelago_proto::{
    node_service_server::NodeService, CreateNodeRequest, CreateNodeResponse, DeleteNodeRequest,
    DeleteNodeResponse, GetNodeRequest, GetNodeResponse, Node, UpdateNodeRequest,
    UpdateNodeResponse,
};
use pelago_storage::PelagoDb;
use std::sync::Arc;
use tonic::{Request, Response, Status};

/// Node service implementation
pub struct NodeServiceImpl {
    db: Arc<PelagoDb>,
}

impl NodeServiceImpl {
    pub fn new(db: Arc<PelagoDb>) -> Self {
        Self { db }
    }
}

#[tonic::async_trait]
impl NodeService for NodeServiceImpl {
    async fn create_node(
        &self,
        request: Request<CreateNodeRequest>,
    ) -> Result<Response<CreateNodeResponse>, Status> {
        let req = request.into_inner();
        let _ctx = req.context.ok_or_else(|| Status::invalid_argument("missing context"))?;
        let entity_type = req.entity_type;
        let properties = proto_to_core_properties(&req.properties);

        // TODO: Implement actual node creation with FDB
        // 1. Load schema and validate properties
        // 2. Allocate node ID
        // 3. Compute index entries
        // 4. Write node data and indexes in transaction

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as i64;

        Ok(Response::new(CreateNodeResponse {
            node: Some(Node {
                id: "placeholder_id".to_string(),
                entity_type,
                properties: req.properties,
                locality: String::new(),
                created_at: now,
                updated_at: now,
            }),
        }))
    }

    async fn get_node(
        &self,
        request: Request<GetNodeRequest>,
    ) -> Result<Response<GetNodeResponse>, Status> {
        let req = request.into_inner();
        let _ctx = req.context.ok_or_else(|| Status::invalid_argument("missing context"))?;
        let _entity_type = req.entity_type;
        let _node_id = req.node_id;
        let _consistency = req.consistency;
        let _fields = req.fields;

        // TODO: Implement actual node retrieval from FDB
        Err(Status::not_found("node not found"))
    }

    async fn update_node(
        &self,
        request: Request<UpdateNodeRequest>,
    ) -> Result<Response<UpdateNodeResponse>, Status> {
        let req = request.into_inner();
        let _ctx = req.context.ok_or_else(|| Status::invalid_argument("missing context"))?;
        let _entity_type = req.entity_type;
        let _node_id = req.node_id;
        let _properties = proto_to_core_properties(&req.properties);

        // TODO: Implement actual node update with FDB
        // 1. Read existing node
        // 2. Merge properties
        // 3. Validate merged properties
        // 4. Compute index changes
        // 5. Write updated node and index changes

        Err(Status::not_found("node not found"))
    }

    async fn delete_node(
        &self,
        request: Request<DeleteNodeRequest>,
    ) -> Result<Response<DeleteNodeResponse>, Status> {
        let req = request.into_inner();
        let _ctx = req.context.ok_or_else(|| Status::invalid_argument("missing context"))?;
        let _entity_type = req.entity_type;
        let _node_id = req.node_id;

        // TODO: Implement actual node deletion with FDB
        // 1. Delete node data
        // 2. Delete all index entries
        // 3. Delete all edges (or mark for cleanup)

        Ok(Response::new(DeleteNodeResponse { deleted: false }))
    }
}
