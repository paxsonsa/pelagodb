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

## Quick Example (Dict API)
```python
from pelagodb import PelagoClient

client = PelagoClient("127.0.0.1:27615", database="default", namespace="default")

node = client.create_node("Person", {"name": "Alice", "age": 31})
print(node.id)

rows = list(client.find_nodes("Person", "age >= 30", limit=10))
print(len(rows))

client.close()
```

## Typed Schema API

Define schemas as Python classes with type-inferred properties, operator-overloaded
filters, and namespace-scoped CRUD:

```python
from pelagodb import (
    PelagoClient, Namespace, Entity, Property, OutEdge, IndexType
)

class TenantNS(Namespace):
    name = "tenant_{tenant_id}"

    class Person(Entity):
        name: str = Property(required=True, index=IndexType.EQUALITY)
        age: int = Property(default=0, index=IndexType.RANGE)
        follows = OutEdge("Person")

client = PelagoClient("127.0.0.1:27615")
acme = TenantNS.bind(tenant_id="acme")
client.register(acme)

acme_ns = client.ns(acme)
alice = acme_ns.create(acme.Person(name="Alice", age=31))
bob = acme_ns.create(acme.Person(name="Bob", age=29))
client.link(alice, "follows", bob)

# Filter with operator overloading
for p in acme_ns.find(acme.Person, acme.Person.age > 30):
    print(p.name, p.age)

# Query builder → PQL
results = (
    acme_ns.query(acme.Person)
    .filter(acme.Person.age > 25)
    .traverse(acme.Person.follows)
    .limit(50)
    .run()
)
```

### Async Watch Streams

Watch for real-time changes using `async with` / `async for`. Breaking out of the
loop automatically cancels the stream and server-side subscription:

```python
import asyncio

async def watch_changes():
    client = PelagoClient("127.0.0.1:27615")
    acme = TenantNS.bind(tenant_id="acme")
    acme_ns = client.ns(acme)

    async with acme_ns.watch_query(acme.Person, acme.Person.age > 30) as events:
        async for event in events:
            print(event.type, event.node.name)

asyncio.run(watch_changes())
```

Three watch scopes are available:
- `watch_node(model, node_id)` — watch a specific node
- `watch_query(model, filter_expr)` — watch a query for result changes
- `watch()` — watch all changes in the namespace

## Auth
```python
client = PelagoClient("127.0.0.1:27615", api_key="my-key")
# or
client = PelagoClient("127.0.0.1:27615", bearer_token="token")
```

## Examples
- `examples/basic_crud.py` — dict-based CRUD
- `examples/typed_crud.py` — typed schema with namespaces and cross-namespace edges
- `examples/query_and_pql.py`
- `examples/auth_and_audit.py`
