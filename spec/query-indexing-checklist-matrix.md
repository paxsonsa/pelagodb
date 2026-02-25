# Query Indexing Checklist Matrix

This document defines an implementation path for fast point lookup, fast range/sequential lookup, and scalable complex predicate lookup.

It is designed to generalize across entity types while still covering the `show + scheme + task + attrs.*` use case.

Related implementation spec:
- `spec/context-indexing-proposal-v1.md`

## Scope and Goals

- Keep point reads "stupid fast" (single key or single unique index hit).
- Make common sequential/range scans (for example, "all entities for shot") a direct ordered index scan.
- Support arbitrary predicate combinations through generic inverted indexes and planner-driven set operations.
- Preserve transactional correctness by keeping canonical indexes in FoundationDB.
- Validate decisions with p50/p95/p99 benchmark gates before rollout.

## Core Decisions

- Partition operationally by show namespace.
- Model scheme as entity-type boundary (or scheme-scoped type family), not as a namespace explosion.
- Flatten hot/filterable attrs to top-level fields for predictable indexing.
- Keep generated composite keys for known hot query templates.
- Add generic inverted term index for arbitrary field/value filters.
- Add selectivity statistics and use them for planner ordering.
- For shot-list style `OR`, use bounded parallel fan-out of selective equality queries in query workers until native `OR` path cost is reduced.
- Keep query service stateless so multiple query nodes can scale horizontally behind a load balancer.
- Evaluate bitmap acceleration only after posting-list set ops are proven hot.

## Checklist Matrix

| Area | Why it exists | Canonical storage/index | Query strategy | Status | Exit criteria |
|---|---|---|---|---|---|
| Namespace partitioning (`show`) | Isolate hot workloads and reduce scan fanout | Namespace = show partition | Route requests to one namespace first | TODO | 100% of read/write paths provide show namespace |
| Scheme boundary | Prevent schema drift from breaking query behavior | Scheme-scoped entity type(s) | Validate predicates against scheme schema | TODO | All schemes have explicit schema contracts |
| Point lookup | Fast direct reads and cache reuse | `entity_id` key and unique hash key | Single key get; cache-first for non-strong reads | IN_PROGRESS | p99 point get meets target on representative dataset |
| Common template lookup | Fast known predicate shapes | Synthetic template term postings (`__tpl_show_scheme_shot_sequence`, `__tpl_show_scheme_shot_sequence_task_label`) | Single template-term posting lookup + optional residual predicates | DONE | Top context templates hit p99 target via benchmark profile |
| Sequential/range lookup | Fast "get all by shot/sequence/task" | Ordered range keys (for example `shot`, `shot+sequence`) | Prefix/range scan with keyset pagination | TODO | Range queries avoid full scan and meet p99 target |
| Arbitrary predicates | Handle long-tail query shapes | Inverted term postings `(field,value)->entity_id` | Posting-list intersection (`AND`) / union (`OR`) then fetch | IN_PROGRESS | Arbitrary multi-predicate queries complete within SLA for target cardinalities |
| Planner ordering | Avoid expensive candidate explosions | Term/document frequency stats (`df`, `n_docs`) | Execute smallest posting list first; adaptive fallback | IN_PROGRESS | Planner chooses lowest estimated-cost plan in benchmark traces |
| Shot-list `OR` execution | Keep list-of-shots/task queries fast under bursty load | Term postings + selective equality indexes | Bounded parallel fan-out (`IN`/`OR` decomposition), merge/dedupe/sort in query worker | IN_PROGRESS | Single-node benchmark shows fan-out p99 materially below monolithic OR for same predicate set |
| Native `OR` optimizer | Reduce large union query latency | Posting lists + batched node fetch path | Server-side union with batched fetches and reduced per-node round trips | DONE | `OR` p99 approaches fan-out p99 on representative workloads |
| Residual filtering | Keep correctness for unsupported predicates | Base row fields | Server-side residual filter after candidate generation | DONE | Correctness tests pass across mixed indexed/non-indexed predicates |
| Pagination semantics | Stable large-result handling | Index-key cursors | Keyset pagination (not offset) + deep-page cursor loop guardrail | DONE | Deep-page latency remains bounded and monotonic |
| Stateless query tier scale-out | Support horizontal query-node scaling without sticky state | External request context + cursor-only continuation tokens | Any query node can serve any page/request; cross-endpoint cursor portability check script | DONE | N query nodes behind LB show linear-ish throughput gain with no correctness regressions |
| Cache-path validation | Quantify when cache helps and where it does not | Rocks cache + CDC projector | Benchmark strong/session/eventual separately for point, list, traverse | IN_PROGRESS | Published benchmark set includes cache-hit and cache-bypass paths with explicit findings |
| Optional bitmap accelerator | Reduce CPU for repeated high-cardinality set ops | Derived bitmap cache (RocksDB optional) | Bitmap `AND/OR` for hot terms only | FUTURE | Proven net win over posting-list ops in perf harness |
| Observability | Detect regressions and tuning opportunities | Metrics + sampled plan traces | Track candidates scanned, bytes read, residual rate, p99 | IN_PROGRESS | Dashboards and alerts for latency and scan amplification |

