# PelagoDB: Complete Specification v0.4
## Graph Database for VFX Production Pipelines

**Status:** Draft v0.4 — Implementation Ready
**Author:** Andrew + Claude
**Date:** 2026-02-13
**Changelog:**
- v0.4: Consolidated all design documents into single authoritative spec. FDB keyspace spec adopted as authoritative. Traversal included in Phase 1. Complete proto definitions. Error code registry. 9-byte node ID format. Snapshot reads. Site ID always required.

---

## 1. Introduction

### 1.1 Purpose

PelagoDB is a distributed graph database built on FoundationDB, designed for VFX production pipelines. It provides:

- Sub-millisecond point lookups
- Property-based queries with CEL predicates
- Multi-hop graph traversal with streaming results
- Schema-from-client patterns with runtime validation
- Multi-site replication with ownership-based conflict resolution
- Lifecycle management for production data (hot → warm → cold)

### 1.2 Goals

1. **Point lookups are fast.** Single-node retrieval by ID should be sub-millisecond at the storage layer.
2. **Criteria/scan lookups are performant.** Property-based queries against indexed fields compile to FDB range scans.
3. **Graph traversal is first-class.** Multi-hop traversal with per-hop filtering streams results asynchronously.
4. **Schema is a client concern.** The API layer declares schema; the storage layer enforces and indexes.
5. **Multi-site replication with ownership semantics.** Nodes have home sites; conflict-free replication via CDC.
6. **Lifecycle management via namespacing.** Projects transition from active (hot) to inactive (warm/cold).

### 1.3 Non-Goals (v1)

- Full-text search (delegate to external search engine)
- Graph analytics / batch processing (OLAP workloads)
- SQL compatibility
- Multi-model (document, relational) — graph-first

### 1.4 Technology Stack

| Component | Choice | Rationale |
|-----------|--------|-----------|
| Language | Rust | Sub-millisecond hot paths, zero-copy serialization |
| Async runtime | tokio | Task groups for traversal fan-out |
| Primary storage | FoundationDB | Ordered KV with strict serializable ACID |
| Query predicates | CEL (cel-rust) | Portable expression language, type-checkable |
| Wire transport | gRPC (tonic) | Streaming RPCs, protobuf framing |
| Key encoding | FDB tuple layer | Lexicographic ordering, type-aware comparison |
| Value encoding | CBOR | Compact binary, schema-flexible |

---

## 2. Architecture Overview

```
┌─────────────────────────────────────────────────────────┐
│                     Client SDK                          │
│  Schema registration · Typed queries · Stream consumer  │
└───────────────────────┬─────────────────────────────────┘
                        │ gRPC (protobuf frames)
                        │
┌───────────────────────▼─────────────────────────────────┐
│                    API Layer                             │
│  Schema validation · CEL compilation · Query planning   │
│  gRPC service handlers · Rate limiting                  │
│  Traversal limit enforcement                            │
└───────┬───────────────────────────────┬─────────────────┘
        │                               │
        ▼                               ▼
┌───────────────────┐    ┌──────────────────────────────┐
│  Query Executor   │    │     Mutation Pipeline         │
│  Index selection  │    │  Validate → FDB write + CDC  │
│  Traversal engine │    │  (unified path for all ops)  │
└───────┬───────────┘    └──────┬───────────────────────┘
        │                       │
        ▼                       ▼
┌─────────────────────────────────┐
│         FoundationDB            │
│  (system of record)             │
│  Entities, edges, indexes,      │
│  CDC stream, schema registry    │
└─────────────────────────────────┘
```

### 2.1 Crate Layout

```
pelago/
├── Cargo.toml                 # workspace root
├── proto/
│   └── pelago.proto           # gRPC service definitions
├── crates/
│   ├── pelago-proto/          # generated protobuf/tonic code
│   ├── pelago-core/           # shared types, encoding, errors
│   │   └── src/
│   │       ├── types.rs       # NodeId, EdgeId, Value
│   │       ├── encoding.rs    # tuple encoding, CBOR helpers
│   │       ├── schema.rs      # EntitySchema, PropertyDef
│   │       ├── errors.rs      # PelagoError, ErrorCode
│   │       └── config.rs      # ServerConfig
│   ├── pelago-storage/        # FDB interactions
│   │   └── src/
│   │       ├── subspace.rs    # Subspace builder
│   │       ├── schema.rs      # Schema CRUD
│   │       ├── node.rs        # Node CRUD + indexes
│   │       ├── edge.rs        # Edge CRUD
│   │       ├── index.rs       # Index operations
│   │       ├── cdc.rs         # CDC entry management
│   │       ├── ids.rs         # ID allocator
│   │       └── jobs.rs        # Background jobs
│   ├── pelago-query/          # CEL compilation, query planning
│   │   └── src/
│   │       ├── cel.rs         # CEL parse, type-check
│   │       ├── planner.rs     # Index matching, plan selection
│   │       ├── executor.rs    # Query execution
│   │       ├── traversal.rs   # Multi-hop traversal engine
│   │       └── pql/           # PQL parser (Phase 1)
│   ├── pelago-api/            # gRPC service handlers
│   │   └── src/
│   │       ├── schema_service.rs
│   │       ├── node_service.rs
│   │       ├── edge_service.rs
│   │       ├── query_service.rs
│   │       └── admin_service.rs
│   └── pelago/                # binary entrypoint
│       └── src/main.rs
└── tests/
    └── integration/           # Rust integration tests
```

---

## 3. FDB Keyspace Layout

**This section is authoritative.** All key layouts reference this specification.

### 3.1 Directory Structure

```
/pelago/
  /_sys/                     → System-wide configuration
  /_auth/                    → Global authorization (Phase 3)
  /<db>/
    /_db/                    → DB-level metadata
    /<ns>/
      /_ns/                  → Namespace metadata
      /_schema/              → Schema definitions
      /_types/               → Entity type registry
      /_cdc/                 → CDC stream
      /_jobs/                → Background jobs
      /_ids/                 → ID allocation counters
      /data/                 → Entity storage
      /loc/                  → Locality index
      /idx/                  → Secondary indexes (system + user property)
      /edge/                 → Edge storage
      /xref/                 → Cross-project reference tracking
```

### 3.2 System Configuration

**Subspace:** `/pelago/_sys/`

| Key | Value | Description |
|-----|-------|-------------|
| `(config)` | System config CBOR | Global system settings |
| `(dc, <dc_id>)` | DC config CBOR | Datacenter registry |
| `(dc_topo, <dc_a>, <dc_b>)` | Topology CBOR | DC-to-DC network characteristics |
| `(repl, <lifecycle>)` | String array | Replication set per lifecycle |

### 3.3 DB Metadata

**Subspace:** `/pelago/<db>/_db/`

| Key | Value | Description |
|-----|-------|-------------|
| `(config)` | DB config CBOR | Database configuration |
| `(lock)` | Lock CBOR or null | Write lock state |
| `(hist, <ts>)` | History event CBOR | Audit history |
| `(ns, <ns_name>)` | Namespace info | Namespace registry entry |

