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

## Related

- [Client SDK Guide](../reference/cli.md) — interface coverage
- [gRPC API Reference](../reference/grpc-api.md) — underlying API
- [Build a Social Graph](build-a-social-graph.md) — CLI-based tutorial
- [Elixir SDK Tutorial](elixir-sdk.md) — Elixir equivalent
