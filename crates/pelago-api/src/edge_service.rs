//! EdgeService gRPC handler
//!
//! Handles edge operations:
//! - CreateEdge: Create an edge between two nodes
//! - DeleteEdge: Delete an edge
//! - ListEdges: Stream edges for a given node

use crate::authz::{authorize, principal_from_request};
use crate::error::ToStatus;
use crate::schema_service::{core_to_proto_properties, proto_to_core_properties};
use metrics::counter;
use pelago_proto::{
    edge_service_server::EdgeService, CreateEdgeRequest, CreateEdgeResponse, DeleteEdgeRequest,
    DeleteEdgeResponse, Edge, EdgeDirection, EdgeResult, ListEdgesRequest, NodeRef as ProtoNodeRef,
    ReadConsistency,
};
use pelago_storage::{
    CacheFallbackReason, CachedReadPath, EdgeStore, IdAllocator, NodeRef, NodeStore, PelagoDb,
    ReadConsistency as CacheReadConsistency, SchemaRegistry,
};
use std::collections::HashSet;
use std::pin::Pin;
use std::sync::Arc;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

const OFFSET_CURSOR_LEN: usize = 8;

/// Edge service implementation
pub struct EdgeServiceImpl {
    db: PelagoDb,
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
            db: db.clone(),
            edge_store: EdgeStore::new(db, schema_registry, id_allocator, node_store, site_id),
            cached_read_path,
        }
    }
}

