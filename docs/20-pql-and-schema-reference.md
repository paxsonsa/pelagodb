# PQL and Schema Reference

This document is the canonical reference for:
- defining entity schemas
- writing and executing PQL queries
- understanding data types and index behavior
- tuning PelagoDB for schema/query workloads

Source-of-truth implementation references:
- `proto/pelago.proto`
- `crates/pelago-core/src/schema.rs`
- `crates/pelago-storage/src/schema.rs`
- `crates/pelago-storage/src/node.rs`
- `crates/pelago-storage/src/edge.rs`
- `crates/pelago-query/src/pql/*`
- `crates/pelago-api/src/query_service.rs`

## 1) Schema Definitions

## 1.1 Entity Schema Model

An entity schema (`EntitySchema`) defines node properties, edge contracts, and validation metadata.

| Field | Type | Required | Notes |
|---|---|---|---|
| `name` | `string` | Yes | Entity type name (example: `Person`) |
| `version` | `uint32` | Server-assigned | Starts at `1`, increments on each schema register/update |
| `properties` | `map<string, PropertyDef>` | No | Node properties |
| `edges` | `map<string, EdgeDef>` | No | Edge definitions keyed by edge label |
| `meta` | `SchemaMeta` | No | Validation policy |
| `created_at` | `int64` | Server-managed | Unix microseconds |
| `created_by` | `string` | Optional | Creator identity |

### `PropertyDef`

| Field | Type | Notes |
|---|---|---|
| `type` | `PropertyType` | `string`, `int`, `float`, `bool`, `timestamp`, `bytes` |
| `required` | `bool` | Required fields cannot be missing or `null` |
| `index` | `IndexType` (optional on write) | `none`, `unique`, `equality`, `range`; omitted index uses server defaults (see below) |
| `default_value` | `Value` | Applied only on create when property is absent |

### `EdgeDef`

| Field | Type | Notes |
|---|---|---|
| `target` | `EdgeTarget` | Specific entity type or polymorphic (`*`) |
| `direction` | `outgoing` or `bidirectional` | Bidirectional writes reverse-direction keys automatically |
| `properties` | `map<string, PropertyDef>` | Edge property schema |
| `sort_key` | `string` | Must exist in `edge.properties`; affects edge key ordering |
| `ownership` | `source_site` or `independent` | `independent` is rejected in current v1 runtime |

### `SchemaMeta`

| Field | Type | Notes |
|---|---|---|
| `allow_undeclared_edges` | `bool` | If `true`, undeclared labels are accepted as outgoing |
| `extras_policy` | `reject`, `allow`, `warn` | Behavior for properties not declared in schema |

## 1.2 Data Types and Value Semantics

PelagoDB value model (`Value`):
- `string_value`
- `int_value` (`int64`)
- `float_value` (`double`)
- `bool_value`
- `timestamp_value` (`int64`, Unix microseconds)
- `bytes_value`
- `null_value`

Type matching rules:
- `null` matches any property type for optional fields.
- `required=true` fields cannot be `null`.
- Timestamp values are integer microseconds (`int64`), not ISO strings.

## 1.3 Index Types

| Index Type | Enforces uniqueness | Supports equality lookups | Supports range operators (`>`, `>=`, `<`, `<=`) | Typical use |
|---|---|---|---|---|
| `none` | No | No | No | Rarely-filtered fields |
| `unique` | Yes | Yes | No | Business keys (`email`, `sku`) |
| `equality` | No | Yes | No | Status/category filters |
| `range` | No | Yes | Yes | Numeric/time ranges |

### Schema registration defaults (global)

On schema registration, server now requires:
- `RegisterSchemaRequest.index_default_mode = INDEX_DEFAULT_MODE_AUTO_BY_TYPE_V1`

With `AUTO_BY_TYPE_V1`, omitted property index values are inferred by type:
- `int`, `float`, `timestamp` -> `range`
- `bool` -> `equality`
- `string`, `bytes` -> `none`