### 3.4 Namespace Metadata

**Subspace:** `/pelago/<db>/<ns>/_ns/`

| Key | Value | Description |
|-----|-------|-------------|
| `(config)` | NS config CBOR | Namespace configuration |
| `(lock)` | Lock CBOR or null | Write lock state |
| `(stats)` | Statistics CBOR | Entity/edge counts (async updated) |

### 3.5 Schema Definitions

**Subspace:** `/pelago/<db>/<ns>/_schema/`

| Key | Value | Description |
|-----|-------|-------------|
| `(<type>, v, <version>)` | Schema CBOR | Versioned schema definition |
| `(<type>, latest)` | Integer | Current version pointer |

### 3.6 Entity Storage

**Subspace:** `/pelago/<db>/<ns>/data/`

| Key | Value | Description |
|-----|-------|-------------|
| `(<type>, <node_id>)` | Entity CBOR | Primary entity storage |

**Entity Record Schema:**
```json
{
  "p": {},          // payload (user properties)
  "s": 3,           // schema_version
  "l": "SF",        // locality (owning DC)
  "t": null,        // tombstoned_at (timestamp or null)
  "c": "<ts>",      // created_at
  "u": "<ts>",      // updated_at
  "cb": "user_123", // created_by
  "ub": "user_456"  // updated_by
}
```

### 3.7 Secondary Indexes

**Subspace:** `/pelago/<db>/<ns>/idx/`

| Key | Value | Description |
|-----|-------|-------------|
| `(tomb, <type>, <id>)` | Timestamp | Tombstoned entities |
| `(byloc, <dc>, <type>, <id>)` | `0x01` | Entities by locality |
| `(byschema, <type>, <version>, <id>)` | `0x01` | Entities by schema version |
| `(byupd, <type>, <versionstamp>)` | Entity ID | By update time (versionstamp key) |
| `(prop, <type>, <field>, unique, <value>)` | Node ID bytes | Unique property index |
| `(prop, <type>, <field>, eq, <value>, <id>)` | `0x01` | Equality property index |
| `(prop, <type>, <field>, range, <encoded_value>, <id>)` | `0x01` | Range property index |

**User Property Indexes:** The `(prop, ...)` keys support property-based queries with three index types:
- **unique**: Value maps to single node ID. Write rejects duplicates.
- **eq (equality)**: Multiple nodes per value. Point lookup.
- **range**: Value sort-order encoded. Range scans supported.

### 3.8 Edge Storage

**Subspace:** `/pelago/<db>/<ns>/edge/`

| Key | Value | Description |
|-----|-------|-------------|
| `(f, <src_type>, <src_id>, <label>, <sort_key>, <tgt_db>, <tgt_ns>, <tgt_type>, <tgt_id>, <edge_id>)` | Edge CBOR | Forward edge |
| `(r, <tgt_db>, <tgt_ns>, <tgt_type>, <tgt_id>, <label>, <sort_key>, <src_type>, <src_id>, <edge_id>)` | `0x01` | Reverse edge |

**Sort Key Placement:** When an edge type declares a `sort_key` property, that value is encoded in the key position to enable vertex-centric range scans. If no sort_key, use constant `0x00`.

**Edge Data Schema:**
```json
{
  "properties": {"role": "Engineer"},
  "_owner": "SF",
  "_pair_id": "pair_001",  // For bidirectional edges
  "_created_at": 1705363200
}
```

### 3.9 CDC Stream

**Subspace:** `/pelago/<db>/<ns>/_cdc/`

| Key | Value |
|-----|-------|
| `(<versionstamp>)` | CDC entry CBOR |

CDC entries use FDB versionstamps as keys for zero-contention, globally-ordered writes.

### 3.10 Background Jobs

**Subspace:** `/pelago/<db>/<ns>/_jobs/`

| Key | Value |
|-----|-------|
| `(<job_type>, <job_id>)` | Job state CBOR |

### 3.11 ID Allocation

**Subspace:** `/pelago/<db>/<ns>/_ids/`

| Key | Value |
|-----|-------|
| `(<entity_type>, <site_id>)` | Counter (u64) |

---

## 4. Data Model

### 4.1 Node ID Format

Node IDs use a 9-byte format that encodes origin site:

```rust
pub struct NodeId {
    pub site: u8,    // Owning site (0-255)
    pub seq: u64,    // Monotonic counter per site per entity type
}

impl NodeId {
    pub fn to_bytes(&self) -> [u8; 9] {
        let mut buf = [0u8; 9];
        buf[0] = self.site;
        buf[1..9].copy_from_slice(&self.seq.to_be_bytes());
        buf
    }

    pub fn from_bytes(bytes: &[u8; 9]) -> Self {
        Self {
            site: bytes[0],
            seq: u64::from_be_bytes(bytes[1..9].try_into().unwrap()),
        }
    }

    pub fn home_site(&self) -> u8 {
        self.site
    }
}
```

**Properties:**
- **9 bytes total**: 1 byte site ID + 8 bytes big-endian counter
- **Site extractable**: `node_id.home_site()` returns owning site
- **No coordination**: Each site allocates from its own counter
- **Display format**: `"{site}_{seq}"` (e.g., `"1_12345"`)

### 4.2 Edge ID Format

Same format as NodeId: `[site_id: u8][counter: u64]`

```rust
pub struct EdgeId {
    pub site: u8,
    pub seq: u64,
}
```

Edge IDs are allocated from a separate counter than node IDs.

### 4.3 Property Types

| Type | Rust | FDB Key Encoding | CBOR | CEL Type |
|------|------|------------------|------|----------|
| `string` | `String` | null-terminated UTF-8 | text | `string` |
| `int` | `i64` | big-endian with sign flip | integer | `int` |
| `float` | `f64` | IEEE 754 order-preserving | float | `double` |
| `bool` | `bool` | single byte | boolean | `bool` |
| `timestamp` | `i64` | big-endian (unix micros) | integer | `timestamp` |
| `bytes` | `Vec<u8>` | raw bytes | bytes | `bytes` |
| `null` | (absence) | not indexed | (omitted) | `null` |

### 4.4 Null Semantics

Missing properties and null values are unified: both mean "this property has no value."

**Core rules:**
1. If a property is not present in CBOR, it is null
2. Null values are NOT stored in CBOR — absence IS null
3. Required fields cannot be null (validated at write time)
4. Null values are NOT indexed

**CEL null handling:**

| Expression | Field is Null | Result |
|------------|--------------|--------|
| `field > value` | yes | `false` |
| `field >= value` | yes | `false` |
| `field < value` | yes | `false` |
| `field <= value` | yes | `false` |
| `field == value` | yes | `false` |
| `field == null` | yes | `true` |
| `field != null` | yes | `false` |
| `field.startsWith(x)` | yes | `false` |

**Boolean logic:** Null is falsy. `null && true` → `false`, `null || true` → `true`

---

