#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

export LIBRARY_PATH="${LIBRARY_PATH:-/usr/local/lib}"
export FDB_CLUSTER_FILE="${FDB_CLUSTER_FILE:-/usr/local/etc/foundationdb/fdb.cluster}"
RUN_IGNORED="${PELAGO_RUN_IGNORED_TESTS:-0}"
PROVISIONED_LANE="${PELAGO_PROVISIONED_LANE:-0}"
ENFORCE_PERF_GATES="${PELAGO_ENFORCE_PERF_GATES:-0}"
PERF_SCENARIO="${PELAGO_PERF_SCENARIO:-S1}"
PERF_PROFILE="${PELAGO_PERF_PROFILE:-default}"
PERF_RUNS="${PELAGO_PERF_RUNS:-30}"
PERF_WARMUP="${PELAGO_PERF_WARMUP:-5}"
PERF_OUTPUT_JSON="${PELAGO_PERF_OUTPUT_JSON:-.tmp/perf/latest-gate.json}"
PERF_OBSERVABILITY_JSON="${PELAGO_PERF_OBSERVABILITY_JSON:-}"
PYTHON_BIN="${PYTHON_BIN:-python3}"

run() {
  echo "+ $*"
  "$@"
}

if [[ "$RUN_IGNORED" == "1" && "$PROVISIONED_LANE" != "1" ]]; then
  echo "error: ignored test suites are restricted to provisioned lanes (set PELAGO_PROVISIONED_LANE=1)."
  exit 2
fi

echo "[1/8] format"
run cargo fmt --all -- --check

echo "[2/8] compile workspace"
run cargo check --workspace

echo "[3/8] unit+component tests"
run cargo test --workspace --all-targets --no-fail-fast

echo "[4/8] optional ignored integration tests"
if [[ "$RUN_IGNORED" == "1" ]]; then
  run cargo test --workspace --all-targets -- --ignored --nocapture --test-threads=1
else
  echo "skipped (set PELAGO_RUN_IGNORED_TESTS=1 and PELAGO_PROVISIONED_LANE=1 to enable)"
fi

echo "[5/8] optional S1-S6 performance gates"
if [[ "$ENFORCE_PERF_GATES" == "1" ]]; then
  PERF_CMD=(
    "$PYTHON_BIN"
    scripts/perf-benchmark.py
    --profile "$PERF_PROFILE"
    --runs "$PERF_RUNS"
    --warmup "$PERF_WARMUP"
    --scenario "$PERF_SCENARIO"
    --enforce-targets
    --enforce-scenario-gates
    --output-json "$PERF_OUTPUT_JSON"
  )
  if [[ -n "$PERF_OBSERVABILITY_JSON" ]]; then
    PERF_CMD+=(--observability-json "$PERF_OBSERVABILITY_JSON")
  fi
  run "${PERF_CMD[@]}"
else
  echo "skipped (set PELAGO_ENFORCE_PERF_GATES=1 to enable)"
fi

echo "[6/8] sdk scripts lint"
run bash -n scripts/*.sh clients/python/scripts/*.sh clients/elixir/scripts/*.sh clients/swift/Scripts/*.sh

echo "[7/8] python syntax checks"
run "$PYTHON_BIN" -m py_compile \
  clients/python/pelagodb/client.py \
  datasets/load_dataset.py \
  scripts/perf-benchmark.py \
  scripts/query-scaleout-check.py

echo "[8/8] rust client sdk compile"
run cargo check --manifest-path clients/rust/pelagodb-client/Cargo.toml

echo "CI gate checks completed"
