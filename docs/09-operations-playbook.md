# Operations Playbook

This guide is the execution playbook for week-to-week operations and pre-release checks.

## Daily Preflight
Run before developer demos and shared environments:

```bash
source scripts/presentation-env.example
scripts/presentation-smoke.sh
```

If smoke checks fail, stop and resolve before loading new data or showing replication behavior.

For production/staging security-first defaults, source `scripts/production-env.example` instead.

If you want browser docs from the running server process, set `PELAGO_DOCS_ENABLED=true` and configure `PELAGO_DOCS_ADDR`.

## Performance Benchmark Harness
Use the benchmark harness to track p50/p95/p99 latency on representative read/query paths.

### 1) Build CLI once
```bash
cargo build -p pelago-cli --release
```

### 2) Load dataset for repeatable benchmark inputs
```bash
python datasets/load_dataset.py vfx_pipeline_50k/show_001 \
  --endpoint 127.0.0.1:27615 \
  --database default \
  --namespace benchmark.vfx
```

### 3) Run benchmark
```bash
scripts/perf-benchmark.py \
  --server http://127.0.0.1:27615 \
  --database default \
  --namespace benchmark.vfx \
  --entity-type Task \
  --seed-node-id 1_0 \
  --runs 50 \
  --warmup 10 \
  --output-json .tmp/bench/latest.json
```

If your benchmark namespace does not yet include traversal-ready edge data:
```bash
scripts/perf-benchmark.py --namespace benchmark.vfx --skip-traverse
```

### 4) Enforce phase targets (optional)
```bash
scripts/perf-benchmark.py --namespace benchmark.vfx --enforce-targets
```

Default p99 targets are aligned to phase docs:
- `node_get`: 1 ms
- `query_find`: 10 ms
- `query_traverse`: 100 ms

## Disaster Recovery Rehearsal
Use a non-destructive dry-run by default.

### Dry-run rehearsal
```bash
scripts/dr-rehearsal.sh --mode dry-run
```

### Live backup rehearsal (restore remains dry-run)
```bash
scripts/dr-rehearsal.sh --mode live --tag weekly-drill
```

Recommended cadence:
- dry-run: daily on shared staging
- live backup rehearsal: weekly

## CI and Release Gates
Use one script for local and CI gate parity:

```bash
scripts/ci-gate.sh
```

To include ignored integration tests (requires healthy local FDB):

```bash
PELAGO_RUN_IGNORED_TESTS=1 scripts/ci-gate.sh
```

## Presentation Week Rhythm
- Monday: run benchmark and capture baseline JSON artifact
- Wednesday: run DR rehearsal + smoke checks
- Day before presentation: rerun benchmark, smoke, dataset load
- Day of presentation: smoke only, no schema migrations

## Kubernetes Topology

For split API + centralized replicator deployment:

- `deploy/k8s/README.md`
- `deploy/k8s/CHECKLIST.md`
