# PelagoDB v1 Implementation Phases (Reconciled)

**Status:** Reconciled Plan  
**Date:** 2026-02-14  
**Inputs Reconciled:**
- `.llm/shared/research/2026-02-14-pelagodb-implementation-phases.md`
- `.llm/context/pelagodb-spec-v1.md`
- `.llm/context/research-phase1-outcome-clarity.md`
- `.llm/context/research-undocumented-gaps.md`
- `.llm/context/research-c1-index-intersection.md`
- `.llm/context/research-c6-property-removal.md`
- `.llm/context/research-i1-null-handling.md`

---

## 1. Reconciliation Outcome

This plan keeps the existing **6 macro phases** and adds **atomic implementation passes** in each phase so that:

1. Every pass ends with a working deployable state.
2. Each pass validates a concrete technical risk.
3. Every spec section (§1–§20) is explicitly covered.
4. Each pass can be reverted independently using feature flags + API gating.

---

## 2. Key Decisions Locked

1. **Macro phases remain 1–6** (foundation, CDC, query/cache/CLI, watch, replication, security).
2. **Traversal stays in Phase 1** (to deliver graph value early), but is split into separate passes.
3. **Snapshot reads are mandatory** for query/traversal batches (read version pinning).
4. **Null model is strict**: missing == null, null not indexed, defined truth table behavior.
5. **Index lifecycle is hybrid**: schema-declared indexes + explicit `CreateIndex` for post-hoc/index rebuild.
6. **Security remains Phase 6 for delivery order**, but pre-security dev mode is explicit and non-production.

---

## 3. Reconciled Phase Plan

### Phase 1: Storage Foundation
**Spec:** §1–§9, §14, §15, §16 (foundation portions), §10 (minimal connectivity tooling)

#### Pass 1.0 Contract + Skeleton
- Complete `pelago.proto` for Phase 1 services/messages.
- Crate/workspace scaffolding and config/tracing/error plumbing.
- Health endpoint and structured `ErrorDetail`.

**Validates:** API contract completeness, error taxonomy correctness.  
**Atomic rollback:** disable server binary/revert to previous tag; no data shape commitment yet.

#### Pass 1.1 Schema Registry
- Schema storage/version pointers/cache.
- Evolution rules and server-side version assignment.
- Explicit `extras_policy=Warn` semantics (log-only warning).

**Validates:** schema correctness/evolution behavior.  
**Atomic rollback:** disable schema mutation endpoints.

#### Pass 1.2 Node CRUD + Index Writes + CDC Append
- Node create/read/update/delete.
- Index write/delete maintenance.
- ID allocation (batch with gap acceptance documented).
- CDC append in same transaction.

**Validates:** transactional atomicity and index correctness.  
**Atomic rollback:** disable mutation endpoints while preserving read path.

#### Pass 1.3 Edge CRUD + Cascade + Cross-Namespace
- Forward/reverse edge keys, metadata keys, bidirectional pair handling.
- Sort key validation rules.
- Cascade cleanup on node delete.

**Validates:** graph adjacency integrity and edge lifecycle.  
**Atomic rollback:** disable edge mutation endpoints.

#### Pass 1.4 Query Core (CEL + Planner + Snapshot + Pagination)
- CEL parse/type-check/predicate extraction.
- Single-index planner with residual filtering.
- Snapshot read version pinning for all query batches.
- Cursor format and pagination semantics.
- Null semantics truth table behavior.

**Validates:** correctness under concurrent writes, deterministic query behavior.  
**Atomic rollback:** route Find/Traverse to simpler safe mode.

#### Pass 1.5 Phase-1 API Surface
- Wire `SchemaService`, `NodeService`, `EdgeService`, `QueryService`, `AdminService` (core ops).
- `CreateIndex` included in Admin API (for rebuild and post-hoc lifecycle).

**Validates:** end-to-end API usability from clients/tools.  
**Atomic rollback:** endpoint-level feature flags.

#### Pass 1.6 Test Gate
- Rust integration tests (storage + atomicity + indexes).
- Python gRPC contract tests.
- Baseline performance measurements.

**Gate:** all tests green + baseline metrics recorded.

---

### Phase 2: CDC and Background Execution
**Spec:** §11, §13

#### Pass 2.1 CDC Consumer Framework
- Checkpointed consumers with HWM.
- Batch processing and restart-safe replay.

#### Pass 2.2 Job Runtime
- Execute `IndexBackfill`, `StripProperty`, `CdcRetention`.
- Cursor-based resumption and idempotence.

#### Pass 2.3 ReplicationService (internal)
- `PullCdcEvents` available for downstream replication/watch/cache.

**Validates:** event durability, replay correctness, long-running maintenance safety.  
**Atomic rollback:** pause consumers/workers; writes continue.

---

### Phase 3: Query Language, Cache, and CLI
**Spec:** §17, §19, §10

#### Pass 3.1 PQL 2a
- Parser + resolver + single-block compilation.
- REPL basic execution + explain.

