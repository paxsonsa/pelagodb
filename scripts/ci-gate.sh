#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

export LIBRARY_PATH="${LIBRARY_PATH:-/usr/local/lib}"
export FDB_CLUSTER_FILE="${FDB_CLUSTER_FILE:-/usr/local/etc/foundationdb/fdb.cluster}"
RUN_IGNORED="${PELAGO_RUN_IGNORED_TESTS:-0}"

run() {
  echo "+ $*"
  "$@"
}

echo "[1/7] format"
run cargo fmt --all -- --check

echo "[2/7] compile workspace"
run cargo check --workspace

echo "[3/7] unit+component tests"
run cargo test --workspace --all-targets --no-fail-fast

echo "[4/7] optional ignored integration tests"
if [[ "$RUN_IGNORED" == "1" ]]; then
  run cargo test --workspace --all-targets -- --ignored --nocapture --test-threads=1
else
  echo "skipped (set PELAGO_RUN_IGNORED_TESTS=1 to enable)"
fi

echo "[5/7] sdk scripts lint"
run bash -n scripts/*.sh clients/python/scripts/*.sh clients/elixir/scripts/*.sh clients/swift/Scripts/*.sh

echo "[6/7] python syntax checks"
PYTHON_BIN="${PYTHON_BIN:-python3}"
run "$PYTHON_BIN" -m py_compile \
  clients/python/pelagodb/client.py \
  datasets/load_dataset.py \
  scripts/perf-benchmark.py \
  scripts/query-scaleout-check.py

echo "[7/7] rust client sdk compile"
run cargo check --manifest-path clients/rust/pelagodb-client/Cargo.toml

echo "CI gate checks completed"
