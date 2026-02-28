# Daily Operations

Execution playbook for week-to-week operations and pre-release checks.

## Daily Preflight

Run before developer demos and shared environments:

```bash
source scripts/presentation-env.example
scripts/presentation-smoke.sh
```

If smoke checks fail, stop and resolve before loading new data or showing replication behavior.

For production/staging security-first defaults, source `scripts/production-env.example` instead.

For browser-accessible docs from the running server, set `PELAGO_DOCS_ENABLED=true` and configure `PELAGO_DOCS_ADDR`.

## Performance Benchmark Harness

Track p50/p95/p99 latency on representative read/query paths.

### 1) Build CLI

```bash
cargo build -p pelago-cli --release
```

### 2) Load Dataset

```bash
python datasets/load_dataset.py vfx_pipeline_50k/show_001 \
  --endpoint 127.0.0.1:27615 \
  --database default \
  --namespace benchmark.vfx
```

### 3) Run Benchmark

```bash
scripts/perf-benchmark.py \
  --transport grpc \
  --server http://127.0.0.1:27615 \
  --database default \
  --namespace benchmark.vfx \
  --entity-type Task \
  --seed-node-id 1_0 \
  --runs 50 \
  --warmup 10 \
  --output-json .tmp/bench/latest.json
```

CLI regression profile (includes per-request CLI startup cost):
```bash
scripts/perf-benchmark.py \
  --transport cli \
  --server http://127.0.0.1:27615 \
  --database default \
  --namespace benchmark.vfx \
  --entity-type Task \
  --seed-node-id 1_0 \
  --runs 50 \
  --warmup 10 \
  --output-json .tmp/bench/latest-cli.json
```

Skip traversal benchmarks if edge data is not loaded:
```bash
scripts/perf-benchmark.py --namespace benchmark.vfx --skip-traverse
```

### 4) Enforce Phase Targets (Optional)

```bash
scripts/perf-benchmark.py --transport grpc --namespace benchmark.vfx --enforce-targets
scripts/perf-benchmark.py --transport cli --namespace benchmark.vfx --enforce-targets \
  --target-get-ms 40 --target-find-ms 35 --target-traverse-ms 100
```

Default p99 targets:
- gRPC transport (service latency): `node_get` 6 ms, `query_find` 12 ms, `query_traverse` 100 ms
- CLI transport (end-to-end regression): `node_get` 40 ms, `query_find` 35 ms, `query_traverse` 100 ms

## Disaster Recovery Rehearsal

### Dry-Run

```bash
scripts/dr-rehearsal.sh --mode dry-run
```

### Live Backup Rehearsal

```bash
scripts/dr-rehearsal.sh --mode live --tag weekly-drill
```

Recommended cadence:
- Dry-run: daily on shared staging
- Live backup rehearsal: weekly

## CI and Release Gates

```bash
scripts/ci-gate.sh
```

Include ignored integration tests (requires healthy local FDB):

```bash
PELAGO_RUN_IGNORED_TESTS=1 scripts/ci-gate.sh
```

Deterministic simulation smoke:

```bash
scripts/simulation-smoke.sh
```

## Presentation Week Rhythm

| Day | Activity |
|---|---|
| Monday | Run benchmark and capture baseline JSON artifact |
| Wednesday | Run DR rehearsal + smoke checks |
| Day before | Rerun benchmark, smoke, dataset load |
| Day of | Smoke only — no schema migrations |

## Related

- [Presentation Runbook](presentation-runbook.md) — demo delivery guide
- [Production Checklist](production-checklist.md) — pre-production readiness
- [Monitoring](monitoring.md) — metrics and health checks
- [Backup and Recovery](backup-and-recovery.md) — DR procedures
- [Simulation and Fuzzing](simulation-fuzzing.md) — seed, replay, fuzz triage
