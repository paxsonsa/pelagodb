# Quickstart

Get from zero to your first query in 10 minutes.

## Who This Is For

Teams that need a schema-first graph database with multi-site replication, strict ownership controls, and cross-tenant relationship queries.

## 1) Start the Server

Set environment variables and launch:

```bash
export PELAGO_FDB_CLUSTER=/usr/local/etc/foundationdb/fdb.cluster
# Apple Silicon: /opt/homebrew/etc/foundationdb/fdb.cluster
# Docker: export PELAGO_FDB_CLUSTER=`pwd`/fdb.cluster

export PELAGO_SITE_ID=1
export PELAGO_SITE_NAME=dc1
export PELAGO_LISTEN_ADDR=127.0.0.1:27615
export PELAGO_AUTH_REQUIRED=false
export PELAGO_CACHE_ENABLED=true

cargo run -p pelago-server --bin pelago-server
```

## 2) Verify Connectivity

In a second terminal:

```bash
cargo run -p pelago-cli -- --server http://127.0.0.1:27615 --database default --namespace default schema list
```

## 3) Register a Schema

```bash
pelago schema register --inline '{
  "name": "Person",
  "properties": {
    "name": {"type": "string", "required": true},
    "age": {"type": "int", "index": "range"}
  }
}'
```

## 4) Create Nodes

```bash
pelago node create Person name=Alice age=32
pelago node create Person name=Bob age=29
```

## 5) Query Your Data

```bash
pelago query find Person --filter 'age >= 30'
```

## 6) Load Demo Datasets (Optional)

```bash
python datasets/load_dataset.py social_graph --namespace demo
```

See the [datasets guide](../guides/datasets-and-loading.md) for all bundled datasets.

## What's Next

| Goal | Next Step |
|---|---|
| Understand the data lifecycle | [Core Workflow](core-workflow.md) |
| Learn PelagoDB's query languages | [Learn CEL Queries](../tutorials/learn-cel-queries.md) / [Learn PQL](../tutorials/learn-pql.md) |
| Build a complete graph | [Build a Social Graph](../tutorials/build-a-social-graph.md) |
| Set up for production | [Deployment Guide](../operations/deployment.md) |
| Evaluate PelagoDB for your use case | [Positioning](../concepts/positioning.md) |

## Related

- [Installation](installation.md) — detailed setup and prerequisites
- [Core Workflow](core-workflow.md) — schema → node → edge → query lifecycle
- [CLI Reference](../reference/cli.md) — complete command reference
