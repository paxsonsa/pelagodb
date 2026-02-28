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
PERF_SERVER="${PELAGO_SERVER:-http://127.0.0.1:27615}"
PERF_DATABASE="${PELAGO_DATABASE:-default}"
PERF_NAMESPACE="${PELAGO_NAMESPACE:-default}"
PERF_BOOTSTRAP_FIXTURE="${PELAGO_PERF_BOOTSTRAP_FIXTURE:-1}"
PERF_FIXTURE_NAMESPACE_PREFIX="${PELAGO_PERF_FIXTURE_NAMESPACE_PREFIX:-ci.perf}"
PERF_FIXTURE_KEEP="${PELAGO_PERF_FIXTURE_KEEP:-0}"
PERF_SEED_ENTITY_TYPE="${PELAGO_PERF_SEED_ENTITY_TYPE:-Person}"
PERF_ADMIN_NAMESPACE="${PELAGO_PERF_ADMIN_NAMESPACE:-default}"
RUN_SIMULATION_TESTS="${PELAGO_RUN_SIMULATION_TESTS:-0}"
RUN_FUZZ_SMOKE="${PELAGO_RUN_FUZZ_SMOKE:-0}"
FUZZ_TIME_SECS="${PELAGO_FUZZ_SMOKE_TIME_SECS:-60}"
PYTHON_BIN="${PYTHON_BIN:-python3}"

run() {
  echo "+ $*"
  "$@"
}

perf_cli() {
  local namespace="$1"
  shift
  cargo run -q -p pelago-cli -- \
    --server "$PERF_SERVER" \
    --database "$PERF_DATABASE" \
    --namespace "$namespace" \
    "$@"
}

parse_node_id() {
  "$PYTHON_BIN" -c 'import json, sys; print(json.load(sys.stdin)["id"])'
}

create_perf_fixture_namespace() {
  local stamp
  stamp="$(date +%s)"
  echo "${PERF_FIXTURE_NAMESPACE_PREFIX}.${stamp}.${RANDOM}"
}

bootstrap_perf_fixture() {
  local namespace="$1"
  local person_schema
  local seed_json
  local peer_json
  local seed_id
  local peer_id

  person_schema='{"name":"Person","properties":{"name":{"type":"string","required":true},"age":{"type":"int","index":"range"}},"edges":{"follows":{"target":"Person","direction":"outgoing"}},"meta":{"allow_undeclared_edges":true,"extras_policy":"allow"}}'

  run perf_cli "$namespace" schema register --inline "$person_schema" --format json >/dev/null

  seed_json="$(perf_cli "$namespace" node create Person name=PerfSeed age=31 --format json)"
  seed_id="$(printf '%s' "$seed_json" | parse_node_id)"
  peer_json="$(perf_cli "$namespace" node create Person name=PerfPeer age=42 --format json)"
  peer_id="$(printf '%s' "$peer_json" | parse_node_id)"

  run perf_cli "$namespace" edge create "Person:${seed_id}" follows "Person:${peer_id}" --format table >/dev/null
  echo "$seed_id"
}

cleanup_perf_fixture() {
  local namespace="$1"
  if [[ "$PERF_FIXTURE_KEEP" == "1" ]]; then
    echo "keeping perf fixture namespace '${namespace}' (PELAGO_PERF_FIXTURE_KEEP=1)"
    return 0
  fi

  if perf_cli "$PERF_ADMIN_NAMESPACE" admin drop-namespace "$namespace" --format table >/dev/null 2>&1; then
    echo "dropped perf fixture namespace '${namespace}'"
  else
    echo "warning: failed to drop perf fixture namespace '${namespace}'" >&2
  fi
}

if [[ "$RUN_IGNORED" == "1" && "$PROVISIONED_LANE" != "1" ]]; then
  echo "error: ignored test suites are restricted to provisioned lanes (set PELAGO_PROVISIONED_LANE=1)."
  exit 2
fi

echo "[1/11] format"
run cargo fmt --all -- --check

echo "[2/11] compile workspace"
run cargo check --workspace

echo "[3/11] unit+component tests"
run cargo test --workspace --all-targets --no-fail-fast

echo "[4/11] optional ignored integration tests"
if [[ "$RUN_IGNORED" == "1" ]]; then
  run cargo test --workspace --all-targets -- --ignored --nocapture --test-threads=1
else
  echo "skipped (set PELAGO_RUN_IGNORED_TESTS=1 and PELAGO_PROVISIONED_LANE=1 to enable)"
fi

