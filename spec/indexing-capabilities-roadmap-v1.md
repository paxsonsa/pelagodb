# Indexing Capabilities Roadmap v1 (Database Features)

This spec defines reusable indexing/query-planning capabilities for PelagoDB across entity models.

`Context` is the first workload profile and is specified in:
- `spec/context-indexing-proposal-v1.md`

## 1. Why this exists

We need to avoid designing one-off indexes per domain model.

The goal is to add database-level features that can be reused by any entity:
- `Context`
- `ProductVersion`
- `WorkAreaRevision`
- future entity types with similar lookup patterns

## 2. Capability Set (General)

## 2.1 Canonical index primitives

1. Point/identity indexes
- unique key to entity id lookup

2. Ordered range/prefix indexes
- prefix scans with keyset cursor pagination

3. Inverted term postings
- `(entity_type, scope, field, value) -> entity_id` postings for flexible boolean filters

4. Optional projection indexes
- store small read projections to reduce base-row fetch amplification

## 2.2 Planner capabilities

1. Selectivity-aware predicate ordering
- `df(term)` and `n_docs(scope)` stats

2. Boolean set operations
- `AND` (intersection), `OR` (union), mixed groups

3. Residual predicate support
- apply remaining filters server-side after candidate generation

4. Keyset pagination
- stable cursor semantics for deep pages

## 2.3 Write-path capabilities

1. Transactional index maintenance
- base row + index rows + stats in one transaction

2. Update-friendly maintenance
- diff-based index updates (remove changed terms only, add new terms only)

3. Tombstone lifecycle
- policy support for eager vs async index cleanup

## 2.4 Optional acceleration layer

1. Derived bitmap cache (RocksDB optional)
- only for hot, high-cardinality terms where posting-list set ops are CPU-bound
- canonical truth remains FDB postings

## 2.5 Stateless query-tier scaling

1. Stateless query execution contract
- no in-memory session affinity for pagination/continuation
- cursor tokens carry continuation state required by any query node

2. Horizontal query-node operation
- multiple query nodes can serve read/query traffic behind an L4/L7 load balancer
- bounded parallel fan-out at query worker layer for selective multi-value lookups (`IN`/`OR`) where beneficial

3. Scale validation
- load tests must include N-node query tier sweeps and burst scenarios
- capture throughput scaling, p99 latency, and error/retry rates

## 3. Entity Profile Strategy

Not all entities should use the same index profile.

## 3.1 Immutable-identity profile

Best for:
- entities with stable identifying fields after create

Strategy:
- richer read indexes are acceptable
- create path optimization dominates

## 3.2 Mutable-state profile

Best for:
- entities with frequent status/time updates

Strategy:
- minimize mutable secondary indexes
- isolate mutable fields from identity indexes
- prefer narrow update-hot indexes only when query demand proves value

## 3.3 High-churn relation profile

Best for:
- relation/edge-like entities with frequent membership changes

Strategy:
- adjacency and direct-hop keys first
- avoid broad generic term indexing until proven necessary

## 4. Benchmark and Rollout Gates

Each capability rollout should include:
- p50/p95/p99 latency
- candidate scan amplification
- index write amplification
- CPU and bytes transferred
- correctness under create/update/delete/tombstone

Rollout sequence:
1. canonical FDB indexes + planner stats
2. validate against workload targets
3. only then evaluate bitmap acceleration

## 5. v1 Deliverables

1. Context workload implementation (first adopter)
2. Reusable posting-list set-op executor (`AND`/`OR`)
3. Reusable term stats store and planner hook
4. Keyset pagination utilities for index scans
5. Benchmark harness extensions for scan amplification and candidate counts
