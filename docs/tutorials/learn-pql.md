# Tutorial: Learn PQL

Learn PelagoDB's graph query language from short form through advanced features with progressive exercises.

## Prerequisites

- PelagoDB server running with data loaded (see [Build a Social Graph](build-a-social-graph.md))

## Part 1: Short Form

The simplest PQL query filters a single entity type:

```bash
pelago query pql --query 'Person @filter(age >= 30) { uid name age }'
```

This is equivalent to `type(Person)` with a filter. The short form is great for quick ad-hoc queries.

### Selecting Fields

List the properties you want returned:

```bash
pelago query pql --query 'Person { uid name email city }'
```

### Adding Filters

Use `@filter` with comparison operators:

```bash
pelago query pql --query 'Person @filter(city == "Portland") { uid name city }'
```

### Limiting Results

```bash
pelago query pql --query 'Person @filter(age >= 25) @limit(first: 10) { uid name age }'
```

With offset:

```bash
pelago query pql --query 'Person @filter(age >= 25) @limit(first: 10, offset: 5) { uid name age }'
```

## Part 2: Full Form

Full form enables named queries, multiple blocks, and root functions.

### Point Lookup

```bash
pelago query pql --query '
query {
  person(func: uid(Person:1_0)) {
    uid name email age
  }
}'
```

### Edge Traversal

Walk relationships from a node:

```bash
pelago query pql --query '
query {
  seed(func: uid(Person:1_0)) {
    name
    -[FOLLOWS]-> { uid name city }
  }
}'
```

### Filtered Traversal

Filter the target nodes of a traversal:

```bash
pelago query pql --query '
query {
  seed(func: uid(Person:1_0)) {
    name
    -[FOLLOWS]-> @filter(age >= 30) { uid name age }
  }
}'
```

## Part 3: Variables and Set Operations

### Variable Capture

Capture results from one block and use them in another:

```bash
pelago query pql --query '
query {
  friends as seed(func: uid(Person:1_0)) {
    -[FOLLOWS]-> { uid name }
  }
  friend_details(func: uid(friends)) @filter(active == true) {
    uid name city age
  }
}'
```

### Set Operations

Combine variable sets:

```bash
pelago query pql --query '
query {
  portland as p(func: type(Person)) @filter(city == "Portland") @limit(first: 100) {
    uid name city
  }
  seniors as s(func: type(Person)) @filter(age >= 30) @limit(first: 100) {
    uid name age
  }
  portland_seniors(func: uid(portland, seniors, intersect)) {
    uid name city age
  }
}'
```

| Syntax | Operation |
|---|---|
| `uid(a, b)` | Union |
| `uid(a, b, intersect)` | Intersection |
| `uid(a, b, difference)` | Subtraction (a - b) |

## Part 4: Parameters

Use `$name` parameters for reusable queries:

```bash
pelago query pql --query 'Person @filter(age >= $min_age) { uid name age }'
```

Parameters are substituted server-side via `ExecutePQLRequest.params`. In the REPL, use `:param` for client-side substitution.

## Part 5: Explain Plans

Check how your PQL query compiles:

```bash
pelago query pql --query 'Person @filter(age >= 30) { uid name age }' --explain
```

In the REPL:
```
:explain Person @filter(age >= 30) { uid name age }
```

## Part 6: Practical Patterns

### Segmentation with Set Algebra

Build complex audiences without deep traversals:

```bash
pelago query pql --query '
query staff_segmentation {
  sf as sf(func: type(Person)) @filter(city == "San Francisco") @limit(first: 5000) {
    name city
  }
  senior as senior(func: type(Person)) @filter(age >= 35) @limit(first: 5000) {
    name age
  }
  target(func: uid(sf, senior, intersect)) @limit(first: 200) {
    uid name city age
  }
}'
```

### Recursive Traversal

For `uid(Type:id)` blocks with edge traversals:

```bash
pelago query pql --query '
query {
  seed(func: uid(Person:1_0)) @recurse(depth: 3) {
    name
    -[FOLLOWS]-> { name }
  }
}'
```

## Current Limitations

- `ExecutePQL` does not provide cursor-based pagination. Use `FindNodes`/`Traverse` for paginated endpoints.
- Aggregate functions (`count`, `sum`) are parsed but not materialized.
- Edge-level `@sort` and `@facets` are captured in plans but not applied at runtime.

## Exercises

1. **Short form:** Find all people in Seattle with age > 25.
2. **Traversal:** From a known person, find who they follow who is active.
3. **Variables:** Capture followers of person A and person B, then find the intersection.
4. **Explain:** Run an explain on exercise 1 and identify the execution plan.
5. **Set algebra:** Find people who are in Portland AND over 30 AND active, using three variable captures.

## Related

- [PQL Reference](../reference/pql.md) — complete grammar and directives
- [CEL Filters Reference](../reference/cel-filters.md) — property filtering
- [Query Model](../concepts/query-model.md) — when to use CEL vs PQL
- [Query Optimization](../guides/query-optimization.md) — tuning strategies
