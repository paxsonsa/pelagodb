//! AdminService gRPC handler
//!
//! Handles administrative operations:
//! - DropIndex: Remove an index from an entity type
//! - StripProperty: Remove a property from all nodes
//! - GetJobStatus: Check status of async jobs
//! - DropEntityType: Remove an entity type
//! - DropNamespace: Remove an entire namespace

use crate::authz::{authorize, principal_from_request};
use pelago_proto::{
    admin_service_server::AdminService, DropEntityTypeRequest, DropEntityTypeResponse,
    DropIndexRequest, DropIndexResponse, DropNamespaceRequest, DropNamespaceResponse,
    GetJobStatusRequest, GetJobStatusResponse, GetReplicationStatusRequest,
    GetReplicationStatusResponse, Job, JobStatus, ListJobsRequest, ListJobsResponse,
    ListSitesRequest, ListSitesResponse, QueryAuditLogRequest, QueryAuditLogResponse,
    ReplicationPeerStatus, SiteInfo, StripPropertyRequest, StripPropertyResponse,
};
use pelago_storage::{
    get_replication_positions_scoped, list_sites, query_audit_records,
    JobStatus as StorageJobStatus, JobStore, JobType, PelagoDb, Subspace,
};
use std::sync::Arc;
use tonic::{Request, Response, Status};

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
