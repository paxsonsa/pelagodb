---
date: 2026-02-25T18:00:00-08:00
researcher: Claude
git_commit: 3e881d0
branch: codex/pelagodb-spec-v1
repository: pelagodb
topic: "Evaluate alternative storage engines (RocksDB, custom) for horizontal scaling and high-performance reads"
tags: [research, storage-engine, rocksdb, foundationdb, horizontal-scaling, performance, architecture]
status: complete
last_updated: 2026-02-25
last_updated_by: Claude
---

# Research: Alternative Storage Engines for PelagoDB

**Date**: 2026-02-25
**Researcher**: Claude
**Git Commit**: 3e881d0
**Branch**: codex/pelagodb-spec-v1
**Repository**: pelagodb

## Research Question

Evaluate what it would take for PelagoDB to use a different storage engine like RocksDB (as primary, not cache) or a custom storage engine. Key requirements:
- Horizontal scaling (data sharding across nodes)
- High-performance reads (sub-millisecond point lookups)

## Executive Summary

PelagoDB is **deeply coupled to FoundationDB** with no storage abstraction layer. All 15 of 17 production source files in `pelago-storage` interact with FDB directly. There are zero storage traits or backend interfaces. However, the *key encoding* layer is largely FDB-agnostic -- the custom `TupleBuilder` and `Subspace` types use FDB-compatible byte conventions but have no FDB imports. The CDC system is the tightest coupling point, using FDB's `SetVersionstampedKey` atomic operation.

Introducing an alternative storage engine would require:
1. **Extracting a storage trait** (~2-4 weeks) -- Define `StorageBackend` trait abstracting transactions, gets, sets, ranges
2. **Replacing CDC ordering** (~1 week) -- Replace versionstamped keys with an alternative global ordering mechanism
3. **Building/integrating a distributed layer** (~months-years depending on approach) -- The backend itself

The most viable paths, ranked by effort-to-value:
- **TiKV as backend**: Best pragmatic option. Rust-native, ordered KV, distributed ACID, Percolator 2PC. SurrealDB validates this pattern.
- **Embedded RocksDB + Multi-Raft** (NebulaGraph pattern): Maximum performance. Sub-ms local reads. But requires building Raft consensus, shard management, and distributed transaction coordination.
- **Keep FDB, scale the read path**: Lowest effort. The existing CQRS architecture (FDB writes, RocksDB reads) already provides horizontal read scaling.

---

## Part 1: Current FDB Coupling Analysis

### 1.1 Coupling Depth

The storage crate has **zero abstraction traits**. Every module uses the concrete `PelagoDb` struct which wraps `Arc<foundationdb::Database>`.

| Metric | Count |
|--------|-------|
| Total `.rs` files in `pelago-storage/src/` | 17 |
| Files with direct `use foundationdb::` import | 3 (`db.rs`, `cdc.rs`, `term_index.rs`) |
| Files that call `create_transaction()` | 12 |
| Files with any FDB interaction (direct or via PelagoDb) | 15 |
| Files with zero FDB dependency | 2 (`cache.rs`, `rocks_cache/config.rs`) |
| Storage abstraction traits defined | **0** |
| Total `create_transaction()` call sites | **32** in production code |

### 1.2 FDB-Specific Features Used

Only **3 FDB-specific features** are used:

1. **`SetVersionstampedKey`** (`cdc.rs:252`) -- Zero-contention CDC key ordering. FDB atomically assigns a 10-byte globally-ordered versionstamp at commit time. This is the single hardest feature to replace.

2. **`set_read_version` / `get_read_version`** (`db.rs:134-136`, `read_path.rs:80`) -- MVCC version pinning for snapshot-consistent reads and cache freshness checks. Used by the query service for `Strict` consistency and by the `CachedReadPath` for `Session` consistency.

3. **Transaction model** (`db.rs:84-118`) -- Optimistic concurrency with automatic conflict detection. All mutations compose multiple operations (data + indexes + CDC) into a single FDB transaction for atomicity.

### 1.3 What IS KV-Store-Agnostic

The key encoding layer is almost entirely portable:

