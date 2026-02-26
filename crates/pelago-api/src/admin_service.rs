//! AdminService gRPC handler
//!
//! Handles administrative operations:
//! - DropIndex: Remove an index from an entity type
//! - StripProperty: Remove a property from all nodes
//! - GetJobStatus: Check status of async jobs
//! - DropEntityType: Remove an entity type
//! - DropNamespace: Remove an entire namespace

use crate::authz::{authorize, principal_from_request};
use crate::error::ToStatus;
use pelago_core::encoding::decode_cbor;
use pelago_core::PelagoError;
use pelago_proto::{
    admin_service_server::AdminService, DropEntityTypeRequest, DropEntityTypeResponse,
    DropIndexRequest, DropIndexResponse, DropNamespaceRequest, DropNamespaceResponse,
    GetJobStatusRequest, GetJobStatusResponse, GetReplicationStatusRequest,
    GetReplicationStatusResponse, Job, JobStatus, ListJobsRequest, ListJobsResponse,
    ListSitesRequest, ListSitesResponse, MutationExecutionMode, QueryAuditLogRequest,
    QueryAuditLogResponse, ReplicationPeerStatus, SiteInfo, StripPropertyRequest,
    StripPropertyResponse,
};
use pelago_storage::{
    get_replication_positions_scoped, list_sites, query_audit_records,
    JobStatus as StorageJobStatus, JobStore, JobType, PelagoDb, Subspace,
};
use serde::Deserialize;
use std::sync::Arc;
use tonic::{Request, Response, Status};

const C4_TX_MAX_MUTATED_KEYS: usize = 5_000;
const C4_TX_MAX_WRITE_BYTES: usize = 2 * 1024 * 1024;

/// Admin service implementation
pub struct AdminServiceImpl {
    db: Arc<PelagoDb>,
}

impl AdminServiceImpl {
    pub fn new(db: Arc<PelagoDb>) -> Self {
        Self { db }
    }
}

#[tonic::async_trait]
impl AdminService for AdminServiceImpl {
    async fn drop_index(
        &self,
        request: Request<DropIndexRequest>,
    ) -> Result<Response<DropIndexResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        let entity_type = req.entity_type;
        let property_name = req.property_name;
        authorize(
            self.db.as_ref(),
            principal.as_ref(),
            "admin.write",
            &ctx.database,
            &ctx.namespace,
            &entity_type,
        )
        .await?;

        let prefix = Subspace::namespace(&ctx.database, &ctx.namespace)
            .index()
            .pack()
            .add_string(&entity_type)
            .add_string(&property_name)
            .build()
            .to_vec();

        let entries_deleted = clear_prefix_entries(&self.db, &prefix)
            .await
            .map_err(|e| Status::internal(format!("drop_index failed: {}", e)))?;