echo "[5/11] optional S1-S6 performance gates"
if [[ "$ENFORCE_PERF_GATES" == "1" ]]; then
  PERF_NAMESPACE_TO_USE="$PERF_NAMESPACE"
  PERF_SEED_NODE_ID="1_0"

  if [[ "$PERF_BOOTSTRAP_FIXTURE" == "1" ]]; then
    if [[ "$PERF_PROFILE" != "default" ]]; then
      echo "error: PELAGO_PERF_BOOTSTRAP_FIXTURE=1 currently supports only PELAGO_PERF_PROFILE=default."
      echo "Set PELAGO_PERF_PROFILE=default or disable fixture bootstrap."
      exit 2
    fi
    if [[ "$PERF_SEED_ENTITY_TYPE" != "Person" ]]; then
      echo "error: PELAGO_PERF_BOOTSTRAP_FIXTURE=1 currently supports PELAGO_PERF_SEED_ENTITY_TYPE=Person."
      exit 2
    fi

    PERF_NAMESPACE_TO_USE="$(create_perf_fixture_namespace)"
    echo "bootstrapping perf fixture in namespace '${PERF_NAMESPACE_TO_USE}'"
    PERF_SEED_NODE_ID="$(bootstrap_perf_fixture "$PERF_NAMESPACE_TO_USE")"
  fi

  PERF_OUTPUT_PREFIX="${PERF_OUTPUT_JSON%.json}"
  if [[ "$PERF_OUTPUT_PREFIX" == "$PERF_OUTPUT_JSON" ]]; then
    PERF_OUTPUT_PREFIX="${PERF_OUTPUT_JSON}"
  fi
  PERF_OUTPUT_GRPC_JSON="${PERF_OUTPUT_PREFIX}-grpc.json"
  PERF_OUTPUT_CLI_JSON="${PERF_OUTPUT_PREFIX}-cli.json"

  PERF_CMD_GRPC=(
    "$PYTHON_BIN"
    scripts/perf-benchmark.py
    --transport grpc
    --server "$PERF_SERVER"
    --database "$PERF_DATABASE"
    --namespace "$PERF_NAMESPACE_TO_USE"
    --profile "$PERF_PROFILE"
    --entity-type "$PERF_SEED_ENTITY_TYPE"
    --seed-node-id "$PERF_SEED_NODE_ID"
    --runs "$PERF_RUNS"
    --warmup "$PERF_WARMUP"
    --target-get-ms 6
    --target-find-ms 12
    --target-traverse-ms 100
    --scenario "$PERF_SCENARIO"
    --enforce-targets
    --enforce-scenario-gates
    --output-json "$PERF_OUTPUT_GRPC_JSON"
  )
  if [[ -n "$PERF_OBSERVABILITY_JSON" ]]; then
    PERF_CMD_GRPC+=(--observability-json "$PERF_OBSERVABILITY_JSON")
  fi

  PERF_CMD_CLI=(
    "$PYTHON_BIN"
    scripts/perf-benchmark.py
    --build-cli
    --transport cli
    --server "$PERF_SERVER"
    --database "$PERF_DATABASE"
    --namespace "$PERF_NAMESPACE_TO_USE"
    --profile "$PERF_PROFILE"
    --entity-type "$PERF_SEED_ENTITY_TYPE"
    --seed-node-id "$PERF_SEED_NODE_ID"
    --runs "$PERF_RUNS"
    --warmup "$PERF_WARMUP"
    --target-get-ms 40
    --target-find-ms 35
    --target-traverse-ms 100
    --enforce-targets
    --output-json "$PERF_OUTPUT_CLI_JSON"
  )

  PERF_EXIT=0
  if run "${PERF_CMD_GRPC[@]}"; then
    echo "grpc perf gate passed"
  else
    PERF_EXIT=$?
    echo "grpc perf gate failed (exit=${PERF_EXIT})" >&2
  fi
  if run "${PERF_CMD_CLI[@]}"; then
    echo "cli perf gate passed"
  else
    CLI_EXIT=$?
    echo "cli perf gate failed (exit=${CLI_EXIT})" >&2
    if [[ "$PERF_EXIT" -eq 0 ]]; then
      PERF_EXIT="$CLI_EXIT"
    fi
  fi

  if [[ "$PERF_BOOTSTRAP_FIXTURE" == "1" ]]; then
    cleanup_perf_fixture "$PERF_NAMESPACE_TO_USE"
  fi

  if [[ "$PERF_EXIT" -ne 0 ]]; then
    exit "$PERF_EXIT"
  fi
else
  echo "skipped (set PELAGO_ENFORCE_PERF_GATES=1 to enable)"
fi

echo "[6/11] optional deterministic simulation smoke"
if [[ "$RUN_SIMULATION_TESTS" == "1" ]]; then
  run cargo test -p pelago-storage --features failpoints --test simulation_tests -- --ignored --nocapture --test-threads=1
else
  echo "skipped (set PELAGO_RUN_SIMULATION_TESTS=1 to enable)"
fi

echo "[7/11] optional fuzz smoke"
if [[ "$RUN_FUZZ_SMOKE" == "1" ]]; then
  if ! command -v cargo-fuzz >/dev/null 2>&1; then
    echo "error: cargo-fuzz is required when PELAGO_RUN_FUZZ_SMOKE=1"
    exit 2
  fi
  run bash -lc "cd fuzz && cargo +nightly fuzz run pql_parser -- -max_total_time=${FUZZ_TIME_SECS} -dict=dictionaries/pql.dict"
  run bash -lc "cd fuzz && cargo +nightly fuzz run cdc_decode -- -max_total_time=${FUZZ_TIME_SECS}"
  run bash -lc "cd fuzz && cargo +nightly fuzz run mutation_payload -- -max_total_time=${FUZZ_TIME_SECS}"
else
  echo "skipped (set PELAGO_RUN_FUZZ_SMOKE=1 to enable)"
fi

echo "[8/11] sdk scripts lint"
run bash -n scripts/*.sh clients/python/scripts/*.sh clients/elixir/scripts/*.sh clients/swift/Scripts/*.sh

echo "[9/11] python syntax checks"
run "$PYTHON_BIN" -m py_compile \
  clients/python/pelagodb/client.py \
  datasets/load_dataset.py \
  scripts/perf-benchmark.py \
  scripts/query-scaleout-check.py

echo "[10/11] python harness unit tests"
run "$PYTHON_BIN" -m unittest scripts/test_perf_benchmark.py

echo "[11/11] rust client sdk compile"
run cargo check --manifest-path clients/rust/pelagodb-client/Cargo.toml

echo "CI gate checks completed"
