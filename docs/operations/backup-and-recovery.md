# Backup and Recovery

Disaster recovery procedures, backup strategies, and the DR rehearsal process for PelagoDB.

## Architecture

PelagoDB's data lives in FoundationDB. Backup and recovery operate at the FDB level:

- **FoundationDB backup** captures the transactional state
- **RocksDB cache** is derivative and can be rebuilt from CDC
- **Schema and configuration** should be in source control

## DR Rehearsal Script

PelagoDB includes a DR rehearsal script for regular practice:

### Dry-Run (Non-Destructive)

```bash
scripts/dr-rehearsal.sh --mode dry-run
```

Validates the backup/restore command path without actually running backup operations.

### Live Backup Rehearsal

```bash
scripts/dr-rehearsal.sh --mode live --tag weekly-drill
```

Performs actual backup but uses dry-run for restore.

### Recommended Cadence

| Environment | Rehearsal Type | Frequency |
|---|---|---|
| Shared staging | Dry-run | Daily |
| Staging | Live backup | Weekly |
| Production | Full backup + restore drill | Monthly |

## FoundationDB Backup

FDB provides built-in backup tools. Key commands:

### Start Backup

```bash
fdbbackup start -d <backup-destination> -C <cluster-file>
```

### Check Backup Status

```bash
fdbbackup status -C <cluster-file>
```

### Restore

```bash
fdbrestore start -r <backup-source> -C <cluster-file>
```

Refer to [FoundationDB documentation](https://apple.github.io/foundationdb/backups.html) for complete backup/restore procedures.

## Recovery Steps

### Full Recovery

1. Restore FoundationDB from backup
2. Start PelagoDB server — schema and data will be present
3. Cache will rebuild automatically via CDC projection
4. Verify with smoke checks:
```bash
scripts/presentation-smoke.sh
```

### Cache Recovery Only

If only the RocksDB cache is lost:
1. Delete cache directory
2. Restart PelagoDB server
3. Cache projector will rebuild from CDC

### Replication Recovery

If a site loses its FDB data:
1. Restore from FDB backup (or start fresh)
2. Configure replication peers
3. Replicator will pull missed CDC events from remote sites
4. Monitor via `pelago admin replication-status`

## What to Backup

| Component | Backup Method | Recovery |
|---|---|---|
| FoundationDB data | FDB backup tools | FDB restore |
| RocksDB cache | Not needed | Rebuilt from CDC |
| Configuration | Source control | Redeploy |
| Schemas | Source control + FDB backup | Registered in FDB |

## Monitoring Recovery

After recovery, verify:
- `pelago admin sites` shows correct site registration
- `pelago admin replication-status` shows peers and advancing versionstamps
- Queries return expected data
- Smoke checks pass

## Related

- [Daily Operations](daily-operations.md) — DR rehearsal schedule
- [Production Checklist](production-checklist.md) — backup validation
- [Deployment Guide](deployment.md) — deployment topologies
- [Troubleshooting](troubleshooting.md) — recovery issues