        Ok(Response::new(DropIndexResponse { entries_deleted }))
    }

    async fn strip_property(
        &self,
        request: Request<StripPropertyRequest>,
    ) -> Result<Response<StripPropertyResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        let entity_type = req.entity_type;
        let property_name = req.property_name;
        authorize(
            self.db.as_ref(),
            principal.as_ref(),
            "admin.write",
            &ctx.database,
            &ctx.namespace,
            &entity_type,
        )
        .await?;

        let job = JobStore::new(self.db.as_ref().clone())
            .create_job(
                &ctx.database,
                &ctx.namespace,
                JobType::StripProperty {
                    entity_type,
                    property_name,
                },
            )
            .await
            .map_err(|e| Status::internal(format!("failed to create strip-property job: {}", e)))?;

        let job_id = job.job_id;
        Ok(Response::new(StripPropertyResponse { job_id }))
    }

    async fn get_job_status(
        &self,
        request: Request<GetJobStatusRequest>,
    ) -> Result<Response<GetJobStatusResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        let job_id = req.job_id;
        authorize(
            self.db.as_ref(),
            principal.as_ref(),
            "admin.read",
            &ctx.database,
            &ctx.namespace,
            "*",
        )
        .await?;

        let store = JobStore::new(self.db.as_ref().clone());
        let job = store
            .get_job(&ctx.database, &ctx.namespace, &job_id)
            .await
            .map_err(|e| Status::internal(format!("failed to fetch job: {}", e)))?
            .ok_or_else(|| Status::not_found(format!("job '{}' not found", job_id)))?;

        Ok(Response::new(GetJobStatusResponse {
            job: Some(storage_job_to_proto_job(job)),
        }))
    }

    async fn list_jobs(
        &self,
        request: Request<ListJobsRequest>,
    ) -> Result<Response<ListJobsResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        authorize(
            self.db.as_ref(),
            principal.as_ref(),
            "admin.read",
            &ctx.database,
            &ctx.namespace,
            "*",
        )
        .await?;

        let store = JobStore::new(self.db.as_ref().clone());
        let jobs = store
            .list_jobs(&ctx.database, &ctx.namespace)
            .await
            .map_err(|e| Status::internal(format!("failed to list jobs: {}", e)))?;

        Ok(Response::new(ListJobsResponse {
            jobs: jobs.into_iter().map(storage_job_to_proto_job).collect(),
        }))
    }

    async fn drop_entity_type(
        &self,
        request: Request<DropEntityTypeRequest>,
    ) -> Result<Response<DropEntityTypeResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        let entity_type = req.entity_type;
        let mutation_mode = match MutationExecutionMode::try_from(req.mutation_mode)
            .unwrap_or(MutationExecutionMode::Unspecified)
        {
            MutationExecutionMode::Unspecified => MutationExecutionMode::AsyncAllowed,
            other => other,
        };
        authorize(
            self.db.as_ref(),
            principal.as_ref(),
            "admin.write",
            &ctx.database,
            &ctx.namespace,
            &entity_type,
        )
        .await?;

        let ns = Subspace::namespace(&ctx.database, &ctx.namespace);
        let schema_prefix = ns.schema().pack().add_string(&entity_type).build().to_vec();
        let data_prefix = ns.data().pack().add_string(&entity_type).build().to_vec();
        let idx_prefix = ns.index().pack().add_string(&entity_type).build().to_vec();
        let edge_forward_prefix = ns
            .edge()
            .pack()
            .add_marker(pelago_storage::subspace::edge_markers::FORWARD)
            .add_string(&entity_type)
            .build()
            .to_vec();
        let edge_meta_prefix = ns
            .edge()
            .pack()
            .add_marker(pelago_storage::subspace::edge_markers::FORWARD_META)
            .add_string(&entity_type)
            .build()
            .to_vec();
        let edge_reverse_target_prefix = ns
            .edge()
            .pack()
            .add_marker(pelago_storage::subspace::edge_markers::REVERSE)
            .add_string(&ctx.database)
            .add_string(&ctx.namespace)
            .add_string(&entity_type)
            .build()
            .to_vec();

        let probe_limit = C4_TX_MAX_MUTATED_KEYS.saturating_add(1);
        let schema_count = count_prefix_entries_up_to(&self.db, &schema_prefix, probe_limit)
            .await
            .map_err(|e| Status::internal(format!("failed to estimate schema scope: {}", e)))?;
        let data_count = count_prefix_entries_up_to(&self.db, &data_prefix, probe_limit)
            .await
            .map_err(|e| Status::internal(format!("failed to estimate data scope: {}", e)))?;
        let idx_count = count_prefix_entries_up_to(&self.db, &idx_prefix, probe_limit)
            .await
            .map_err(|e| Status::internal(format!("failed to estimate index scope: {}", e)))?;
        let edge_forward_count =
            count_prefix_entries_up_to(&self.db, &edge_forward_prefix, probe_limit)
                .await
                .map_err(|e| {
                    Status::internal(format!("failed to estimate forward edge scope: {}", e))
                })?;
        let edge_meta_count = count_prefix_entries_up_to(&self.db, &edge_meta_prefix, probe_limit)
            .await
            .map_err(|e| {
                Status::internal(format!("failed to estimate edge metadata scope: {}", e))
            })?;
        let edge_reverse_count =
            count_prefix_entries_up_to(&self.db, &edge_reverse_target_prefix, probe_limit)
                .await
                .map_err(|e| {
                    Status::internal(format!("failed to estimate reverse edge scope: {}", e))
                })?;
        let edge_target_forward_meta_count = count_forward_meta_rows_for_target_type_up_to(
            self.db.as_ref(),
            &ctx.database,
            &ctx.namespace,
            &entity_type,
            1000,
            probe_limit,
        )
        .await
        .map_err(|e| {
            Status::internal(format!(
                "failed to estimate forward/meta target cleanup scope: {}",
                e
            ))
        })?;

        let estimated_mutated_keys = schema_count
            .saturating_add(data_count)
            .saturating_add(idx_count)
            .saturating_add(edge_forward_count)
            .saturating_add(edge_meta_count)
            .saturating_add(edge_reverse_count)
            .saturating_add(edge_target_forward_meta_count.saturating_mul(2));
        let estimated_write_bytes = estimated_mutated_keys.saturating_mul(256);
        if estimated_mutated_keys > C4_TX_MAX_MUTATED_KEYS
            || estimated_write_bytes > C4_TX_MAX_WRITE_BYTES
        {
            let mode_label = match mutation_mode {
                MutationExecutionMode::InlineStrict => "INLINE_STRICT",
                MutationExecutionMode::AsyncAllowed => "ASYNC_ALLOWED",
                MutationExecutionMode::Unspecified => "UNSPECIFIED",
            };
            if mutation_mode == MutationExecutionMode::AsyncAllowed {
                let job = JobStore::new(self.db.as_ref().clone())
                    .create_job(
                        &ctx.database,
                        &ctx.namespace,
                        JobType::DropEntityTypeCleanup {
                            entity_type: entity_type.clone(),
                        },
                    )
                    .await
                    .map_err(|e| {
                        Status::internal(format!(
                            "failed to create drop-entity-type cleanup job: {}",
                            e
                        ))
                    })?;
                return Ok(Response::new(DropEntityTypeResponse {
                    cleanup_job_id: job.job_id,
                }));
            }

            return Err(PelagoError::MutationScopeTooLarge {
                operation: format!("drop_entity_type({mode_label})"),
                estimated_mutated_keys,
                estimated_write_bytes,
                estimated_cascade_edges: edge_forward_count.saturating_add(edge_reverse_count),
                recommended_mode: "ASYNC_ALLOWED".to_string(),
            }
            .into_status());
        }

        clear_prefix_entries(&self.db, &schema_prefix)
            .await
            .map_err(|e| Status::internal(format!("failed to clear schema: {}", e)))?;
        clear_prefix_entries(&self.db, &data_prefix)
            .await
            .map_err(|e| Status::internal(format!("failed to clear data: {}", e)))?;
        clear_prefix_entries(&self.db, &idx_prefix)
            .await
            .map_err(|e| Status::internal(format!("failed to clear indexes: {}", e)))?;
        clear_prefix_entries(&self.db, &edge_forward_prefix)
            .await
            .map_err(|e| Status::internal(format!("failed to clear forward edges: {}", e)))?;
        clear_reverse_keys_for_source_type(
            self.db.as_ref(),
            &ctx.database,
            &ctx.namespace,
            &entity_type,
            1000,
        )
        .await
        .map_err(|e| {
            Status::internal(format!(
                "failed to clear reverse edges by dropped source type: {}",
                e
            ))
        })?;
        clear_forward_meta_for_target_type(
            self.db.as_ref(),
            &ctx.database,
            &ctx.namespace,
            &entity_type,
            1000,
        )
        .await
        .map_err(|e| {
            Status::internal(format!(
                "failed to clear forward/meta edges by dropped target type: {}",
                e
            ))
        })?;
        clear_prefix_entries(&self.db, &edge_meta_prefix)
            .await
            .map_err(|e| Status::internal(format!("failed to clear edge metadata: {}", e)))?;
        clear_prefix_entries(&self.db, &edge_reverse_target_prefix)
            .await
            .map_err(|e| Status::internal(format!("failed to clear reverse edges: {}", e)))?;

        // No async cleanup job is needed for this implementation.
        let cleanup_job_id = String::new();
        Ok(Response::new(DropEntityTypeResponse { cleanup_job_id }))
    }

    async fn drop_namespace(
        &self,
        request: Request<DropNamespaceRequest>,
    ) -> Result<Response<DropNamespaceResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        authorize(
            self.db.as_ref(),
            principal.as_ref(),
            "admin.write",
            &ctx.database,
            &ctx.namespace,
            "*",
        )
        .await?;
        let ns = Subspace::namespace(&ctx.database, &ctx.namespace);
        let start = ns.prefix().to_vec();
        let end = ns.range_end().to_vec();

        let existed = !self
            .db
            .get_range(&start, &end, 1)
            .await
            .map_err(|e| Status::internal(format!("namespace probe failed: {}", e)))?
            .is_empty();
        if existed {
            self.db
                .clear_range(&start, &end)
                .await
                .map_err(|e| Status::internal(format!("drop_namespace failed: {}", e)))?;
        }

        Ok(Response::new(DropNamespaceResponse { dropped: existed }))
    }

    async fn list_sites(
        &self,
        request: Request<ListSitesRequest>,
    ) -> Result<Response<ListSitesResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        authorize(
            self.db.as_ref(),
            principal.as_ref(),
            "admin.read",
            &ctx.database,
            &ctx.namespace,
            "*",
        )
        .await?;
        let sites = list_sites(self.db.as_ref())
            .await
            .map_err(|e| Status::internal(format!("list_sites failed: {}", e)))?;

        let response = ListSitesResponse {
            sites: sites
                .into_iter()
                .map(|s| SiteInfo {
                    site_id: s.site_id,
                    site_name: s.site_name,
                    claimed_at: s.claimed_at,
                    status: "active".to_string(),
                })
                .collect(),
        };
        Ok(Response::new(response))
    }

    async fn get_replication_status(
        &self,
        request: Request<GetReplicationStatusRequest>,
    ) -> Result<Response<GetReplicationStatusResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        authorize(
            self.db.as_ref(),
            principal.as_ref(),
            "admin.read",
            &ctx.database,
            &ctx.namespace,
            "*",
        )
        .await?;
        let positions =
            get_replication_positions_scoped(self.db.as_ref(), &ctx.database, &ctx.namespace)
                .await
                .map_err(|e| Status::internal(format!("get_replication_status failed: {}", e)))?;

        let peers = positions
            .into_iter()
            .map(|p| ReplicationPeerStatus {
                remote_site_id: p.remote_site_id,
                last_applied_versionstamp: p
                    .last_applied_versionstamp
                    .map(|vs| vs.to_bytes().to_vec())
                    .unwrap_or_default(),
                lag_events: p.lag_events,
                updated_at: p.updated_at,
            })
            .collect();

        Ok(Response::new(GetReplicationStatusResponse { peers }))
    }

    async fn query_audit_log(
        &self,
        request: Request<QueryAuditLogRequest>,
    ) -> Result<Response<QueryAuditLogResponse>, Status> {
        let principal = principal_from_request(&request);
        let req = request.into_inner();
        let ctx = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context"))?;
        authorize(
            self.db.as_ref(),
            principal.as_ref(),
            "admin.read",
            &ctx.database,
            &ctx.namespace,
            "*",
        )
        .await?;
        let limit = if req.limit == 0 {
            100
        } else {
            req.limit as usize
        };
        let events = query_audit_records(
            self.db.as_ref(),
            if req.principal_id.is_empty() {
                None
            } else {
                Some(req.principal_id.as_str())
            },
            if req.action.is_empty() {
                None
            } else {
                Some(req.action.as_str())
            },
            if req.from_timestamp == 0 {
                None
            } else {
                Some(req.from_timestamp)
            },
            if req.to_timestamp == 0 {
                None
            } else {
                Some(req.to_timestamp)
            },
            limit,
        )
        .await
        .map_err(|e| Status::internal(format!("query_audit_log failed: {}", e)))?;

        Ok(Response::new(QueryAuditLogResponse {
            events: events
                .into_iter()
                .map(|e| pelago_proto::AuditEvent {
                    event_id: e.event_id,
                    timestamp: e.timestamp,
                    principal_id: e.principal_id,
                    action: e.action,
                    resource: e.resource,
                    allowed: e.allowed,
                    reason: e.reason,
                    metadata: e.metadata,
                })
                .collect(),
        }))
    }
}

