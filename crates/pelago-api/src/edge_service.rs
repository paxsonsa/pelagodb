//! EdgeService gRPC handler
//!
//! Handles edge operations:
//! - CreateEdge: Create an edge between two nodes
//! - DeleteEdge: Delete an edge
//! - ListEdges: Stream edges for a given node

use crate::error::ToStatus;
use crate::schema_service::{core_to_proto_properties, proto_to_core_properties};
use pelago_proto::{
    edge_service_server::EdgeService, CreateEdgeRequest, CreateEdgeResponse, DeleteEdgeRequest,
    DeleteEdgeResponse, Edge, EdgeDirection, EdgeResult, ListEdgesRequest,
    NodeRef as ProtoNodeRef, ReadConsistency,
};
use pelago_storage::{
    CachedReadPath, EdgeStore, IdAllocator, NodeRef, NodeStore, PelagoDb,
    ReadConsistency as CacheReadConsistency, SchemaRegistry,
};
use std::pin::Pin;
use std::sync::Arc;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

/// Edge service implementation
pub struct EdgeServiceImpl {
    edge_store: EdgeStore,
    cached_read_path: Option<Arc<CachedReadPath>>,
}

impl EdgeServiceImpl {
    pub fn new(
        db: PelagoDb,
        schema_registry: Arc<SchemaRegistry>,
        id_allocator: Arc<IdAllocator>,
        node_store: Arc<NodeStore>,
        site_id: String,
        cached_read_path: Option<Arc<CachedReadPath>>,
    ) -> Self {
        Self {
            edge_store: EdgeStore::new(db, schema_registry, id_allocator, node_store, site_id),
            cached_read_path,
        }
    }
}

fn proto_to_core_node_ref(proto: &ProtoNodeRef, ctx_db: &str, ctx_ns: &str) -> NodeRef {
    NodeRef::new(
        if proto.database.is_empty() { ctx_db } else { &proto.database },
        if proto.namespace.is_empty() { ctx_ns } else { &proto.namespace },
        &proto.entity_type,
        &proto.node_id,
    )
}

fn core_to_proto_node_ref(core: &NodeRef) -> ProtoNodeRef {
    ProtoNodeRef {
        database: core.database.clone(),
        namespace: core.namespace.clone(),
        entity_type: core.entity_type.clone(),
        node_id: core.node_id.clone(),
    }
}

#[tonic::async_trait]
impl EdgeService for EdgeServiceImpl {
    async fn create_edge(
        &self,
        request: Request<CreateEdgeRequest>,
    ) -> Result<Response<CreateEdgeResponse>, Status> {
        let req = request.into_inner();
        let ctx = req.context.ok_or_else(|| Status::invalid_argument("missing context"))?;
        let source_proto = req.source.ok_or_else(|| Status::invalid_argument("missing source"))?;
        let target_proto = req.target.ok_or_else(|| Status::invalid_argument("missing target"))?;
        let label = req.label;
        let properties = proto_to_core_properties(&req.properties);

        // Convert proto refs to core refs
        let source = proto_to_core_node_ref(&source_proto, &ctx.database, &ctx.namespace);
        let target = proto_to_core_node_ref(&target_proto, &ctx.database, &ctx.namespace);

        // Create edge in FDB
        let stored_edge = self.edge_store
            .create_edge(&ctx.database, &ctx.namespace, source, target, &label, properties)
            .await
            .map_err(|e| e.into_status())?;

        Ok(Response::new(CreateEdgeResponse {
            edge: Some(Edge {
                edge_id: stored_edge.edge_id,
                source: Some(core_to_proto_node_ref(&stored_edge.source)),
                target: Some(core_to_proto_node_ref(&stored_edge.target)),
                label: stored_edge.label,
                properties: core_to_proto_properties(&stored_edge.properties),
                created_at: stored_edge.created_at,
            }),
        }))
    }

    async fn delete_edge(
        &self,
        request: Request<DeleteEdgeRequest>,
    ) -> Result<Response<DeleteEdgeResponse>, Status> {
        let req = request.into_inner();
        let ctx = req.context.ok_or_else(|| Status::invalid_argument("missing context"))?;
        let source_proto = req.source.ok_or_else(|| Status::invalid_argument("missing source"))?;
        let target_proto = req.target.ok_or_else(|| Status::invalid_argument("missing target"))?;
        let label = req.label;

        // Convert proto refs to core refs
        let source = proto_to_core_node_ref(&source_proto, &ctx.database, &ctx.namespace);
        let target = proto_to_core_node_ref(&target_proto, &ctx.database, &ctx.namespace);

        // Delete edge from FDB
        let deleted = self.edge_store
            .delete_edge(&ctx.database, &ctx.namespace, source, target, &label)
            .await
            .map_err(|e| e.into_status())?;

        Ok(Response::new(DeleteEdgeResponse { deleted }))
    }

    type ListEdgesStream = Pin<Box<dyn Stream<Item = Result<EdgeResult, Status>> + Send>>;

    async fn list_edges(
        &self,
        request: Request<ListEdgesRequest>,
    ) -> Result<Response<Self::ListEdgesStream>, Status> {
        let req = request.into_inner();
        let ctx = req.context.ok_or_else(|| Status::invalid_argument("missing context"))?;
        let entity_type = req.entity_type;
        let node_id = req.node_id;
        let label_filter = if req.label.is_empty() { None } else { Some(req.label.as_str()) };
        let limit = if req.limit > 0 { req.limit as usize } else { 1000 };
        let consistency = match ReadConsistency::try_from(req.consistency)
            .unwrap_or(ReadConsistency::Strong)
        {
            ReadConsistency::Strong | ReadConsistency::Unspecified => CacheReadConsistency::Strong,
            ReadConsistency::Session => CacheReadConsistency::Session,
            ReadConsistency::Eventual => CacheReadConsistency::Eventual,
        };

        let mut edges = Vec::new();
        if let Some(cache) = &self.cached_read_path {
            edges = cache
                .list_edges_cached(
                    &ctx.database,
                    &ctx.namespace,
                    &entity_type,
                    &node_id,
                    label_filter,
                    consistency,
                )
                .await
                .map_err(|e| e.into_status())?;
        }

        if edges.is_empty() {
            edges = self
                .edge_store
                .list_edges(
                    &ctx.database,
                    &ctx.namespace,
                    &entity_type,
                    &node_id,
                    label_filter,
                    limit,
                )
                .await
                .map_err(|e| e.into_status())?;
        }

        let direction = EdgeDirection::try_from(req.direction).unwrap_or(EdgeDirection::Outgoing);
        if direction != EdgeDirection::Both {
            edges.retain(|e| match direction {
                EdgeDirection::Outgoing | EdgeDirection::Unspecified => {
                    e.source.entity_type == entity_type && e.source.node_id == node_id
                }
                EdgeDirection::Incoming => {
                    e.target.entity_type == entity_type && e.target.node_id == node_id
                }
                EdgeDirection::Both => true,
            });
        }

        // Convert to stream
        let stream = tokio_stream::iter(edges.into_iter().map(|edge| {
            Ok(EdgeResult {
                edge: Some(Edge {
                    edge_id: edge.edge_id,
                    source: Some(core_to_proto_node_ref(&edge.source)),
                    target: Some(core_to_proto_node_ref(&edge.target)),
                    label: edge.label,
                    properties: core_to_proto_properties(&edge.properties),
                    created_at: edge.created_at,
                }),
                next_cursor: vec![], // Pagination cursor not implemented yet
            })
        }));

        Ok(Response::new(Box::pin(stream)))
    }
}
