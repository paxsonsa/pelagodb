# Schema Design Strategy

Index strategy, DDL vs JSON, and schema evolution best practices.

## Registration Formats

PelagoDB supports three schema registration formats:

| Format | Best For | Example |
|---|---|---|
| Compact JSON | Scripts and automation | `pelago schema register --inline '{...}'` |
| Protobuf JSON | Full feature access (edge properties, sort keys) | `grpcurl` or SDK registration |
| SQL-like DDL | Interactive REPL and readability | `CREATE TYPE Person (...)` |

For edge property schemas and sort key validation, use the protobuf JSON or gRPC path — the compact JSON and Python SDK helper have narrower coverage.

## Index Strategy

### When to Index

| Index Type | When to Use | When Not To |
|---|---|---|
| `unique` | Business keys (`email`, `sku`, `order_number`) | Fields that aren't used for lookups |
| `equality` | Status/category filters (`status`, `role`, `stage`) | Low-selectivity fields on small types |
| `range` | Numeric/time ranges (`age`, `created_at`, `priority`) | Fields only used in edge properties |
| `none` | Description fields, notes, large text | — |

### Auto-Indexing Defaults

With `AUTO_BY_TYPE_V1` (required on registration):
- `int`, `float`, `timestamp` → `range`
- `bool` → `equality`
- `string`, `bytes` → `none`

Use explicit `"index": "none"` to override inferred indexing.

### Index Maintenance

- Null/missing values do not create index entries
- Adding a new indexed property triggers an `IndexBackfill` background job
- Monitor via `pelago admin job list` / `pelago admin job status <job_id>`

## Schema Metadata Choices

### `extras_policy`

| Policy | Behavior | Use When |
|---|---|---|
| `reject` | Fail on undeclared properties | Strict API contracts |
| `warn` | Accept but log warning | Transition period |
| `allow` | Accept silently | Flexible ingest, exploration |

### `allow_undeclared_edges`

- `true`: Accept any edge label as outgoing
- `false`: Only declared edge labels are accepted

### `required` Fields

Required fields must exist and be non-null on create. Defaults are applied before validation, so a field with a default value does not need to be explicitly provided.

## Schema Evolution

### Versioning

- First registration creates version `1`
- Each subsequent registration increments version
- Historical versions remain queryable

```bash
pelago schema get Person --version 2
pelago schema diff Person 1 2
```

### Safe Evolution Patterns

1. **Additive changes first:** Add new optional properties before making them required
2. **Index backfill:** Adding indexes to existing types creates async backfill jobs — monitor completion
3. **Extras policy transition:** Start with `allow`, tighten to `warn`, then `reject`
4. **Per-namespace rollout:** Isolate schema changes in tenant namespaces when global coupling is unnecessary

### Breaking Changes

- Adding `required: true` to an existing property may break existing writers
- Changing from `allow` to `reject` extras policy will reject previously valid payloads
- Changing index types requires dropping the old index and backfilling the new one

## Schema Rollout Checklist

| Step | Action | Verify |
|---|---|---|
| 1 | Define/adjust schema with explicit indexes and metadata | `schema register` succeeds, version increments |
| 2 | Validate strictness choice | Create/update tests pass for expected payloads |
| 3 | Monitor backfill for new indexes on existing types | `admin job list/status` reaches `COMPLETED` |
| 4 | Run query explain against critical CEL filters | Planner selects intended index path |
| 5 | Load-test hot reads/writes with representative data | p95/p99 acceptable |

## Related

- [Schema Specification](../reference/schema-spec.md) — format reference
- [Data Modeling Patterns](data-modeling-patterns.md) — modeling best practices
- [Query Optimization](query-optimization.md) — index selection in queries
- [Schema System Concept](../concepts/schema-system.md) — schema-first philosophy
