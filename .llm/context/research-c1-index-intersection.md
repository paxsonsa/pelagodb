# Research Document: Query Execution and Index Selection Strategy
## Gap C1 Investigation

**Status:** Research
**Author:** Andrew + Claude
**Date:** 2026-02-10
**Scope:** Complete query execution and index selection strategy for multi-predicate queries with multiple index-backed predicates.

---

## 1. Problem Statement

### The Gap

Section 5 of the spec (CEL to Query Plan Pipeline) states that when multiple predicates are index-backed, the planner should "execute most selective first, verify second against candidates." However, the spec provides no definition of:

- **What "most selective" means** without explicit cardinality statistics
- **How to "verify" efficiently** when verification requires a second index lookup
- **When to abandon index strategies entirely** and do a full scan
- **How to combine three or more index-backed predicates**

This document provides the **complete theoretical design** for a query execution system that handles all these cases. We model the design after two proven systems:

1. **DynamoDB's simplicity:** Query execution forces explicit index selection; the system picks ONE index and applies remaining predicates as residual filters. This is predictable and maintainable.
2. **Postgres's cost model:** The system maintains column-level statistics and uses them to estimate selectivity, then chooses the best execution plan based on cost estimates.

### Concrete Examples

#### Example 1: Age + Department (Both Indexed)

```
Query: "age >= 30 AND department == 'rendering'"
Indexes available:
  - age: range index (Age field)
  - department: equality index (Department field)

Expected decisions:
  - If department is much more selective (200 matches vs 8000): scan department, verify age in-memory
  - If both are similar selectivity (2000 vs 1500): consider merging sorted ID lists
  - If statistics are unavailable: use heuristics or declare a composite index
```

#### Example 2: Three Predicates

```
Query: "age >= 30 AND salary >= 100000 AND department == 'rendering'"
Indexes:
  - age: range
  - salary: range
  - department: equality (most selective, ~200 people in 'rendering')

Decisions:
  - If department is declared composite with age: use composite, residual-filter salary
  - If no composite: use selectivity estimates to pick best single index
  - If all have similar selectivity: consider multi-way intersection
```

#### Example 3: Composite Index

```
Query: "age >= 30 AND department == 'rendering' AND salary >= 100000"
Schema declares composite index: indexes: [{ fields: ["department", "age"] }]

The composite index is efficient for the first two predicates:
  scan (department="rendering", age >= 30)  ← FDB range scan

Salary is still unindexed. Decision: residual-filter all results in-memory.
```

---

## 2. Design Philosophy: DynamoDB + Postgres

### DynamoDB's Approach: Explicit Index Selection

DynamoDB requires developers to specify which index to use (or scan the table). The planner doesn't do index intersection—it picks ONE index and applies filters as residuals. This is simple and predictable.

**Our system's baseline:** Pick the best single index (or composite), then apply remaining predicates as in-memory residual filters. This is always correct and efficient for most queries.

### Postgres's Approach: Cost-Based Optimization with Statistics

Postgres maintains table and column statistics (row counts, distinct values, histograms) and uses them in a cost model to choose between execution plans. The cost model estimates IO operations, memory usage, and CPU cost for each plan, then picks the cheapest.

**Our system's enhancement:** Maintain similar statistics and use them to decide when single-index selection is suboptimal, and when multi-index intersection (merge-join) is worthwhile.

---

## 3. Query Execution System Design

The complete system has four components working together:

### 3.1 Statistics Maintenance (Background)

The system maintains **per-entity-type and per-indexed-field statistics** in FDB metadata, refreshed in the background.

**Statistics collected:**
- **Per-entity-type:** Total node count
- **Per-indexed-field:** Distinct value count, min/max values, value distribution histogram (optional)

**Stored in FDB metadata:**
```
(_meta, stats, entity_type, field) → statistics_blob
  {
    total_nodes: u64,
    index_entries: u64,
    distinct_values: u64,
    histogram: Option<Vec<{bucket, count}>>,
    last_updated: i64,
  }
```