Explicit index values always win, including explicit opt-out:
- `index: none` disables inferred indexing for that property.

Index maintenance behavior:
- Null or missing values do not create index entries.
- Adding a new indexed property to an existing type creates an `IndexBackfill` background job.

## 1.4 Validation and Defaults

Server-side schema validation currently enforces:
- Schema `name` is non-empty and only `[A-Za-z0-9_]`.
- Property names are non-empty.
- Edge names are non-empty.
- `sort_key` must reference an existing edge property.
- `ownership=independent` is rejected in current v1 runtime.

Node validation rules:
- Required fields must exist and be non-null.
- Value types must match schema property types.
- Extra properties follow `extras_policy` (`reject`, `warn`, `allow`).
- Defaults are applied before validation on create.
- Edge property payloads are currently stored as-is; runtime does not enforce `edge.properties` type/required constraints on write.

Practical note:
- For `unique` fields, prefer omitting absent values instead of explicitly sending `null`.

## 1.5 Schema Registration Surfaces

### Canonical surface: gRPC / protobuf API (full schema support)

Use `SchemaService.RegisterSchema` with full protobuf fields when you need:
- edge definitions
- edge property schemas
- default values
- sort keys
- full metadata policy

Breaking contract change:
- `RegisterSchemaRequest.index_default_mode` is now required.
- Legacy writers that do not set `INDEX_DEFAULT_MODE_AUTO_BY_TYPE_V1` are rejected with `INVALID_ARGUMENT`.

Example (`RegisterSchemaRequest.schema` shape):

```json
{
  "name": "Person",
  "properties": {
    "email": {
      "type": "PROPERTY_TYPE_STRING",
      "required": true,
      "index": "INDEX_TYPE_UNIQUE"
    },
    "age": {
      "type": "PROPERTY_TYPE_INT",
      "index": "INDEX_TYPE_RANGE"
    },
    "active": {
      "type": "PROPERTY_TYPE_BOOL",
      "default_value": { "bool_value": true }
    }
  },
  "edges": {
    "KNOWS": {
      "target": { "specific_type": "Person" },
      "direction": "EDGE_DIRECTION_DEF_BIDIRECTIONAL",
      "properties": {
        "since": { "type": "PROPERTY_TYPE_TIMESTAMP" }
      },
      "sort_key": "since",
      "ownership": "OWNERSHIP_MODE_SOURCE_SITE"
    }
  },
  "meta": {
    "allow_undeclared_edges": false,
    "extras_policy": "EXTRAS_POLICY_REJECT"
  }
}
```

And request mode:

```json
{
  "index_default_mode": "INDEX_DEFAULT_MODE_AUTO_BY_TYPE_V1"
}
```

### Python `register_schema_dict` helper: convenience subset

`clients/python/pelagodb/client.py` exposes `register_schema_dict`, which is convenient but currently narrower than raw protobuf:
- supports node property type/required/index/default (`default`)
- supports edge target/direction/sort_key/ownership
- does not currently populate `edges.<label>.properties` from dict input
- omitting a property `index` now intentionally defers to server type-based defaults
- set `"index": "none"` to explicitly opt out of inferred indexes

If you need edge property schema + sort key validation, use the raw protobuf registration path.

Python helper example (valid subset):

```python
schema = {
  "name": "Person",
  "properties": {
    "email": {"type": "string", "required": True, "index": "unique"},
    "age": {"type": "int", "index": "range"},
    "active": {"type": "bool", "default": True}
  },
  "edges": {
    "KNOWS": {
      "target": "Person",
      "direction": "bidirectional",
      "ownership": "source_site"
    }
  },
  "meta": {
    "allow_undeclared_edges": False,
    "extras_policy": "reject"
  }
}
client.register_schema_dict(schema)
```

### CLI surface: JSON + protobuf-style JSON + SQL-like DDL

