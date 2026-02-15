## 18. Security and Authorization

This section defines the authentication, authorization, and audit model for PelagoDB. The design follows HashiCorp Vault's path-based ACL paradigm, mapping permission paths directly to the FDB keyspace structure.

### 18.1 Security Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              Client Request                                  │
│                    (API Key / mTLS / Service Token)                         │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                          Authentication Layer                                │
│   • API Key validation                                                       │
│   • mTLS certificate verification                                            │
│   • Service account token validation                                         │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                          Identity Resolution                                 │
│   • Map credential → Principal (user, service, or role)                     │
│   • Load attached policies                                                   │
│   • Resolve effective permissions                                            │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                          Authorization Engine                                │
│   • Extract target path from request                                         │
│   • Match path against policy rules                                          │
│   • Evaluate capabilities (read, write, admin)                              │
│   • Return allow/deny decision                                               │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                         ┌────────────┴────────────┐
                         ▼                         ▼
                    ┌─────────┐              ┌─────────┐
                    │ ALLOWED │              │ DENIED  │
                    └────┬────┘              └────┬────┘
                         │                        │
                         ▼                        ▼
                   Execute RPC              Return ERR_PERMISSION_DENIED
                         │
                         ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                            Audit Logger                                      │
│   • Log operation, principal, path, outcome                                  │
│   • Emit to audit log stream                                                 │
└─────────────────────────────────────────────────────────────────────────────┘
```

**Security Boundaries:**

| Boundary | Enforcement | Description |
|----------|-------------|-------------|
| Database | Namespace isolation | Each database is a logical isolation unit |
| Namespace | Path-based ACL | Permissions scoped to namespace subpaths |
| Entity Type | Path-based ACL | Permissions can target specific entity types |
| Entity | Not enforced | No per-entity ACLs in v1 (future consideration) |

### 18.2 Authentication Mechanisms

PelagoDB supports three authentication methods, evaluated in order of precedence.

#### 18.2.1 API Keys

API keys are the primary authentication method for programmatic access.

**Format:** `plg_<version>_<random_bytes>`

```
plg_v1_a3f8c9d2e1b4a5f6c7d8e9f0a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8
└─┬─┘└┬┘└──────────────────────────────────────────────────────────────┘
prefix ver                    32 random bytes (base62)
```

**API Key Record:**

```json
{
  "key_id": "plgk_abc123",
  "key_hash": "<argon2id hash>",
  "name": "production-app",
  "principal_id": "usr_12345",
  "policies": ["production-read", "analytics-write"],
  "created_at": "<timestamp>",
  "expires_at": "<timestamp or null>",
  "last_used_at": "<timestamp>",
  "metadata": {
    "created_by": "usr_admin",
    "purpose": "ETL pipeline"
  }
}
```

**Header Transport:**

```
Authorization: Bearer plg_v1_a3f8c9d2e1b4a5f6c7d8e9f0a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8
```

#### 18.2.2 mTLS (Mutual TLS)

For service-to-service communication in zero-trust environments.

**Certificate Requirements:**

| Field | Requirement |
|-------|-------------|
| Subject CN | Must match registered service name |
| SAN | Optional DNS/IP entries for additional validation |
| Key Usage | Digital Signature, Key Encipherment |
| Extended Key Usage | Client Authentication |
| Validity | Max 90 days recommended |

**Certificate-to-Principal Mapping:**

```json
{
  "certificate_fingerprint": "sha256:abc123...",
  "subject_cn": "etl-service.internal",
  "principal_id": "svc_etl_001",
  "policies": ["etl-full-access"],
  "valid_until": "<certificate expiry>",
  "issuer_fingerprint": "sha256:def456..."
}
```

**Server Configuration:**

| Environment Variable | Default | Description |
|---------------------|---------|-------------|
| `PELAGO_TLS_CERT` | (required for mTLS) | Server certificate path |
| `PELAGO_TLS_KEY` | (required for mTLS) | Server private key path |
| `PELAGO_TLS_CA` | (required for mTLS) | CA certificate for client verification |
| `PELAGO_TLS_CLIENT_AUTH` | `require` | Client auth mode: `none`, `request`, `require` |

#### 18.2.3 Service Account Tokens

For internal services and operators requiring elevated privileges.

**Token Format:** JWT with PelagoDB-specific claims

```json
{
  "iss": "pelago:auth",
  "sub": "svc_replicator_sf",
  "aud": "pelago:api",
  "exp": 1707926400,
  "iat": 1707840000,
  "jti": "tok_unique_id",
  "pelago": {
    "principal_type": "service",
    "policies": ["replication-admin"],
    "site_id": 1,
    "capabilities": ["cross_site_read", "cdc_subscribe"]
  }
}
```

**Token Signing:**

- Algorithm: EdDSA (Ed25519) or RS256
- Key rotation: Automatic via JWKS endpoint
- Max lifetime: 24 hours for service tokens

### 18.3 Principal Model

A **Principal** represents an authenticated identity with attached policies.

#### 18.3.1 Principal Types

| Type | Prefix | Description | Use Case |
|------|--------|-------------|----------|
| `user` | `usr_` | Human operator | CLI, admin console |
| `service` | `svc_` | Application service | API integration |
| `operator` | `opr_` | System operator | Replication, maintenance |
| `anonymous` | `anon` | Unauthenticated | Public read endpoints (if enabled) |

#### 18.3.2 Principal Record

```json
{
  "principal_id": "usr_12345",
  "type": "user",
  "name": "alice@example.com",
  "policies": ["default", "production-read"],
  "inline_policy": null,
  "metadata": {
    "department": "engineering",
    "team": "platform"
  },
  "created_at": "<timestamp>",
  "updated_at": "<timestamp>",
  "disabled": false
}
```

#### 18.3.3 Policy Attachment

Principals can have:

1. **Named policies:** References to stored policy documents
2. **Inline policy:** A single policy document embedded in the principal record

Effective permissions are the **union** of all attached policies.

### 18.4 Path-Based Permission Model

PelagoDB uses a path-based ACL system inspired by HashiCorp Vault, where permission paths map directly to the FDB keyspace structure.

#### 18.4.1 Path Syntax

Permission paths follow the FDB keyspace hierarchy:

```
<database>/<namespace>/<subspace>/<entity_type>/<node_id>
```

**Path Components:**

| Component | Description | Example Values |
|-----------|-------------|----------------|
| `<database>` | Database identifier | `production`, `staging`, `*` |
| `<namespace>` | Namespace within database | `core`, `analytics`, `*` |
| `<subspace>` | FDB subspace | `data`, `idx`, `edge`, `_schema`, `*` |
| `<entity_type>` | Entity type name | `Person`, `Company`, `*` |
| `<node_id>` | Specific node (rarely used) | `1_42`, `*` |

**Wildcard Semantics:**

| Wildcard | Meaning | Example |
|----------|---------|---------|
| `*` | Match any single segment | `*/core/data/*` matches `production/core/data/Person` |
| `+` | Match one or more segments (glob) | `production/+` matches `production/core/data/Person/1_42` |

#### 18.4.2 Path Examples

```
# All data in production database
production/*/data/*

# Person entities in core namespace (any database)
*/core/data/Person/*

# Full access to analytics namespace
*/analytics/+