**Maintenance strategy: Eventually-consistent background updates**
- Not maintained synchronously on every write (too expensive)
- A background job periodically samples nodes per entity type
- Computes per-field statistics
- Updates FDB metadata asynchronously
- Query planner reads cached statistics; statistics may lag behind actual data (acceptable, like Postgres ANALYZE)

**Cost:** Negligible. A background job running once per hour is sufficient for most workloads. The async approach means statistics don't block writes.

### 3.2 Selectivity Estimation

The query planner estimates how many results each predicate will return. With statistics, these estimates are accurate. Without statistics, the planner falls back to heuristics.

**With statistics (accurate):**
```rust
fn estimate_selectivity(pred: &Predicate, stats: &IndexStats) -> f64 {
    match &pred.op {
        Op::Eq => {
            // For equality: estimate = total_nodes / distinct_values
            // Example: department == 'rendering', distinct_values = 80
            // Estimate: 16000 / 80 = 200 rows
            stats.total_nodes as f64 / stats.distinct_values as f64
        },
        Op::Range(_) => {
            // For range: without histogram, assume 50% of indexed values match
            // With histogram: bucket-based estimation
            stats.index_entries as f64 * 0.5
        },
        _ => stats.total_nodes as f64,
    }
}
```

**Without statistics (heuristics, acceptable fallback):**
```rust
fn estimate_selectivity_heuristic(pred: &Predicate) -> f64 {
    match (&pred.field_type, &pred.op) {
        (_, Op::Eq) => 0.1,                      // 10% for equalities
        (Type::Int, Op::Range(RangeOp::Gte)) => 0.5,  // 50% for >=
        (Type::Timestamp, Op::Range(RangeOp::Gte)) => 0.3,  // 30% for >= on timestamps
        _ => 1.0,  // unknown: worst case
    }
}
```

### 3.3 Single-Index Selection Strategy

**This is the baseline and default:** Pick the most selective indexed predicate, execute it, then apply all remaining predicates as in-memory filters.

**When to use:** Most queries, especially those with one or more equality predicates. Simple, predictable, efficient.

**Algorithm:**
1. Identify all indexed predicates
2. Estimate selectivity of each
3. Pick the one with lowest estimated cardinality
4. Execute that index scan
5. Apply remaining predicates as residual filters (in-memory or second index lookup)

**Example:**
```
Query: age >= 30 AND department == 'rendering'
Selectivity estimates:
  - age >= 30: 8000 rows (50%)
  - department == 'rendering': 200 rows (1.25%)
Decision: execute department index, residual-filter age in-memory
Cost: 1 index scan + 200 memory filters
```

**Pseudocode:**
```rust
fn plan_single_best_index(predicates: Vec<Predicate>, schema: &Schema) -> ExecutionPlan {
    let indexed_preds: Vec<_> = predicates.iter()
        .filter(|p| schema.has_index(&p.field))
        .collect();
    let unindexed_preds: Vec<_> = predicates.iter()
        .filter(|p| !schema.has_index(&p.field))
        .collect();

    if indexed_preds.is_empty() {
        return ExecutionPlan::FullScan { residual: Some(compile(&predicates)) };
    }

    // Estimate selectivity for each indexed predicate
    let selectivities: Vec<_> = indexed_preds.iter()
        .map(|p| (p, estimate_selectivity(p, stats)))
        .collect();

    // Pick the most selective
    let (best_pred, _) = selectivities
        .iter()
        .min_by_key(|(_, sel)| (*sel * 1000.0) as i64)
        .unwrap();

    // Plan: scan best index, residual-filter everything else
    let primary = plan_index_scan(best_pred);
    let residual_preds: Vec<_> = predicates.iter()
        .filter(|p| p.field != best_pred.field)
        .collect();

    ExecutionPlan {
        primary,
        residual: if !residual_preds.is_empty() {
            Some(compile(&residual_preds))
        } else {
            None
        },
    }
}
```

### 3.4 Multi-Index Intersection (Sorted Merge-Join)

