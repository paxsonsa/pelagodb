# Architecture Revision Findings and Improvement Matrix (Verified)

**Date:** 2026-02-25  
**Purpose:** Verify prior findings against current code and revise/refute/append as needed.

---

## Verification Summary

| Metric | Count |
|---|---|
| Original findings reviewed | 15 |
| Verified as still valid | 4 |
| Revised (partially valid / shape changed) | 5 |
| Refuted (no longer accurate) | 6 |
| New findings appended | 4 |

---

## Disposition Matrix (Original Report)

| # | Original Finding (short) | Disposition | Verification Notes |
|---|---|---|---|
| 1 | Incoming edge reads incomplete | **Refuted** | Incoming path now exists and is used: `list_incoming_edges*` in `crates/pelago-storage/src/edge.rs:948`, `:971`; API/traversal call it in `crates/pelago-api/src/edge_service.rs:269` and `crates/pelago-query/src/traversal.rs:667`. |
| 2 | Edge delete fails with sort_key | **Refuted** | Delete now resolves stored edge data first, derives sort key from stored properties, then deletes computed keys in `crates/pelago-storage/src/edge.rs:683`, `:690`, `:695` (same pattern in replica path `:469`, `:476`, `:481`). |
| 3 | Node delete does not cascade edges | **Revised** | Cascade exists, but is post-commit and non-atomic: node delete commit at `crates/pelago-storage/src/node.rs:859`, then edge cascade starts at `:863` and `:871`. |
| 4 | Read-modify-write split across txns | **Revised** | Still present on key paths; severity narrowed to specific race windows: create unique pre-check outside write txn (`crates/pelago-storage/src/node.rs:445`), edge endpoint validation outside write txn (`crates/pelago-storage/src/edge.rs:517`, `:534`). |
| 5 | Query has no pinned snapshot path | **Revised** | `FindNodes`/`Traverse` strict modes now pin read version (`crates/pelago-api/src/query_service.rs:116`, `:355`) and propagate it; gap remains for `ExecutePQL` (new finding). |
| 6 | DropEntityType leaves reverse orphans | **Revised** | Cleanup improved with `clear_reverse_keys_for_source_type` (`crates/pelago-api/src/admin_service.rs:322`, `:588`), but bidirectional reverse-direction forward entries can still orphan (new shape detailed below). |
| 7 | Keyspace abstraction split (`_sys:` raw keys) | **Refuted** | Security/replication/watch-state now use `Subspace::system()` based tuples (`crates/pelago-storage/src/security.rs:313`, `crates/pelago-storage/src/replication.rs:217`, `crates/pelago-storage/src/watch_state.rs:150`). |
| 8 | Schema replication incomplete | **Refuted** | CDC carries full schema payload (`crates/pelago-storage/src/cdc.rs:157`, `proto/pelago.proto:659`), and replicator applies schema register (`crates/pelago-server/src/replicator.rs:571`, `:588`). |
| 9 | Planner/term parser duplicated models | **Revised** | Both paths now share predicate helpers (`crates/pelago-query/src/planner.rs:13`, `crates/pelago-query/src/executor.rs:13`, `:1065`), but still not CEL-AST-first planning. |
| 10 | Traversal continuation uses stub path | **Verified** | Stub-path resume is explicit in code comment (`crates/pelago-query/src/traversal.rs:315`) and continuation integration tests remain ignored placeholders (`crates/pelago-query/tests/traversal_continuation_tests.rs:48`, `:60`). |
| 11 | Oversized mixed-responsibility services | **Verified** | Still large/cohesion-heavy (`watch_service.rs` 1420 LOC, `auth_service.rs` 1195 LOC). |
| 12 | Local vs replica path duplication | **Verified** | Node and edge paths still duplicated (`crates/pelago-storage/src/node.rs:115` vs `:423`; `crates/pelago-storage/src/edge.rs:317` vs `:507`). |
| 13 | Data model drift from spec | **Refuted** | Comparison was against legacy spec. Canonical spec is v1 (`.llm/context/pelagodb-spec-v1.md:15`) and node shape is aligned with current `NodeData`. |
| 14 | Integration validation mostly ignored/placeholders | **Verified** | Broadly true: 41 ignored tests in `crates/`, plus placeholder-only cache/traversal continuation tests (`crates/pelago-storage/tests/cache_integration.rs:9`, `crates/pelago-query/tests/traversal_continuation_tests.rs:48`). |
| 15 | Dead code warning in CEL validator | **Refuted** | Referenced unused loop is no longer present in current `cel.rs`; schema-property loop is used (`crates/pelago-query/src/cel.rs:84`). |

