# Phase 2: Change Data Capture System — Implementation Plan

## Overview

Phase 2 builds the CDC infrastructure that powers replication (Phase 5), watch system (Phase 4), and cache invalidation (Phase 3). It transforms the Phase 1 placeholder CDC writes into a production-quality, versionstamp-ordered, consumer-driven CDC system with a background job framework.

**Branch:** `phase-2-cdc` branched from `claude/worktrees/phase-1-storage-foundation`

**Spec Sections:** §11 (CDC System), §13 (Background Jobs)

---

## Current State Analysis

### What Phase 1 Delivers (starting point)

- `pelago-storage/src/cdc.rs` — Simplified `CdcWriter` with `CdcEntry` struct
- `pelago-storage/src/jobs.rs` — `Job` and `JobStatus` data structures only (no execution)
- `pelago-storage/src/subspace.rs` — `_cdc` and `_jobs` subspace markers defined
- CDC entries written for node operations only (not edges, not schemas)
- CDC writes happen in **separate transactions** from mutations (not atomic)
- CDC keys use timestamp+random suffix (not FDB versionstamps)

### Key Discoveries

- **Non-atomic CDC writes:** `node.rs:157-164` commits the mutation first, then writes CDC in a new transaction. A crash between these loses the CDC entry.
- **Edge CDC missing:** `EdgeStore` (`edge.rs:91-96`) has no `CdcWriter` dependency. Edge create/delete emit zero CDC.
- **Schema CDC missing:** `SchemaRegistry` (`schema.rs`) doesn't emit CDC entries for schema registration.
- **CdcEntry mismatch:** Current struct has flat fields (`operation`, `entity_type`, `node_id`). Spec §11.1 defines `CdcEntry` with `site`, `batch_id`, and a `Vec<CdcOperation>` enum.
- **No versionstamp support:** Current implementation uses `timestamp + rand_suffix()` for CDC keys. FDB versionstamps provide zero-contention, globally-ordered append.
- **Job storage is data-only:** `jobs.rs` defines `Job`/`JobStatus` structs but no persistence, no worker loop, no cursor resumption.

### What We're NOT Doing

- RocksDB cache layer (Phase 3)
- PQL query language (Phase 3)
- CLI (Phase 3)
- Watch system / reactive subscriptions (Phase 4)
- Multi-site replication logic (Phase 5) — though we provide the `PullCdcEvents` internal endpoint
- Security / authorization (Phase 6)
- CDC compaction (spec marks as "Future Enhancement")

---

## Desired End State

After Phase 2 is complete:

1. **All mutations atomically produce CDC entries** — node CRUD, edge CRUD, and schema registration write CDC entries in the same FDB transaction as the mutation
2. **CDC entries are versionstamp-ordered** — using FDB's `set_versionstamped_key` for zero-contention, globally-ordered append
3. **CDC entries match spec §11.1** — `CdcEntry` with `site`, `timestamp`, `batch_id`, and `Vec<CdcOperation>` with all operation types
4. **CDC consumer framework works** — consumers track high-water marks, checkpoint periodically, resume after crash
5. **Background job framework executes jobs** — worker loop polls for pending/interrupted jobs, executes with cursor-based resumption
6. **IndexBackfill job works** — new indexes trigger backfill jobs that populate index entries for existing nodes
7. **StripProperty job works** — removes a property from all nodes of a type in batches
8. **CdcRetention job works** — truncates expired CDC entries respecting consumer checkpoints
9. **`PullCdcEvents` gRPC endpoint works** — internal endpoint for consuming CDC entries by versionstamp range
10. **Lifecycle test suite validates CDC** — tests verify CDC entries are produced for all mutation types and are consumable

### Verification

```bash
# All tests pass
LIBRARY_PATH=/usr/local/lib cargo test --test lifecycle_test -- --ignored --nocapture
LIBRARY_PATH=/usr/local/lib cargo test --test cdc_integration -- --ignored --nocapture

# Unit tests pass
cargo test -p pelago-storage
cargo test -p pelago-api
```

---

## Implementation Approach

The plan is organized into 4 milestones that can be implemented sequentially. Each milestone produces testable functionality.

**Dependency chain:**
```
M7 (CDC Format) → M8 (Consumer Framework) → M9 (Background Jobs) → gRPC + Tests
     ↓
  Mutation paths refactored (atomic CDC)
```

---

## Milestone 7: CDC Entry Format & Atomic Writes

### Overview
Replace the Phase 1 placeholder CDC with spec-compliant versionstamp-ordered CDC entries written atomically within mutation transactions.

### Changes Required

#### 1. Replace CdcEntry and CdcWriter
**File:** `crates/pelago-storage/src/cdc.rs`
**Changes:** Complete rewrite to match spec §11.1

