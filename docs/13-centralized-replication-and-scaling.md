# Centralized Replication and Scaling

This document describes the production runtime model for centralized replication and local scale-out in PelagoDB.

## Runtime Topology

Per site:

- API/query nodes: serve client traffic, cache enabled, replication disabled.
- Replicator workers: pull CDC from remote sites, replication enabled.
- FoundationDB backend: shared transactional store.

Typical deployment split:

- API tier: many replicas (`PELAGO_REPLICATION_ENABLED=false`)
- Replicator tier: one or more replicas (`PELAGO_REPLICATION_ENABLED=true`) with lease gating

## Replication Flow

1. Remote site commits a mutation and emits CDC.
2. Local replicator pulls CDC from that source site (`source_site` filter).
3. Local replicator applies replica-safe operations with ownership/LWW enforcement.
4. Local replicator mirrors successfully applied operations into local CDC.
5. Local cache projectors consume local CDC and update per-node Rocks caches.

This keeps pull replication centralized while preserving cache convergence on all API nodes.

## Lease-Gated Singleton Behavior

Replicator execution is lease-gated per `<site_id, database, namespace>` scope.

Configuration:

- `PELAGO_REPLICATION_LEASE_ENABLED` (default `true`)
- `PELAGO_REPLICATION_LEASE_TTL_MS` (default `10000`)
- `PELAGO_REPLICATION_LEASE_HEARTBEAT_MS` (default `2000`)

Operational effect:

- Only the lease holder actively pulls/applies replication for that scope.
- Standby replicators can exist; they remain passive until lease takeover.
- On failure, another worker acquires lease and resumes from checkpoint.

## Scoped Checkpoints

Replication positions are tracked per remote site and namespace scope:

- Scope key: `<database, namespace, remote_site_id>`
- State: last applied versionstamp, lag events, updated timestamp

`pelago admin replication-status` is namespace-aware and reports scoped status for the request context.

## Cache Convergence Semantics

Key rule:

- Applied replica operations are mirrored to local CDC.

Result:

- Cache projectors see replicated updates through the same CDC path used for local writes.
- Read nodes remain coherent without running their own pull replicators.

## Modeling Implications

Replication and scaling outcomes depend on data model choices:

- Namespace boundaries influence isolation and hot-partition behavior.
- Ownership strategy affects conflict frequency.
- Cross-namespace edge fanout affects traversal latency and replication pressure.

See `docs/14-data-modeling-and-scaling.md` for modeling guidance.

## Local and Kubernetes Deployment

Local multisite compose:

- `docker-compose.multisite.yml`

Kubernetes split deployment:

- `deploy/k8s/README.md`
- `deploy/k8s/CHECKLIST.md`

## Validation Checklist

- `pelago admin sites` shows expected site claims.
- `pelago admin replication-status` advances for configured peers/namespaces.
- `replication.conflict` audit events are low and explainable.
- Cache read behavior remains stable after replicated writes.

## Current Operational Gaps

- Lease holder/epoch is not yet surfaced via a dedicated admin API field.
- Full failover and cache-rebuild drills should be run regularly in staging.
