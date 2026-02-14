//! AdminService gRPC handler
//!
//! Handles administrative operations:
//! - DropIndex: Remove an index from an entity type
//! - StripProperty: Remove a property from all nodes
//! - GetJobStatus: Check status of async jobs
//! - DropEntityType: Remove an entity type
//! - DropNamespace: Remove an entire namespace

use pelago_proto::{
    admin_service_server::AdminService, DropEntityTypeRequest, DropEntityTypeResponse,
    DropIndexRequest, DropIndexResponse, DropNamespaceRequest, DropNamespaceResponse,
    GetJobStatusRequest, GetJobStatusResponse, Job, JobStatus, StripPropertyRequest,
    StripPropertyResponse,
};
use pelago_storage::PelagoDb;
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
        let req = request.into_inner();
        let _ctx = req.context.ok_or_else(|| Status::invalid_argument("missing context"))?;
        let _entity_type = req.entity_type;
        let _property_name = req.property_name;

        // TODO: Implement index dropping with FDB
        // 1. Update schema to remove index
        // 2. Delete all index entries for this property
        // 3. Return count of deleted entries

        Ok(Response::new(DropIndexResponse { entries_deleted: 0 }))
    }

    async fn strip_property(
        &self,
        request: Request<StripPropertyRequest>,
    ) -> Result<Response<StripPropertyResponse>, Status> {
        let req = request.into_inner();
        let _ctx = req.context.ok_or_else(|| Status::invalid_argument("missing context"))?;
        let _entity_type = req.entity_type;
        let _property_name = req.property_name;

        // TODO: Implement property stripping with background job
        // 1. Create background job
        // 2. Iterate all nodes of type
        // 3. Remove property from each
        // 4. Update indexes

        let job_id = format!("strip_{}_{}", _entity_type, _property_name);
        Ok(Response::new(StripPropertyResponse { job_id }))
    }

    async fn get_job_status(
        &self,
        request: Request<GetJobStatusRequest>,
    ) -> Result<Response<GetJobStatusResponse>, Status> {
        let req = request.into_inner();
        let _ctx = req.context.ok_or_else(|| Status::invalid_argument("missing context"))?;
        let job_id = req.job_id;

        // TODO: Implement job status retrieval from FDB

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as i64;

        Ok(Response::new(GetJobStatusResponse {
            job: Some(Job {
                job_id,
                job_type: "unknown".to_string(),
                status: JobStatus::Pending.into(),
                created_at: now,
                updated_at: now,
                progress: 0.0,
                error: String::new(),
            }),
        }))
    }

    async fn drop_entity_type(
        &self,
        request: Request<DropEntityTypeRequest>,
    ) -> Result<Response<DropEntityTypeResponse>, Status> {
        let req = request.into_inner();
        let _ctx = req.context.ok_or_else(|| Status::invalid_argument("missing context"))?;
        let _entity_type = req.entity_type;

        // TODO: Implement entity type dropping with FDB
        // 1. Delete schema
        // 2. Create background job to delete all nodes
        // 3. Create background job to clean up orphaned edges

        let cleanup_job_id = format!("cleanup_{}", _entity_type);
        Ok(Response::new(DropEntityTypeResponse { cleanup_job_id }))
    }

    async fn drop_namespace(
        &self,
        request: Request<DropNamespaceRequest>,
    ) -> Result<Response<DropNamespaceResponse>, Status> {
        let req = request.into_inner();
        let _ctx = req.context.ok_or_else(|| Status::invalid_argument("missing context"))?;

        // TODO: Implement namespace dropping with FDB
        // 1. Delete all data in namespace subspace
        // This is a dangerous operation!

        Ok(Response::new(DropNamespaceResponse { dropped: false }))
    }
}
