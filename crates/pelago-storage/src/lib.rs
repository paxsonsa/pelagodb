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
#[cfg(feature = "cache")]
pub mod rocks_cache;
pub mod consumer;
pub mod db;
pub mod edge;
pub mod ids;
pub mod index;
pub mod job_executor;
pub mod job_worker;
pub mod jobs;
pub mod node;
pub mod schema;
pub mod subspace;

pub use cache::SchemaCache;
pub use cdc::{read_cdc_entries, CdcAccumulator, CdcEntry, CdcOperation, Versionstamp};
pub use consumer::{fetch_all_checkpoints, CdcConsumer, ConsumerConfig};
pub use db::{init_fdb_network, PelagoDb, PelagoTxn};
pub use edge::{EdgeStore, NodeRef, StoredEdge};
pub use ids::IdAllocator;
pub use job_worker::JobWorker;
pub use jobs::{JobState, JobStatus, JobStore, JobType};
pub use node::{NodeStore, StoredNode};
pub use schema::SchemaRegistry;
pub use subspace::Subspace;

#[cfg(feature = "cache")]
pub use rocks_cache::{CachedReadPath, CdcProjector, ReadConsistency, RocksCacheConfig, RocksCacheStore};