**When single-index selection is suboptimal:** If two (or more) indexed predicates have similar selectivity and both will return large result sets, scanning both indexes and merging the sorted ID lists can be more efficient than single-index + residual.

**Algorithm:** (Sorted Merge-Join)
1. Scan both indexes in parallel, getting sorted ID lists
2. Merge-join the two sorted lists: O(N + M) pointer comparisons, very efficient
3. Intersection produces the combined result set
4. Apply remaining predicates as residual

**When to use:** Both predicates have similar selectivity, or three+ predicates all with comparable selectivity.

**Example:**
```
Query: age >= 30 AND salary >= 100000
Selectivity estimates:
  - age >= 30: 8000 rows
  - salary >= 100000: 2000 rows

Option A (single-index):
  Scan salary (2000), residual age on 2000 results
  Cost: 1 scan + 2000 residual checks

Option B (merge-join):
  Scan age (8000 FDB ops), scan salary (2000 FDB ops), merge (10000 pointer ops)
  Cost: 10 FDB ops + CPU (fast)

Decision: Merge-join is better because both indexes must be hit anyway (age residual filter would
require either loading node data or second index lookup, both expensive). Merge-join avoids this.
```

**Cost model decision:**
```rust
let selectivity_ratio = most_selective / second_most_selective;

if selectivity_ratio >= 5.0 {
    // One predicate is much better: single-index + residual
    plan_single_best_index(predicates)
} else {
    // Selectivities are similar: merge-join
    plan_merge_join(indexed_preds)
}
```

**Pseudocode (merge-join):**
```rust
async fn plan_merge_join(preds: Vec<&Predicate>) -> ExecutionPlan {
    // Scan both indexes in parallel
    let (ids_1, ids_2) = tokio::join!(
        scan_index(&preds[0]),
        scan_index(&preds[1])
    );

    // Merge-join (sorted lists are naturally sorted by FDB)
    let intersection = merge_intersect(&ids_1, &ids_2);

    ExecutionPlan {
        primary: QueryPlan::Intersection { plans: vec![...] },
        residual: compile(&remaining_preds),
    }
}

fn merge_intersect(ids_1: &[NodeId], ids_2: &[NodeId]) -> Vec<NodeId> {
    let mut result = Vec::new();
    let (mut i, mut j) = (0, 0);

    while i < ids_1.len() && j < ids_2.len() {
        match ids_1[i].cmp(&ids_2[j]) {
            Ordering::Equal => {
                result.push(ids_1[i].clone());
                i += 1;
                j += 1;
            }
            Ordering::Less => i += 1,
            Ordering::Greater => j += 1,
        }
    }

    result
}
```

### 3.5 Composite Index Support

Composite indexes are declared by developers and provide the BEST performance for common multi-field queries. They short-circuit the entire planner logic.

**When a composite index is declared:**
```json
{
  "indexes": [
    { "fields": ["department", "age"] },
    { "fields": ["department", "salary"] }
  ]
}
```

**Planner checks:** If predicates match a composite index signature, use it.

**Example:**
```
Query: department == 'rendering' AND age >= 30
Composite index [department, age] exists
Execution: Single FDB range scan (dept, rendering, age >= 30)
Cost: 1 range scan, optimal
```

**Cost of composites:** Write amplification (each mutation maintains multiple indexes), but worth it for hot queries. Developers should declare composites for known access patterns.

### 3.6 Full Scan Fallback

If no indexed predicates exist, or if cost estimate shows full scan is cheaper, scan all nodes and apply all predicates as residual.

```rust
fn plan_full_scan(predicates: Vec<Predicate>) -> ExecutionPlan {
    ExecutionPlan {
        primary: QueryPlan::FullScan { entity_type: ... },
        residual: Some(compile(&predicates)),
    }
}
```

---

## 4. Decision Algorithm

The complete decision algorithm integrates all strategies:

```rust
fn plan_multi_predicate_query(
    predicates: Vec<Predicate>,
    schema: &Schema,
    stats: Option<&Statistics>,
) -> ExecutionPlan {
    // Step 1: Classify by index availability
    let indexed: Vec<_> = predicates.iter()
        .filter(|p| schema.has_index(&p.field))
        .collect();
    let unindexed: Vec<_> = predicates.iter()
        .filter(|p| !schema.has_index(&p.field))
        .collect();

    // Step 2: No indexed predicates → full scan
    if indexed.is_empty() {
        return ExecutionPlan::FullScan {
            residual: compile(&predicates),
        };
    }

    // Step 3: One indexed predicate → use it, residual-filter the rest
    if indexed.len() == 1 {
        return ExecutionPlan::Single {
            primary: plan_index_scan(&indexed[0]),
            residual: compile(&unindexed),
        };
    }

    // Step 4: Multiple indexed predicates
    // 4a: Check for matching composite index
    if let Some(composite) = find_matching_composite(&indexed, schema) {
        return ExecutionPlan::Composite {
            primary: plan_composite_scan(composite),
            residual: compile(&unindexed),
        };
    }

    // 4b: No composite; decide between single-best and merge-join
    let selectivities: Vec<_> = indexed.iter()
        .map(|p| (p, estimate_selectivity(p, stats)))
        .collect();

    let most_selective = selectivities.iter()
        .min_by_key(|(_, sel)| (*sel * 1000.0) as i64)
        .unwrap();

    let second_most_selective = selectivities.iter()
        .filter(|(p, _)| p != &most_selective.0)
        .min_by_key(|(_, sel)| (*sel * 1000.0) as i64)
        .map(|(_, sel)| *sel)
        .unwrap_or(1.0);

    let selectivity_ratio = second_most_selective / most_selective.1;

    if selectivity_ratio >= 5.0 {
        // Most selective is much better → single-best + residual
        let other_indexed = indexed.iter()
            .filter(|p| p != &most_selective.0)
            .collect();

        ExecutionPlan::Single {
            primary: plan_index_scan(&most_selective.0),
            residual: compile(&[other_indexed, unindexed].concat()),
        }
    } else {
        // Similar selectivity → merge-join (at least on top 2)
        ExecutionPlan::Intersection {
            plans: vec![
                plan_index_scan(&selectivities[0].0),
                plan_index_scan(&selectivities[1].0),
            ],
            residual: compile(&remaining_indexed_and_unindexed),
        }
    }
}
```

---

## 5. Statistics Maintenance in Detail

### Background Job

A periodic background job maintains statistics:

```rust
async fn statistics_maintenance_job() {
    loop {
        // Every N minutes (e.g., 60 minutes)
        tokio::time::sleep(Duration::from_secs(3600)).await;

        for entity_type in schema_registry.all_entity_types() {
            compute_and_update_stats(entity_type).await;
        }
    }
}

async fn compute_and_update_stats(entity_type: &str) {
    let total_nodes = count_nodes(entity_type).await;

    for indexed_field in schema.indexed_fields(entity_type) {
        // Sample nodes to compute distinct values and distribution
        let sample = sample_nodes(entity_type, 1000).await;
        let distinct_values = sample.iter()
            .map(|n| n.get(indexed_field))
            .collect::<HashSet<_>>()
            .len();

        // Compute histogram buckets if needed
        let histogram = compute_histogram(&sample, indexed_field);

        // Store in FDB metadata
        let stats = IndexStats {
            total_nodes,
            index_entries: count_index_entries(entity_type, indexed_field).await,
            distinct_values,
            histogram,
            last_updated: now(),
        };

        db.set(
            (namespace, "_meta", "stats", entity_type, indexed_field),
            serde_json::to_vec(&stats)?,
        ).await?;
    }
}
```

**Key properties:**
- **Eventually consistent:** Statistics may lag behind actual data. This is fine. Postgres does the same (ANALYZE runs periodically, not on every write).
- **Async:** Background job runs independently, doesn't block write path
- **Sampling-based:** Don't need exact counts. A sample of 1000 nodes is sufficient to estimate distinct values and distribution.
- **Refreshed periodically:** Once per hour, once per day, etc. Configurable based on workload.