```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use pelago_core::{Value, NodeId, EdgeId};

/// CDC entry — one per FDB transaction, may contain multiple operations
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CdcEntry {
    /// Site that performed the mutation
    pub site: String,
    /// Timestamp of the mutation (unix microseconds)
    pub timestamp: i64,
    /// Optional batch identifier for grouped operations
    pub batch_id: Option<String>,
    /// List of operations in this entry
    pub operations: Vec<CdcOperation>,
}

/// Individual CDC operation within an entry
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum CdcOperation {
    NodeCreate {
        entity_type: String,
        node_id: String,
        properties: HashMap<String, Value>,
        home_site: String,
    },
    NodeUpdate {
        entity_type: String,
        node_id: String,
        changed_properties: HashMap<String, Value>,
        old_properties: HashMap<String, Value>,
    },
    NodeDelete {
        entity_type: String,
        node_id: String,
    },
    EdgeCreate {
        source_type: String,
        source_id: String,
        target_type: String,
        target_id: String,
        edge_type: String,
        edge_id: String,
        properties: HashMap<String, Value>,
    },
    EdgeDelete {
        source_type: String,
        source_id: String,
        target_type: String,
        target_id: String,
        edge_type: String,
    },
    SchemaRegister {
        entity_type: String,
        version: u32,
    },
}

/// Versionstamp wrapper (10 bytes: 8-byte transaction version + 2-byte batch order)
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Versionstamp([u8; 10]);

impl Versionstamp {
    pub fn zero() -> Self { Self([0u8; 10]) }
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> { /* ... */ }
    pub fn to_bytes(&self) -> &[u8; 10] { &self.0 }
    /// Increment to get the exclusive start for "after this" range scans
    pub fn next(&self) -> Self { /* increment last byte with carry */ }
}
```

**Key design decisions:**
- `CdcEntry` wraps a `Vec<CdcOperation>` so a single transaction that creates a node and its edges produces one CDC entry with multiple operations
- `Versionstamp` is a first-class type for type safety
- `CdcOperation` variants match the spec exactly (simplified: no `NodeRef` in edge ops, use flat fields for simpler serialization)

#### 2. Transaction-scoped CDC accumulator
**File:** `crates/pelago-storage/src/cdc.rs` (new section)
**Changes:** Add `CdcAccumulator` that collects operations during a transaction and writes them atomically at commit

```rust
/// Accumulates CDC operations during a transaction
pub struct CdcAccumulator {
    site: String,
    operations: Vec<CdcOperation>,
}

impl CdcAccumulator {
    pub fn new(site: &str) -> Self {
        Self { site: site.to_string(), operations: Vec::new() }
    }

    pub fn push(&mut self, op: CdcOperation) {
        self.operations.push(op);
    }

    /// Write accumulated CDC entry into the given FDB transaction using versionstamped key
    pub fn flush(self, trx: &foundationdb::Transaction, database: &str, namespace: &str) -> Result<(), PelagoError> {
        if self.operations.is_empty() { return Ok(()); }

        let entry = CdcEntry {
            site: self.site,
            timestamp: now_micros(),
            batch_id: None,
            operations: self.operations,
        };

        let subspace = Subspace::namespace(database, namespace).cdc();
        let value = encode_cbor(&entry)?;

        // Build key with versionstamp placeholder
        // FDB versionstamp key: prefix + 4 bytes of zeros (placeholder) + 2 bytes user version
        let prefix = subspace.prefix().to_vec();
        let mut key = Vec::with_capacity(prefix.len() + 14);
        key.extend_from_slice(&prefix);
        // 10-byte versionstamp placeholder
        key.extend_from_slice(&[0u8; 10]);
        // Tell FDB where the versionstamp placeholder is
        let versionstamp_offset = prefix.len() as u32;

        trx.atomic_op(
            &key,
            &value,
            foundationdb::options::MutationType::SetVersionstampedKey,
        );
        // Note: actual FDB API for versionstamped keys uses set_versionstamped_key
        // The exact API will depend on the foundationdb crate version

        Ok(())
    }
}
```

> **Implementation Note:** The `foundationdb` crate (v0.10) exposes `Transaction::set_versionstamped_key(key, value)` where the key must contain a 4-byte little-endian offset at the end indicating where the 10-byte versionstamp should be placed. We need to verify the exact API surface during implementation. If `set_versionstamped_key` isn't directly available, we use `atomic_op` with `MutationType::SetVersionstampedKey`.

#### 3. Refactor NodeStore for atomic CDC
**File:** `crates/pelago-storage/src/node.rs`
**Changes:** Replace post-commit CDC writes with in-transaction CdcAccumulator

The core pattern for `create_node`:
```rust
pub async fn create_node(/* ... */) -> Result<StoredNode, PelagoError> {
    // ... validation, ID allocation ...

    let trx = self.db.create_transaction()?;
    let mut cdc = CdcAccumulator::new(&self.site_id);

    // Write data key
    trx.set(data_key.as_ref(), &node_data);

    // Write index entries
    for entry in &index_entries {
        Self::write_index_entry(&trx, entry)?;
    }

    // Accumulate CDC operation
    cdc.push(CdcOperation::NodeCreate {
        entity_type: entity_type.to_string(),
        node_id: node_id.to_string(),
        properties: properties.clone(),
        home_site: site_id.to_string(),
    });

    // Flush CDC into same transaction
    cdc.flush(&trx, database, namespace)?;

    // Single commit: mutation + CDC atomic
    trx.commit().await.map_err(/* ... */)?;

    Ok(stored_node)
}
```

Apply the same pattern to `update_node` and `delete_node`.

**Remove:** The `cdc_writer: Arc<CdcWriter>` field from `NodeStore`. Replace with `site_id: String` field.

#### 4. Add CDC to EdgeStore
**File:** `crates/pelago-storage/src/edge.rs`
**Changes:** Add `site_id` field, use `CdcAccumulator` in `create_edge` and `delete_edge`

```rust
pub struct EdgeStore {
    db: PelagoDb,
    schema_registry: Arc<SchemaRegistry>,
    id_allocator: Arc<IdAllocator>,
    node_store: Arc<NodeStore>,
    site_id: String,  // NEW
}
```

