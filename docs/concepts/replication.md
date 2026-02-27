# Replication

PelagoDB replication is a pull-based CDC model over gRPC, designed for multi-site availability with explicit conflict semantics.

## Model

- Each site owns local writes for owned entities.
- Replicator workers pull CDC from remote sites via `ReplicationService.PullCdcEvents`.
- Remote events are applied through replica-safe methods.
- Replication position is checkpointed per remote site and namespace scope.

## Runtime Topology

Per site:
- **API/query nodes:** Serve client traffic, cache enabled, replication disabled.
- **Replicator workers:** Pull CDC from remote sites, replication enabled.
- **FoundationDB backend:** Shared transactional store.

Deployment split:
- API tier: many replicas (`PELAGO_REPLICATION_ENABLED=false`)
- Replicator tier: one or more replicas (`PELAGO_REPLICATION_ENABLED=true`) with lease gating

## Replication Flow

1. Remote site commits a mutation and emits CDC.
2. Local replicator pulls CDC from that source site (`source_site` filter).
3. Local replicator applies replica-safe operations with ownership/LWW enforcement.
4. Local replicator mirrors successfully applied operations into local CDC.
5. Local cache projectors consume local CDC and update per-node caches.

This keeps pull replication centralized while preserving cache convergence on all API nodes.

## Ownership and Conflicts

- **Owner-wins** is the normal path for existing entity updates.
- Split-brain edge cases use **LWW (last-write-wins)** fallback during replica apply.
- Conflict conditions are logged and audited (`replication.conflict`).

Design for conflict:
- Expect occasional dead-letter or reconciliation workflows for create conflicts.
- Use stable business keys and idempotency keys on create APIs where possible.
- Treat conflict observability as a feature, not a failure mode.

## Lease-Gated Singleton

Replicator execution is lease-gated per `<site_id, database, namespace>` scope.

| Setting | Default | Notes |
|---|---|---|
| `PELAGO_REPLICATION_LEASE_ENABLED` | `true` | Enable lease gating |
| `PELAGO_REPLICATION_LEASE_TTL_MS` | `10000` | Lease time-to-live |
| `PELAGO_REPLICATION_LEASE_HEARTBEAT_MS` | `2000` | Heartbeat interval |

Only the lease holder actively pulls/applies replication for that scope. Standby replicators remain passive until lease takeover. On failure, another worker acquires the lease and resumes from checkpoint.

## Scoped Checkpoints

Replication positions are tracked per remote site and namespace scope:
- Scope key: `<database, namespace, remote_site_id>`
- State: last applied versionstamp, lag events, updated timestamp

`pelago admin replication-status` is namespace-aware and reports scoped status.

## Cache Convergence

Applied replica operations are mirrored to local CDC. Cache projectors see replicated updates through the same CDC path used for local writes. Read nodes remain coherent without running their own pull replicators.

## Modeling Implications

Replication and scaling outcomes depend on data model choices:
- Namespace boundaries influence isolation and hot-partition behavior
- Ownership strategy affects conflict frequency
- Cross-namespace edge fanout affects traversal latency and replication pressure

## Current Gaps

- Lease holder/epoch is not yet surfaced via a dedicated admin API field
- Full failover and cache-rebuild drills should be run regularly in staging

## Related

- [CDC and Event Model](cdc-and-event-model.md) — CDC backbone powering replication
- [Caching and Consistency](caching-and-consistency.md) — read cache convergence
- [Replication Operations](../operations/replication-operations.md) — operational procedures
- [Set Up Multi-Site Tutorial](../tutorials/set-up-multi-site.md) — hands-on setup
- [Configuration Reference](../reference/configuration.md) — replication settings