# Read schemas in any namespace
*/*/\_schema/*

# Edge operations in production/core
production/core/edge/*

# Index access for Person type
*/*/idx/Person/*

# CDC stream access
production/core/\_cdc/*

# Admin operations (namespace metadata)
production/*/_ns/*
```

#### 18.4.3 Capability Types

| Capability | Description | Typical Operations |
|------------|-------------|-------------------|
| `read` | Read data at path | `GetNode`, `FindNodes`, `ListEdges`, `GetSchema` |
| `write` | Create/update/delete at path | `CreateNode`, `UpdateNode`, `DeleteNode`, `CreateEdge` |
| `admin` | Administrative operations | `RegisterSchema`, `DropIndex`, `DropEntityType` |
| `deny` | Explicit deny (overrides allow) | Block specific paths |

**Capability Hierarchy:**

```
admin ⊃ write ⊃ read
```

`admin` implies `write` and `read`. `write` implies `read`.

#### 18.4.4 Path-to-Operation Mapping

| gRPC Operation | Required Path | Required Capability |
|----------------|---------------|---------------------|
| `GetNode` | `{db}/{ns}/data/{type}/*` | `read` |
| `CreateNode` | `{db}/{ns}/data/{type}/*` | `write` |
| `UpdateNode` | `{db}/{ns}/data/{type}/*` | `write` |
| `DeleteNode` | `{db}/{ns}/data/{type}/*` | `write` |
| `FindNodes` | `{db}/{ns}/data/{type}/*` | `read` |
| `ListEdges` | `{db}/{ns}/edge/*` | `read` |
| `CreateEdge` | `{db}/{ns}/edge/*` | `write` |
| `DeleteEdge` | `{db}/{ns}/edge/*` | `write` |
| `Traverse` | `{db}/{ns}/data/*` + `{db}/{ns}/edge/*` | `read` |
| `GetSchema` | `{db}/{ns}/_schema/*` | `read` |
| `RegisterSchema` | `{db}/{ns}/_schema/*` | `admin` |
| `ListSchemas` | `{db}/{ns}/_schema/*` | `read` |
| `DropIndex` | `{db}/{ns}/idx/*` | `admin` |
| `DropEntityType` | `{db}/{ns}/data/{type}/*` | `admin` |
| `DropNamespace` | `{db}/{ns}/*` | `admin` |
| `GetJobStatus` | `{db}/{ns}/_jobs/*` | `read` |

### 18.5 Policy Definition

Policies are JSON documents that define a set of path rules.

#### 18.5.1 Policy Structure

```json
{
  "name": "production-read-only",
  "version": 1,
  "description": "Read-only access to production database",
  "rules": [
    {
      "path": "production/*/data/*",
      "capabilities": ["read"]
    },
    {
      "path": "production/*/edge/*",
      "capabilities": ["read"]
    },
    {
      "path": "production/*/_schema/*",
      "capabilities": ["read"]
    }
  ],
  "created_at": "<timestamp>",
  "created_by": "usr_admin",
  "updated_at": "<timestamp>"
}
```

#### 18.5.2 Rule Evaluation

Rules are evaluated in order until a match is found:

1. **Explicit deny** takes precedence over all allows
2. **Most specific path** wins (longer paths before shorter)
3. **First matching rule** determines capability
4. **No match** results in implicit deny

**Evaluation Algorithm:**

```rust
fn evaluate(principal: &Principal, path: &str, required: Capability) -> Decision {
    let policies = resolve_policies(principal);

    // Collect all matching rules across policies
    let mut matches: Vec<(&Rule, usize)> = vec![];
    for policy in &policies {
        for rule in &policy.rules {
            if rule.path.matches(path) {
                matches.push((rule, rule.path.specificity()));
            }
        }
    }

    // Sort by specificity (most specific first)
    matches.sort_by(|a, b| b.1.cmp(&a.1));

    // Evaluate in order
    for (rule, _) in matches {
        if rule.capabilities.contains(&Capability::Deny) {
            return Decision::Deny;
        }
        if rule.capabilities.contains(&required) {
            return Decision::Allow;
        }
        if rule.capabilities.iter().any(|c| c.implies(&required)) {
            return Decision::Allow;
        }
    }

    Decision::Deny  // Implicit deny
}
```

#### 18.5.3 Policy Examples

**Full Admin Policy:**

```json
{
  "name": "admin",
  "rules": [
    {
      "path": "+",
      "capabilities": ["admin"]
    }
  ]
}
```

**Production Data Scientist:**

```json
{
  "name": "data-scientist-prod",
  "rules": [
    {
      "path": "production/analytics/data/*",
      "capabilities": ["read", "write"]
    },
    {
      "path": "production/core/data/*",
      "capabilities": ["read"]
    },
    {
      "path": "production/*/edge/*",
      "capabilities": ["read"]
    },
    {
      "path": "production/*/_schema/*",
      "capabilities": ["read"]
    }
  ]
}
```

**ETL Service Policy:**

```json
{
  "name": "etl-service",
  "rules": [
    {
      "path": "*/*/data/*",
      "capabilities": ["read", "write"]
    },
    {
      "path": "*/*/edge/*",
      "capabilities": ["read", "write"]
    },
    {
      "path": "*/*/_schema/*",
      "capabilities": ["admin"]
    },
    {
      "path": "*/*/_cdc/*",
      "capabilities": ["read"]
    }
  ]
}
```

**Deny-Based Policy (Block PII):**

```json
{
  "name": "no-pii-access",
  "rules": [
    {
      "path": "*/*/data/PII_*",
      "capabilities": ["deny"]
    },
    {
      "path": "*/sensitive/+",
      "capabilities": ["deny"]
    }
  ]
}
```

### 18.6 Built-in Roles

PelagoDB provides built-in roles for common access patterns.

| Role | Policy | Description |
|------|--------|-------------|
| `pelago:admin` | Full admin | Complete access to all paths |
| `pelago:operator` | System operator | CDC, replication, job management |
| `pelago:reader` | Global read | Read-only access to all data |
| `pelago:schema-admin` | Schema management | Register/modify schemas, no data access |

**Built-in Role Policies:**

```json
// pelago:admin
{
  "name": "pelago:admin",
  "rules": [
    { "path": "+", "capabilities": ["admin"] }
  ]
}

