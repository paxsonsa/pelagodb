# Schema System

PelagoDB is schema-first: every entity type must be defined before data can be written.

## Why Schema-First?

- **Write-time validation:** Catch data shape errors before they're persisted
- **Index guarantees:** Schemas declare which properties are indexed, enabling query planning
- **Team coordination:** Schemas serve as contracts between producers and consumers
- **Evolution tracking:** Versioned schemas make change management explicit

## Schema Components

### Entity Schema

Defines an entity type's structure:

```json
{
  "name": "Person",
  "properties": {
    "name": {"type": "string", "required": true},
    "email": {"type": "string", "index": "unique"},
    "age": {"type": "int", "index": "range"},
    "active": {"type": "bool", "default": true}
  },
  "edges": {
    "KNOWS": {
      "target": "Person",
      "direction": "bidirectional"
    }
  },
  "meta": {
    "allow_undeclared_edges": false,
    "extras_policy": "reject"
  }
}
```

### PropertyDef

Each property declares:
- **Type:** `string`, `int`, `float`, `bool`, `timestamp`, `bytes`
- **Required:** Whether the field must be present and non-null
- **Index:** `none`, `unique`, `equality`, `range`
- **Default:** Value applied on create when property is absent

### EdgeDef

Each edge declaration specifies:
- **Target:** Specific entity type or polymorphic (`*`)
- **Direction:** `outgoing` (one-way) or `bidirectional` (both directions)
- **Properties:** Optional typed properties on the edge itself
- **Sort key:** Property used for ordered edge traversal
- **Ownership:** `source_site` (default) or `independent`

### SchemaMeta

Controls validation behavior:
- **`allow_undeclared_edges`:** Accept edge labels not declared in the schema
- **`extras_policy`:** What to do with properties not declared in the schema
  - `reject` — fail the write
  - `warn` — accept with warning
  - `allow` — accept silently

## Validation Flow

On every node create/update:

1. Look up the entity's schema
2. Apply default values (create only)
3. Check required fields are present and non-null
4. Verify property types match declarations
5. Apply extras policy for undeclared properties
6. Enforce unique constraints
7. Write to FoundationDB with index entries

## Auto-Indexing

Schema registration requires `INDEX_DEFAULT_MODE_AUTO_BY_TYPE_V1`. When a property's index is omitted:

| Property Type | Inferred Index |
|---|---|
| `int`, `float`, `timestamp` | `range` |
| `bool` | `equality` |
| `string`, `bytes` | `none` |

Explicit `"index": "none"` overrides inference.

## Schema Versioning

- First registration: version `1`
- Each re-registration of the same name: version increments
- Historical versions are queryable
- Schema diff shows changes between versions

```bash
pelago schema get Person --version 2
pelago schema diff Person 1 2
```

## Registration Surfaces

| Surface | Formats |
|---|---|
| gRPC API | Full protobuf (all features) |
| CLI | Compact JSON, protobuf JSON, SQL-like DDL |
| Python SDK | Dict helper (narrower feature set) |
| REPL | SQL-like DDL |

## Related

- [Schema Specification](../reference/schema-spec.md) — full format reference
- [Schema Design Strategy](../guides/schema-design-strategy.md) — best practices
- [Data Model](data-model.md) — nodes, edges, properties
- [Core Workflow](../getting-started/core-workflow.md) — schema → node → edge → query