---

## Revised Severity-Ranked Findings (Current)

1. **Critical: unique index constraints are not enforced on node update**

- Evidence: create path explicitly checks unique keys before write (`crates/pelago-storage/src/node.rs:445`), but update path has no equivalent check before writing index additions (`crates/pelago-storage/src/node.rs:646`, `:706`, `:746`).
- Why it matters: updating a unique field to an already-used value can overwrite the unique index pointer, violating logical uniqueness.
- Refactor/remove: perform unique-check + conditional write in one transaction with retry on conflict.

2. **Critical: node delete and edge cascade are non-atomic (dangling-edge race window)**

- Evidence: node delete commits first (`crates/pelago-storage/src/node.rs:859`), then separate edge-cascade workflow runs (`:863`, `:871`). Edge create validates endpoints before its write txn (`crates/pelago-storage/src/edge.rs:517`, `:534`) and writes later (`:599`).
- Why it matters: concurrent edge writes can interleave with delete, leaving post-delete orphan edges.
- Refactor/remove: enforce single mutation workflow (or tombstone/guard key) so node existence and edge cleanup are serialized.

3. **High: DropEntityType can still orphan bidirectional reverse-direction forward keys**

- Evidence: bidirectional write includes `rev_forward_key` (`crates/pelago-storage/src/edge.rs:1229`, `:1240`). Drop cleanup clears forward keys by dropped **source** type (`crates/pelago-api/src/admin_service.rs:212`), plus reverse cleanup (`:322`, `:339`), but does not clear forward entries where dropped type is the **target** in reverse-direction materialization.
- Why it matters: stale edge artifacts remain addressable from surviving types.
- Refactor/remove: add forward cleanup pass keyed by target-entity match (or edge-id index driven cleanup).

4. **High: mutation validation windows still sit outside commit transactions**

- Evidence: unique pre-check outside create txn (`crates/pelago-storage/src/node.rs:445`, write txn starts `:499`); edge endpoint checks outside edge-create txn (`crates/pelago-storage/src/edge.rs:517`, write txn starts `:600`).
- Why it matters: decisions can be stale at commit time, surfacing as conflict/error behavior instead of deterministic domain errors.
- Refactor/remove: move read+validate+write into one retryable transaction boundary for each mutation path.

5. **Medium: `ExecutePQL` bypasses strict snapshot/read-version plumbing used by FindNodes/Traverse**

- Evidence: strict snapshot capture exists in `find_nodes`/`traverse` (`crates/pelago-api/src/query_service.rs:116`, `:355`) and is propagated via read options. `execute_pql` uses default execution paths (`crates/pelago-api/src/query_service.rs:654`, `:709`), while default executor path uses `ReadExecutionOptions::default()` (`crates/pelago-query/src/executor.rs:109`).
- Why it matters: multi-block PQL execution can observe non-pinned snapshots.
- Refactor/remove: add snapshot mode/read-version wiring to `ExecutePQL` and pass read options through all block executions.

6. **Medium: traversal continuation resume still reconstructs path stubs**

- Evidence: explicit stub-path comment in resume logic (`crates/pelago-query/src/traversal.rs:315`); integration tests for continuation remain ignored placeholders (`crates/pelago-query/tests/traversal_continuation_tests.rs:48`, `:60`).
- Why it matters: resumed path fidelity can diverge from non-resumed traversal semantics.
- Refactor/remove: persist enough continuation context to reconstruct exact path segments deterministically.

7. **Medium: delete-scope estimator undercounts bidirectional edge key mutations**

- Evidence: estimator assumes `edge_count * 3` keys (`crates/pelago-storage/src/node.rs:939`), while delete path may clear five keys for bidirectional edges (`crates/pelago-storage/src/edge.rs:287`, `:289`; key count semantics at `:1298`).
- Why it matters: inline budget checks can greenlight operations that exceed expected transaction size.
- Refactor/remove: make estimate topology-aware (3 vs 5 keys) using schema edge direction.

8. **Medium: integration validation remains heavily opt-in, with placeholder suites**

- Evidence: 41 `#[ignore]` tests across crates; placeholder cache and continuation suites (`crates/pelago-storage/tests/cache_integration.rs:9`, `crates/pelago-query/tests/traversal_continuation_tests.rs:48`).
- Why it matters: high-risk paths (replication/cache/continuation) are weakly exercised in default workflows.
- Refactor/remove: add CI profile for ignored suites in provisioned environments and replace placeholders with executable scenarios.

9. **Medium: large service files and duplicated local/replica mutation paths raise change risk**

