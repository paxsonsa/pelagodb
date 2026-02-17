# Replication and Multi-Site

PelagoDB replication is pull-based CDC over gRPC.

Related docs:
- `docs/13-centralized-replication-and-scaling.md` (runtime topology and operations)
- `docs/14-data-modeling-and-scaling.md` (modeling decisions that affect replication/scaling)

## Implementation Status (2026-02-17)
- Centralized replicators can be run as dedicated server instances.
- Replication workers are lease-gated to avoid duplicate active pull/apply loops.
- Successfully applied replica operations are mirrored into local CDC.
- Cache projectors consume local CDC, so replicated changes converge across API-node caches.

## Model
- Each site owns local writes for owned entities.
- Replicators pull from remote `ReplicationService.PullCdcEvents`.
- Remote events are applied through replica-safe apply methods.
- Replication position is checkpointed per remote site and namespace scope.

## Ownership + Conflicts
- Owner-wins is the normal path.
- Split-brain edge cases use LWW fallback during replica apply.
- Conflict conditions are logged/audited (`replication.conflict`).

## Configure Replication
Set on each site:

```bash
export PELAGO_REPLICATION_ENABLED=true
export PELAGO_REPLICATION_PEERS='2=127.0.0.1:27616,3=127.0.0.1:27617'
export PELAGO_REPLICATION_DATABASE=default
export PELAGO_REPLICATION_NAMESPACE=default
export PELAGO_REPLICATION_BATCH_SIZE=512
export PELAGO_REPLICATION_POLL_MS=300
export PELAGO_REPLICATION_LEASE_ENABLED=true
export PELAGO_REPLICATION_LEASE_TTL_MS=10000
export PELAGO_REPLICATION_LEASE_HEARTBEAT_MS=2000
# optional if auth required on source
export PELAGO_REPLICATION_API_KEY=<key>
```

Recommended deployment split:
- API/query nodes: `PELAGO_REPLICATION_ENABLED=false`
- Dedicated replicator nodes: `PELAGO_REPLICATION_ENABLED=true`

## Local Docker Topology
For end-to-end local multisite testing (two sites, API + centralized replicator per site):

```bash
docker compose -f docker-compose.multisite.yml up --build -d
```

Site endpoints:
- Site 1 API: `127.0.0.1:27615`
- Site 2 API: `127.0.0.1:27616`

Shutdown:
```bash
docker compose -f docker-compose.multisite.yml down -v
```

## Validate
```bash
pelago admin sites
pelago admin replication-status
```

Expected:
- peers listed
- `last_applied_versionstamp` advancing
- lag values stable or decreasing

## Operational Guidance
- Use unique site IDs globally.
- Keep clocks reasonably synchronized for predictable LWW behavior.
- For planned maintenance, monitor replication lag to near-zero first.
