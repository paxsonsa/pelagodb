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
- Global relationship modeling: relationships can cross tenant and namespace boundaries when your domain requires it.
- Partition-aware writes: preserve availability and writability, with explicit LWW handling for conflicting new-data creation paths.
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

## Who Should Use PelagoDB
- Teams building globally distributed applications with cross-DC writes.
- Teams that need strict schema governance and explicit ownership controls.
- Teams that need graph relationships across tenant boundaries in one model.
- Teams comfortable operating replication, cache, watch, and security controls.

## Who Should Not Use PelagoDB
- Teams prioritizing minimal-ops managed graph experience above all else.
- Teams needing immediate deep compatibility with Cypher/Gremlin ecosystems.
- Teams unable to accept LWW resolution plus reconciliation for conflicting new-data creation under partition.
- Teams whose workload is mostly ad-hoc graph exploration with minimal governance requirements.

## Data Modeling Principles (Read Before First Schema)
- Keep `tenant`, `namespace`, and `ownership` as separate concepts.
- Use ownership to enforce mutation safety at the entity level.
- Use namespaces for operational partitioning and lifecycle control, not as a complete substitute for ownership.
- Model cross-tenant relationships intentionally, and review fanout/latency costs.
- Assume partition scenarios will happen and design create flows for deterministic conflict handling and reconciliation.

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

Path C: run full local multi-site topology (2 sites + centralized replicators)
```bash
docker compose -f docker-compose.multisite.yml up --build -d
```
This boots:
- `site1-api` on `127.0.0.1:27615`
- `site2-api` on `127.0.0.1:27616`
- dedicated replicator workers for each site

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
- Architecture and design: `docs/15-architecture-and-design.md`
- When to use PelagoDB: `docs/16-when-to-use-pelagodb.md`
- Watch and streaming guide: `docs/17-watch-and-streaming.md`
- Data modeling and scaling: `docs/14-data-modeling-and-scaling.md`
- Replication runtime architecture: `docs/13-centralized-replication-and-scaling.md`
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

## Latest Performance Snapshot
Local benchmark snapshot from **February 19, 2026** using:
- single API node (`127.0.0.1:27615`)
- local FDB test container
- Context benchmark profile (`scripts/perf-benchmark.py`)
- dataset: `benchmark` namespace with `Context` records (`show_001`, `scheme=main`)

Command:
```bash
python3 scripts/perf-benchmark.py \
  --pelago-bin target/debug/pelago \
  --server http://127.0.0.1:27615 \
  --database default \
  --namespace benchmark \
  --profile context \
  --entity-type Context \
  --seed-node-id 1_0 \
  --context-show show_001 \
  --context-scheme main \
  --context-shot shot_001 \
  --context-shot-alt shot_002 \
  --context-sequence seq_001 \
  --context-task fx \
  --context-label default \
  --context-page-size 20 \
  --context-deep-pages 5 \
  --runs 10 \
  --warmup 2 \
  --output-json .tmp/bench/context-latest.json
```

Results (ms):

| Case | p50 | p95 | p99 |
|---|---:|---:|---:|
| `context_point_lookup` | 36.026 | 36.843 | 36.847 |
| `context_show_shot` | 96.259 | 101.597 | 102.504 |
| `context_show_shot_task_label` | 64.957 | 68.897 | 69.156 |
| `context_or_shots` | 93.845 | 102.122 | 102.768 |
| `context_deep_page` | 88.450 | 95.417 | 95.456 |

Artifact:
- `.tmp/bench/context-latest.json`

## Single-Node VFX Baseline Snapshot
Local baseline snapshot from **February 19, 2026** using:
- single API node (`127.0.0.1:27615`)
- single local FoundationDB test node/container
- dataset: `vfx_pipeline_50k/show_001` loaded into namespace `vfx.show.001.full`

Dataset load command:
```bash
PYTHONPATH=clients/python/pelagodb/generated:clients/python \
python3 datasets/load_dataset.py vfx_pipeline_50k/show_001 \
  --endpoint 127.0.0.1:27615 \
  --database default \
  --namespace vfx.show.001.full
```

### CLI Harness Results (`scripts/perf-benchmark.py`)
Notes:
- these include per-request CLI process startup cost
- warmup/runs varied by case as shown in artifact files

| Case | p50 (ms) | p95 (ms) | p99 (ms) |
|---|---:|---:|---:|
| `node_get` (`Task:1_0`, unique-filter run) | 32.128 | 35.402 | 37.048 |
| `query_find` (`task_code == 'S001-TSK-001-001-01'`) | 39.508 | 43.471 | 46.529 |
| `query_find` (`stage == 'comp'`) | 743.499 | 925.431 | 1182.810 |
| `query_find` (`stage == 'comp' || stage == 'fx'`) | 1072.159 | 1155.109 | 1198.310 |
| `query_traverse` (`Shot:1_0`, `has_task`, depth 2) | 54.667 | 61.533 | 62.124 |

Artifacts:
- `.tmp/bench/vfx-show001-unique.json`
- `.tmp/bench/vfx-show001-stage-comp.json`
- `.tmp/bench/vfx-show001-stage-or.json`
- `.tmp/bench/vfx-show001-shot-traverse.json`

### Direct gRPC Results (Persistent Client)
Notes:
- persistent gRPC channel, no CLI spawn overhead
- includes p90 and tighter estimate of server/query path latency

| Case | p50 (ms) | p90 (ms) | p95 (ms) | p99 (ms) |
|---|---:|---:|---:|---:|
| `grpc_node_get_task_1_0` | 3.268 | 5.170 | 5.642 | 6.668 |
| `grpc_find_task_unique_code` | 9.592 | 12.744 | 14.223 | 17.275 |
| `grpc_find_task_stage_comp` | 828.053 | 903.877 | 943.520 | 997.301 |
| `grpc_find_task_stage_or` | 1141.298 | 1218.957 | 1254.694 | 1316.191 |

Artifact:
- `.tmp/bench/vfx-show001-direct-grpc.json`

### Parallel Operation Checks (Single Query Node)
Mixed-operation parallel checks using direct gRPC against `vfx.show.001.full`.

1) Selective mix (point + unique-equality find):
- 32 workers, 800 total ops (400 point + 400 find)
- throughput: ~3961 ops/sec
- point p50/p95/p99: 5.492 / 9.954 / 11.551 ms
- find p50/p95/p99: 9.606 / 11.631 / 13.590 ms

2) Broad-filter mix (point + `stage == 'comp'` find):
- 16 workers, 240 total ops (120 point + 120 find)
- throughput: ~29.45 ops/sec
- point p50/p95/p99: 4.919 / 9.125 / 9.753 ms
- find p50/p95/p99: 1011.736 / 1169.336 / 1173.376 ms

Artifacts:
- `.tmp/bench/vfx-show001-parallel-ops.json`
- `.tmp/bench/vfx-show001-parallel-ops-broad.json`
