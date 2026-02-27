# CEL Filters Reference

CEL (Common Expression Language) is used for property-based filtering in `FindNodes`, `Traverse` node filters, and `WatchQuery` subscriptions.

## Operators

### Comparison

| Operator | Description | Example |
|---|---|---|
| `==` | Equal | `city == "Portland"` |
| `!=` | Not equal | `status != "archived"` |
| `>` | Greater than | `age > 30` |
| `>=` | Greater than or equal | `age >= 30` |
| `<` | Less than | `priority < 5` |
| `<=` | Less than or equal | `score <= 100` |

### Logical

| Operator | Description | Example |
|---|---|---|
| `&&` | Logical AND | `age >= 30 && active == true` |
| `\|\|` | Logical OR | `city == "Portland" \|\| city == "Seattle"` |

### Grouping

Parentheses for precedence:

```cel
(city == "Portland" || city == "Seattle") && age >= 25
```

## Property Types in Expressions

| Type | Literal Syntax | Example |
|---|---|---|
| `string` | Double-quoted | `name == "Alice"` |
| `int` | Numeric | `age >= 30` |
| `float` | Decimal | `score >= 4.5` |
| `bool` | `true` / `false` | `active == true` |
| `timestamp` | Integer (Unix microseconds) | `created_at >= 1738368000000000` |

## Null Semantics

- Properties that are `null` or absent do not match comparison operators
- To check for existence, compare against a known value
- `null` values do not create index entries, so they are invisible to index scans

## Index Interaction

CEL expressions are analyzed by the query planner for index utilization:

| Expression Pattern | Index Path |
|---|---|
| `field == value` | `unique` or `equality` index lookup |
| `field >= value` (or `>`, `<`, `<=`) | `range` index scan |
| `field == a && field2 >= b` | Conjunction uses best available per field |
| Simple `==` with `&&` / `\|\|` | Term-posting fast path |
| Complex boolean with parentheses | May fall back to scan |

### Term-Posting Fast Path

The query executor accelerates simple equality boolean expressions:

```cel
# Fast path ✓
status == "active"
status == "active" && role == "admin"
status == "active" || status == "pending"

# Not fast path — parenthesized grouping
(status == "active" || status == "pending") && age >= 30
```

## Usage in Different Contexts

### FindNodes

```bash
pelago query find Person --filter 'age >= 30 && city == "Portland"' --limit 100
```

### Traverse Node Filter

```json
{
  "steps": [{
    "edge_type": "follows",
    "direction": "EDGE_DIRECTION_OUTGOING",
    "node_filter": "active == true"
  }]
}
```

### WatchQuery

```json
{
  "entity_type": "Person",
  "cel_expression": "age >= 30 && active == true"
}
```

## Explain Plans

Always verify index usage before production:

```bash
grpcurl -plaintext \
  -d '{
    "context": {"database":"default","namespace":"default"},
    "entity_type": "Person",
    "cel_expression": "age >= 30 && active == true"
  }' \
  127.0.0.1:27615 pelago.v1.QueryService/Explain
```

## Best Practices

- Keep expressions simple for predictable index planning
- Use conjunction (`&&`) of indexed fields for best performance
- Set `limit` on all queries
- Check `Explain` output before deploying new filter patterns
- Avoid complex nested boolean groupings in latency-critical paths

## Related

- [Query Model](../concepts/query-model.md) — CEL vs PQL
- [PQL Reference](pql.md) — graph query language
- [Query Optimization](../guides/query-optimization.md) — tuning strategies
- [Learn CEL Tutorial](../tutorials/learn-cel-queries.md) — hands-on guide