### Query Planner Caching

The query planner reads statistics at planning time. Statistics are cached in memory and refreshed periodically or on schema change.

```rust
struct CachedStatistics {
    entity_type: String,
    field: String,
    stats: IndexStats,
    loaded_at: i64,
}

impl QueryPlanner {
    async fn get_stats(&self, entity_type: &str, field: &str) -> Option<IndexStats> {
        // Check cache first
        if let Some(cached) = self.stats_cache.get(&(entity_type, field)) {
            if now() - cached.loaded_at < STATS_CACHE_TTL {
                return Some(cached.stats.clone());
            }
        }

        // Cache miss or expired: load from FDB
        let key = (namespace, "_meta", "stats", entity_type, field);
        match db.get(&key).await {
            Ok(Some(bytes)) => {
                let stats = serde_json::from_slice(&bytes).ok()?;
                self.stats_cache.insert((entity_type, field), CachedStatistics {
                    stats: stats.clone(),
                    loaded_at: now(),
                });
                Some(stats)
            }
            _ => None,
        }
    }
}
```

---

## 6. Cost Model

The cost model estimates FDB operations for each plan variant and chooses the cheapest.

```rust
enum ExecutionStrategy {
    SingleIndex {
        field: String,
        estimated_cardinality: f64,
    },
    MergeJoin {
        fields: Vec<String>,
        estimated_intersection_cardinality: f64,
    },
    FullScan,
}

fn estimate_cost(strategy: &ExecutionStrategy, stats: &Statistics) -> f64 {
    match strategy {
        ExecutionStrategy::SingleIndex { estimated_cardinality, .. } => {
            // Cost: 1 index scan + residual filtering
            let scan_cost = 1.0 * (estimated_cardinality / 1000.0).ceil();
            let residual_cost = estimated_cardinality * 0.01;  // assume 1% CPU cost per row
            scan_cost + residual_cost
        }
        ExecutionStrategy::MergeJoin { fields, estimated_intersection_cardinality } => {
            // Cost: scan both indexes + merge-join
            let scan_costs: f64 = fields.iter()
                .map(|f| estimate_selectivity_for_field(f) / 1000.0)
                .sum();
            let merge_cost = estimated_intersection_cardinality * 0.001;
            scan_costs + merge_cost
        }
        ExecutionStrategy::FullScan => {
            // Cost: scan all nodes
            stats.total_nodes as f64 / 1000.0
        }
    }
}

fn choose_strategy(strategies: Vec<ExecutionStrategy>, stats: &Statistics) -> ExecutionStrategy {
    strategies.into_iter()
        .min_by_key(|s| (estimate_cost(s, stats) * 1000.0) as i64)
        .unwrap()
}
```

---

## 7. EXPLAIN Endpoint

Clients can request the query plan without executing it (resolves gap I6).

```protobuf
message ExplainRequest {
    string entity_type = 1;
    string cel_expression = 2;
}

message ExplainResponse {
    QueryPlan plan = 1;
    repeated string steps = 2;  // human-readable breakdown
    repeated StatisticsSummary statistics = 3;
}

message QueryPlan {
    string strategy = 1;  // "single_index", "merge_join", "full_scan"
    repeated IndexOperation index_scans = 2;
    string residual_filter = 3;
    double estimated_cost = 4;
    double estimated_cardinality = 5;
}
```

**Example response:**
```json
{
  "plan": {
    "strategy": "merge_join",
    "index_scans": [
      {
        "field": "age",
        "operator": ">=",
        "value": "30",
        "estimated_rows": 8000
      },
      {
        "field": "salary",
        "operator": ">=",
        "value": 100000,
        "estimated_rows": 2000
      }
    ],
    "residual_filter": null,
    "estimated_cost": 12.5,
    "estimated_cardinality": 1500
  }
}
```

---

## 8. How Other Systems Solve This