- **TupleBuilder** (`pelago-core/src/encoding.rs:86-170`) -- Custom implementation that follows FDB tuple conventions but has zero FDB imports. Uses type-byte prefixes (`0x02` for strings, `0x14` for ints) with null-terminated escaping.
- **Subspace** (`pelago-storage/src/subspace.rs`) -- Custom struct for hierarchical key prefix construction. No FDB imports.
- **NodeId/EdgeId** (`pelago-core/src/types.rs`) -- Fixed 9-byte encoding (`[site_u8 | seq_u64_be]`). No FDB dependency.
- **CBOR values** (`encoding.rs:72-84`) -- Standard serialization for property maps.
- **Sort-order-preserving encodings** (`encoding.rs:16-57`) -- Sign-bit-flip for integers, IEEE 754 order-preserving for floats.
- **Range-end via `0xFF` append** -- Standard prefix-scan technique for any ordered KV store.

### 1.4 Transaction Boundary Patterns

Every mutation follows a single pattern: create transaction -> buffer mutations -> flush CDC -> commit.

```
create_transaction() -> trx
  trx.set(data_key, ...)        // entity data
  trx.set(idx_key, ...)         // index entries
  cdc.flush(&trx, ...)          // CDC entry (versionstamped)
  trx.commit()                  // atomic commit
```

This pattern is used in `node.rs` (create/update/delete), `edge.rs` (create/delete), `schema.rs` (register), and `replication.rs` (apply_replica_*). **32 call sites** create transactions, but only **3 functions** accept `&Transaction` as a parameter to participate in a caller's transaction (`write_index_entry`, `CdcAccumulator::flush`, `apply_term_posting_changes`).

Notably, **0 of 32 sites use auto-retry** (`db.transact()` exists but has no callers). All use the manual `create_transaction()` + `commit()` path.

### 1.5 RocksDB Cache Layer (Current State)

RocksDB is already integrated as a **read-through cache**, not a storage backend:

- Gated behind `#[cfg(feature = "cache")]` (default: on)
- `CdcProjector` consumes FDB CDC entries and materializes them into local RocksDB
- `CachedReadPath` routes reads based on consistency level: `Strong` -> FDB, `Session` -> cache if HWM fresh, `Eventual` -> cache-first
- Replicator has **zero RocksDB interaction** -- it writes to FDB, which triggers CDC, which the projector picks up
- RocksDB keyspace: `n:{db}:{ns}:{type}:{id}` for nodes, `e:{db}:{ns}:...` for edges, `_hwm` for high-water mark

---

## Part 2: Storage Engine Alternatives Assessment

### 2.1 Option A: TiKV as Backend

**Architecture**: Multi-Raft regions over RocksDB. Each Region (96MB default) is a Raft group. Placement Driver handles auto-splitting/rebalancing. Percolator-style 2PC for distributed transactions.

| Property | Assessment |
|----------|-----------|
| Ordered KV | Yes (memcomparable encoding) |
| Distributed ACID | Yes (Percolator 2PC across regions) |
| Horizontal scaling | Yes (auto-splitting, rebalancing) |
| Sub-ms point lookups | Unlikely (RPC overhead: network hop per operation) |
| Language | Rust (perfect match for PelagoDB) |
| Maturity | CNCF Graduated. Used by TiDB, SurrealDB |
| Transaction limits | More generous than FDB (no 5-second limit) |

**What PelagoDB would need to change:**
1. Replace `PelagoDb` wrapper from FDB to TiKV client
2. Replace `SetVersionstampedKey` with TiKV's timestamp oracle for CDC ordering
3. Map PelagoDB's transaction pattern to TiKV's `begin_optimistic_txn()` / `commit()`
4. No change to key encoding (TupleBuilder works as-is)
5. No change to CBOR value encoding

**Effort**: ~4-8 weeks for a working prototype. The key encoding, value encoding, and subspace hierarchy transfer directly. The main work is the transaction lifecycle and CDC ordering adaptation.

**Tradeoff**: Gains distributed ACID without building it. Loses sub-ms reads (every operation is an RPC). Gains freedom from FDB's 5-second transaction limit and 10MB transaction size limit.

