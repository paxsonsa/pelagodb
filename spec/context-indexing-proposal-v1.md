# Context Indexing Proposal v1 (Implementation Spec)

This is an implementation-facing spec for high-cardinality `Context` lookup performance in PelagoDB.

Scope note:
- This file defines the `Context` workload profile and concrete key/index strategy.
- General reusable database indexing features are defined in `spec/indexing-capabilities-roadmap-v1.md`.

## 1. Confirmed Requirements

### 1.1 Context model (v1 test profile)

`Context` required fields:
- `show` (namespace partition key)
- `scheme`
- `task`
- `label`
- `attrs`
- flattened required attrs for this scheme: `shot`, `sequence`

Notes:
- In practice schemes vary, but v1 test can assume one scheme profile with `shot` + `sequence`.
- `shot` and `sequence` are required and usually provided together.
- Core identity is effectively immutable after create (updates are rare).

### 1.2 Uniqueness

Must be unique on:
- `(show, scheme, shot, sequence, task, label)`

### 1.3 Query patterns (most common)

- `show + scheme + shot`
- `show + scheme + shot + task + label`
- point lookup by id (or equivalent unique key)
- multi-value `OR` patterns:
  - multiple shots in same show/scheme
  - multiple tasks in same shot

### 1.4 Non-functional goals

- Point lookups: "instant" perception.
- `show+shot` list lookup: target sub-50ms end-to-end (goal).
- Reads dominate writes (~100:1).
- Deletions are rare and generally tombstoned.
- Strong consistency preferred; selective eventual/session acceptable where safe.
- Page size often ~500, stable sort required.

### 1.5 Sizing assumptions for v1 benchmark

- target `Context` count per active show: ~250k
- product version creation rate: low/moderate (typical low, burstable; exact ceiling to confirm in benchmark profile)
- subscription relation updates: ~2-3 changes/sec on active show

These numbers are used to size benchmark fixtures and validate read/write amplification behavior.

## 2. Storage and Index Strategy

All canonical indexes are stored in FoundationDB and maintained transactionally with base row writes.

### 2.1 Base row

- `ctx:data:{show_ns}:{context_id} -> ContextPayload`

Payload includes:
- `scheme`, `task`, `label`, `shot`, `sequence`
- `attrs` (raw map payload or encoded blob)
- system fields (`created_at`, `updated_at`, `tombstone`, etc.)

### 2.2 Identity / point lookup indexes

1. Primary id path:
- `ctx:id:{show_ns}:{context_id} -> exists`

2. Composite uniqueness path:
- `ctx:uniq:{show_ns}:{scheme}:{shot}:{sequence}:{task}:{label} -> context_id`

3. Optional deterministic hash path (for cache-friendly point lookup):
- `ctx:keyhash:{show_ns}:{hash(normalized_identity)} -> context_id`

### 2.3 Hot criteria indexes

For common list patterns:

1. `show+scheme+shot`:
- `ctx:by_shot:{show_ns}:{scheme}:{shot}:{context_id} -> small projection`

2. `show+scheme+shot+task+label`:
- `ctx:by_shot_task_label:{show_ns}:{scheme}:{shot}:{task}:{label}:{context_id} -> small projection`

`small projection` can include frequently displayed fields to reduce base-row fetch amplification.

### 2.4 Generic term postings (internal index structure)

This is an internal index structure, not a first-class business entity.

- `ctx:term:{show_ns}:{scheme}:{field}:{value}:{context_id} -> ""`

Examples:
- `field=task, value=fx`
- `field=label, value=default`
- `field=shot, value=shot001`
- `field=sequence, value=seq010`
- optional dynamic attrs for scheme-driven fields as needed

This supports flexible boolean criteria (`AND`/`OR`) without requiring every composite index upfront.

### 2.5 Term statistics for planner ordering

- `ctx:stats:df:{show_ns}:{scheme}:{field}:{value} -> count`
- `ctx:stats:n_docs:{show_ns}:{scheme} -> count`

Use:
- choose smallest `df` term first for `AND`
- bound expensive `OR` unions by expected candidate size

## 3. Query Execution Rules

## 3.1 Point lookup

Priority:
1. by `context_id`
2. by composite uniqueness key
3. by deterministic hash key (if provided)

For non-strong reads:
- cache-first is allowed when read consistency policy allows it.

## 3.2 Fast criteria lookup

If query shape matches hot index exactly:
- use corresponding materialized index scan
- keyset pagination cursor from last key (not offset)

