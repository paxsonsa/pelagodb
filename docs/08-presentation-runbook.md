# Presentation Runbook (Next Week)

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
3. Load `social_graph` dataset.
4. Run `scripts/presentation-smoke.sh`.
5. Run `scripts/perf-benchmark.py --namespace demo --output-json .tmp/bench/latest.json`.

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
pelago query find Person --filter 'age >= 30' --limit 10 --format table
```
3. Show traversal:
```bash
pelago query traverse Person:1_0 follows --max-depth 2 --max-results 20 --format table
```
4. Show PQL:
```bash
pelago query pql --query 'Person @filter(age >= 30) { uid name age }'
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