fn storage_status_to_proto(status: StorageJobStatus) -> i32 {
    match status {
        StorageJobStatus::Pending => JobStatus::Pending.into(),
        StorageJobStatus::Running => JobStatus::Running.into(),
        StorageJobStatus::Completed => JobStatus::Completed.into(),
        StorageJobStatus::Failed => JobStatus::Failed.into(),
    }
}

#[derive(Debug, Deserialize)]
struct EdgeMetaEnvelope {
    #[serde(default)]
    source: Option<pelago_storage::NodeRef>,
    #[serde(default)]
    target: Option<pelago_storage::NodeRef>,
    #[serde(default)]
    label: Option<String>,
}

fn storage_job_to_proto_job(job: pelago_storage::JobState) -> Job {
    Job {
        job_id: job.job_id,
        job_type: job.job_type.name().to_string(),
        status: storage_status_to_proto(job.status),
        created_at: job.created_at,
        updated_at: job.updated_at,
        progress: job.progress_pct,
        error: job.error.unwrap_or_default(),
    }
}

async fn clear_prefix_entries(
    db: &PelagoDb,
    prefix: &[u8],
) -> Result<i64, pelago_core::PelagoError> {
    let mut range_end = prefix.to_vec();
    range_end.push(0xFF);

    let mut deleted = 0i64;
    loop {
        let batch = db.get_range(prefix, &range_end, 1000).await?;
        if batch.is_empty() {
            break;
        }

        let trx = db.create_transaction()?;
        for (key, _) in &batch {
            trx.clear(key);
        }
        trx.commit().await.map_err(|e| {
            pelago_core::PelagoError::Internal(format!("clear prefix batch commit failed: {}", e))
        })?;

        deleted += batch.len() as i64;
    }

    Ok(deleted)
}

