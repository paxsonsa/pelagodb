#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../../.." && pwd)"
SDK_DIR="$(cd "$(dirname "$0")/.." && pwd)"
OUT_DIR="$SDK_DIR/pelagodb/generated"
PROTO_FILE="$ROOT_DIR/proto/pelago.proto"

ROOT_DIR="$ROOT_DIR" SDK_DIR="$SDK_DIR" OUT_DIR="$OUT_DIR" PROTO_FILE="$PROTO_FILE" python - <<'PY'
import os
import importlib.util
import pathlib
import subprocess
import sys

root = pathlib.Path(os.environ["ROOT_DIR"])
sdk = pathlib.Path(os.environ["SDK_DIR"])
out = pathlib.Path(os.environ["OUT_DIR"])
proto = pathlib.Path(os.environ["PROTO_FILE"])
out.mkdir(parents=True, exist_ok=True)

spec = importlib.util.find_spec("grpc_tools")
if spec is None:
    print("grpcio-tools is required. Install with: pip install grpcio-tools", file=sys.stderr)
    sys.exit(1)

import grpc_tools
include = pathlib.Path(grpc_tools.__file__).parent / "_proto"

cmd = [
    sys.executable,
    "-m",
    "grpc_tools.protoc",
    f"-I{root / 'proto'}",
    f"-I{include}",
    f"--python_out={out}",
    f"--grpc_python_out={out}",
    str(proto),
]
print("Running:", " ".join(str(x) for x in cmd))
subprocess.check_call(cmd)
PY

cat > "$OUT_DIR/__init__.py" <<'PY'
"""Generated package marker for pelago protobuf/grpc modules.

grpc_tools generates `pelago_pb2_grpc.py` with a top-level
`import pelago_pb2 as pelago__pb2`. When imported as
`pelagodb.generated.pelago_pb2_grpc`, that absolute import fails unless
`pelago_pb2` is aliased into `sys.modules`.
"""

from __future__ import annotations

import sys

from . import pelago_pb2 as _pelago_pb2

# Compatibility alias for grpc_tools absolute import style.
sys.modules.setdefault("pelago_pb2", _pelago_pb2)
PY

echo "Generated Python stubs under $OUT_DIR"
