# Troubleshooting

## Platform-Specific Setup Issues

### macOS Apple Silicon: cluster file path mismatch

Symptoms:
- server startup fails with FDB open/connect errors
- `fdbcli` works only when you pass explicit cluster-file path

Checks:

```bash
which fdbcli
ls -l /opt/homebrew/etc/foundationdb/fdb.cluster
ls -l /usr/local/etc/foundationdb/fdb.cluster
```

Fixes:
- set `PELAGO_FDB_CLUSTER` to the actual installed path
- avoid assuming Intel Homebrew path on Apple Silicon

### Docker on macOS: host connectivity issues

Symptoms:
- `scripts/start-fdb.sh` completes, but PelagoDB cannot connect
- intermittent localhost reachability from host tools

Fixes:
- prefer native local FoundationDB install for day-to-day development
- if using Docker, verify cluster file and endpoint mapping before starting server

### Linux package permissions

Symptoms:
- cluster file exists but startup still fails with permission errors

Fixes:
- ensure PelagoDB process user can read the cluster file
- validate FoundationDB service health with `fdbcli --exec "status minimal"`

## FoundationDB Connection Errors

Symptoms:
- server fails at startup with FDB client errors
- integration tests fail immediately

Checks:

```bash
fdbcli --exec "status minimal"
ls -l /usr/local/etc/foundationdb/fdb.cluster
```

Fixes:
- set `PELAGO_FDB_CLUSTER` to your actual cluster file
- if using tests, set `FDB_CLUSTER_FILE` consistently

## CLI Cannot Reach Server

Symptoms:
- timeout or unavailable errors from `pelago`

Checks:

```bash
lsof -iTCP -sTCP:LISTEN | rg 27615
pelago --server http://127.0.0.1:27615 admin sites
```

Fixes:
- ensure server is running and `PELAGO_LISTEN_ADDR` matches client target
- confirm firewall/VPN is not intercepting localhost traffic

## Auth Failures

Symptoms:
- `UNAUTHENTICATED` or `PERMISSION_DENIED`

Checks:
- `PELAGO_AUTH_REQUIRED` value in server env
- whether client sends `x-api-key` or bearer token
- audit stream for denied actions:

```bash
pelago admin audit --action authz.denied --limit 50
```

Fixes:
- verify API key is present in `PELAGO_API_KEYS`
- regenerate token via `AuthService.Authenticate`
- validate token via `AuthService.ValidateToken`

## Pagination and Cursor Errors

Symptoms:
- duplicate or skipped results when resuming streams
- invalid cursor parsing errors from grpc clients

Checks:
- confirm you reuse `next_cursor` from the same endpoint and query shape
- confirm bytes are base64 encoded in grpcurl JSON

Fixes:
- treat cursor tokens as opaque
- do not mix cursors across `FindNodes` and `Traverse`
- reset cursor to empty when query parameters change

## Watch Streams Disconnect or Drop Events

Symptoms:
- watch stream closes unexpectedly
- consumers fall behind and miss change events

Checks:
- watch limits (`PELAGO_WATCH_MAX_QUEUE_SIZE`, `PELAGO_WATCH_MAX_DROPPED_EVENTS`)
- namespace/principal subscription limits
- audit/admin logs for watch rejections

Fixes:
- reconnect with `resume_after` using last processed event versionstamp
- reduce consumer lag and increase queue limit where appropriate
- use narrower watch scope (point/query) instead of namespace watch when possible

## Replication Lag Not Moving

Symptoms:
- `admin replication-status` shows stale `last_applied_versionstamp`

Checks:
- source/target peer config in `PELAGO_REPLICATION_PEERS`
- source auth key if auth is required (`PELAGO_REPLICATION_API_KEY`)
- source site health via `pelago admin sites`

## SDK Generation Fails

Symptoms:
- missing generated modules under `clients/*/generated`

Fixes:
- Python: `pip install grpcio-tools`
- Elixir: install `protoc-gen-elixir`
- Swift: install `protoc-gen-swift` and `protoc-gen-grpc-swift`

## RocksDB Cache Lock Errors in Tests

Symptoms:
- lock-file errors in cache integration tests

Fixes:
- ensure tests use unique temp paths
- avoid sharing cache paths across parallel integration test runs

## Fast Recovery Sequence (Presentation)

1. Verify FDB health.
2. Restart server with `scripts/presentation-env.example`.
3. Reload `vfx_pipeline_50k/show_001` into `vfx.show.001`.
4. Run `scripts/presentation-smoke.sh`.
