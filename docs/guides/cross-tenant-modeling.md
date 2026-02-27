# Cross-Tenant Modeling

Patterns and costs for modeling relationships that cross tenant boundaries in PelagoDB.

## When Cross-Tenant Edges Make Sense

PelagoDB supports cross-tenant relationships as a first-class capability. Use them when:

- Entities in different tenants have real business relationships
- Global reference data (vendors, controls, regions) is shared across tenants
- Cross-organizational analysis requires unified graph traversal

## Common Patterns

### Shared Reference Entities

Keep global reference data in a shared namespace:

```
Namespace: asset/global
  Vendor, Control, GeoRegion

Namespace: asset/tenant.acme
  Asset, WorkOrder, Incident
```

Cross-namespace edges:
- `Asset(tenant.acme) → depends_on → Vendor(global)`
- `Incident(tenant.acme) → linked_to → Control(global)`

### Cross-Tenant Intelligence

When analysis requires cross-boundary visibility:

- `Asset(tenant-a) → shares_ioc_with → Asset(tenant-b)`
- `Person(tenant-a) → collaborates_with → Person(tenant-b)`

## Costs to Evaluate

### Fanout Impact

Cross-namespace edges require lookups across namespace boundaries. For each cross-namespace edge in a traversal:

1. Resolve the target node's namespace
2. Read from the target namespace
3. Apply authorization checks for the target namespace

High-fanout cross-namespace edges multiply this cost.

### Replication Pressure

Cross-namespace edges create replication dependencies between namespaces. If namespace A has edges pointing into namespace B, replication of namespace B must also be configured at each site that needs to resolve those edges.

### Authorization Complexity

Cross-tenant edges may require the requesting principal to have read access in both the source and target namespaces. Design policies accordingly.

## Best Practices

1. **Be deliberate:** Review every cross-tenant edge type for fanout/latency impact before deploying
2. **Keep reference data in shared namespaces:** Don't duplicate global entities per tenant
3. **Bound traversals:** Set strict `max_depth` and `max_results` on traversals that may cross boundaries
4. **Monitor:** Track traversal latency for cross-namespace query paths
5. **Authorization:** Ensure policies cover cross-namespace read access

## Anti-Patterns

| Anti-Pattern | Problem | Better Approach |
|---|---|---|
| One namespace for all tenants | No isolation, unbounded fanout | Separate tenant namespaces |
| Heavy cross-namespace edges in hot paths | Latency blowup | Cache or denormalize for critical paths |
| Unreviewed cross-tenant edge creation | Security and performance risks | Gate cross-tenant edges through policy review |

## Related

- [Namespaces and Tenancy](../concepts/namespaces-and-tenancy.md) — partitioning concepts
- [Data Modeling Patterns](data-modeling-patterns.md) — general modeling guidance
- [Query Optimization](query-optimization.md) — traversal tuning
- [Replication Concepts](../concepts/replication.md) — replication scope implications