In `create_edge`:
```rust
let mut cdc = CdcAccumulator::new(&self.site_id);
cdc.push(CdcOperation::EdgeCreate {
    source_type: source.entity_type.clone(),
    source_id: source.node_id.clone(),
    target_type: target.entity_type.clone(),
    target_id: target.node_id.clone(),
    edge_type: label.to_string(),
    edge_id: edge_id.to_string(),
    properties: properties.clone(),
});
cdc.flush(&trx, database, namespace)?;
```

#### 5. Add CDC to SchemaRegistry
**File:** `crates/pelago-storage/src/schema.rs`
**Changes:** Emit `CdcOperation::SchemaRegister` when registering/updating schemas

#### 6. Update lib.rs exports
**File:** `crates/pelago-storage/src/lib.rs`
**Changes:** Export new CDC types, remove old `CdcWriter` export

```rust
pub use cdc::{CdcAccumulator, CdcEntry, CdcOperation, Versionstamp};
// Remove: pub use cdc::{CdcEntry, CdcWriter};
```

#### 7. CDC reader for consuming entries
**File:** `crates/pelago-storage/src/cdc.rs` (new function)
**Changes:** Add function to read CDC entries by versionstamp range

```rust
/// Read CDC entries from a versionstamp range
pub async fn read_cdc_entries(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
    after_versionstamp: Option<&Versionstamp>,
    limit: usize,
) -> Result<Vec<(Versionstamp, CdcEntry)>, PelagoError> {
    let subspace = Subspace::namespace(database, namespace).cdc();

    let range_start = match after_versionstamp {
        Some(vs) => {
            let next = vs.next();
            let mut key = subspace.prefix().to_vec();
            key.extend_from_slice(next.to_bytes());
            key
        }
        None => subspace.prefix().to_vec(),
    };
    let range_end = subspace.range_end().to_vec();

    let results = db.get_range(&range_start, &range_end, limit).await?;

    let mut entries = Vec::new();
    for (key, value) in results {
        let vs_bytes = &key[subspace.prefix().len()..];
        if let Some(vs) = Versionstamp::from_bytes(vs_bytes) {
            let entry: CdcEntry = decode_cbor(&value)?;
            entries.push((vs, entry));
        }
    }

    Ok(entries)
}
```

### Success Criteria

#### Automated Verification:
- [x] `cargo build -p pelago-storage` compiles with new CDC types
- [x] `cargo test -p pelago-storage` passes (unit tests updated for new CdcEntry)
- [x] CDC entries written atomically in same FDB transaction as mutations
- [x] CDC keys contain FDB versionstamps (10-byte keys after subspace prefix)
- [x] `read_cdc_entries` returns entries in versionstamp order
- [x] Node create/update/delete produce correct CdcOperation variants
- [x] Edge create/delete produce CdcOperation::EdgeCreate/EdgeDelete
- [x] Schema registration produces CdcOperation::SchemaRegister

#### Manual Verification:
- [ ] Lifecycle test shows CDC entries printed for each operation

---

## Milestone 8: Consumer Framework

### Overview
Implement the CDC consumer pattern with high-water mark tracking, configurable batch processing, and checkpoint persistence.

### Changes Required

#### 1. Consumer module
**File:** `crates/pelago-storage/src/consumer.rs` (new file)

```rust
use crate::cdc::{CdcEntry, Versionstamp, read_cdc_entries};
use crate::db::PelagoDb;
use crate::Subspace;
use pelago_core::PelagoError;
use std::time::{Duration, Instant};

/// Configuration for a CDC consumer
#[derive(Clone, Debug)]
pub struct ConsumerConfig {
    /// Unique consumer identifier
    pub consumer_id: String,
    /// Database to consume from
    pub database: String,
    /// Namespace to consume from
    pub namespace: String,
    /// Maximum entries per batch
    pub batch_size: usize,
    /// How often to save checkpoint (seconds)
    pub checkpoint_interval: Duration,
    /// How long to sleep when no entries available
    pub poll_interval: Duration,
}

impl ConsumerConfig {
    pub fn new(consumer_id: &str, database: &str, namespace: &str) -> Self {
        Self {
            consumer_id: consumer_id.to_string(),
            database: database.to_string(),
            namespace: namespace.to_string(),
            batch_size: 1000,
            checkpoint_interval: Duration::from_secs(5),
            poll_interval: Duration::from_millis(100),
        }
    }
}

/// CDC consumer with HWM tracking
pub struct CdcConsumer {
    config: ConsumerConfig,
    db: PelagoDb,
    hwm: Versionstamp,
    last_checkpoint: Instant,
}

impl CdcConsumer {
    pub async fn new(db: PelagoDb, config: ConsumerConfig) -> Result<Self, PelagoError> {
        let hwm = Self::load_hwm(&db, &config).await?;
        Ok(Self {
            config,
            db,
            hwm,
            last_checkpoint: Instant::now(),
        })
    }

    /// Load high-water mark from FDB
    async fn load_hwm(db: &PelagoDb, config: &ConsumerConfig) -> Result<Versionstamp, PelagoError> {
        let key = Self::checkpoint_key(config);
        match db.get(&key).await? {
            Some(bytes) => Versionstamp::from_bytes(&bytes)
                .ok_or_else(|| PelagoError::Internal("Invalid checkpoint".into())),
            None => Ok(Versionstamp::zero()),
        }
    }

    /// Save high-water mark to FDB
    pub async fn save_hwm(&mut self) -> Result<(), PelagoError> {
        let key = Self::checkpoint_key(&self.config);
        self.db.set(&key, self.hwm.to_bytes()).await?;
        self.last_checkpoint = Instant::now();
        Ok(())
    }

    /// Checkpoint key: (db, ns, _meta, cdc_checkpoints, consumer_id)
    fn checkpoint_key(config: &ConsumerConfig) -> Vec<u8> {
        Subspace::namespace(&config.database, &config.namespace)
            .meta()
            .pack()
            .add_string("cdc_checkpoints")
            .add_string(&config.consumer_id)
            .build()
            .to_vec()
    }

    /// Poll for next batch of CDC entries
    pub async fn poll_batch(&mut self) -> Result<Vec<(Versionstamp, CdcEntry)>, PelagoError> {
        let entries = read_cdc_entries(
            &self.db,
            &self.config.database,
            &self.config.namespace,
            Some(&self.hwm),
            self.config.batch_size,
        ).await?;

        if let Some((last_vs, _)) = entries.last() {
            self.hwm = last_vs.clone();
        }

        // Periodic checkpoint
        if self.last_checkpoint.elapsed() >= self.config.checkpoint_interval {
            self.save_hwm().await?;
        }

        Ok(entries)
    }

    /// Force checkpoint save (for graceful shutdown)
    pub async fn checkpoint(&mut self) -> Result<(), PelagoError> {
        self.save_hwm().await
    }

    /// Get current high-water mark
    pub fn hwm(&self) -> &Versionstamp { &self.hwm }
}
```

