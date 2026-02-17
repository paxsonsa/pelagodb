//! NodeService gRPC handler
//!
//! Handles node CRUD operations:
//! - CreateNode: Create a new node with validated properties
//! - GetNode: Retrieve a node by ID
//! - UpdateNode: Update node properties (merge semantics)
//! - DeleteNode: Delete a node and its edges

use crate::authz::{authorize, principal_from_request};
use crate::error::ToStatus;
use crate::schema_service::{core_to_proto_properties, proto_to_core_properties};
use pelago_proto::{
    node_service_server::NodeService, CreateNodeRequest, CreateNodeResponse, DeleteNodeRequest,
    DeleteNodeResponse, GetNodeRequest, GetNodeResponse, Node, ReadConsistency,
    TransferOwnershipRequest, TransferOwnershipResponse, UpdateNodeRequest, UpdateNodeResponse,
};
use pelago_storage::{
    CachedReadPath, IdAllocator, NodeStore, PelagoDb, ReadConsistency as CacheReadConsistency,
    SchemaRegistry,
};
use std::sync::Arc;
use tonic::{Request, Response, Status};

/// Node service implementation
pub struct NodeServiceImpl {
    db: PelagoDb,
    node_store: NodeStore,
    cached_read_path: Option<Arc<CachedReadPath>>,
}

impl NodeServiceImpl {
    pub fn new(
        db: PelagoDb,
        schema_registry: Arc<SchemaRegistry>,
        id_allocator: Arc<IdAllocator>,
        site_id: String,
        cached_read_path: Option<Arc<CachedReadPath>>,
    ) -> Self {
        Self {
            db: db.clone(),
            node_store: NodeStore::new(db, schema_registry, id_allocator, site_id),
            cached_read_path,
        }
    }
}

#[tonic::async_trait]
impl NodeService for NodeServiceImpl {
    async fn create_node(
        &self,
        request: Request<CreateNodeRequest>,
    ) -> Result<Response<CreateNodeResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        let entity_type = req.entity_type;
        authorize(
            &self.db,
            principal.as_ref(),
            "node.write",
            &ctx.database,
            &ctx.namespace,
            &entity_type,
        )
        .await?;
        let properties = proto_to_core_properties(&req.properties);

        // Create node in FDB
        let stored_node = self
            .node_store
            .create_node(&ctx.database, &ctx.namespace, &entity_type, properties)
            .await
            .map_err(|e| e.into_status())?;

        Ok(Response::new(CreateNodeResponse {
            node: Some(Node {
                id: stored_node.id,
                entity_type: stored_node.entity_type,
                properties: core_to_proto_properties(&stored_node.properties),
                locality: stored_node.locality.to_string(),
                created_at: stored_node.created_at,
                updated_at: stored_node.updated_at,
            }),
        }))
    }

    async fn get_node(
        &self,
        request: Request<GetNodeRequest>,
    ) -> Result<Response<GetNodeResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        let entity_type = req.entity_type;
        let node_id = req.node_id;
        authorize(
            &self.db,
            principal.as_ref(),
            "node.read",
            &ctx.database,
            &ctx.namespace,
            &entity_type,
        )
        .await?;

        let consistency = match ReadConsistency::try_from(req.consistency)
            .unwrap_or(ReadConsistency::Strong)
        {
            ReadConsistency::Strong | ReadConsistency::Unspecified => CacheReadConsistency::Strong,
            ReadConsistency::Session => CacheReadConsistency::Session,
            ReadConsistency::Eventual => CacheReadConsistency::Eventual,
        };

        let mut stored_node = None;
        if let Some(cache) = &self.cached_read_path {
            stored_node = cache
                .get_node(
                    &ctx.database,
                    &ctx.namespace,
                    &entity_type,
                    &node_id,
                    consistency,
                )
                .await
                .map_err(|e| e.into_status())?;
        }

        if stored_node.is_none() {
            stored_node = self
                .node_store
                .get_node(&ctx.database, &ctx.namespace, &entity_type, &node_id)
                .await
                .map_err(|e| e.into_status())?;
        }

        match stored_node {
            Some(node) => {
                // Apply field projection if specified
                let properties = if req.fields.is_empty() {
                    core_to_proto_properties(&node.properties)
                } else {
                    let filtered: std::collections::HashMap<_, _> = node
                        .properties
                        .iter()
                        .filter(|(k, _)| req.fields.contains(k))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    core_to_proto_properties(&filtered)
                };

                Ok(Response::new(GetNodeResponse {
                    node: Some(Node {
                        id: node.id,
                        entity_type: node.entity_type,
                        properties,
                        locality: node.locality.to_string(),
                        created_at: node.created_at,
                        updated_at: node.updated_at,
                    }),
                }))
            }
            None => Err(Status::not_found(format!(
                "node '{}:{}' not found",
                entity_type, node_id
            ))),
        }
    }

    async fn update_node(
        &self,
        request: Request<UpdateNodeRequest>,
    ) -> Result<Response<UpdateNodeResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        let entity_type = req.entity_type;
        let node_id = req.node_id;
        authorize(
            &self.db,
            principal.as_ref(),
            "node.write",
            &ctx.database,
            &ctx.namespace,
            &entity_type,
        )
        .await?;
        let properties = proto_to_core_properties(&req.properties);

        // Update node in FDB
        let stored_node = self
            .node_store
            .update_node(
                &ctx.database,
                &ctx.namespace,
                &entity_type,
                &node_id,
                properties,
            )
            .await
            .map_err(|e| e.into_status())?;

        Ok(Response::new(UpdateNodeResponse {
            node: Some(Node {
                id: stored_node.id,
                entity_type: stored_node.entity_type,
                properties: core_to_proto_properties(&stored_node.properties),
                locality: stored_node.locality.to_string(),
                created_at: stored_node.created_at,
                updated_at: stored_node.updated_at,
            }),
        }))
    }

    async fn delete_node(
        &self,
        request: Request<DeleteNodeRequest>,
    ) -> Result<Response<DeleteNodeResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        let entity_type = req.entity_type;
        let node_id = req.node_id;
        authorize(
            &self.db,
            principal.as_ref(),
            "node.write",
            &ctx.database,
            &ctx.namespace,
            &entity_type,
        )
        .await?;

        // Delete node from FDB
        let deleted = self
            .node_store
            .delete_node(&ctx.database, &ctx.namespace, &entity_type, &node_id)
            .await
            .map_err(|e| e.into_status())?;

        Ok(Response::new(DeleteNodeResponse { deleted }))
    }

    async fn transfer_ownership(
        &self,
        request: Request<TransferOwnershipRequest>,
    ) -> Result<Response<TransferOwnershipResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        authorize(
            &self.db,
            principal.as_ref(),
            "node.transfer",
            &ctx.database,
            &ctx.namespace,
            &req.entity_type,
        )
        .await?;
        let target_site_id = req
            .target_site_id
            .parse::<u8>()
            .map_err(|_| Status::invalid_argument("target_site_id must be a u8 string"))?;

        let (transferred, previous_site_id, current_site_id) = self
            .node_store
            .transfer_ownership(
                &ctx.database,
                &ctx.namespace,
                &req.entity_type,
                &req.node_id,
                target_site_id,
            )
            .await
            .map_err(|e| e.into_status())?;

        Ok(Response::new(TransferOwnershipResponse {
            transferred,
            previous_site_id: previous_site_id.to_string(),
            current_site_id: current_site_id.to_string(),
        }))
    }
}
