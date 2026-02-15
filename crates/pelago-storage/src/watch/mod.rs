//! Watch System — reactive subscriptions for real-time change notifications
//!
//! The watch module provides three subscription types:
//! - **Point watches**: O(1) lookup for specific node/edge changes
//! - **Query watches**: CEL/PQL filter with enter/update/exit semantics
//! - **Namespace watches**: all changes within a namespace
//!
//! Architecture:
//! - `SubscriptionRegistry` manages lifecycle, limits, and connection tracking
//! - `DispatcherState` holds subscription maps (shared between registry and dispatcher)
//! - `WatchDispatcher` runs the CDC consumer loop and routes events
//! - `TtlManager` periodically expires subscriptions

pub mod dispatcher;
pub mod registry;
pub mod resume;
pub mod subscriptions;
pub mod ttl_manager;

pub use dispatcher::{DispatcherState, WatchDispatcher};
pub use registry::{RegistryConfig, SubscriptionRegistry};
pub use resume::{validate_resume_position, ResumeStatus};
pub use subscriptions::*;
pub use ttl_manager::TtlManager;
