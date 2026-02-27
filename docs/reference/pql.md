# PQL Reference

PQL (Pelago Query Language) is PelagoDB's graph-oriented query language for multi-block selections, traversals, set operations, and variable captures.

Source of truth: `crates/pelago-query/src/pql/grammar.pest`, `crates/pelago-query/src/pql/compiler.rs`

## Query Forms

### Short Form

```pql
Person @filter(age >= 30) @limit(first: 50) { name, age }
```

Short form is equivalent to a single block rooted at `type(Person)`.

### Full Form

```pql
query friends {
  seed(func: uid(Person:1_42)) {
    name
    -[KNOWS]-> @filter(age >= 30) { name, age }
  }
}
```

## Core Grammar

- Comments start with `#`
- Identifiers: `[A-Za-z][A-Za-z0-9_]*`
- Node IDs in `uid(Type:id)`: `[A-Za-z0-9_-]+`
- Edge labels: uppercase-first (`[A-Z][A-Za-z0-9_]*`)

Recommendation: Use alphanumeric + underscore naming for types/properties/edge labels to keep schema, CEL, and PQL compatibility predictable.

## Root Functions

| Root Function | Parsed | Executed |
|---|---|---|
| `uid(Type:id)` | Yes | Yes |
| `uid(var)` | Yes | Yes |
| `uid(a, b, ...)` | Yes | Yes (union default; `intersect`/`difference` supported) |
| `type(Type)` | Yes | Yes |
| `eq/ge/le/gt/lt/between/has/allofterms` | Yes | Not executable in `ExecutePQL` blocks |

`upsert` blocks are explicitly rejected at parse time.

## Selections

| Kind | Description | Runtime Status |
|---|---|---|
| Property field | `name`, `age` | Executed |
| Edge traversal | `-[KNOWS]-> { ... }` | Executed as traversal steps |
| Aggregate | `count(...)`, `sum(...)` | Parsed, not materialized |
| Value variable | `x as count(...)` | Parsed, not materialized |

## Block-Level Directives

| Directive | Parsed | Executed |
|---|---|---|
| `@filter(expr)` | Yes | Yes (for `type`, `uid(var)`, `uid(set)`, short-form blocks) |
| `@limit(first:N[, offset:M])` | Yes | Yes |
| `@cascade` | Yes | Yes (only for `uid(Type:id)` traversal blocks) |
| `@recurse(depth:N)` | Yes | Yes (only for `uid(Type:id)` traversal blocks) |
| `@sort(...)` | Yes | Compile error at block level |
| `@groupby(...)` | Yes | Compile error |
| `@explain` | Yes | Compile error (use request-level `explain=true`) |

For plain point lookups (`uid(Type:id)`) without edge traversal selections, block directives are currently unsupported.

## Edge-Level Directives

| Directive | Parsed | Compiled | Runtime Effect |
|---|---|---|---|
| `@edge(expr)` | Yes | Yes | Applied as edge filter |
| `@filter(expr)` | Yes | Yes | Applied as target-node filter |
| `@limit(first:N[, offset:M])` | Yes | Yes | Captured; per-node fanout limit not enforced |
| `@sort(field:asc\|desc[, on_edge:true\|false])` | Yes | Yes | Captured in plan; not applied in execution |
| `@facets(a,b,...)` | Yes | Yes | Captured in plan; not emitted as payload |
| `@groupby/@recurse/@cascade/@explain` | Yes | No | Compile error on edge |

## Variables and Set Operations

### Block Capture

```pql
query {
  friends as seed(func: uid(Person:1_42)) {
    -[KNOWS]-> { name }
  }
  mutual(func: uid(friends)) @filter(age >= 25) { name, age }
}
```

### Set Operations

| Syntax | Operation |
|---|---|
| `uid(a, b)` | Union |
| `uid(a, b, intersect)` | Intersection |
| `uid(a, b, difference)` | Subtraction (`a - b`) |

## Parameters

`ExecutePQLRequest.params` supports `$name` replacement before parsing.

```json
{
  "pql": "Person @filter(age >= $min_age) { name, age }",
  "params": { "min_age": "30" }
}
```

Notes:
- Replacement is token-aware in server-side `ExecutePQL`.
- Unresolved `$name` tokens are left unchanged.
- CLI `pelago query pql` has no explicit `--params` flag; REPL supports `:param` substitutions.

## Execution and Result Shape

`ExecutePQL` returns a stream of `PQLResult` rows:

| Field | Notes |
|---|---|
| `block_name` | Which query block this result belongs to |
| `node` | Node payload (commonly populated) |
| `edge` | Edge payload (not currently populated) |
| `explain` | Present when `explain=true` |

Results are emitted block-by-block in dependency order. `ExecutePQL` does not currently provide paginated continuation. For large paginated reads, use `FindNodes` and `Traverse`.

## Current Limitations

These constructs parse but are not fully executed in `ExecutePQL`:

- Root predicate functions (`eq/ge/le/gt/lt/between/has/allofterms`) as standalone query roots
- Nested traversal blocks beyond top-level edge step
- Edge namespace and target type qualifiers in traversal
- Edge-level sort/facets/per-node-limit enforcement
- Aggregate result materialization

## Examples

### Simple Filter

```pql
Person @filter(age >= 30) { uid name age }
```

### Traversal from Known Node

```pql
query {
  seed(func: uid(Person:1_42)) {
    name
    -[KNOWS]-> @filter(age >= 30) { name, age }
  }
}
```

### Set Algebra Segmentation

```pql
query staff_segmentation {
  sf as sf(func: type(Person)) @filter(city == "San Francisco") @limit(first: 5000) {
    name, city, role, seniority
  }
  senior as senior(func: type(Person)) @filter(seniority == "staff") @limit(first: 5000) {
    name, city, role, seniority
  }
  final(func: uid(sf, senior, intersect)) @limit(first: 200) {
    name, city, role, seniority
  }
}
```

## Related

- [Query Model](../concepts/query-model.md) — CEL vs PQL, when to use which
- [CEL Filters](cel-filters.md) — CEL expression syntax
- [Learn PQL Tutorial](../tutorials/learn-pql.md) — hands-on PQL guide
- [Query Optimization Guide](../guides/query-optimization.md) — tuning strategies
