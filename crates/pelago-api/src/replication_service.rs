//! ReplicationService gRPC handler
//!
//! Provides the internal CDC consumption endpoint:
//! - PullCdcEvents: Stream CDC events from a namespace starting after a versionstamp

use crate::authz::{authorize, principal_from_request};
use crate::error::ToStatus;
use crate::schema_service::{core_to_proto_properties, core_to_proto_schema};
use pelago_proto::{
    replication_service_server::ReplicationService, CdcEntryProto, CdcEventResponse,
    CdcOperationProto, EdgeCreateOp, EdgeDeleteOp, NodeCreateOp, NodeDeleteOp, NodeUpdateOp,
    OwnershipTransferOp, PullCdcEventsRequest, SchemaRegisterOp,
};
use pelago_storage::{read_cdc_entries, CdcOperation, PelagoDb, Versionstamp};
use std::pin::Pin;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

/// Replication service implementation
pub struct ReplicationServiceImpl {
    db: PelagoDb,
}

impl ReplicationServiceImpl {
    pub fn new(db: PelagoDb) -> Self {
        Self { db }
    }
}

#[tonic::async_trait]
impl ReplicationService for ReplicationServiceImpl {
    type PullCdcEventsStream = Pin<Box<dyn Stream<Item = Result<CdcEventResponse, Status>> + Send>>;

    async fn pull_cdc_events(
        &self,
        request: Request<PullCdcEventsRequest>,
    ) -> Result<Response<Self::PullCdcEventsStream>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        authorize(
            &self.db,
            principal.as_ref(),
            "replication.pull",
            &ctx.database,
            &ctx.namespace,
            "*",
        )
        .await?;

        let after_vs = if req.after_versionstamp.is_empty() {
            None
        } else {
            Some(
                Versionstamp::from_bytes(&req.after_versionstamp).ok_or_else(|| {
                    Status::invalid_argument("invalid versionstamp (must be 10 bytes)")
                })?,
            )
        };

        let limit = if req.limit > 0 {
            req.limit as usize
        } else {
            1000
        };

        let entries = read_cdc_entries(
            &self.db,
            &ctx.database,
            &ctx.namespace,
            after_vs.as_ref(),
            limit,
        )
        .await
        .map_err(|e| e.into_status())?;

        let source_site_filter = if req.source_site.is_empty() {
            None
        } else {
            Some(req.source_site)
        };
        let filtered = entries
            .into_iter()
            .filter(move |(_, entry)| {
                source_site_filter
                    .as_ref()
                    .map(|s| &entry.site == s)
                    .unwrap_or(true)
            })
            .collect::<Vec<_>>();

        let stream = tokio_stream::iter(filtered.into_iter().map(|(vs, entry)| {
            let proto_ops: Vec<CdcOperationProto> = entry
                .operations
                .into_iter()
                .map(|op| cdc_op_to_proto(op))
                .collect();

            Ok(CdcEventResponse {
                versionstamp: vs.to_bytes().to_vec(),
                entry: Some(CdcEntryProto {
                    site: entry.site,
                    timestamp: entry.timestamp,
                    batch_id: entry.batch_id.unwrap_or_default(),
                    operations: proto_ops,
                }),
            })
        }));

        Ok(Response::new(Box::pin(stream)))
    }
}

/// Convert a core CdcOperation to its proto representation
fn cdc_op_to_proto(op: CdcOperation) -> CdcOperationProto {
    use pelago_proto::cdc_operation_proto::Operation;

    let operation = match op {
        CdcOperation::NodeCreate {
            entity_type,
            node_id,
            properties,
            home_site,
        } => Operation::NodeCreate(NodeCreateOp {
            entity_type,
            node_id,
            properties: core_to_proto_properties(&properties),
            home_site,
        }),
        CdcOperation::NodeUpdate {
            entity_type,
            node_id,
            changed_properties,
            old_properties,
        } => Operation::NodeUpdate(NodeUpdateOp {
            entity_type,
            node_id,
            changed_properties: core_to_proto_properties(&changed_properties),
            old_properties: core_to_proto_properties(&old_properties),
        }),
        CdcOperation::NodeDelete {
            entity_type,
            node_id,
        } => Operation::NodeDelete(NodeDeleteOp {
            entity_type,
            node_id,
        }),
        CdcOperation::EdgeCreate {
            source_type,
            source_id,
            target_type,
            target_id,
            edge_type,
            edge_id,
            properties,
        } => Operation::EdgeCreate(EdgeCreateOp {
            source_type,
            source_id,
            target_type,
            target_id,
            edge_type,
            edge_id,
            properties: core_to_proto_properties(&properties),
        }),
        CdcOperation::EdgeDelete {
            source_type,
            source_id,
            target_type,
            target_id,
            edge_type,
        } => Operation::EdgeDelete(EdgeDeleteOp {
            source_type,
            source_id,
            target_type,
            target_id,
            edge_type,
        }),
        CdcOperation::SchemaRegister {
            entity_type,
            version,
            schema,
        } => Operation::SchemaRegister(SchemaRegisterOp {
            entity_type,
            version,
            schema: Some(core_to_proto_schema(&schema)),
        }),
        CdcOperation::OwnershipTransfer {
            entity_type,
            node_id,
            previous_site_id,
            current_site_id,
        } => Operation::OwnershipTransfer(OwnershipTransferOp {
            entity_type,
            node_id,
            previous_site_id,
            current_site_id,
        }),
    };

    CdcOperationProto {
        operation: Some(operation),
    }
}