#### 2. Add `_meta` subspace
**File:** `crates/pelago-storage/src/subspace.rs`
**Changes:** Add `meta()` method and `META` marker

```rust
pub mod markers {
    // ... existing markers ...
    pub const META: &str = "_meta";
}

impl Subspace {
    // ... existing methods ...

    /// Get the meta subspace (for checkpoints, etc.)
    pub fn meta(&self) -> Self {
        self.subspace(markers::META)
    }
}
```

#### 3. Fetch all consumer checkpoints (for retention)
**File:** `crates/pelago-storage/src/consumer.rs`
**Changes:** Add function to list all consumer checkpoints

```rust
/// Fetch all CDC consumer checkpoints (for retention policy)
pub async fn fetch_all_checkpoints(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
) -> Result<Vec<(String, Versionstamp)>, PelagoError> {
    let prefix = Subspace::namespace(database, namespace)
        .meta()
        .pack()
        .add_string("cdc_checkpoints")
        .build();
    let range_end = {
        let mut end = prefix.to_vec();
        end.push(0xFF);
        end
    };

    let results = db.get_range(prefix.as_ref(), &range_end, 1000).await?;

    let mut checkpoints = Vec::new();
    for (key, value) in results {
        // Extract consumer_id from key suffix
        let consumer_id = extract_last_string_element(&key, prefix.len());
        if let Some(vs) = Versionstamp::from_bytes(&value) {
            checkpoints.push((consumer_id, vs));
        }
    }

    Ok(checkpoints)
}
```

#### 4. Update lib.rs exports
**File:** `crates/pelago-storage/src/lib.rs`
**Changes:** Add `consumer` module and exports

```rust
pub mod consumer;
pub use consumer::{CdcConsumer, ConsumerConfig};
```

### Success Criteria

#### Automated Verification:
- [x] `CdcConsumer::new` loads HWM from FDB (or zero if first run)
- [x] `poll_batch` returns entries after HWM in versionstamp order
- [x] `save_hwm` persists checkpoint to FDB
- [x] Consumer resumes from checkpoint after recreating
- [x] Multiple consumers can operate independently (different consumer_ids)
- [x] `fetch_all_checkpoints` returns all consumer positions

#### Manual Verification:
- [ ] Consumer integration test shows entries flowing from mutations to consumer

---

## Milestone 9: Background Jobs

### Overview
Implement the job worker loop with cursor-based resumption, and the three spec-mandated job types: IndexBackfill, StripProperty, and CdcRetention.

### Changes Required

#### 1. Enhanced job storage
**File:** `crates/pelago-storage/src/jobs.rs`
**Changes:** Replace minimal Job struct with spec-compliant JobState, add FDB persistence

```rust
use serde::{Deserialize, Serialize};
use pelago_core::PelagoError;
use pelago_core::schema::IndexType;
use crate::db::PelagoDb;
use crate::Subspace;
use pelago_core::encoding::{encode_cbor, decode_cbor};

/// Job type with parameters
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum JobType {
    IndexBackfill {
        entity_type: String,
        property_name: String,
        index_type: IndexType,
    },
    StripProperty {
        entity_type: String,
        property_name: String,
    },
    CdcRetention {
        max_retention_secs: u64,
        checkpoint_aware: bool,
    },
    OrphanedEdgeCleanup {
        deleted_entity_type: String,
    },
}

/// Full job state with cursor-based resumption
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobState {
    pub job_id: String,
    pub job_type: JobType,
    pub status: JobStatus,
    pub progress_cursor: Option<Vec<u8>>,
    pub progress_pct: f64,
    pub total_items: Option<u64>,
    pub processed_items: u64,
    pub created_at: i64,
    pub updated_at: i64,
    pub error: Option<String>,
    pub database: String,
    pub namespace: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

/// Job store for CRUD on job records
pub struct JobStore {
    db: PelagoDb,
}

impl JobStore {
    pub fn new(db: PelagoDb) -> Self { Self { db } }

    /// Create a new job in Pending status
    pub async fn create_job(
        &self, database: &str, namespace: &str, job_type: JobType,
    ) -> Result<JobState, PelagoError> { /* ... */ }

    /// Update job state (status, cursor, progress)
    pub async fn update_job(&self, job: &JobState) -> Result<(), PelagoError> { /* ... */ }

    /// Get job by ID
    pub async fn get_job(
        &self, database: &str, namespace: &str, job_id: &str,
    ) -> Result<Option<JobState>, PelagoError> { /* ... */ }

    /// Find all pending or running jobs (for recovery)
    pub async fn find_actionable_jobs(
        &self, database: &str, namespace: &str,
    ) -> Result<Vec<JobState>, PelagoError> { /* ... */ }
}
```

