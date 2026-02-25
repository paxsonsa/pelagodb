# Production Readiness Checklist

This is the practical checklist for moving from demo to production hardening.

## Must-Have Before Production
- Authentication required in all environments (`PELAGO_AUTH_REQUIRED=true`, baseline: `scripts/production-env.example`)
- API keys or bearer token workflows defined and rotated
- Audit logging enabled with retention policy tuned
- Backups and restore drills validated for FoundationDB data (`scripts/dr-rehearsal.sh`)
- Replication lag and site health monitoring in place
- CI gates running full workspace tests including ignored integration suites (`scripts/ci-gate.sh`)

## Operational SLO Validation
- Define p50/p95/p99 latency budgets for:
  - node point reads
  - indexed query reads
  - write paths
  - traversal queries
- Track replication lag (events + time)
- Track watch queue pressure and dropped events

## Current Known Gaps (Non-Blocking for Presentation)
- Expanded partition-recovery simulation suite
- CI required-check policy wiring in remote VCS (script-level gates are ready)

## Recommended Next Actions
1. Schedule nightly benchmark job using `scripts/perf-benchmark.py` with JSON artifact retention.
2. Schedule weekly DR rehearsal using `scripts/dr-rehearsal.sh --mode live`.
3. Add failover playbook for multi-site outage scenarios.
4. Add dashboard alerts for:
   - replication lag growth
   - authz denied spikes
   - audit cleanup failures

## Immediate Presentation Operations
- Use `scripts/presentation-env.example` as the starting runtime profile.
- Run `scripts/presentation-smoke.sh` before showtime.
- Keep latest benchmark output at `.tmp/bench/latest.json`.

## Production Baseline Profile
- Use `scripts/production-env.example` for security-first runtime defaults.
