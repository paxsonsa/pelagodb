---
date: 2026-02-13T18:00:00-08:00
researcher: Claude
git_commit: HEAD (no implementation yet)
branch: main
repository: pelagodb
topic: "v0.4 Spec Critique — Dual-Perspective Analysis"
tags: [research, critique, spec-validation, v0.4, implementation-readiness]
status: complete
last_updated: 2026-02-13
last_updated_by: Claude
---

# Research: v0.4 Specification Critical Analysis

**Date**: 2026-02-13T18:00:00-08:00
**Researcher**: Claude
**Document Under Review**: `/Users/apaxson/work/projects/pelagodb/llm/context/graph-db-spec-v0.4.md`

---

## Implementation Reality Check

**Critical Finding: Zero implementation exists.**

The v0.4 spec describes a complete graph database system, but the repository contains:
- 0 Rust source files
- 0 Cargo.toml files
- 0 proto files
- 0 test files
- Only specification documents (16 markdown files across `llm/` and `.llm/`)

This is a **specification document for planned work**, not documentation of existing code. The spec status "Implementation Ready" is accurate — it's ready *for* implementation, not describing *completed* implementation.

---

## Optimist Perspective: Strengths of v0.4

### 1. Comprehensive Consolidation

The v0.4 spec successfully consolidates all prior design work:
- **Adopted authoritative keyspace** from `pelago-fdb-keyspace-spec.md`
- **Integrated traversal** (was Phase 2, now Phase 1)
- **Complete proto definitions** (was marked TODO, now 495 lines of proto)
- **Error code registry** with 31 error codes and gRPC mappings
- **9-byte node ID format** with extractable site ID

All 12 gaps identified in `2026-02-13-v1-spec-gap-analysis.md` have been addressed.

### 2. Clear Architectural Boundaries

The crate layout (Section 2.1) provides clean separation:
- `pelago-proto` — Generated code, no logic
- `pelago-core` — Types and encoding, no FDB
- `pelago-storage` — FDB operations, no API
- `pelago-query` — Query planning, no transport
- `pelago-api` — gRPC handlers, thin layer
- `pelago` — Binary entrypoint only

This enables parallel development and clear ownership.

### 3. FDB-Native Design

The spec embraces FDB idioms rather than fighting them:
- **Versionstamps** for CDC ordering (zero-contention writes)
- **Snapshot reads** with pinned read version (Section 8.5)
- **Batch ID allocation** with acceptable leak behavior
- **Key layouts** designed for range scan efficiency
- **Conflict ranges** for optimistic concurrency

### 4. Complete API Contract

Section 10.1 provides 495 lines of proto definitions covering:
- All 5 services (SchemaService, NodeService, EdgeService, QueryService, AdminService)
- All request/response messages
- Common types (RequestContext, NodeRef, EdgeRef, Value)
- Pagination (cursor in every streaming response)
- Error details (ErrorDetail, FieldError)

An implementer can generate tonic code immediately.

### 5. Traversal in Phase 1

Moving traversal from Phase 2 to Phase 1 (Section 9) provides immediate graph database value:
- Multi-hop traversal with streaming
- Per-hop filtering (edge + node)
- Server-enforced limits (prevents runaway queries)
- @cascade and @recurse directives

This makes the Phase 1 deliverable genuinely useful for VFX production pipelines.

### 6. Schema Evolution Path

Section 5.5 provides clear property removal workflow:
1. Server stops validating property on writes
2. CEL queries referencing property fail type-check
3. Old data remains in CBOR (invisible to queries)
4. Client calls `DropIndex` RPC to clean up
5. Client optionally calls `StripProperty` RPC

This allows schema evolution without blocking deploys.

---

## Skeptic Perspective: Risks and Gaps

### Risk 1: Complexity Scope Creep

**Concern:** Phase 1 now includes traversal, which was originally Phase 2.

The traversal engine (Section 9) requires:
- Multi-hop state management
- Cycle detection
- Concurrent edge scans
- Streaming backpressure
- Timeout enforcement

This is significant additional complexity. The spec describes it at a high level but doesn't address:
- Memory management for large traversals
- How cycle detection scales with visited set size
- Backpressure mechanism between hops

**Recommendation:** Consider gating traversal behind a feature flag for M5. Get basic CRUD working first.

### Risk 2: CEL Integration Unknowns

**Concern:** CEL (cel-rust) is critical to the query layer but not well-specified.

The spec assumes cel-rust provides:
- Type environment construction from schema
- AST walking for predicate extraction
- Null-safe comparison semantics

But cel-rust is a young library. The spec doesn't address:
- What happens if cel-rust doesn't support needed operations?
- How are custom functions (e.g., `startsWith` for strings) registered?
- What's the CEL version/dialect?

**Recommendation:** Add an M0 task to validate cel-rust capabilities against spec requirements. Write a proof-of-concept before committing to the design.

### Risk 3: Query Planner Heuristics Undefined

**Concern:** Section 8.4 describes query plan selection as "heuristic-based" but provides no selectivity values.

The research doc `research-c1-index-intersection.md` recommends:
- Unique: 0.01
- Equality: 0.10
- Range: 0.50
- FullScan: 1.0

But v0.4 spec doesn't include these. An implementer doesn't know what "most selective" means.

**Recommendation:** Add selectivity heuristics to Section 8.4.

### Risk 4: No CLI in Phase 1

**Concern:** The spec defines `pelago serve` (Section 14.1) but no CLI commands.

The gap analysis identified this: `2026-02-13-query-api-cli-design-v2.md` defines a full CLI hierarchy but it's not in the v0.4 spec.

For testing and demos, a CLI is essential. Without it, every interaction requires writing gRPC client code.

