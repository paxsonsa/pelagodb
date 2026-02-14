//! PelagoDB Protocol Buffers and gRPC definitions
//!
//! This crate contains the generated protobuf types and gRPC service definitions.

pub mod pelago {
    pub mod v1 {
        tonic::include_proto!("pelago.v1");
    }
}

pub use pelago::v1::*;