// pelago:operator
{
  "name": "pelago:operator",
  "rules": [
    { "path": "*/*/_cdc/*", "capabilities": ["read"] },
    { "path": "*/*/_jobs/*", "capabilities": ["read", "write"] },
    { "path": "*/_db/*", "capabilities": ["read", "admin"] },
    { "path": "_sys/*", "capabilities": ["read", "admin"] }
  ]
}

// pelago:reader
{
  "name": "pelago:reader",
  "rules": [
    { "path": "*/*/data/*", "capabilities": ["read"] },
    { "path": "*/*/edge/*", "capabilities": ["read"] },
    { "path": "*/*/_schema/*", "capabilities": ["read"] }
  ]
}

// pelago:schema-admin
{
  "name": "pelago:schema-admin",
  "rules": [
    { "path": "*/*/_schema/*", "capabilities": ["admin"] },
    { "path": "*/*/_types/*", "capabilities": ["admin"] },
    { "path": "*/*/idx/*", "capabilities": ["admin"] }
  ]
}
```

### 18.7 Policy Storage in FDB

Security objects are stored in a dedicated system subspace.

#### 18.7.1 Security Keyspace Layout

```
/pelago/_sys/auth/
  /principals/
    (<principal_id>)              → Principal CBOR
  /credentials/
    /api_keys/
      (<key_id>)                  → ApiKey CBOR
      /by_hash/(<key_hash>)       → key_id (lookup)
    /certificates/
      (<fingerprint>)             → CertMapping CBOR
  /policies/
    (<policy_name>)               → Policy CBOR
    /versions/(<policy_name>, <version>) → Policy CBOR (history)
  /tokens/
    /active/(<jti>)               → TokenMetadata CBOR
    /revoked/(<jti>)              → revocation_time
  /audit/
    (<versionstamp>)              → AuditEntry CBOR