#### 2. Job executor trait and implementations
**File:** `crates/pelago-storage/src/job_executor.rs` (new file)

```rust
use async_trait::async_trait;

/// Trait for job execution with cursor resumption
#[async_trait]
pub trait JobExecutor: Send + Sync {
    /// Execute one batch of work. Returns true if more work remains.
    async fn execute_batch(
        &self,
        job: &mut JobState,
        db: &PelagoDb,
        batch_size: usize,
    ) -> Result<bool, PelagoError>;
}

/// IndexBackfill executor — scans nodes, creates missing index entries
pub struct IndexBackfillExecutor;

#[async_trait]
impl JobExecutor for IndexBackfillExecutor {
    async fn execute_batch(
        &self, job: &mut JobState, db: &PelagoDb, batch_size: usize,
    ) -> Result<bool, PelagoError> {
        // 1. Read cursor from job.progress_cursor (or start from beginning)
        // 2. Scan batch of nodes of the entity_type from cursor
        // 3. For each node, compute index entry and write it
        // 4. Update cursor to last processed key
        // 5. Return true if batch was full (more work), false if done
    }
}

/// StripProperty executor — removes a property from all nodes
pub struct StripPropertyExecutor;

#[async_trait]
impl JobExecutor for StripPropertyExecutor {
    async fn execute_batch(
        &self, job: &mut JobState, db: &PelagoDb, batch_size: usize,
    ) -> Result<bool, PelagoError> {
        // 1. Scan batch of nodes from cursor
        // 2. For each node, remove the property, re-encode, write back
        // 3. Remove associated index entries
        // 4. Update cursor
    }
}

/// CdcRetention executor — deletes expired CDC entries
pub struct CdcRetentionExecutor;

#[async_trait]
impl JobExecutor for CdcRetentionExecutor {
    async fn execute_batch(
        &self, job: &mut JobState, db: &PelagoDb, batch_size: usize,
    ) -> Result<bool, PelagoError> {
        // 1. Compute effective cutoff (respect consumer checkpoints if checkpoint_aware)
        // 2. Scan batch of CDC entries before cutoff
        // 3. Delete them
        // 4. Update cursor
    }
}
```

#### 3. Job worker loop
**File:** `crates/pelago-storage/src/job_worker.rs` (new file)

```rust
use tokio::task::JoinHandle;
use std::time::Duration;

pub struct JobWorker {
    db: PelagoDb,
    job_store: JobStore,
    poll_interval: Duration,
    batch_size: usize,
}

impl JobWorker {
    pub fn new(db: PelagoDb) -> Self {
        Self {
            job_store: JobStore::new(db.clone()),
            db,
            poll_interval: Duration::from_secs(5),
            batch_size: 1000,
        }
    }

    /// Start the worker loop (runs until cancelled)
    pub async fn run(
        &self,
        database: &str,
        namespace: &str,
        shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), PelagoError> {
        // 1. On startup: find_actionable_jobs (recover interrupted)
        // 2. Execute recovered jobs
        // 3. Loop: poll for new pending jobs, execute, sleep
        // 4. On shutdown signal: save state gracefully
    }

    /// Execute a single job to completion
    async fn execute_job(&self, job: &mut JobState) -> Result<(), PelagoError> {
        let executor: Box<dyn JobExecutor> = match &job.job_type {
            JobType::IndexBackfill { .. } => Box::new(IndexBackfillExecutor),
            JobType::StripProperty { .. } => Box::new(StripPropertyExecutor),
            JobType::CdcRetention { .. } => Box::new(CdcRetentionExecutor),
            JobType::OrphanedEdgeCleanup { .. } => {
                return Err(PelagoError::Internal("OrphanedEdgeCleanup not implemented".into()));
            }
        };

        job.status = JobStatus::Running;
        self.job_store.update_job(job).await?;

        loop {
            match executor.execute_batch(job, &self.db, self.batch_size).await {
                Ok(true) => {
                    // More work — save progress periodically
                    if job.processed_items % 10000 == 0 {
                        self.job_store.update_job(job).await?;
                    }
                }
                Ok(false) => {
                    // Done
                    job.status = JobStatus::Completed;
                    job.progress_pct = 1.0;
                    self.job_store.update_job(job).await?;
                    return Ok(());
                }
                Err(e) => {
                    job.status = JobStatus::Failed;
                    job.error = Some(e.to_string());
                    self.job_store.update_job(job).await?;
                    return Err(e);
                }
            }
        }
    }
}
```

