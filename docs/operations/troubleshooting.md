# Troubleshooting

Common issues and their fixes, organized by symptom.

## Platform-Specific Setup Issues

### macOS Apple Silicon: Cluster File Path Mismatch

**Symptoms:** Server startup fails with FDB open/connect errors. `fdbcli` works only with an explicit cluster-file path.

**Checks:**
```bash
which fdbcli
ls -l /opt/homebrew/etc/foundationdb/fdb.cluster
ls -l /usr/local/etc/foundationdb/fdb.cluster
```

**Fix:** Set `PELAGO_FDB_CLUSTER` to the actual installed path. Do not assume Intel Homebrew path on Apple Silicon.

### Docker on macOS: Host Connectivity Issues

**Symptoms:** `scripts/start-fdb.sh` completes, but PelagoDB cannot connect. Intermittent localhost reachability.

**Fix:** Prefer native local FoundationDB install for development. If using Docker, verify cluster file and endpoint mapping before starting server.

### Linux Package Permissions

**Symptoms:** Cluster file exists but startup fails with permission errors.

**Fix:** Ensure the PelagoDB process user can read the cluster file. Validate FDB service health:
```bash
fdbcli --exec "status minimal"
```

## FoundationDB Connection Errors

**Symptoms:** Server fails at startup. Integration tests fail immediately.

**Checks:**
```bash
fdbcli --exec "status minimal"
ls -l /usr/local/etc/foundationdb/fdb.cluster
```

**Fix:** Set `PELAGO_FDB_CLUSTER` to your actual cluster file. For tests, set `FDB_CLUSTER_FILE` consistently.

## CLI Cannot Reach Server

**Symptoms:** Timeout or unavailable errors from `pelago`.

**Checks:**
```bash
lsof -iTCP -sTCP:LISTEN | rg 27615
pelago --server http://127.0.0.1:27615 admin sites
```

**Fix:** Ensure server is running and `PELAGO_LISTEN_ADDR` matches client target. Check that firewall/VPN is not intercepting localhost traffic.

## Auth Failures

**Symptoms:** `UNAUTHENTICATED` or `PERMISSION_DENIED`

**Checks:**
- `PELAGO_AUTH_REQUIRED` value in server env
- Whether client sends `x-api-key` or bearer token
- Audit stream:
```bash
pelago admin audit --action authz.denied --limit 50
```

**Fix:**
- Verify API key is present in `PELAGO_API_KEYS`
- Regenerate token via `AuthService.Authenticate`
- Validate token via `AuthService.ValidateToken`

## Pagination and Cursor Errors

**Symptoms:** Duplicate or skipped results when resuming streams. Invalid cursor parsing errors.

**Checks:**
- Confirm you reuse `next_cursor` from the same endpoint and query shape
- Confirm bytes are base64 encoded in grpcurl JSON

**Fix:**
- Treat cursor tokens as opaque
- Do not mix cursors across `FindNodes` and `Traverse`
- Reset cursor to empty when query parameters change

## Watch Streams Disconnect or Drop Events

**Symptoms:** Watch stream closes unexpectedly. Consumers fall behind and miss change events.

**Checks:**
- Watch limits: `PELAGO_WATCH_MAX_QUEUE_SIZE`, `PELAGO_WATCH_MAX_DROPPED_EVENTS`
- Namespace/principal subscription limits
- Audit/admin logs for watch rejections

**Fix:**
- Reconnect with `resume_after` using last processed event versionstamp
- Reduce consumer lag and increase queue limit where appropriate
- Use narrower watch scope (point/query) instead of namespace watch when possible

## Replication Lag Not Moving

**Symptoms:** `admin replication-status` shows stale `last_applied_versionstamp`

**Checks:**
- Source/target peer config in `PELAGO_REPLICATION_PEERS`
- Source auth key if auth is required (`PELAGO_REPLICATION_API_KEY`)
- Source site health via `pelago admin sites`

**Fix:** Verify peer connectivity, auth credentials, and that the replicator process is running with `PELAGO_REPLICATION_ENABLED=true`.

## SDK Generation Fails

**Symptoms:** Missing generated modules under `clients/*/generated`

**Fix by language:**
- Python: `pip install grpcio-tools`
- Elixir: install `protoc-gen-elixir`
- Swift: install `protoc-gen-swift` and `protoc-gen-grpc-swift`

## RocksDB Cache Lock Errors in Tests

**Symptoms:** Lock-file errors in cache integration tests.

**Fix:** Ensure tests use unique temp paths. Avoid sharing cache paths across parallel test runs.

## Fast Recovery Sequence (Presentation)

1. Verify FDB health.
2. Restart server with `scripts/presentation-env.example`.
3. Reload `vfx_pipeline_50k/show_001` into `vfx.show.001`.
4. Run `scripts/presentation-smoke.sh`.

## Related

- [Installation](../getting-started/installation.md) — platform setup
- [Configuration Reference](../reference/configuration.md) — all server settings
- [Monitoring](monitoring.md) — health checks and metrics
