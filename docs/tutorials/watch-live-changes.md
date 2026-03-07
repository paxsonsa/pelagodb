# Tutorial: Watch Live Changes

Learn to use PelagoDB's watch subscriptions for real-time change notifications.

## Prerequisites

- PelagoDB server running with auth and data
- `grpcurl` installed (watch is API-only; no CLI support yet)

## Concepts

Watch subscriptions let you receive real-time notifications when data changes. Three subscription types:

| Type | Scope | Use Case |
|---|---|---|
| **WatchPoint** | Single node | Track a specific entity |
| **WatchQuery** | CEL or PQL query | Track query result changes |
| **WatchNamespace** | All namespace changes | Broad change feed |

## Step 1: Watch a Single Node

Open a terminal and subscribe to a specific node:

```bash
grpcurl -plaintext \
  -H 'x-api-key: dev-admin-key' \
  -d '{
    "context": {"database":"default","namespace":"default"},
    "entity_type": "Person",
    "node_id": "1_0",
    "options": {
      "include_initial": true,
      "ttl_secs": 300,
      "max_queue_size": 100
    }
  }' \
  127.0.0.1:27615 pelago.v1.WatchService/WatchPoint
```

In another terminal, update the node:

```bash
pelago node update Person 1_0 --props '{"age":33}'
```

You'll see a `WatchEvent` with type `UPDATE` appear in the watch stream.

## Step 2: Watch a Query

Subscribe to changes matching a CEL filter:

```bash
grpcurl -plaintext \
  -H 'x-api-key: dev-admin-key' \
  -d '{
    "context": {"database":"default","namespace":"default"},
    "entity_type": "Person",
    "cel_expression": "age >= 30",
    "options": {
      "include_initial": true,
      "ttl_secs": 300,
      "max_queue_size": 500
    }
  }' \
  127.0.0.1:27615 pelago.v1.WatchService/WatchQuery
```

Events you'll see:
- **ENTER** — a node newly matches the query (e.g., age updated from 29 to 31)
- **UPDATE** — a matching node is modified
- **EXIT** — a node no longer matches (e.g., age updated below 30)
- **DELETE** — a matching node is deleted

## Step 3: Watch a Namespace

Subscribe to all changes in a namespace:

```bash
grpcurl -plaintext \
  -H 'x-api-key: dev-admin-key' \
  -d '{
    "context": {"database":"default","namespace":"default"},
    "options": {
      "include_initial": false,
      "ttl_secs": 120,
      "max_queue_size": 1000
    }
  }' \
  127.0.0.1:27615 pelago.v1.WatchService/WatchNamespace
```

Create, update, or delete any node in the namespace and watch events flow.

## Step 4: Resume After Disconnect

Each event includes a `versionstamp`. Use it to resume after reconnecting:

```bash
grpcurl -plaintext \
  -H 'x-api-key: dev-admin-key' \
  -d '{
    "context": {"database":"default","namespace":"default"},
    "entity_type": "Person",
    "cel_expression": "active == true",
    "resume_after": "AAECAwQFBgcICQ==",
    "options": {"ttl_secs": 300}
  }' \
  127.0.0.1:27615 pelago.v1.WatchService/WatchQuery
```

## Step 5: Manage Subscriptions

List active subscriptions:

```bash
grpcurl -plaintext \
  -H 'x-api-key: dev-admin-key' \
  -d '{"context":{"database":"default","namespace":"default"}}' \
  127.0.0.1:27615 pelago.v1.WatchService/ListSubscriptions
```

Cancel a subscription:

```bash
grpcurl -plaintext \
  -H 'x-api-key: dev-admin-key' \
  -d '{
    "context":{"database":"default","namespace":"default"},
    "subscription_id":"<id>"
  }' \
  127.0.0.1:27615 pelago.v1.WatchService/CancelSubscription
```

## Best Practices

- Use **point watches** for tracking specific entities — they're cheapest
- Use **query watches** for monitoring result set changes
- Use **namespace watches** sparingly — they generate the most events
- Always set `ttl_secs` and reconnect explicitly
- Store the last `versionstamp` and use `resume_after` on reconnect
- Monitor dropped events and increase `max_queue_size` if needed

## Python SDK

The Python SDK provides typed async watch streams. Breaking out of the loop
or exiting the `async with` block automatically cancels both the gRPC stream
and the server-side subscription.

### Watch a Query

```python
import asyncio
from pelagodb import PelagoClient, Namespace, Entity, Property, IndexType, WatchEventType

class MyNS(Namespace):
    name = "default"

    class Person(Entity):
        name: str = Property(required=True)
        age: int = Property(default=0, index=IndexType.RANGE)

async def main():
    client = PelagoClient("127.0.0.1:27615")
    ns = client.ns(MyNS)

    async with ns.watch_query(MyNS.Person, MyNS.Person.age >= 30) as events:
        async for event in events:
            print(f"{event.type}: {event.node.name} (age={event.node.age})")
            # ENTER — node newly matches
            # UPDATE — matching node changed
            # EXIT — node no longer matches
            # DELETE — matching node deleted

asyncio.run(main())
```

### Watch a Specific Node

```python
async with ns.watch_node(MyNS.Person, "1_0", include_initial=True) as events:
    async for event in events:
        print(f"{event.type}: {event.node}")
```

### Watch an Entire Namespace

```python
async with ns.watch(include_initial=False) as events:
    async for event in events:
        print(f"{event.type}: {event.raw}")  # untyped events
```

### Resume After Disconnect

Store the `versionstamp` from the last event and pass it on reconnect:

```python
last_vs = b""
async with ns.watch_query(MyNS.Person, MyNS.Person.age >= 30, resume_after=last_vs) as events:
    async for event in events:
        last_vs = event.versionstamp
        print(event.type, event.node.name)
```

## Related

- [Watch API Reference](../reference/watch-api.md) — complete RPC reference
- [CDC and Event Model](../concepts/cdc-and-event-model.md) — how watch is powered
- [Configuration Reference](../reference/configuration.md) — watch settings
- [Python SDK Tutorial](python-sdk.md) — full Python SDK tutorial
