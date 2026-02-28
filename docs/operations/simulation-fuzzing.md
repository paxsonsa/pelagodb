# Simulation and Fuzzing Runbook

Operational guide for seeded simulation, replay, and fuzzing triage.

## Deterministic Simulation (Storage/CDC/Replication)

Run all simulation suites against native FoundationDB:

```bash
PELAGO_SIM_SEED=17 PELAGO_SIM_STEPS=180 PELAGO_SIM_WORKERS=4 \
  scripts/simulation-smoke.sh
```

Artifacts are written to `.tmp/simulation/<scenario>_<seed>.json`.

### Seed pinning policy

When a seed finds a regression:

1. Add the seed to your CI lane env (`PELAGO_SIM_SEED=<seed>`).
2. Keep it pinned until the fix lands and the seed is green.
3. Keep at least one historical bug seed per scenario as a permanent guardrail.

## Replay A Failing Seed/Trace

Replay from a previously saved trace:

```bash
PELAGO_SIM_REPLAY_TRACE=.tmp/simulation/storage_consistency_17.json \
  cargo test -p pelago-storage --features failpoints --test simulation_tests \
  test_sim_storage_consistency_seeded -- --ignored --nocapture --test-threads=1
```

## Optional FDB Client Chaos Knobs

These are read once before the FDB network boots:

- `CLIENT_BUGGIFY_ENABLE=1`
- `CLIENT_BUGGIFY_SECTION_ACTIVATED_PROBABILITY=<0-100>`
- `CLIENT_BUGGIFY_SECTION_FIRED_PROBABILITY=<0-100>`
- `CLIENT_BUGGIFY_DISABLE_CLIENT_BYPASS=1`

Use in provisioned lanes only.

## Property Tests

Run proptest invariants:

```bash
cargo test -p pelago-storage --test proptest_invariants
```

## Fuzz Smoke

```bash
cd fuzz
cargo +nightly fuzz run pql_parser -- -max_total_time=60 -dict=dictionaries/pql.dict
cargo +nightly fuzz run cdc_decode -- -max_total_time=60
cargo +nightly fuzz run mutation_payload -- -max_total_time=60
```

## Fuzz Crash Reproduction

```bash
cd fuzz
cargo +nightly fuzz run pql_parser artifacts/pql_parser/crash-*
```

Attach artifact, seed, and replay command to the incident ticket.