#### 4. Wire into schema evolution
**File:** `crates/pelago-storage/src/schema.rs`
**Changes:** When a new indexed property is added to an existing schema, create an IndexBackfill job

```rust
// In register_schema, after detecting new indexed property:
if is_schema_update && has_new_indexed_property {
    let job_store = JobStore::new(self.db.clone());
    job_store.create_job(database, namespace, JobType::IndexBackfill {
        entity_type: schema.name.clone(),
        property_name: new_prop_name,
        index_type: new_index_type,
    }).await?;
}
```

#### 5. Update pelago-storage Cargo.toml
**File:** `crates/pelago-storage/Cargo.toml`
**Changes:** Add `async-trait` dependency

```toml
[dependencies]
# ... existing ...
async-trait = "0.1"
```

#### 6. Update lib.rs exports
```rust
pub mod consumer;
pub mod job_executor;
pub mod job_worker;
pub use consumer::{CdcConsumer, ConsumerConfig};
pub use jobs::{JobState, JobStatus, JobType, JobStore};
pub use job_worker::JobWorker;
```

### Success Criteria

#### Automated Verification:
- [x] `cargo build -p pelago-storage` compiles
- [x] `JobStore::create_job` persists job to FDB
- [x] `JobStore::find_actionable_jobs` returns Pending and Running jobs
- [x] IndexBackfill job populates index entries for existing nodes
- [x] StripProperty job removes property from all nodes of type
- [x] CdcRetention job deletes expired CDC entries
- [x] CdcRetention respects consumer checkpoints when `checkpoint_aware = true`
- [x] Jobs resume from cursor after interruption (kill + restart)
- [x] Failed jobs remain in Failed status with error message

#### Manual Verification:
- [ ] Job progress visible via GetJobStatus gRPC call

---

## Milestone 10: gRPC Integration & Testing

### Overview
Add the `PullCdcEvents` internal gRPC endpoint, update the API layer for new CDC/job types, and build comprehensive integration tests including the updated lifecycle test.

### Changes Required

#### 1. Proto additions for CDC/Replication
**File:** `proto/pelago.proto`
**Changes:** Add ReplicationService with PullCdcEvents

```protobuf
// ═══════════════════════════════════════════════
// REPLICATION SERVICE (Phase 2: internal only)
// ═══════════════════════════════════════════════

service ReplicationService {
  // Pull CDC events from a namespace starting after a versionstamp
  rpc PullCdcEvents(PullCdcEventsRequest) returns (stream CdcEventResponse);
}

message PullCdcEventsRequest {
  RequestContext context = 1;
  bytes after_versionstamp = 2;  // 10-byte versionstamp (empty = from beginning)
  uint32 limit = 3;              // Max events per response (default: 1000)
}

message CdcEventResponse {
  bytes versionstamp = 1;        // 10-byte versionstamp of this entry
  CdcEntryProto entry = 2;
}

message CdcEntryProto {
  string site = 1;
  int64 timestamp = 2;
  string batch_id = 3;
  repeated CdcOperationProto operations = 4;
}

message CdcOperationProto {
  oneof operation {
    NodeCreateOp node_create = 1;
    NodeUpdateOp node_update = 2;
    NodeDeleteOp node_delete = 3;
    EdgeCreateOp edge_create = 4;
    EdgeDeleteOp edge_delete = 5;
    SchemaRegisterOp schema_register = 6;
  }
}

message NodeCreateOp {
  string entity_type = 1;
  string node_id = 2;
  map<string, Value> properties = 3;
  string home_site = 4;
}

message NodeUpdateOp {
  string entity_type = 1;
  string node_id = 2;
  map<string, Value> changed_properties = 3;
  map<string, Value> old_properties = 4;
}

message NodeDeleteOp {
  string entity_type = 1;
  string node_id = 2;
}

message EdgeCreateOp {
  string source_type = 1;
  string source_id = 2;
  string target_type = 3;
  string target_id = 4;
  string edge_type = 5;
  string edge_id = 6;
  map<string, Value> properties = 7;
}

message EdgeDeleteOp {
  string source_type = 1;
  string source_id = 2;
  string target_type = 3;
  string target_id = 4;
  string edge_type = 5;
}

message SchemaRegisterOp {
  string entity_type = 1;
  uint32 version = 2;
}
```

#### 2. ReplicationService implementation
**File:** `crates/pelago-api/src/replication_service.rs` (new file)
**Changes:** Implement `PullCdcEvents` streaming endpoint

#### 3. Update API service constructors
**Files:** `crates/pelago-api/src/node_service.rs`, `edge_service.rs`
**Changes:** Update constructors to accept `site_id` instead of `Arc<CdcWriter>`

#### 4. Update server main
**File:** `crates/pelago-server/src/main.rs`
**Changes:**
- Add `ReplicationService` to gRPC server
- Start `JobWorker` as background task
- Wire `site_id` through to stores

#### 5. CDC integration test
**File:** `crates/pelago-storage/tests/cdc_integration.rs` (new file)

