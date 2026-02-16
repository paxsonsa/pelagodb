# PelagoDB

PelagoDB is a schema-first graph database built on FoundationDB for teams that need:
- strict data shape guarantees
- expressive graph querying
- CDC/event-driven integration
- multi-site replication with ownership controls
- operational security and auditability

It combines low-level durability/consistency from FoundationDB with high-level graph primitives and developer tooling (gRPC APIs, CLI, SDKs, datasets, and ops scripts).

## Why Pelago?
`Pelago` is a play on *archipelago*: a connected group of islands.

That maps directly to PelagoDB's multi-site model:
- each site is part of a larger cluster
- each site can remain operationally distinct
- ownership rules keep data independent where it should be independent
- replication keeps islands connected without collapsing them into one undifferentiated node

So the name reflects the core idea: connected, clustered systems with explicit locality and ownership boundaries.

## Why PelagoDB
- Schema-first graph model: enforce entity and edge contracts up front.
- Two query surfaces: CEL for filter/range style queries and PQL for graph-native query composition.
- Built-in CDC: watch streams, cache projection, and replication all flow from the same event backbone.
- Multi-site by design: pull-based CDC replication and owner-aware conflict handling.
- Operational controls: authn/authz/audit APIs plus retention and admin surfaces.

## Feature Highlights
- Node and edge CRUD with schema validation and indexing
- CEL `find` queries and traversal queries
- PQL parser/compiler/execution + REPL
- Watch subscriptions with resume semantics
- Pull-based CDC replication service
- Authentication, authorization, and audit logging
- CLI for admin + query workflows
- Client SDKs/scaffolds for Python, Elixir, Rust, and Swift

## Quick Start (ASAP, Local FDB)

### 1) Prerequisites
- Rust stable toolchain
- FoundationDB client tools (`fdbcli`) and either:
  - a running local FoundationDB cluster, or
  - Docker (for test cluster via project script)

### 2) FoundationDB: choose one path

Path A: use existing local cluster (recommended if already running)
```bash
fdbcli --exec "status minimal"
```

Path B: start local test cluster with Docker
```bash
./scripts/start-fdb.sh
```
This creates `./fdb.cluster` for the containerized test cluster.

### 3) Start PelagoDB server
Open terminal A:
```bash
export PELAGO_FDB_CLUSTER=/usr/local/etc/foundationdb/fdb.cluster
# If you used Docker test cluster instead, use:
# export PELAGO_FDB_CLUSTER=./fdb.cluster

export PELAGO_SITE_ID=1
export PELAGO_SITE_NAME=local
export PELAGO_LISTEN_ADDR=127.0.0.1:27615
export PELAGO_AUTH_REQUIRED=false
export PELAGO_CACHE_ENABLED=true

cargo run -p pelago-server --bin pelago-server
```

Optional config-file path (precedence is `file < env < CLI`):
```bash
cp pelago-server.example.toml pelago-server.toml
cargo run -p pelago-server --bin pelago-server
# override from CLI when needed:
# cargo run -p pelago-server --bin pelago-server -- --site-id 2 --listen-addr 127.0.0.1:27616
```

### 4) Validate connectivity from CLI
Open terminal B:
```bash
cargo run -p pelago-cli -- --server http://127.0.0.1:27615 --database default --namespace default schema list
```

### 5) Create a tiny graph and query it
Register schema:
```bash
cargo run -p pelago-cli -- --server http://127.0.0.1:27615 schema register --inline '{
  "name":"Person",
  "properties":{
    "name":{"type":"string","required":true},
    "age":{"type":"int","index":"range"}
  }
}'
```

Create nodes:
```bash
cargo run -p pelago-cli -- --server http://127.0.0.1:27615 node create Person name=Alice age=31
cargo run -p pelago-cli -- --server http://127.0.0.1:27615 node create Person name=Bob age=29
```

Query:
```bash
cargo run -p pelago-cli -- --server http://127.0.0.1:27615 query find Person --filter 'age >= 30' --limit 10
```

### 6) Load example datasets
```bash
pip install -r clients/python/requirements-dev.txt
./clients/python/scripts/generate_proto.sh
python datasets/load_dataset.py social_graph --endpoint 127.0.0.1:27615 --database default --namespace demo
```
More: `datasets/README.md`

### 7) Optional: serve docs from running server
```bash
export PELAGO_DOCS_ENABLED=true
export PELAGO_DOCS_ADDR=127.0.0.1:4070
cargo run -p pelago-server --bin pelago-server
```
Open `http://127.0.0.1:4070/docs/`

## Quick Links
- Documentation index: `docs/README.md`
- Getting started: `docs/01-getting-started.md`
- Operations playbook: `docs/09-operations-playbook.md`
- Hosted docs setup: `docs/12-server-docs-site.md`
- API protocol: `proto/pelago.proto`
- CLI: `crates/pelago-cli`
- Server: `crates/pelago-server`
- Client SDKs: `clients/README.md`
- Example datasets: `datasets/README.md`

## Production-Oriented Tooling
- CI/release gate script: `scripts/ci-gate.sh`
- Benchmark harness: `scripts/perf-benchmark.py`
- DR rehearsal: `scripts/dr-rehearsal.sh`
- Production env baseline: `scripts/production-env.example`

## Stop Local Test FDB (if started via Docker)
```bash
./scripts/stop-fdb.sh
```