```

#### 18.7.2 Value Schemas

**Principal:**

```rust
struct Principal {
    principal_id: String,
    principal_type: PrincipalType,  // user, service, operator
    name: String,
    policies: Vec<String>,          // policy names
    inline_policy: Option<Policy>,
    metadata: HashMap<String, String>,
    created_at: i64,
    updated_at: i64,
    disabled: bool,
}
```

**ApiKey:**

```rust
struct ApiKey {
    key_id: String,
    key_hash: String,               // argon2id
    name: String,
    principal_id: String,
    policies: Vec<String>,          // additional policies
    created_at: i64,
    expires_at: Option<i64>,
    last_used_at: Option<i64>,
    metadata: HashMap<String, String>,
    revoked: bool,
}
```

**Policy:**

```rust
struct Policy {
    name: String,
    version: u32,
    description: Option<String>,
    rules: Vec<Rule>,
    created_at: i64,
    created_by: String,
    updated_at: i64,
}

struct Rule {
    path: String,
    capabilities: Vec<Capability>,
}

enum Capability {
    Read,
    Write,
    Admin,
    Deny,
}
```

### 18.8 RequestContext Integration

Authentication and authorization flow through the `RequestContext` message, extended with auth fields.

#### 18.8.1 Extended RequestContext

```protobuf
message RequestContext {
  // Existing fields
  string database = 1;
  string namespace = 2;
  string site_id = 3;
  string request_id = 4;

  // Auth fields (set by auth interceptor, not client)
  AuthContext auth = 10;
}

message AuthContext {
  // Principal information (populated after authentication)
  string principal_id = 1;
  PrincipalType principal_type = 2;
  repeated string effective_policies = 3;

  // Request metadata
  string credential_id = 4;        // key_id, cert fingerprint, or token jti
  CredentialType credential_type = 5;

  // For audit
  string client_ip = 6;
  string user_agent = 7;
}

enum PrincipalType {
  PRINCIPAL_TYPE_UNSPECIFIED = 0;
  PRINCIPAL_TYPE_USER = 1;
  PRINCIPAL_TYPE_SERVICE = 2;
  PRINCIPAL_TYPE_OPERATOR = 3;
  PRINCIPAL_TYPE_ANONYMOUS = 4;
}

