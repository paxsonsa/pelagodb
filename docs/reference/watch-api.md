# Watch API Reference

The `WatchService` provides real-time change subscriptions for cache invalidation, async workflows, live UIs, and integration bridges.

Source of truth: `WatchService` in `proto/pelago.proto`

## RPCs

| RPC | Type | Description |
|---|---|---|
| `WatchPoint` | Server-streaming | Subscribe to one node by `(entity_type, node_id)` |
| `WatchQuery` | Server-streaming | Subscribe to a dynamic CEL or PQL query |
| `WatchNamespace` | Server-streaming | Subscribe to all changes in a namespace |
| `ListSubscriptions` | Unary | Inspect active subscriptions |
| `CancelSubscription` | Unary | Cancel by subscription ID |

## Event Semantics

Each `WatchEvent` includes:

| Field | Type | Notes |
|---|---|---|
| `subscription_id` | `string` | Identifies the subscription |
| `type` | enum | `ENTER`, `UPDATE`, `EXIT`, `DELETE` |
| `versionstamp` | `bytes` | Ordered position for resume |
| `node` / `edge` | message | Payload when available |
| `reason` | `string` | Human-readable change reason |

## Resume Model

Use `versionstamp` from the last processed event as `resume_after` on reconnection. In grpcurl JSON, `bytes` values are base64 strings.

## WatchPoint Example

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

## WatchQuery Example (CEL)

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

## WatchQuery Example (PQL)

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

## WatchNamespace Example

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

If last received event had `versionstamp` `AAECAwQFBgcICQ==`:

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

| Variable | Controls |
|---|---|
| `PELAGO_WATCH_MAX_SUBSCRIPTIONS` | Global subscription limit |
| `PELAGO_WATCH_MAX_NAMESPACE_SUBSCRIPTIONS` | Per-namespace limit |
| `PELAGO_WATCH_MAX_QUERY_WATCHES` | Query watch limit |
| `PELAGO_WATCH_MAX_PRINCIPAL_SUBSCRIPTIONS` | Per-principal limit |
| `PELAGO_WATCH_MAX_TTL_SECS` | Maximum subscription TTL |
| `PELAGO_WATCH_MAX_QUEUE_SIZE` | Per-subscription queue depth |
| `PELAGO_WATCH_MAX_DROPPED_EVENTS` | Dropped event threshold |

Operational guidance:
- Prefer point watches over query/namespace watches — they are cheaper.
- Keep TTL bounded and reconnect explicitly from clients.
- Alert on dropped events and reconnect with `resume_after` on consumer restart.

## Current Gaps

- CLI does not yet expose watch RPCs directly.
- Use `grpcurl` or generated SDK stubs for watch workflows.

## Related

- [CDC and Event Model](../concepts/cdc-and-event-model.md) — how watch is powered by CDC
- [Watch Live Changes Tutorial](../tutorials/watch-live-changes.md) — hands-on tutorial
- [Configuration Reference](configuration.md) — all watch settings
- [Troubleshooting](../operations/troubleshooting.md) — watch stream issues
