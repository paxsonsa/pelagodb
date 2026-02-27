---
date: 2026-02-25T14:15:46Z
researcher: Claude
git_commit: 8e67a37
branch: main
repository: pelagodb
topic: "Re-evaluation of architectural review findings against current codebase"
tags: [research, architecture, review, revalidation, findings, bugs, performance]
status: complete
last_updated: 2026-02-25
last_updated_by: Claude
---

# Research: Re-evaluation of Architectural Review Findings

**Date**: 2026-02-25T14:15:46Z
**Researcher**: Claude
**Git Commit**: 8e67a37
**Branch**: main
**Repository**: pelagodb

## Research Question

Re-evaluate the architectural review findings plan (`.llm/shared/plans/2026-02-25-architectural-review-fixes.md`) and the storage engine evaluation (`.llm/shared/research/2026-02-25-storage-engine-evaluation.md`) against the current codebase. Determine which findings are obsolete, partially addressed, or still valid.

## Executive Summary

Of the **15 original findings**, **3 are fully addressed**, **2 are partially addressed**, and **10 remain completely valid**. The most impactful changes were:

1. **Cascade delete** (P1 1.1) — now uses batched edge cleanup with async job fallback for large cascades
2. **CEL predicate parsing** (P2 2.2+3.1) — extracted into a shared `predicate.rs` module, eliminating duplication (though still string-split-based, not AST-based)
3. **C4/C5 guardrails** (P3 2.8) — enforcement logic added, but thresholds remain compile-time constants

The **storage engine evaluation** is largely still accurate — zero storage abstraction traits, zero `db.transact()` callers, and `SetVersionstampedKey` remains the tightest FDB coupling point. The main change is the file count grew from 17→29 files in `pelago-storage/src/` due to watch service and cache module expansion.

---

## Findings Status Matrix

| # | Finding | Priority | Original Status | Current Status | Changed By |
|---|---------|----------|----------------|----------------|------------|
| 1.1 | Cascade delete atomicity | P1 | NOT STARTED | **PARTIALLY FIXED** | `87b2194` |
| 1.2 | ID allocation no retry | P1 | NOT STARTED | **STILL VALID** | — |
| 1.3 | ListEdges in-memory loading | P2 | NOT STARTED | **STILL VALID** | — |
| 1.4 | CEL recompilation per edge/node | P2 | NOT STARTED | **STILL VALID** | — |
| 2.1 | VFX domain logic in storage | P2 | NOT STARTED | **STILL VALID** | — |
| 2.2+3.1 | CEL pipeline regex splitting | P2 | NOT STARTED | **PARTIALLY FIXED** | `efd0be5` |
| 2.3 | No range combination | P2 | NOT STARTED | **STILL VALID** | — |
| 2.4 | Hardcoded edge limit 1000 | P2 | NOT STARTED | **STILL VALID** | — |
| 2.5 | Float encoding (-0.0/NaN) | P3 | NOT STARTED | **STILL VALID** | — |
| 2.6 | No graceful shutdown | P3 | NOT STARTED | **STILL VALID** | — |
| 2.7 | Missing error metadata | P3 | NOT STARTED | **STILL VALID** | — |
| 2.8 | Guardrails not configurable | P3 | NOT STARTED | **PARTIALLY FIXED** | `87b2194` |
| 2.9 | Duplicate pelago_value_to_cel | P4 | NOT STARTED | **RESOLVED DIFFERENTLY** | pre-review |
| 2.10 | Continuation token stub edges | P4 | NOT STARTED | **STILL VALID** | — |
| 3.2 | Inconsistent CEL error semantics | P5 | NOT STARTED | **STILL VALID** | — |

---

## Detailed Findings

### FULLY/PARTIALLY ADDRESSED

#### 1.1 Cascade Delete Atomicity — PARTIALLY FIXED

**What changed:** Commit `87b2194` added:
- `EdgeStore::delete_edges_for_node()` at `edge.rs:143-320` — batches edge cleanup in groups of 256 (`CASCADE_TX_EDGE_LIMIT`)
- `NodeStore::estimate_delete_scope()` at `node.rs:536-607` — probes edge count before execution
- If cascade exceeds `C4_INLINE_CASCADE_MAX_EDGES` (1000) or `C4_TX_MAX_MUTATED_KEYS` (5000), the API layer creates a `DeleteNodeCascade` background job via `JobStore`
- `DeleteNodeCascadeExecutor` at `job_executor.rs:618-651` handles the background path