**Recommendation:** Add minimal CLI commands to M6:
- `pelago schema register <file>`
- `pelago node create <type> <json>`
- `pelago node get <type> <id>`

### Risk 5: Cross-Database Edges Underspecified

**Concern:** The edge key layout includes `tgt_db` and `tgt_ns`:

```
(f, <src_type>, <src_id>, <label>, <sort_key>, <tgt_db>, <tgt_ns>, <tgt_type>, <tgt_id>, <edge_id>)
```

But Section 7 doesn't explain when/how cross-database edges are created or what validation occurs. The EdgeService proto (Section 10.1) doesn't have `target_database` or `target_namespace` fields in CreateEdgeRequest.

**Implication:** The keyspace supports cross-database edges, but the API doesn't. This is either:
- An oversight (API needs update)
- Or intentional (cross-database is Phase 3)

**Recommendation:** Clarify in Section 7 that cross-database edges are Phase 3 scope. Update edge key documentation to note that `tgt_db` and `tgt_ns` are always same as request context in Phase 1.

### Risk 6: Batch Operations Transaction Limits

**Concern:** `BatchCreateNodes` (Section 10.1) creates multiple nodes in a single transaction.

FDB has transaction limits:
- 10MB transaction size
- 10 seconds transaction time
- 5 seconds is recommended max

The spec doesn't address:
- What's the max batch size?
- How does it handle partial failures?
- Should large batches be split across transactions?

**Recommendation:** Add batch size limit (e.g., 100 nodes per batch) to Section 6. Document that batch operations fail atomically — no partial commits.

### Risk 7: No Health Check Endpoint

**Concern:** For Docker/Kubernetes deployments, health checks are essential.

The spec defines 5 services but no `HealthService` or readiness probe.

**Recommendation:** Add gRPC health checking (grpc.health.v1.Health) or a simple `/health` endpoint to M6.

### Risk 8: Schema Cache Invalidation

**Concern:** Section 5.4 says "Schema cache invalidates on version change" but doesn't specify the mechanism.

Options:
- Poll-based (check FDB periodically)
- Watch-based (FDB watch on schema key)
- Request-scoped (check schema on every request)

Each has different consistency/performance tradeoffs.

**Recommendation:** Specify poll-based invalidation for Phase 1 (simple, good enough). Document cache TTL (e.g., 5 seconds).

### Risk 9: Index Backfill Consistency

**Concern:** Section 13 describes IndexBackfill job but doesn't address:
- What happens to new writes during backfill?
- How does the job coordinate with schema cache?
- What's the scan batch size?

If a node is updated during backfill, both the backfill worker and the update path might write the same index entry.

**Recommendation:** Add to Section 13: "IndexBackfill reads from schema version N. Writes during backfill use the live schema cache, which also reflects version N. Both paths write indexes, resulting in duplicate writes (idempotent, acceptable). Backfill uses 1000-row batches."

### Risk 10: Deferred Work Accumulation

**Concern:** The spec defers significant work to Phase 2+:
- Index intersection
- Cost-based planning with statistics
- RocksDB cache layer
- CDC retention/truncation
- Multi-site replication
- Auth

These deferrals are reasonable individually, but collectively they mean Phase 1 is a **minimal viable product**. Production deployments will hit limitations quickly:
- No query optimization → slow queries on large datasets
- No caching → high FDB read load
- No auth → can't expose to untrusted clients

**Recommendation:** Be explicit in documentation: "Phase 1 is for development and single-site testing. Production deployments require Phase 2+ features."

---

## Gaps Remaining in v0.4

| # | Gap | Severity | Notes |
|---|-----|----------|-------|
| 1 | Selectivity heuristics not in spec | Medium | In research doc, not v0.4 |
| 2 | CLI commands not specified | Medium | Design doc has them, not v0.4 |
| 3 | Cross-db edge API unclear | Low | Keyspace supports, API doesn't |
| 4 | Batch size limits unspecified | Medium | FDB has hard limits |
| 5 | Health check missing | Low | Easy to add |
| 6 | Schema cache invalidation method | Medium | Important for correctness |
| 7 | Index backfill coordination | Medium | Consistency concern |
| 8 | PQL not mentioned | Low | Intentionally deferred |

---

## Verdict

**The v0.4 spec is implementation-ready with caveats.**

### Ready:
- Keyspace layout is complete and consistent
- Proto definitions are complete
- Error codes are comprehensive
- Data model is well-defined
- Node/edge operations are clear
- Traversal limits are specified

### Needs Clarification Before M4:
- Selectivity heuristics
- Schema cache invalidation method
- cel-rust validation

### Needs Addition Before M6:
- Basic CLI commands
- Health check endpoint
- Batch size limits

### Overall Assessment:

The v0.4 spec represents a well-designed graph database system. The consolidation of prior research documents is thorough. An experienced Rust developer could implement this with minimal ambiguity.

The main risk is **scope** — Phase 1 is ambitious. Traversal in particular adds significant complexity. Consider starting implementation with M0-M3 and validating the design before committing to M4-M5 traversal work.

---

## Actionable Recommendations

### Before Implementation Starts

1. **Validate cel-rust** — write a proof-of-concept that builds type environment and extracts predicates
2. **Add selectivity heuristics** to Section 8.4
3. **Clarify cross-database edges** as Phase 3 scope

### Before M4 (Query Milestone)

4. **Specify schema cache invalidation** — poll-based, 5-second TTL
5. **Document batch size limits** — 100 nodes per BatchCreateNodes

### Before M6 (gRPC Milestone)

6. **Add minimal CLI** — schema register, node create/get
7. **Add health check** — grpc.health.v1.Health

### Documentation

8. **Add "Phase 1 Limitations" section** — be explicit about what's not supported

---

*Critique complete. The v0.4 spec is a solid foundation for implementation.*
