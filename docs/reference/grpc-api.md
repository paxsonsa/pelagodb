# gRPC API Reference

PelagoDB exposes gRPC services defined in `proto/pelago.proto` (package `pelago.v1`).

## Services

| Service | Purpose |
|---|---|
| `SchemaService` | Schema registration, retrieval, listing |
| `NodeService` | Node CRUD, ownership transfer |
| `EdgeService` | Edge creation, deletion, listing |
| `QueryService` | CEL find, traversal, PQL execution, explain |
| `AdminService` | Jobs, sites, replication status, audit, drops |
| `ReplicationService` | CDC pull for multi-site replication |
| `WatchService` | Change subscriptions (see [Watch API](watch-api.md)) |
| `AuthService` | Authentication and authorization (see [Auth API](auth-api.md)) |
| `HealthService` | Health checks |

## Request Context

Most requests include a context block:

```json
{"database": "default", "namespace": "default", "site_id": "", "request_id": ""}
```

| Field | Required | Notes |
|---|---|---|
| `database` | Yes | Target database |
| `namespace` | Yes | Target namespace |
| `site_id` | Optional | For client flows |
| `request_id` | Optional | Recommended for tracing |

## Authentication Headers

When server auth is required (`PELAGO_AUTH_REQUIRED=true`):

```bash
# API key
grpcurl -plaintext -H 'x-api-key: dev-admin-key' 127.0.0.1:27615 list pelago.v1

# Bearer token
grpcurl -plaintext -H 'authorization: Bearer <access-token>' 127.0.0.1:27615 list pelago.v1
```

## Streaming Endpoints

These endpoints return server-streaming responses:

- `QueryService.FindNodes`
- `QueryService.Traverse`
- `QueryService.ExecutePQL`
- `EdgeService.ListEdges`
- `ReplicationService.PullCdcEvents`
- `WatchService.WatchPoint`
- `WatchService.WatchQuery`
- `WatchService.WatchNamespace`

## Pagination and Cursors

`FindNodes`, `Traverse`, and `ListEdges` support continuation tokens.

Pattern:
1. Call endpoint with empty `cursor`.
2. Read streamed results.
3. Capture `next_cursor` from the last emitted message.
4. Re-issue request with `cursor` set to that value.

Important:
- `bytes` fields in grpcurl JSON are base64-encoded strings.
- No `next_cursor` means end-of-results.

### FindNodes Pagination

First page:

```bash
grpcurl -plaintext \
  -d '{
    "context": {"database":"default","namespace":"default"},
    "entity_type": "Person",
    "cel_expression": "age >= 30",
    "consistency": "READ_CONSISTENCY_STRONG",
    "limit": 2
  }' \
  127.0.0.1:27615 pelago.v1.QueryService/FindNodes
```

Resume with `next_cursor`:

```bash
grpcurl -plaintext \
  -d '{
    "context": {"database":"default","namespace":"default"},
    "entity_type": "Person",
    "cel_expression": "age >= 30",
    "consistency": "READ_CONSISTENCY_STRONG",
    "limit": 2,
    "cursor": "AAECAw=="
  }' \
  127.0.0.1:27615 pelago.v1.QueryService/FindNodes
```

### Traverse Pagination

```bash
grpcurl -plaintext \
  -d '{
    "context": {"database":"default","namespace":"default"},
    "start": {"entity_type":"Person","node_id":"1_0"},
    "steps": [{
      "edge_type": "follows",
      "direction": "EDGE_DIRECTION_OUTGOING",
      "node_filter": "active == true",
      "per_node_limit": 50
    }],
    "max_depth": 2,
    "max_results": 100,
    "consistency": "READ_CONSISTENCY_STRONG"
  }' \
  127.0.0.1:27615 pelago.v1.QueryService/Traverse
```

## Explain Plan

```bash
grpcurl -plaintext \
  -d '{
    "context": {"database":"default","namespace":"default"},
    "entity_type": "Person",
    "cel_expression": "age >= 30 && active == true"
  }' \
  127.0.0.1:27615 pelago.v1.QueryService/Explain
```

## Service Discovery

List services:

```bash
grpcurl -plaintext 127.0.0.1:27615 list pelago.v1
```

Describe a service:

```bash
grpcurl -plaintext 127.0.0.1:27615 describe pelago.v1.QueryService
```

Health check:

```bash
grpcurl -plaintext \
  -d '{"service":"pelago"}' \
  127.0.0.1:27615 pelago.v1.HealthService/Check
```

## Compatibility Notes

- Source of truth for fields and enums is always `proto/pelago.proto`.
- SDK generation should pin protoc and plugin versions for reproducibility.
- Some operations are API-first and may not yet have CLI parity (see [CLI Reference](cli.md)).

## Related

- [CLI Reference](cli.md) — command-line interface
- [Watch API](watch-api.md) — change subscriptions
- [Auth API](auth-api.md) — authentication and authorization
- [CEL Filters](cel-filters.md) — expression syntax
- [PQL Reference](pql.md) — graph query language