fn proto_to_core_node_ref(proto: &ProtoNodeRef, ctx_db: &str, ctx_ns: &str) -> NodeRef {
    NodeRef::new(
        if proto.database.is_empty() {
            ctx_db
        } else {
            &proto.database
        },
        if proto.namespace.is_empty() {
            ctx_ns
        } else {
            &proto.namespace
        },
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
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        let source_proto = req
            .source
            .ok_or_else(|| Status::invalid_argument("missing source"))?;
        let target_proto = req
            .target
            .ok_or_else(|| Status::invalid_argument("missing target"))?;
        let label = req.label;
        let entity_for_acl = if source_proto.entity_type.is_empty() {
            "*".to_string()
        } else {
            source_proto.entity_type.clone()
        };
        authorize(
            &self.db,
            principal.as_ref(),
            "edge.write",
            &ctx.database,
            &ctx.namespace,
            &entity_for_acl,
        )
        .await?;
        let properties = proto_to_core_properties(&req.properties);

        // Convert proto refs to core refs
        let source = proto_to_core_node_ref(&source_proto, &ctx.database, &ctx.namespace);
        let target = proto_to_core_node_ref(&target_proto, &ctx.database, &ctx.namespace);

        // Create edge in FDB
        let stored_edge = self
            .edge_store
            .create_edge(
                &ctx.database,
                &ctx.namespace,
                source,
                target,
                &label,
                properties,
            )
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
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        let source_proto = req
            .source
            .ok_or_else(|| Status::invalid_argument("missing source"))?;
        let target_proto = req
            .target
            .ok_or_else(|| Status::invalid_argument("missing target"))?;
        let label = req.label;
        let entity_for_acl = if source_proto.entity_type.is_empty() {
            "*".to_string()
        } else {
            source_proto.entity_type.clone()
        };
        authorize(
            &self.db,
            principal.as_ref(),
            "edge.write",
            &ctx.database,
            &ctx.namespace,
            &entity_for_acl,
        )
        .await?;

        // Convert proto refs to core refs
        let source = proto_to_core_node_ref(&source_proto, &ctx.database, &ctx.namespace);
        let target = proto_to_core_node_ref(&target_proto, &ctx.database, &ctx.namespace);

        // Delete edge from FDB
        let deleted = self
            .edge_store
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
            "edge.read",
            &ctx.database,
            &ctx.namespace,
            &entity_type,
        )
        .await?;
        let label_filter = if req.label.is_empty() {
            None
        } else {
            Some(req.label.as_str())
        };
        let page_size = if req.limit > 0 {
            req.limit as usize
        } else {
            1000
        };
        let cursor_offset = decode_offset_cursor(&req.cursor)?;
        let fetch_limit = cursor_offset.saturating_add(page_size).saturating_add(1);
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
        let direction = EdgeDirection::try_from(req.direction).unwrap_or(EdgeDirection::Outgoing);
        let needs_outgoing = matches!(
            direction,
            EdgeDirection::Outgoing | EdgeDirection::Unspecified | EdgeDirection::Both
        );
        let needs_incoming = matches!(direction, EdgeDirection::Incoming | EdgeDirection::Both);

        let mut outgoing = Vec::new();
        let mut outgoing_fallback_reason: Option<CacheFallbackReason> = None;
        if needs_outgoing {
            if let Some(cache) = &self.cached_read_path {
                let cache_lookup = cache
                    .list_edges_cached(
                        &ctx.database,
                        &ctx.namespace,
                        &entity_type,
                        &node_id,
                        label_filter,
                        consistency,
                        session_read_version,
                    )
                    .await
                    .map_err(|e| e.into_status())?;
                outgoing_fallback_reason = cache_lookup.fallback_reason();
                outgoing = cache_lookup.value().unwrap_or_default();
            }

            if outgoing.is_empty() {
                if let Some(reason) = outgoing_fallback_reason {
                    counter!(
                        "pelago.cache.fallback_to_fdb_total",
                        "operation" => "list_edges_outgoing",
                        "reason" => reason.as_str()
                    )
                    .increment(1);
                }
                outgoing = self
                    .edge_store
                    .list_edges(
                        &ctx.database,
                        &ctx.namespace,
                        &entity_type,
                        &node_id,
                        label_filter,
                        fetch_limit,
                    )
                    .await
                    .map_err(|e| e.into_status())?;
            }
        }

        let mut incoming = Vec::new();
        let mut incoming_fallback_reason: Option<CacheFallbackReason> = None;
        if needs_incoming {
            if let Some(cache) = &self.cached_read_path {
                let cache_lookup = cache
                    .list_incoming_edges_cached(
                        &ctx.database,
                        &ctx.namespace,
                        &entity_type,
                        &node_id,
                        label_filter,
                        consistency,
                        session_read_version,
                    )
                    .await
                    .map_err(|e| e.into_status())?;
                incoming_fallback_reason = cache_lookup.fallback_reason();
                incoming = cache_lookup.value().unwrap_or_default();
            }

            if incoming.is_empty() {
                if let Some(reason) = incoming_fallback_reason {
                    counter!(
                        "pelago.cache.fallback_to_fdb_total",
                        "operation" => "list_edges_incoming",
                        "reason" => reason.as_str()
                    )
                    .increment(1);
                }
                incoming = self
                    .edge_store
                    .list_incoming_edges(
                        &ctx.database,
                        &ctx.namespace,
                        &entity_type,
                        &node_id,
                        label_filter,
                        fetch_limit,
                    )
                    .await
                    .map_err(|e| e.into_status())?;
            }
        }

        let mut edges = if needs_outgoing && needs_incoming {
            let mut merged = Vec::with_capacity(outgoing.len().saturating_add(incoming.len()));
            let mut seen = HashSet::new();

            for edge in outgoing.into_iter().chain(incoming.into_iter()) {
                let key = format!(
                    "{}|{}|{}|{}|{}|{}|{}",
                    edge.edge_id,
                    edge.source.entity_type,
                    edge.source.node_id,
                    edge.label,
                    edge.target.entity_type,
                    edge.target.node_id,
                    edge.created_at
                );
                if seen.insert(key) {
                    merged.push(edge);
                }
            }
            merged
        } else if needs_outgoing {
            outgoing
        } else {
            incoming
        };

        if direction_filter_required(direction) {
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

        let total = edges.len();
        let start = cursor_offset.min(total);
        let end = start.saturating_add(page_size).min(total);
        let has_more = total > end;
        let next_cursor = if has_more {
            encode_offset_cursor(end)
        } else {
            Vec::new()
        };
        let page_edges: Vec<_> = edges.into_iter().skip(start).take(page_size).collect();
        let last_idx = page_edges.len().saturating_sub(1);

        // Convert to stream
        let stream =
            tokio_stream::iter(page_edges.into_iter().enumerate().map(move |(idx, edge)| {
                let item_cursor = if idx == last_idx {
                    next_cursor.clone()
                } else {
                    Vec::new()
                };
                Ok(EdgeResult {
                    edge: Some(Edge {
                        edge_id: edge.edge_id,
                        source: Some(core_to_proto_node_ref(&edge.source)),
                        target: Some(core_to_proto_node_ref(&edge.target)),
                        label: edge.label,
                        properties: core_to_proto_properties(&edge.properties),
                        created_at: edge.created_at,
                    }),
                    next_cursor: item_cursor,
                })
            }));

        Ok(Response::new(Box::pin(stream)))
    }
}

fn direction_filter_required(direction: EdgeDirection) -> bool {
    !matches!(direction, EdgeDirection::Both)
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

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Code;

    #[test]
    fn test_offset_cursor_roundtrip() {
        let cursor = encode_offset_cursor(123);
        assert_eq!(decode_offset_cursor(&cursor).unwrap(), 123);
    }

    #[test]
    fn test_offset_cursor_invalid_length() {
        let err = decode_offset_cursor(&[1, 2, 3]).unwrap_err();
        assert_eq!(err.code(), Code::InvalidArgument);
    }
}
