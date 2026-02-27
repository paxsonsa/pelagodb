# Auth API Reference

The `AuthService` provides authentication, token management, and policy-based authorization.

Source of truth: `AuthService` in `proto/pelago.proto`

## RPCs

| RPC | Description |
|---|---|
| `Authenticate` | Exchange API key for access/refresh token pair |
| `RefreshToken` | Exchange refresh token for new access token |
| `ValidateToken` | Validate a bearer token |
| `CreatePolicy` | Create an authorization policy |
| `GetPolicy` | Retrieve a policy by ID |
| `ListPolicies` | List policies for a principal |
| `DeletePolicy` | Remove a policy |
| `CheckPermission` | Check if a principal has a specific permission |

## Authentication Flow

### 1) API Key Authentication

```bash
pelago --api-key dev-admin-key schema list
```

Or via gRPC header:

```bash
grpcurl -plaintext \
  -H 'x-api-key: dev-admin-key' \
  127.0.0.1:27615 pelago.v1.SchemaService/ListSchemas
```

### 2) Token Exchange

Authenticate with API key to get tokens:

```bash
grpcurl -plaintext \
  -d '{"api_key":"dev-admin-key"}' \
  127.0.0.1:27615 pelago.v1.AuthService/Authenticate
```

Use the returned `access_token`:

```bash
pelago --bearer-token <access_token> admin sites
```

### 3) Token Validation

```bash
grpcurl -plaintext \
  -d '{"token":"<access_token>"}' \
  127.0.0.1:27615 pelago.v1.AuthService/ValidateToken
```

### 4) Token Refresh

```bash
grpcurl -plaintext \
  -d '{"refresh_token":"<refresh_token>"}' \
  127.0.0.1:27615 pelago.v1.AuthService/RefreshToken
```

## Policy Management

### Create a Policy

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

### Check Permission

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

## Built-In Roles

| Role | Access |
|---|---|
| `admin` | Full access to all operations |
| `read-only` | Read, query, watch, and replication pull actions |
| `namespace-admin` | Global namespace admin, or scoped `namespace-admin:<database>:<namespace>` |

## Authorization Model

Authorization checks run in service handlers through `authorize()` with:
- Role-based shortcuts (`admin`, `read-only`, `namespace-admin`)
- Policy-based permission checks from storage
- Audit events for allow/deny decisions

## Audit

Audit events are appended for auth/authz and replication conflict actions.

```bash
pelago admin audit --principal admin-user --limit 200
pelago admin audit --action authz.denied --limit 200
```

Retention is controlled by:
- `PELAGO_AUDIT_ENABLED=true`
- `PELAGO_AUDIT_RETENTION_DAYS`

## Related

- [Security Model](../concepts/security-model.md) — conceptual overview
- [Security Setup](../operations/security-setup.md) — operational configuration
- [Configuration Reference](configuration.md) — all security settings
