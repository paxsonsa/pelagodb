# PelagoDB Architectural Review - Implementation Plan

**Date:** 2026-02-25
**Status:** NOT STARTED
**Source:** Adversarial architectural review with verified findings

---

## Priority Order of Fixes

### P1 - Critical Bugs

#### 1. Fix cascade delete atomicity (Finding 1.1)
- **File:** `crates/pelago-storage/src/node.rs:340-361` (replica delete), also `delete_node` at ~lines 837-872
- **Problem:** Node delete commits in one FDB transaction, then edge cleanup starts a separate transaction. Crash between them = permanent orphan edges.
- **Fix:** Either (a) delete node + edges in a single transaction for small fan-out, or (b) use the job system to make cascade a background job that completes before clearing the node data key (mark-then-sweep).

#### 2. Add retry to ID allocation (Finding 1.2)
- **File:** `crates/pelago-storage/src/ids.rs:88-128`
- **Problem:** `allocate_batch_from_fdb()` uses `db.create_transaction()` (no retry) instead of `db.transact()` (auto-retry). Concurrent allocations get opaque conflict errors.
- **Fix:** Wrap in `db.transact()` or add explicit retry logic with conflict handling.

### P2 - High Severity

#### 3. Stream ListEdges with keyset cursors (Finding 1.3)
- **File:** `crates/pelago-api/src/edge_service.rs:220-330`
- **Problem:** For bidirectional queries, loads up to ~200K edge records into memory for large offsets to return a page of 10.
- **Fix:** Replace offset cursors with keyset-based cursors using FDB range scans directly.

#### 4. Compile CEL once per hop (Finding 1.4)
- **File:** `crates/pelago-query/src/traversal.rs:747-792`
- **Problem:** `matches_edge_filter()` and `matches_node_filter()` call `Program::compile(filter)` per edge/node in traversal loop.
- **Fix:** Compile once per hop, pass the compiled `Program` as a parameter.

#### 5. Unify CEL pipeline (Finding 2.2 + 3.1)
- **File:** `crates/pelago-query/src/planner.rs:88-106`
- **Problem:** Query planner splits CEL on `&&` via regex. Fails on parenthesized expressions, OR, negation, function calls, strings containing `&&`.
- **Fix:** Walk AST for predicate extraction instead of string parsing. Unify with CelEnvironment.

#### 6. Add range combination (Finding 2.3)
- **File:** `crates/pelago-query/src/planner.rs:133-196`
- **Problem:** `age >= 18 && age <= 65` produces two separate Range predicates. Only one used for index scan.
- **Fix:** Combine bounded ranges on the same field into a single scan.

#### 7. Extract VFX domain logic (Finding 2.1)
- **Files:** `crates/pelago-storage/src/term_index.rs:17-19,388-437` and `crates/pelago-query/src/executor.rs:23-25`
- **Problem:** `append_template_postings()` hardcodes VFX field names in generic storage layer.
- **Fix:** Extract into configurable "computed index" abstraction defined per-schema.

#### 8. Make hardcoded edge limit configurable (Finding 2.4)
- **File:** `crates/pelago-query/src/traversal.rs:661,674,687,698`
- **Problem:** All four edge-listing calls hardcode `1000` as limit. No warning on truncation.
- **Fix:** Add `edge_fetch_limit` to `TraversalConfig`, use it in all call sites.

### P3 - Medium Severity

#### 9. Make guardrails configurable (Finding 2.8)
- **Files:** `crates/pelago-api/src/query_service.rs:42-44`, `admin_service.rs:31-32`
- **Problem:** C4/C5 budget constants are compile-time only.
- **Fix:** Make them server config parameters.

#### 10. Add graceful shutdown (Finding 2.6)
- **File:** `crates/pelago-server/src/main.rs:358-365`
- **Problem:** Server sends shutdown signal but doesn't wait for spawned tasks to finish.
- **Fix:** Use `JoinSet` for background tasks, drain before exit.

