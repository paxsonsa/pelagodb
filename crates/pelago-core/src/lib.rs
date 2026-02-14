//! PelagoDB Core Types and Utilities
//!
//! This crate provides the foundational types used throughout PelagoDB:
//! - `NodeId` and `EdgeId` for entity identification
//! - `Value` for property values
//! - Encoding utilities for FDB key construction
//! - Error types with gRPC status mapping

pub mod config;
pub mod encoding;
pub mod errors;
pub mod schema;
pub mod types;

pub use config::*;
pub use errors::*;
pub use types::*;