`pelago schema register` accepts:
- compact JSON (`type`, `required`, `index`, `default`)
- protobuf-style JSON (`PROPERTY_TYPE_*`, `INDEX_TYPE_*`, `default_value`, full `edges.*` shape)
- SQL-like DDL (`CREATE TYPE` / `CREATE SCHEMA`)

For `index`, omitted values remain unset so server-side defaults can infer by type; use explicit `"index": "none"` to opt out per property.

Example (compact JSON):

```json
{
  "name": "Person",
  "properties": {
    "name": { "type": "string", "required": true },
    "age": { "type": "int", "index": "range" },
    "email": { "type": "string", "index": "unique" }
  },
  "meta": {
    "allow_undeclared_edges": false,
    "extras_policy": "reject"
  }
}
```

Example (SQL-like):

```sql
CREATE TYPE Person (
  email STRING REQUIRED INDEX UNIQUE,
  age INT INDEX RANGE,
  active BOOL DEFAULT true
)
WITH (allow_undeclared_edges = false, extras_policy = reject)
EDGE KNOWS TO Person (
  since TIMESTAMP
) DIRECTION BIDIRECTIONAL OWNERSHIP SOURCE_SITE SORT_KEY since;
```

## 1.6 Schema Evolution

Versioning behavior:
- First registration is version `1`.
- Every subsequent registration of same `name` increments version.
- Historical versions remain queryable (`GetSchema.version`).

Operational tools:
- `pelago schema get <Type> --version <N>`
- `pelago schema diff <Type> <V1> <V2>`
- `pelago admin job list` / `pelago admin job status <job_id>` for async backfills/cleanup

## 2) PQL Reference

## 2.1 Query Forms

### Short form

```pql
Person @filter(age >= 30) @limit(first: 50) { name, age }
```

Short form is equivalent to a single block rooted at `type(Person)`.

### Full form

```pql
query friends {
  seed(func: uid(Person:1_42)) {
    name
    -[KNOWS]-> @filter(age >= 30) { name, age }
  }
}
```

## 2.2 Core Grammar Elements

- Comments start with `#`.
- Identifiers are `[A-Za-z][A-Za-z0-9_]*`.
- Node IDs in `uid(Type:id)` are `[A-Za-z0-9_-]+`.
- Edge labels in PQL grammar are uppercase-first (`[A-Z][A-Za-z0-9_]*`).

Recommendation:
- Use alphanumeric + underscore naming for types/properties/edge labels to keep schema, CEL, and PQL compatibility predictable.

## 2.3 Root Functions

| Root function | Parsed | ExecutePQL runtime |
|---|---|---|
| `uid(Type:id)` | Yes | Yes |
| `uid(var)` | Yes | Yes |
| `uid(a, b, ...)` | Yes | Yes (`union` default, `intersect`/`difference` supported) |
| `type(Type)` | Yes | Yes |
| `eq/ge/le/gt/lt/between/has/allofterms` as block root | Yes | Not executable in `ExecutePQL` blocks (no type context in resolver) |

Important:
- `upsert` blocks are explicitly rejected at parse time.

## 2.4 Selections

Selection kinds:
- Property field selection (`name`, `age`)
- Edge traversal (`-[KNOWS]-> { ... }`)
- Aggregate expressions (`count(...)`, `sum(...)`, etc.)
- Value variables (`x as count(...)`)

Current runtime behavior in `ExecutePQL`:
- Field selection: executed.
- Edge traversal: executed as traversal steps.
- Aggregates/value-vars: parsed but not returned as aggregate result payloads.

## 2.5 Directives

### Block-level directives

