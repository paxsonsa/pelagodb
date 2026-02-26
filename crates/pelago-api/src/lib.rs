//! PelagoDB gRPC API Layer
//!
//! This crate wires storage and query operations into tonic service handlers:
//! - SchemaService: Schema registration and retrieval
//! - NodeService: Node CRUD operations
//! - EdgeService: Edge operations
//! - QueryService: CEL-based queries and graph traversals
//! - AdminService: Administrative operations
//! - HealthService: Health check endpoints

pub mod admin_service;
pub mod auth_service;
pub mod authz;
pub mod edge_service;
pub mod error;
pub mod health_service;
pub mod node_service;
pub mod query_service;
pub mod replication_service;
pub mod schema_service;
mod service_common;
pub mod watch_service;

pub use admin_service::AdminServiceImpl;
pub use auth_service::{AuthPrincipal, AuthRuntime, AuthServiceImpl};
pub use edge_service::EdgeServiceImpl;
pub use error::{to_status, IntoStatus, ToStatus};
pub use health_service::HealthServiceImpl;
pub use node_service::NodeServiceImpl;
pub use query_service::QueryServiceImpl;
pub use replication_service::ReplicationServiceImpl;
pub use schema_service::SchemaServiceImpl;
pub use watch_service::{WatchRegistry, WatchServiceImpl};
