# Configuration Reference

PelagoDB server configuration is loaded from TOML files, environment variables, and CLI flags.

## Precedence

Settings load in this order (lowest to highest precedence):
1. Config file (`pelago-server.toml` by default, or `--config` / `PELAGO_CONFIG`)
2. Environment variables
3. CLI flags

Use `--no-config` (or `PELAGO_NO_CONFIG=true`) to disable config file loading.

## Config File Format

Reference template: `pelago-server.example.toml`

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
ui_enabled = false
ui_addr = "127.0.0.1:4080"
```

## Core Runtime

Each env var has a matching CLI flag in kebab-case (e.g., `PELAGO_SITE_ID` → `--site-id`).

| Variable | Default | Notes |
|---|---|---|
| `PELAGO_FDB_CLUSTER` | — (required) | FDB cluster file or connection string path |
| `PELAGO_SITE_ID` | — (required) | Local site ID (`0..255`) |
| `PELAGO_SITE_NAME` | `default` | Human-readable site name |
| `PELAGO_LISTEN_ADDR` | `[::1]:50051` | gRPC listen address |
| `PELAGO_ID_BATCH_SIZE` | `100` | ID allocation batch size |
| `PELAGO_DEFAULT_DATABASE` | `default` | Default database for requests |
| `PELAGO_DEFAULT_NAMESPACE` | `default` | Default namespace for requests |
| `RUST_LOG` | `info` | Log level filter |

## Cache Controls

| Variable | Default | Notes |
|---|---|---|
| `PELAGO_CACHE_ENABLED` | `true` | Enable RocksDB read cache |
| `PELAGO_CACHE_PATH` | — | Cache directory path |
| `PELAGO_CACHE_SIZE_MB` | — | Maximum cache size |
| `PELAGO_CACHE_WRITE_BUFFER_MB` | — | Write buffer size |
| `PELAGO_CACHE_MAX_WRITE_BUFFERS` | — | Maximum concurrent write buffers |
| `PELAGO_CACHE_PROJECTOR_BATCH_SIZE` | — | CDC projector batch size |
| `PELAGO_CACHE_PROJECTOR_SCOPES` | — | Optional CSV: `db/ns,db2/ns2` |

## Replication Controls

| Variable | Default | Notes |
|---|---|---|
| `PELAGO_REPLICATION_ENABLED` | `false` | Enable pull replication |
| `PELAGO_REPLICATION_PEERS` | — | Format: `2=127.0.0.1:27616,3=127.0.0.1:27617` |
| `PELAGO_REPLICATION_DATABASE` | — | Target database for replication |
| `PELAGO_REPLICATION_NAMESPACE` | — | Target namespace for replication |
| `PELAGO_REPLICATION_SCOPES` | — | Optional CSV overriding database/namespace |
| `PELAGO_REPLICATION_BATCH_SIZE` | `512` | Batch size for CDC pull |
| `PELAGO_REPLICATION_POLL_MS` | `300` | Poll interval in milliseconds |
| `PELAGO_REPLICATION_API_KEY` | — | API key for authenticating to source |
| `PELAGO_REPLICATION_LEASE_ENABLED` | `true` | Enable lease-gated singleton |
| `PELAGO_REPLICATION_LEASE_TTL_MS` | `10000` | Lease time-to-live |
| `PELAGO_REPLICATION_LEASE_HEARTBEAT_MS` | `2000` | Lease heartbeat interval |

Recommended topology:
- API/query servers: `PELAGO_REPLICATION_ENABLED=false`
- Dedicated replicator deployment: `PELAGO_REPLICATION_ENABLED=true`

## Security and Audit Controls

| Variable | Default | Notes |
|---|---|---|
| `PELAGO_AUTH_REQUIRED` | `false` | Require authentication |
| `PELAGO_API_KEYS` | — | CSV entries; map key to principal with `key:principal` |
| `PELAGO_MTLS_ENABLED` | `false` | Enable mTLS authentication |
| `PELAGO_MTLS_SUBJECT_HEADER` | `x-mtls-subject` | Header for mTLS subject |
| `PELAGO_MTLS_SUBJECTS` | — | `subject=principal` CSV |
| `PELAGO_MTLS_FINGERPRINTS` | — | `sha256:fingerprint=principal` CSV |
| `PELAGO_MTLS_DEFAULT_ROLE` | `service` | Default role for mTLS principals |
| `PELAGO_TLS_CERT` | — | PEM certificate path |
| `PELAGO_TLS_KEY` | — | PEM private key path |
| `PELAGO_TLS_CA` | — | PEM CA bundle (required for client cert validation) |
| `PELAGO_TLS_CLIENT_AUTH` | `none` | `none`, `request`, or `require` |
| `PELAGO_AUDIT_ENABLED` | `true` | Enable audit logging |
| `PELAGO_AUDIT_RETENTION_DAYS` | `90` | Audit retention period |
| `PELAGO_AUDIT_RETENTION_SWEEP_SECS` | `300` | Cleanup sweep interval |
| `PELAGO_AUDIT_RETENTION_BATCH` | `1000` | Cleanup batch size |

## Watch Controls

| Variable | Default | Notes |
|---|---|---|
| `PELAGO_WATCH_MAX_SUBSCRIPTIONS` | — | Global subscription limit |
| `PELAGO_WATCH_MAX_NAMESPACE_SUBSCRIPTIONS` | — | Per-namespace subscription limit |
| `PELAGO_WATCH_MAX_QUERY_WATCHES` | — | Query watch limit |
| `PELAGO_WATCH_MAX_PRINCIPAL_SUBSCRIPTIONS` | — | Per-principal subscription limit |
| `PELAGO_WATCH_MAX_TTL_SECS` | — | Maximum subscription TTL |
| `PELAGO_WATCH_MAX_QUEUE_SIZE` | — | Per-subscription queue depth |
| `PELAGO_WATCH_MAX_DROPPED_EVENTS` | — | Dropped event threshold |

## Docs Site Controls

| Variable | Default | Notes |
|---|---|---|
| `PELAGO_DOCS_ENABLED` | `false` | Enable embedded docs server |
| `PELAGO_DOCS_ADDR` | `127.0.0.1:4070` | Docs listen address |
| `PELAGO_DOCS_DIR` | `docs` | Documentation directory |
| `PELAGO_DOCS_TITLE` | `PelagoDB Documentation` | Page title |

## UI Console Controls

| Variable | Default | Notes |
|---|---|---|
| `PELAGO_UI_ENABLED` | `false` | Enable embedded UI console |
| `PELAGO_UI_ADDR` | `127.0.0.1:4080` | UI listen address |
| `PELAGO_UI_ASSETS_DIR` | `ui/dist` | UI assets directory |
| `PELAGO_UI_TITLE` | `PelagoDB Console` | Console title |

## Operational Scripts

| Script | Purpose |
|---|---|
| `scripts/presentation-smoke.sh` | Pre-demo smoke checks |
| `scripts/perf-benchmark.py` | Performance benchmark harness |
| `scripts/dr-rehearsal.sh` | DR backup/restore rehearsal |
| `scripts/ci-gate.sh` | Release/CI gate checks |
| `scripts/production-env.example` | Security-first runtime profile |

## Related

- [CLI Reference](cli.md) — CLI flags and config
- [Deployment Guide](../operations/deployment.md) — deployment topologies
- [Security Setup](../operations/security-setup.md) — auth configuration
