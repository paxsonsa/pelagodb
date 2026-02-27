# Security Model

PelagoDB's security model covers authentication, authorization, and audit as first-class runtime concerns — not afterthoughts.

## Auth Modes

### Development Mode

- `PELAGO_AUTH_REQUIRED=false`
- Requests are accepted without credentials.
- Suitable for local development and prototyping.

### Auth Required Mode

- `PELAGO_AUTH_REQUIRED=true`
- Every request must supply one of:
  - `x-api-key` header
  - `authorization: Bearer <token>` header

## Authentication Methods

### API Keys

Static keys configured via `PELAGO_API_KEYS` environment variable:

```bash
PELAGO_API_KEYS='admin-key:admin-user,read-key:reader-user'
```

Each key maps to a principal identifier. Keys are passed via the `x-api-key` header.

### Bearer Tokens

Obtained by exchanging an API key through `AuthService.Authenticate`. Returns an access/refresh token pair. Tokens are passed via the `authorization: Bearer <token>` header.

### mTLS

When `PELAGO_MTLS_ENABLED=true`, client certificates are validated and mapped to principals:

- Subject-based: `PELAGO_MTLS_SUBJECTS` (`subject=principal` CSV)
- Fingerprint-based: `PELAGO_MTLS_FINGERPRINTS` (`sha256:fingerprint=principal` CSV)
- Default role for mTLS principals: `PELAGO_MTLS_DEFAULT_ROLE` (default: `service`)

## Authorization Model

Authorization checks run at service boundaries through `authorize()` in `crates/pelago-api/src/authz.rs`.

Three mechanisms:

1. **Role shortcuts:** `admin`, `read-only`, `namespace-admin`
2. **Policy-based checks:** Permission policies stored in FDB, evaluated per request
3. **Audit recording:** Every allow/deny decision is logged

### Built-In Roles

| Role | Access |
|---|---|
| `admin` | Full access to all operations |
| `read-only` | Read, query, watch, and replication pull |
| `namespace-admin` | Global or scoped (`namespace-admin:<database>:<namespace>`) |

### Policy Structure

Policies bind a principal to specific permissions:

```json
{
  "policy_id": "reader-default-policy",
  "principal_id": "reader-user",
  "permissions": [{
    "database": "default",
    "namespace": "default",
    "entity_type": "*",
    "actions": ["schema.read", "node.read", "edge.read", "query.find", "query.traverse"]
  }]
}
```

## Audit

Audit events are appended for:
- Authentication attempts (success/failure)
- Authorization decisions (allow/deny)
- Replication conflict actions
- Administrative operations

Audit retention is controlled by:
- `PELAGO_AUDIT_ENABLED` (default: `true`)
- `PELAGO_AUDIT_RETENTION_DAYS` (default: `90`)

Query audit events:

```bash
pelago admin audit --principal admin-user --limit 200
pelago admin audit --action authz.denied --limit 200
```

## Design Philosophy

- **Production controls without blocking prototyping:** Auth is optional in dev, required in production.
- **Explicit deny visibility:** Every denied request is auditable, enabling security posture assessment.
- **Per-entity ownership:** Mutation safety is enforced below the namespace level, complementing auth-level access control.

## Related

- [Auth API Reference](../reference/auth-api.md) — RPC details
- [Security Setup](../operations/security-setup.md) — operational configuration
- [Configuration Reference](../reference/configuration.md) — all security settings
