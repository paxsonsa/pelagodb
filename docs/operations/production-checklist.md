# Production Checklist

Practical checklist for moving from demo to production hardening.

## Must-Have Before Production

- [ ] Authentication required in all environments (`PELAGO_AUTH_REQUIRED=true`)
- [ ] API keys or bearer token workflows defined and rotated
- [ ] Audit logging enabled with retention policy tuned
- [ ] Backups and restore drills validated for FoundationDB (`scripts/dr-rehearsal.sh`)
- [ ] Replication lag and site health monitoring in place
- [ ] CI gates running full workspace tests (`scripts/ci-gate.sh`)

## Security Baseline

- [ ] Use `scripts/production-env.example` as security-first runtime profile
- [ ] mTLS configured for inter-site replication traffic
- [ ] API keys scoped to appropriate principals
- [ ] Audit retention policy documented and enforced

## Operational SLO Validation

Define p50/p95/p99 latency budgets for:
- Node point reads
- Indexed query reads
- Write paths
- Traversal queries

Track:
- Replication lag (events + time)
- Watch queue pressure and dropped events
- Audit cleanup success rate

## Data Modeling Validation

- [ ] Tenant boundary and namespace strategy documented
- [ ] Namespace lifecycle policy documented (provision, migrate, archive, decommission)
- [ ] Ownership rules documented per critical entity/edge type
- [ ] Query-critical indexes identified and justified
- [ ] Cross-tenant and cross-namespace edges reviewed for fanout/latency impact
- [ ] Schema migration plan exists for current release
- [ ] Client-side reconciliation plan exists for create-time conflict handling

## Schema Rollout Validation

- [ ] Schema registered with explicit indexes and metadata
- [ ] Strictness validated (`extras_policy`, required fields)
- [ ] Backfill jobs monitored for new indexes on existing types
- [ ] Query explain run against critical CEL filters
- [ ] Load-tested with representative data

## Query Tuning Validation

- [ ] Indexed CEL filters verified via `Explain`
- [ ] Limits/cursors configured for pageable surfaces
- [ ] Traversal depth/results/timeouts bounded
- [ ] Snapshot mode chosen appropriately per endpoint
- [ ] Benchmarks run after schema/query changes

## Known Gaps (Non-Blocking)

- Expanded partition-recovery simulation suite
- CI required-check policy wiring in remote VCS (script-level gates are ready)

## Recommended Ongoing Actions

1. Schedule nightly benchmark job (`scripts/perf-benchmark.py`) with JSON artifact retention
2. Schedule weekly DR rehearsal (`scripts/dr-rehearsal.sh --mode live`)
3. Add failover playbook for multi-site outage scenarios
4. Add dashboard alerts for replication lag growth, authz denied spikes, and audit cleanup failures

## Related

- [Deployment Guide](deployment.md) — deployment topologies
- [Security Setup](security-setup.md) — auth configuration
- [Monitoring](monitoring.md) — metrics and health checks
- [Daily Operations](daily-operations.md) — operational rhythm
- [Configuration Reference](../reference/configuration.md) — all settings