- Evidence: `watch_service.rs` and `auth_service.rs` remain large; local/replica implementations in node/edge stores repeat substantial logic.
- Why it matters: correctness fixes require synchronized edits across mirrored paths.
- Refactor/remove: extract shared mutation primitives with policy/CDC strategy flags; split service modules by concern.

---

## Refuted Findings (No Longer Applicable)

1. Incoming edge listing/traversal missing reverse reads.
2. Sort-key edges undeletable due key reconstruction with `sort_key=None`.
3. System metadata keyspace still using raw `_sys:` string keys in storage helpers.
4. Schema replication missing schema payload/apply handling.
5. Data-model drift claim tied to legacy v0.4 spec instead of canonical v1.
6. CEL validator dead-loop warning item.

---

## Checklist Matrix (Prioritized)

| ID | Phase | Action | Priority | Impact | Effort | Detailed Steps |
|---|---|---|---|---|---|---|
| C1 | Correctness | Enforce unique constraints on update in-txn | P0 | Very High | M | Add unique-check function that runs on candidate updated values inside write txn; map conflict to `UniqueConstraintViolation`; add update-specific unique tests. |
| C2 | Correctness | Make node delete + edge cleanup atomic/serialized | P0 | Very High | L | Introduce delete guard/tombstone or unified transactional workflow; block edge create against deleting nodes; add race tests for delete-vs-create edges. |
| C3 | Correctness | Fix DropEntityType bidirectional orphan cleanup | P1 | High | M | Add cleanup of reverse-direction forward keys; validate with mixed-type bidirectional fixtures and keyspace scan assertions. |
| C4 | Consistency | Move validation-critical reads into retryable write txns | P1 | High | M | Consolidate create/update/edge-create read+validate+write units; add retry wrapper with domain-error mapping. |
| C5 | Query Consistency | Add strict snapshot plumbing to ExecutePQL | P1 | High | M | Extend request/options; capture strict read version once; pass through query/traversal block executors; add snapshot metadata to PQL stream output. |
| C6 | Traversal | Improve continuation fidelity | P2 | Medium | M | Persist richer frontier/path state or deterministic recompute inputs; replace ignored continuation tests with end-to-end assertions. |
| C7 | Mutation Budgeting | Correct delete-scope estimation for bidirectional edges | P2 | Medium | S | Use schema edge direction to estimate 3/5 key clears; validate against synthetic high-degree bidirectional datasets. |
| C8 | Testing | Un-gate critical integration paths in CI profile | P2 | Medium | M | Define provisioned FDB/RocksDB CI lane for ignored tests; convert placeholder suites to runnable integration tests. |
| C9 | Maintainability | Reduce service/path duplication | P3 | Medium | L | Split large services by domain modules; extract shared node/edge mutation primitives for local+replica modes. |

---

## Addendum: Revalidation + CQRS Evaluation (2026-02-25)

### A) Evaluation of `.llm/shared/research/2026-02-25-architectural-review-revalidation.md`

| Revalidation Claim Group | Disposition | Evidence-backed Revision |
|---|---|---|
| "3 fully addressed, 2 partial, 10 still valid" | **Revised** | Directionally useful, but overstates closure. `cascade delete` remains non-atomic (`crates/pelago-storage/src/node.rs:859`, `:863`) and guardrails remain compile-time constants (`crates/pelago-api/src/query_service.rs:42`, `crates/pelago-api/src/node_service.rs:26`). |
| ID allocation lacks retry | **Confirmed** | Still true: allocation uses `create_transaction` flow without retry wrapper in allocator path (`crates/pelago-storage/src/ids.rs:98`). |
| CEL recompilation in traversal loops | **Confirmed** | Still true (`crates/pelago-query/src/traversal.rs:750`, `:775`). |
| Hardcoded traversal edge fetch limit | **Confirmed** | Still true at `1000` per hop fetch call (`crates/pelago-query/src/traversal.rs:661`, `:674`, `:687`, `:698`). |
| Range-bound combination not produced by planner | **Confirmed** | Still true (`crates/pelago-query/src/planner.rs:133`). |
| Continuation path stubs remain | **Confirmed** | Still true (`crates/pelago-query/src/traversal.rs:315`). |
| Storage abstraction coupling remains high | **Partially Confirmed** | Core coupling concerns remain, but ratio-style claims vary with file growth and should be treated as trend data, not a hard architecture regression verdict. |

### B) Evaluation of Provided FDB+RocksDB CQRS Improvements