| Directive | Parsed | Executed on `ExecutePQL` block |
|---|---|---|
| `@filter(expr)` | Yes | Yes (`type`, `uid(var)`, `uid(set)`, short-form type blocks) |
| `@limit(first:N[, offset:M])` | Yes | Yes |
| `@cascade` | Yes | Yes only for `uid(Type:id)` traversal blocks |
| `@recurse(depth:N)` | Yes | Yes only for `uid(Type:id)` traversal blocks |
| `@sort(...)` | Yes | Compile error at block level |
| `@groupby(...)` | Yes | Compile error |
| `@explain` | Yes | Compile error (use request-level `explain=true`) |

Additional rule:
- For plain point lookups (`uid(Type:id)`) without edge traversal selections, block directives are currently unsupported.

### Edge-level directives

| Directive | Parsed | Compile accepted | Current traversal runtime effect |
|---|---|---|---|
| `@edge(expr)` | Yes | Yes | Applied as edge filter |
| `@filter(expr)` | Yes | Yes | Applied as target-node filter |
| `@limit(first:N[, offset:M])` | Yes | Yes | Captured, but per-node fanout limit is not currently enforced in traversal engine |
| `@sort(field:asc|desc[, on_edge:true|false])` | Yes | Yes | Captured in compiled plan/explain; not applied in traversal execution |
| `@facets(a,b,...)` | Yes | Yes | Captured in compiled plan/explain; not emitted as facet payload in PQL results |
| `@groupby(...)`, `@recurse(...)`, `@cascade`, `@explain` | Yes | No | Compile error on edge |

## 2.6 Variables and Set Operations

Block capture:

```pql
query {
  friends as seed(func: uid(Person:1_42)) {
    -[KNOWS]-> { name }
  }
  mutual(func: uid(friends)) @filter(age >= 25) { name, age }
}
```

Set operations:
- `uid(a, b)` -> union
- `uid(a, b, intersect)` -> intersection
- `uid(a, b, difference)` -> subtraction (`a - b`)

## 2.7 Parameters

`ExecutePQLRequest.params` supports `$name` replacement before parsing.

Example request payload fragment:

```json
{
  "pql": "Person @filter(age >= $min_age) { name, age }",
  "params": {
    "min_age": "30"
  }
}
```

Notes:
- Replacement is token-aware in server-side `ExecutePQL`.
- Unresolved `$name` tokens are left unchanged.
- CLI `pelago query pql` currently has no explicit `--params` flag; REPL supports `:param` substitutions client-side.

## 2.8 Execution Semantics and Result Shape

`ExecutePQL` returns a stream of `PQLResult` rows:
- `block_name`
- `node` (commonly populated)
- `edge` (currently not populated by execute path)
- `explain` (when request `explain=true`)
- snapshot/degradation metadata

Behavior notes:
- Results are emitted block-by-block in resolved dependency order.
- `next_cursor` is part of the protobuf schema, but `ExecutePQL` currently does not provide paginated continuation behavior.
- For large paginated reads, use `FindNodes` and `Traverse` endpoints directly.

## 2.9 Current PQL Limitations (Important)

These constructs parse but are not fully executed today in `ExecutePQL`:
- Root predicate functions (`eq/ge/le/gt/lt/between/has/allofterms`) as standalone full-query roots
- Nested traversal blocks beyond top-level edge step compilation
- Edge namespace and target type qualifiers in traversal execution
- Edge-level sort/facets/per-node-limit enforcement in traversal runtime
- Aggregate result materialization

## 3) Tuning DB and Schemas

## 3.1 Schema Tuning Matrix

| Goal | Primary tool | Secondary tools | Tradeoff |
|---|---|---|---|
| Fast exact lookup by business key | `index: unique` | `required: true` | Higher write cost and uniqueness enforcement |
| Fast equality filters | `index: equality` | `extras_policy: reject` for cleaner data shape | Extra index write amplification |
| Fast numeric/time range queries | `index: range` | Use `timestamp`/`int` fields | Larger index footprint than no index |
| Stable edge ordering per source | Edge `sort_key` + consistently populated edge property | Edge label-specific query patterns | Additional key complexity |
| Strict API contracts | `required`, `extras_policy: reject`, explicit edges | `allow_undeclared_edges: false` | Lower schema flexibility |
| Flexible ingest phase | `extras_policy: allow` or `warn` | Backfill and tighten later | Weaker data-shape guarantees |

