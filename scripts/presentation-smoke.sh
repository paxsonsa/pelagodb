#!/usr/bin/env bash
set -euo pipefail

SERVER_URL="${PELAGO_SERVER:-http://127.0.0.1:27615}"
DATABASE="${PELAGO_DATABASE:-default}"
NAMESPACE="${PELAGO_NAMESPACE:-default}"
SMOKE_ENTITY_TYPE="${PELAGO_SMOKE_ENTITY_TYPE:-Task}"
SMOKE_FILTER="${PELAGO_SMOKE_FILTER:-stage == 'comp'}"

run_cli() {
  cargo run -p pelago-cli -- --server "$SERVER_URL" --database "$DATABASE" --namespace "$NAMESPACE" "$@"
}

echo "[1/5] schema list"
run_cli schema list --format table >/dev/null

echo "[2/5] admin sites"
run_cli admin sites --format table >/dev/null

echo "[3/5] replication status"
run_cli admin replication-status --format table >/dev/null

echo "[4/5] audit query"
run_cli admin audit --limit 5 --format table >/dev/null

echo "[5/5] query smoke (${SMOKE_ENTITY_TYPE})"
run_cli query find "$SMOKE_ENTITY_TYPE" --filter "$SMOKE_FILTER" --limit 1 --format table >/dev/null || true

echo "Smoke checks completed against $SERVER_URL"