enum CredentialType {
  CREDENTIAL_TYPE_UNSPECIFIED = 0;
  CREDENTIAL_TYPE_API_KEY = 1;
  CREDENTIAL_TYPE_MTLS = 2;
  CREDENTIAL_TYPE_SERVICE_TOKEN = 3;
}
```

#### 18.8.2 Auth Interceptor Flow

```rust
async fn auth_interceptor(
    request: Request<()>,
) -> Result<Request<()>, Status> {
    // 1. Extract credentials
    let credential = extract_credential(&request)?;

    // 2. Authenticate
    let principal = match credential {
        Credential::ApiKey(key) => authenticate_api_key(&key).await?,
        Credential::Certificate(cert) => authenticate_mtls(&cert).await?,
        Credential::Token(token) => authenticate_token(&token).await?,
        Credential::None => {
            if config.allow_anonymous {
                Principal::anonymous()
            } else {
                return Err(Status::unauthenticated("No credentials provided"));
            }
        }
    };

    // 3. Check if principal is disabled
    if principal.disabled {
        return Err(Status::permission_denied("Principal is disabled"));
    }

    // 4. Resolve effective policies
    let policies = resolve_policies(&principal).await?;

    // 5. Build AuthContext
    let auth_context = AuthContext {
        principal_id: principal.principal_id,
        principal_type: principal.principal_type,
        effective_policies: policies.iter().map(|p| p.name.clone()).collect(),
        credential_id: credential.id(),
        credential_type: credential.credential_type(),
        client_ip: extract_client_ip(&request),
        user_agent: extract_user_agent(&request),
    };

    // 6. Attach to request extensions
    request.extensions_mut().insert(auth_context);

    Ok(request)
}
```

#### 18.8.3 Authorization Check

```rust
async fn authorize(
    auth_context: &AuthContext,
    path: &str,
    required_capability: Capability,
) -> Result<(), Status> {
    // Load cached policies
    let policies = get_policies(&auth_context.effective_policies).await?;

    // Evaluate
    match evaluate(&policies, path, required_capability) {
        Decision::Allow => Ok(()),
        Decision::Deny => {
            // Audit the denial
            audit_log(AuditEvent::Denied {
                principal_id: &auth_context.principal_id,
                path,
                capability: required_capability,
            }).await;

            Err(Status::permission_denied(format!(
                "Permission denied: {} requires {:?} on {}",
                auth_context.principal_id,
                required_capability,
                path
            )))
        }
    }
}
```

### 18.9 Token and Credential Management

#### 18.9.1 API Key Lifecycle

| Operation | Endpoint | Permission Required |
|-----------|----------|---------------------|
| Create API key | `CreateApiKey` | `_sys/auth/credentials/*:admin` |
| List API keys | `ListApiKeys` | `_sys/auth/credentials/*:read` |
| Rotate API key | `RotateApiKey` | `_sys/auth/credentials/*:admin` |
| Revoke API key | `RevokeApiKey` | `_sys/auth/credentials/*:admin` |
| Get API key metadata | `GetApiKey` | `_sys/auth/credentials/*:read` |

**Key Rotation:**

1. Generate new key with same `key_id` suffix
2. New key becomes active immediately
3. Old key remains valid for grace period (default: 24 hours)
4. Old key hash moved to `revoked` subspace after grace period

#### 18.9.2 Service Token Lifecycle

| Operation | Description |
|-----------|-------------|
| Issue | Server generates JWT signed with private key |
| Validate | Verify signature, check expiry, check revocation list |
| Revoke | Add `jti` to revoked tokens subspace |
| Refresh | Issue new token with new `jti`, same claims |

**Token Revocation Check:**

```rust
async fn is_token_revoked(jti: &str) -> bool {
    let key = ("/pelago/_sys/auth/tokens/revoked", jti);
    db.get(&key).await.is_some()
}
```

#### 18.9.3 Certificate Management

Certificates are managed externally (PKI system). PelagoDB stores mappings:

| Operation | Description |
|-----------|-------------|
| Register | Map certificate fingerprint to principal |
| Unregister | Remove fingerprint mapping |
| List | List all registered certificates |

### 18.10 Audit Logging

All security-relevant operations are logged to a dedicated audit stream.

#### 18.10.1 Audit Events

| Event Type | Trigger |
|------------|---------|
| `auth.success` | Successful authentication |
| `auth.failure` | Failed authentication attempt |
| `authz.allowed` | Authorization check passed |
| `authz.denied` | Authorization check failed |
| `principal.created` | New principal created |
| `principal.updated` | Principal modified |
| `principal.disabled` | Principal disabled |
| `policy.created` | New policy created |
| `policy.updated` | Policy modified |
| `policy.deleted` | Policy deleted |
| `credential.created` | API key or cert registered |
| `credential.revoked` | Credential revoked |
| `token.issued` | Service token issued |
| `token.revoked` | Token revoked |

#### 18.10.2 Audit Entry Structure

```rust
struct AuditEntry {
    timestamp: i64,
    event_type: String,
    principal_id: Option<String>,
    credential_id: Option<String>,
    client_ip: String,
    user_agent: Option<String>,
    path: Option<String>,
    capability: Option<String>,
    outcome: AuditOutcome,      // success, failure, denied
    details: HashMap<String, String>,
    request_id: String,
    site_id: u8,
}
```

#### 18.10.3 Audit Storage

Audit entries use FDB versionstamps for ordering:

```
/pelago/_sys/auth/audit/(<versionstamp>) → AuditEntry CBOR
```

**Retention:** Configurable, default 90 days. Cleanup via background job.

**Export:** Audit entries can be streamed via CDC-like subscription for SIEM integration.

### 18.11 gRPC Auth Service

```protobuf
syntax = "proto3";
package pelago.auth.v1;

// ═══════════════════════════════════════════════════════════════════════════
// AUTH SERVICE
// ═══════════════════════════════════════════════════════════════════════════

service AuthService {
  // API Key management
  rpc CreateApiKey(CreateApiKeyRequest) returns (CreateApiKeyResponse);
  rpc GetApiKey(GetApiKeyRequest) returns (GetApiKeyResponse);
  rpc ListApiKeys(ListApiKeysRequest) returns (ListApiKeysResponse);
  rpc RotateApiKey(RotateApiKeyRequest) returns (RotateApiKeyResponse);
  rpc RevokeApiKey(RevokeApiKeyRequest) returns (RevokeApiKeyResponse);

  // Principal management
  rpc CreatePrincipal(CreatePrincipalRequest) returns (CreatePrincipalResponse);
  rpc GetPrincipal(GetPrincipalRequest) returns (GetPrincipalResponse);
  rpc UpdatePrincipal(UpdatePrincipalRequest) returns (UpdatePrincipalResponse);
  rpc DisablePrincipal(DisablePrincipalRequest) returns (DisablePrincipalResponse);
  rpc ListPrincipals(ListPrincipalsRequest) returns (ListPrincipalsResponse);

  // Policy management
  rpc CreatePolicy(CreatePolicyRequest) returns (CreatePolicyResponse);
  rpc GetPolicy(GetPolicyRequest) returns (GetPolicyResponse);
  rpc UpdatePolicy(UpdatePolicyRequest) returns (UpdatePolicyResponse);
  rpc DeletePolicy(DeletePolicyRequest) returns (DeletePolicyResponse);
  rpc ListPolicies(ListPoliciesRequest) returns (ListPoliciesResponse);

  // Certificate mapping
  rpc RegisterCertificate(RegisterCertificateRequest) returns (RegisterCertificateResponse);
  rpc UnregisterCertificate(UnregisterCertificateRequest) returns (UnregisterCertificateResponse);
  rpc ListCertificates(ListCertificatesRequest) returns (ListCertificatesResponse);

  // Token operations
  rpc IssueServiceToken(IssueServiceTokenRequest) returns (IssueServiceTokenResponse);
  rpc RevokeToken(RevokeTokenRequest) returns (RevokeTokenResponse);

  // Introspection
  rpc WhoAmI(WhoAmIRequest) returns (WhoAmIResponse);
  rpc CheckPermission(CheckPermissionRequest) returns (CheckPermissionResponse);
}

// ═══════════════════════════════════════════════════════════════════════════
// API KEY MESSAGES
// ═══════════════════════════════════════════════════════════════════════════

message CreateApiKeyRequest {
  string name = 1;
  string principal_id = 2;
  repeated string policies = 3;
  int64 expires_at = 4;           // Unix timestamp, 0 = never
  map<string, string> metadata = 5;
}

message CreateApiKeyResponse {
  string key_id = 1;
  string api_key = 2;             // Full key (only returned on create)
  int64 created_at = 3;
  int64 expires_at = 4;
}

message GetApiKeyRequest {
  string key_id = 1;
}

message GetApiKeyResponse {
  ApiKeyInfo key = 1;
}

message ApiKeyInfo {
  string key_id = 1;
  string name = 2;
  string principal_id = 3;
  repeated string policies = 4;
  int64 created_at = 5;
  int64 expires_at = 6;
  int64 last_used_at = 7;
  map<string, string> metadata = 8;
  bool revoked = 9;
}

message ListApiKeysRequest {
  string principal_id = 1;        // Filter by principal (optional)
  bool include_revoked = 2;
}

message ListApiKeysResponse {
  repeated ApiKeyInfo keys = 1;
}

message RotateApiKeyRequest {
  string key_id = 1;
  int64 grace_period_seconds = 2; // How long old key remains valid
}

message RotateApiKeyResponse {
  string new_api_key = 1;
  int64 old_key_expires_at = 2;
}

message RevokeApiKeyRequest {
  string key_id = 1;
}

message RevokeApiKeyResponse {
  bool revoked = 1;
}

// ═══════════════════════════════════════════════════════════════════════════
// PRINCIPAL MESSAGES
// ═══════════════════════════════════════════════════════════════════════════

message CreatePrincipalRequest {
  PrincipalType type = 1;
  string name = 2;
  repeated string policies = 3;
  Policy inline_policy = 4;
  map<string, string> metadata = 5;
}

message CreatePrincipalResponse {
  string principal_id = 1;
  int64 created_at = 2;
}

message GetPrincipalRequest {
  string principal_id = 1;
}

message GetPrincipalResponse {
  PrincipalInfo principal = 1;
}

message PrincipalInfo {
  string principal_id = 1;
  PrincipalType type = 2;
  string name = 3;
  repeated string policies = 4;
  Policy inline_policy = 5;
  map<string, string> metadata = 6;
  int64 created_at = 7;
  int64 updated_at = 8;
  bool disabled = 9;
}

message UpdatePrincipalRequest {
  string principal_id = 1;
  repeated string policies = 2;     // Replace policies
  Policy inline_policy = 3;         // Replace inline policy
  map<string, string> metadata = 4; // Merge with existing
}

message UpdatePrincipalResponse {
  PrincipalInfo principal = 1;
}

message DisablePrincipalRequest {
  string principal_id = 1;
  bool disabled = 2;
}

message DisablePrincipalResponse {
  bool disabled = 1;
}

message ListPrincipalsRequest {
  PrincipalType type = 1;           // Filter by type (optional)
  bool include_disabled = 2;
}

message ListPrincipalsResponse {
  repeated PrincipalInfo principals = 1;
}

// ═══════════════════════════════════════════════════════════════════════════
// POLICY MESSAGES
// ═══════════════════════════════════════════════════════════════════════════

message CreatePolicyRequest {
  string name = 1;
  string description = 2;
  repeated PolicyRule rules = 3;
}

message CreatePolicyResponse {
  string name = 1;
  uint32 version = 2;
  int64 created_at = 3;
}

message GetPolicyRequest {
  string name = 1;
  uint32 version = 2;               // 0 = latest
}

message GetPolicyResponse {
  Policy policy = 1;
}

message Policy {
  string name = 1;
  uint32 version = 2;
  string description = 3;
  repeated PolicyRule rules = 4;
  int64 created_at = 5;
  string created_by = 6;
  int64 updated_at = 7;
}

message PolicyRule {
  string path = 1;
  repeated Capability capabilities = 2;
}

enum Capability {
  CAPABILITY_UNSPECIFIED = 0;
  CAPABILITY_READ = 1;
  CAPABILITY_WRITE = 2;
  CAPABILITY_ADMIN = 3;
  CAPABILITY_DENY = 4;
}

message UpdatePolicyRequest {
  string name = 1;
  string description = 2;
  repeated PolicyRule rules = 3;
}

message UpdatePolicyResponse {
  Policy policy = 1;
}

message DeletePolicyRequest {
  string name = 1;
}

message DeletePolicyResponse {
  bool deleted = 1;
}

message ListPoliciesRequest {
  bool include_builtin = 1;
}

message ListPoliciesResponse {
  repeated Policy policies = 1;
}

// ═══════════════════════════════════════════════════════════════════════════
// CERTIFICATE MESSAGES
// ═══════════════════════════════════════════════════════════════════════════

message RegisterCertificateRequest {
  string certificate_pem = 1;       // PEM-encoded certificate
  string principal_id = 2;
  repeated string policies = 3;     // Additional policies
}

message RegisterCertificateResponse {
  string fingerprint = 1;
  string subject_cn = 2;
  int64 valid_until = 3;
}

message UnregisterCertificateRequest {
  string fingerprint = 1;
}

message UnregisterCertificateResponse {
  bool unregistered = 1;
}

message ListCertificatesRequest {
  string principal_id = 1;          // Filter by principal (optional)
}

message ListCertificatesResponse {
  repeated CertificateInfo certificates = 1;
}

message CertificateInfo {
  string fingerprint = 1;
  string subject_cn = 2;
  string principal_id = 3;
  repeated string policies = 4;
  int64 valid_until = 5;
  int64 registered_at = 6;
}

// ═══════════════════════════════════════════════════════════════════════════
// TOKEN MESSAGES
// ═══════════════════════════════════════════════════════════════════════════

message IssueServiceTokenRequest {
  string principal_id = 1;
  int64 ttl_seconds = 2;            // Max 86400 (24 hours)
  repeated string capabilities = 3; // Additional capabilities claim
}

message IssueServiceTokenResponse {
  string token = 1;                 // JWT
  int64 expires_at = 2;
  string jti = 3;
}

message RevokeTokenRequest {
  string jti = 1;
}

message RevokeTokenResponse {
  bool revoked = 1;
}

// ═══════════════════════════════════════════════════════════════════════════
// INTROSPECTION MESSAGES
// ═══════════════════════════════════════════════════════════════════════════

message WhoAmIRequest {}

message WhoAmIResponse {
  string principal_id = 1;
  PrincipalType principal_type = 2;
  string credential_id = 3;
  CredentialType credential_type = 4;
  repeated string effective_policies = 5;
}

message CheckPermissionRequest {
  string path = 1;
  Capability capability = 2;
}

message CheckPermissionResponse {
  bool allowed = 1;
  string matched_policy = 2;        // Policy that granted/denied
  string matched_rule_path = 3;     // Specific rule path
}
```

### 18.12 Security Configuration

| Environment Variable | CLI Flag | Default | Description |
|---------------------|----------|---------|-------------|
| `PELAGO_AUTH_ENABLED` | `--auth-enabled` | `true` | Enable authentication |
| `PELAGO_AUTH_ALLOW_ANONYMOUS` | `--allow-anonymous` | `false` | Allow unauthenticated requests |
| `PELAGO_AUTH_TOKEN_SIGNING_KEY` | `--token-key` | (required) | Ed25519 private key for JWT signing |
| `PELAGO_AUTH_ARGON2_TIME` | - | `3` | Argon2id time cost |
| `PELAGO_AUTH_ARGON2_MEMORY` | - | `65536` | Argon2id memory cost (KB) |
| `PELAGO_AUTH_ARGON2_PARALLELISM` | - | `4` | Argon2id parallelism |
| `PELAGO_AUDIT_ENABLED` | `--audit-enabled` | `true` | Enable audit logging |
| `PELAGO_AUDIT_RETENTION_DAYS` | `--audit-retention` | `90` | Audit log retention |

### 18.13 Error Codes

| Error Code | gRPC Status | Description |
|------------|-------------|-------------|
| `ERR_UNAUTHENTICATED` | `UNAUTHENTICATED` | No valid credentials provided |
| `ERR_INVALID_API_KEY` | `UNAUTHENTICATED` | API key not found or invalid |
| `ERR_EXPIRED_API_KEY` | `UNAUTHENTICATED` | API key has expired |
| `ERR_REVOKED_API_KEY` | `UNAUTHENTICATED` | API key has been revoked |
| `ERR_INVALID_TOKEN` | `UNAUTHENTICATED` | JWT signature invalid |
| `ERR_EXPIRED_TOKEN` | `UNAUTHENTICATED` | JWT has expired |
| `ERR_REVOKED_TOKEN` | `UNAUTHENTICATED` | Token has been revoked |
| `ERR_INVALID_CERTIFICATE` | `UNAUTHENTICATED` | mTLS certificate not registered |
| `ERR_PRINCIPAL_DISABLED` | `PERMISSION_DENIED` | Principal account is disabled |
| `ERR_PERMISSION_DENIED` | `PERMISSION_DENIED` | Insufficient permissions for path |
| `ERR_POLICY_NOT_FOUND` | `NOT_FOUND` | Referenced policy does not exist |
| `ERR_PRINCIPAL_NOT_FOUND` | `NOT_FOUND` | Principal does not exist |
| `ERR_INVALID_PATH` | `INVALID_ARGUMENT` | Policy path syntax error |
| `ERR_BUILTIN_POLICY` | `FAILED_PRECONDITION` | Cannot modify built-in policy |

### 18.14 Security Considerations

#### 18.14.1 Credential Storage

- API key secrets are **never stored in plaintext**; only Argon2id hashes
- JWT signing keys should be stored in secure key management (HSM, Vault)
- Certificate private keys never touch PelagoDB servers

#### 18.14.2 Transport Security

- All client connections should use TLS 1.3
- mTLS required for service-to-service in production
- Internal replication uses separate mTLS certificates

#### 18.14.3 Rate Limiting

Auth operations should be rate-limited to prevent brute force:

| Operation | Default Limit |
|-----------|---------------|
| Failed auth attempts | 10/minute per IP |
| Token issuance | 100/minute per principal |
| Policy updates | 10/minute per principal |

#### 18.14.4 Key Rotation

- API keys: Rotate every 90 days recommended
- JWT signing keys: Rotate every 30 days with JWKS support
- mTLS certificates: Follow organizational PKI policy

---