## 3.2 Query Tuning Matrix

| Workload | Preferred surface | Tuning knobs |
|---|---|---|
| Property filtering (`age >= 30`) | `FindNodes` (CEL) | Proper indexes + `Explain` + keyset cursor pagination |
| One-hop/multi-hop graph paths | `Traverse` / PQL traversal blocks | `max_depth`, `max_results`, `timeout_ms`, node/edge filters |
| Variable-based multi-stage graph selection | PQL multi-block + captures | Keep blocks selective early (`@filter`, `@limit`) |
| Large OR of equality terms | `FindNodes` CEL with simple `==`/`&&`/`||` forms | Leverages term-posting fast path for supported expressions |

Fast-path note:
- Term-posting acceleration in query executor targets simple equality boolean expressions.
- Parenthesized boolean grouping and non-equality operators are not part of that fast path.

## 3.3 Consistency and Snapshot Tuning

Per-request controls:
- `ReadConsistency`: `STRONG`, `SESSION`, `EVENTUAL`
- `SnapshotMode`: `STRICT`, `BEST_EFFORT`
- `allow_degrade_to_best_effort`: enables fallback instead of hard failure when strict snapshot budgets are exceeded

Default snapshot mode when request value is unspecified:
- `FindNodes`: `STRICT`
- `Traverse`: `BEST_EFFORT`
- `ExecutePQL`: `BEST_EFFORT`

Strict snapshot guardrails (query service):
- max elapsed: ~2000 ms
- max scanned keys: ~50,000
- max result bytes: ~8 MiB

When strict limits are exceeded:
- if `allow_degrade_to_best_effort=true`: response is marked degraded
- else: request fails with snapshot budget error

## 3.4 Admin and Operational Tuning Tools

Schema/index maintenance:
- `AdminService.DropIndex` (drop index entries)
- `AdminService.StripProperty` (async job)
- `schema diff/get/list` (track evolution)

Query planning visibility:
- `QueryService.Explain` for CEL plans
- `ExecutePQL(explain=true)` for compiled PQL block plans

Large mutation safety:
- `MutationExecutionMode`: `INLINE_STRICT` vs `ASYNC_ALLOWED`
- Large delete/drop operations may return cleanup job IDs instead of inline execution

Job operations:
- `pelago admin job list`
- `pelago admin job status <job_id>`

## 3.5 Server-Level Tuning Knobs

High-impact runtime knobs in `ServerConfig` / env:
- Cache: `PELAGO_CACHE_*` (enablement, size, buffers, projector scopes)
- Replication: `PELAGO_REPLICATION_*` (peers, scopes, batch, poll, lease)
- Watch: `PELAGO_WATCH_*` (subscription limits, queue, TTL)
- Audit retention: `PELAGO_AUDIT_*`
- Namespace defaults and worker scope: `PELAGO_DEFAULT_DATABASE`, `PELAGO_DEFAULT_NAMESPACE`

Reference:
- `crates/pelago-core/src/config.rs`
- `pelago-server.example.toml`
- `docs/04-administration.md`

## 4) Performance-Oriented Modeling Techniques

## 4.1 Model From Access Patterns First

Start from your top read/write paths, then shape schema and edges to fit them.

| High-value workload | Preferred model shape | Why this performs better |
|---|---|---|
| Lookup by external/business ID | Property with `index: unique` (`email`, `sku`, `order_number`) | Fast indexed lookup and deterministic uniqueness |
| Filtered lists by state + time window | `state` on `equality` index + `created_at` on `range` index | Narrows candidate set before residual filtering |
| High-fanout parent lists | Bucket/hub nodes (`Customer -> OrderMonth -> Order`) | Prevents supernode fanout blowups on single-hop scans |
| Deep workflow path queries | Explicit hierarchy (`Show -> Sequence -> Shot -> Task`) with typed edges | Keeps traversals intentional and easier to bound |
| Dynamic audience/segment selection | Separate indexed dimensions and set algebra (`uid(a,b,intersect)`) | Efficient multi-stage narrowing without overloading one query block |