**What's still open:** The node data delete and edge cascade are still **separate transactions**. The node data commits in Transaction 1 (`node.rs:837-861`), then edge cleanup starts in Transaction 2+ (`node.rs:863-872`). A crash between them still leaves orphan edges. The improvement is that the edge cascade is now bounded and batched, and large cascades go through the job system — but the fundamental two-transaction gap remains.

**Revised severity:** Reduced from P1 to P2. The bounded batching and job fallback significantly reduce the blast radius, but the atomicity gap is still present for inline deletes.

#### 2.2+3.1 CEL Pipeline Predicate Parsing — PARTIALLY FIXED

**What changed:** Commit `efd0be5` created `crates/pelago-query/src/predicate.rs` (198 lines) with:
- `split_boolean_expression()` at `predicate.rs:50-80` — shared splitting logic
- `parse_comparison()` at `predicate.rs:83-109` — shared comparison parsing
- Both `planner.rs` and `executor.rs` now import from `predicate.rs` instead of duplicating logic

**What's still open:**
- Still uses `str::split("&&")` / `str::split("||")` — **not AST-based**
- Parenthesized expressions like `(a == 1 || b == 2) && c == 3` are incorrectly split
- String literals containing `&&` or `||` will cause incorrect splits
- Function calls like `bio.contains('&&')` will misparse
- `parse_comparison` uses `str::find` for operators, which misparses `name == 'a >= b'`
- The planner explicitly bails to full-scan for OR expressions (`groups.len() != 1`)
- `CelEnvironment` unification is incomplete — 3 sites bypass it and call `Program::compile` directly

**Revised severity:** Reduced from P2 to P3. The deduplication and shared module are improvements, but the fundamental string-splitting limitation persists.

#### 2.8 C4/C5 Guardrails — PARTIALLY FIXED

**What changed:** Commit `87b2194` added enforcement logic for C4/C5 guardrails with proper error types:
- `C4_TX_MAX_MUTATED_KEYS = 5_000` and `C4_TX_MAX_WRITE_BYTES = 2MB` in `admin_service.rs:31-32` and `node_service.rs:26-27`
- `C5_STRICT_MAX_SNAPSHOT_MS = 2_000`, `C5_STRICT_MAX_SCAN_KEYS = 50_000`, `C5_STRICT_MAX_RESULT_BYTES = 8MB` in `query_service.rs:42-44`

**What's still open:** All values remain compile-time `const`. They are not exposed in `ServerConfig`, CLI flags, environment variables, or TOML config. Additionally, the C4 constants are duplicated across `admin_service.rs` and `node_service.rs`.

#### 2.9 Duplicate pelago_value_to_cel — RESOLVED DIFFERENTLY

The function was removed from `cel.rs` entirely. It now only exists at `traversal.rs:796-808`. The original fix proposed importing from `cel.rs`; instead the opposite happened. The duplication is eliminated either way.

---

### STILL VALID (Unchanged)

#### 1.2 ID Allocation No Retry — STILL VALID (P1)

`ids.rs:98-126`: `allocate_batch_from_fdb()` still uses `db.create_transaction()` with no retry logic. The `db.transact()` auto-retry wrapper exists at `db.rs:84-93` but has **zero callers** across the entire codebase. The read-modify-write pattern on the counter key is conflict-prone under concurrent multi-site access. The in-process `Mutex` at `ids.rs:62` only serializes within a single process.

#### 1.3 ListEdges In-Memory Loading — STILL VALID (P2)

`edge_service.rs:189-355`: Offset cursors are still used (`decode_offset_cursor`/`encode_offset_cursor` at lines 362-382). For bidirectional queries, both outgoing and incoming edge vectors are loaded into memory, chained, deduplicated via `HashSet<String>`, then sliced with `skip(start).take(page_size)`. No keyset cursor implementation exists.

#### 1.4 CEL Recompilation per Edge/Node — STILL VALID (P2)

`traversal.rs:747-792`: Both `matches_edge_filter()` (line 750) and `matches_node_filter()` (line 775) call `Program::compile(filter)` on every invocation. The comment at line 771-772 explicitly acknowledges: "Simple implementation - compile and evaluate" / "In a full implementation, we'd cache compiled expressions". No caching exists anywhere.

