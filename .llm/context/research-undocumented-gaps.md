# Graph Database Engine: Undocumented Gaps Research

**Status:** Research Document
**Author:** Andrew + Claude
**Date:** 2026-02-10
**Scope:** Analysis of underspecified (I4) and missing gaps in v0.3 spec

---

## Overview

This document investigates critical gaps in the v0.3 architecture spec that are either marked as important (I4+) or entirely missing from the gap analysis. Each gap includes technical analysis, design options, recommendations, and impact on implementation roadmap.

The four primary gaps are:
1. **Gap I4: Query transactionality and snapshot reads** — multi-step queries lack consistency guarantees
2. **Gap: Authentication and authorization model** — "Auth" box in diagram with zero specification
3. **Gap: Error model and status codes** — many error conditions defined but no taxonomy
4. **Gap: Schema registry consistency across processes** — multiple processes with stale schema caches

Two additional gaps are also analyzed:
5. **Gap: CDC consumer backpressure** — no flow control when consumers fall behind
6. **Gap: CDC retention and compaction** — 7-day retention mentioned, compaction strategy missing

---

## Gap I4: Query Transactionality and Snapshot Reads

### Problem Statement

A FindNodes query that uses index scan → data fetch is not atomic:

```
Time T0:  Index scan for "age >= 30" → returns [p_1, p_2, p_3]
Time T1:  Mutation: delete p_2 (now in no index)
Time T2:  Data fetch for p_2 → key not found, error or omitted
```

The application sees p_1, p_3 (but not p_2, which was in the index scan result set at T0). This is inconsistent: the query does not represent any single committed state.

More critically for **traversal**:

```
Time T0:  Traverse: find Person → WORKS_AT → Company
          Hop 1: find all Persons (5000 results)
Time T1:  Mutations: modify schema, delete person p_99
Time T2:  Hop 2: starting to resolve WORKS_AT edges for Persons
          p_99's WORKS_AT edges still exist but p_99 is deleted
          → dangling references, or version mismatch issues
```

For multi-hop traversals that take seconds (returning 5000 results, each hop fetching edges), the lack of a consistent read snapshot is problematic.

### FDB Snapshot Reads: How They Work

FDB's transaction model:

```rust
let db = fdb::open()?;
let tx = db.create_transaction()?;

// Option A: Full transaction (default, latest committed)
let v1 = tx.get(&key1).await?;

// Option B: Snapshot read (at explicit version)
let read_version = tx.get_read_version().await?;  // Latest committed
let snapshot = db.create_transaction()?;
snapshot.set_read_version(read_version);  // Pin to version
let v2 = snapshot.get(&key2).await?;      // Both reads at same version
```

Key properties:
- **get_read_version()**: Returns the latest committed version. Cheap — single round-trip.
- **set_read_version(v)**: Subsequent reads in that transaction use version `v`.
- **Consistency guarantee**: All reads in the transaction see the same committed state at version `v`.
- **5-second transaction window**: Snapshot reads are only valid for 5 seconds after commit. Longer transactions are rejected with `TXN_TOO_OLD`.

### Current Spec Behavior

From Section 3.6:
```
"Check RocksDB for cached result... cache HWM >= required freshness → return cached"
```

From Section 9:
```
Strong: direct FDB read (latest committed)
Session: RocksDB cache verified against FDB read version
Eventual: RocksDB cache only
```

**Gap**: The spec defines consistency levels but never specifies HOW queries acquire a read version or whether multi-step reads (index scan + data fetch) use the same version.

### Design Options

#### Option A: Snapshot Reads (Pinned Read Version)

Every read query acquires an FDB read version at the start, then uses snapshot reads for all subsequent FDB operations.

```rust
// Pseudocode
async fn find_nodes(req: FindNodesRequest) -> Result<Vec<Node>> {
    // Acquire snapshot read version
    let read_version = db.get_read_version().await?;

    // All FDB reads use this version
    let mut snapshot_tx = db.create_transaction()?;
    snapshot_tx.set_read_version(read_version);

    // Index scan at read_version
    let index_results = snapshot_tx.get_range(&index_key_range).await?;

    // Data fetch at same read_version
    for id in index_results {
        let node = snapshot_tx.get(&node_key(id)).await?;
        results.push(node);
    }

    Ok(results)
}
```

**Consistency guarantee**: The entire query sees a single, consistent snapshot. No anomalies from concurrent mutations.

**Performance implications**:
- `get_read_version()` is a single network round-trip (cheap, ~1ms).
- Snapshot reads are the same performance as normal reads (no overhead).
- Caveat: **5-second transaction window**. Long-running reads/traversals that exceed 5 seconds fail with `TXN_TOO_OLD`.

**Trade-off**: Strong consistency during query execution, but long operations must be paginated.

#### Option B: No Snapshot — Eventually Consistent

Leave behavior as-is: no snapshot version pinning. Queries see whatever is current at the moment each read is issued.

**Pros**: No extra machinery, no 5-second window hassles.

**Cons**: Inconsistency anomalies in results. Worse for traversal (intermediate nodes deleted/modified).

This is likely **not acceptable** for a graph database where users expect consistent traversal.

#### Option C: Snapshot with Cursor Checkpointing (Hybrid)

Acquire a snapshot for the first page of results. If traversal/pagination exceeds 5 seconds:
- Checkpoint the cursor at current depth/position
- Acquire a NEW snapshot version
- Resume from checkpoint with new version

Results may show inconsistency **across page boundaries** but each page is consistent.

```rust
async fn traverse(req: TraverseRequest) -> Result<TraverseStream> {
    let mut read_version = db.get_read_version().await?;
    let mut depth = 0;
    let mut current_results = vec![];

    loop {
        let mut snapshot_tx = db.create_transaction()?;
        snapshot_tx.set_read_version(read_version);

        // Execute one hop, accumulate results
        let hop_results = execute_hop(&snapshot_tx, &current_results, depth).await?;
        current_results = hop_results;
        depth += 1;

        // Check if we're approaching 5-second limit
        if elapsed_time > 4500ms && depth < max_depth {
            // Checkpoint: emit results so far
            stream.send(current_results.clone()).await?;

            // Refresh snapshot for next hop
            read_version = db.get_read_version().await?;
            current_results.clear();
        }

        if depth >= max_depth { break; }
    }

    stream.send(current_results).await?;
}
```