async fn clear_reverse_keys_for_source_type(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
    source_entity_type: &str,
    batch_size: usize,
) -> Result<i64, pelago_core::PelagoError> {
    let edge_subspace = Subspace::namespace(database, namespace).edge();
    let meta_prefix = edge_subspace
        .pack()
        .add_marker(pelago_storage::subspace::edge_markers::FORWARD_META)
        .add_string(source_entity_type)
        .build()
        .to_vec();
    let range_end = {
        let mut end = meta_prefix.clone();
        end.push(0xFF);
        end
    };
    let mut range_start = meta_prefix;
    let mut cleared = 0i64;

    loop {
        let rows = db
            .get_range(&range_start, &range_end, batch_size.max(1))
            .await?;
        if rows.is_empty() {
            break;
        }

        let trx = db.create_transaction()?;
        let mut has_mutations = false;

        for (_meta_key, meta_value) in &rows {
            let edge: EdgeMetaEnvelope = match decode_cbor(meta_value) {
                Ok(edge) => edge,
                Err(_) => continue,
            };
            let (source, target, label) = match (
                edge.source.as_ref(),
                edge.target.as_ref(),
                edge.label.as_deref(),
            ) {
                (Some(source), Some(target), Some(label)) if !label.is_empty() => {
                    (source, target, label)
                }
                _ => continue,
            };

            let reverse_key = edge_subspace
                .pack()
                .add_marker(pelago_storage::subspace::edge_markers::REVERSE)
                .add_string(&target.database)
                .add_string(&target.namespace)
                .add_string(&target.entity_type)
                .add_string(&target.node_id)
                .add_string(label)
                .add_string(&source.entity_type)
                .add_string(&source.node_id)
                .build();
            trx.clear(reverse_key.as_ref());
            has_mutations = true;
            cleared = cleared.saturating_add(1);
        }

        if has_mutations {
            trx.commit().await.map_err(|e| {
                pelago_core::PelagoError::Internal(format!(
                    "clear reverse keys by source type commit failed: {}",
                    e
                ))
            })?;
        }

        if rows.len() < batch_size.max(1) {
            break;
        }
        if let Some((last_key, _)) = rows.last() {
            let mut next = last_key.clone();
            next.push(0x00);
            range_start = next;
        } else {
            break;
        }
    }

    Ok(cleared)
}

