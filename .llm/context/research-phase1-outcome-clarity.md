# Research: Phase 1 Outcome Clarity
## Cross-Reference Analysis of All Context Documents vs Phase 1 Plan

**Status:** Research Complete
**Author:** Andrew + Claude
**Date:** 2026-02-11
**Scope:** Identify discrepancies, ambiguities, and missing specifications in the Phase 1 plan by cross-referencing all llm/context documents.

---

## Method

Four parallel analysis threads were run:

1. **Spec vs Plan** — graph-db-spec-v0.3.md against phase-1-plan.md
2. **Edge Spec vs Plan** — graph-db-edge-spec.md against phase-1-plan.md
3. **Research Docs vs Plan** — all research-*.md against phase-1-plan.md
4. **Internal Clarity** — phase-1-plan.md analyzed for ambiguities an implementer would hit

---

## Critical Issues (Must Resolve Before Implementation)

### C1. Proto Message Definitions Are Stub-Level

**Source:** Spec vs Plan, Internal Clarity

The plan defines gRPC service signatures but no message field definitions. Plan line 756 explicitly marks this as TODO. Without field definitions, proto generation cannot begin, which blocks every crate downstream of `pelago-proto`.

**What's needed:**
- All Request/Response message fields for every RPC
- Pagination fields (`limit`, `cursor`) in FindNodesRequest/Response and ListEdgesRequest/Response
- ErrorDetail proto message for structured error responses
- CDC operation types as a proto enum (even if internal-only for Phase 1)

**Recommendation:** Create a complete `pelago.proto` file as a standalone deliverable before M0 code begins. This becomes the API contract document.

---

### C2. ID Allocation Batch Safety on Crash

**Source:** Spec vs Plan

Plan says: "Batch allocation: grab a block of N IDs (e.g., 100) per transaction to reduce contention." If the server crashes after incrementing the FDB counter but before using all IDs, those IDs are permanently leaked. Not a correctness issue (no duplicates), but a waste.

Worse scenario: if the batch is held in memory and the process crashes after allocating some IDs but before committing all nodes, the atomic counter in FDB already advanced. The next process starts from counter+100. IDs in the gap are never used.

**Options:**
- A) **Single-ID-per-txn** — increment counter by 1, allocate within same FDB txn as node write. Simpler, slightly more contention, zero waste.
- B) **Batch with leak acceptance** — current plan, accept that crashed batches leak IDs. IDs are i64, so even leaking millions is fine for the keyspace. Document the behavior.
- C) **Transactional reservation** — write reservation record, reclaim on startup. Adds complexity.

**Recommendation:** Option B. The i64 space is enormous. Document: "ID gaps may exist after crash recovery. This is by design — IDs are not guaranteed sequential."

---

### C3. Snapshot Reads Not Specified for Query Execution

**Source:** Research Docs vs Plan (from research-undocumented-gaps.md, Gap I4)

FindNodes executes as: scan index → load candidate nodes → apply residual filter. Without pinning FDB read version at query start, a node could be deleted between the index scan and the data load. The query would then see a phantom index entry pointing to a deleted node.

FDB provides `get_read_version()` and `set_read_version()` to pin reads to a consistent snapshot. This is cheap and essential.

**Recommendation:** Add to M4: "All FindNodes/ListEdges queries acquire a read version at the start and use it for all reads within that query. This ensures snapshot consistency within a single query invocation."

---

### C4. Edge Key Layout: Node ID Placement

**Source:** Edge Spec vs Plan

The plan's edge key layout (M3) places `node_id` before the `edges` segment:
```
(ds, ns, entities, "Person", <p_42>, edges, "WORKS_AT", ...)
```

The edge spec's layout uses:
```
(namespace, entities, {entity_type}, edges, {edge_type}, {direction}, {sort_key}, ...)
```

These are actually consistent — both place `node_id` in the key path. The discrepancy is cosmetic: the plan includes node_id as part of the entity prefix, while the spec's Section 6 shows `{node_id}` implicitly between `{entity_type}` and `edges`.

**Resolved:** No conflict. The plan's layout `(ds, ns, entities, type, node_id, edges, edge_type, direction, sort_key, target_type, target_id, edge_id)` is correct and enables prefix scans per vertex per edge type — the vertex-centric index pattern.

---

### C5. Edge Metadata Fields Missing from Plan

**Source:** Edge Spec vs Plan

The edge spec defines `EdgeData` with system metadata:
```rust
struct EdgeData {
    properties: HashMap<String, Value>,
    _owner: String,           // Owning site
    _pair_id: Option<String>, // For bidirectional
    _created_at: i64,         // Creation timestamp
}
```

The plan's M3 shows `pair_id` in the CBOR example but doesn't include `_owner` or `_created_at`. For Phase 1 (single-site), `_owner` is always the local site, but the field should be present in the CBOR schema so Phase 3 multi-site doesn't require a data migration.