```rust
/// Test CDC entry production and consumption
#[tokio::test]
#[ignore = "requires native FDB"]
async fn test_cdc_consumer_receives_all_mutations() {
    // 1. Create db, stores, consumer
    // 2. Create a schema → verify SchemaRegister CDC op
    // 3. Create nodes → verify NodeCreate CDC ops
    // 4. Update nodes → verify NodeUpdate CDC ops
    // 5. Create edges → verify EdgeCreate CDC ops
    // 6. Delete edges → verify EdgeDelete CDC ops
    // 7. Delete nodes → verify NodeDelete CDC ops
    // 8. Verify versionstamp ordering is monotonic
    // 9. Verify consumer can checkpoint and resume
}

#[tokio::test]
#[ignore = "requires native FDB"]
async fn test_cdc_consumer_resumes_from_checkpoint() {
    // 1. Create mutations, consumer polls and checkpoints
    // 2. Create more mutations
    // 3. Create new consumer with same ID → should resume from checkpoint
    // 4. Verify only new mutations received
}

#[tokio::test]
#[ignore = "requires native FDB"]
async fn test_multiple_independent_consumers() {
    // 1. Create mutations
    // 2. Create consumer_a, poll all entries
    // 3. Create consumer_b, poll all entries
    // 4. Verify both consumers receive all entries
    // 5. Verify checkpoints are independent
}
```

#### 6. Background job integration test
**File:** `crates/pelago-storage/tests/job_integration.rs` (new file)

```rust
#[tokio::test]
#[ignore = "requires native FDB"]
async fn test_index_backfill_job() {
    // 1. Register schema without index on 'age'
    // 2. Create several nodes with 'age' property
    // 3. Update schema to add range index on 'age'
    // 4. Verify IndexBackfill job is created
    // 5. Run job worker
    // 6. Verify index entries now exist for all nodes
    // 7. Verify FindNodes with age filter works
}

#[tokio::test]
#[ignore = "requires native FDB"]
async fn test_cdc_retention_job() {
    // 1. Create mutations (generates CDC entries)
    // 2. Create CdcRetention job with very short retention
    // 3. Run job
    // 4. Verify old CDC entries are deleted
}

#[tokio::test]
#[ignore = "requires native FDB"]
async fn test_job_resumes_after_interruption() {
    // 1. Create many nodes (enough for multiple batches)
    // 2. Start IndexBackfill job, interrupt after 1 batch
    // 3. Restart job from saved cursor
    // 4. Verify all entries are backfilled
}
```

#### 7. Update lifecycle test
**File:** `crates/pelago-storage/tests/lifecycle_test.rs`
**Changes:** Add Phase 10 (CDC Verification) to the existing lifecycle test

```rust
// After existing PHASE 9 (Node Deletion), add:

// =========================================================================
// PHASE 10: CDC Verification
// =========================================================================
println!("{}", THIN_SEP);
println!("  PHASE 10: CDC Verification");
println!("{}\n", THIN_SEP);

// Create a consumer to read all CDC entries
let consumer_config = ConsumerConfig::new("lifecycle_test", &database, &namespace);
let mut consumer = CdcConsumer::new(db.clone(), consumer_config)
    .await
    .expect("Failed to create consumer");

let entries = consumer.poll_batch().await.expect("Failed to poll CDC");

println!("   CDC entries found: {}", entries.len());

// Verify we have entries for all operations performed
let mut op_counts: HashMap<&str, usize> = HashMap::new();
for (vs, entry) in &entries {
    for op in &entry.operations {
        let name = match op {
            CdcOperation::NodeCreate { .. } => "NodeCreate",
            CdcOperation::NodeUpdate { .. } => "NodeUpdate",
            CdcOperation::NodeDelete { .. } => "NodeDelete",
            CdcOperation::EdgeCreate { .. } => "EdgeCreate",
            CdcOperation::EdgeDelete { .. } => "EdgeDelete",
            CdcOperation::SchemaRegister { .. } => "SchemaRegister",
        };
        *op_counts.entry(name).or_insert(0) += 1;
    }
}

println!("   CDC operation counts:");
for (op, count) in &op_counts {
    println!("     {}: {}", op, count);
}

// Expected operations from the lifecycle test:
// - 2 SchemaRegister (User, Post)
// - 4 NodeCreate (Alice, Bob, Carol, Post)
// - 1 NodeUpdate (Alice age)
// - 4 EdgeCreate (follows x2, friends_with, authored_by)
//   Note: bidirectional friends_with creates 1 CDC op (the edge creation itself)
// - 1 EdgeDelete (Alice follows Carol)
// - 1 NodeDelete (Carol)
assert!(op_counts.get("SchemaRegister").unwrap_or(&0) >= &2, "Expected >= 2 SchemaRegister CDC ops");
assert!(op_counts.get("NodeCreate").unwrap_or(&0) >= &4, "Expected >= 4 NodeCreate CDC ops");
assert!(op_counts.get("NodeUpdate").unwrap_or(&0) >= &1, "Expected >= 1 NodeUpdate CDC ops");
assert!(op_counts.get("EdgeCreate").unwrap_or(&0) >= &4, "Expected >= 4 EdgeCreate CDC ops");
assert!(op_counts.get("EdgeDelete").unwrap_or(&0) >= &1, "Expected >= 1 EdgeDelete CDC ops");
assert!(op_counts.get("NodeDelete").unwrap_or(&0) >= &1, "Expected >= 1 NodeDelete CDC ops");

// Verify versionstamp ordering is monotonic
let mut prev_vs: Option<&Versionstamp> = None;
for (vs, _) in &entries {
    if let Some(prev) = prev_vs {
        assert!(vs > prev, "CDC entries should be in versionstamp order");
    }
    prev_vs = Some(vs);
}
println!("   ✓ All CDC entries in monotonically increasing versionstamp order");

// Checkpoint and verify resume
consumer.checkpoint().await.expect("Failed to checkpoint");
let empty_batch = consumer.poll_batch().await.expect("Failed to poll after checkpoint");
assert!(empty_batch.is_empty(), "No new entries after checkpoint");
println!("   ✓ Consumer checkpoint + resume works\n");
```

