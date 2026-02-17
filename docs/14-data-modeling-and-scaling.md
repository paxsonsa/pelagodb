# Data Modeling and Scaling Guide

This guide defines how to model data in PelagoDB so global availability, ownership safety, and replication behavior remain predictable.

## Why Modeling Is Operational

In PelagoDB, modeling choices are not just schema choices:

- They determine write authority (ownership).
- They influence replication lag and conflict rates.
- They shape read/query fanout and cache efficiency.
- They control tenant isolation under load.
- They determine where cross-tenant relationships are allowed and how expensive they become.

Treat data modeling as part of system architecture, not only API design.

## Tenant vs Namespace vs Ownership

These are different controls and should not be conflated:

- `Tenant`: business/account boundary (who owns data logically).
- `Namespace`: operational partition for performance, lifecycle, and replication scope.
- `Ownership`: per-entity write authority that controls safe mutation under multi-site operation.

PelagoDB design principle:
- tenant isolation is important,
- but global relationship graphs can cross tenant boundaries when your product requires it,
- and write safety is enforced below namespace level through ownership.

## Core Modeling Axes

## 1) Namespace Partitioning and Lifecycle

Use namespaces to isolate hot workloads, but keep global reference data shared when needed.

Recommended pattern for asset-management workloads:

- shared baseline namespace: `asset/global`
- tenant namespaces: `asset/tenant.<tenant_id>`
- optional high-churn operational namespaces: `asset/ops.<tenant_id>`

Guidance:

- Keep global reference entities (`Vendor`, `Control`, `GeoRegion`) in `asset/global`.
- Keep tenant-owned mutable entities (`Asset`, `WorkOrder`, `Incident`) in `asset/tenant.<tenant_id>`.
- Isolate bursty ingest or workflow streams in `asset/ops.<tenant_id>` when needed.
- Keep cross-namespace edge fanout intentional and review it for latency cost.

Result:

- One hot tenant is less likely to degrade others.
- Replication and cache projector lag are easier to reason about per namespace.
- You preserve a global relationship graph where it matters.

Lifecycle stages to define per namespace family:

- Provisioning: who creates it, when, and with which baseline schemas/policies.
- Active operation: expected traffic profile, retention rules, and SLOs.
- Migration/evolution: how schema/version changes are rolled out safely.
- Archive/freeze: when writes stop and what read access remains.
- Decommission: explicit criteria and process for `drop-namespace` or long-term retention.

## 2) Entity and Edge Boundaries

Prefer clear ownership, bounded fanout, and explicit edge semantics.

Guidance:

- Keep entities cohesive; avoid giant polymorphic objects.
- Model many-to-many relationships with explicit edge types.
- Index only query-critical properties; avoid indexing everything.
- Use sort keys on high-volume edges when traversal order matters.

Cross-tenant relationship examples that are often valid:

- `Asset(tenant-a) -> depends_on -> Vendor(global)`
- `Asset(tenant-a) -> shares_ioc_with -> Asset(tenant-b)`
- `Incident(tenant-a) -> linked_to -> Control(global)`

Anti-patterns:

- One namespace for all tenants with unbounded fanout traversals.
- Over-indexing low-selectivity properties.
- Edges that force heavy cross-namespace lookups in critical low-latency paths.

## 3) Ownership Strategy

Ownership governs who can mutate data directly.

Current default:

- Node ownership follows locality/site.
- Edge ownership is source-site by default.

Modeling guidance:

- Pick a canonical owner for mutable entities.
- Avoid ownership ambiguity for frequently updated records.
- Use ownership transfer intentionally for planned handoff workflows.

Replication impact:

- Owner-aligned writes reduce conflicts.
- Existing entity updates/mutations are conflict-safe when ownership is modeled correctly.

## 4) Cross-DC Conflict Model for New Data

PelagoDB prioritizes availability and writability during partitions, with explicit tradeoffs.

Behavior to design for:

- Under partition, writes continue at multiple sites.
- Consistency/latency tradeoffs are primarily for new data creation paths.
- Concurrent "new data" conflicts can occur (for example, duplicate logical entities created independently).
- Conflict handling uses deterministic latest-write-wins (LWW) on replica apply paths.
- Conflicts are surfaced through audit/replication signals instead of silently corrupting state.

What this means for clients:

- Expect occasional dead-letter or reconciliation workflows for create conflicts.
- Use stable business keys and idempotency keys on create APIs where possible.
- Treat conflict observability as a feature, not a failure mode.

## 5) Schema Versioning Strategy

Plan schema evolution explicitly by tenant and namespace boundaries.

Guidance:

- Version schemas per entity type.
- Roll out additive changes first when possible.
- Avoid mixed assumptions in clients during transition windows.
- Isolate tenant-specific drift in tenant namespaces when global coupling is unnecessary.

## 6) Replication-Aware Modeling

Design so pull replication remains efficient and stable.

Guidance:

- Keep mutation-heavy entities in namespaces with clear ownership.
- Keep replication domains scoped (`database + namespace`).
- Favor locality-aligned mutations to minimize conflict/audit noise.
- Monitor `replication.conflict` and sustained LWW activity as a modeling signal.

## 7) Scale-Aware Modeling

For read-heavy systems, optimize for cache locality and traversal efficiency.

Guidance:

- Separate hot and cold workloads by namespace.
- Keep high-frequency traversals within a namespace when possible.
- Define edge labels with clear semantics to reduce query ambiguity.
- Validate p99 behavior using representative tenant-scale datasets.

## Practical Checklist

- [ ] Tenant boundary and namespace strategy documented.
- [ ] Namespace lifecycle policy documented (provision, migrate, archive, decommission).
- [ ] Ownership rules documented per critical entity/edge type.
- [ ] Query-critical indexes identified and justified.
- [ ] Cross-tenant and cross-namespace edges reviewed for fanout/latency impact.
- [ ] Schema migration plan exists for current release.
- [ ] Replication lag/conflict SLOs defined for each hot namespace.
- [ ] Client-side reconciliation plan exists for create-time conflict/dead-letter handling.
- [ ] Decommission runbook exists for namespace retirement and data retention compliance.

## Related Docs

- `docs/06-replication-and-multi-site.md`
- `docs/13-centralized-replication-and-scaling.md`
- `docs/09-operations-playbook.md`
- `docs/04-administration.md`
- `docs/16-when-to-use-pelagodb.md`
- `datasets/README.md`
