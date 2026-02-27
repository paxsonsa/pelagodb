# Schema Specification

This is the canonical reference for PelagoDB entity schema format, property definitions, edge definitions, validation rules, and schema evolution.

Source of truth: `proto/pelago.proto`, `crates/pelago-core/src/schema.rs`

## Entity Schema Model

An entity schema (`EntitySchema`) defines node properties, edge contracts, and validation metadata.

| Field | Type | Required | Notes |
|---|---|---|---|
| `name` | `string` | Yes | Entity type name (e.g., `Person`) |
| `version` | `uint32` | Server-assigned | Starts at `1`, increments on each register/update |
| `properties` | `map<string, PropertyDef>` | No | Node properties |
| `edges` | `map<string, EdgeDef>` | No | Edge definitions keyed by edge label |
| `meta` | `SchemaMeta` | No | Validation policy |
| `created_at` | `int64` | Server-managed | Unix microseconds |
| `created_by` | `string` | Optional | Creator identity |

## PropertyDef

| Field | Type | Notes |
|---|---|---|
| `type` | `PropertyType` | `string`, `int`, `float`, `bool`, `timestamp`, `bytes` |
| `required` | `bool` | Required fields cannot be missing or `null` |
| `index` | `IndexType` | `none`, `unique`, `equality`, `range` (see [Index Types](#index-types)) |
| `default_value` | `Value` | Applied only on create when property is absent |

## EdgeDef

| Field | Type | Notes |
|---|---|---|
| `target` | `EdgeTarget` | Specific entity type or polymorphic (`*`) |
| `direction` | enum | `outgoing` or `bidirectional` |
| `properties` | `map<string, PropertyDef>` | Edge property schema |
| `sort_key` | `string` | Must exist in `edge.properties`; affects edge key ordering |
| `ownership` | enum | `source_site` or `independent` (`independent` rejected in v1) |

## SchemaMeta

| Field | Type | Notes |
|---|---|---|
| `allow_undeclared_edges` | `bool` | If `true`, undeclared labels accepted as outgoing |
| `extras_policy` | enum | `reject`, `allow`, `warn` — behavior for undeclared properties |

## Index Types

| Index Type | Uniqueness | Equality Lookups | Range Operators | Typical Use |
|---|---|---|---|---|
| `none` | No | No | No | Rarely-filtered fields |
| `unique` | Yes | Yes | No | Business keys (`email`, `sku`) |
| `equality` | No | Yes | No | Status/category filters |
| `range` | No | Yes | Yes | Numeric/time ranges |

### Auto-Indexing Defaults

Schema registration requires `index_default_mode = INDEX_DEFAULT_MODE_AUTO_BY_TYPE_V1`.

With `AUTO_BY_TYPE_V1`, omitted property index values are inferred:

| Property Type | Inferred Index |
|---|---|
| `int`, `float`, `timestamp` | `range` |
| `bool` | `equality` |
| `string`, `bytes` | `none` |

Explicit index values always take precedence. Use `"index": "none"` to opt out of inferred indexing.

### Index Maintenance

- Null or missing values do not create index entries.
- Adding a new indexed property to an existing type triggers an `IndexBackfill` background job.

## Validation Rules

Server-side schema validation enforces:
- Schema `name` is non-empty and matches `[A-Za-z0-9_]`
- Property names are non-empty
- Edge names are non-empty
- `sort_key` must reference an existing edge property
- `ownership=independent` is rejected in v1

Node validation on write:
- Required fields must exist and be non-null
- Value types must match schema property types
- Extra properties follow `extras_policy` (`reject`, `warn`, `allow`)
- Defaults are applied before validation on create
- Edge property payloads are stored as-is (runtime does not enforce `edge.properties` type/required constraints on write)

Practical note: For `unique` fields, prefer omitting absent values instead of explicitly sending `null`.

## Registration Surfaces

### gRPC / Protobuf API (full support)

Use `SchemaService.RegisterSchema` with full protobuf fields for edge definitions, edge property schemas, default values, sort keys, and full metadata policy.

**Breaking contract:** `RegisterSchemaRequest.index_default_mode` is required. Legacy writers that omit it are rejected with `INVALID_ARGUMENT`.

```json
{
  "name": "Person",
  "properties": {
    "email": { "type": "PROPERTY_TYPE_STRING", "required": true, "index": "INDEX_TYPE_UNIQUE" },
    "age": { "type": "PROPERTY_TYPE_INT", "index": "INDEX_TYPE_RANGE" },
    "active": { "type": "PROPERTY_TYPE_BOOL", "default_value": { "bool_value": true } }
  },
  "edges": {
    "KNOWS": {
      "target": { "specific_type": "Person" },
      "direction": "EDGE_DIRECTION_DEF_BIDIRECTIONAL",
      "properties": { "since": { "type": "PROPERTY_TYPE_TIMESTAMP" } },
      "sort_key": "since",
      "ownership": "OWNERSHIP_MODE_SOURCE_SITE"
    }
  },
  "meta": { "allow_undeclared_edges": false, "extras_policy": "EXTRAS_POLICY_REJECT" }
}
```

### CLI (JSON, Protobuf JSON, SQL-like DDL)

`pelago schema register` accepts compact JSON, protobuf-style JSON, and SQL-like DDL.

Compact JSON:
```json
{
  "name": "Person",
  "properties": {
    "name": { "type": "string", "required": true },
    "age": { "type": "int", "index": "range" }
  },
  "meta": { "allow_undeclared_edges": false, "extras_policy": "reject" }
}
```

SQL-like DDL:
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

### Python SDK Helper

`register_schema_dict` is convenient but narrower than raw protobuf:
- Supports node property type/required/index/default
- Supports edge target/direction/sort_key/ownership
- Does not currently populate `edges.<label>.properties` from dict input

For edge property schema + sort key validation, use the raw protobuf registration path.

## Schema Evolution

- First registration is version `1`.
- Every subsequent registration of same `name` increments version.
- Historical versions remain queryable.

```bash
pelago schema get Person --version 2
pelago schema diff Person 1 2
pelago admin job list    # monitor backfill jobs
```

## Related

- [Schema System](../concepts/schema-system.md) — conceptual overview
- [Schema Design Strategy](../guides/schema-design-strategy.md) — best practices
- [Data Types](data-types.md) — value types and encoding
- [CLI Reference](cli.md) — schema commands