## 5. Schema System

### 5.1 Schema Structure

```rust
pub struct EntitySchema {
    pub datastore: String,
    pub namespace: String,
    pub name: String,              // Entity type name
    pub version: u32,              // Server auto-incremented
    pub properties: HashMap<String, PropertyDef>,
    pub edges: HashMap<String, EdgeDef>,
    pub meta: SchemaMeta,
}

pub struct PropertyDef {
    pub property_type: PropertyType,
    pub required: bool,
    pub index: Option<IndexType>,
    pub default: Option<Value>,
}

pub enum IndexType {
    Unique,     // Equality, enforces uniqueness
    Equality,   // Equality lookups, multiple values
    Range,      // Range scans supported
}

pub struct EdgeDef {
    pub target: EdgeTarget,        // Specific("Company") or Polymorphic
    pub direction: EdgeDirection,  // Outgoing, Bidirectional
    pub properties: HashMap<String, PropertyDef>,
    pub sort_key: Option<String>,  // Property for vertex-centric index
    pub ownership: OwnershipMode,  // SourceSite (default), Independent
}

pub struct SchemaMeta {
    pub allow_undeclared_edges: bool,
    pub extras_policy: ExtrasPolicy,  // Reject, Allow, Warn
}

pub enum ExtrasPolicy {
    Reject,  // Reject undeclared properties
    Allow,   // Accept and store undeclared properties
    Warn,    // Accept but log warning (server-side diagnostics)
}
```

### 5.2 Schema JSON Format

```json
{
  "namespace": "core",
  "entity": {
    "name": "Person",
    "properties": {
      "name": {
        "type": "string",
        "required": true,
        "index": "unique"
      },
      "age": {
        "type": "int",
        "index": "range"
      },
      "email": {
        "type": "string",
        "required": true,
        "index": "unique"
      }
    },
    "edges": {
      "KNOWS": {
        "target": "Person",
        "direction": "bidirectional",
        "properties": {
          "since": { "type": "timestamp", "sort_key": true }
        }
      },
      "WORKS_AT": {
        "target": "Company",
        "direction": "outgoing",
        "properties": {
          "role": { "type": "string" },
          "started": { "type": "timestamp", "sort_key": true }
        }
      }
    },
    "meta": {
      "allow_undeclared_edges": false,
      "extras_policy": "reject"
    }
  }
}
```

### 5.3 Schema Registration Rules

| Scenario | Behavior |
|----------|----------|
| Node creation without registered type | **REJECT** |
| Edge declaration to unregistered target type | **ALLOW** (forward reference) |
| Edge creation to unregistered target type | **REJECT** |
| Edge creation to non-existent target node | **REJECT** |
| Undeclared edge type | Depends on `allow_undeclared_edges` |
| Undeclared property | Depends on `extras_policy` |

**Forward references:** When schema A declares edges to type B, and B isn't registered yet, schema registration succeeds. Edge creation validates target type at creation time.

### 5.4 Schema Versioning

- Server auto-increments version on `RegisterSchema`
- Client does not supply version number
- `RegisterSchemaResponse` returns assigned version
- All versions retained in `/_schema/(<type>, v, <version>)`
- Schema cache invalidates on version change

### 5.5 Schema Evolution

**Adding properties:**
- Non-indexed: Immediate, no backfill
- Indexed: Background job backfills existing nodes

**Removing properties:**
1. Server stops validating property on writes
2. CEL queries referencing property fail type-check
3. Old data remains in CBOR (invisible to queries)
4. Old indexes become orphaned
5. Client explicitly calls `DropIndex` RPC to clean up
6. Client optionally calls `StripProperty` RPC to remove from CBOR

---

## 6. Node Operations

### 6.1 Create Node

**Validation:**
1. Entity type must be registered
2. Required properties present
3. Property types match schema
4. `extras_policy` checked for undeclared properties
5. Unique index constraints checked

**Single FDB Transaction:**
1. Allocate NodeId from counter
2. Write data: `/data/(<type>, <node_id>)` → CBOR
3. Write locality: `/loc/(<type>, <node_id>)` → site_id
4. Write indexes for each indexed property
5. Write system indexes (byloc, byschema, byupd)
6. Append CDC entry

**Node CBOR includes:**
- `p`: User properties
- `s`: Schema version
- `l`: Locality (owning site)
- `t`: null (not tombstoned)
- `c`, `u`: Created/updated timestamps
- `cb`, `ub`: Created/updated by user

### 6.2 Get Node

Direct FDB `get()` on `/data/(<type>, <node_id>)`.

**Response excludes system fields unless requested.**

### 6.3 Update Node

1. Read current node data
2. Validate changed properties
3. Compute index diff (old values to remove, new to add)
4. Single transaction: update data + index maintenance + CDC
5. Optimistic concurrency via FDB conflict ranges

### 6.4 Delete Node

1. Read current node (need property values for index cleanup)
2. Scan all outgoing edges from node
3. Scan all incoming edges to node
4. Single transaction:
   - Delete data key
   - Delete all index entries
   - Delete all outgoing edge entries
   - Delete corresponding OUT entries at source nodes
   - Append CDC entry

For high-degree nodes, edge cleanup moves to background job.

### 6.5 ID Allocation

**Batch allocation** to reduce contention:
1. Atomic increment counter by batch size (e.g., 100)
2. Process allocates IDs from batch in memory
3. On crash, remaining batch IDs are "leaked" (acceptable — IDs are not guaranteed sequential)

**Counter location:** `/_ids/(<entity_type>, <site_id>)` → `u64`

---

## 7. Edge Operations

### 7.1 Edge Create

**Validation:**
1. Source node exists
2. Target node exists
3. Edge type validated (if declared in schema)
4. Target type matches declaration
5. Edge properties validated

**Single FDB Transaction:**
1. Allocate EdgeId
2. Extract sort_key value (or use `0x00`)
3. Write forward edge: `/edge/(f, ...)`
4. Write reverse edge: `/edge/(r, ...)`
5. For bidirectional: generate `pair_id`, write both directions
6. Append CDC entry

### 7.2 Bidirectional Edges

Bidirectional edges are implemented as **paired directed edges**:

```
Person_42 --[KNOWS]--> Person_99   [pair_id="pair_001"]
Person_99 --[KNOWS]<-- Person_42   [pair_id="pair_001"]
```

- Single `CreateEdge` call writes both directions
- `pair_id` ties them together
- Deleting one deletes both
- Each direction owned by its source node's home site

### 7.3 Edge Delete

1. Lookup edge to get edge_id and metadata
2. Verify ownership
3. Delete forward entry
4. Delete reverse entry
5. For bidirectional: delete paired entries
6. Append CDC entry

### 7.4 Edge Listing

Range scan on edge subspace:
- By source + edge type + direction: prefix scan
- Sort key in key position enables ordered results
- Streaming response

### 7.5 Vertex-Centric Indexes

When edge type declares `sort_key`, range predicates compile to subspace range scans:

```
Query: WORKS_AT edges from Person_42 where started >= 2020-01-01
Key range: (f, Person, <p_42>, WORKS_AT, [1577836800000000, MAX], ...)
```

This enables efficient time-ordered edge traversal without separate index structures.

---

## 8. Query System

### 8.1 CEL Integration

Build CEL type environment from EntitySchema:

```rust
fn build_cel_env(schema: &EntitySchema) -> CelEnv {
    let mut env = CelEnv::new();
    for (name, prop) in &schema.properties {
        env.add_variable(name, prop.property_type.to_cel_type());
    }
    env
}
```

**Pipeline:**
1. Parse CEL string → AST
2. Type-check against schema environment
3. Error on unknown properties (removed from schema = type-check error)
4. Apply null semantics

### 8.2 Predicate Extraction

Walk AST, extract conjuncts from top-level AND:

```
"age >= 30 && name == 'Andrew'"
→ [age >= 30] AND [name == 'Andrew']
```

Classify each predicate: field, operator, value.

### 8.3 Index Matching

Match predicates to available indexes:

| CEL Expression | Index Required | FDB Operation |
|----------------|----------------|---------------|
| `field == value` | unique/equality | Point lookup |
| `field > value` | range | Range scan (value+1, MAX) |
| `field >= value` | range | Range scan (value, MAX) |
| `field < value` | range | Range scan (MIN, value) |
| `field.startsWith(x)` | range (string) | Range scan (x, x+\xFF) |
| `field.contains(x)` | NONE | Residual filter |

### 8.4 Query Plan

```rust
pub enum QueryPlan {
    PointLookup {
        index_field: String,
        value: Value,
        estimated_rows: f64,
    },
    RangeScan {
        index_field: String,
        lower: Option<Bound>,
        upper: Option<Bound>,
        estimated_rows: f64,
    },
    FullScan {
        entity_type: String,
        estimated_rows: f64,
    },
}

pub struct ExecutionPlan {
    pub primary: QueryPlan,
    pub residual: Option<CompiledCelExpr>,
    pub projection: Vec<String>,
    pub limit: Option<u32>,
    pub cursor: Option<Cursor>,
}
```

**Phase 1 strategy:** Single-best index + residual filter. Most selective indexed predicate chosen heuristically.

### 8.5 Snapshot Reads

All queries acquire a read version at start for snapshot consistency:

```rust
async fn execute_find_nodes(request: FindNodesRequest) -> Result<Stream<NodeResult>> {
    let db = fdb.database();
    let read_version = db.get_read_version().await?;

    // All subsequent reads use this version
    let txn = db.create_transaction()?;
    txn.set_read_version(read_version);

    // Execute query with pinned read version
    execute_with_txn(txn, request).await
}
```

**Benefits:**
- Index scan + data load sees consistent snapshot
- No phantom entries from concurrent deletes
- Queries exceeding 5 seconds rejected with `QueryTimeout`

### 8.6 FindNodes Execution

1. Execute primary plan (index scan or full scan)
2. For each candidate: load node data
3. Apply residual CEL filter
4. Apply field projection
5. Stream results with backpressure
6. Enforce limit, return cursor for continuation

---

## 9. Traversal System

### 9.1 TraverseRequest

```protobuf
message TraverseRequest {
  RequestContext context = 1;
  oneof start {
    NodeRef start_node = 2;           // Single start node
    FindNodesRequest start_query = 3; // Query for start nodes
  }
  repeated TraversalStep steps = 4;
  uint32 max_depth = 5;               // Default: 4, max: 10
  uint32 timeout_ms = 6;              // Default: 5000ms, max: 30000ms
  uint32 max_results = 7;             // Default: 10000
  bool cascade = 8;                   // Require all paths exist
  RecurseConfig recurse = 9;          // Variable-depth traversal
}

message TraversalStep {
  string edge_type = 1;
  EdgeDirection direction = 2;        // OUT, IN, BOTH
  string edge_filter = 3;             // CEL on edge properties
  string node_filter = 4;             // CEL on target node
  repeated string fields = 5;         // Node field projection
  uint32 per_node_limit = 6;          // Fan-out control per source
  repeated string edge_fields = 7;    // Edge property projection
  SortSpec sort = 8;
}
```

### 9.2 Execution Model

```
Hop 1: From start nodes, traverse step[0]
    │
    ├─ Scan edges matching step[0].edge_type + direction
    ├─ Apply step[0].edge_filter (CEL on edge properties)
    ├─ Load target nodes
    ├─ Apply step[0].node_filter (CEL on node properties)
    ├─ Apply per_node_limit
    └─ Stream results + feed into Hop 2
         │
Hop 2: From Hop 1 results, traverse step[1]
    └─ ...
```

**Streaming:** Results from each hop stream back immediately. Client receives depth indicator, path breadcrumb, and node/edge at each hop.

### 9.3 Server-Enforced Limits

| Parameter | Default | Maximum |
|-----------|---------|---------|
| `max_depth` | 4 | 10 |
| `timeout_ms` | 5000 | 30000 |
| `max_results` | 10000 | 100000 |
| `per_node_limit` | 100 | 1000 |

Client can request lower limits but cannot exceed server maximums.

### 9.4 @cascade Directive

When `cascade = true`, only return paths where ALL nested levels have results:

```
Person -> KNOWS -> Person -> WORKS_AT -> Company
```

If any person has no WORKS_AT edges, exclude that entire path from the first person's results.

### 9.5 @recurse (Variable-Depth)

```protobuf
message RecurseConfig {
  uint32 max_depth = 1;
  bool detect_cycles = 2;  // Default: true
}
```

Traverses a single edge type repeatedly until max depth or no more results.

---

## 10. gRPC API

### 10.1 Complete Proto Definitions

