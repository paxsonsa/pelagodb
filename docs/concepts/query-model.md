# Query Model

PelagoDB offers two query surfaces — CEL and PQL — plus direct traversal, each suited to different use cases.

## Query Surfaces

### CEL Filters (via FindNodes)

Common Expression Language filters for property-based retrieval.

```bash
pelago query find Person --filter 'age >= 30 && active == true' --limit 100
```

Best for:
- Property filtering with indexed predicates
- Paginated lists
- Simple equality/range lookups
- User-facing endpoints needing cursor-based pagination

### Traversal

Graph path exploration from a starting node.

```bash
pelago query traverse Person:1_0 follows --max-depth 2 --max-results 200
```

Best for:
- One-hop and multi-hop relationship exploration
- Bounded graph path queries
- Finding connected nodes

### PQL (Pelago Query Language)

Graph-native query language supporting multi-block queries, variable captures, and set operations.

```bash
pelago query pql --query 'Person @filter(age >= 30) { uid name age }'
```

Best for:
- Multi-stage graph selection with variable captures
- Set algebra (union, intersection, difference)
- Complex queries combining multiple blocks
- Internal workflows and exploratory queries

## When to Use Which

| Need | Use | Why |
|---|---|---|
| Paginated property filtering | CEL `FindNodes` | Best index utilization, cursor pagination |
| Graph path from known node | `Traverse` | Purpose-built with depth/result bounds |
| Multi-stage set operations | PQL | Variable captures and set algebra |
| Quick ad-hoc exploration | PQL (REPL) | Expressive short form |
| Production paginated endpoints | CEL or `Traverse` | Continuation cursor support |
| Explain plan analysis | CEL `Explain` or PQL `--explain` | Plan visibility before rollout |

## Execution Pipeline

All query surfaces follow a similar pipeline:

```
Parse → Schema Resolution → Compile → Execute → Stream Results
```

- **Parse:** Validate syntax (CEL expression, PQL grammar, traversal parameters)
- **Schema Resolution:** Resolve entity types and properties against registered schemas
- **Compile:** Generate execution plan (index selection, traversal strategy)
- **Execute:** Run against FoundationDB (and optionally cache)
- **Stream:** Return results with continuation support

## Consistency Controls

Per-request controls apply to all surfaces:

| Control | Options |
|---|---|
| `ReadConsistency` | `STRONG`, `SESSION`, `EVENTUAL` |
| `SnapshotMode` | `STRICT`, `BEST_EFFORT` |

## Current Limitations

- `ExecutePQL` does not provide paginated continuation cursors. Use `FindNodes`/`Traverse` for paginated user-facing endpoints.
- PQL aggregate materialization is parsed but not yet executed.
- Edge-level sort/facets in PQL traversals are captured in plans but not applied at runtime.

## Related

- [CEL Filters Reference](../reference/cel-filters.md) — expression syntax
- [PQL Reference](../reference/pql.md) — grammar and directives
- [gRPC API Reference](../reference/grpc-api.md) — query endpoints
- [Query Optimization](../guides/query-optimization.md) — tuning strategies
- [Learn CEL Tutorial](../tutorials/learn-cel-queries.md) — hands-on CEL
- [Learn PQL Tutorial](../tutorials/learn-pql.md) — hands-on PQL
