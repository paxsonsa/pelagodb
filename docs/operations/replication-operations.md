# Replication Operations

Peer configuration, scope management, monitoring, and validation for multi-site replication.

## Configuring Peers

Set on each site's replicator nodes:

```bash
export PELAGO_REPLICATION_ENABLED=true
export PELAGO_REPLICATION_PEERS='2=127.0.0.1:27616,3=127.0.0.1:27617'
export PELAGO_REPLICATION_DATABASE=default
export PELAGO_REPLICATION_NAMESPACE=default
export PELAGO_REPLICATION_BATCH_SIZE=512
export PELAGO_REPLICATION_POLL_MS=300
```

### Multi-Scope Replication

Override single database/namespace with multiple scopes:

```bash
export PELAGO_REPLICATION_SCOPES='default/default,analytics/live'
```

### Auth Between Sites

If source sites require authentication:

```bash
export PELAGO_REPLICATION_API_KEY=<key>
```

## Lease Configuration

Lease gating ensures only one replicator actively pulls per scope:

```bash
export PELAGO_REPLICATION_LEASE_ENABLED=true
export PELAGO_REPLICATION_LEASE_TTL_MS=10000
export PELAGO_REPLICATION_LEASE_HEARTBEAT_MS=2000
```

On failure, another replicator acquires the lease and resumes from checkpoint.

## Monitoring

### Site Health

```bash
pelago admin sites
```

Verify all expected sites are registered with correct IDs.

### Replication Status

```bash
pelago admin replication-status
```

Check:
- Peers are listed
- `last_applied_versionstamp` is advancing
- Lag values are stable or decreasing

### Conflict Monitoring

```bash
pelago admin audit --action replication.conflict --limit 100
```

Sustained `replication.conflict` events may indicate modeling issues (ownership ambiguity, concurrent creates).

## Validation Checklist

- [ ] `pelago admin sites` shows expected site claims
- [ ] `pelago admin replication-status` advances for all configured peers/namespaces
- [ ] `replication.conflict` audit events are low and explainable
- [ ] Cache read behavior remains stable after replicated writes
- [ ] Replication lag converges to near-zero under steady state

## Local Testing with Docker Compose

```bash
docker compose -f docker-compose.multisite.yml up --build -d
```

- Site 1 API: `127.0.0.1:27615`
- Site 2 API: `127.0.0.1:27616`

```bash
docker compose -f docker-compose.multisite.yml down -v
```

## Operational Guidance

- Use unique site IDs globally
- Keep clocks synchronized for predictable LWW behavior
- Monitor replication lag to near-zero before planned maintenance
- Run failover and cache-rebuild drills regularly in staging

## Current Gaps

- Lease holder/epoch not yet surfaced via dedicated admin API field
- Full failover drills should be practiced in staging environments

## Related

- [Replication Concepts](../concepts/replication.md) — design and model
- [CDC and Event Model](../concepts/cdc-and-event-model.md) — CDC backbone
- [Deployment Guide](deployment.md) — deployment topologies
- [Configuration Reference](../reference/configuration.md) — replication settings
- [Set Up Multi-Site Tutorial](../tutorials/set-up-multi-site.md) — hands-on setup
