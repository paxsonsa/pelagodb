# Presentation Runbook

## Goal

Deliver a reproducible demo that shows:
- Schema registration
- Node/edge CRUD
- Query + traversal + PQL
- Multi-site replication status
- Auth/audit visibility
- Dataset loading

## Pre-Demo Setup (Day Before)

1. Validate FDB is healthy.
2. Start PelagoDB server with stable env config.
3. Load `vfx_pipeline_50k/show_001` into `vfx.show.001`.
4. Run `scripts/presentation-smoke.sh`.
5. Run benchmark:
```bash
scripts/perf-benchmark.py \
  --namespace vfx.show.001 \
  --entity-type Task \
  --seed-node-id 1_0 \
  --output-json .tmp/bench/latest.json
```

Dataset load command:
```bash
python datasets/load_dataset.py vfx_pipeline_50k/show_001 \
  --endpoint 127.0.0.1:27615 \
  --database default \
  --namespace vfx.show.001
```

Suggested helpers:
- `scripts/presentation-env.example`
- `scripts/presentation-smoke.sh`
- Optional hosted docs: `PELAGO_DOCS_ENABLED=true`

## Demo Sequence

### 1. Show Schema
```bash
pelago schema list --format table
```

### 2. Show Query
```bash
pelago query find Task --filter "stage == 'comp'" --limit 10 --format table
```

### 3. Show Traversal
```bash
pelago query find Shot --filter "shot_code == 'S001-SQ001-SH001'" --limit 1 --format table
# Use the returned id:
pelago query traverse Shot:<id> has_task --max-depth 2 --max-results 20 --format table
```

### 4. Show PQL
```bash
pelago query pql --query "Task @filter(stage == 'comp') { uid task_code stage status }"
```

### 5. Show Operations Visibility
```bash
pelago admin sites
pelago admin replication-status
pelago admin audit --limit 20
```

## Fallback Plan

If watch or replication live demo is unstable:
- Switch to recorded output snapshots
- Show `admin replication-status` + audit history
- Use static two-site dataset proof from integration test logs

## Final 15-Minute Checklist

- [ ] Server process up
- [ ] CLI target endpoint correct
- [ ] Auth mode confirmed (required vs open)
- [ ] Dataset already loaded
- [ ] Smoke script passed in current terminal session
- [ ] Terminal windows prepared with commands pre-filled

## Related

- [Onboarding Course](../tutorials/onboarding-course.md) — 60-min guided course
- [Daily Operations](daily-operations.md) — operational rhythm
- [Datasets and Loading](../guides/datasets-and-loading.md) — bundled datasets
- [Using the Console](../guides/using-the-console.md) — web UI guide
