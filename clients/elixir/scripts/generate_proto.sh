#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../../.." && pwd)"
SDK_DIR="$(cd "$(dirname "$0")/.." && pwd)"
OUT_DIR="$SDK_DIR/lib/generated"
PROTO_FILE="$ROOT_DIR/proto/pelago.proto"

mkdir -p "$OUT_DIR"

if ! command -v protoc >/dev/null 2>&1; then
  echo "protoc is required but not found" >&2
  exit 1
fi

if ! command -v protoc-gen-elixir >/dev/null 2>&1; then
  echo "protoc-gen-elixir not found. Install with: mix escript.install hex protobuf" >&2
  exit 1
fi

protoc \
  --proto_path="$ROOT_DIR/proto" \
  --elixir_out=plugins=grpc:"$OUT_DIR" \
  "$PROTO_FILE"

echo "Generated Elixir protobuf/grpc modules under $OUT_DIR"
