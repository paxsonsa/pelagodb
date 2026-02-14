//! PelagoDB Storage Layer
//!
//! This crate provides FDB-backed storage operations:
//! - Schema registry
//! - Node CRUD with index maintenance
//! - Edge CRUD with bidirectional pairs
//! - CDC entry emission
//! - ID allocation

pub mod cache;
pub mod cdc;
pub mod db;
pub mod edge;
pub mod ids;
pub mod index;
pub mod jobs;
pub mod node;
pub mod schema;
pub mod subspace;

pub use cache::SchemaCache;
pub use cdc::{CdcEntry, CdcWriter};
pub use db::{init_fdb_network, PelagoDb, PelagoTxn};
pub use edge::{EdgeStore, NodeRef, StoredEdge};
pub use ids::IdAllocator;
pub use node::{NodeStore, StoredNode};
pub use schema::SchemaRegistry;
pub use subspace::Subspace;