```protobuf
syntax = "proto3";
package pelago.v1;

// ─────────────────────────────────────────────────────────────
// Common Types
// ─────────────────────────────────────────────────────────────

message RequestContext {
  string datastore = 1;
  string namespace = 2;
  string request_id = 3;
}

message NodeRef {
  string entity_type = 1;
  bytes node_id = 2;       // 9 bytes: [site:u8][seq:u64]
}

message EdgeRef {
  bytes edge_id = 1;
  string edge_type = 2;
  NodeRef source = 3;
  NodeRef target = 4;
}

message Value {
  oneof kind {
    string string_value = 1;
    int64 int_value = 2;
    double float_value = 3;
    bool bool_value = 4;
    int64 timestamp_value = 5;
    bytes bytes_value = 6;
    bool is_null = 7;
  }
}

enum ReadConsistency {
  STRONG = 0;     // Direct FDB read
  SESSION = 1;    // Bounded staleness
  EVENTUAL = 2;   // Cache only
}

enum EdgeDirection {
  OUT = 0;
  IN = 1;
  BOTH = 2;
}

// ─────────────────────────────────────────────────────────────
// Schema Service
// ─────────────────────────────────────────────────────────────

service SchemaService {
  rpc RegisterSchema(RegisterSchemaRequest) returns (RegisterSchemaResponse);
  rpc GetSchema(GetSchemaRequest) returns (GetSchemaResponse);
  rpc ListSchemas(ListSchemasRequest) returns (ListSchemasResponse);
  rpc GetSchemaHistory(GetSchemaHistoryRequest) returns (GetSchemaHistoryResponse);
}

message RegisterSchemaRequest {
  RequestContext context = 1;
  string schema_json = 2;      // JSON schema definition
}

message RegisterSchemaResponse {
  string entity_type = 1;
  uint32 version = 2;          // Server-assigned version
  repeated string index_backfill_jobs = 3;  // Job IDs for new indexes
}

message GetSchemaRequest {
  RequestContext context = 1;
  string entity_type = 2;
  optional uint32 version = 3; // Omit for current version
}

message GetSchemaResponse {
  string entity_type = 1;
  uint32 version = 2;
  string schema_json = 3;
}

message ListSchemasRequest {
  RequestContext context = 1;
}

message ListSchemasResponse {
  repeated SchemaSummary schemas = 1;
}

message SchemaSummary {
  string entity_type = 1;
  uint32 current_version = 2;
  int64 entity_count = 3;
}

message GetSchemaHistoryRequest {
  RequestContext context = 1;
  string entity_type = 2;
}

message GetSchemaHistoryResponse {
  repeated SchemaVersion versions = 1;
}

message SchemaVersion {
  uint32 version = 1;
  int64 created_at = 2;
  string schema_json = 3;
}

// ─────────────────────────────────────────────────────────────
// Node Service
// ─────────────────────────────────────────────────────────────

service NodeService {
  rpc CreateNode(CreateNodeRequest) returns (CreateNodeResponse);
  rpc GetNode(GetNodeRequest) returns (GetNodeResponse);
  rpc UpdateNode(UpdateNodeRequest) returns (UpdateNodeResponse);
  rpc DeleteNode(DeleteNodeRequest) returns (DeleteNodeResponse);
  rpc BatchCreateNodes(BatchCreateNodesRequest) returns (BatchCreateNodesResponse);
}

message CreateNodeRequest {
  RequestContext context = 1;
  string entity_type = 2;
  map<string, Value> properties = 3;
}

message CreateNodeResponse {
  NodeRef node = 1;
}

message GetNodeRequest {
  RequestContext context = 1;
  string entity_type = 2;
  bytes node_id = 3;
  repeated string fields = 4;         // Empty = all fields
  ReadConsistency consistency = 5;
}

message GetNodeResponse {
  NodeRef node = 1;
  map<string, Value> properties = 2;
  NodeMetadata metadata = 3;
}

message NodeMetadata {
  uint32 schema_version = 1;
  string locality = 2;
  int64 created_at = 3;
  int64 updated_at = 4;
}

message UpdateNodeRequest {
  RequestContext context = 1;
  string entity_type = 2;
  bytes node_id = 3;
  map<string, Value> properties = 4;  // Merge semantics
}

message UpdateNodeResponse {
  NodeRef node = 1;
  map<string, Value> properties = 2;
}

message DeleteNodeRequest {
  RequestContext context = 1;
  string entity_type = 2;
  bytes node_id = 3;
}

message DeleteNodeResponse {
  bool success = 1;
  int32 edges_deleted = 2;
}

message BatchCreateNodesRequest {
  RequestContext context = 1;
  string entity_type = 2;
  repeated map<string, Value> nodes = 3;
  optional string batch_id = 4;       // For CDC tracking
}

message BatchCreateNodesResponse {
  repeated NodeRef nodes = 1;
  int32 failed_count = 2;
  repeated BatchError errors = 3;
}

message BatchError {
  int32 index = 1;
  string error_code = 2;
  string message = 3;
}

// ─────────────────────────────────────────────────────────────
// Edge Service
// ─────────────────────────────────────────────────────────────

service EdgeService {
  rpc CreateEdge(CreateEdgeRequest) returns (CreateEdgeResponse);
  rpc DeleteEdge(DeleteEdgeRequest) returns (DeleteEdgeResponse);
  rpc ListEdges(ListEdgesRequest) returns (stream EdgeResult);
  rpc GetEdge(GetEdgeRequest) returns (GetEdgeResponse);
}

message CreateEdgeRequest {
  RequestContext context = 1;
  NodeRef source = 2;
  NodeRef target = 3;
  string edge_type = 4;
  map<string, Value> properties = 5;
}

message CreateEdgeResponse {
  EdgeRef edge = 1;
  optional bytes pair_id = 2;         // For bidirectional edges
}

message DeleteEdgeRequest {
  RequestContext context = 1;
  bytes edge_id = 2;
}

message DeleteEdgeResponse {
  bool success = 1;
  bool pair_deleted = 2;              // True if bidirectional pair deleted
}

message ListEdgesRequest {
  RequestContext context = 1;
  string entity_type = 2;
  bytes node_id = 3;
  optional string edge_type = 4;      // Filter by type
  EdgeDirection direction = 5;
  uint32 limit = 6;
  bytes cursor = 7;
}

message EdgeResult {
  EdgeRef edge = 1;
  map<string, Value> properties = 2;
  bytes cursor = 3;                   // For pagination
}

message GetEdgeRequest {
  RequestContext context = 1;
  bytes edge_id = 2;
}

message GetEdgeResponse {
  EdgeRef edge = 1;
  map<string, Value> properties = 2;
  EdgeMetadata metadata = 3;
}

message EdgeMetadata {
  string owner = 1;
  int64 created_at = 2;
  optional bytes pair_id = 3;
}

// ─────────────────────────────────────────────────────────────
// Query Service
// ─────────────────────────────────────────────────────────────

service QueryService {
  rpc FindNodes(FindNodesRequest) returns (stream NodeResult);
  rpc Traverse(TraverseRequest) returns (stream TraverseResult);
  rpc Explain(ExplainRequest) returns (ExplainResponse);
}

message FindNodesRequest {
  RequestContext context = 1;
  string entity_type = 2;
  string cel_expression = 3;          // CEL predicate
  repeated string fields = 4;         // Field projection
  uint32 limit = 5;
  bytes cursor = 6;
  ReadConsistency consistency = 7;
}

message NodeResult {
  NodeRef node = 1;
  map<string, Value> properties = 2;
  bytes cursor = 3;
}

message TraverseRequest {
  RequestContext context = 1;
  oneof start {
    NodeRef start_node = 2;
    FindNodesRequest start_query = 3;
  }
  repeated TraversalStep steps = 4;
  uint32 max_depth = 5;
  uint32 timeout_ms = 6;
  uint32 max_results = 7;
  bool cascade = 8;
  RecurseConfig recurse = 9;
  ReadConsistency consistency = 10;
}

message TraversalStep {
  string edge_type = 1;
  EdgeDirection direction = 2;
  string edge_filter = 3;             // CEL on edge properties
  string node_filter = 4;             // CEL on target node
  repeated string fields = 5;
  uint32 per_node_limit = 6;
  repeated string edge_fields = 7;    // Edge property projection
  SortSpec sort = 8;
}

message SortSpec {
  string field = 1;
  bool descending = 2;
  bool on_edge = 3;                   // Sort by edge vs node property
}

message RecurseConfig {
  uint32 max_depth = 1;
  bool detect_cycles = 2;
}

message TraverseResult {
  int32 depth = 1;
  repeated NodeRef path = 2;
  NodeRef node = 3;
  map<string, Value> properties = 4;
  EdgeRef edge = 5;
  map<string, Value> edge_facets = 6;
  bytes cursor = 7;
}

message ExplainRequest {
  RequestContext context = 1;
  string entity_type = 2;
  string cel_expression = 3;
}

message ExplainResponse {
  QueryPlan plan = 1;
  repeated string explanation_steps = 2;
  double estimated_cost = 3;
  double estimated_rows = 4;
}

message QueryPlan {
  string strategy = 1;                // point_lookup, range_scan, full_scan
  repeated IndexOperation index_ops = 2;
  string residual_filter = 3;
}

message IndexOperation {
  string field = 1;
  string operator = 2;
  string value = 3;
  double estimated_rows = 4;
}

// ─────────────────────────────────────────────────────────────
// Admin Service
// ─────────────────────────────────────────────────────────────

service AdminService {
  rpc CreateIndex(CreateIndexRequest) returns (CreateIndexResponse);
  rpc DropIndex(DropIndexRequest) returns (DropIndexResponse);
  rpc ListIndexes(ListIndexesRequest) returns (ListIndexesResponse);
  rpc StripProperty(StripPropertyRequest) returns (StripPropertyResponse);
  rpc GetJobStatus(GetJobStatusRequest) returns (GetJobStatusResponse);
  rpc ListJobs(ListJobsRequest) returns (ListJobsResponse);
  rpc DropEntityType(DropEntityTypeRequest) returns (DropEntityTypeResponse);
  rpc DropNamespace(DropNamespaceRequest) returns (DropNamespaceResponse);
}

message CreateIndexRequest {
  RequestContext context = 1;
  string entity_type = 2;
  string property_name = 3;
  string index_type = 4;              // unique, equality, range
}

message CreateIndexResponse {
  string job_id = 1;                  // Backfill job
}

message DropIndexRequest {
  RequestContext context = 1;
  string entity_type = 2;
  string property_name = 3;
}

message DropIndexResponse {
  bool success = 1;
  int64 entries_deleted = 2;
}

message ListIndexesRequest {
  RequestContext context = 1;
  string entity_type = 2;
}

message ListIndexesResponse {
  repeated IndexInfo indexes = 1;
}

message IndexInfo {
  string property_name = 1;
  string index_type = 2;
  int64 entry_count = 3;
}

message StripPropertyRequest {
  RequestContext context = 1;
  string entity_type = 2;
  string property_name = 3;
}

message StripPropertyResponse {
  string job_id = 1;
}

message GetJobStatusRequest {
  RequestContext context = 1;
  string job_id = 2;
}

message GetJobStatusResponse {
  string job_id = 1;
  string job_type = 2;
  string status = 3;                  // pending, running, completed, failed
  int64 total_items = 4;
  int64 processed_items = 5;
  optional string error = 6;
}

message ListJobsRequest {
  RequestContext context = 1;
  optional string status_filter = 2;
}

message ListJobsResponse {
  repeated JobSummary jobs = 1;
}

message JobSummary {
  string job_id = 1;
  string job_type = 2;
  string status = 3;
  int64 created_at = 4;
}

message DropEntityTypeRequest {
  RequestContext context = 1;
  string entity_type = 2;
}

message DropEntityTypeResponse {
  bool success = 1;
  int64 nodes_deleted = 2;
  int64 edges_deleted = 3;
  string orphan_cleanup_job_id = 4;
}

message DropNamespaceRequest {
  RequestContext context = 1;
}

message DropNamespaceResponse {
  bool success = 1;
}

// ─────────────────────────────────────────────────────────────
// Error Details
// ─────────────────────────────────────────────────────────────

message ErrorDetail {
  string error_code = 1;
  string message = 2;
  string category = 3;
  map<string, string> metadata = 4;
  repeated FieldError field_errors = 5;
}

message FieldError {
  string field = 1;
  string error_code = 2;
  string message = 3;
}
```