#### 2.1 VFX Domain Logic in Storage — STILL VALID (P2)

`term_index.rs:17-19,388-437`: `TEMPLATE_FIELD_SHOW_SCHEME_SHOT_SEQUENCE` and friends hardcode VFX field names (`show`, `scheme`, `shot`, `sequence`, `task`, `label`). `executor.rs:26-28,1120-1145` duplicates the constants and performs inverse mapping in `rewrite_template_term_groups()`. No computed index abstraction exists.

#### 2.3 No Range Combination — STILL VALID (P2)

`planner.rs:133-176`: Each range operator produces a `Range` with only one bound filled. `age >= 18 && age <= 65` creates two separate `Range` predicates; only one is used for the index scan. The `IndexPredicate::Range` type supports both bounds (`plan.rs:66-69`), and `QueryExplanation` even has a `"BETWEEN"` label for `(Some(_), Some(_))`, but the planner never produces combined ranges.

#### 2.4 Hardcoded Edge Limit 1000 — STILL VALID (P2)

`traversal.rs:661,674,687,698`: All four edge-listing calls in `get_edges_for_hop()` hardcode `1000`. No `edge_fetch_limit` field in `TraversalConfig` (which has only `max_depth`, `max_results`, `timeout`, `buffer_size`). No truncation warning when the limit is hit. Nodes with >1000 edges silently lose traversal coverage.

#### 2.5 Float Encoding (-0.0/NaN) — STILL VALID (P3)

`encoding.rs:38-46`: `encode_float_for_index()` operates on raw `f.to_bits()` with no canonicalization. `-0.0` encodes to `0x7FFF...` while `0.0` encodes to `0x8000...`. Tests at lines 198-218 don't cover `-0.0` or `NaN`.

#### 2.6 No Graceful Shutdown — STILL VALID (P3)

`main.rs:358-365`: `shutdown_tx.send(true)` fires, then `main()` returns `Ok(())` immediately. Six background tasks (job worker, CDC projector, audit retention, watch state retention, replicators, docs server) all use `tokio::spawn` with no saved `JoinHandle`. No `JoinSet` or `TaskTracker` exists. Tokio runtime drop cancels in-flight work.

#### 2.7 Missing Error Metadata — STILL VALID (P3)

`errors.rs:372`: The `_ => {}` catch-all still drops metadata for 9 structured variants: `CelSyntax`, `CelType`, `EdgeNotFound`, `TargetTypeMismatch`, `UndeclaredEdgeType`, `VersionConflict`, `SchemaMismatch`, `TraversalTimeout`, `SchemaValidation`. Note: commit `87b2194` added metadata for 5 *new* error variants (`MutationScopeTooLarge`, `TxConflictRetryExhausted`, `TxWallBudgetExceeded`, `SnapshotBudgetExceeded`, `SnapshotExpired`), but the original 9 remain unaddressed.

#### 2.10 Continuation Token Stub Edges — STILL VALID (P4)

`traversal.rs:341-358`: Resuming from continuation token creates `StoredEdge` with `edge_id: String::new()`, `label: String::new()`, `properties: HashMap::new()`, `created_at: 0`. The `FrontierEntry` struct (lines 106-118) doesn't store edge data, only `path_keys` (entity_type, node_id pairs).

#### 3.2 Inconsistent CEL Error Semantics — STILL VALID (P5)

Five CEL evaluation sites have divergent non-bool handling:

| Site | Non-bool non-null result |
|------|------------------------|
| `cel.rs:94` (CelExpression) | **Err(CelType)** |
| `traversal.rs:764` (edge filter) | false |
| `traversal.rs:789` (node filter) | false |
| `query_service.rs:987` (variable filter) | false |
| `repl/mod.rs:997` (CLI filter) | false |

Additional inconsistency: `CelExpression::evaluate()` injects null bindings for missing schema properties; the other 4 sites do not.

---

## Storage Engine Evaluation Revalidation