## 4.2 Index Design Patterns

Pattern A: direct key + operational filters

```json
{
  "name": "Order",
  "properties": {
    "order_number": { "type": "string", "required": true, "index": "unique" },
    "customer_id": { "type": "string", "index": "equality" },
    "status": { "type": "string", "index": "equality" },
    "created_at": { "type": "timestamp", "index": "range" },
    "total_cents": { "type": "int", "index": "range" }
  }
}
```

Pattern B: synthetic composite key (when true composite indexes are unavailable)

```json
{
  "name": "Task",
  "properties": {
    "task_code": { "type": "string", "required": true, "index": "unique" },
    "stage": { "type": "string", "index": "equality" },
    "status": { "type": "string", "index": "equality" },
    "status_day_key": { "type": "string", "index": "equality" }
  }
}
```

`status_day_key` is application-maintained (example: `review:20260227`) and can reduce costly two-filter scans for high-volume dashboards.

## 4.3 Fanout and Supernode Mitigation

Use intermediate aggregation nodes when a parent can have very large child counts.

Anti-pattern:
- `Customer -[HAS_ORDER]-> Order` with millions of outgoing edges per customer.

Preferred:
- `Customer -[HAS_ORDER_MONTH]-> OrderMonth -[HAS_ORDER]-> Order`

Benefits:
- smaller per-hop edge scans
- natural pagination boundaries
- easier caching and archiving by bucket period

## 4.4 Hot/Cold Entity Split

For write-heavy domains, separate frequently changing fields from mostly static fields.

Pattern:
- `Task` (identity + static metadata + relationship anchors)
- `TaskState` (status, assignee, updated timestamp, SLA fields)

Benefits:
- lower write amplification on heavily indexed immutable attributes
- cleaner retention and audit policies for mutable state history

## 4.5 Query Surface Alignment for Performance

| Query need | Best surface | Current caveat |
|---|---|---|
| Indexed property filtering and pagination | `FindNodes` (CEL) | Planner index selection is strongest on conjunction-style predicates |
| Multi-hop expansion from known start node | `Traverse` | `TraversalStep` currently applies label/direction/edge_filter/node_filter; projection/sort/per-node-limit fields are not enforced in runtime |
| Multi-block orchestration and set operations | `ExecutePQL` | Powerful composition, but execute path has documented limitations (aggregates/materialization, pagination, some directives) |

Operational tactic:
- for user-facing paginated endpoints, prefer `FindNodes`/`Traverse`
- use `ExecutePQL` for internal workflows, explain/debug, and set-algebra style selection

## 5) Do's and Don'ts

| Do | Don't | Why |
|---|---|---|
| Define at least one stable unique business key per high-value entity | Depend only on generated node IDs for user-facing lookups | Query paths become slower and harder to make deterministic |
| Keep edge labels uppercase if you plan to use PQL edge traversal syntax | Use lowercase edge labels and expect PQL traversal parsing to work | PQL grammar expects uppercase-first edge labels |
| Add indexes only for real production filters/sorts | Index every property preemptively | Write amplification and storage growth increase quickly |
| Omit absent values for unique-indexed properties | Send explicit `null` for unique-indexed fields | Null unique values can fail during unique index encoding |
| Keep CEL filters simple for predictable planning | Assume complex boolean expressions always get indexed plans | Planner index extraction is most reliable on simple conjunction forms |
| Use simple equality boolean forms to benefit from term-posting acceleration | Expect term fast path with parenthesized/non-equality forms | Term path only supports constrained `==` with `&&/||` patterns |
| Bound graph reads with depth/result/timeout controls | Run unbounded traversals in latency-critical endpoints | Unbounded expansion causes p95/p99 instability |
| Use `QueryService.Explain` and `ExecutePQL(explain=true)` before rollout | Promote queries without plan visibility | Hidden full scans frequently surface only in production |
| Monitor backfill jobs after adding indexes | Assume new index performance is immediate | New index creation on existing data is asynchronous |
| Use `FindNodes`/`Traverse` for continuation-based pagination | Rely on `ExecutePQL` for paginated continuation | `ExecutePQL` does not currently provide continuation cursor behavior |

