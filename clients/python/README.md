# PelagoDB Python SDK

High-level Python wrapper for PelagoDB gRPC APIs.

## Install
```bash
cd clients/python
python -m venv .venv
source .venv/bin/activate
pip install -e .
```

## Generate Protobuf Stubs
```bash
pip install -r requirements-dev.txt
./scripts/generate_proto.sh
```

This creates:
- `pelagodb/generated/pelago_pb2.py`
- `pelagodb/generated/pelago_pb2_grpc.py`

## Schema Registration Defaults
- `register_schema_dict` now sets `index_default_mode=INDEX_DEFAULT_MODE_AUTO_BY_TYPE_V1`.
- If a property omits `index`, the server infers:
  - `int`, `float`, `timestamp` -> `range`
  - `bool` -> `equality`
  - `string`, `bytes` -> `none`
- Set `"index": "none"` to explicitly disable inferred indexing for a property.

## Quick Example
```python
from pelagodb import PelagoClient

client = PelagoClient("127.0.0.1:27615", database="default", namespace="default")

node = client.create_node("Person", {"name": "Alice", "age": 31})
print(node.id)

rows = list(client.find_nodes("Person", "age >= 30", limit=10))
print(len(rows))

client.close()
```

## Auth
```python
client = PelagoClient("127.0.0.1:27615", api_key="my-key")
# or
client = PelagoClient("127.0.0.1:27615", bearer_token="token")
```

## Examples
- `examples/basic_crud.py`
- `examples/query_and_pql.py`
- `examples/auth_and_audit.py`