#### 11. Complete error metadata (Finding 2.7)
- **File:** `crates/pelago-core/src/errors.rs:244-375`
- **Problem:** 9 variants with useful context fields fall through to `_ => {}` in `metadata()`.
- **Fix:** Add metadata extraction for all 9 missing variants:
  - `CelSyntax` (has `expression`, `message`)
  - `CelType` (has `field`, `message`)
  - `EdgeNotFound` (has `source_node`, `target_node`, `label`)
  - `TargetTypeMismatch` (has `edge_type`, `expected`, `actual`)
  - `UndeclaredEdgeType` (has `entity_type`, `edge_type`)
  - `VersionConflict` (has `entity_type`, `node_id`)
  - `SchemaMismatch` (has `expected`, `actual`)
  - `TraversalTimeout` (has `max_depth`, `reached_depth`)
  - `SchemaValidation` (has `message`)

#### 12. Fix NaN and -0.0 float encoding (Finding 2.5)
- **File:** `crates/pelago-core/src/encoding.rs:38-46`
- **Problem:** -0.0 and 0.0 encode to different byte sequences. NaN has inconsistent encoding.
- **Fix:** Canonicalize -0.0 to 0.0 and NaN to a single canonical NaN before encoding.

### P4 - Low Severity

#### 13. Deduplicate pelago_value_to_cel (Finding 2.9)
- **Files:** `crates/pelago-query/src/cel.rs:117-127` and `crates/pelago-query/src/traversal.rs:796-808`
- **Fix:** Remove duplicate, import from cel.rs.

#### 14. Fix continuation token stub edges (Finding 2.10)
- **File:** `crates/pelago-query/src/traversal.rs:341-358`
- **Problem:** Resuming from continuation token creates stub `StoredEdge` with empty fields.
- **Fix:** Populate edge data from storage when resuming.

### P5 - Inconsistencies

#### 15. Fix inconsistent CEL error semantics (Finding 3.2)
- **Files:** `cel.rs:94-97` vs `traversal.rs:764`
- **Problem:** cel.rs treats non-bool as error; traversal.rs treats as false.
- **Fix:** Unify semantics — decide on one approach and apply consistently.

---

## Feature Gaps (NOT in scope for this pass)
- No composite indexes
- No graph algorithms beyond BFS/DFS
- No batch/bulk operations
- No observability metrics
- No schema evolution tooling
- No full-text search
- No query caching
- No readiness probes
- Plaintext token storage

## Refuted Findings (No action needed)
| Original Claim | Why Refuted |
|---|---|
| update_node TOCTOU causes silent data loss | FDB detects conflict at commit time; write txn fails (not silently) |
| PQL parser unwraps will panic | Pest guarantees non-empty output for matched rules |
| Subspace null-byte key collision | All callers pass constant ASCII strings (latent, not active) |
| Offset pagination fetches everything | Uses `fetch_limit = offset + page_size + 1` (still O(offset) inefficient but bounded) |

## Verification Table

| Finding | File:Line | How to Verify |
|---|---|---|
| Cascade split txns | `node.rs:346,350-359` | `trx.commit()` precedes `EdgeStore::new()` |
| ID alloc no retry | `ids.rs:99,124-126` | `create_transaction()` not `transact()` |
| ListEdges in-memory | `edge_service.rs:220-330` | Full merge+dedup before slice |
| CEL recompilation | `traversal.rs:750,775` | `Program::compile()` inside per-item methods |
| VFX hardcoding | `term_index.rs:388-437` | `append_template_postings()` with literal field names |
| No range combination | `planner.rs:133-196` | `select_best_index()` picks one plan per predicate |
| Edge limit 1000 | `traversal.rs:661,674,687,698` | Literal `1000` in all four call sites |
| Float -0.0 | `encoding.rs:38-46` | Bit analysis: `-0.0` → `0x7FFF...` vs `0.0` → `0x8000...` |
| Missing error metadata | `errors.rs:372` | `_ => {}` drops 9 variants |
| No graceful shutdown | `main.rs:358-365` | `shutdown_tx.send()` without `join_all()` |