| Claim | Original | Current | Verdict |
|-------|----------|---------|---------|
| Zero storage abstraction traits | 0 | 0 | **STILL ACCURATE** |
| `create_transaction()` call sites | 32 | 31 | **MINOR CHANGE** (-1) |
| `db.transact()` callers | 0 | 0 | **STILL ACCURATE** |
| Files interacting with FDB | 15/17 (88%) | 20/29 (69%) | **SIGNIFICANTLY CHANGED** |
| Functions accepting `&Transaction` | 3 public | 3 public | **STILL ACCURATE** |
| `SetVersionstampedKey` coupling | 1 site (cdc.rs:252) | 1 site (cdc.rs:252) | **STILL ACCURATE** |

The file count growth (17→29) is driven by the watch service refactoring into `watch/` submodules and `rocks_cache/` expansion. Many new files (9 of 12) have zero FDB interaction, so the FDB coupling ratio actually *decreased* from 88% to 69%.

---

## Revised Priority Recommendations

### P1 — Critical (1 remaining)
1. **ID allocation retry** (Finding 1.2) — Only remaining P1. Zero retry on concurrent conflict.

### P2 — High (5 remaining)
2. **ListEdges in-memory** (1.3) — O(offset) memory for bidirectional queries
3. **CEL recompilation** (1.4) — `Program::compile()` per edge in traversal loops
4. **Range combination** (2.3) — BETWEEN queries use half-open scans
5. **Hardcoded edge limit** (2.4) — Silent data loss above 1000 edges per hop
6. **VFX hardcoding** (2.1) — Domain-specific logic in generic layers

### P3 — Medium (4 remaining)
7. **Float encoding** (2.5) — -0.0 ≠ 0.0 in index keys
8. **Graceful shutdown** (2.6) — In-flight work cancelled on stop
9. **Error metadata** (2.7) — 9 variants drop structured context
10. **Guardrails configurability** (2.8) — Constants still compile-time

### Downgraded / Resolved
- **Cascade delete** (1.1) — Downgraded from P1→P2; batching + job system mitigates but gap remains
- **CEL predicate parsing** (2.2) — Downgraded from P2→P3; shared module approach but still string-based
- **Duplicate value_to_cel** (2.9) — Resolved (removed from cel.rs)

---

## Code References

- `crates/pelago-storage/src/node.rs:837-872` — Cascade delete (still two-transaction)
- `crates/pelago-storage/src/ids.rs:88-129` — ID allocation (no retry)
- `crates/pelago-api/src/edge_service.rs:189-355` — ListEdges (offset cursors)
- `crates/pelago-query/src/traversal.rs:747-792` — CEL recompilation
- `crates/pelago-query/src/traversal.rs:661,674,687,698` — Hardcoded 1000 limit
- `crates/pelago-query/src/planner.rs:133-196` — No range combination
- `crates/pelago-query/src/predicate.rs:50-80` — Shared predicate splitting (string-based)
- `crates/pelago-storage/src/term_index.rs:17-19,388-437` — VFX hardcoding
- `crates/pelago-core/src/encoding.rs:38-46` — Float encoding
- `crates/pelago-server/src/main.rs:358-365` — No graceful shutdown
- `crates/pelago-core/src/errors.rs:245-375` — Missing metadata (line 372 catch-all)
- `crates/pelago-api/src/query_service.rs:42-44` — C5 guardrail constants
- `crates/pelago-query/src/traversal.rs:341-358` — Continuation token stubs
- `crates/pelago-query/src/cel.rs:91-104` — CEL error semantics (Err path)

## Related Research

- `.llm/shared/plans/2026-02-25-architectural-review-fixes.md` — Original fixes plan (being revalidated)
- `.llm/shared/research/2026-02-25-storage-engine-evaluation.md` — Storage engine evaluation (being revalidated)
- `.llm/shared/plans/2026-02-25-c4-c5-consistency-guardrails.md` — C4/C5 guardrails plan

## Open Questions

1. **Should cascade delete atomicity (1.1) be re-promoted to P1?** The batching helps, but inline deletes under 1000 edges still have the two-transaction gap. Is crash-between-transactions a realistic failure mode in practice?
2. **Is the string-based CEL splitting (2.2) acceptable for v1?** It handles the common `field == value && field2 > value2` case. The failure modes (parenthesized OR, string literals with `&&`) are edge cases. Is AST-based extraction worth the investment now?
3. **Should `db.transact()` be adopted broadly?** Zero callers of the auto-retry wrapper across 31 transaction sites is a systemic concern, not just an ID allocation issue.
