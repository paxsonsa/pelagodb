# Administration Guide

## Configuration Precedence
Server settings load in this order (lowest to highest precedence):
1. Config file (`pelago-server.toml` by default, or `--config` / `PELAGO_CONFIG`)
2. Environment variables
3. CLI flags

`--no-config` (or `PELAGO_NO_CONFIG=true`) disables config file loading.

## Config File Format
Example `pelago-server.toml`:

```toml
site_id = 1
site_name = "sf"
listen_addr = "127.0.0.1:27615"
fdb_cluster = "./fdb.cluster"
default_database = "default"
default_namespace = "default"
cache_enabled = true
auth_required = false
docs_enabled = true
docs_addr = "127.0.0.1:4070"
```

Reference template: `pelago-server.example.toml`.

## Core Runtime Environment
From `crates/pelago-core/src/config.rs` and `crates/pelago-server/src/main.rs`.
Each env var below also has a matching CLI flag using kebab-case names (for example, `PELAGO_SITE_ID` -> `--site-id`).

- `PELAGO_FDB_CLUSTER` (required): FDB cluster file or connection string path
- `PELAGO_SITE_ID` (required): local site ID (`0..255`)
- `PELAGO_SITE_NAME` (default: `default`)
- `PELAGO_LISTEN_ADDR` (default: `[::1]:50051`)
- `PELAGO_ID_BATCH_SIZE` (default: `100`)
- `RUST_LOG` (default: `info`)

## Cache Controls
- `PELAGO_CACHE_ENABLED` (default: `true`)
- `PELAGO_CACHE_PATH`
- `PELAGO_CACHE_SIZE_MB`
- `PELAGO_CACHE_WRITE_BUFFER_MB`
- `PELAGO_CACHE_MAX_WRITE_BUFFERS`
- `PELAGO_CACHE_PROJECTOR_BATCH_SIZE`

## Replication Controls
- `PELAGO_REPLICATION_ENABLED`
- `PELAGO_REPLICATION_PEERS` (format: `2=127.0.0.1:27616,3=127.0.0.1:27617`)
- `PELAGO_REPLICATION_DATABASE`
- `PELAGO_REPLICATION_NAMESPACE`
- `PELAGO_REPLICATION_BATCH_SIZE`
- `PELAGO_REPLICATION_POLL_MS`
- `PELAGO_REPLICATION_API_KEY`
- `PELAGO_REPLICATION_LEASE_ENABLED` (default: `true`)
- `PELAGO_REPLICATION_LEASE_TTL_MS` (default: `10000`)
- `PELAGO_REPLICATION_LEASE_HEARTBEAT_MS` (default: `2000`)

Recommended topology:
- API/query servers: `PELAGO_REPLICATION_ENABLED=false`
- Dedicated replicator deployment: `PELAGO_REPLICATION_ENABLED=true`

## Security and Audit Controls
- `PELAGO_AUTH_REQUIRED` (default: `false`)
- `PELAGO_API_KEYS` (CSV entries; can map key to principal with `key:principal`)
- `PELAGO_AUDIT_ENABLED` (default: `true`)
- `PELAGO_AUDIT_RETENTION_DAYS` (default: `90`)
- `PELAGO_AUDIT_RETENTION_SWEEP_SECS` (default: `300`)
- `PELAGO_AUDIT_RETENTION_BATCH` (default: `1000`)

## Watch Controls
- `PELAGO_WATCH_MAX_SUBSCRIPTIONS`
- `PELAGO_WATCH_MAX_NAMESPACE_SUBSCRIPTIONS`
- `PELAGO_WATCH_MAX_QUERY_WATCHES`
- `PELAGO_WATCH_MAX_PRINCIPAL_SUBSCRIPTIONS`
- `PELAGO_WATCH_MAX_TTL_SECS`
- `PELAGO_WATCH_MAX_QUEUE_SIZE`
- `PELAGO_WATCH_MAX_DROPPED_EVENTS`

## Docs Site Controls
- `PELAGO_DOCS_ENABLED` (default: `false`)
- `PELAGO_DOCS_ADDR` (default: `127.0.0.1:4070`)
- `PELAGO_DOCS_DIR` (default: `docs`)
- `PELAGO_DOCS_TITLE` (default: `PelagoDB Documentation`)

## Common Admin Commands
```bash
pelago admin sites
pelago admin replication-status
pelago admin job list
pelago admin audit --limit 100
```

## Operational Scripts
- Smoke checks: `scripts/presentation-smoke.sh`
- Performance benchmark harness: `scripts/perf-benchmark.py`
- DR rehearsal (backup/restore command path): `scripts/dr-rehearsal.sh`
- Release/CI gate command set: `scripts/ci-gate.sh`
- Security-first runtime profile: `scripts/production-env.example`

## Site Bring-Up Checklist
1. Verify FDB health.
2. Start server with unique `PELAGO_SITE_ID` and `PELAGO_SITE_NAME`.
3. Check site registration via `pelago admin sites`.
4. If multi-site: verify peers via `pelago admin replication-status`.
5. Confirm audit activity via `pelago admin audit`.
