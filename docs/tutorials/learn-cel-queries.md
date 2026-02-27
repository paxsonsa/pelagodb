# Tutorial: Learn CEL Queries

Learn PelagoDB's CEL (Common Expression Language) filtering system with progressive exercises.

## Prerequisites

- PelagoDB server running with some data loaded
- See [Build a Social Graph](build-a-social-graph.md) to set up sample data, or load a bundled dataset:
  ```bash
  python datasets/load_dataset.py social_graph --namespace demo
  ```

## What is CEL?

CEL is used for property-based filtering in `FindNodes` queries. It's a simple, expressive language for writing predicates over node properties.

## Basic Comparisons

### Equality

```bash
pelago query find Person --filter 'city == "Portland"'
```

### Inequality

```bash
pelago query find Person --filter 'city != "Seattle"'
```

### Numeric Comparisons

```bash
pelago query find Person --filter 'age >= 30'
pelago query find Person --filter 'age < 25'
pelago query find Person --filter 'age > 20 && age <= 35'
```

## Boolean Logic

### AND (conjunction)

```bash
pelago query find Person --filter 'city == "Portland" && age >= 30'
```

### OR (disjunction)

```bash
pelago query find Person --filter 'city == "Portland" || city == "Seattle"'
```

### Combined

```bash
pelago query find Person --filter '(city == "Portland" || city == "Seattle") && age >= 25'
```

## String Operations

### Equality

```bash
pelago query find Person --filter 'email == "alice@example.com"'
```

### Boolean fields

```bash
pelago query find Person --filter 'active == true'
pelago query find Person --filter 'active == false'
```

## Working with Limits and Pagination

Always set limits for production queries:

```bash
pelago query find Person --filter 'age >= 25' --limit 10
```

For cursor-based pagination, use the gRPC API with `next_cursor`. See [gRPC API](../reference/grpc-api.md#pagination-and-cursors).

## Using Explain Plans

Before deploying a query to production, check its execution plan:

```bash
grpcurl -plaintext \
  -d '{
    "context": {"database":"default","namespace":"default"},
    "entity_type": "Person",
    "cel_expression": "age >= 30 && active == true"
  }' \
  127.0.0.1:27615 pelago.v1.QueryService/Explain
```

Look for:
- `index_scan` — good, using an index
- `full_scan` — expensive, consider adding an index

## Index-Aware Query Writing

CEL queries perform best when they align with indexes:

| Expression | Index Used | Performance |
|---|---|---|
| `email == "alice@example.com"` | `unique` on email | Fast point lookup |
| `city == "Portland"` | `equality` on city | Fast equality scan |
| `age >= 30` | `range` on age | Fast range scan |
| `age >= 30 && city == "Portland"` | Best available per field | Efficient conjunction |
| `age >= 30 \|\| city == "Portland"` | May use term-posting | Check with Explain |

### Term-Posting Fast Path

Simple equality expressions with `&&` and `||` can use the term-posting acceleration path:

```bash
# Fast path ✓
pelago query find Person --filter 'status == "active" && role == "admin"'

# Fast path ✓
pelago query find Person --filter 'status == "active" || status == "pending"'

# May not use fast path — complex boolean grouping
pelago query find Person --filter '(status == "active" || status == "pending") && age >= 30'
```

## Exercises

1. **Basic filter:** Find all people in Seattle.
2. **Range query:** Find people between ages 25 and 35 (inclusive).
3. **Combined:** Find active people in Portland over 30.
4. **Explain:** Run an explain plan on exercise 3 and identify which indexes are used.
5. **Pagination:** Use `grpcurl` to paginate through results with `limit: 2` and `cursor`.

## Related

- [CEL Filters Reference](../reference/cel-filters.md) — complete syntax
- [Query Model](../concepts/query-model.md) — CEL vs PQL, when to use which
- [Query Optimization](../guides/query-optimization.md) — tuning strategies
- [Learn PQL](learn-pql.md) — graph-native queries
