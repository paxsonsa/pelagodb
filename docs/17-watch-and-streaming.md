# Watch and Streaming Guide

PelagoDB watch APIs are for change-driven systems: cache invalidation, async workflows, live UIs, and integration bridges.

Source of truth:
- `WatchService` in `proto/pelago.proto`
- server implementation in `crates/pelago-api/src/watch_service.rs`

## Watch Types

- `WatchPoint`: subscribe to one node by `(entity_type, node_id)`
- `WatchQuery`: subscribe to a dynamic query (CEL or PQL)
- `WatchNamespace`: subscribe to all changes in a namespace
- `ListSubscriptions`: inspect active subscriptions
- `CancelSubscription`: cancel by subscription ID

## Event Semantics

Each `WatchEvent` includes:
- `subscription_id`
- `type` (`ENTER`, `UPDATE`, `EXIT`, `DELETE`)
- `versionstamp` (bytes)
- `node` or `edge` payload (when available)
- `reason`

Resume model:
- Use `versionstamp` from the last event you processed.
- Send it back as `resume_after` on new watch requests.
- In grpcurl JSON, `bytes` values are base64 strings.

## `WatchPoint` Example

```bash
grpcurl -plaintext \
  -H 'x-api-key: dev-admin-key' \
  -d '{
    "context": {"database":"default","namespace":"default"},
    "entity_type": "Person",
    "node_id": "1_0",
    "options": {
      "include_initial": true,
      "ttl_secs": 900,
      "max_queue_size": 1000
    }
  }' \
  127.0.0.1:27615 pelago.v1.WatchService/WatchPoint
```

## `WatchQuery` Example (CEL)

```bash
grpcurl -plaintext \
  -H 'x-api-key: dev-admin-key' \
  -d '{
    "context": {"database":"default","namespace":"default"},
    "entity_type": "Person",
    "cel_expression": "age >= 30 && active == true",
    "options": {
      "include_initial": true,
      "ttl_secs": 600,
      "max_queue_size": 2000
    }
  }' \
  127.0.0.1:27615 pelago.v1.WatchService/WatchQuery
```

## `WatchQuery` Example (PQL)

```bash
grpcurl -plaintext \
  -H 'x-api-key: dev-admin-key' \
  -d '{
    "context": {"database":"default","namespace":"default"},
    "entity_type": "Person",
    "pql_query": "Person @filter(age >= 30) { uid name age }",
    "options": {
      "include_initial": false,
      "ttl_secs": 300,
      "max_queue_size": 1000
    }
  }' \
  127.0.0.1:27615 pelago.v1.WatchService/WatchQuery
```

## `WatchNamespace` Example

```bash
grpcurl -plaintext \
  -H 'x-api-key: dev-admin-key' \
  -d '{
    "context": {"database":"default","namespace":"default"},
    "options": {
      "include_initial": false,
      "ttl_secs": 300,
      "max_queue_size": 2000
    }
  }' \
  127.0.0.1:27615 pelago.v1.WatchService/WatchNamespace
```

## Resume Example

If last received event had `versionstamp` `AAECAwQFBgcICQ==`, reconnect with:

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

## List and Cancel Subscriptions

List:

```bash
grpcurl -plaintext \
  -H 'x-api-key: dev-admin-key' \
  -d '{"context":{"database":"default","namespace":"default"}}' \
  127.0.0.1:27615 pelago.v1.WatchService/ListSubscriptions
```

Cancel:

```bash
grpcurl -plaintext \
  -H 'x-api-key: dev-admin-key' \
  -d '{
    "context":{"database":"default","namespace":"default"},
    "subscription_id":"<id-from-list>"
  }' \
  127.0.0.1:27615 pelago.v1.WatchService/CancelSubscription
```

## Tuning and Limits

Key watch limits:
- `PELAGO_WATCH_MAX_SUBSCRIPTIONS`
- `PELAGO_WATCH_MAX_NAMESPACE_SUBSCRIPTIONS`
- `PELAGO_WATCH_MAX_QUERY_WATCHES`
- `PELAGO_WATCH_MAX_PRINCIPAL_SUBSCRIPTIONS`
- `PELAGO_WATCH_MAX_TTL_SECS`
- `PELAGO_WATCH_MAX_QUEUE_SIZE`
- `PELAGO_WATCH_MAX_DROPPED_EVENTS`

Operational guidance:
- Prefer query/namespace watch only where necessary; point watches are cheaper.
- Keep TTL bounded and reconnect explicitly from clients.
- Alert on dropped events and reconnect with `resume_after` on consumer restart.

## Current Surface Gaps

- CLI does not yet expose watch RPCs directly.
- Use `grpcurl` or generated SDK stubs for watch workflows.
