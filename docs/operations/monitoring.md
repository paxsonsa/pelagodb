# Monitoring

Metrics, health checks, and operational monitoring for PelagoDB.

## Health Checks

### gRPC Health Service

```bash
grpcurl -plaintext \
  -d '{"service":"pelago"}' \
  127.0.0.1:27615 pelago.v1.HealthService/Check
```

### Site Registration

```bash
pelago admin sites
```

Verify all expected sites are registered with correct IDs and names.

## Replication Monitoring

### Status

```bash
pelago admin replication-status
```

Key metrics:
- **Peers listed:** All configured peers should appear
- **`last_applied_versionstamp`:** Should be advancing
- **Lag values:** Should be stable or decreasing

### Conflict Rate

```bash
pelago admin audit --action replication.conflict --limit 100
```

Sustained conflict events indicate modeling issues. Review ownership strategy and namespace boundaries.

### Recommended Alerts

| Metric | Alert Condition |
|---|---|
| Replication lag | Growing beyond threshold (e.g., > 5s) |
| Conflict rate | Spike above baseline |
| Lease acquisition failures | Repeated failures in replicator logs |

## Watch Monitoring

### Subscription Status

```bash
grpcurl -plaintext \
  -H 'x-api-key: dev-admin-key' \
  -d '{"context":{"database":"default","namespace":"default"}}' \
  127.0.0.1:27615 pelago.v1.WatchService/ListSubscriptions
```

### Watch Pressure Indicators

| Signal | Meaning | Action |
|---|---|---|
| Dropped events | Consumer falling behind | Increase `PELAGO_WATCH_MAX_QUEUE_SIZE`, reduce subscription scope |
| Subscription limit reached | Too many concurrent subscriptions | Review subscription strategy, increase limits |
| Frequent reconnections | Stream instability | Check server health, network stability |

## Audit Monitoring

```bash
pelago admin audit --limit 100
pelago admin audit --action authz.denied --limit 100
pelago admin audit --principal <user> --limit 50
```

### Recommended Alerts

| Metric | Alert Condition |
|---|---|
| `authz.denied` rate | Spike above baseline |
| Audit cleanup failures | Retention sweep errors in logs |

## Job Monitoring

```bash
pelago admin job list
pelago admin job status <job_id>
```

Monitor background jobs for:
- Index backfill completion
- Large delete/drop operations
- Strip property jobs

## Performance Monitoring

Use the benchmark harness for ongoing performance tracking:

```bash
scripts/perf-benchmark.py \
  --namespace benchmark.vfx \
  --entity-type Task \
  --seed-node-id 1_0 \
  --output-json .tmp/bench/latest.json
```

Track trends in `.tmp/bench/*.json` artifacts.

### Latency Targets

| Operation | p99 Target |
|---|---|
| Node point read | 1 ms |
| Indexed query | 10 ms |
| Traversal (bounded) | 100 ms |

## Admin Commands Summary

| Command | Purpose |
|---|---|
| `pelago admin sites` | Site registration status |
| `pelago admin replication-status` | Replication health |
| `pelago admin job list` | Background job status |
| `pelago admin audit` | Security and operation audit trail |

## Related

- [Configuration Reference](../reference/configuration.md) — all monitoring-related settings
- [Troubleshooting](troubleshooting.md) — issue diagnosis
- [Daily Operations](daily-operations.md) — operational rhythm
- [Production Checklist](production-checklist.md) — pre-production validation
