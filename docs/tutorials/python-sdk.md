# Tutorial: Python SDK

End-to-end tutorial using the PelagoDB Python SDK for schema registration, CRUD operations, and querying.

## Prerequisites

### Install the SDK

```bash
cd clients/python
python -m venv .venv
source .venv/bin/activate
pip install -e .
pip install -r requirements-dev.txt
./scripts/generate_proto.sh
```

### Ensure server is running

```bash
pelago admin sites   # verify connectivity
```

## Step 1: Connect

```python
from pelagodb import PelagoClient

client = PelagoClient("127.0.0.1:27615", database="default", namespace="demo")
```

With authentication:

```python
client = PelagoClient(
    "127.0.0.1:27615",
    database="default",
    namespace="demo",
    api_key="dev-admin-key"
)
```

## Step 2: Register a Schema

```python
schema = {
    "name": "Person",
    "properties": {
        "name": {"type": "string", "required": True},
        "email": {"type": "string", "index": "unique"},
        "age": {"type": "int", "index": "range"},
        "active": {"type": "bool", "default": True}
    },
    "edges": {
        "FOLLOWS": {
            "target": "Person",
            "direction": "outgoing"
        }
    },
    "meta": {
        "allow_undeclared_edges": True,
        "extras_policy": "allow"
    }
}

client.register_schema_dict(schema)
```

> **Note:** `register_schema_dict` is a convenience helper. For full feature access (edge properties, sort keys), use the raw protobuf registration path.

## Step 3: Create Nodes

```python
alice = client.create_node("Person", {
    "name": "Alice",
    "email": "alice@example.com",
    "age": 32
})
print(f"Created Alice: {alice.node_id}")

bob = client.create_node("Person", {
    "name": "Bob",
    "email": "bob@example.com",
    "age": 29
})
print(f"Created Bob: {bob.node_id}")
```

## Step 4: Query Nodes

### CEL Filter

```python
results = list(client.find_nodes("Person", "age >= 30", limit=10))
for node in results:
    print(f"{node.node_id}: {node.properties}")
```

### PQL Query

```python
results = list(client.execute_pql(
    'Person @filter(age >= 25) { uid name age }'
))
for result in results:
    print(f"{result.block_name}: {result.node}")
```

## Step 5: Update and Delete

```python
# Update
client.update_node("Person", alice.node_id, {"age": 33})

# Get
updated = client.get_node("Person", alice.node_id)
print(f"Updated age: {updated.properties['age']}")

# Delete
client.delete_node("Person", bob.node_id)
```

## Step 6: Clean Up

```python
client.close()
```

## Complete Example

```python
from pelagodb import PelagoClient

client = PelagoClient("127.0.0.1:27615", database="default", namespace="demo")

# Register schema
client.register_schema_dict({
    "name": "Person",
    "properties": {
        "name": {"type": "string", "required": True},
        "email": {"type": "string", "index": "unique"},
        "age": {"type": "int", "index": "range"}
    },
    "meta": {"allow_undeclared_edges": True, "extras_policy": "allow"}
})

# Create data
alice = client.create_node("Person", {"name": "Alice", "email": "alice@example.com", "age": 32})
bob = client.create_node("Person", {"name": "Bob", "email": "bob@example.com", "age": 29})

# Query
results = list(client.find_nodes("Person", "age >= 25", limit=10))
print(f"Found {len(results)} people age >= 25")

# PQL
pql_results = list(client.execute_pql('Person @filter(age >= 30) { uid name age }'))
print(f"PQL found {len(pql_results)} results")

client.close()
```

## Error Handling

```python
try:
    client.create_node("Person", {"name": "Invalid"})
except Exception as e:
    print(f"Error: {e}")
```

## Typed Schema API

The Python SDK also provides a typed schema layer with Pydantic-style class
definitions, operator-overloaded filters, and namespace-scoped operations.

### Define Schemas as Classes

```python
from pelagodb import (
    PelagoClient, Namespace, Entity, Property, OutEdge, IndexType
)

class GlobalNS(Namespace):
    name = "global"

    class Vendor(Entity):
        name: str = Property(required=True, index=IndexType.EQUALITY)
        industry: str = Property()

class TenantNS(Namespace):
    name = "tenant_{tenant_id}"

    class Person(Entity):
        name: str = Property(required=True, index=IndexType.EQUALITY)
        age: int = Property(default=0, index=IndexType.RANGE)
        active: bool = Property(default=True)
        follows = OutEdge("Person")
        supplied_by = OutEdge(GlobalNS.Vendor)  # cross-namespace
```

Key features:
- **Type inference** — Python type annotations (`str`, `int`, `float`, `bool`,
  `datetime`, `bytes`) map to PelagoDB types automatically
- **Templated namespaces** — `tenant_{tenant_id}` resolves via `.bind(tenant_id="acme")`
- **Cross-namespace edges** — `OutEdge(GlobalNS.Vendor)` references entities in
  other namespaces by class

### Register and Create

```python
client = PelagoClient("127.0.0.1:27615")
client.register(GlobalNS)

acme = TenantNS.bind(tenant_id="acme")
client.register(acme)

acme_ns = client.ns(acme)
alice = acme_ns.create(acme.Person(name="Alice", age=31))
bob = acme_ns.create(acme.Person(name="Bob", age=29))
```

### Typed CRUD

```python
# Read
fetched = acme_ns.get(acme.Person, alice.id)
print(fetched.name)  # "Alice"

# Update
updated = acme_ns.update(alice, age=32)
print(updated.age)  # 32

# Delete
acme_ns.delete(bob)
```

### Filter Expressions and Query Builder

Property descriptors support operator overloading for filter expressions:

```python
# Simple filter scan
for p in acme_ns.find(acme.Person, acme.Person.age > 30):
    print(p.name)

# Compound filters
seniors = acme_ns.find(
    acme.Person,
    (acme.Person.age >= 30) & (acme.Person.active == True),
    limit=50
)

# Query builder with traversals
results = (
    acme_ns.query(acme.Person)
    .filter(acme.Person.name == "Alice")
    .traverse(acme.Person.follows, filter=acme.Person.age > 25)
    .limit(20)
    .run()
)
```

### Edges

```python
# Cross-namespace edge
global_ns = client.ns(GlobalNS)
vendor = global_ns.create(GlobalNS.Vendor(name="Acme Corp"))
client.link(alice, "supplied_by", vendor)

# Same-namespace edge
client.link(alice, "follows", bob)

# Remove edge
client.unlink(alice, "follows", bob)
```

### Async Watch Streams

Watch for real-time changes using `async with` / `async for`. The stream and
server subscription are automatically cancelled when the loop exits:

```python
import asyncio

async def monitor():
    acme_ns = client.ns(acme)

    async with acme_ns.watch_query(acme.Person, acme.Person.age > 30) as events:
        async for event in events:
            print(f"{event.type}: {event.node.name} age={event.node.age}")
            if event.type == WatchEventType.DELETE:
                break  # auto-cancels

asyncio.run(monitor())
```

Three watch scopes:
- `watch_node(model, node_id)` — single node changes
- `watch_query(model, filter_expr)` — query result changes
- `watch()` — all namespace changes

See `examples/typed_crud.py` for a complete end-to-end example.

## Related

- [Client SDK Guide](../reference/cli.md) — interface coverage
- [gRPC API Reference](../reference/grpc-api.md) — underlying API
- [Build a Social Graph](build-a-social-graph.md) — CLI-based tutorial
- [Elixir SDK Tutorial](elixir-sdk.md) — Elixir equivalent
- [Watch Live Changes](watch-live-changes.md) — watch tutorial
