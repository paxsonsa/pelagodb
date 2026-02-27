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

## Related

- [Watch API Reference](../reference/watch-api.md) — complete RPC reference
- [CDC and Event Model](../concepts/cdc-and-event-model.md) — how watch is powered
- [Configuration Reference](../reference/configuration.md) — watch settings
