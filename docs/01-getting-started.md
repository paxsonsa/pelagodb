# Getting Started

## Prerequisites

- Rust toolchain (stable)
- FoundationDB 7.3.x client + server
- A valid cluster file (examples):
  - `/usr/local/etc/foundationdb/fdb.cluster` (common on macOS Intel)
  - `/opt/homebrew/etc/foundationdb/fdb.cluster` (common on macOS Apple Silicon)
  - `/etc/foundationdb/fdb.cluster` (common on Linux packages)
  - `./fdb.cluster` (if using containerized FDB for tests)

## Who This Is For

This quickstart is optimized for teams that:
- run across multiple sites/data centers,
- need strict schema and ownership controls,
- and still need relationship queries across tenant boundaries.

If you need a lowest-ops managed graph runtime first, or require zero reconciliation logic for create-time partition conflicts, PelagoDB may not be the best fit.

## Modeling Principles Before You Start

Before creating your first schema, align on these rules:
- `tenant` is a business boundary, `namespace` is an operational partition, and `ownership` is mutation authority.
- Updates to existing entities should follow ownership rules for conflict-safe behavior.
- Under partitions, concurrent creation of logically new records may resolve via LWW, so create flows should be idempotent and reconciliation-aware.
- Cross-tenant relationships are supported, but should be deliberate and reviewed for fanout/latency impact.

## Platform Callouts

### macOS Apple Silicon

- Homebrew prefix is usually `/opt/homebrew`, not `/usr/local`.
- If `fdbcli` is installed but the cluster path is wrong, startup fails with FDB connection errors.

Quick check:

```bash
which fdbcli
ls -l /opt/homebrew/etc/foundationdb/fdb.cluster
```

### Docker on macOS

- Docker Desktop networking can be different from Linux host networking.
- If `scripts/start-fdb.sh` cannot expose FDB reliably to host clients, prefer a native local FDB install.

### Linux

- Ensure your user can read the cluster file (often root-owned by package defaults).
- If using systemd services, verify FoundationDB daemon is healthy before starting PelagoDB.

Quick check:

```bash
fdbcli --exec "status minimal"
```

## Option A: Use an Existing Local FDB Cluster

If you already have a local cluster running, reuse it.

Quick check:

```bash
fdbcli --exec "status minimal"
```

## Option B: Start Test FDB via Docker

```bash
./scripts/start-fdb.sh
```

If this path fails due Docker networking behavior, use Option A instead.

## Start PelagoDB Server

Use environment variables for a predictable local demo setup:

```bash
export PELAGO_FDB_CLUSTER=/usr/local/etc/foundationdb/fdb.cluster
# Apple Silicon default is often:
# export PELAGO_FDB_CLUSTER=/opt/homebrew/etc/foundationdb/fdb.cluster

export PELAGO_SITE_ID=1
export PELAGO_SITE_NAME=sf
export PELAGO_LISTEN_ADDR=127.0.0.1:27615

# Optional runtime toggles
export PELAGO_AUTH_REQUIRED=false
export PELAGO_CACHE_ENABLED=true

cargo run -p pelago-server --bin pelago-server
```

## Use the CLI

In a second terminal:

```bash
cargo run -p pelago-cli -- --server http://127.0.0.1:27615 --database default --namespace default schema list
```

## Create a Minimal Schema and Data

Register schema:

```bash
cargo run -p pelago-cli -- --server http://127.0.0.1:27615 schema register --inline '{
  "name": "Person",
  "properties": {
    "name": {"type": "string", "required": true},
    "age": {"type": "int", "index": "range"}
  }
}'
```

Create nodes:

```bash
cargo run -p pelago-cli -- --server http://127.0.0.1:27615 node create Person name=Alice age=32
cargo run -p pelago-cli -- --server http://127.0.0.1:27615 node create Person name=Bob age=29
```

Query nodes:

```bash
cargo run -p pelago-cli -- --server http://127.0.0.1:27615 query find Person --filter 'age >= 30'
```

## Load Demo Datasets

See `datasets/README.md` for loading `social_graph` and `ecommerce` examples.

## Optional: Local Multi-Site Topology With Docker Compose

Run two sites with centralized replicators for replication/caching tests:

```bash
docker compose -f docker-compose.multisite.yml up --build -d
```

Endpoints:
- Site 1: `127.0.0.1:27615`
- Site 2: `127.0.0.1:27616`

Stop and clean volumes:

```bash
docker compose -f docker-compose.multisite.yml down -v
```

## Suggested Next Reads

- `docs/14-data-modeling-and-scaling.md`
- `docs/02-cli-reference.md`
- `docs/03-api-grpc.md`
- `docs/17-watch-and-streaming.md`
