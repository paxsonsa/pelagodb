# Centralized Replicator And Local Scaling Spec

Status: Implemented (Core Runtime)  
Date: 2026-02-17  
Scope: PelagoDB multi-node operation with pull-based CDC replication and per-node Rocks cache

## 1. Problem Statement

We want a simpler operating model:

- One active replicator worker per site (not per API server).
- Horizontal scale for read/query traffic inside a site.
- Deterministic cache updates across all nodes.
- A clear setup path for local testing and production deployment.

Today, local mutations emit CDC atomically, and cache projectors consume CDC.  
Replica-apply paths intentionally do not emit local CDC, which can leave cache projection behind for replicated changes.

## 2. Goals And Non-Goals

## Goals

- Centralize replication execution to reduce duplicate pull/apply loops.
- Keep source-site ownership semantics.
- Preserve pull-only replication over gRPC with `source_site` filtering.
- Make cross-node cache convergence deterministic.
- Provide a concrete local scaling evaluation matrix.

## Non-Goals

- Replacing FoundationDB.
- Introducing push replication.
- Implementing full distributed consensus for every write path.

## 3. Current Baseline (Implemented)

- Pull API supports `source_site` filter: `ReplicationService.PullCdcEvents`.
- Replicator pulls from peers and applies replica-safe mutations.
- Ownership/LWW conflict handling exists in `apply_replica_*` methods.
- Replication checkpoint metadata exists per remote site.
- Cache projector consumes namespace CDC and applies to Rocks cache.

Implemented core outcomes:

- Replica-apply workers mirror successful remote operations into local CDC.
- Lease-based gating is available for centralized replicator workers.
- Scoped replication checkpoints are tracked by `<database, namespace, remote_site>`.

## 4. Target Runtime Topology

Per site:

- `N` API/query nodes (`pelago-server`, replication disabled).
- `1` active replicator worker (`pelago-server`, replication enabled).
- `N` local cache projectors (one per API node, fed by local CDC).

Data flow:

1. Remote source site writes mutation and CDC entry.
2. Local singleton replicator pulls with `source_site=<remote_site_id>`.
3. Local replicator applies mutation via replica-safe storage methods.
4. Local replicator appends a local CDC mirror entry for projector consumption.
5. Every local API node projector consumes mirrored CDC and updates Rocks cache.

Loop prevention rule:

- CDC mirror entries keep `entry.site = <remote_site_id>`.
- Remote peers pull with `source_site = <their expected peer id>`.
- A site never re-exports mirrored entries as its own source stream.

## 5. Administration Model

Two operating profiles are defined.

## Profile A (Immediate, No New Code)

- Deploy one dedicated replicator instance per site.
- Set `PELAGO_REPLICATION_ENABLED=true` only on that instance.
- Set `PELAGO_REPLICATION_ENABLED=false` on API/query instances.
- Enforce singleton operationally (Kubernetes deployment replicas = 1 or external supervisor).

Tradeoff:

- Works now, but cache updates from replicated changes can still lag unless reads fall back to FDB.

## Profile B (Target Hardened)

Profile A plus:

- Inbound replica applies also write a CDC mirror entry locally.
- FDB-backed replicator lease/fencing for singleton safety.
- Per `<database, namespace, remote_site>` replication position keys.

## 6. Proposed Metadata And Control Keys (Hardened Profile)

Use `_sys` metadata keys (or equivalent meta subspace) for control-plane state:

- `_sys:repl_lease:<site_id>:<db>:<ns>` -> `{ holder_id, epoch, lease_expires_at_micros }`
- `_sys:repl_epoch:<site_id>:<db>:<ns>` -> `u64`
- `_sys:repl_pos:<site_id>:<db>:<ns>:<remote_site_id>` -> `{ last_vs, lag_events, updated_at }`

Lease behavior:

- Heartbeat interval: 2s.
- Lease TTL: 10s.
- Takeover allowed only after expiration.
- Epoch increments on takeover; stale holders must stop applying.

## 7. Setup Guidance

## 7.1 Single-Site Local Scale (Read/Query Focus)