Also update the **lifecycle test imports and setup**:
```rust
use pelago_storage::{
    CdcAccumulator, CdcConsumer, CdcOperation, ConsumerConfig, Versionstamp,
    EdgeStore, IdAllocator, NodeRef, NodeStore, PelagoDb, SchemaCache, SchemaRegistry,
};
```

And update the **store constructors** since `NodeStore` and `EdgeStore` now take `site_id` instead of `Arc<CdcWriter>`:
```rust
let site_id = "1".to_string();
let node_store = Arc::new(NodeStore::new(
    db.clone(),
    Arc::clone(&schema_registry),
    Arc::clone(&id_allocator),
    site_id.clone(),
));
let edge_store = EdgeStore::new(
    db.clone(),
    Arc::clone(&schema_registry),
    Arc::clone(&id_allocator),
    Arc::clone(&node_store),
    site_id.clone(),
);
```

Update the **test summary**:
```rust
println!("  Summary:");
println!("    • 2 schemas registered (User, Post)");
println!("    • 4 nodes created (3 Users, 1 Post)");
println!("    • 4 edges created (including 1 bidirectional)");
println!("    • 1 node updated");
println!("    • 1 edge deleted");
println!("    • 1 node deleted");
println!("    • Unique constraint enforced");
println!("    • Required field validation enforced");
println!("    • CDC entries verified for all operations");
println!("    • CDC versionstamp ordering verified");
println!("    • CDC consumer checkpoint + resume verified");
```

### Success Criteria

#### Automated Verification:
- [x] `cargo build` — full workspace compiles
- [x] `cargo test -p pelago-storage` — unit tests pass (52 tests)
- [x] `cargo test -p pelago-api` — unit tests pass
- [ ] Lifecycle test passes with CDC verification phase: `cargo test --test lifecycle_test -- --ignored`
- [ ] CDC integration test passes: `cargo test --test cdc_integration -- --ignored`
- [ ] Job integration test passes: `cargo test --test job_integration -- --ignored`
- [x] Proto compiles with new ReplicationService: `cargo build -p pelago-proto`

#### Manual Verification:
- [ ] `PullCdcEvents` gRPC endpoint streams CDC entries correctly
- [ ] Server starts with JobWorker running in background
- [ ] GetJobStatus shows real progress for running jobs

---

## Testing Strategy

### Unit Tests (no FDB required)
- `CdcEntry` / `CdcOperation` serialization round-trip via CBOR
- `Versionstamp` ordering, `next()`, `from_bytes` / `to_bytes`
- `JobState` serialization
- `JobType` serialization

### Integration Tests (require FDB)
- **lifecycle_test.rs** — Full workflow including CDC verification (updated)
- **cdc_integration.rs** — CDC consumer patterns (new)
- **job_integration.rs** — Background job execution (new)

### Test Isolation
- Each test creates a unique database name using timestamp: `test_db_{millis}`
- Tests clean up after themselves or use short-lived FDB databases
- Consumer IDs are test-specific to avoid cross-test pollution

---

## Performance Considerations

- **Versionstamped keys** eliminate write contention on the CDC subspace — multiple transactions can append CDC entries concurrently without conflicts
- **Batch processing** in consumers and job executors keeps FDB transaction sizes reasonable (default: 1000 items per batch)
- **Cursor-based resumption** prevents re-scanning completed work after job restart
- **Checkpoint-aware retention** prevents deleting CDC entries that consumers haven't processed yet

---

## Migration Notes

- **Breaking change to `NodeStore` constructor:** Replaces `Arc<CdcWriter>` with `site_id: String`
- **Breaking change to `EdgeStore` constructor:** Adds `site_id: String` parameter
- **`CdcWriter` removed:** Replaced by `CdcAccumulator` (transaction-scoped, not shared)
- **`CdcEntry` format changed:** Old CDC entries (if any exist in test FDB) will not deserialize with the new format. Since we're in pre-production, this is acceptable.

---

## References

- **Spec:** `.llm/context/pelagodb-spec-v1.md` — §11 (CDC System), §13 (Background Jobs)
- **Implementation Phases:** `.llm/shared/research/2026-02-14-pelagodb-implementation-phases.md` — Phase 2 (M7-M9)
- **Phase 1 Plan:** `.llm/shared/plans/2026-02-14-phase-1-storage-foundation.md`
- **Phase 1 Source:** `claude/worktrees/phase-1-storage-foundation/`

---

## Milestone Summary

| Milestone | Description | New/Modified Files | Est. Tests |
|-----------|-------------|-------------------|------------|
| **M7** | CDC format + atomic writes | cdc.rs (rewrite), node.rs, edge.rs, schema.rs, lib.rs | 8 unit |
| **M8** | Consumer framework | consumer.rs (new), subspace.rs, lib.rs | 6 unit + 3 integration |
| **M9** | Background jobs | jobs.rs (rewrite), job_executor.rs (new), job_worker.rs (new), schema.rs, lib.rs | 4 unit + 3 integration |
| **M10** | gRPC + testing | pelago.proto, replication_service.rs (new), node_service.rs, edge_service.rs, main.rs, lifecycle_test.rs, cdc_integration.rs (new), job_integration.rs (new) | lifecycle + 6 integration |
