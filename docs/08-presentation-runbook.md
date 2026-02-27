# Presentation Runbook

## Goal
Have a reproducible demo that shows:
- schema registration
- node/edge CRUD
- query + traversal + PQL
- multi-site replication status
- auth/audit visibility
- dataset loading

## Pre-Demo Setup (Day Before)
1. Validate FDB is healthy.
2. Start PelagoDB server with stable env config.
3. Load `vfx_pipeline_50k/show_001` into `vfx.show.001`.
4. Run `scripts/presentation-smoke.sh`.
5. Run `scripts/perf-benchmark.py --namespace vfx.show.001 --entity-type Task --seed-node-id 1_0 --output-json .tmp/bench/latest.json`.

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
- optional hosted docs: `PELAGO_DOCS_ENABLED=true` (`docs/12-server-docs-site.md`)

## Demo Sequence (Recommended)
1. Show schema:
```bash
pelago schema list --format table
```
2. Show query:
```bash
pelago query find Task --filter "stage == 'comp'" --limit 10 --format table
```
3. Show traversal:
```bash
pelago query find Shot --filter "shot_code == 'S001-SQ001-SH001'" --limit 1 --format table
# Use the returned id in place of REPLACE_WITH_SHOT_NODE_ID:
pelago query traverse Shot:REPLACE_WITH_SHOT_NODE_ID has_task --max-depth 2 --max-results 20 --format table
```
4. Show PQL:
```bash
pelago query pql --query "Task @filter(stage == 'comp') { uid task_code stage status }"
```
5. Show ops visibility:
```bash
pelago admin sites
pelago admin replication-status
pelago admin audit --limit 20
```

## Fallback Plan
- If watch or replication live demo is unstable, switch to:
  - recorded output snapshots
  - `admin replication-status` + audit history
  - static two-site dataset proof from integration test logs

## Final 15-Minute Checklist
- server process up
- CLI target endpoint correct
- auth mode confirmed (required vs open)
- dataset already loaded
- smoke script passed in current terminal session
- terminal windows prepared with commands pre-filled
