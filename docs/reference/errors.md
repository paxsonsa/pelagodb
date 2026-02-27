# Errors Reference

PelagoDB error types, gRPC status code mappings, and troubleshooting guidance.

## Error Structure

PelagoDB errors are represented as `PelagoError` variants internally and mapped to gRPC status codes on the wire.

## gRPC Status Code Mapping

| gRPC Status | Meaning | Common Causes |
|---|---|---|
| `OK` (0) | Success | — |
| `INVALID_ARGUMENT` (3) | Request validation failed | Missing required fields, invalid schema, malformed CEL/PQL |
| `NOT_FOUND` (5) | Entity not found | Node/schema/policy doesn't exist |
| `ALREADY_EXISTS` (6) | Duplicate entity | Unique constraint violation |
| `PERMISSION_DENIED` (7) | Authorization failed | Insufficient permissions for action |
| `UNAUTHENTICATED` (16) | Authentication required | Missing or invalid API key/token |
| `RESOURCE_EXHAUSTED` (8) | Limit exceeded | Watch subscription limits, snapshot budget exceeded |
| `FAILED_PRECONDITION` (9) | Operation precondition not met | Schema validation, ownership conflict |
| `INTERNAL` (13) | Server error | FDB transaction failure, unexpected runtime error |
| `UNAVAILABLE` (14) | Service unavailable | Server starting up, FDB connectivity loss |
| `DEADLINE_EXCEEDED` (4) | Timeout | Query timeout, traversal timeout |

## Common Error Scenarios

### Schema Errors

| Error | Status | Cause |
|---|---|---|
| Missing `index_default_mode` | `INVALID_ARGUMENT` | Registration without `INDEX_DEFAULT_MODE_AUTO_BY_TYPE_V1` |
| Invalid schema name | `INVALID_ARGUMENT` | Name contains characters outside `[A-Za-z0-9_]` |
| Invalid sort key | `INVALID_ARGUMENT` | `sort_key` references nonexistent edge property |
| Independent ownership | `INVALID_ARGUMENT` | `ownership=independent` rejected in v1 |

### Node Errors

| Error | Status | Cause |
|---|---|---|
| Required field missing | `INVALID_ARGUMENT` | Required property absent or null |
| Type mismatch | `INVALID_ARGUMENT` | Value type doesn't match schema property type |
| Extra property rejected | `INVALID_ARGUMENT` | Undeclared property with `extras_policy: reject` |
| Unique constraint violation | `ALREADY_EXISTS` | Duplicate value on `unique` indexed property |
| Node not found | `NOT_FOUND` | Invalid entity type or node ID |

### Query Errors

| Error | Status | Cause |
|---|---|---|
| Invalid CEL expression | `INVALID_ARGUMENT` | Syntax error in CEL filter |
| PQL parse error | `INVALID_ARGUMENT` | Syntax error in PQL query |
| PQL compile error | `INVALID_ARGUMENT` | Unsupported directive combination |
| Snapshot budget exceeded | `RESOURCE_EXHAUSTED` | Query exceeded strict snapshot limits |
| Query timeout | `DEADLINE_EXCEEDED` | `timeout_ms` exceeded |

### Auth Errors

| Error | Status | Cause |
|---|---|---|
| No credentials | `UNAUTHENTICATED` | Auth required but no key/token provided |
| Invalid API key | `UNAUTHENTICATED` | Key not in `PELAGO_API_KEYS` |
| Expired token | `UNAUTHENTICATED` | Access token expired; refresh needed |
| Permission denied | `PERMISSION_DENIED` | Principal lacks required permission |

### Watch Errors

| Error | Status | Cause |
|---|---|---|
| Subscription limit | `RESOURCE_EXHAUSTED` | Max subscriptions reached |
| Queue overflow | `RESOURCE_EXHAUSTED` | Max dropped events exceeded |
| Invalid resume position | `INVALID_ARGUMENT` | Malformed `resume_after` versionstamp |

## Error Handling Best Practices

- Check gRPC status code first for routing error handling logic
- Use `INVALID_ARGUMENT` to detect client-side fixable errors
- Retry on `UNAVAILABLE` with backoff
- Refresh tokens on `UNAUTHENTICATED` with valid refresh token
- Check `Explain` plans when `RESOURCE_EXHAUSTED` occurs on queries

## Related

- [gRPC API Reference](grpc-api.md) — API surface
- [Troubleshooting](../operations/troubleshooting.md) — common issue resolution
- [Auth API Reference](auth-api.md) — auth error details
