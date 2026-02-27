# Security Setup

Operational guide for configuring authentication, API keys, mTLS, and authorization policies.

## Quick Start: Enable Auth

```bash
export PELAGO_AUTH_REQUIRED=true
export PELAGO_API_KEYS='admin-key:admin-user,app-key:app-user'
cargo run -p pelago-server --bin pelago-server
```

Verify unauthenticated access is blocked:
```bash
pelago schema list   # should return UNAUTHENTICATED
```

Verify authenticated access:
```bash
pelago --api-key admin-key schema list   # should succeed
```

## API Key Configuration

Format: `PELAGO_API_KEYS='key1:principal1,key2:principal2'`

Each key maps to a principal identifier used for authorization checks and audit logging.

## Bearer Token Workflow

1. Exchange API key for tokens:
```bash
grpcurl -plaintext \
  -d '{"api_key":"admin-key"}' \
  127.0.0.1:27615 pelago.v1.AuthService/Authenticate
```

2. Use the `access_token`:
```bash
pelago --bearer-token <access_token> admin sites
```

3. Refresh before expiry:
```bash
grpcurl -plaintext \
  -d '{"refresh_token":"<refresh_token>"}' \
  127.0.0.1:27615 pelago.v1.AuthService/RefreshToken
```

## mTLS Configuration

Enable mTLS for transport security and client authentication:

```bash
export PELAGO_TLS_CERT=/path/to/server.pem
export PELAGO_TLS_KEY=/path/to/server-key.pem
export PELAGO_TLS_CA=/path/to/ca.pem
export PELAGO_TLS_CLIENT_AUTH=require

export PELAGO_MTLS_ENABLED=true
export PELAGO_MTLS_SUBJECTS='CN=app-service=app-principal'
export PELAGO_MTLS_FINGERPRINTS='sha256:abc123=ops-principal'
export PELAGO_MTLS_DEFAULT_ROLE=service
```

## Authorization Policies

Create a read-only policy for a principal:

```bash
grpcurl -plaintext \
  -H 'x-api-key: admin-key' \
  -d '{
    "context": {"database":"default","namespace":"default"},
    "policy": {
      "policy_id": "reader-policy",
      "principal_id": "app-user",
      "permissions": [{
        "database": "default",
        "namespace": "default",
        "entity_type": "*",
        "actions": ["schema.read","node.read","edge.read","query.find","query.traverse","query.pql"]
      }]
    }
  }' \
  127.0.0.1:27615 pelago.v1.AuthService/CreatePolicy
```

Verify:
```bash
grpcurl -plaintext \
  -H 'x-api-key: admin-key' \
  -d '{
    "context": {"database":"default","namespace":"default"},
    "principal_id": "app-user",
    "action": "query.find",
    "database": "default",
    "namespace": "default",
    "entity_type": "Person"
  }' \
  127.0.0.1:27615 pelago.v1.AuthService/CheckPermission
```

## Replication Auth

When source sites require auth, configure the replicator:

```bash
export PELAGO_REPLICATION_API_KEY=replication-key
```

Ensure the replication key has at least `read-only` role or appropriate policy on the source site.

## Audit Monitoring

```bash
pelago admin audit --action authz.denied --limit 100
pelago admin audit --principal app-user --limit 50
```

Configure retention:
```bash
export PELAGO_AUDIT_ENABLED=true
export PELAGO_AUDIT_RETENTION_DAYS=90
```

## Production Security Profile

Use `scripts/production-env.example` as a starting point for security-first defaults.

## Related

- [Security Model](../concepts/security-model.md) — conceptual overview
- [Auth API Reference](../reference/auth-api.md) — RPC details
- [Configuration Reference](../reference/configuration.md) — all security settings
- [Production Checklist](production-checklist.md) — pre-production readiness