### 2.2 Option B: Embedded RocksDB + Multi-Raft (NebulaGraph Pattern)

**Architecture**: One RocksDB instance per data shard. Each shard has its own Raft group for replication. A metadata service manages shard assignment and splitting.

This is the architecture used by NebulaGraph, YugabyteDB, and (with Pebble instead of RocksDB) CockroachDB.

| Property | Assessment |
|----------|-----------|
| Ordered KV | Yes (native RocksDB) |
| Distributed ACID | Single-shard: Yes (via Raft). Cross-shard: requires custom 2PC |
| Horizontal scaling | Yes (shard per Raft group) |
| Sub-ms point lookups | Yes (embedded, no RPC for local leader reads) |
| Full tuning control | Yes (compaction, bloom filters, compression, block cache) |
| Transaction limits | None (no 5-second or 10MB limits) |

**What PelagoDB would need to build:**
1. **Multi-Raft consensus layer** -- One Raft group per shard. Libraries exist (`raft-rs` from TiKV) but integration is substantial
2. **Shard management / Placement Driver** -- Auto-splitting, rebalancing, replica placement, metadata service
3. **Distributed transaction protocol** -- Percolator-style 2PC or status-tablet approach for cross-shard atomicity
4. **Timestamp oracle** -- Centralized or HLC-based, for MVCC and CDC ordering
5. **Graph-aware key encoding** -- Prefix bloom filters for vertex-ID-based adjacency list scans
6. **MVCC layer** -- Multiple column families (data, lock, write) following TiKV's pattern

**Effort**: Person-years. This is what TiKV, YugabyteDB, CockroachDB, and NebulaGraph each took large engineering teams to build. The Raft + RocksDB part is tractable (6-12 months for a competent team). The correct distributed transaction protocol is the hardest part.

**Tradeoff**: Maximum performance and control. Sub-ms local reads. No external database dependency. But massive infrastructure investment and correctness risk.

### 2.3 Option C: Keep FDB, Optimize the Read Path (CQRS Scale-Out)

**Architecture**: The existing architecture -- FDB as system of record, RocksDB as CDC-driven read cache, N API nodes per site.

The scaling spec (`2026-02-17-centralized-replicator-scaling-spec-internal.md`) already defines this:
- N API/query nodes with RocksDB cache (read-only)
- 1 replicator node per site
- CDC projector on each API node

| Scenario | API Nodes | Expected Outcome |
|----------|-----------|-----------------|
| S1 Baseline | 1 | Stable p99 |
| S2 Read Scale | 4 | Near-linear read throughput gain |
| S3 Show Isolation | 4, 20 namespaces | No p99 regression on cold shows |

**What PelagoDB would need to do:**
1. Nothing fundamentally new -- the architecture is already designed and partially implemented
2. Run the S1-S6 evaluation matrix to validate read scaling
3. Optimize the RocksDB cache layer (bloom filters, block cache tuning, prefix iterators)
4. Add incoming-edge caching (currently only outgoing edges are cached)

**Effort**: ~2-4 weeks to complete the cache layer and run validation.

**Tradeoff**: Minimal effort. Reads scale horizontally via cache replicas. Writes still go through FDB (single-writer per site). Limited by FDB's transaction constraints for write-heavy workloads.

### 2.4 Other Options Evaluated

| Option | Verdict | Reason |
|--------|---------|--------|
| **ScyllaDB/Cassandra** | Poor fit | No ordered key semantics, no cross-partition transactions |
| **DragonflyDB** | Not suitable | In-memory cache, not a durable storage engine |
| **etcd** | Not suitable | Limited to ~few GB, designed for config/metadata |
| **Custom Pebble (Go)** | Language mismatch | CockroachDB's approach but requires Go, not Rust |

---

## Part 3: Abstraction Layer Design (Prerequisites for Any Switch)

Regardless of which alternative is chosen, PelagoDB needs a storage abstraction layer. Here is the minimal interface:

### 3.1 Proposed `StorageBackend` Trait

