# Security and Access

## Auth Modes

### Development Mode

- `PELAGO_AUTH_REQUIRED=false`
- Requests are accepted without credentials.

### Auth Required Mode

- `PELAGO_AUTH_REQUIRED=true`
- Supply one of:
  - `x-api-key` header
  - `authorization: Bearer <token>`

CLI equivalents:

```bash
pelago --api-key <key> schema list
pelago --bearer-token <token> admin audit --limit 20
```

## Auth Required End-to-End Tutorial

This is the shortest reproducible secure workflow for local/staging.

### 1) Start server with auth enabled

```bash
export PELAGO_FDB_CLUSTER=/usr/local/etc/foundationdb/fdb.cluster
export PELAGO_SITE_ID=1
export PELAGO_SITE_NAME=local
export PELAGO_LISTEN_ADDR=127.0.0.1:27615
export PELAGO_AUTH_REQUIRED=true
export PELAGO_API_KEYS='dev-admin-key:admin-user,dev-read-key:reader-user'

cargo run -p pelago-server --bin pelago-server
```

### 2) Verify unauthenticated access fails

```bash
pelago --server http://127.0.0.1:27615 schema list
```

Expected result: `UNAUTHENTICATED`.

### 3) Use API key auth from CLI

```bash
pelago --server http://127.0.0.1:27615 --api-key dev-admin-key schema list
```

### 4) Exchange API key for bearer and use token

Authenticate:

```bash
grpcurl -plaintext \
  -d '{"api_key":"dev-admin-key"}' \
  127.0.0.1:27615 pelago.v1.AuthService/Authenticate
```

Extract `access_token` from response JSON and use it:

```bash
pelago --server http://127.0.0.1:27615 --bearer-token <access_token> admin sites
```

Validate bearer:

```bash
grpcurl -plaintext \
  -d '{"token":"<access_token>"}' \
  127.0.0.1:27615 pelago.v1.AuthService/ValidateToken
```

Refresh bearer:

```bash
grpcurl -plaintext \
  -d '{"refresh_token":"<refresh_token>"}' \
  127.0.0.1:27615 pelago.v1.AuthService/RefreshToken
```

### 5) Create and verify a policy

Create policy:

```bash
grpcurl -plaintext \
  -H 'x-api-key: dev-admin-key' \
  -d '{
    "context": {"database":"default","namespace":"default"},
    "policy": {
      "policy_id": "reader-default-policy",
      "principal_id": "reader-user",
      "permissions": [{
        "database": "default",
        "namespace": "default",
        "entity_type": "*",
        "actions": ["schema.read","node.read","edge.read","query.find","query.traverse","query.pql","watch.subscribe"]
      }]
    }
  }' \
  127.0.0.1:27615 pelago.v1.AuthService/CreatePolicy
```

Check permission:

```bash
grpcurl -plaintext \
  -H 'x-api-key: dev-admin-key' \
  -d '{
    "context": {"database":"default","namespace":"default"},
    "principal_id": "reader-user",
    "action": "query.find",
    "database": "default",
    "namespace": "default",
    "entity_type": "Person"
  }' \
  127.0.0.1:27615 pelago.v1.AuthService/CheckPermission
```

### 6) Audit what happened

```bash
pelago --server http://127.0.0.1:27615 --api-key dev-admin-key admin audit --limit 100
```

## Auth Service

`AuthService` supports:
- `Authenticate`
- `RefreshToken`
- `ValidateToken`
- `CreatePolicy`
- `GetPolicy`
- `ListPolicies`
- `DeletePolicy`
- `CheckPermission`

Current surface note:
- CLI currently supports passing API key and bearer token, but does not yet expose auth lifecycle subcommands.
- Use `grpcurl` or SDK stubs for auth/policy RPC operations.

## Authorization Model

Authorization checks are applied in service handlers through `authorize()` (`crates/pelago-api/src/authz.rs`) with:
- role shortcuts (`admin`, `read-only`, `namespace-admin`)
- policy-based permission checks from storage
- audit events for allow/deny decisions

## Built-In Role Behaviors

- `admin`: full access
- `read-only`: read/query/watch/replication pull actions
- `namespace-admin`:
  - global namespace admin role
  - or scoped `namespace-admin:<database>:<namespace>`

## Audit

Audit events are appended for auth/authz and replication conflict actions.

Query examples:

```bash
pelago admin audit --principal admin-user --limit 200
pelago admin audit --action authz.denied --limit 200
```

Retention is enforced by background cleanup when enabled:
- `PELAGO_AUDIT_ENABLED=true`
- `PELAGO_AUDIT_RETENTION_DAYS`
