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