### DynamoDB

Single-index selection, no automatic intersection. Developer specifies which index to use; any other predicates are applied as client-side filters.

**Tradeoff:** Simplicity at the cost of developer responsibility.

### Postgres

Cost-based optimizer with detailed statistics (histograms per column). Estimates cardinality and cost for each plan, chooses cheapest. Supports bitmap index scans to combine multiple single-column indexes.

**Tradeoff:** Complexity in the planner, but automatic optimal plan selection.

### Neo4j

Cost-based optimizer with statistics. Supports index intersection (index seeks combined via set intersection). Schema allows composite indexes.

**Tradeoff:** Similar to Postgres; more feature-rich query optimization.

### CockroachDB

Postgres-compatible cost-based planner with automatic statistics collection and "zigzag join" for multi-index intersection.

**Tradeoff:** Sophisticated but reliable.

---

## 9. Complete System Architecture

### The Full Picture

The query execution system combines all components:

```
CEL Expression
     │
     ▼
┌─────────────────────────────┐
│ 1. Parse & Type-Check       │
│    Validate against schema   │
└──────────┬──────────────────┘
           │
     ▼─────────────┐
     │             │
     ▼             ▼
┌─────────┐ ┌──────────────┐
│ Indexed │ │  Unindexed   │
│ Preds   │ │  Preds       │
└────┬────┘ └──────────────┘
     │
     ▼
┌─────────────────────────────┐
│ 2. Load Statistics (FDB)    │
│    (or use heuristics)      │
└──────────┬──────────────────┘
           │
     ▼─────────────┐
     │             │
     ▼             ▼
┌─────────────┐ ┌──────────────────┐
│ Selectivity │ │ For each predicate│
│ Estimates   │ │ check composite   │
└────┬────────┘ └──────────────────┘
     │
     ▼
┌─────────────────────────────────┐
│ 3. Decision Algorithm           │
│ - Single-best: if 1 indexed     │
│ - Composite: if match found     │
│ - Merge-join: if similar sel.   │
│ - Full scan: fallback           │
└──────────┬──────────────────────┘
           │
     ▼─────────────┐
     │             │
     ▼             ▼
┌──────────────┐ ┌───────────────────┐
│ Primary Plan │ │ Residual Filter   │
│ (index scans)│ │ (in-memory)       │
└──────┬───────┘ └───────────────────┘
       │
       ▼
   Execute
```

### Component Interaction

1. **Query Planner** (API layer):
   - Parses CEL, type-checks against schema
   - Loads statistics from FDB (cached locally)
   - Estimates selectivity for each indexed predicate
   - Applies decision algorithm
   - Returns ExecutionPlan

2. **Statistics Maintenance** (background job):
   - Periodically samples entity types
   - Computes per-field statistics
   - Updates FDB metadata asynchronously
   - No impact on write path

3. **Query Executor** (query execution layer):
   - Takes ExecutionPlan from planner
   - Executes primary strategy (index scan, merge-join, or full scan)
   - Applies residual filter to results
   - Returns streaming results to client

4. **EXPLAIN Endpoint** (API layer):
   - Takes CEL expression
   - Runs planner (same as execution)
   - Returns plan without executing
   - Clients debug slow queries

---

## 10. Implementation Dependencies and Natural Ordering

The above describes the **complete theoretical system**. Implementation naturally follows this dependency order:

### Phase 1: Baseline (Single-Index + Composite)
- Implement single-index selection with hardcoded heuristics (no statistics)
- Composite index support
- Full scan fallback
- Code: ~500 lines for planner, 200 lines for executor
- Cost: Minimal; heuristics are simple

**Why first:** This handles the majority of real-world queries. Developers declare composites for hot patterns. Heuristics work reasonably well (10% for equality, 50% for range).

### Phase 1.5: Statistics Collection (Optional, Early)
- Background job for statistics maintenance
- Per-field cardinality counters in FDB metadata
- Better selectivity estimation than heuristics
- Code: ~200 lines for job, ~100 lines for stats lookup
- Cost: Minimal; background job runs infrequently, no write-path overhead

