# Data Types Reference

PelagoDB's value types, encoding behavior, null semantics, and index interactions.

## Value Types

PelagoDB supports 6 value types plus null:

| Type | Proto Field | Storage | Description |
|---|---|---|---|
| `string` | `string_value` | UTF-8 bytes | Text values |
| `int` | `int_value` | 64-bit signed (`int64`) | Integer values |
| `float` | `float_value` | 64-bit double | Floating-point values |
| `bool` | `bool_value` | Single byte | Boolean true/false |
| `timestamp` | `timestamp_value` | 64-bit signed (`int64`) | Unix microseconds |
| `bytes` | `bytes_value` | Raw bytes | Arbitrary binary data |
| `null` | `null_value` | No storage | Absence of value |

## Encoding

Values are encoded for storage in FoundationDB using sort-order-preserving encoding:

- **Integers** use big-endian encoding with sign bit flipped for correct sort order
- **Floats** use IEEE 754 encoding with sign/exponent manipulation for sort order
- **Strings** use null-terminated UTF-8 encoding
- **Timestamps** follow integer encoding (they are `int64` microseconds)
- **Booleans** encode as single byte (`0x00` or `0x01`)
- **Bytes** use escaped null encoding for sort-order preservation

This encoding ensures that range indexes produce correctly ordered results without additional sorting.

## Null Semantics

- `null` matches any property type for optional (non-required) fields
- `required: true` fields cannot be `null`
- `null` values do not create index entries
- For `unique` indexed fields, prefer omitting absent values instead of explicitly sending `null` (null unique values can fail during encoding)

## Type Matching Rules

On node create/update, property values are validated against schema types:

| Schema Type | Accepted Values |
|---|---|
| `string` | String value or null |
| `int` | Integer value or null |
| `float` | Float value or null |
| `bool` | Boolean value or null |
| `timestamp` | Integer value (microseconds) or null |
| `bytes` | Bytes value or null |

Type mismatches produce a validation error.

## Index Behavior by Type

| Property Type | Auto-Index (when omitted) | Null Indexed? |
|---|---|---|
| `string` | `none` | No |
| `int` | `range` | No |
| `float` | `range` | No |
| `bool` | `equality` | No |
| `timestamp` | `range` | No |
| `bytes` | `none` | No |

## Timestamp Convention

Timestamps are Unix microseconds stored as `int64`:

```
1738368000000000 = 2025-02-01T00:00:00 UTC (in microseconds)
```

Timestamps are **not** ISO strings. Client-side conversion is required for human-readable display.

## Default Values

Default values in schema definitions are applied on node create when the property is absent:

```json
{
  "active": {"type": "bool", "default": true},
  "priority": {"type": "int", "default": 0}
}
```

Defaults are applied before validation, so a required field with a default does not need to be explicitly provided.

## Related

- [Schema Specification](schema-spec.md) — property and type definitions
- [CEL Filters](cel-filters.md) — type usage in expressions
- [Data Model](../concepts/data-model.md) — nodes, edges, properties