**Recommendation:** Add to M3 EdgeCreate: "Edge CBOR includes system fields: `_owner: site_id` (from server config), `_pair_id: Option<edge_id>` (for bidirectional), `_created_at: i64` (unix seconds). These are not returned to clients unless explicitly requested. Present in CDC entries."

---

### C6. CreateIndex RPC Missing from AdminService

**Source:** Research Docs vs Plan (from research-c6-property-removal.md)

The C6 research document recommends explicit index creation via `CreateIndex` RPC, separate from schema registration. The plan's AdminService has `DropIndex` and `StripProperty` but no `CreateIndex`. Plan M1 says "enqueue index backfill job if new indexed property" during schema registration — this contradicts C6's recommendation of decoupled index lifecycle.

**Two design options:**
- A) **Index-in-schema** (current plan): `PropertyDef.index: Option<IndexType>` triggers automatic index creation on schema registration.
- B) **Explicit index RPCs** (C6 recommendation): Schema only defines types. Indexes are created/dropped via separate RPCs.

**Recommendation:** Keep Option A for Phase 1 (simpler to implement, less API surface). But also add `CreateIndex` to AdminService for creating indexes on existing properties after the fact. This gives both the convenience of schema-declared indexes and the flexibility of runtime index management. Document that `DropIndex` + re-`CreateIndex` is the path for index rebuild.

---

### C7. Error Model: No gRPC Status Code Mapping

**Source:** Spec vs Plan, Research Docs vs Plan, Internal Clarity

Plan defines `PelagoError` enum with variants but never maps them to gRPC status codes. The research-undocumented-gaps document recommends structured `ErrorDetail` proto.

**Recommended mapping:**

| PelagoError | gRPC Code | Notes |
|---|---|---|
| SchemaNotFound | NOT_FOUND | |
| SchemaValidation | INVALID_ARGUMENT | Include field-level details |
| NodeNotFound | NOT_FOUND | |
| EdgeNotFound | NOT_FOUND | |
| UniqueConstraintViolation | ALREADY_EXISTS | Include conflicting value |
| TypeMismatch | INVALID_ARGUMENT | Include expected vs actual type |
| CelCompilation | INVALID_ARGUMENT | Include parse error position |
| MissingRequiredField | INVALID_ARGUMENT | Include field name |
| ReferentialIntegrityViolation | FAILED_PRECONDITION | Source/target doesn't exist |
| FdbError | INTERNAL | Don't leak FDB details to client |
| Internal | INTERNAL | |

**Recommendation:** Add this table to M0 as a deliverable. Add `ErrorDetail` proto message: `{ string error_code = 1; string message = 2; map<string, string> metadata = 3; }`. Wire into `tonic::Status::with_details()`.

---

## Important Issues (Should Resolve Before Each Milestone)

### I1. `extras_policy: Warn` Behavior Undefined

**Source:** Internal Clarity (M1)

Three values: Reject, Allow, Warn. Reject and Allow are clear. "Warn" in a gRPC API with no return field for warnings is ambiguous.

**Recommendation:** "Warn = Accept undeclared properties (same as Allow) but emit `tracing::warn!` with field names. No client-visible warning. This is server-side diagnostics only."

---

### I2. `_home_site` Field — Scope in Phase 1

**Source:** Internal Clarity (M2)

Plan shows `_home_site` in node CBOR example but Phase 1 is single-site. Setting this field is unnecessary for Phase 1 but useful to have in the data model for Phase 3.

**Recommendation:** "Set `_home_site: server_config.site_id` on every node at creation time. Store in CBOR. Do not return to client in Phase 1. This prepopulates the field for Phase 3 multi-site without migration."

---

### I3. Forward Reference Resolution

**Source:** Internal Clarity (M1)

Plan says edge targets allow forward references, "stored as unresolved." When are they resolved?

**Recommendation:** "Forward references are never explicitly 'resolved.' Edge target type validation happens at edge creation time (M3), not at schema registration time. If schema A declares edges to type B, and B isn't registered yet, that's fine. When a CreateEdge request arrives targeting type B, the server checks if B's schema exists at that point. If not, reject with `SchemaNotFound`."

---

### I4. Schema Version Increment Enforcement

**Source:** Internal Clarity (M1)

Who enforces version increment? Client-supplied or server auto-increment?

**Recommendation:** "Server auto-increments. Client does not supply version number. RegisterSchemaRequest has no `version` field — server reads current version from FDB, increments, stores new version. RegisterSchemaResponse returns the assigned version number."

---

### I5. Cursor Format for Pagination

**Source:** Internal Clarity (M4), Research Docs vs Plan

Pagination cursor is mentioned but format undefined. Critical for FindNodes and ListEdges.

**Recommendation:** "Cursor is an opaque byte string (base64-encoded in proto `bytes` field). Contents: `[scan_type: u8][last_key: variable]`. For index scans: last_key = full FDB index key. For full scans: last_key = last node_id bytes. Exclusive — next scan starts after the cursor position. Empty cursor = start from beginning."

---

### I6. Null Handling Semantics Need Full Specification

