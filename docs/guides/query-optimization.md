# Query Optimization

Explain plans, index selection, and tuning strategies for PelagoDB queries.

## Query Surface Selection

| Workload | Preferred Surface | Tuning Knobs |
|---|---|---|
| Property filtering (`age >= 30`) | `FindNodes` (CEL) | Indexes + `Explain` + keyset cursor pagination |
| One-hop/multi-hop graph paths | `Traverse` / PQL traversal blocks | `max_depth`, `max_results`, `timeout_ms`, node/edge filters |
| Multi-stage graph selection | PQL multi-block + captures | Keep blocks selective early (`@filter`, `@limit`) |
| Large OR of equality terms | `FindNodes` with simple `==`/`&&`/`\|\|` | Term-posting fast path |

## Using Explain Plans

### CEL Explain

```bash
grpcurl -plaintext \
  -d '{
    "context": {"database":"default","namespace":"default"},
    "entity_type": "Person",
    "cel_expression": "age >= 30 && active == true"
  }' \
  127.0.0.1:27615 pelago.v1.QueryService/Explain
```

Look for `index_scan` vs `full_scan` in the plan output.

### PQL Explain

```bash
pelago query pql --query "Person @filter(age >= 30) { uid name age }" --explain
```

Or in the REPL:
```
:explain Person @filter(age >= 30) { uid name age }
```

## Index Selection

The query planner selects indexes based on CEL expression shape:

| Expression Shape | Index Used | Notes |
|---|---|---|
| `field == value` | `equality` or `unique` | Fast path for simple equality |
| `field >= value` | `range` | Range scan |
| `field == a && field2 >= b` | Best available per field | Conjunction of indexed fields |
| `field == a \|\| field == b` | Term-posting acceleration | Only for simple `==` with `&&/||` |
| Complex boolean with parens | May fall back to scan | Planner works best on simple conjunctions |

### Term-Posting Fast Path

The query executor has a term-posting acceleration path for simple equality boolean expressions. This works for:
- `status == "active"` — simple equality
- `status == "active" && role == "admin"` — conjunction of equalities
- `status == "active" || status == "pending"` — disjunction of equalities

This does **not** work for:
- Parenthesized boolean grouping
- Non-equality operators in the fast path

## Schema Tuning Matrix

| Goal | Primary Tool | Tradeoff |
|---|---|---|
| Fast exact lookup | `index: unique` | Higher write cost, uniqueness enforcement |
| Fast equality filters | `index: equality` | Extra index write amplification |
| Fast range queries | `index: range` | Larger index footprint |
| Stable edge ordering | Edge `sort_key` | Additional key complexity |
| Strict API contracts | `required`, `extras_policy: reject` | Lower flexibility |
| Flexible ingest | `extras_policy: allow` or `warn` | Weaker data-shape guarantees |

## Consistency and Snapshot Tuning

| Control | Options |
|---|---|
| `ReadConsistency` | `STRONG`, `SESSION`, `EVENTUAL` |
| `SnapshotMode` | `STRICT`, `BEST_EFFORT` |
| `allow_degrade_to_best_effort` | Fallback instead of hard failure |

Default snapshot modes:
- `FindNodes`: `STRICT`
- `Traverse`: `BEST_EFFORT`
- `ExecutePQL`: `BEST_EFFORT`

Strict guardrails: ~2000 ms elapsed, ~50,000 scanned keys, ~8 MiB result bytes.

## Traversal Tuning

Always bound traversals:

```bash
pelago query traverse Person:1_0 follows --max-depth 2 --max-results 200
```

| Parameter | Purpose | Recommendation |
|---|---|---|
| `max_depth` | Maximum hop count | Keep at 2-3 for user-facing queries |
| `max_results` | Total result cap | Match to UI/API page size |
| `timeout_ms` | Hard time limit | Set based on SLO (e.g., 2000 ms) |
| `node_filter` | Per-step CEL filter | Reduce candidate set early |
| `edge_filter` | Edge property filter | Narrow edge traversal |

## PQL Optimization

- Keep blocks selective early with `@filter` and `@limit`
- Use variables to compose intermediate sets efficiently
- Prefer `FindNodes`/`Traverse` for paginated user-facing endpoints
- Use `ExecutePQL` for internal workflows and set-algebra selection

## Query Tuning Checklist

- [ ] Indexed CEL filters verified via `Explain`
- [ ] Limits/cursors configured for pageable surfaces
- [ ] Traversal depth/results/timeouts bounded
- [ ] Strict snapshots used only where required
- [ ] Benchmarks run after schema/query changes

## Related

- [CEL Filters Reference](../reference/cel-filters.md) — expression syntax
- [PQL Reference](../reference/pql.md) — query language
- [Data Modeling Patterns](data-modeling-patterns.md) — schema design
- [Schema Design Strategy](schema-design-strategy.md) — index strategies