```rust
#[async_trait]
pub trait StorageBackend: Send + Sync + 'static {
    type Transaction: StorageTransaction;

    async fn begin_transaction(&self) -> Result<Self::Transaction, PelagoError>;
    async fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, PelagoError>;
    async fn get_read_version(&self) -> Result<u64, PelagoError>;
}

#[async_trait]
pub trait StorageTransaction: Send + 'static {
    async fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, PelagoError>;
    fn set(&self, key: &[u8], value: &[u8]);
    fn clear(&self, key: &[u8]);
    async fn get_range(
        &self,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, PelagoError>;
    fn set_read_version(&self, version: u64);
    async fn commit(self) -> Result<(), PelagoError>;
}

pub trait CdcOrderingProvider: Send + Sync + 'static {
    /// Write a CDC entry with a globally-ordered key.
    /// Replaces FDB's SetVersionstampedKey.
    fn write_ordered_cdc_entry(
        &self,
        txn: &dyn StorageTransaction,
        prefix: &[u8],
        value: &[u8],
    ) -> Result<(), PelagoError>;
}
```

### 3.2 Migration Steps

1. **Extract trait** -- Define `StorageBackend` and `StorageTransaction` traits
2. **Wrap FDB** -- Implement traits for FDB (`FdbBackend`) wrapping current `PelagoDb`
3. **Genericize stores** -- Change `NodeStore`, `EdgeStore`, etc. from holding `PelagoDb` to `Arc<dyn StorageBackend>`
4. **Isolate CDC ordering** -- Extract `SetVersionstampedKey` into `CdcOrderingProvider` trait
5. **Add alternative backend** -- Implement traits for TiKV or custom engine

**Critical difficulty**: The 3 functions that take raw `&foundationdb::Transaction` parameters (`write_index_entry`, `CdcAccumulator::flush`, `apply_term_posting_changes`) need to be refactored to use `&dyn StorageTransaction`.

---

## Part 4: Comparative Matrix

| Requirement | FDB (current) | TiKV | Custom RocksDB+Raft | FDB + CQRS Scale-Out |
|------------|--------------|------|---------------------|---------------------|
| Horizontal write scaling | Yes (FDB shards) | Yes (regions) | Yes (shards) | Limited (FDB only) |
| Horizontal read scaling | Via RocksDB cache | Via replicas | Yes (local reads) | Yes (N cache nodes) |
| Sub-ms point reads | Via cache (Eventual) | No (RPC) | Yes (embedded) | Via cache (Eventual) |
| Distributed ACID | Yes (strict serializable) | Yes (Percolator) | Requires custom 2PC | Yes (FDB) |
| Transaction size limits | 10MB, 5-sec | More generous | None | 10MB, 5-sec |
| Engineering effort | Done | 4-8 weeks | Person-years | 2-4 weeks |
| Operational complexity | FDB cluster | TiKV + PD cluster | Custom Raft cluster | FDB cluster |
| Storage tuning control | Minimal | Via TiKV config | Full | Minimal (FDB) + Full (RocksDB cache) |

---

## Part 5: Recommendations

### Immediate (v1 completion): Option C -- CQRS Scale-Out
- Finish the RocksDB cache layer (add incoming-edge caching, bloom filter tuning)
- Run the S1-S6 evaluation matrix from the centralized replicator spec
- This requires no architectural changes and validates the read-scaling story

### Short-term (if FDB limits become painful): Storage Trait Extraction
- Introduce the `StorageBackend` trait boundary
- Keep FDB as the sole implementation initially
- This is a pure refactoring effort that does not change behavior but enables future backend swaps

### Medium-term (if distributed write scaling is needed): TiKV Backend
- Implement `StorageBackend` for TiKV
- Use TiKV's timestamp oracle for CDC ordering
- Run both backends in parallel (FDB for validation, TiKV for scaling)
- This is the pragmatic path to horizontal scaling without building distributed infrastructure from scratch