async fn clear_forward_meta_for_target_type(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
    target_entity_type: &str,
    batch_size: usize,
) -> Result<i64, pelago_core::PelagoError> {
    let edge_subspace = Subspace::namespace(database, namespace).edge();
    let meta_prefix = edge_subspace
        .pack()
        .add_marker(pelago_storage::subspace::edge_markers::FORWARD_META)
        .build()
        .to_vec();
    let range_end = {
        let mut end = meta_prefix.clone();
        end.push(0xFF);
        end
    };
    let mut range_start = meta_prefix;
    let mut cleared = 0i64;

    loop {
        let rows = db
            .get_range(&range_start, &range_end, batch_size.max(1))
            .await?;
        if rows.is_empty() {
            break;
        }

        let trx = db.create_transaction()?;
        let mut has_mutations = false;

        for (meta_key, meta_value) in &rows {
            let edge: EdgeMetaEnvelope = match decode_cbor(meta_value) {
                Ok(edge) => edge,
                Err(_) => continue,
            };
            let (source, target, label) = match (
                edge.source.as_ref(),
                edge.target.as_ref(),
                edge.label.as_deref(),
            ) {
                (Some(source), Some(target), Some(label)) if !label.is_empty() => {
                    (source, target, label)
                }
                _ => continue,
            };
            if target.entity_type != target_entity_type {
                continue;
            }

            trx.clear(meta_key);

            if let Some(forward_key) = forward_key_from_meta_key(&edge_subspace, meta_key) {
                trx.clear(&forward_key);
            }

            let reverse_key = edge_subspace
                .pack()
                .add_marker(pelago_storage::subspace::edge_markers::REVERSE)
                .add_string(&target.database)
                .add_string(&target.namespace)
                .add_string(&target.entity_type)
                .add_string(&target.node_id)
                .add_string(label)
                .add_string(&source.entity_type)
                .add_string(&source.node_id)
                .build();
            trx.clear(reverse_key.as_ref());

            // Bidirectional reverse-direction key (safe clear even if absent).
            let rev_reverse_key = edge_subspace
                .pack()
                .add_marker(pelago_storage::subspace::edge_markers::REVERSE)
                .add_string(&source.database)
                .add_string(&source.namespace)
                .add_string(&source.entity_type)
                .add_string(&source.node_id)
                .add_string(label)
                .add_string(&target.entity_type)
                .add_string(&target.node_id)
                .build();
            trx.clear(rev_reverse_key.as_ref());

            has_mutations = true;
            cleared = cleared.saturating_add(1);
        }

        if has_mutations {
            trx.commit().await.map_err(|e| {
                pelago_core::PelagoError::Internal(format!(
                    "clear forward/meta keys by target type commit failed: {}",
                    e
                ))
            })?;
        }

        if rows.len() < batch_size.max(1) {
            break;
        }
        if let Some((last_key, _)) = rows.last() {
            let mut next = last_key.clone();
            next.push(0x00);
            range_start = next;
        } else {
            break;
        }
    }

    Ok(cleared)
}