## 6) Complex Data Model Demonstrations

## 6.1 E-commerce Timeline Model (High-Fanout Control)

Goal:
- keep customer order-history queries fast even when order counts are very high.

Model shape:
- `Customer -[HAS_ORDER_MONTH]-> OrderMonth -[HAS_ORDER]-> Order`

Schema subset (register via gRPC/SDK/CLI):

```json
[
  {
    "name": "Customer",
    "properties": {
      "customer_id": { "type": "string", "required": true, "index": "unique" },
      "email": { "type": "string", "required": true, "index": "unique" },
      "segment": { "type": "string", "index": "equality" }
    },
    "edges": {
      "HAS_ORDER_MONTH": { "target": "OrderMonth", "direction": "outgoing" }
    }
  },
  {
    "name": "OrderMonth",
    "properties": {
      "customer_id": { "type": "string", "required": true, "index": "equality" },
      "month": { "type": "int", "required": true, "index": "range" }
    },
    "edges": {
      "HAS_ORDER": { "target": "Order", "direction": "outgoing" }
    }
  },
  {
    "name": "Order",
    "properties": {
      "order_number": { "type": "string", "required": true, "index": "unique" },
      "status": { "type": "string", "index": "equality" },
      "created_at": { "type": "timestamp", "index": "range" },
      "total_cents": { "type": "int", "index": "range" }
    }
  }
]
```

Query flow:
1. Resolve customer node with indexed lookup (`email`).
2. Traverse to month bucket and then orders with bounded results.

Traverse request shape (gRPC/SDK):

```json
{
  "start": { "entity_type": "Customer", "node_id": "1_42" },
  "steps": [
    {
      "edge_type": "HAS_ORDER_MONTH",
      "direction": "EDGE_DIRECTION_OUTGOING",
      "node_filter": "month == 202602"
    },
    {
      "edge_type": "HAS_ORDER",
      "direction": "EDGE_DIRECTION_OUTGOING",
      "node_filter": "status == \"paid\" && created_at >= 1738368000000000"
    }
  ],
  "max_depth": 2,
  "max_results": 200,
  "timeout_ms": 2000
}
```

## 6.2 VFX Workflow Graph (Deep Chain + Dependency Edges)

This mirrors `datasets/vfx_pipeline_50k`:
- hierarchy edges: `has_sequence`, `has_shot`, `has_task`
- dependency edges: `depends_on` (`Task -> Task`)

Schema subset:

```json
[
  {
    "name": "Shot",
    "properties": {
      "shot_code": { "type": "string", "required": true, "index": "unique" },
      "order": { "type": "int", "index": "range" }
    },
    "edges": {
      "has_task": { "target": "Task", "direction": "outgoing" }
    }
  },
  {
    "name": "Task",
    "properties": {
      "task_code": { "type": "string", "required": true, "index": "unique" },
      "stage": { "type": "string", "index": "equality" },
      "status": { "type": "string", "index": "equality" },
      "priority": { "type": "int", "index": "range" }
    },
    "edges": {
      "depends_on": { "target": "Task", "direction": "outgoing" }
    }
  }
]
```

High-value query pattern:
1. `FindNodes` by `shot_code` (unique).
2. `Traverse` to tasks and dependency tasks with hard bounds.