**Trade-off**: Consistent within each checkpoint page, but across pages may see schema changes or deleted nodes.

### Interaction with Consistency Levels

Consistency level affects **which data source** is read, not whether reads are snapshotted:

```rust
async fn get_node(id: NodeId, consistency: ReadConsistency) -> Result<Node> {
    match consistency {
        Strong => {
            // FDB snapshot read at current version
            let read_version = db.get_read_version().await?;
            let tx = db.create_transaction()?;
            tx.set_read_version(read_version);
            tx.get(&node_key(id)).await
        },
        Session => {
            // Try RocksDB first; verify freshness against FDB read version
            if let Some(cached) = rocksdb.get(&node_key(id)) {
                let read_version = db.get_read_version().await?;
                let cache_hwm = fetch_cache_hwm_from_fdb().await?;
                if cache_hwm >= read_version {
                    return Ok(cached);  // Cache is fresh
                }
            }
            // Fall back to FDB snapshot read
            let read_version = db.get_read_version().await?;
            let tx = db.create_transaction()?;
            tx.set_read_version(read_version);
            tx.get(&node_key(id)).await
        },
        Eventual => {
            // RocksDB only, no freshness check
            rocksdb.get(&node_key(id))
                .or_else(|| fdb_get(&node_key(id)).await)
        }
    }
}
```

The **snapshot version pinning** is orthogonal to consistency level. Both Strong and Session use snapshots; the difference is whether the source is FDB (strong) or RocksDB verified by FDB (session).

### Recommendation

**Adopt Option A: Snapshot Reads with Mandatory Pagination**

1. **API change**: All queries must include `limit` and pagination via cursor.
2. **Acquire snapshot at query start**: `read_version = db.get_read_version().await`.
3. **Pin all FDB reads**: Every read in a query uses the same `read_version`.
4. **Enforce timeout**: Queries that exceed 5 seconds are rejected or paginated.

For **long traversals** (Option C hybrid):
- Implement cursor checkpointing in the traversal engine.
- Each checkpoint acquires a new snapshot.
- Document the cross-checkpoint inconsistency (node may be deleted, schema may change).

**Implementation plan**:
- Add snapshot version pinning to query executor. All queries default to paginated (limit enforced).
- Implement cursor checkpointing for traversal resumption.
- Document consistency guarantees and edge cases.

**Spec updates**:
- Section 5 (CEL to Query Plan): Add "Snapshot read version acquired at query start."
- Section 3.4 (US-9, US-10): Add pagination examples using cursors.
- Section 9: Expand "Consistency Levels" to explain snapshot versioning.

---

## Gap: Authentication and Authorization Model

### Problem Statement

The v0.3 spec shows an "Auth" box in the API Layer diagram (Section 2, line 54) and never mentions it again. Multi-tenant, multi-namespace, multi-site scenarios have no definition of:
- Who can access which namespaces?
- Who can read vs. write vs. modify schema?
- How do sites authenticate each other?
- Is there a per-API-server identity, or per-client identity?

### Requirements Implicit in the Architecture

From the spec:
- **Multi-tenant**: Optional tenant prefix (Section 11). Implies isolation — Tenant A should not see Tenant B's data.
- **Namespace isolation**: Each namespace is "fully isolated in storage" (Section 3.5, US-13). Implies namespaces are access boundaries.
- **Multi-site replication**: Sites replicate CDC entries to remote sites (Section 10). Implies inter-site authentication.

### Design Options

#### Option A: API Keys per Namespace (Minimal for v1)

Each client gets an API key bound to a namespace.

```protobuf
message AuthContext {
    string api_key = 1;
    // Server resolves api_key → (tenant, namespace, permissions)
}
```

**Token format**: `pk_<namespace>_<random_base64>` (namespaced keys)

**Permissions**: Single role per key:
- `read_only`: ListNodes, FindNodes, Traverse, WatchNode (read ops only)
- `read_write`: + CreateNode, CreateEdge, UpdateNode, DeleteNode, DeleteEdge
- `admin`: + RegisterSchema, DropNamespace, DropEntity

**Advantages**:
- Dead simple to implement (lookup table: key → (tenant, namespace, role))
- Sufficient for single-tenant per-API-key model
- No shared secret complexity

