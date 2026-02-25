#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/dr-rehearsal.sh [options]

Non-destructive by default. This validates backup/restore command wiring against
FoundationDB and produces an operator-friendly DR rehearsal record.

Options:
  --mode dry-run|live         Run mode (default: dry-run)
  --cluster-file PATH         Source cluster file (default: $FDB_CLUSTER_FILE or /usr/local/etc/foundationdb/fdb.cluster)
  --dest-cluster-file PATH    Destination cluster file for restore checks (default: same as --cluster-file)
  --backup-dir PATH           Backup container directory (default: .tmp/dr-backups/<timestamp>)
  --tag NAME                  Backup tag name (default: pelagodb-drill)
  -h, --help                  Show this help

Examples:
  scripts/dr-rehearsal.sh
  scripts/dr-rehearsal.sh --mode dry-run --backup-dir .tmp/dr-demo
  scripts/dr-rehearsal.sh --mode live --tag weekly-drill
EOF
}

MODE="dry-run"
CLUSTER_FILE="${FDB_CLUSTER_FILE:-/usr/local/etc/foundationdb/fdb.cluster}"
DEST_CLUSTER_FILE=""
BACKUP_DIR=".tmp/dr-backups/$(date +%Y%m%d-%H%M%S)"
TAG="pelagodb-drill"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --mode)
      MODE="$2"
      shift 2
      ;;
    --cluster-file)
      CLUSTER_FILE="$2"
      shift 2
      ;;
    --dest-cluster-file)
      DEST_CLUSTER_FILE="$2"
      shift 2
      ;;
    --backup-dir)
      BACKUP_DIR="$2"
      shift 2
      ;;
    --tag)
      TAG="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown arg: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ "$MODE" != "dry-run" && "$MODE" != "live" ]]; then
  echo "invalid --mode '$MODE' (expected dry-run or live)" >&2
  exit 2
fi

if [[ -z "$DEST_CLUSTER_FILE" ]]; then
  DEST_CLUSTER_FILE="$CLUSTER_FILE"
fi

for cmd in fdbcli fdbbackup fdbrestore; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "missing required command: $cmd" >&2
    exit 2
  fi
done

if [[ ! -f "$CLUSTER_FILE" ]]; then
  echo "cluster file not found: $CLUSTER_FILE" >&2
  exit 2
fi

mkdir -p "$BACKUP_DIR"
BACKUP_URL="file://$(cd "$BACKUP_DIR" && pwd)/"

run() {
  echo "+ $*"
  "$@"
}

echo "[1/5] FoundationDB health"
run fdbcli -C "$CLUSTER_FILE" --exec "status minimal"

echo "[2/5] backup container path"
echo "backup_dir=$BACKUP_DIR"
echo "backup_url=$BACKUP_URL"

echo "[3/5] backup command rehearsal"
if [[ "$MODE" == "dry-run" ]]; then
  run fdbbackup start -n -C "$CLUSTER_FILE" -t "$TAG" -d "$BACKUP_URL"
else
  run fdbbackup start -C "$CLUSTER_FILE" -t "$TAG" -d "$BACKUP_URL"
  run fdbbackup status -C "$CLUSTER_FILE" -t "$TAG"
  run fdbbackup discontinue -C "$CLUSTER_FILE" -t "$TAG"
fi

echo "[4/5] restore command rehearsal"
# Keep restore dry-run in both modes to avoid mutating destination data during rehearsals.
set +e
RESTORE_OUTPUT="$(fdbrestore start -n --dest-cluster-file "$DEST_CLUSTER_FILE" -t "$TAG" -r "$BACKUP_URL" 2>&1)"
RESTORE_EXIT=$?
set -e
echo "$RESTORE_OUTPUT"

if [[ $RESTORE_EXIT -ne 0 ]]; then
  if [[ "$MODE" == "dry-run" ]] && [[ "$RESTORE_OUTPUT" == *"not restorable"* ]]; then
    echo "dry-run note: restore validation requires a restorable backup chain; command path is otherwise validated."
  else
    echo "restore rehearsal failed" >&2
    exit "$RESTORE_EXIT"
  fi
fi

echo "[5/5] completion"
echo "DR rehearsal completed"
echo "mode=$MODE"
echo "cluster_file=$CLUSTER_FILE"
echo "dest_cluster_file=$DEST_CLUSTER_FILE"
echo "tag=$TAG"