| Proposed Finding | Disposition | Verification Notes |
|---|---|---|
| Make fallback behavior observable (P0) | **Appended (Valid)** | Cache fallback is currently implicit (`None`/empty result) with no reason enum/metrics in cache read path (`crates/pelago-storage/src/rocks_cache/read_path.rs:31`, `:98`) and no fallback reason surfaced by API handlers (`crates/pelago-api/src/node_service.rs:125`, `crates/pelago-api/src/edge_service.rs:239`). |
| Remove per-call Session consistency tax (P0) | **Appended (Valid)** | Session checks fetch FDB read version per call (`crates/pelago-storage/src/rocks_cache/read_path.rs:79`), causing extra round-trips. |
| Add incoming-edge cache path (P0) | **Appended (Valid)** | Rocks cache keying and list path are source-forward only (`crates/pelago-storage/src/rocks_cache/store.rs:47`, `:139`); incoming edges always use FDB list path (`crates/pelago-api/src/edge_service.rs:267`). |
| Tune RocksDB for access pattern (P1) | **Appended (Valid)** | Open options are minimal (`crates/pelago-storage/src/rocks_cache/store.rs:15`) and do not consume `cache_size_mb` tuning intent from config (`crates/pelago-storage/src/rocks_cache/config.rs:6`, `crates/pelago-server/src/main.rs:109`). |
| Increase projector throughput / reduce lag spikes (P1) | **Appended (Valid)** | `project_batch` applies each CDC entry serially with no coalescing/adaptive batching (`crates/pelago-storage/src/rocks_cache/projector.rs:81`). |
| Define freshness budgets per consistency mode (P1) | **Appended (Valid)** | Eventual mode has no bounded staleness policy; only session-vs-read-version check exists (`crates/pelago-storage/src/rocks_cache/read_path.rs:45`, `:79`). |
| Harden rebuild/catch-up behavior (P2) | **Revised (Partially Valid)** | Rebuild function exists (`crates/pelago-storage/src/rocks_cache/projector.rs:102`), but warm-start knobs are not wired (`crates/pelago-storage/src/rocks_cache/config.rs:10`). Needs operational hardening rather than greenfield rebuild logic. |
| Gate release on S1-S6 criteria (P2) | **Revised (Partially Valid)** | Scenario matrix exists in spec (`.llm/shared/research/2026-02-17-centralized-replicator-scaling-spec-internal.md:174`), and perf harness exists (`scripts/perf-benchmark.py`), but CI gate script does not execute it (`scripts/ci-gate.sh`). |

### CQRS Checklist Matrix (Appended)

| ID | Phase | Action | Priority | Impact | Effort | Detailed Steps |
|---|---|---|---|---|---|---|
| Q1 | Observability | Add fallback reason taxonomy + metrics | P0 | Very High | M | Define enum (`cold`, `stale_hwm`, `key_absent`, `strong_forced_fdb`, `decode_error`); return reason from read-path calls; add counters/histograms for node and edge paths. |
| Q2 | Consistency | Remove per-call session read-version fetch | P0 | High | M | Capture request read-version once at API boundary; pass through `CachedReadPath` calls; compare HWM once per request. |
| Q3 | Cache Coverage | Add incoming-edge cache read path | P0 | High | M | Add reverse/incoming edge key prefix + list API in Rocks store; wire into `EdgeService` incoming reads before FDB fallback. |
| Q4 | Storage Tuning | Configure RocksDB for read-heavy CQRS | P1 | High | M | Add block cache, bloom filters, and prefix extractor; use CF split for node/edge/meta; map server config values to DB options. |
| Q5 | Projector Throughput | Adaptive batching + in-batch coalescing | P1 | High | M | Group operations per node/edge within polled batch; reduce repeated read-modify-write work; add lag and apply-rate instrumentation. |
| Q6 | Freshness Policy | Bound eventual consistency staleness | P1 | Medium | S | Add config knobs (`max_cache_lag_ms`, optional `max_hwm_delta`); force fallback when exceeded; expose applied consistency metadata. |
| Q7 | Rebuild Hardening | Operationalize restart/catch-up drills | P2 | Medium | M | Wire `warm_on_start`; add startup state checks and HWM validation; add rebuild drill test script with expected catch-up SLA. |
| Q8 | Release Gating | Enforce S1-S6 in CI/staging | P2 | Medium | M | Add perf job invoking `scripts/perf-benchmark.py --enforce-targets`; include pass/fail thresholds for p95/p99, hit ratio, fallback rate, projector lag. |

### Practical Execution Order (Validated)

1. Q1 + Q2 + Q3
2. Q4 + Q5
3. Q6 + Q7 + Q8