```json
{
  "start": { "entity_type": "Shot", "node_id": "1_2048" },
  "steps": [
    {
      "edge_type": "has_task",
      "direction": "EDGE_DIRECTION_OUTGOING",
      "node_filter": "stage == \"fx\" && (status == \"review\" || status == \"blocked\")"
    },
    {
      "edge_type": "depends_on",
      "direction": "EDGE_DIRECTION_OUTGOING",
      "node_filter": "status != \"approved\""
    }
  ],
  "max_depth": 2,
  "max_results": 300,
  "timeout_ms": 3000
}
```

## 6.3 PQL Segmentation Model (Set Algebra Without Deep Traversal)

Use PQL variable captures and set operations to build complex audiences from indexed predicates.

```pql
query staff_segmentation {
  sf as sf(func: type(Person)) @filter(city == "San Francisco") @limit(first: 5000) {
    name, city, role, seniority
  }
  senior as senior(func: type(Person)) @filter(seniority == "staff") @limit(first: 5000) {
    name, city, role, seniority
  }
  backend as backend(func: type(Person)) @filter(role == "Backend Engineer") @limit(first: 5000) {
    name, city, role, seniority
  }

  sf_senior(func: uid(sf, senior, intersect)) @limit(first: 1000) {
    name, city, role, seniority
  }
  final(func: uid(sf_senior, backend, intersect)) @limit(first: 200) {
    name, city, role, seniority
  }
}
```

Why this pattern is practical:
- keeps each block selective
- composes reusable intermediate sets
- avoids relying on not-yet-complete deep traversal semantics in `ExecutePQL`

## 7) Matrix Checklists

## 7.1 Data Modeling Checklist (Performance-First)

| Step | Action | Verify |
|---|---|---|
| 1 | List top 5 read paths and top 3 write paths before schema design | Every critical endpoint maps to a concrete graph path/filter |
| 2 | Assign one primary lookup key per entity (`unique` where needed) | Key lookups never require full scan |
| 3 | Add only query-critical indexes (`equality`/`range`) | `Explain` shows intended access path on hot queries |
| 4 | Identify potential supernodes and add bucket/hub entities | Traversal fanout stays bounded at expected cardinalities |
| 5 | Decide namespace/ownership boundaries for hot data | Replication/conflict behavior remains predictable under load |
| 6 | Define pagination and timeout strategy per endpoint | No user-facing query is unbounded |

## 7.2 Schema Rollout Checklist

| Step | Action | Verify |
|---|---|---|
| 1 | Define/adjust schema with explicit indexes and metadata | `schema register` succeeds, version increments |
| 2 | Validate strictness choice (`extras_policy`, required fields) | Create/update tests pass for expected payloads |
| 3 | For new indexes on existing types, monitor backfill job | `admin job list/status` reaches `COMPLETED` |
| 4 | Run query explain against critical CEL filters | Planner selects intended index path |
| 5 | Load-test hot reads/writes with representative data | p95/p99 and scan amplification acceptable |

## 7.3 Query Tuning Checklist

| Step | Action | Verify |
|---|---|---|
| 1 | Start with indexed CEL filters for high-volume reads | `Explain` shows `index_scan` when expected |
| 2 | Add limits/cursors for pageable surfaces | Stable pagination with no deep-page blowup |
| 3 | For graph paths, bound traversal depth/results/timeouts | Traversal latency and result volume stay bounded |
| 4 | Use strict snapshots only where required | Degradation/failure rates remain acceptable |
| 5 | Benchmark regularly after schema/query changes | Compare `.tmp/bench/*.json` trendlines |

## 8) Recommended Related Docs

- `docs/02-cli-reference.md`
- `docs/03-api-grpc.md`
- `docs/04-administration.md`
- `docs/14-data-modeling-and-scaling.md`
- `docs/15-architecture-and-design.md`
- `docs/09-operations-playbook.md`
- `datasets/README.md`
