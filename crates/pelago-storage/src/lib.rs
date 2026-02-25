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
pub mod consumer;
pub mod db;
pub mod edge;
pub mod ids;
pub mod index;
pub mod job_executor;
pub mod job_worker;
pub mod jobs;
mod mutation;
pub mod node;
pub mod replication;
#[cfg(feature = "cache")]
pub mod rocks_cache;
pub mod schema;
pub mod security;
pub mod subspace;
pub mod term_index;
pub mod watch_state;

pub use cache::SchemaCache;
pub use cdc::{
    append_cdc_entry, cdc_position_exists, oldest_cdc_position, read_cdc_entries, CdcAccumulator,
    CdcEntry, CdcOperation, Versionstamp,
};
pub use consumer::{fetch_all_checkpoints, CdcConsumer, ConsumerConfig};
pub use db::{init_fdb_network, PelagoDb, PelagoTxn};
pub use edge::{EdgeStore, NodeRef, StoredEdge};
pub use ids::IdAllocator;
pub use job_worker::JobWorker;
pub use jobs::{JobState, JobStatus, JobStore, JobType};
pub use node::{NodeStore, StoredNode};
pub use replication::{
    claim_site, get_replication_positions, get_replication_positions_scoped, get_replicator_lease,
    list_sites, try_acquire_replicator_lease, update_replication_position,
    update_replication_position_scoped, ReplicationLease, ReplicationPosition, SiteClaim,
};
pub use schema::SchemaRegistry;
pub use security::{
    append_audit_record, check_permission, cleanup_audit_records, delete_policy, get_policy,
    get_token_session_by_access, get_token_session_by_refresh, list_policies, query_audit_records,
    revoke_access_token, revoke_refresh_token, upsert_policy, upsert_token_session, AuditRecord,
    AuthPolicy, AuthTokenSession, PolicyPermission,
};
pub use subspace::Subspace;
pub use watch_state::{
    cleanup_query_watch_states, delete_query_watch_state, get_query_watch_state,
    upsert_query_watch_state, QueryWatchState,
};

#[cfg(feature = "cache")]
pub use rocks_cache::{
    CachedReadPath, CdcProjector, ReadConsistency, RocksCacheConfig, RocksCacheStore,
};