fn forward_key_from_meta_key(edge_subspace: &Subspace, meta_key: &[u8]) -> Option<Vec<u8>> {
    let marker_pos = edge_subspace.prefix().len();
    if meta_key.len() <= marker_pos {
        return None;
    }
    if meta_key[marker_pos] != pelago_storage::subspace::edge_markers::FORWARD_META {
        return None;
    }

    let mut out = meta_key.to_vec();
    out[marker_pos] = pelago_storage::subspace::edge_markers::FORWARD;
    Some(out)
}

async fn count_forward_meta_rows_for_target_type_up_to(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
    target_entity_type: &str,
    batch_size: usize,
    cap: usize,
) -> Result<usize, pelago_core::PelagoError> {
    if cap == 0 {
        return Ok(0);
    }

    let edge_subspace = Subspace::namespace(database, namespace).edge();
    let meta_prefix = edge_subspace
        .pack()
        .add_marker(pelago_storage::subspace::edge_markers::FORWARD_META)
        .build()
        .to_vec();
    let range_end = {
        let mut end = meta_prefix.clone();
        end.push(0xFF);
        end
    };
    let mut range_start = meta_prefix;
    let mut matched = 0usize;

    loop {
        let rows = db
            .get_range(&range_start, &range_end, batch_size.max(1))
            .await?;
        if rows.is_empty() {
            break;
        }

        for (_meta_key, meta_value) in &rows {
            let edge: EdgeMetaEnvelope = match decode_cbor(meta_value) {
                Ok(edge) => edge,
                Err(_) => continue,
            };
            let Some(target) = edge.target.as_ref() else {
                continue;
            };
            if target.entity_type != target_entity_type {
                continue;
            }

            matched = matched.saturating_add(1);
            if matched >= cap {
                return Ok(matched);
            }
        }

        if rows.len() < batch_size.max(1) {
            break;
        }
        if let Some((last_key, _)) = rows.last() {
            let mut next = last_key.clone();
            next.push(0x00);
            range_start = next;
        } else {
            break;
        }
    }

    Ok(matched)
}

async fn count_prefix_entries_up_to(
    db: &PelagoDb,
    prefix: &[u8],
    cap: usize,
) -> Result<usize, pelago_core::PelagoError> {
    if cap == 0 {
        return Ok(0);
    }
    let mut range_end = prefix.to_vec();
    range_end.push(0xFF);
    let rows = db.get_range(prefix, &range_end, cap).await?;
    Ok(rows.len())
}