## Proposal Path (Recommended)

1. Establish Baseline and Workload Profiles

- Define representative workloads:
  - point lookup by id/hash
  - range lookup by shot / shot+sequence
  - complex boolean (`AND`/`OR`) with varying selectivity
- Capture baseline p50/p95/p99 and scan amplification.

2. Implement Canonical Index Surfaces in FDB

- Add generated composite keys for known hot templates.
- Add ordered range keys for sequential access patterns.
- Add inverted term postings for arbitrary filters.
- Keep all index mutations in the same transaction as base row writes.

3. Add Planner + Stats Loop

- Track `df(term)` and `n_docs(scope)`.
- Choose primary candidate generator by lowest estimated cost.
- For multi-term predicates, run set operations in selectivity order.
- Apply server-side residual filtering for unsupported predicate pieces.

4. Add Cursor and Streaming Improvements

- Use index-key keyset cursors for range and inverted scans.
- Stream results with bounded buffers and backpressure.

5. Add Stateless Query-Node Scale Path

- Keep cursor tokens self-contained and portable.
- Avoid in-process/session affinity requirements for query continuation.
- Run load tests with multiple query nodes against same storage backend.

6. Evaluate Optional Bitmap Layer

- Only after step 1-4 are in place and measured.
- Enable per-term bitmap materialization for terms that exceed configured heat/cardinality thresholds.
- Keep bitmaps as derived cache, not source of truth.

## Current Single-Node Findings (VFX `show_001`)

- On current hardware, monolithic `OR` over 20 shot-like selective terms measured significantly slower than bounded parallel fan-out.
- Measured sample:
  - single `OR` p99: ~263ms
  - parallel fan-out (20 equality queries) p99:
    - 4 workers: ~73ms
    - 8 workers: ~33ms
    - 16 workers: ~22ms
- Interim recommendation:
  - use bounded fan-out in query workers for list-of-shots/task filters now.
  - prioritize server-side `OR` union + batched fetch optimization next.

## "Client-side filter vs server residual filter" Test Plan

Hypothesis to test:
- Streaming broader candidate sets and filtering on client can be faster only when candidate sets are already very small and network/serialization cost is low.

Benchmark both modes:
- Mode A: server performs residual filtering.
- Mode B: server streams candidates; client filters.

Collect:
- server CPU
- bytes transferred
- candidate rows scanned
- result rows returned
- p50/p95/p99 end-to-end latency

Gate:
- Keep client-side filtering only for explicit narrow cases where it wins on p99 and total cost.
- Default to server-side filtering for correctness, bandwidth, and predictable multi-client behavior.

## Acceptance Gates

- Point lookup p99 meets target on representative dataset.
- Range/sequential query p99 meets target with keyset pagination.
- Complex lookup p99 remains within SLA at target cardinalities.
- No correctness regressions across create/update/delete and index maintenance.
- Plan traces show selectivity-aware ordering and bounded residual work.
