# Data Model

How PelagoDB represents data: nodes, edges, properties, identifiers, and value types.

## Nodes

A node is an instance of a schema-defined entity type. Every node has:

| Field | Description |
|---|---|
| `entity_type` | The schema type name (e.g., `Person`) |
| `node_id` | Site-aware identifier (e.g., `1_0`) |
| `properties` | Key-value map matching the schema |
| `ownership` | Which site has write authority |
| `created_at` | Unix microsecond timestamp |
| `updated_at` | Unix microsecond timestamp |

### Node IDs

Node IDs encode the originating site:

```
<site_id>_<sequence>
```

- `1_0` — site 1, sequence 0
- `2_42` — site 2, sequence 42

IDs are allocated in batches per site (`PELAGO_ID_BATCH_SIZE`), ensuring uniqueness without cross-site coordination.

## Edges

An edge connects two nodes with a labeled, directed relationship.

| Field | Description |
|---|---|
| Source | `entity_type:node_id` of the originating node |
| Label | Edge type name (e.g., `follows`, `HAS_ORDER`) |
| Target | `entity_type:node_id` of the destination node |
| Direction | `outgoing` or `bidirectional` |
| Properties | Optional key-value map (if schema defines edge properties) |
| Sort key | Optional ordering field for edge traversal |

### Edge Direction

- **Outgoing:** A → B. Only forward traversal keys are written.
- **Bidirectional:** A ↔ B. Reverse-direction keys are automatically written, enabling traversal from either end.

### Edge Labels

PQL grammar expects edge labels to be uppercase-first (`[A-Z][A-Za-z0-9_]*`). For consistent cross-surface behavior, use uppercase labels like `KNOWS`, `HAS_ORDER`, `WORKS_AT`.

## Properties

Properties are typed key-value pairs defined by the schema.

### Value Types

PelagoDB supports 6 value types plus null:

| Type | Proto Field | Description |
|---|---|---|
| `string` | `string_value` | UTF-8 text |
| `int` | `int_value` | 64-bit signed integer |
| `float` | `float_value` | 64-bit double |
| `bool` | `bool_value` | Boolean |
| `timestamp` | `timestamp_value` | Unix microseconds (`int64`) |
| `bytes` | `bytes_value` | Arbitrary byte sequence |
| `null` | `null_value` | Absence of value |

### Type Rules

- `null` matches any property type for optional fields.
- `required: true` fields cannot be `null`.
- Timestamps are integer microseconds, not ISO strings.
- For `unique` indexed fields, prefer omitting absent values instead of sending explicit `null`.

## Schema-First Enforcement

Every node must conform to its schema:
- Required fields must be present and non-null
- Value types must match declared property types
- Extra properties follow the schema's `extras_policy` (`reject`, `warn`, `allow`)
- Default values are applied before validation on create

## Namespaces and Databases

Data is scoped by:
- **Database:** Top-level container (default: `default`)
- **Namespace:** Operational partition within a database (default: `default`)

Every request includes a database/namespace context that determines which data is accessed.

## Ownership

Each node has an owning site. Ownership governs:
- Which site can mutate the node directly
- Conflict resolution during replication (owner-wins)
- Edge ownership follows source-site by default

## Related

- [Schema System](schema-system.md) — schema definitions and validation
- [Data Types Reference](../reference/data-types.md) — encoding and null semantics
- [Core Workflow](../getting-started/core-workflow.md) — the lifecycle in practice
- [Namespaces and Tenancy](namespaces-and-tenancy.md) — partitioning
