# Data Modeling Patterns

Best practices, anti-patterns, and worked examples for modeling data in PelagoDB.

## Why Modeling Is Operational

In PelagoDB, modeling choices are not just schema choices:
- They determine write authority (ownership)
- They influence replication lag and conflict rates
- They shape read/query fanout and cache efficiency
- They control tenant isolation under load
- They determine where cross-tenant relationships are allowed and how expensive they become

Treat data modeling as part of system architecture, not only API design.

## Model From Access Patterns First

Start from your top read/write paths, then shape schema and edges to fit them.

| Workload | Model Shape | Why It Performs |
|---|---|---|
| Lookup by external/business ID | Property with `index: unique` | Fast indexed lookup, deterministic uniqueness |
| Filtered lists by state + time window | `state` on `equality` + `created_at` on `range` | Narrows candidate set before residual filtering |
| High-fanout parent lists | Bucket/hub nodes (e.g., `Customer → OrderMonth → Order`) | Prevents supernode fanout blowups |
| Deep workflow path queries | Explicit hierarchy with typed edges | Keeps traversals intentional and bounded |
| Dynamic audience/segment selection | Indexed dimensions + PQL set algebra | Efficient multi-stage narrowing |

## Entity and Edge Boundaries

- Keep entities cohesive — avoid giant polymorphic objects
- Model many-to-many with explicit edge types
- Index only query-critical properties
- Use sort keys on high-volume edges when traversal order matters

## Index Design Patterns

### Pattern A: Direct Key + Operational Filters

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

### Pattern B: Synthetic Composite Key

When true composite indexes are unavailable:

```json
{
  "name": "Task",
  "properties": {
    "task_code": { "type": "string", "required": true, "index": "unique" },
    "status": { "type": "string", "index": "equality" },
    "status_day_key": { "type": "string", "index": "equality" }
  }
}
```

`status_day_key` is application-maintained (e.g., `review:20260227`) to reduce costly two-filter scans.

## Fanout and Supernode Mitigation

**Anti-pattern:** `Customer -[HAS_ORDER]-> Order` with millions of outgoing edges per customer.

**Preferred:** `Customer -[HAS_ORDER_MONTH]-> OrderMonth -[HAS_ORDER]-> Order`

Benefits:
- Smaller per-hop edge scans
- Natural pagination boundaries
- Easier caching and archiving by bucket period

## Hot/Cold Entity Split

Separate frequently changing fields from mostly static ones:

- `Task` — identity, static metadata, relationship anchors
- `TaskState` — status, assignee, updated timestamp, SLA fields

Benefits:
- Lower write amplification on indexed immutable attributes
- Cleaner retention and audit policies for mutable state

## Ownership Strategy

- Pick a canonical owner for mutable entities
- Avoid ownership ambiguity for frequently updated records
- Owner-aligned writes reduce conflicts
- Use ownership transfer for planned handoff workflows

## Replication-Aware Modeling

- Keep mutation-heavy entities in namespaces with clear ownership
- Keep replication domains scoped (`database + namespace`)
- Favor locality-aligned mutations to minimize conflict/audit noise
- Monitor `replication.conflict` as a modeling signal

## Do's and Don'ts

| Do | Don't | Why |
|---|---|---|
| Define a stable unique business key per entity | Depend only on generated node IDs | Query paths become slower and harder to make deterministic |
| Keep edge labels uppercase for PQL | Use lowercase edge labels | PQL grammar expects uppercase-first edge labels |
| Add indexes only for real production filters | Index every property | Write amplification grows quickly |
| Omit absent values for unique-indexed fields | Send explicit `null` for unique fields | Null unique values can fail during encoding |
| Keep CEL filters simple | Assume complex booleans always get indexed plans | Planner is most reliable on simple conjunctions |
| Bound graph reads with depth/result/timeout | Run unbounded traversals | Unbounded expansion causes p99 instability |
| Use `Explain` before rollout | Promote queries without plan visibility | Hidden full scans surface in production |

## Practical Checklist

- [ ] Top 5 read paths and top 3 write paths identified before schema design
- [ ] One primary lookup key per entity (`unique` where needed)
- [ ] Only query-critical indexes added
- [ ] Potential supernodes identified and mitigated with bucket/hub entities
- [ ] Namespace/ownership boundaries defined for hot data
- [ ] Pagination and timeout strategy defined per endpoint

## Related

- [Schema Design Strategy](schema-design-strategy.md) — index and DDL strategies
- [Query Optimization](query-optimization.md) — tuning reads
- [Cross-Tenant Modeling](cross-tenant-modeling.md) — cross-boundary patterns
- [Namespaces and Tenancy](../concepts/namespaces-and-tenancy.md) — partitioning concepts
- [Schema Specification](../reference/schema-spec.md) — format reference