**Why soon:** Low complexity, significant improvement over heuristics. Enables better planning without merge-join.

### Phase 2: Merge-Join + Cost Model
- Implement merge-join algorithm
- Build cost model (estimate FDB operations)
- Decision algorithm: choose between single-best and merge-join based on cost
- EXPLAIN endpoint
- Code: ~400 lines for merge-join, ~200 lines for cost model

**Why second:** Builds on Phase 1 infrastructure. Requires statistics to be worthwhile (Phase 1.5). Handles edge cases with similar selectivity.

### Phase 2.5: Advanced Optimization (Optional)
- Bitmap index intersection (for dense ID spaces, not UUIDs)
- Histogram-based selectivity (instead of assuming 50% for range)
- Adaptive planning (track actual vs estimated cardinality)
- Code: ~300 lines each, optional

**Why optional:** Incremental improvements. Not necessary for MVP or Phase 2.

---

## 11. Risk Mitigation

### Risk: Suboptimal Plans with Heuristics (Phase 1)

**Mitigation:**
- Provide EXPLAIN command early so developers can see and debug plans
- Document heuristic rules clearly
- Recommend composite indexes for known hot queries
- Accept that Phase 1 plans may not be optimal but are always correct

### Risk: Statistics Staleness

**Mitigation:**
- Statistics are eventually-consistent by design (acceptable, like Postgres)
- Background job refreshes once per hour (configurable)
- Manual refresh command available
- Query planner falls back to heuristics if statistics unavailable

### Risk: Merge-Join Memory Usage

**Mitigation:**
- For very large result sets (> 100K IDs per index), fall back to single-best
- Implement streaming variant: buffer one batch per index instead of full lists
- Log warning if either index scan returns > 100K IDs

---

## 12. Testing Strategy

### Unit Tests
- Selectivity estimation (heuristic and with statistics)
- Decision algorithm (single-best vs merge-join vs full scan)
- Cost model (estimate cost for different strategies)

### Integration Tests
- Planning with various predicate combinations
- Composite index detection and usage
- Fallback to full scan when appropriate

### Performance Tests
- Planning latency (should be < 1ms)
- Statistics collection impact (background job overhead)
- Merge-join vs single-best actual execution time

---

## 13. Conclusion

This document presents the **complete theoretical design** for query execution and index selection. The system is modeled after proven approaches: DynamoDB's simplicity (single index selection) and Postgres's sophistication (cost-based optimization with statistics).

### Key Principles

1. **Single-index selection is the default:** Pick the most selective index, apply remaining predicates as residual filters. This is always correct, simple to implement, and efficient for most queries.

2. **Statistics drive optimization:** Background job maintains per-field cardinality. Query planner uses statistics to estimate selectivity and choose between execution strategies.

3. **Composite indexes are primary:** Developers declare composites for known multi-field patterns. Planner checks for composite matches before applying general strategies.

4. **Multi-index intersection (merge-join) is a refinement:** When two or more indexed predicates have similar selectivity, merging sorted ID lists can beat single-index + residual. Cost model decides.

5. **Full scan is the fallback:** When few predicates are indexed or cost estimates favor it, scan all nodes and apply predicates as residual filter.

6. **EXPLAIN endpoint for debugging:** Clients can inspect query plans without executing, helping developers optimize schemas and queries.

### Implementation Strategy

- **Phase 1:** Single-index + composite + heuristics
- **Phase 1.5 (optional early):** Statistics collection
- **Phase 2:** Merge-join + cost model + EXPLAIN
- **Phase 2.5 (optional):** Advanced optimizations (bitmap, histogram, adaptive)

This phasing allows shipping an MVP in Phase 1, improving it incrementally, and eventually reaching a Postgres-like cost-based optimizer.

---

**Document authored by:** Andrew + Claude
**Reviewed by:** (pending)
**Date:** 2026-02-10
**Status:** Research (ready for design review)