**Disadvantages**:
- No fine-grained permission (can't restrict writes to specific entity types)
- No user identity (can't audit who created a node)
- Not suitable for multi-user SaaS

#### Option B: RBAC with Roles + User Identity

Each request includes a bearer token (JWT or opaque session token) that identifies a user. Users are bound to roles at namespace level.

```protobuf
message AuthContext {
    string bearer_token = 1;  // JWT or session ID
}

// Server decodes JWT:
// {
//   "user_id": "u_123",
//   "tenant": "acme",
//   "permissions": [
//     { "namespace": "myapp", "role": "read_write" },
//     { "namespace": "analytics", "role": "read_only" }
//   ]
// }
```

**Roles**:
- `read_only`: ListNodes, FindNodes, Traverse, Watch
- `write`: + CreateNode, CreateEdge (existing entity types)
- `schema_admin`: + RegisterSchema, DropEntity (schema mutations)
- `admin`: DropNamespace, ManageUsers, etc.

**Advantages**:
- Multi-user SaaS model (can audit by user)
- Flexible permission assignment
- Standard JWT/session patterns

**Disadvantages**:
- Token issuance + refresh complexity (separate auth service?)
- JWT validation overhead on every request
- More state to manage

#### Option C: gRPC Interceptor with TLS Client Certificates + API Keys

Use gRPC interceptor for auth middleware. Accept either:
- **mTLS**: Client cert identifies API server/site, verified by CA
- **API Key**: For client SDKs

```rust
// In gRPC interceptor
match extract_auth(&metadata) {
    Auth::Mtls(peer_cert) => {
        // Verify certificate chain
        // Extract CN as site_id or service_id
        context.authenticated_as = peer_cert.subject.cn;
    },
    Auth::ApiKey(key) => {
        // Look up key in registry
        context.authenticated_as = resolve_api_key(key)?;
    },
    _ => Err("missing auth")
}
```

**Advantages**:
- mTLS for inter-site (site-to-site CDC replication)
- API keys for client SDKs
- Single auth interceptor handles both

**Disadvantages**:
- Two different auth paths complicates logic
- mTLS cert management overhead

### Multi-Site Authentication

How do sites authenticate each other when replicating CDC?

**Option 1: Mutual TLS (mTLS)**
- Each site has a certificate signed by shared CA
- When Site A connects to Site B to stream CDC, they verify each other's certs
- Simple, standard, no shared secrets

**Option 2: Shared Auth Service**
- Single auth service (e.g., Keystone, Okta) issues tokens to sites
- Site A gets a token, uses it to authenticate to Site B
- Scalable but adds operational complexity

**Option 3: Pre-shared Keys per Site Pair**
- Site A and Site B agree on a shared key during setup
- Each request is signed with the key (HMAC)
- Simple but key rotation is painful

### Namespace-Level vs. Entity-Type-Level Permissions

Should permissions be:
- **Namespace-scoped**: "read-write in namespace 'myapp'" (current design)
- **Entity-scoped**: "read-write Person nodes, read-only Company nodes in 'myapp'"
- **Both**: Layered (namespace role, then entity-type overrides)

**Recommendation**: Start with namespace-scoped. Entity-type-level can be Phase 2.

### Recommendation

**Adopt Option A + gRPC interceptor:**

1. **API Key model**: Namespaced keys (`pk_<namespace>_<random>`).
2. **Role**: Single role per key (read_only, read_write, admin).
3. **gRPC Interceptor**: Extract API key from metadata, resolve to (namespace, role), attach to context.
4. **Multi-site auth**: mTLS for inter-site replication. Each site has a self-signed cert or CA-signed cert. On connect, verify cert CN matches expected peer site ID.

**API changes**:
```protobuf
// Per-request auth
metadata {
    "authorization": "Bearer pk_myapp_abc123def456"
}

// Server validates in interceptor:
// 1. Extract key from header
// 2. Look up in registry: key → (namespace, role)
// 3. Check namespace in request matches
// 4. Check role has permission for operation
// 5. Attach auth context to gRPC context
```

**Spec updates**:
- Add new Section 14: "Security and Authorization"
- Document API key format and issuance
- Document gRPC interceptor hook point
- Document mTLS setup for inter-site auth
- Define per-role allowed operations

**Future enhancements**:
- User identity (JWT bearer tokens)
- Entity-type-level permissions
- Central auth service (Oauth/OIDC)

---

## Gap: Error Model and Status Codes

### Problem Statement

The spec defines error conditions in Section 4:
- Node creation without registered type → **REJECT**
- Edge creation to unregistered target type → **REJECT**
- Unique constraint violation → **OBSOLETE marking** (per Gap C7)
- Schema validation failure → implied rejection
- CEL compilation error → implied rejection

But there is **no error taxonomy**, no gRPC error codes, and no structured error response format.

Clients have no way to distinguish:
- Schema validation error (missing property) vs. type error (property is wrong type)
- Transient error (network) vs. permanent error (data invalid)
- Recoverable error (retry) vs. fatal error (client bug)

### Error Categories in the Spec

Enumerated from all sections:

**Validation Errors** (client's input is wrong):
- Unregistered entity type in CreateNode
- Property missing (required field)
- Property wrong type (age: "thirty" when schema expects int)
- Extra properties in "reject" mode
- Unregistered edge target in CreateEdge
- CEL expression syntax error
- CEL type error ("age > 'string'")

**Referential Integrity Errors**:
- Target node doesn't exist in CreateEdge
- Target type not registered in CreateEdge (even if forward declaration allowed in schema)

**Ownership Errors** (multi-site):
- Mutation to node from non-home site
- Mutation to edge from non-owning site (if ownership: "independent")

**Consistency Errors**:
- Unique constraint violation (conflict with existing node) → OBSOLETE in multi-site
- Schema mismatch during evolution (property type changed)

**Concurrency/Timeout Errors**:
- Transaction too old (snapshot read > 5 seconds)
- Query timeout exceeded
- Traversal depth limit exceeded
- Traversal result limit exceeded

**Resource Errors**:
- FDB storage quota exceeded (namespace or global)
- CDC retention expired (trying to access old CDC entry)

**Internal/System Errors**:
- FDB cluster unavailable
- gRPC codec failure
- Background job failed (retry later)

### gRPC Status Code Mapping

gRPC defines standard status codes:
- `OK (0)`: Success
- `CANCELLED (1)`: Cancelled by client
- `UNKNOWN (2)`: Unknown error
- `INVALID_ARGUMENT (3)`: Invalid argument (input validation)
- `DEADLINE_EXCEEDED (4)`: Timeout
- `NOT_FOUND (5)`: Resource not found
- `ALREADY_EXISTS (6)`: Resource already exists
- `PERMISSION_DENIED (7)`: Permission denied
- `RESOURCE_EXHAUSTED (8)`: Resource limit exceeded
- `FAILED_PRECONDITION (9)`: Operation precondition failed
- `ABORTED (10)`: Aborted (transaction conflict)
- `OUT_OF_RANGE (11)`: Out of range
- `UNIMPLEMENTED (12)`: Not implemented
- `INTERNAL (13)`: Internal server error
- `UNAVAILABLE (14)`: Service temporarily unavailable
- `DATA_LOSS (15)`: Unrecoverable data loss
- `UNAUTHENTICATED (16)`: Unauthenticated

**Mapping**:

| Spec Error | gRPC Code | Reason |
|---|---|---|
| Unregistered entity type | `INVALID_ARGUMENT` | Input references non-existent schema |
| Property type mismatch | `INVALID_ARGUMENT` | Input data doesn't match schema |
| Required property missing | `INVALID_ARGUMENT` | Input incomplete |
| CEL syntax error | `INVALID_ARGUMENT` | Expression unparseable |
| CEL type error | `INVALID_ARGUMENT` | Expression doesn't type-check |
| Target node not found | `NOT_FOUND` | Referential integrity violation |
| Target type not registered | `NOT_FOUND` | Edge target type doesn't exist |
| Unique constraint conflict | `ALREADY_EXISTS` (multi-site) or `FAILED_PRECONDITION` (single-site) | Duplicate value detected |
| Mutation to non-home node (multi-site) | `PERMISSION_DENIED` | Only home site can modify |
| Mutation to non-owned edge | `PERMISSION_DENIED` | Only owning site can modify |
| Query timeout | `DEADLINE_EXCEEDED` | Took longer than max_depth / timeout_ms |
| Traversal result limit exceeded | `RESOURCE_EXHAUSTED` | Partial result returned; use cursor |
| FDB storage quota exceeded | `RESOURCE_EXHAUSTED` | Can't write more data |
| FDB cluster unavailable | `UNAVAILABLE` | Transient, can retry |
| CDC retention expired | `OUT_OF_RANGE` | Requested version too old |
| Transaction too old | `ABORTED` | Snapshot read window exceeded; retry |

### Structured Error Response

Instead of error message strings, return structured error details:

```protobuf
message ErrorDetail {
    string error_code = 1;              // e.g., "ERR_UNREGISTERED_TYPE"
    string message = 2;                 // Human-readable (e.g., "Entity type 'Person' not registered")
    string category = 3;                // e.g., "VALIDATION", "REFERENTIAL", "OWNERSHIP"
    map<string, string> metadata = 4;   // Context: {"entity_type": "Person", "namespace": "myapp"}
    repeated FieldError field_errors = 5; // For multi-field errors
}

message FieldError {
    string field_name = 1;
    string error_code = 2;              // e.g., "TYPE_MISMATCH"
    string message = 3;                 // e.g., "Expected int, got string"
}

// In gRPC response:
message CreateNodeResponse {
    oneof result {
        Node node = 1;
        ErrorDetail error = 2;
    }
}
```

### Client SDK Error Mapping (Rust Example)

```rust
pub enum GraphError {
    Validation(ValidationError),
    Referential(ReferentialError),
    Ownership(OwnershipError),
    Consistency(ConsistencyError),
    Timeout(TimeoutError),
    ResourceExhausted(String),
    Internal(String),
}

pub struct ValidationError {
    pub entity_type: Option<String>,
    pub field_errors: Vec<FieldError>,
}

pub struct FieldError {
    pub field_name: String,
    pub error_code: String,
    pub message: String,
}

impl From<tonic::Status> for GraphError {
    fn from(status: tonic::Status) -> Self {
        let code = status.code();
        match code {
            Code::InvalidArgument => {
                // Parse error details from status metadata
                GraphError::Validation(...)
            },
            Code::NotFound => GraphError::Referential(...),
            Code::PermissionDenied => GraphError::Ownership(...),
            Code::DeadlineExceeded => GraphError::Timeout(...),
            Code::ResourceExhausted => GraphError::ResourceExhausted(status.message().to_string()),
            _ => GraphError::Internal(status.message().to_string()),
        }
    }
}

// Usage:
match client.create_node(req).await {
    Ok(node) => { /* success */ },
    Err(GraphError::Validation(ve)) => {
        eprintln!("Validation error in {}: {:?}", ve.entity_type.unwrap_or_default(), ve.field_errors);
    },
    Err(GraphError::Ownership(oe)) => {
        eprintln!("Ownership error: can't mutate from non-home site");
    },
    Err(e) => eprintln!("Other error: {:?}", e),
}
```

### Recommendation

**Adopt structured error model with gRPC codes + ErrorDetail:**

1. **Enumerate all error codes**: Create a registry of domain-specific codes (e.g., `ERR_UNREGISTERED_TYPE`, `ERR_PROPERTY_TYPE_MISMATCH`, etc.).
2. **Map to gRPC status codes**: Per table above.
3. **Return ErrorDetail in response**: Include category, metadata, field-level details.
4. **SDK support**: Clients map gRPC status + ErrorDetail to typed exceptions (per language).

**Spec updates**:
- Add new Section 14: "Error Model"
- Enumerate all error conditions with codes
- Document gRPC status code mapping
- Provide example protobuf definitions
- Document SDK error handling patterns

**Implementation**:
- Define all error codes and categories
- Map to gRPC codes
- Implement ErrorDetail in all responses

**Future enhancements**:
- Retry logic for transient errors (UNAVAILABLE, ABORTED)
- Exponential backoff in SDK
- Structured logging with error categories for observability

---

## Gap: Schema Registry Consistency Across Processes

### Problem Statement

The API Layer caches compiled CEL environments and schema definitions in memory (line 53: "CEL compilation").

When multiple API server processes are running:
```
Server Process 1                    Server Process 2
┌──────────────────┐              ┌──────────────────┐
│ CEL env cache:   │              │ CEL env cache:   │
│ Person v1        │              │ Person v1        │
│ (compiled)       │              │ (compiled)       │
└──────────────────┘              └──────────────────┘
           │                               │
           └───────────┬───────────────────┘
                       │
                    FDB: Person v1
```

An operator registers **Person v2** (add a field `phone: string`):

```
FDB writes Person v2 to schema registry
    │
    ▼
Server 1: Stale. Still has v1 CEL in memory. Compiles against v1 schema.
Server 2: Still has v1 CEL in memory. Compiles against v1 schema.
Server 3: (new process starts) Loads Person v2 from FDB. Has v2 CEL.
```

**Issues**:
- Old servers reject `phone` field as "extra property" (extras_policy: "reject").
- New servers accept `phone` field.
- Clients are routed by load balancer, sometimes hitting old servers (rejection), sometimes new servers (acceptance).
- Temporal anomaly: same request rejected by one server, accepted by another.

Even worse: CEL expressions compiled against v1 don't have `phone` in the environment. A new node with `phone` is created on Server 3, then Server 1 loads it and tries to evaluate an old CEL expression that doesn't reference `phone` — it's okay, but if the query filter is `phone > '555'` compiled on v1, the CEL environment doesn't have `phone` and the expression is invalid.

### Current Spec

Section 3.3 (US-3):
```
Adding a new indexed property: triggers async background index backfill (via background job)
Schema version is tracked; CEL cache invalidates on version change
```

But **where is the version tracked?** In FDB? In memory? Does every request check the version?

Section 5:
```
A CEL expression enters as a string and exits as a set of FDB operations
```

Doesn't specify when/how the CEL environment is loaded or invalidated.

### Design Options

#### Option A: CDC-Driven Schema Invalidation

Every API process is a CDC consumer for schema changes. When a schema CDC entry is received, invalidate the CEL cache.

```rust
// CDC consumer task
loop {
    let entry = cdc_stream.next().await?;

    match entry.operation {
        CdcOperation::SchemaRegister { entity_type, schema_version, .. } => {
            // Invalidate local cache
            cel_cache.remove(&entity_type);
            schema_cache.remove(&entity_type);
            // Cache will be repopulated on next query
        },
        _ => {}
    }
}

// Query path
async fn find_nodes(req: FindNodesRequest) -> Result<...> {
    // Check cache first
    let schema = schema_cache.get_or_else(|| {
        // Cache miss: load from FDB
        let schema = fetch_schema_from_fdb(&req.entity_type).await?;
        schema_cache.insert(req.entity_type.clone(), schema.clone());
        schema
    })?;

    let cel_env = cel_cache.get_or_else(|| {
        let env = build_cel_environment(&schema);
        cel_cache.insert(req.entity_type.clone(), env.clone());
        env
    })?;

    // ... compile and execute query
}
```

**Pros**:
- Automatic, passive: every process sees schema changes immediately
- No extra FDB reads on every query
- Consistent with existing CDC architecture

**Cons**:
- Requires every process to be a CDC consumer (operational overhead)
- Schema changes are eventually consistent (bounded by CDC lag, typically ~100ms)

#### Option B: FDB Watch on Schema Keys

Use FDB's `txn.watch()` to observe schema changes. When schema changes, watch fires and cache is invalidated.

```rust
async fn watch_schema(entity_type: &str) {
    let schema_key = (..., "_meta", "schemas", entity_type);

    loop {
        let tx = db.create_transaction()?;
        tx.watch(&schema_key).await?;

        // Watch fired: schema changed
        cel_cache.remove(entity_type);
        schema_cache.remove(entity_type);

        // Re-watch for next change
    }
}
```

**Pros**:
- Immediate notification (FDB watch latency, sub-millisecond)
- No need to be a full CDC consumer
- Fine-grained: watch only relevant schemas

**Cons**:
- Per-process watches + per-schema = many concurrent watches (FDB limits?)
- Watch re-subscription adds complexity

#### Option C: TTL-Based Refresh

Cache schema for a fixed TTL (e.g., 5 seconds). On every query, check TTL; if expired, reload from FDB.

```rust
struct CacheEntry<T> {
    value: T,
    inserted_at: Instant,
    ttl: Duration,
}

async fn find_nodes(req: FindNodesRequest) -> Result<...> {
    let schema = {
        let cached = schema_cache.get(&req.entity_type);
        if let Some(entry) = cached {
            if entry.inserted_at.elapsed() < entry.ttl {
                entry.value.clone()
            } else {
                // TTL expired, reload
                let schema = fetch_schema_from_fdb(&req.entity_type).await?;
                schema_cache.insert(req.entity_type.clone(), CacheEntry {
                    value: schema.clone(),
                    inserted_at: Instant::now(),
                    ttl: Duration::from_secs(5),
                });
                schema
            }
        } else {
            let schema = fetch_schema_from_fdb(&req.entity_type).await?;
            schema_cache.insert(...);
            schema
        }
    };
    // ... compile and execute
}
```

**Pros**:
- Simple, no CDC or watch machinery
- Bounded staleness: schema is never more than 5 seconds old
- One extra FDB read per query if cache expired (acceptable cost)

**Cons**:
- Staleness window: up to 5 seconds, queries may see old schema
- Wasted FDB reads if schema doesn't change frequently (but cheap)

#### Option D: Version Counter in Query Path

Every request includes a schema version hint (or loads it from FDB once per request).

```rust
// Client-side hint (optional)
message FindNodesRequest {
    string entity_type = 1;
    string cel_expression = 2;
    uint32 schema_version_hint = 3;  // optional
}

async fn find_nodes(req: FindNodesRequest) -> Result<...> {
    // Load schema metadata to get current version
    let current_version = fetch_schema_version_from_fdb(&req.entity_type).await?;

    if req.schema_version_hint != current_version {
        // Stale hint or no hint: invalidate cache, use current
        let schema = fetch_schema_from_fdb(&req.entity_type).await?;
        // CEL compile against current schema
    } else {
        // Hint matches current: use cached schema
        let schema = schema_cache.get(&req.entity_type)?;
    }
    // ... compile and execute
}
```

**Pros**:
- Always correct (every request checks version)
- No cache inconsistency possible

**Cons**:
- Extra FDB read on every query (version fetch)
- Performance hit (~1ms per query)

### Interaction with Process Startup

When a new process starts:
- Load all schemas from FDB into cache (expensive if many schemas)
- Or lazy-load on first query to that entity type

With CDC approach: new process joins CDC consumer group, consumes from beginning (or checkpoint), populates cache gradually.

With watch approach: process subscribes to all active schemas as they're queried.

With TTL approach: process queries FDB on demand.

### Recommendation

**Adopt Option A + lazy-load: CDC-Driven with FDB Fallback**

1. **Every API process is a CDC consumer** for schema changes.
2. **Cache schema in memory** with reference counting (TTL of 1 minute as safety fallback).
3. **On schema CDC entry**: invalidate cache entry.
4. **On query**: if cache miss, load from FDB and populate cache.
5. **Startup**: don't preload all schemas; lazy-load on first query.

```rust
struct SchemaCacheEntry {
    schema: EntitySchema,
    version: u32,
    loaded_at: Instant,
    ttl: Duration,  // 60 seconds, safety fallback
}

async fn get_schema(entity_type: &str) -> Result<EntitySchema> {
    // Check cache
    {
        let cache = schema_cache.read();
        if let Some(entry) = cache.get(entity_type) {
            if entry.loaded_at.elapsed() < entry.ttl {
                return Ok(entry.schema.clone());
            }
        }
    }

    // Cache miss or TTL expired: fetch from FDB
    let schema = fetch_schema_from_fdb(entity_type).await?;
    let version = fetch_schema_version_from_fdb(entity_type).await?;

    // Populate cache
    let mut cache = schema_cache.write();
    cache.insert(entity_type.to_string(), SchemaCacheEntry {
        schema: schema.clone(),
        version,
        loaded_at: Instant::now(),
        ttl: Duration::from_secs(60),
    });

    Ok(schema)
}

// CDC consumer loop
async fn schema_cdc_consumer() {
    let mut hwm = load_schema_cdc_hwm().await;
    loop {
        let entries = cdc_stream.get_range(&cdc_subspace.range_from(&hwm)).await?;
        for entry in entries {
            match entry.operation {
                CdcOperation::SchemaRegister { entity_type, schema_version, .. } => {
                    // Invalidate cache
                    schema_cache.write().remove(&entity_type);
                    cel_cache.write().remove(&entity_type);
                },
                _ => {}
            }
            hwm = entry.versionstamp;
        }
        save_schema_cdc_hwm(&hwm).await;
    }
}
```

**Spec updates**:
- Section 3.3: Clarify "CEL cache invalidates on version change" means CDC-driven invalidation.
- Add new subsection: "Schema Registry Consistency" explaining cache invalidation.
- Document startup behavior (lazy-load, not preload).

**Operational notes**:
- Schema changes propagate to all processes within CDC lag time (typically ~100ms).
- During the lag window, some processes may reject or accept the same input differently.
- TTL fallback (60s) catches any missed CDC invalidations.

---

## Gap: CDC Consumer Backpressure

### Problem Statement

CDC entries accumulate in FDB as mutations are written:

```
Time T0:  Write 1000 nodes → 1000 CDC entries
Time T1:  RocksDB projector reads 1000 CDC entries, applies to cache (normal)
Time T2:  Write 100K nodes (bulk import) → 100K CDC entries
Time T3:  RocksDB projector is slow, only processed 10K entries
Time T4:  More writes arrive → CDC grows unbounded, storage keeps accumulating
```

If a consumer (e.g., RocksDB projector) falls behind:
- CDC entries accumulate in FDB storage (7-day retention per Section 3.6)
- Read queries fall through to FDB cache misses, causing FDB load spike
- Watch dispatcher falls behind, live subscriptions have stale events
- Replication streams to remote sites lag

**Critical question**: What slows down the writers? Should there be a mechanism?

### Current Spec

Section 3.6:
```
CDC entries are retained for a configurable duration (default: 7 days)
```

Section 7:
```
Versionstamps are globally ordered across all transactions
Consumer pattern: Forward range scan from last high-water mark
```

**Gap**: No backpressure mechanism. Writes are not throttled if consumers fall behind.

### Storage Growth Analysis

Assume:
- 1 million mutations per day (11.6 per second)
- 7-day retention
- 500 bytes per CDC entry (operations, metadata)

Storage: `1M × 7 × 500B = 3.5 GB`

For large deployments (100M mutations/day): `350 GB` for 7-day retention. For FDB cluster with TB-scale storage, this is acceptable but significant.

**Compaction pressure**: If CDC entries are never deleted (retention expired but not compacted), storage grows unbounded and eventually hits quota.

### Design Options

#### Option A: No Backpressure (Accept Eventually Consistent Reads)

Writers are never throttled. Consumers fall behind gracefully:
- **Cache projector falls behind**: Reads hit FDB directly (slower, but correct).
- **Watch dispatcher falls behind**: Events arrive out-of-order or delayed.
- **Replication lags**: Remote sites see stale data for longer.

**Pros**: No coordination, no writer slowdown.

**Cons**: Users experience read latency spikes when cache is cold. Unmonitored consumer failure can degrade performance.

#### Option B: CDC Consumer Monitoring + Alerting

No active backpressure, but monitoring tracks consumer lag:

```rust
struct ConsumerMetrics {
    consumer_name: &'static str,
    last_hwm: Versionstamp,      // Last processed CDC entry
    fdb_current: Versionstamp,   // Latest CDC entry in FDB
    lag_seconds: f64,             // Time behind
    lag_entries: u64,             // Number of unprocessed entries
}

// Emit metrics periodically
async fn monitor_consumers() {
    loop {
        let fdb_current = get_latest_cdc_versionstamp().await?;

        for consumer in &consumers {
            let last_hwm = consumer.load_hwm().await?;
            let lag = fdb_current - last_hwm;

            emit_metric("cdc_consumer_lag_seconds", lag);
            emit_metric("cdc_consumer_lag_entries", lag_entries);

            if lag > ALERT_THRESHOLD {
                alert("CDC consumer falling behind", consumer.name());
            }
        }
    }
}
```

**Pros**: Operators see problems early, can investigate.

**Cons**: No automatic mitigation. Manual operator response required.

#### Option C: Active Backpressure (Slow Down Writers)

When consumer lag exceeds threshold, writers are throttled.

```rust
async fn write_mutation(mutation: &Mutation) -> Result<()> {
    // Check CDC consumer lag
    let consumer_lag = check_cdc_lag().await?;

    if consumer_lag > BACKPRESSURE_THRESHOLD {
        // Apply adaptive backpressure
        let delay_ms = min(100, consumer_lag / 1000);
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }

    // Proceed with write
    let tx = db.create_transaction()?;
    // ... write mutation to FDB + CDC entry
    Ok(())
}
```

**Pros**: Consumers never fall too far behind. Cache remains warm.

**Cons**: Writer latency increases. Clients see degraded write performance during bulk imports.

#### Option D: CDC Compaction and Expiration

Implement a background job that:
- Reads expired CDC entries (> 7 days old)
- Deletes or archives them
- Manages retention window

```rust
async fn cdc_compaction_job() {
    loop {
        // Find CDC entries older than 7 days
        let cutoff_versionstamp = get_versionstamp_from_7_days_ago().await?;
        let expired_range = cdc_subspace.range(..=cutoff_versionstamp);

        // Delete in batches
        for batch in expired_range.chunked(10000) {
            let mut tx = db.create_transaction()?;
            for (key, _) in batch {
                tx.clear(&key);
            }
            tx.commit().await?;
        }
    }
}
```

**Pros**: Storage bounded. Enforces retention policy.

**Cons**: Doesn't solve backpressure (compaction is cleanup, not flow control).

#### Option E: Flow-Control Protocol (Complex)

Implement producer-consumer flow control similar to Kafka consumer groups:

- Each consumer tracks a committed offset (in FDB, not in memory)
- Writers query consumer offsets before committing
- If any critical consumer is too far behind, writer rejects write with `TEMPORARY_UNAVAILABLE`

```rust
async fn write_mutation(mutation: &Mutation) -> Result<()> {
    let current_versionstamp = db.get_read_version().await?;

    // Check all critical consumers
    for (consumer_name, min_lag_threshold) in CRITICAL_CONSUMERS {
        let consumer_offset = fetch_consumer_offset(consumer_name).await?;
        let lag = current_versionstamp - consumer_offset;

        if lag > min_lag_threshold {
            return Err(tonic::Status::unavailable(
                format!("Consumer {} too far behind", consumer_name)
            ));
        }
    }

    // All consumers healthy: proceed
    let tx = db.create_transaction()?;
    // ... write mutation + CDC entry
    Ok(())
}
```

**Pros**: Precise control. Guarantees critical consumers never lag beyond threshold.

**Cons**: Complex state management. Consumer commits must be durable (write to FDB). High coordination overhead.

### Watch Dispatcher as Special Case

The watch dispatcher is in-memory only. If it falls behind:
- Active subscriptions don't get events
- Client watches time out or receive stale data

**Mitigation**:
- Run watch dispatcher on same process as CDC consumer (co-located)
- Watch dispatcher subscribes to CDC stream directly, not via separate consumer
- No separate lag — watch events are emitted as CDC entries are processed

### Recommendation

**Adopt Option B + Option D:**

1. **Consumer monitoring**: Emit lag metrics per consumer (RocksDB projector, replication streamer, watch dispatcher).
2. **Alerting**: Ops team is alerted if lag exceeds 5 minutes (for example).
3. **CDC compaction**: Background job deletes expired CDC entries after retention window.
4. **No active backpressure**: Let consumers fall behind gracefully; reads degrade to FDB.
5. **Co-locate watch dispatcher**: Run watch dispatcher on same process as CDC consumer to avoid separate lag.

**Future enhancements**:
- Implement Option C (adaptive backpressure) if monitoring shows bulk imports regularly cause consumer lag.
- Implement consumer group semantics (committed offsets in FDB) if multi-consumer scenarios emerge.

**Spec updates**:
- Section 7: Add "CDC Retention and Compaction" subsection.
- Document consumer lag metrics.
- Document alert thresholds for operational readiness.

**Operational runbook**:
- If CDC projector falls behind: check RocksDB memory usage (may need to increase cache size).
- If replication lags: check network bandwidth and latency between sites.
- Manual backpressure: operators can `rate_limit` the API layer independently if needed.

---

## Gap: CDC Retention and Compaction

### Problem Statement

The spec mentions CDC retention in Section 3.6:
```
CDC entries are retained for a configurable duration (default: 7 days)
```

And again in Section 12 (original open questions):
```
CDC retention and compaction: Checkpointing strategy?
```

But there is **no specification** for:
- How retention is enforced (automatic deletion? archive?)
- How compaction works (changelog-style consolidation? tombstone markers?)
- What happens when a consumer asks for CDC entries older than retention window
- How consumers track checkpoints

### Use Cases for Different Retention Strategies

**Scenario 1: Short-term cache rebuild**
- Cache projector crashes
- Operator wants to rebuild RocksDB from CDC
- Needs CDC entries from crash point to now
- Scenario: 1 hour of CDC entries

**Scenario 2: Replica catchup**
- New replica (or remote site) joins
- Wants to replay all historical data
- Scenario: 7 days of CDC entries

**Scenario 3: Audit trail**
- Compliance requires 90 days of audit logs
- Need to keep CDC entries and export them
- Scenario: 90 days of CDC entries

### Design Options

#### Option A: Simple Truncation (Current Spec)

Delete CDC entries older than 7 days in a background job.

```rust
async fn truncate_old_cdc() {
    loop {
        let cutoff = SystemTime::now() - Duration::from_secs(7 * 24 * 3600);
        let cutoff_versionstamp = estimate_versionstamp_at(cutoff);

        // Delete all CDC entries before cutoff
        let mut tx = db.create_transaction()?;
        let range = cdc_subspace.range(..cutoff_versionstamp);
        tx.clear_range(&range);
        tx.commit().await?;
    }
}
```

**Pros**:
- Simple, no state tracking
- Storage bounded

**Cons**:
- Hard cutoff: after 7 days, CDC entries are gone
- Consumers that checkpoint infrequently may miss entries if they reconnect after cutoff
- Audit trail is lost

#### Option B: Tiered Storage

Keep recent CDC entries in FDB (7 days), older entries in cold storage (archive).

```
FDB (_cdc subspace)      [recent 7 days]
    ↓
Archive (S3/GCS)         [7 days - 90 days]
    ↓
Deleted                  [older than 90 days]
```

**Pros**:
- Long retention without bloating FDB
- Audit trail available in archive

**Cons**:
- Complex: manage two storage systems
- Consumer must check both FDB and archive

#### Option C: CDC Changelog Compaction (Like RocksDB/Kafka)

Instead of deleting, consolidate: if node_123 is created, updated, updated, delete — the CDC changelog is reduced to a single "delete" entry.

```
Before compaction:
  vs_1001: create node_123 { age: 30 }
  vs_1002: update node_123 { age: 31 }
  vs_1003: update node_123 { age: 32 }
  vs_1004: delete node_123

After compaction:
  vs_1004: delete node_123    (sole entry, others consolidated)
```

**Pros**:
- Reduces CDC size dramatically (especially for hot nodes with many updates)
- Retention can be longer without storage explosion

**Cons**:
- Complex state machine to merge operations
- Lost visibility into node history

#### Option D: Checkpoint-Based Retention

Consumers commit a checkpoint (high-water mark) to FDB. Retention policy: keep CDC entries only for consumers with committed checkpoints.

```rust
struct ConsumerCheckpoint {
    consumer_id: String,
    last_processed_versionstamp: Versionstamp,
    committed_at: Instant,
}

// In _meta subspace
("myapp", "_meta", "cdc_checkpoints", "rocksdb_projector") → vs_1234567
("myapp", "_meta", "cdc_checkpoints", "replication_streamer") → vs_1234560

// Retention: keep CDC entries >= min(all checkpoints)
let min_checkpoint = fetch_all_checkpoints().await?
    .iter()
    .min_by_key(|cp| cp.last_processed_versionstamp)
    .unwrap();

let retention_start = min_checkpoint.last_processed_versionstamp;
// Entries before retention_start can be deleted
```

**Pros**:
- Retention is dynamic, based on consumer progress
- Slow consumers force retention to extend
- Clear semantics: checkpoint tracks position

**Cons**:
- Requires consumer coordination (checkpoint updates)
- Stalled consumers block truncation (storage fills up)

### Checkpoint Semantics

A consumer's checkpoint represents: "I have safely applied all CDC entries up to and including this versionstamp."

**Update pattern**:
```rust
async fn cdc_consumer_loop() {
    let mut checkpoint = load_checkpoint().await?;

    loop {
        let new_entries = fetch_cdc_entries_since(checkpoint).await?;

        for entry in new_entries {
            apply_entry(entry).await?;
            checkpoint = entry.versionstamp;
        }

        // Periodically commit checkpoint
        if elapsed_since_last_commit > 5_seconds {
            save_checkpoint(checkpoint).await?;  // Write to FDB
        }
    }
}
```

**Crash safety**: On restart, consumer loads checkpoint from FDB and resumes from next entry.

### Recommendation

**Adopt Option A (simple truncation) + Option D (checkpoints):**

**Implementation**:
1. Default retention: 7 days (hard cutoff).
2. Background job: delete CDC entries older than cutoff daily.
3. Consumer crash scenario: if projector crashes and restarts within 7 days, resume from last in-memory HWM. If restart is > 7 days later, full CDC replay is lost (operator rebuilds cache manually).
4. Implement checkpoint-based retention: consumers write checkpoints to FDB.
5. Retention policy: keep CDC entries since oldest committed checkpoint (or minimum 1 day).
6. Backpressure: if any consumer is stalled (checkpoint not updating), operator is alerted.

**Spec updates**:
- Section 3.6: Expand "CDC replay for cache rebuild" with checkpoint semantics.
- Document retention configuration (7 days default, min 1 day, max 90 days).
- Document consumer checkpoint mechanism.
- Add operational guidance on checkpoint monitoring.

**Implementation details**:
```rust
// CDC consumer with checkpoint
struct CdcConsumer {
    consumer_id: String,
    checkpoint_key: FdbKey,  // ("ns", "_meta", "cdc_checkpoints", consumer_id)
}

impl CdcConsumer {
    async fn load_checkpoint(&self) -> Result<Versionstamp> {
        db.get(&self.checkpoint_key).await
            .map(|bytes| Versionstamp::from_bytes(&bytes))
            .or_else(|_| Ok(Versionstamp::default()))  // First run: start from 0
    }

    async fn save_checkpoint(&self, vs: Versionstamp) -> Result<()> {
        db.set(&self.checkpoint_key, vs.to_bytes());
        db.commit().await
    }
}

// Truncation job
async fn truncate_expired_cdc() {
    loop {
        let all_checkpoints = fetch_all_cdc_checkpoints().await?;
        let oldest = all_checkpoints.iter().map(|cp| cp.1).min()
            .unwrap_or_else(|| get_versionstamp_from_7_days_ago());

        let retention_start = oldest.max(get_versionstamp_from_7_days_ago());

        // Delete all CDC entries before retention_start
        db.clear_range(&cdc_subspace.range(..retention_start)).await?;
    }
}
```

---

## Summary of Recommendations

| Gap | Recommendation |
|---|---|
| **I4** | Snapshot reads with mandatory pagination. Acquire read version at query start, pin all reads. 5-second timeout with cursor checkpointing for long traversals. |
| **Auth/Authz** | Namespaced API keys + role-based access (read_only, read_write, admin). gRPC interceptor for auth validation. mTLS for inter-site replication. Future: JWT bearer tokens, entity-type-level permissions. |
| **Error Model** | Structured error response with gRPC status codes + ErrorDetail proto. Enumerate all error codes and categories. Client SDKs provide typed error classes. |
| **Schema Registry** | CDC-driven invalidation + lazy FDB load. Every process is a CDC consumer for schema changes. TTL fallback (60s) for safety. Startup: lazy-load, not preload. |
| **CDC Backpressure** | Monitoring + alerting (no active backpressure) with co-located watch dispatcher. Future: Adaptive backpressure if bulk imports regularly cause consumer lag. |
| **CDC Retention** | Simple 7-day truncation with checkpoint-based retention for dynamic policy. Operator alerted if checkpoint stalls. |

---

*Document authors: Andrew + Claude*
*Last updated: 2026-02-10*
*Related documents: Graph Database Engine Architecture Spec v0.3*