### 10.2 Error Detail Protocol

Errors include structured `ErrorDetail` in `tonic::Status::details()`:

```rust
let status = Status::with_details(
    Code::InvalidArgument,
    "Schema validation failed",
    Bytes::from(ErrorDetail::encode_to_vec(&error_detail)),
);
```

---

## 11. Error Code Registry

### 11.1 Error Categories

| Category | Description |
|----------|-------------|
| `VALIDATION` | Input validation failures |
| `REFERENTIAL` | Referential integrity violations |
| `OWNERSHIP` | Locality/ownership violations |
| `QUERY` | Query compilation/execution errors |
| `SYSTEM` | System-level errors |

### 11.2 Complete Error Codes

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PelagoErrorCode {
    // ─── Validation (V) ───
    SchemaNotFound,
    SchemaValidation,
    SchemaVersionConflict,
    InvalidEntityType,
    InvalidPropertyType,
    RequiredFieldMissing,
    ExtraPropertyRejected,
    InvalidNodeId,
    InvalidEdgeId,
    InvalidCursor,
    InvalidLimit,

    // ─── Referential (R) ───
    NodeNotFound,
    NodeAlreadyExists,
    EdgeNotFound,
    EdgeAlreadyExists,
    TargetNodeNotFound,
    SourceNodeNotFound,
    EdgeTypeNotDeclared,
    TargetTypeMismatch,
    UniqueConstraintViolation,

    // ─── Ownership (O) ───
    LocalityViolation,
    NotNodeOwner,
    NotEdgeOwner,
    WriteLocked,

    // ─── Query (Q) ───
    CelSyntaxError,
    CelTypeError,
    CelEvaluationError,
    QueryTimeout,
    TraversalDepthExceeded,
    ResultLimitExceeded,
    InvalidProjection,

    // ─── System (S) ───
    FdbUnavailable,
    FdbTransactionConflict,
    FdbTransactionTimeout,
    JobNotFound,
    JobFailed,
    Internal,
    NotImplemented,
}