**Source:** Research Docs vs Plan (from research-i1-null-handling.md)

Plan captures the core rule ("comparisons on null return false") but omits:
- `field != null` → `true` if field has a value, `false` if null
- Null in AND/OR → null is falsy
- Null in string ops → return false
- Multiple nulls allowed in unique index

**Recommendation:** Add a "Null Semantics" subsection to M4 referencing the full truth table from research-i1-null-handling.md. Implementation: wrap cel-rust evaluation with null-safe comparison functions.

---

### I7. Sort Key Null/Missing Behavior in Edges

**Source:** Edge Spec vs Plan, Internal Clarity (M3)

If an edge type declares a sort_key property but an edge is created without it, what goes in the key?

**Recommendation:** "If edge type declares sort_key, the property is required on edge creation (enforce in validation). If no sort_key declared, use empty byte string `0x00` as constant — all edges sort by target_id within the direction prefix."

---

### I8. DropNamespace Race Condition

**Source:** Internal Clarity (M5)

`clearRange` on a large namespace may not be atomic. In-flight requests could see partial state.

**Recommendation:** "Phase 1: DropNamespace is best-effort. Document: 'Do not issue DropNamespace while other requests are in-flight to that namespace. Behavior is undefined.' Phase 2: add a 'deleted' flag in namespace metadata, checked at request start, before clearRange begins."

---

### I9. Orphaned Edge Cleanup Deferred

**Source:** Edge Spec vs Plan

When DropEntityType removes all data for a type, incoming edges from OTHER types become orphaned. Plan M5 says "Enqueue orphaned edge cleanup job" but doesn't define the job.

**Recommendation:** "Phase 1: DropEntityType clears the type's data but does NOT clean up incoming edges from other types. Document this limitation. The orphaned edge cleanup job is Phase 2 scope. Listing edges from other types will include stale entries pointing to deleted nodes — these return NOT_FOUND on traversal and can be lazily cleaned."

---

## Minor Issues (Document and Move On)

### M1. Heuristic Selectivity Values

Research-c1-index-intersection.md recommends 10% for equality, 50% for range. Plan says "heuristic-based" but doesn't list values. Add to M4 planner comments: `Unique=0.01, Equality=0.10, Range=0.50, FullScan=1.0`.

### M2. CBOR Canonical Encoding

Plan doesn't specify canonical vs non-canonical CBOR. For property storage, order doesn't matter (HashMap). For CDC entries consumed by other systems, deterministic encoding would be nice but isn't required in Phase 1 (CDC consumers are internal only).

### M3. Composite Indexes

Mentioned in spec and research-c1 but absent from Phase 1 plan. Correctly deferred to Phase 2. Document in plan's "Deferred" section.

### M4. Auth/Authz

Research-undocumented-gaps recommends namespaced API keys. Correctly deferred — Phase 1 is single-site, trusted environment. Document: "Phase 1: no authentication. All requests are trusted."

### M5. CDC Retention

Research recommends 7-day truncation. Phase 1 doesn't implement CDC consumers, so retention is moot. Add to plan: "CDC entries accumulate in Phase 1. Truncation job added in Phase 2 with configurable retention (default 7 days)."

### M6. Performance Baselines

Plan mentions throughput numbers but sets no targets. For Phase 1, measure but don't gate on specific numbers. Document baselines for Phase 2 optimization targets.

### M7. Health Check Endpoint

No health/readiness RPC defined. For Docker Compose / k8s, add a minimal `HealthService.Check` RPC or use gRPC health checking protocol. Low effort, high value for the multi-site demo.

---

## Summary: Action Items by Priority

### Before Any Code (Blocking)

| # | Action | Affects |
|---|---|---|
| C1 | Write complete proto file with all message fields | All milestones |
| C3 | Add snapshot read version pinning to M4 query execution | M4, M5 |
| C7 | Define error code mapping table + ErrorDetail proto | M0, M5 |

### Before Each Milestone

| # | Action | Milestone |
|---|---|---|
| I1 | Define `extras_policy: Warn` = log-only | M1 |
| I3 | Document forward reference resolution = at edge creation | M1, M3 |
| I4 | Server auto-increments schema version | M1 |
| C5 | Add `_owner`, `_created_at` to edge CBOR schema | M3 |
| I7 | Sort key required if declared; else constant 0x00 | M3 |
| I5 | Define cursor format (opaque bytes, base64 in proto) | M4 |
| I6 | Add null semantics truth table reference | M4 |
| C6 | Add CreateIndex to AdminService | M5 |
| I8 | Document DropNamespace race condition limitation | M5 |

### Document-Only (No Code Change Needed)

| # | Action |
|---|---|
| C2 | ID batch allocation: document that gaps are expected after crash |
| I2 | `_home_site` set to server site_id at creation, not exposed in Phase 1 |
| I9 | Orphaned edge cleanup is Phase 2; document the limitation |
| M1-M7 | Minor items: add to relevant plan sections |

---

*Research complete. Ready for plan updates.*
