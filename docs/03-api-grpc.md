# gRPC API

PelagoDB exposes gRPC services defined in `proto/pelago.proto` (package `pelago.v1`).

## Services

- `SchemaService`
- `NodeService`
- `EdgeService`
- `QueryService`
- `AdminService`
- `ReplicationService`
- `WatchService`
- `AuthService`
- `HealthService`

## Request Context

Most requests include:
- `database`
- `namespace`
- `site_id` (optional for client flows)
- `request_id` (optional but recommended)

Example context block:

```json
{"database":"default","namespace":"default","site_id":"","request_id":""}
```

## Authentication Headers

When server auth is required (`PELAGO_AUTH_REQUIRED=true`):
- API key: `x-api-key: <key>`
- Bearer token: `authorization: Bearer <token>`

`grpcurl` header examples:

```bash
grpcurl -plaintext \
  -H 'x-api-key: dev-admin-key' \
  127.0.0.1:27615 list pelago.v1
```

```bash
grpcurl -plaintext \
  -H 'authorization: Bearer <access-token>' \
  127.0.0.1:27615 list pelago.v1
```

## Streaming Endpoints

- `QueryService.FindNodes`
- `QueryService.Traverse`
- `QueryService.ExecutePQL`
- `EdgeService.ListEdges`
- `ReplicationService.PullCdcEvents`
- `WatchService.WatchPoint`
- `WatchService.WatchQuery`
- `WatchService.WatchNamespace`

## Pagination and Cursor Semantics

`FindNodes`, `Traverse`, and `ListEdges` support continuation tokens.

Pattern:
1. Call endpoint with empty `cursor`.
2. Read stream results.
3. Capture `next_cursor` from the last emitted message when present.
4. Re-issue request with `cursor` set to that value.

Important:
- `bytes` fields in grpcurl JSON are base64-encoded strings.
- No `next_cursor` means end-of-results.

### `FindNodes` Pagination Example

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

Resume with `next_cursor` (example value `AAECAw==`):

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

### `Traverse` Pagination Example

First page:

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

Resume with `next_cursor`:

```bash
grpcurl -plaintext \
  -d '{
    "context": {"database":"default","namespace":"default"},
    "start": {"entity_type":"Person","node_id":"1_0"},
    "steps": [{
      "edge_type": "follows",
      "direction": "EDGE_DIRECTION_OUTGOING"
    }],
    "max_depth": 2,
    "max_results": 100,
    "consistency": "READ_CONSISTENCY_STRONG",
    "cursor": "AAECAw=="
  }' \
  127.0.0.1:27615 pelago.v1.QueryService/Traverse
```

## Explain Plan Example (CEL)

```bash
grpcurl -plaintext \
  -d '{
    "context": {"database":"default","namespace":"default"},
    "entity_type": "Person",
    "cel_expression": "age >= 30 && active == true"
  }' \
  127.0.0.1:27615 pelago.v1.QueryService/Explain
```

## Compatibility Notes

- Source of truth for fields and enums is always `proto/pelago.proto`.
- SDK generation should pin protoc and plugin versions for reproducibility.
- Some operations are API-first and may not yet have CLI parity (`docs/02-cli-reference.md`).

## Basic `grpcurl` Discovery

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