impl PelagoErrorCode {
    pub fn grpc_code(&self) -> tonic::Code {
        use PelagoErrorCode::*;
        match self {
            // NOT_FOUND
            SchemaNotFound | NodeNotFound | EdgeNotFound |
            TargetNodeNotFound | SourceNodeNotFound | JobNotFound => Code::NotFound,

            // INVALID_ARGUMENT
            SchemaValidation | InvalidEntityType | InvalidPropertyType |
            RequiredFieldMissing | ExtraPropertyRejected | InvalidNodeId |
            InvalidEdgeId | InvalidCursor | InvalidLimit | EdgeTypeNotDeclared |
            TargetTypeMismatch | CelSyntaxError | CelTypeError |
            InvalidProjection => Code::InvalidArgument,

            // ALREADY_EXISTS
            NodeAlreadyExists | EdgeAlreadyExists |
            UniqueConstraintViolation => Code::AlreadyExists,

            // FAILED_PRECONDITION
            SchemaVersionConflict => Code::FailedPrecondition,

            // PERMISSION_DENIED
            LocalityViolation | NotNodeOwner | NotEdgeOwner |
            WriteLocked => Code::PermissionDenied,

            // DEADLINE_EXCEEDED
            QueryTimeout | FdbTransactionTimeout => Code::DeadlineExceeded,

            // RESOURCE_EXHAUSTED
            TraversalDepthExceeded | ResultLimitExceeded => Code::ResourceExhausted,

            // ABORTED (retryable)
            FdbTransactionConflict => Code::Aborted,

            // UNAVAILABLE
            FdbUnavailable => Code::Unavailable,

            // UNIMPLEMENTED
            NotImplemented => Code::Unimplemented,

            // INTERNAL
            CelEvaluationError | JobFailed | Internal => Code::Internal,
        }
    }

    pub fn is_retryable(&self) -> bool {
        matches!(self,
            Self::FdbTransactionConflict |
            Self::FdbUnavailable |
            Self::QueryTimeout
        )
    }

    pub fn category(&self) -> &'static str {
        use PelagoErrorCode::*;
        match self {
            SchemaNotFound | SchemaValidation | SchemaVersionConflict |
            InvalidEntityType | InvalidPropertyType | RequiredFieldMissing |
            ExtraPropertyRejected | InvalidNodeId | InvalidEdgeId |
            InvalidCursor | InvalidLimit => "VALIDATION",

            NodeNotFound | NodeAlreadyExists | EdgeNotFound |
            EdgeAlreadyExists | TargetNodeNotFound | SourceNodeNotFound |
            EdgeTypeNotDeclared | TargetTypeMismatch |
            UniqueConstraintViolation => "REFERENTIAL",

            LocalityViolation | NotNodeOwner | NotEdgeOwner |
            WriteLocked => "OWNERSHIP",

            CelSyntaxError | CelTypeError | CelEvaluationError |
            QueryTimeout | TraversalDepthExceeded | ResultLimitExceeded |
            InvalidProjection => "QUERY",

            _ => "SYSTEM",
        }
    }
}
```

---

## 12. CDC System

### 12.1 CDC Entry Schema

```rust
pub struct CdcEntry {
    // Key: /_cdc/(<versionstamp>) — server-assigned
    pub site: String,
    pub timestamp: i64,
    pub batch_id: Option<String>,
    pub operations: Vec<CdcOperation>,
}

