//! HealthService gRPC handler
//!
//! Implements health check endpoints for load balancer probes and monitoring.

use pelago_proto::{
    health_service_server::HealthService, HealthCheckRequest, HealthCheckResponse, ServingStatus,
};
use tonic::{Request, Response, Status};

/// Health service implementation
pub struct HealthServiceImpl {
    /// Whether the service is currently serving
    serving: bool,
}

impl HealthServiceImpl {
    pub fn new() -> Self {
        Self { serving: true }
    }
}

impl Default for HealthServiceImpl {
    fn default() -> Self {
        Self::new()
    }
}

#[tonic::async_trait]
impl HealthService for HealthServiceImpl {
    async fn check(
        &self,
        _request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        let status = if self.serving {
            ServingStatus::Serving
        } else {
            ServingStatus::NotServing
        };

        Ok(Response::new(HealthCheckResponse {
            status: status.into(),
        }))
    }
}