### Long-term (if maximum performance matters): Custom RocksDB + Raft
- Only pursue this if TiKV's RPC latency becomes a measured bottleneck
- Use `raft-rs` (TiKV's Raft library) for consensus
- Embed RocksDB per shard for sub-ms local reads
- Build shard management and distributed transactions incrementally

---

## Code References

- `crates/pelago-storage/src/db.rs:58` -- `PelagoDb` struct (FDB wrapper, no trait)
- `crates/pelago-storage/src/db.rs:114-118` -- `create_transaction()` (32 call sites)
- `crates/pelago-storage/src/cdc.rs:235-253` -- `SetVersionstampedKey` atomic operation (hardest to replace)
- `crates/pelago-storage/src/cdc.rs:30-74` -- `Versionstamp` type (10-byte FDB ordering primitive)
- `crates/pelago-core/src/encoding.rs:86-170` -- `TupleBuilder` (FDB-compatible but no FDB imports)
- `crates/pelago-storage/src/subspace.rs:52-158` -- `Subspace` (custom, no FDB dependency)
- `crates/pelago-storage/src/rocks_cache/` -- Current RocksDB cache layer (5 files)
- `crates/pelago-storage/src/rocks_cache/projector.rs:12-100` -- `CdcProjector` (CDC -> RocksDB)
- `crates/pelago-storage/src/rocks_cache/read_path.rs:20-133` -- `CachedReadPath` (consistency routing)
- `crates/pelago-storage/src/node.rs:498-528` -- Node create (data + index + CDC atomic txn pattern)
- `crates/pelago-server/src/replicator.rs` -- Replicator (zero RocksDB interaction)
- `crates/pelago-core/src/types.rs:7-55` -- `NodeId` (9-byte encoding, no FDB dependency)

## Architecture Insights

1. **The key encoding is already portable.** The `TupleBuilder` and `Subspace` types follow FDB tuple-layer conventions but have zero FDB imports. Any ordered KV store that supports byte-ordered keys would work with the existing encoding unchanged.

2. **The CDC ordering is the tightest coupling point.** FDB's `SetVersionstampedKey` provides zero-contention globally-ordered CDC keys. Replacing this requires either a centralized timestamp oracle (TiKV's TSO approach) or hybrid logical clocks (CockroachDB's approach).

3. **The CQRS architecture already provides read scaling.** The existing FDB (write) + RocksDB cache (read) + N API nodes architecture scales reads horizontally without changing the storage engine. The centralized replicator spec validates this with concrete scenarios.

4. **No auto-retry is used.** All 32 transaction sites use manual `create_transaction()` + `commit()`. This simplifies backend replacement since there's no FDB-specific retry semantic to replicate.

5. **The read path already has consistency-level routing.** `CachedReadPath` with `Strong`/`Session`/`Eventual` consistency levels means the system already handles the "serve from cache vs. authoritative store" decision. This pattern transfers directly to any backend.

## Historical Context

- `.llm/context/section-18-rocksdb-cache.md` -- RocksDB cache layer specification with full CQRS architecture
- `.llm/shared/research/2026-02-17-centralized-replicator-scaling-spec-internal.md` -- Scaling spec with read-replica topology and evaluation matrix
- `.llm/context/janusgraph-architecture-deep-dive.md` -- Reference architecture showing how JanusGraph uses pluggable backends (Cassandra, HBase, FDB)
- `.llm/context/pelagodb-spec-v1.md` -- Canonical v1 spec (Sections 3, 11, 19 most relevant)

## Related Research

- `.llm/shared/research/2026-02-14-pelagodb-implementation-phases.md` -- Phase implementation plan
- `.llm/shared/research/2026-02-13-v1-spec-gap-analysis.md` -- Spec gap analysis

## Open Questions

1. **What is the actual read latency budget?** Sub-ms reads are achievable with RocksDB cache (Eventual consistency) today. Is the goal sub-ms for Strong reads too?
2. **What is the write scale requirement?** FDB handles significant write throughput. Is the bottleneck reads, writes, or both?
3. **Is cross-shard transaction support critical?** Graph mutations that span entity types (node + edges + indexes) currently rely on FDB's cross-key transactions. Some backends (NebulaGraph) punt on cross-partition transactions entirely.
4. **What is the operational tolerance?** TiKV adds PD + TiKV cluster complexity. Custom Raft adds even more. FDB is operationally simpler but more constrained.
5. **Should the storage trait be introduced now?** It's a clean refactoring regardless of whether the backend changes. It improves testability (in-memory backend for tests) and future flexibility.