pub enum CdcOperation {
    NodeCreate {
        entity_type: String,
        node_id: NodeId,
        properties: HashMap<String, Value>,
    },
    NodeUpdate {
        entity_type: String,
        node_id: NodeId,
        changed_properties: HashMap<String, Value>,
        previous_version: u64,
    },
    NodeDelete {
        entity_type: String,
        node_id: NodeId,
    },
    EdgeCreate {
        source: NodeRef,
        target: NodeRef,
        edge_type: String,
        edge_id: EdgeId,
        properties: HashMap<String, Value>,
        pair_id: Option<String>,
    },
    EdgeDelete {
        edge_id: EdgeId,
        pair_id: Option<String>,
    },
    SchemaRegister {
        entity_type: String,
        version: u32,
    },
}
```

### 12.2 Versionstamp Ordering

CDC uses FDB versionstamps:
- **Zero contention**: Server-assigned at commit
- **Total order**: Globally ordered across all transactions
- **Consumer pattern**: Range scan from high-water mark

```rust
// CDC consumer loop
async fn consume_cdc(db: &FdbDatabase) -> Result<()> {
    let mut hwm = load_high_water_mark().await?;

    loop {
        let entries = db.read_range(
            cdc_subspace.range_from(hwm),
            RangeOption { limit: 1000, .. }
        ).await?;

        for entry in entries {
            process_cdc_entry(entry)?;
            hwm = entry.versionstamp;
        }

        save_high_water_mark(hwm).await?;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
```

### 12.3 CDC Retention

Phase 1: CDC entries accumulate. No automatic truncation.
Phase 2: Configurable retention with truncation job (default: 7 days).

---

## 13. Background Jobs

### 13.1 Job Types

```rust
pub enum JobType {
    // Phase 1
    IndexBackfill {
        entity_type: String,
        field: String,
        index_type: IndexType,
    },
    StripProperty {
        entity_type: String,
        property: String,
    },

    // Phase 2
    OrphanedEdgeCleanup {
        dropped_entity_type: String,
    },
    StatisticsRefresh {
        entity_type: String,
    },
    CdcRetention {
        retention_days: u32,
    },
}
```

### 13.2 Job State

```rust
pub struct BackgroundJob {
    pub job_id: String,
    pub job_type: JobType,
    pub status: JobStatus,
    pub progress_cursor: Vec<u8>,
    pub total_items: Option<i64>,
    pub processed_items: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub error: Option<String>,
}

pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
}
```

### 13.3 Worker Loop

```rust
async fn job_worker_loop(db: &FdbDatabase) {
    loop {
        let job = find_next_job(db).await;
        if let Some(job) = job {
            let cursor = job.progress_cursor.clone();

            loop {
                let batch = read_batch_from(cursor, BATCH_SIZE).await;
                if batch.is_empty() {
                    mark_job_completed(job.job_id).await;
                    break;
                }

                process_batch(&batch).await;
                advance_cursor(&job.job_id, &batch.last_key()).await;
            }
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
```

**Crash recovery:** On startup, scan `/_jobs/` for `Running` status, resume from cursor.

---

## 14. Configuration

### 14.1 CLI Structure

The `pelago` binary serves as both server and client tool:

```
pelago
├── serve                    # Start the gRPC server
├── version                  # Show version info
└── (future: repl, schema, node, edge commands)
```

### 14.2 Server Configuration

**IMPORTANT:** Site ID must always be provided via config or CLI.

```rust
#[derive(Parser)]
#[command(name = "pelago", about = "PelagoDB graph database")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the PelagoDB server
    Serve(ServeArgs),
    /// Show version information
    Version,
}

#[derive(Args)]
struct ServeArgs {
    /// FDB cluster file path (required)
    #[arg(long, env = "PELAGO_FDB_CLUSTER")]
    fdb_cluster: String,

    /// Site ID for this server instance (required, 0-255)
    #[arg(long, env = "PELAGO_SITE_ID")]
    site_id: u8,

    /// gRPC listen address
    #[arg(long, default_value = "[::1]:27615", env = "PELAGO_LISTEN_ADDR")]
    listen_addr: String,

    /// Log level filter
    #[arg(long, default_value = "info", env = "RUST_LOG")]
    log_level: String,
}
```

**Usage:**
```bash
# Start server
pelago serve --fdb-cluster /etc/foundationdb/fdb.cluster --site-id 0

# Or with environment variables
PELAGO_FDB_CLUSTER=/etc/foundationdb/fdb.cluster PELAGO_SITE_ID=0 pelago serve
```

**Validation:** Server refuses to start without `site_id` and `fdb_cluster`.

### 14.3 Site ID

Site ID is a u8 (0-255) identifying the physical deployment:
- Embedded in all NodeId/EdgeId generated by this instance
- Used for ownership determination
- Extractable via `node_id.home_site()`

Phase 1 is single-site (site_id=0 acceptable). Multi-site coordination is Phase 3.

### 14.4 Authentication

**Research step needed.** Phase 1 has no authentication. All requests are trusted. Auth (API keys, mTLS, gRPC interceptor) is Phase 3 scope.

---

## 15. Phase 1 Implementation Milestones

### M0: Project Scaffolding
- Workspace with all crates defined
- `pelago-core` types: NodeId, EdgeId, Value, PropertyType
- Encoding layer: sort-order encoding, CBOR helpers, tuple encoding
- FDB connection and basic transaction helpers
- ServerConfig with clap CLI
- PelagoError enum with all error codes

### M1: Schema Registry
- EntitySchema, PropertyDef, EdgeDef structures
- Schema validation logic
- FDB schema storage in `/_schema/`
- In-memory schema cache
- Forward reference handling

### M2: Node CRUD + Indexes
- ID allocator with batch allocation
- Node create with validation and index maintenance
- Point lookup
- Node update with index diff
- Node delete with cascade
- All index types: unique, equality, range
- Null handling (not indexed)
- CDC entry on every mutation

### M3: Edge CRUD
- Edge create with referential integrity
- Bidirectional edge pairs
- Sort key in edge keys
- Edge delete (single and paired)
- Edge listing with streaming
- Polymorphic edges

### M4: Query System
- CEL integration with type environment
- Predicate extraction
- Index matching
- Query plan generation
- Snapshot reads
- FindNodes execution with streaming
- Cursor pagination
- Explain endpoint

### M5: Traversal Engine
- TraverseRequest handling
- Multi-hop execution
- Edge filter + node filter per step
- @cascade support
- @recurse support
- Server-enforced limits
- Streaming results

### M6: gRPC API + Background Jobs
- All service handlers wired to tonic
- Background job framework
- IndexBackfill job
- StripProperty job
- DropIndex (synchronous)
- DropEntityType, DropNamespace
- `pelago serve` command with graceful shutdown

### M7: Integration Testing
- Rust integration tests against FDB
- Full workflow tests
- Concurrent operation tests
- Error condition tests

---

## 16. Deferred to Phase 2+

### Phase 2: Query Optimization + Caching
- Multi-predicate index intersection/union
- Cost-based query planning with statistics
- RocksDB cache layer
- Read coalescing
- Query plan caching

### Phase 3: Multi-Site + Auth
- CDC streaming between sites
- Ownership transfer protocol
- Causal ordering and dependency buffer
- API keys and gRPC interceptor
- Watch/reactive subscriptions

### Phase 4: Operational Maturity
- Schema migration tooling
- Observability and metrics (OTel export)
- Backup/restore integration
- Resource limits and quotas

---

## Appendix A: Key Layout Reference

```
/pelago/
│
├── _sys/
│   ├── (config)                                    → System config
│   ├── (dc, <dc>)                                  → DC registry
│   ├── (dc_topo, <dc>, <dc>)                       → DC topology
│   └── (repl, <lifecycle>)                         → Replication sets
│
├── _auth/
│   ├── (scope, <name>)                             → Permission bundles
│   ├── (user, <user_id>)                           → User permissions
│   └── (cache, <user>, <db>, <ns>)                 → Auth cache
│
└── <db>/
    ├── _db/
    │   ├── (config)                                → DB config
    │   ├── (lock)                                  → Write lock
    │   ├── (hist, <ts>)                            → History
    │   └── (ns, <ns>)                              → Namespace registry
    │
    └── <ns>/
        ├── _ns/
        │   ├── (config)                            → NS config
        │   ├── (lock)                              → Write lock
        │   └── (stats)                             → Statistics
        │
        ├── _types/
        │   └── (<type>)                            → Type registry
        │
        ├── _schema/
        │   ├── (<type>, v, <ver>)                  → Schema definition
        │   └── (<type>, latest)                    → Current version
        │
        ├── _cdc/
        │   └── (<versionstamp>)                    → CDC entry
        │
        ├── _jobs/
        │   └── (<job_type>, <job_id>)              → Job state
        │
        ├── _ids/
        │   └── (<entity_type>, <site_id>)          → ID counter
        │
        ├── data/
        │   └── (<type>, <id>)                      → Entity record
        │
        ├── loc/
        │   └── (<type>, <id>)                      → Locality (3-char)
        │
        ├── idx/
        │   ├── (tomb, <type>, <id>)                → Tombstone index
        │   ├── (byloc, <dc>, <type>, <id>)         → By-locality
        │   ├── (byschema, <type>, <ver>, <id>)     → By-schema
        │   ├── (byupd, <type>, <versionstamp>)     → By-update (→ id)
        │   ├── (prop, <type>, <field>, unique, <value>)      → Unique property
        │   ├── (prop, <type>, <field>, eq, <value>, <id>)    → Equality property
        │   └── (prop, <type>, <field>, range, <value>, <id>) → Range property
        │
        ├── edge/
        │   ├── (f, <src_t>, <src_id>, <lbl>, <sort_key>, <tgt_db>, <tgt_ns>, <tgt_t>, <tgt_id>, <edge_id>)
        │   └── (r, <tgt_db>, <tgt_ns>, <tgt_t>, <tgt_id>, <lbl>, <sort_key>, <src_t>, <src_id>, <edge_id>)
        │
        └── xref/
            ├── (out, <tgt_db>)                     → Outgoing ref count
            └── (in, <src_db>, <src_ns>)            → Incoming ref count
```

---

## Appendix B: Value Encoding Reference

### Key Encoding (FDB Tuple Layer)

| Type | Encoding | Notes |
|------|----------|-------|
| String | Null-terminated UTF-8 with escape | FDB tuple standard |
| Int (i64) | Big-endian with sign flip | `v ^ (1 << 63)` for correct ordering |
| Float (f64) | IEEE 754 order-preserving | Sign bit flip + conditional invert |
| Bytes | Length-prefixed | FDB tuple standard |
| Timestamp | Big-endian i64 (micros) | Same as Int |
| Versionstamp | 10 bytes | FDB server-assigned |
| NodeId | 9 bytes | `[site:u8][seq:u64 BE]` |

### Value Encoding (CBOR)

All values use CBOR via `ciborium` crate:
- Compact binary format
- Schema-flexible
- Fast encode/decode
- Serde integration

---

*This specification is the authoritative reference for PelagoDB Phase 1 implementation.*
