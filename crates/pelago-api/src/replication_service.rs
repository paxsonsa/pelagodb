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
use pelago_storage::{read_cdc_entries, CdcEntry, CdcOperation, PelagoDb, Versionstamp};
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

async fn collect_cdc_entries<F, Fut>(
    mut fetch_page: F,
    after_versionstamp: Option<Versionstamp>,
    limit: usize,
    source_site_filter: Option<&str>,
) -> Result<Vec<(Versionstamp, CdcEntry)>, pelago_core::PelagoError>
where
    F: FnMut(Option<Versionstamp>, usize) -> Fut,
    Fut: std::future::Future<
        Output = Result<Vec<(Versionstamp, CdcEntry)>, pelago_core::PelagoError>,
    >,
{
    if source_site_filter.is_none() {
        return fetch_page(after_versionstamp, limit).await;
    }
    let source_site = source_site_filter.unwrap();
    let page_size = limit.clamp(1, 1000);

    let mut filtered = Vec::with_capacity(limit);
    let mut cursor = after_versionstamp;
    loop {
        let page = fetch_page(cursor.clone(), page_size).await?;
        if page.is_empty() {
            break;
        }

        let fetched = page.len();
        let mut last_vs: Option<Versionstamp> = None;
        for (vs, entry) in page {
            last_vs = Some(vs.clone());
            if entry.site == source_site {
                filtered.push((vs, entry));
                if filtered.len() == limit {
                    return Ok(filtered);
                }
            }
        }

        if fetched < page_size {
            break;
        }
        cursor = last_vs;
    }

    Ok(filtered)
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

        let source_site_filter = if req.source_site.is_empty() {
            None
        } else {
            Some(req.source_site)
        };
        let db = self.db.clone();
        let database = ctx.database.clone();
        let namespace = ctx.namespace.clone();
        let entries = collect_cdc_entries(
            move |after, page_limit| {
                let db = db.clone();
                let database = database.clone();
                let namespace = namespace.clone();
                async move {
                    read_cdc_entries(&db, &database, &namespace, after.as_ref(), page_limit).await
                }
            },
            after_vs,
            limit,
            source_site_filter.as_deref(),
        )
        .await
        .map_err(|e| e.into_status())?;

        let stream = tokio_stream::iter(entries.into_iter().map(|(vs, entry)| {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::ready;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn versionstamp(value: u16) -> Versionstamp {
        let mut bytes = [0u8; 10];
        bytes[8] = (value >> 8) as u8;
        bytes[9] = (value & 0xFF) as u8;
        Versionstamp::from_bytes(&bytes).expect("invalid versionstamp")
    }

    fn entry(site: &str) -> CdcEntry {
        CdcEntry {
            site: site.to_string(),
            timestamp: 0,
            batch_id: None,
            operations: Vec::new(),
        }
    }

    #[tokio::test]
    async fn test_collect_cdc_entries_scans_multiple_pages_for_source_site() {
        let dataset = vec![
            (versionstamp(1), entry("1")),
            (versionstamp(2), entry("1")),
            (versionstamp(3), entry("2")),
            (versionstamp(4), entry("1")),
            (versionstamp(5), entry("2")),
        ];
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = Arc::clone(&calls);

        let results = collect_cdc_entries(
            move |after, limit| {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                let start = match after {
                    Some(vs) => dataset
                        .iter()
                        .position(|(candidate, _)| candidate > &vs)
                        .unwrap_or(dataset.len()),
                    None => 0,
                };
                let end = (start + limit).min(dataset.len());
                ready(Ok(dataset[start..end].to_vec()))
            },
            None,
            2,
            Some("2"),
        )
        .await
        .expect("collect failed");

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, versionstamp(3));
        assert_eq!(results[1].0, versionstamp(5));
        assert!(calls.load(Ordering::SeqCst) > 1);
    }

    #[tokio::test]
    async fn test_collect_cdc_entries_without_filter_reads_once() {
        let dataset = vec![
            (versionstamp(1), entry("1")),
            (versionstamp(2), entry("2")),
            (versionstamp(3), entry("3")),
        ];
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = Arc::clone(&calls);

        let results = collect_cdc_entries(
            move |after, limit| {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                let start = match after {
                    Some(vs) => dataset
                        .iter()
                        .position(|(candidate, _)| candidate > &vs)
                        .unwrap_or(dataset.len()),
                    None => 0,
                };
                let end = (start + limit).min(dataset.len());
                ready(Ok(dataset[start..end].to_vec()))
            },
            None,
            2,
            None,
        )
        .await
        .expect("collect failed");

        assert_eq!(results.len(), 2);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
