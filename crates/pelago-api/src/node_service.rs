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
use metrics::counter;
use pelago_core::PelagoError;
use pelago_proto::{
    node_service_server::NodeService, CreateNodeRequest, CreateNodeResponse, DeleteNodeRequest,
    DeleteNodeResponse, GetNodeRequest, GetNodeResponse, MutationExecutionMode, Node,
    ReadConsistency, TransferOwnershipRequest, TransferOwnershipResponse, UpdateNodeRequest,
    UpdateNodeResponse,
};
use pelago_storage::{
    CacheFallbackReason, CachedReadPath, IdAllocator, JobStore, JobType, NodeStore, PelagoDb,
    ReadConsistency as CacheReadConsistency, SchemaRegistry,
};
use std::sync::Arc;
use tonic::{Request, Response, Status};

const C4_TX_MAX_MUTATED_KEYS: usize = 5_000;
const C4_TX_MAX_WRITE_BYTES: usize = 2 * 1024 * 1024;
const C4_INLINE_CASCADE_MAX_EDGES: usize = 1_000;

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
        let session_read_version =
            if consistency == CacheReadConsistency::Session && self.cached_read_path.is_some() {
                Some(
                    self.db
                        .get_read_version()
                        .await
                        .map_err(|e| e.into_status())?,
                )
            } else {
                None
            };

        let mut stored_node = None;
        let mut fallback_reason: Option<CacheFallbackReason> = None;
        if let Some(cache) = &self.cached_read_path {
            let cache_lookup = cache
                .get_node(
                    &ctx.database,
                    &ctx.namespace,
                    &entity_type,
                    &node_id,
                    consistency,
                    session_read_version,
                )
                .await
                .map_err(|e| e.into_status())?;
            fallback_reason = cache_lookup.fallback_reason();
            stored_node = cache_lookup.value();
        }

        if stored_node.is_none() {
            if let Some(reason) = fallback_reason {
                counter!(
                    "pelago.cache.fallback_to_fdb_total",
                    "operation" => "get_node",
                    "reason" => reason.as_str()
                )
                .increment(1);
            }
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
        let mutation_mode = match MutationExecutionMode::try_from(req.mutation_mode)
            .unwrap_or(MutationExecutionMode::Unspecified)
        {
            MutationExecutionMode::Unspecified => MutationExecutionMode::AsyncAllowed,
            other => other,
        };
        authorize(
            &self.db,
            principal.as_ref(),
            "node.write",
            &ctx.database,
            &ctx.namespace,
            &entity_type,
        )
        .await?;

        let estimate = self
            .node_store
            .estimate_delete_scope(
                &ctx.database,
                &ctx.namespace,
                &entity_type,
                &node_id,
                C4_INLINE_CASCADE_MAX_EDGES,
            )
            .await
            .map_err(|e| e.into_status())?;
        let exceeds_inline_budget = estimate.exceeded_cascade_probe_limit
            || estimate.estimated_cascade_edges > C4_INLINE_CASCADE_MAX_EDGES
            || estimate.estimated_mutated_keys > C4_TX_MAX_MUTATED_KEYS
            || estimate.estimated_write_bytes > C4_TX_MAX_WRITE_BYTES;
        if exceeds_inline_budget {
            let mode_label = match mutation_mode {
                MutationExecutionMode::InlineStrict => "INLINE_STRICT",
                MutationExecutionMode::AsyncAllowed => "ASYNC_ALLOWED",
                MutationExecutionMode::Unspecified => "UNSPECIFIED",
            };
            if mutation_mode == MutationExecutionMode::AsyncAllowed {
                let job = JobStore::new(self.db.clone())
                    .create_job(
                        &ctx.database,
                        &ctx.namespace,
                        JobType::DeleteNodeCascade {
                            entity_type: entity_type.clone(),
                            node_id: node_id.clone(),
                        },
                    )
                    .await
                    .map_err(|e| {
                        Status::internal(format!("failed to create delete-node cleanup job: {}", e))
                    })?;
                return Ok(Response::new(DeleteNodeResponse {
                    deleted: false,
                    cleanup_job_id: job.job_id,
                }));
            }

            return Err(PelagoError::MutationScopeTooLarge {
                operation: format!("delete_node({mode_label})"),
                estimated_mutated_keys: estimate.estimated_mutated_keys,
                estimated_write_bytes: estimate.estimated_write_bytes,
                estimated_cascade_edges: estimate.estimated_cascade_edges,
                recommended_mode: "ASYNC_ALLOWED".to_string(),
            }
            .into_status());
        }

        // Delete node from FDB
        let deleted = self
            .node_store
            .delete_node(&ctx.database, &ctx.namespace, &entity_type, &node_id)
            .await
            .map_err(|e| e.into_status())?;

        Ok(Response::new(DeleteNodeResponse {
            deleted,
            cleanup_job_id: String::new(),
        }))
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
