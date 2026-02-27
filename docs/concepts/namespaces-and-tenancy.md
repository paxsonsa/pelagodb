# Namespaces and Tenancy

PelagoDB distinguishes between tenant, namespace, and ownership — three different controls that should not be conflated.

## Core Distinctions

| Concept | Purpose | Scope |
|---|---|---|
| **Tenant** | Business/account boundary — who owns data logically | Application-level |
| **Namespace** | Operational partition for performance, lifecycle, and replication scope | Infrastructure-level |
| **Ownership** | Per-entity write authority for safe mutation under multi-site operation | Entity-level |

## Design Principle

- Tenant isolation is important.
- But global relationship graphs can cross tenant boundaries when your product requires it.
- Write safety is enforced below the namespace level through ownership.

## Namespace Partitioning

Use namespaces to isolate hot workloads while keeping global reference data shared.

Recommended pattern for asset-management workloads:

| Namespace | Contents |
|---|---|
| `asset/global` | Shared reference entities (`Vendor`, `Control`, `GeoRegion`) |
| `asset/tenant.<tenant_id>` | Tenant-owned mutable entities (`Asset`, `WorkOrder`, `Incident`) |
| `asset/ops.<tenant_id>` | Optional high-churn operational streams |

Benefits:
- One hot tenant is less likely to degrade others.
- Replication and cache projector lag are easier to reason about per namespace.
- You preserve a global relationship graph where it matters.

## Namespace Lifecycle

Define these stages for each namespace family:

| Stage | Questions to Answer |
|---|---|
| **Provisioning** | Who creates it, when, with which baseline schemas/policies? |
| **Active operation** | Expected traffic profile, retention rules, SLOs? |
| **Migration/evolution** | How are schema/version changes rolled out safely? |
| **Archive/freeze** | When do writes stop and what read access remains? |
| **Decommission** | Explicit criteria and process for `drop-namespace`? |

## Cross-Tenant Relationships

Cross-tenant edges are explicitly supported but should be deliberate:

Valid examples:
- `Asset(tenant-a) → depends_on → Vendor(global)`
- `Asset(tenant-a) → shares_ioc_with → Asset(tenant-b)`
- `Incident(tenant-a) → linked_to → Control(global)`

Review cross-tenant edges for:
- Fanout and latency impact
- Replication pressure
- Authorization implications

## Ownership Strategy

Ownership governs who can mutate data directly.

Current defaults:
- Node ownership follows locality/site.
- Edge ownership is source-site by default.

Modeling guidance:
- Pick a canonical owner for mutable entities.
- Avoid ownership ambiguity for frequently updated records.
- Use ownership transfer intentionally for planned handoff workflows.

Owner-aligned writes reduce conflicts. Existing entity updates/mutations are conflict-safe when ownership is modeled correctly.

## Namespace Settings

```bash
SHOW NAMESPACE SETTINGS;
SET NAMESPACE OWNER site-west;
CLEAR NAMESPACE OWNER;
```

## Related

- [Data Model](data-model.md) — nodes, edges, properties
- [Replication](replication.md) — how namespaces scope replication
- [Cross-Tenant Modeling Guide](../guides/cross-tenant-modeling.md) — patterns and costs
- [Data Modeling Patterns](../guides/data-modeling-patterns.md) — best practices
