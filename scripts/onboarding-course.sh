#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STATE_FILE="${ROOT_DIR}/.tmp/onboarding-course-state.env"

COURSE_SERVER="${PELAGO_SERVER:-http://127.0.0.1:27615}"
COURSE_DATABASE="${PELAGO_COURSE_DATABASE:-default}"
COURSE_NAMESPACE="${PELAGO_COURSE_NAMESPACE:-onboarding.demo}"
COURSE_MODE="learn"
FORCE_RESET="false"

COURSE_ENDPOINT="${COURSE_SERVER#http://}"
COURSE_ENDPOINT="${COURSE_ENDPOINT#https://}"
COURSE_ENDPOINT="${COURSE_ENDPOINT%%/*}"

PERSON_SCHEMA='{"name":"Person","properties":{"name":{"type":"string","required":true},"role":{"type":"string","index":"equality"},"level":{"type":"int","index":"range"}},"meta":{"allow_undeclared_edges":true,"extras_policy":"allow"}}'
PROJECT_SCHEMA='{"name":"Project","properties":{"name":{"type":"string","required":true},"status":{"type":"string","index":"equality"},"priority":{"type":"int","index":"range"}},"meta":{"allow_undeclared_edges":true,"extras_policy":"allow"}}'

usage() {
  cat <<USAGE
Usage:
  ./scripts/onboarding-course.sh <command> [--mode learn|demo] [--yes]

Commands:
  plan       Print 60-minute course matrix and flow
  preflight  Validate local prerequisites and server connectivity
  run        Execute full onboarding flow (modules 1-7)
  verify     Validate onboarding artifacts in the course namespace
  reset      Drop the course namespace and local state file

Environment:
  PELAGO_SERVER             Server URL (default: http://127.0.0.1:27615)
  PELAGO_COURSE_DATABASE    Database for course (default: default)
  PELAGO_COURSE_NAMESPACE   Namespace for course (default: onboarding.demo)
USAGE
}

section() {
  printf '\n== %s ==\n' "$1"
}

pause_step() {
  if [[ "${COURSE_MODE}" == "learn" && -t 0 ]]; then
    read -r -p "Press Enter to continue..."
  fi
}

fail() {
  echo "Error: $*" >&2
  exit 1
}

run_cli() {
  if command -v pelago >/dev/null 2>&1; then
    command pelago \
      --server "${COURSE_SERVER}" \
      --database "${COURSE_DATABASE}" \
      --namespace "${COURSE_NAMESPACE}" \
      "$@"
    return
  fi

  if command -v cargo >/dev/null 2>&1; then
    (
      cd "${ROOT_DIR}"
      cargo run -q -p pelago-cli -- \
        --server "${COURSE_SERVER}" \
        --database "${COURSE_DATABASE}" \
        --namespace "${COURSE_NAMESPACE}" \
        "$@"
    )
    return
  fi

  fail "Neither 'pelago' nor 'cargo' is available in PATH"
}

parse_node_id() {
  python3 -c 'import json, sys; print(json.load(sys.stdin)["id"])'
}

save_state() {
  local alice_id="$1"
  local bob_id="$2"
  local project_id="$3"

  mkdir -p "$(dirname "${STATE_FILE}")"
  cat >"${STATE_FILE}" <<STATE
export ALICE_ID='${alice_id}'
export BOB_ID='${bob_id}'
export PROJECT_ID='${project_id}'
STATE
}

load_state() {
  if [[ -f "${STATE_FILE}" ]]; then
    # shellcheck disable=SC1090
    source "${STATE_FILE}"
  fi
}

print_plan() {
  cat <<PLAN
60-Minute Onboarding Matrix (${COURSE_DATABASE}/${COURSE_NAMESPACE})

| Module | Time | Outcome | Validation |
|---|---:|---|---|
| M1 Deploy + Connect | 10m | Reach live server and namespace | schema list + admin sites |
| M2 Schema Design | 8m | Register schema-first model | schema get/list |
| M3 Data + Edges | 10m | Create connected graph records | node create + edge list |
| M4 Querying | 10m | Use CEL, traversal, and PQL | query find/traverse/pql |
| M5 Python SDK | 8m | Programmatic create/query flow | python example output |
| M6 Elixir SDK | 8m | Programmatic create/query flow | elixir example output |
| M7 Ops Wrap-Up | 6m | Observe audit + replication surface | admin replication-status/audit |

Run:
  ./scripts/onboarding-course.sh preflight
  ./scripts/onboarding-course.sh run --mode learn
  ./scripts/onboarding-course.sh verify
PLAN
}

preflight() {
  section "Preflight"

  if ! command -v python3 >/dev/null 2>&1; then
    fail "python3 is required"
  fi

  if ! command -v pelago >/dev/null 2>&1 && ! command -v cargo >/dev/null 2>&1; then
    fail "Install pelago CLI or ensure cargo is available"
  fi

  echo "Checking server connectivity: ${COURSE_SERVER}"
  run_cli schema list --format json >/dev/null
  echo "Server reachable"

  echo "Course scope: database=${COURSE_DATABASE} namespace=${COURSE_NAMESPACE}"
  echo "Tip: use an isolated namespace for repeatable demos"
}

module_1_deploy_connect() {
  section "M1 Deploy + Connect (10m)"
  echo "Verifying live control plane commands"
  run_cli schema list --format table
  run_cli admin sites --format table

  cat <<NOTE
Presenter cue:
- Explain deployment options: local FDB, docker multi-site, Kubernetes manifests.
- Confirm CLI global flags map to server/database/namespace context.
NOTE

  pause_step
}

module_2_schema() {
  section "M2 Schema Design (8m)"

  run_cli schema register --inline "${PERSON_SCHEMA}" --format json
  run_cli schema register --inline "${PROJECT_SCHEMA}" --format json

  run_cli schema get Person --format table
  run_cli schema get Project --format table

  cat <<NOTE
Presenter cue:
- Reinforce schema-first development and index-by-query-pattern choices.
- Compare required fields vs indexed fields.
NOTE

  pause_step
}

module_3_data_edges() {
  section "M3 Data + Edges (10m)"

  local alice_json
  local bob_json
  local project_json
  local alice_id
  local bob_id
  local project_id

  alice_json="$(run_cli node create Person name=Alice role=Lead level=4 --format json)"
  bob_json="$(run_cli node create Person name=Bob role=Engineer level=2 --format json)"
  project_json="$(run_cli node create Project name=Onboarding status=active priority=1 --format json)"

  alice_id="$(printf '%s' "${alice_json}" | parse_node_id)"
  bob_id="$(printf '%s' "${bob_json}" | parse_node_id)"
  project_id="$(printf '%s' "${project_json}" | parse_node_id)"

  run_cli edge create "Person:${alice_id}" mentors "Person:${bob_id}" --format table
  run_cli edge create "Project:${project_id}" assigned_to "Person:${alice_id}" --format table

  run_cli edge list Person "${alice_id}" --dir out --format table

  save_state "${alice_id}" "${bob_id}" "${project_id}"
  echo "State saved to ${STATE_FILE}"

  pause_step
}

module_4_querying() {
  section "M4 Querying (10m)"
  load_state

  if [[ -z "${PROJECT_ID:-}" ]]; then
    fail "Missing project id state. Run the full flow or module M3 first."
  fi

  run_cli query find Person --filter "level >= 3" --limit 20 --format table
  run_cli query traverse "Project:${PROJECT_ID}" assigned_to --max-depth 2 --max-results 20 --format table
  run_cli query pql --query "Person @filter(level >= 2) { uid name role level }" --format table

  cat <<NOTE
Presenter cue:
- CEL is ideal for targeted filters.
- Traverse is relationship-first access.
- PQL is a graph-native composition surface for demos and advanced query logic.
NOTE

  pause_step
}

module_5_python_sdk() {
  section "M5 Python SDK (8m)"

  if [[ ! -f "${ROOT_DIR}/clients/python/pelagodb/generated/pelago_pb2.py" ]]; then
    fail "Python protobuf stubs missing. Run clients/python/scripts/generate_proto.sh first."
  fi

  (
    cd "${ROOT_DIR}/clients/python"
    PYTHONPATH=. \
      PELAGO_ENDPOINT="${COURSE_ENDPOINT}" \
      PELAGO_DATABASE="${COURSE_DATABASE}" \
      PELAGO_NAMESPACE="${COURSE_NAMESPACE}" \
      python3 examples/onboarding_course.py
  )

  pause_step
}

module_6_elixir_sdk() {
  section "M6 Elixir SDK (8m)"

  if ! command -v mix >/dev/null 2>&1; then
    fail "mix is required for Elixir SDK module"
  fi

  if [[ ! -d "${ROOT_DIR}/clients/elixir/lib/generated" ]]; then
    fail "Elixir protobuf modules missing. Run clients/elixir/scripts/generate_proto.sh first."
  fi

  (
    cd "${ROOT_DIR}/clients/elixir"
    PELAGO_ENDPOINT="${COURSE_ENDPOINT}" \
      PELAGO_DATABASE="${COURSE_DATABASE}" \
      PELAGO_NAMESPACE="${COURSE_NAMESPACE}" \
      mix run examples/onboarding_course.exs
  )

  pause_step
}

module_7_ops_wrapup() {
  section "M7 Ops Wrap-Up (6m)"

  run_cli admin replication-status --format table
  run_cli admin audit --limit 10 --format table

  cat <<NOTE
Presenter cue:
- Close by showing observability and governance surfaces.
- Point to replication status and audit queries as day-2 operations anchors.
NOTE
}

verify() {
  section "Verify Course Namespace"
  load_state

  local person_count
  local project_count
  person_count="$(run_cli query find Person --limit 200 --format json | python3 -c 'import json, sys; print(len(json.load(sys.stdin)))')"
  project_count="$(run_cli query find Project --limit 200 --format json | python3 -c 'import json, sys; print(len(json.load(sys.stdin)))')"

  echo "Person records: ${person_count}"
  echo "Project records: ${project_count}"

  if [[ "${person_count}" -lt 2 ]]; then
    fail "Expected at least 2 Person records in the course namespace"
  fi

  if [[ "${project_count}" -lt 1 ]]; then
    fail "Expected at least 1 Project record in the course namespace"
  fi

  if [[ -n "${ALICE_ID:-}" ]]; then
    local edge_count
    edge_count="$(run_cli edge list Person "${ALICE_ID}" --dir out --format json | python3 -c 'import json, sys; print(len(json.load(sys.stdin)))')"
    echo "Outgoing edges from Alice: ${edge_count}"
  fi

  echo "Verification passed"
}

reset_course_namespace() {
  section "Reset Course Namespace"

  if [[ "${FORCE_RESET}" != "true" && -t 0 ]]; then
    read -r -p "Drop namespace '${COURSE_NAMESPACE}' in database '${COURSE_DATABASE}'? [y/N] " answer
    case "${answer}" in
      y|Y|yes|YES)
        ;;
      *)
        echo "Reset cancelled"
        return 0
        ;;
    esac
  fi

  run_cli admin drop-namespace "${COURSE_NAMESPACE}" --format table
  rm -f "${STATE_FILE}"
  echo "State cleared: ${STATE_FILE}"
}

run_all() {
  preflight
  module_1_deploy_connect
  module_2_schema
  module_3_data_edges
  module_4_querying
  module_5_python_sdk
  module_6_elixir_sdk
  module_7_ops_wrapup

  echo
  echo "Onboarding flow complete"
  echo "Run './scripts/onboarding-course.sh verify' to confirm state"
}

main() {
  local command="${1:-plan}"
  if [[ $# -gt 0 ]]; then
    shift
  fi

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --mode)
        COURSE_MODE="${2:-}"
        shift 2
        ;;
      --yes)
        FORCE_RESET="true"
        shift
        ;;
      --help|-h)
        usage
        exit 0
        ;;
      *)
        fail "Unknown option: $1"
        ;;
    esac
  done

  if [[ "${COURSE_MODE}" != "learn" && "${COURSE_MODE}" != "demo" ]]; then
    fail "--mode must be 'learn' or 'demo'"
  fi

  case "${command}" in
    plan)
      print_plan
      ;;
    preflight)
      preflight
      ;;
    run)
      run_all
      ;;
    verify)
      verify
      ;;
    reset)
      reset_course_namespace
      ;;
    *)
      fail "Unknown command: ${command}"
      ;;
  esac
}

main "$@"