#### Pass 3.2 PQL 2b/2c
- Variables, multi-block DAG, advanced directives (`@cascade`, `@facets`, `@recurse`, `@groupby`).

#### Pass 3.3 PQL Upserts
- Query-then-mutate conditional blocks in one FDB transaction.

#### Pass 3.4 RocksDB Cache + Projector
- CDC projector, HWM tracking, strong/session/eventual read paths.

#### Pass 3.5 Traversal Cache
- Edge adjacency caching and continuation support.

#### Pass 3.6 CLI + REPL Parity
- CLI command tree and REPL workflows aligned with API/PQL.

**Validates:** operator usability, high-throughput reads, PQL execution parity.  
**Atomic rollback:** disable `ExecutePql` and cache read path separately.

---

### Phase 4: Reactive Watch System
**Spec:** §18

#### Pass 4.1 Watch API + Point/Namespace Watches
- gRPC service, point watch, namespace watch, limits/TTL.

#### Pass 4.2 Query Watches (CEL first, then PQL)
- Enter/update/exit semantics.
- PQL watches enabled only after Phase 3 parser/compiler is stable.

#### Pass 4.3 Resume/Backpressure
- Position-based resume, heartbeat, queue/backpressure handling.

**Validates:** real-time streaming reliability and bounded resource behavior.  
**Atomic rollback:** disable WatchService while CDC continues.

---

### Phase 5: Multi-Site Replication
**Spec:** §12

#### Pass 5.1 Site Registry + Collision Guard
- Site claim checks and startup safety.

#### Pass 5.2 Ownership Enforcement + Transfer
- Node/edge ownership rules and transfer protocol.

#### Pass 5.3 Replicator + Projection
- Per-remote position tracking, ownership-filtered projection, idempotent apply.

#### Pass 5.4 Conflict/Failure Handling
- Owner-wins path, LWW fallback only for split-brain edge cases.
- Recovery from partitions/outages.

**Validates:** distributed convergence and ownership safety model.  
**Atomic rollback:** stop replicators; each site continues standalone.

---

### Phase 6: Security and Authorization
**Spec:** §20

#### Pass 6.1 Authentication
- API key, mTLS, service token authn paths.
- RequestContext auth population.

#### Pass 6.2 Authorization
- Path-based ACL engine, policy storage, built-in roles.
- Deny-by-default + capability resolution.

#### Pass 6.3 Auth Service + Credential Lifecycle
- Principal/policy/key/token management RPCs.

#### Pass 6.4 Audit Logging
- Security event audit stream + retention.

**Validates:** production access control and compliance trail.  
**Atomic rollback:** `PELAGO_AUTH_ALLOW_ANONYMOUS=true` fallback only for non-prod.

---

## 4. Coverage Matrix (Spec → Phase)

- **§1–§4:** Phase 1 (Passes 1.0–1.3)
- **§5–§9:** Phase 1 (Passes 1.1–1.5)
- **§10 CLI:** Phase 3 (Pass 3.6)
- **§11 CDC:** Phase 1 write path + Phase 2 consumer/runtime
- **§12 Replication:** Phase 5
- **§13 Jobs:** Phase 1 storage + Phase 2 execution
- **§14 Error Model:** Phase 1.0 + 1.5
- **§15 Config:** Phase 1 baseline + later phases extend config
- **§16 Testing:** Phase gates in all phases
- **§17 PQL:** Phase 3 (Passes 3.1–3.3)
- **§18 Watch:** Phase 4
- **§19 RocksDB Cache:** Phase 3 (Passes 3.4–3.5)
- **§20 Security:** Phase 6

---

## 5. Mandatory Phase Gates

### Gate A: Completeness
- Every pass has explicit API/tests/rollback notes.
- No spec section unassigned.

### Gate B: Usability
- End-of-phase usable via gRPC and CLI/REPL (when introduced).

### Gate C: Safety
- Feature flag or endpoint gating for each pass.
- Known failure modes tested before phase close.

### Gate D: Revertability
- Each pass tagged and deployable independently.
- Rollback never requires data rewrite for previous stable pass.

---

## 6. LLM Agent Lanes (Parallel Work)

1. **Contract Lane:** proto, API contracts, error detail standards.
2. **Storage Lane:** keyspace, schema, CRUD, CDC emit path.
3. **Query Lane:** CEL/PQL planner/executor, traversal, explain.
4. **Streaming Lane:** CDC consumers, watch dispatcher, backpressure.
5. **Distributed Lane:** replication, ownership, conflict/recovery.
6. **Security Lane:** authn/authz/audit.
7. **Verification Lane:** integration, chaos, compatibility, perf gates.

Each lane can run in parallel inside a phase, but merge is blocked by the phase gate.

---

## 7. Final Recommendation

Use this reconciled plan as the execution baseline. It preserves the original 6-phase strategy while adding the pass-level atomicity, risk validation, and full-spec traceability needed for AI-first implementation without omissions.