## 3.3 Complex criteria lookup

Planner flow:
1. Build term set from predicates.
2. Fetch `df` stats for each term.
3. Execute posting-list set ops in selectivity order.
4. Apply residual predicates server-side.
5. Fetch final rows and apply stable sorting.

`OR`:
- union postings for OR groups, then intersect with mandatory terms.

## 3.4 Deletion model

For tombstone delete:
- set tombstone on base row.
- remove or mark index entries per policy.
- residual tombstone filter required if lazy cleanup is used.

Given read-heavy goals, prefer eager index cleanup for hot indexes and allow batched cleanup for long-tail term postings if needed.

## 4. Sorting and Pagination

- Default page size support: 500.
- Use stable keyset cursors tied to index key order.
- For common fixed sort orders, maintain dedicated ordered index.
- For ad hoc sorts, allow in-memory sort only after candidate set is bounded.

## 5. Write Path and Consistency

Single transaction for:
- base row
- uniqueness key
- hot criteria keys
- term postings
- term stats increments/decrements

Given immutability bias:
- optimize create path heavily
- support update path but treat it as lower-frequency reindex operation

Important caveat:
- the immutability optimization applies to `Context` identity paths.
- other entities (for example work area revisions and product versions) can have mutable status/time fields and may be update-heavy.
- those entities should use update-friendly index profiles (minimal mutable secondary indexes, diff-based index maintenance, and narrow hot-update projections).

## 6. Benchmark Plan (v1)

## 6.1 Workload groups

1. Point:
- get by `context_id`
- get by `(scheme, shot, sequence, task, label)`

2. Criteria hot:
- list by `show+scheme+shot`
- list by `show+scheme+shot+task+label`

3. Complex:
- mixed `AND` terms
- `OR` over shots/tasks within same show/scheme

## 6.2 Metrics

- p50/p95/p99 latency
- candidates scanned
- rows returned
- bytes transferred
- index keys touched
- residual filter reject rate

## 6.3 Gates

- point lookup perceived instant under normal UI path
- `show+scheme+shot` sub-50ms goal at representative scale
- no unbounded deep-page latency with page size 500

## 7. Relationship to Graph Traversal

Traversal optimization is out of scope for this phase.

Current phase objective:
- make namespace-local criteria and point lookup paths fast.
- accept that cross-namespace traversals are higher-cost and optimize later.

## 8. High-Churn Data Considerations (Subscriptions + Product Versions)

Two high-churn zones were identified:
- subscription relations between work area revisions and product versions
- continuously created product versions

These should influence index policy now, even if traversal/indexed dependency crawling is deferred.

### 8.1 High-churn indexing rules

- Minimize canonical secondary indexes on high-churn fields.
- Prefer append-friendly key patterns for version creation paths.
- Keep write amplification bounded by indexing only fields required for active UI queries.
- Avoid routing high-churn relation predicates through heavyweight generic term postings by default.

### 8.2 Recommended handling (phase-friendly)

1. Product versions:
- optimize for append-heavy writes with ordered keys by parent scope and version identifier/time.
- add only the minimal hot read indexes needed for current UI listing patterns.

2. Subscriptions:
- treat as edge-like adjacency data with direct source/target access keys.
- prioritize fast point/single-hop retrieval over broad multi-hop precomputation in v1.
- benchmark with ~2-3 subscription mutations/sec on active show and high fanout adjacency lists.

3. Caching:
- use CDC-driven cache projection for high-read surfaces.
- accept eventual consistency on derived caches where explicitly allowed by read policy.

### 8.3 Planner and stats impact

- Keep selectivity stats for stable/low-churn criteria first (`shot`, `sequence`, `task`, `label`).
- Introduce high-churn term stats only when query demand justifies the maintenance cost.
- Distinguish "hot read, low churn" indexes from "hot write, append" indexes in benchmark reports.

## 9. Open Decisions (Need Final Input)

1. Stable default sort for `show+scheme+shot` results:
- choose one canonical order (for example `label asc`, or `created_at desc`).

2. Term-posting coverage:
- v1 should include only flattened `shot/sequence/task/label`, or also selected dynamic attrs from `attrs`?

3. Tombstone cleanup policy:
- eager remove from all indexes vs eager hot-index + async long-tail cleanup.

4. Product version write-rate ceiling for benchmark profile:
- confirm target burst assumption (for example, 10/min, 10/sec, or other).
