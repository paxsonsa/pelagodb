#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../../.." && pwd)"
SDK_DIR="$(cd "$(dirname "$0")/.." && pwd)"
OUT_DIR="$SDK_DIR/Sources/PelagoDBClient/Generated"

mkdir -p "$OUT_DIR"

if ! command -v protoc >/dev/null 2>&1; then
  echo "protoc is required" >&2
  exit 1
fi

if ! command -v protoc-gen-swift >/dev/null 2>&1; then
  echo "protoc-gen-swift not found" >&2
  exit 1
fi

if ! command -v protoc-gen-grpc-swift >/dev/null 2>&1; then
  echo "protoc-gen-grpc-swift not found" >&2
  exit 1
fi

protoc \
  --proto_path="$ROOT_DIR/proto" \
  --swift_out="$OUT_DIR" \
  --grpc-swift_out=Client=true,Server=false:"$OUT_DIR" \
  "$ROOT_DIR/proto/pelago.proto"

echo "Generated Swift protobuf/grpc files in $OUT_DIR"