Run:

- 1 replicator node (replication optional if no peers).
- 2-8 API nodes behind LB.

Requirements:

- Same `site_id` and `site_name` across nodes in a site.
- Unique `listen_addr` and `cache_path` per process.
- `PELAGO_REPLICATION_ENABLED=false` on API nodes.

## 7.2 Two-Site Local Simulation

Run two sites on one machine:

- Site 1: `site_id=1`, port `27615`
- Site 2: `site_id=2`, port `27616`
- Cross-configure `PELAGO_REPLICATION_PEERS`.
- Use different `cache_path` values.

Note:

- Using one shared local FDB cluster is valid for logic testing.
- It is not a true failure-domain test. Use separate FDB clusters/containers for realistic partition testing.

## 7.3 Minimal Config Templates

API node:

```toml
site_id = 1
site_name = "us-east-1"
listen_addr = "0.0.0.0:27615"
fdb_cluster = "./fdb.cluster"
cache_enabled = true
cache_path = "./data/cache-api-1"
replication_enabled = false
default_database = "qi"
default_namespace = "core.showA"
```

Replicator node:

```toml
site_id = 1
site_name = "us-east-1"
listen_addr = "0.0.0.0:28615"
fdb_cluster = "./fdb.cluster"
cache_enabled = false
replication_enabled = true
replication_peers = "2=10.0.0.22:27615"
replication_database = "qi"
replication_namespace = "core.showA"
replication_batch_size = 512
replication_poll_ms = 300
```

## 8. Local Scaling Evaluation Matrix

| Scenario | API Nodes | Replicators/Site | Namespaces | Workload | Pass Criteria |
|---|---:|---:|---:|---|---|
| S1 Baseline | 1 | 1 | 1 | 95% reads / 5% writes | Stable p99 + zero errors |
| S2 Read Scale | 4 | 1 | 1 | 99% reads / 1% writes | Near-linear read throughput gain |
| S3 Show Isolation | 4 | 1 | 20 (`core.show*`) | Hot show + cold shows | No significant p99 regression on cold shows |
| S4 Replication Ingress | 4 | 1 | 1 | Remote CDC + local reads | Replication lag bounded and cache-hit stable |
| S5 Replicator Failover | 4 | 1 active + 1 standby | 1 | Kill active replicator | Recovery without duplicate apply loops |
| S6 Cache Rebuild | 4 | 1 | 1 | Projector restart/rebuild | Cache catches up from CDC without data loss |

Suggested metrics:

- Read latency p50/p95/p99 (`GetNode`, `ListEdges`, query traversal).
- Throughput (req/s).
- Replication lag (events and wall-clock).
- CDC projector lag (HWM distance).
- Replication conflict count (`replication.conflict` audit events).
- FDB retry/error rates.

## 9. Operational Runbooks

## Planned Replicator Maintenance

1. Confirm replication lag is near zero.
2. Stop active replicator.
3. Start replacement replicator.
4. Verify replication status advances again.

## Unplanned Replicator Crash

1. Restart dedicated replicator instance.
2. Confirm checkpoint resumes from last applied versionstamp.
3. Confirm conflict count does not spike unexpectedly.

## Conflict Spike

1. Inspect audit records for `replication.conflict`.
2. Verify peer/source-site mapping and ownership assumptions.
3. Check clocks if LWW behavior appears inconsistent.

## 10. Rollout Plan

Completed:

- Profile B runtime essentials (mirrored CDC + lease gating + scoped checkpoints).
- Admin status surfaces scoped replication checkpoints.
- Kubernetes split-topology templates and rollout checklist.

Remaining recommended rollout steps:

- Run S5/S6 failover and cache-rebuild drills in staging cadence.
- Add explicit admin lease-holder visibility and alerting surfaces.

## 11. Acceptance Criteria

- One active replicator per site per `<db, namespace>` domain.
- No replication loop amplification.
- Replicated mutations reflected in local caches within bounded lag.
- Replica checkpoint resumes correctly after restart/failover.
- Show-level namespace partitioning prevents hot-show impact on unrelated shows.
