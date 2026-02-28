#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

export LIBRARY_PATH="${LIBRARY_PATH:-/usr/local/lib}"
export FDB_CLUSTER_FILE="${FDB_CLUSTER_FILE:-/usr/local/etc/foundationdb/fdb.cluster}"
export PELAGO_SIM_SEED="${PELAGO_SIM_SEED:-17}"
export PELAGO_SIM_STEPS="${PELAGO_SIM_STEPS:-180}"
export PELAGO_SIM_WORKERS="${PELAGO_SIM_WORKERS:-4}"
export PELAGO_SIM_SAVE_TRACE="${PELAGO_SIM_SAVE_TRACE:-1}"

echo "Running deterministic storage simulation smoke"
cargo test -p pelago-storage --features failpoints --test simulation_tests \
  test_sim_storage_consistency_seeded -- --ignored --nocapture --test-threads=1

echo "Running CDC simulation smoke"
cargo test -p pelago-storage --features failpoints --test simulation_tests \
  test_sim_cdc_correctness_seeded -- --ignored --nocapture --test-threads=1

echo "Running resume/checkpoint simulation smoke"
cargo test -p pelago-storage --features failpoints --test simulation_tests \
  test_sim_consumer_resume_checkpoint_with_injected_failure \
  -- --ignored --nocapture --test-threads=1

echo "Simulation smoke completed"

