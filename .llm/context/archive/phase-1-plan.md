# PelagoDB: Phase 1 Implementation Plan
## Storage Foundation + Working API

**Status:** Plan — pending implementation
**Parent spec:** graph-db-spec-v0.3.md
**Scope:** Phase 1 from spec Section 13, minus RocksDB cache layer
**Date:** 2026-02-10

---

## Decisions Made

| Decision | Choice | Rationale |
|---|---|---|
| Node/Edge ID format | `[site_id: u8][counter: u64]` = 9 bytes | Compact, no cross-site coordination, origin site extractable from ID |
| Project structure | Workspace multi-crate | Clean boundaries, parallel compilation |
| FDB dev environment | Local install + Docker for CI | Native perf for dev, reproducible CI |
| Key hierarchy | `(datastore, namespace, ...)` | Datastore = logical DB / isolation. Namespace = entity grouping within. |
| Schema scope | Per-namespace | Each `(datastore, namespace)` has independent schema registry |
| Site ID | Server startup config | Env var or config file. All IDs from this instance use it. |
| Tenant prefix | Replaced by "datastore" | Same concept, better name for the domain |
| RocksDB cache | Excluded from Phase 1 | All reads go direct to FDB. Cache layer added in Phase 2. |

## Crate Layout

```
pelago/
├── Cargo.toml                 # workspace root
├── Dockerfile                 # multi-stage: builder + slim runtime with FDB client libs
├── docker-compose.yml         # FDB + 2 pelago sites + test runner
├── proto/
│   └── pelago.proto           # gRPC service + message definitions
├── examples/
│   └── multi_site_demo.py     # full working demo: 2 sites, schema, nodes, edges, queries
├── crates/
│   ├── pelago-proto/          # generated protobuf/tonic code
│   │   ├── Cargo.toml
│   │   ├── build.rs
│   │   └── src/lib.rs
│   ├── pelago-core/           # shared types, encoding, errors
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── types.rs       # NodeId, EdgeId, Value, PropertyType, etc.
│   │       ├── encoding.rs    # tuple encoding, CBOR helpers, sort-order encoding
│   │       ├── schema.rs      # EntitySchema, PropertyDef, EdgeDef, IndexType
│   │       ├── errors.rs      # PelagoError enum (thiserror)
│   │       └── config.rs      # ServerConfig (site_id, fdb_cluster_file, etc.)
│   ├── pelago-storage/        # FDB interactions, subspace layout
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── subspace.rs    # Subspace builder: datastore/namespace/entities/...
│   │       ├── schema.rs      # Schema CRUD in FDB _meta subspace
│   │       ├── node.rs        # Node CRUD + index maintenance
│   │       ├── edge.rs        # Edge CRUD, bidirectional pairs
│   │       ├── index.rs       # Index read/write/delete operations
│   │       ├── cdc.rs         # CDC entry write (in mutation txn) + read
│   │       ├── ids.rs         # ID allocator (atomic counter per entity type per site)
│   │       └── jobs.rs        # Background job framework (_jobs subspace)
│   ├── pelago-query/          # CEL compilation, query planning
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── cel.rs         # CEL parse, type-check, null semantics
│   │       ├── planner.rs     # Predicate extraction, index matching, plan selection
│   │       ├── executor.rs    # FindNodes execution (scan + filter + stream)
│   │       └── plan.rs        # QueryPlan enum, ExecutionPlan struct
│   ├── pelago-api/            # gRPC service handlers
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── schema_service.rs
│   │       ├── node_service.rs
│   │       ├── edge_service.rs
│   │       ├── query_service.rs
│   │       └── admin_service.rs   # DropIndex, StripProperty, Explain, job status
│   └── pelago-server/         # binary entrypoint
│       ├── Cargo.toml
│       └── src/
│           └── main.rs        # config loading, FDB init, tonic server startup
└── tests/
    ├── integration/           # Rust tests against real FDB (storage layer)
    │   ├── schema_tests.rs
    │   ├── node_tests.rs
    │   ├── edge_tests.rs
    │   ├── query_tests.rs
    │   └── cdc_tests.rs
    └── python/                # Python gRPC test client (API contract)
        ├── requirements.txt
        ├── generate_stubs.sh
        ├── conftest.py
        ├── test_schema.py
        ├── test_nodes.py
        ├── test_edges.py
        ├── test_queries.py
        ├── test_admin.py
        └── test_lifecycle.py
```

## Tech Stack + Key Dependencies

**Do not pin versions in Cargo.toml.** Use `cargo add <crate>` to pull latest at implementation time. The crates below are the selection, not the versions.

| Category | Crates | Notes |
|---|---|---|
| Async runtime | `tokio` (full features) | |
| Core | `thiserror`, `serde` (derive), `ciborium` | ciborium = serde_cbor successor |
| FDB | `fdb` | Tokio-native, FDB 7.1, verify versionstamp + tuple layer support |
| gRPC | `tonic`, `prost`, `tonic-build` (build.rs) | |
| Observability | `tracing`, `tracing-subscriber` | OTel export deferred to Phase 4 |
| CLI | `clap` (derive feature) | |
| CEL | `cel-rust` or `cel-interpreter` | Evaluate both — need AST access + custom type envs |
| IDs | `uuid` (v7 feature) | Fallback if site-prefixed int64 needs a UUID variant |
| Test client | `grpcio-tools` (Python, pip) | Generate Python stubs from proto for testing |

### Logging / Tracing Strategy

Use `tracing` throughout — not `log`. Every significant operation gets a span:

- **API layer:** span per gRPC request with `datastore`, `namespace`, `entity_type`, request ID
- **Storage layer:** span per FDB transaction with operation type
- **Query layer:** span per query plan compilation + execution
- **Background jobs:** span per job + per batch

Subscriber setup in `pelago-server/main.rs`: `tracing_subscriber::fmt` with `EnvFilter` for dev, JSON output for production. OTel export deferred to Phase 4.

### CLI Structure

`pelago-server` binary uses clap derive:

```rust
#[derive(Parser)]
#[command(name = "pelago", about = "PelagoDB graph database server")]
struct Cli {
    /// FDB cluster file path
    #[arg(long, env = "PELAGO_FDB_CLUSTER")]
    fdb_cluster: String,

    /// Site ID for this server instance (0-255)
    #[arg(long, env = "PELAGO_SITE_ID")]
    site_id: u8,

    /// gRPC listen address
    #[arg(long, default_value = "[::1]:50051", env = "PELAGO_LISTEN_ADDR")]
    listen_addr: String,

    /// Log level filter (e.g., "info", "pelago=debug,fdb=warn")
    #[arg(long, default_value = "info", env = "RUST_LOG")]
    log_level: String,
}
```

All config values support both CLI flags and env vars. CLI flags take precedence.

> **Risk:** `cel-rust` crate maturity. If it's insufficient, alternatives: `cel-interpreter` crate, or hand-roll a minimal predicate engine for Phase 1 and swap in a full CEL impl later.

---

## Milestone 0: Project Scaffolding

**Goal:** Compilable workspace, FDB connection, basic types, encoding layer.

### Deliverables

1. **Workspace Cargo.toml** with all crates defined
2. **`pelago-core` types:**
   - `NodeId { site: u8, seq: u64 }` — 9-byte encoding, `Display`/`FromStr` impl
   - `EdgeId { site: u8, seq: u64 }` — same format
   - `Value` enum: `String(String)`, `Int(i64)`, `Float(f64)`, `Bool(bool)`, `Timestamp(i64)`, `Bytes(Vec<u8>)`, `Null`
   - `PropertyType` enum matching the spec's type table
   - `DatastoreId(String)`, `NamespaceId(String)` — validated newtype wrappers
3. **Encoding layer (`pelago-core::encoding`):**
   - Sort-order encoding for index keys: big-endian i64 with sign flip, IEEE 754 order-preserving floats, null-terminated strings
   - CBOR helpers: serialize/deserialize `HashMap<String, Value>` via ciborium
   - Tuple encoding: wrapper over FDB tuple layer for building subspace keys
4. **`pelago-storage` FDB connection:**
   - `FdbCluster` / `FdbDatabase` initialization from cluster file path
   - Connection pool or shared database handle
   - Basic transaction helpers (read, write, range scan)
5. **`pelago-core::config`:**
   - `ServerConfig { site_id: u8, fdb_cluster_file: String, listen_addr: String, ... }`
   - Load from env vars or TOML config file
6. **`pelago-core::errors`:**
   - `PelagoError` enum with variants: `SchemaNotFound`, `SchemaValidation`, `NodeNotFound`, `EdgeNotFound`, `UniqueConstraintViolation`, `TypeMismatch`, `CelCompilation`, `FdbError`, `Internal`, etc.
7. **`pelago-proto`:**
   - Proto file with Phase 1 service and message definitions (see API section below)
   - `build.rs` with tonic-build

### Proto Definitions (Phase 1 Scope)

```protobuf
syntax = "proto3";
package pelago.v1;

// ─── Schema Service ───
service SchemaService {
  rpc RegisterSchema(RegisterSchemaRequest) returns (RegisterSchemaResponse);
  rpc GetSchema(GetSchemaRequest) returns (GetSchemaResponse);
  rpc ListSchemas(ListSchemasRequest) returns (ListSchemasResponse);
}

// ─── Node Service ───
service NodeService {
  rpc CreateNode(CreateNodeRequest) returns (CreateNodeResponse);
  rpc GetNode(GetNodeRequest) returns (GetNodeResponse);
  rpc UpdateNode(UpdateNodeRequest) returns (UpdateNodeResponse);
  rpc DeleteNode(DeleteNodeRequest) returns (DeleteNodeResponse);
}

// ─── Edge Service ───
service EdgeService {
  rpc CreateEdge(CreateEdgeRequest) returns (CreateEdgeResponse);
  rpc DeleteEdge(DeleteEdgeRequest) returns (DeleteEdgeResponse);
  rpc ListEdges(ListEdgesRequest) returns (stream EdgeResult);
}

// ─── Query Service ───
service QueryService {
  rpc FindNodes(FindNodesRequest) returns (stream NodeResult);
  rpc Explain(ExplainRequest) returns (ExplainResponse);
}

// ─── Admin Service ───
service AdminService {
  rpc DropIndex(DropIndexRequest) returns (DropIndexResponse);
  rpc StripProperty(StripPropertyRequest) returns (StripPropertyResponse);
  rpc GetJobStatus(GetJobStatusRequest) returns (GetJobStatusResponse);
  rpc DropEntityType(DropEntityTypeRequest) returns (DropEntityTypeResponse);
  rpc DropNamespace(DropNamespaceRequest) returns (DropNamespaceResponse);
}
```

### Exit Criteria
- `cargo build` succeeds across all crates
- `NodeId` round-trips through FDB tuple encoding
- `Value` round-trips through CBOR
- FDB connection established and a test key written/read

---

## Milestone 1: Schema Registry

**Goal:** Register, store, retrieve, and validate entity schemas. Foundation for all data operations.

**Depends on:** M0

### Deliverables

1. **Schema data structures (`pelago-core::schema`):**
   ```rust
   struct EntitySchema {
       datastore: DatastoreId,
       namespace: NamespaceId,
       name: String,              // entity type name, e.g., "Person"
       version: u32,
       properties: HashMap<String, PropertyDef>,
       edges: HashMap<String, EdgeDef>,
       meta: SchemaMeta,
   }

   struct PropertyDef {
       property_type: PropertyType,
       required: bool,
       index: Option<IndexType>,
       default: Option<Value>,
   }

   enum IndexType { Unique, Equality, Range }

   struct EdgeDef {
       target: EdgeTarget,        // Specific("Company") or Polymorphic
       direction: EdgeDirection,   // Outgoing, Bidirectional
       properties: HashMap<String, PropertyDef>,
       sort_key: Option<String>,
       ownership: OwnershipMode,  // SourceSite (default), Independent
   }

   struct SchemaMeta {
       allow_undeclared_edges: bool,
       extras_policy: ExtrasPolicy,  // Reject, Allow, Warn
   }
   ```

2. **Schema validation logic:**
   - Property type validation (all types in spec table)
   - Required field enforcement
   - Edge target validation (forward references allowed, stored as unresolved)
   - `extras_policy` enforcement
   - Version must increment (or be 1 for new types)

3. **FDB schema storage (`pelago-storage::schema`):**
   - Write current schema: `(ds, ns, _meta, schemas, entity_type)` → CBOR
   - Write version history: `(ds, ns, _meta, schema_versions, entity_type, version)` → CBOR
   - Read current schema by entity type
   - List all schemas in a namespace
   - Schema evolution: detect added/removed properties, enqueue index backfill job if new indexed property

4. **In-memory schema cache:**
   - `HashMap<(DatastoreId, NamespaceId, String), Arc<EntitySchema>>` with version check
   - Cache invalidation on `RegisterSchema` (local invalidation; multi-instance invalidation deferred)
   - Cache miss → FDB read → populate cache

### FDB Key Layout

```
(ds, ns, _meta, schemas, "Person")
  → CBOR { version: 1, properties: {...}, edges: {...}, meta: {...} }

(ds, ns, _meta, schema_versions, "Person", 1)
  → CBOR { ... full schema at version 1 ... }
```

### Exit Criteria
- Register a schema, retrieve it, verify all fields
- Register with forward-reference edge target — succeeds
- Register with invalid property type — fails with `SchemaValidation` error
- Schema version history is queryable
- Re-register with new version (added property) — succeeds, old version preserved
- Re-register with removed property — succeeds, old schema version retained

---

## Milestone 2: Node CRUD + Indexes

**Goal:** Create, read, update, delete nodes with schema validation, index maintenance, and CDC entries in unified single-transaction write path.

**Depends on:** M1

### Deliverables

1. **ID Allocator (`pelago-storage::ids`):**
   - Atomic counter at `(ds, ns, _ids, entity_type, site_id)` → `u64`
   - Batch allocation: grab a block of N IDs (e.g., 100) per transaction to reduce contention
   - Returns `NodeId { site: config.site_id, seq: next_id }`

2. **Node Create:**
   - Validate properties against schema (types, required fields, extras_policy)
   - Apply defaults for missing optional properties with `default` set
   - Allocate ID
   - Single FDB transaction:
     - Write data: `(ds, ns, entities, type, data, node_id)` → CBOR properties
     - Write index entries for every indexed property (unique, equality, range)
     - Unique index: check-then-write (read index key first, conflict if exists)
     - Write CDC entry: `(ds, ns, _cdc, versionstamp)` → `CdcEntry { ops: [NodeCreate {...}] }`
   - Return created node ID

3. **Point Lookup (GetNode):**
   - Direct FDB `get()` on `(ds, ns, entities, type, data, node_id)`
   - Deserialize CBOR → property map
   - Return properties (minus internal fields like `_home_site` unless requested)

4. **Node Update:**
   - Read current node data
   - Validate changed properties against schema
   - Compute index diff: which index entries need to be removed (old values) and added (new values)
   - Single FDB transaction:
     - Write updated data
     - Delete old index entries, write new index entries
     - Write CDC entry (NodeUpdate with changed_properties)
   - Optimistic concurrency: use FDB read conflict ranges (if another txn modified the node, this txn conflicts and retries)

5. **Node Delete:**
   - Read current node data (need property values to delete index entries)
   - Scan incoming edges: `(ds, ns, entities, type, node_id, edges, *, IN, ...)` to find all incoming edges
   - Single FDB transaction:
     - Delete data key
     - Delete all index entries for this node
     - Delete all outgoing edge entries
     - For each incoming edge: delete the corresponding OUT entry at the source node
     - Write CDC entry (NodeDelete)

6. **Index Operations (`pelago-storage::index`):**
   - Write unique index: `(ds, ns, entities, type, idx, field, value)` → `node_id` bytes. Read-before-write to enforce uniqueness.
   - Write equality index: `(ds, ns, entities, type, idx, field, value, node_id)` → empty
   - Write range index: `(ds, ns, entities, type, idx, field, encoded_value, node_id)` → empty
   - Delete index entry (by exact key)
   - Range scan on index (for query execution — used in M4)
   - Null handling: no index entry written if value is null/missing

### FDB Key Layout (Node + Index)

```
# Node data
(ds, ns, entities, "Person", data, <node_id_bytes>)
  → CBOR { "name": "Andrew", "age": 38, "_home_site": "site_a" }

# Unique index (name)
(ds, ns, entities, "Person", idx, "name", "Andrew")
  → <node_id_bytes>

# Range index (age) — value is sort-order encoded
(ds, ns, entities, "Person", idx, "age", <be_i64(38)>, <node_id_bytes>)
  → (empty)

# CDC entry
(ds, ns, _cdc, <versionstamp>)
  → CBOR { site: "site_a", ops: [{ NodeCreate { ... } }] }
```

### Exit Criteria
- Create node → ID returned, data readable via GetNode
- Create node with missing required field → rejected
- Create node with extra field + `extras_policy: "reject"` → rejected
- Create two nodes with same unique-indexed value → second rejected
- Update node → old index entries gone, new ones present
- Delete node → data gone, all index entries gone, incoming edges cleaned up
- Every mutation has a corresponding CDC entry in `_cdc` subspace

---

## Milestone 3: Edge CRUD

**Goal:** Create, list, and delete typed edges with referential integrity, bidirectional pairs, sort keys, and edge properties.

**Depends on:** M2

### Deliverables

1. **Edge Create:**
   - Validate: source node exists, target node exists, both types registered
   - If edge type declared in source schema: validate target type matches declaration
   - If edge type undeclared: check `allow_undeclared_edges` flag
   - Validate edge properties against edge type's property schema
   - Allocate edge ID (same `[site_id:u8][counter:u64]` pattern)
   - Extract sort key value (if edge type has `sort_key` property)
   - Single FDB transaction:
     - Write OUT entry at source: `(ds, ns, entities, source_type, source_id, edges, edge_type, OUT, sort_key, target_type, target_id, edge_id)` → CBOR edge properties
     - Write IN entry at target: `(ds, ns, entities, target_type, target_id, edges, edge_type, IN, sort_key, source_type, source_id, edge_id)` → CBOR edge properties
     - For bidirectional: write reverse pair too (swap source/target), shared `pair_id`
     - Write CDC entry (EdgeCreate)

2. **Edge Delete:**
   - Delete OUT entry at source + IN entry at target
   - For bidirectional: delete both pairs (4 entries total)
   - Write CDC entry (EdgeDelete)

3. **Edge Listing (ListEdges):**
   - By source node + edge type + direction: range scan on edge subspace
   - Returns streaming results
   - Sort key in key position enables ordered traversal (e.g., edges sorted by timestamp)

4. **Polymorphic edges (`target: "*"`):**
   - Target type encoded in edge key — no special handling needed
   - Listing returns edges to any target type

### FDB Key Layout (Edges)

```
# Outgoing edge (Person → Company via WORKS_AT)
(ds, ns, entities, "Person", <p_42>, edges, "WORKS_AT", 0, <sort_key>, "Company", <c_7>, <e_101>)
  → CBOR { "role": "Engineer" }

# Incoming edge (at Company, reverse pointer)
(ds, ns, entities, "Company", <c_7>, edges, "WORKS_AT", 1, <sort_key>, "Person", <p_42>, <e_101>)
  → CBOR { "role": "Engineer" }

# Bidirectional edge (Person ←KNOWS→ Person)
# Pair 1: p_42 → p_99
(ds, ns, entities, "Person", <p_42>, edges, "KNOWS", 0, <sort_key>, "Person", <p_99>, <e_200>)
  → CBOR { "since": 1705363200, "pair_id": <pair_id> }

# Pair 2: p_99 → p_42
(ds, ns, entities, "Person", <p_99>, edges, "KNOWS", 0, <sort_key>, "Person", <p_42>, <e_201>)
  → CBOR { "since": 1705363200, "pair_id": <pair_id> }

# IN entries for both (omitted for brevity — mirror of above with direction=1)
```

### Exit Criteria
- Create edge between existing nodes → both OUT and IN entries written
- Create edge to non-existent node → rejected
- Create edge of undeclared type with `allow_undeclared_edges: false` → rejected
- Create bidirectional edge → 4 entries written (2 OUT + 2 IN)
- Delete bidirectional edge → all 4 entries removed
- List edges by type + direction → returns correct set, sorted by sort key
- Polymorphic edge to different target types → all listed correctly
- CDC entries for all edge mutations

---

## Milestone 4: CEL Query Pipeline

**Goal:** Parse CEL expressions, type-check against schemas, extract predicates, match to indexes, execute FindNodes with streaming results.

**Depends on:** M1 (schema), M2 (index read operations)

**Can start in parallel with M3.**

### Deliverables

1. **CEL Integration (`pelago-query::cel`):**
   - Build CEL type environment from `EntitySchema` properties
   - Parse CEL string → AST
   - Type-check AST against schema environment
   - Null semantics: comparisons on null fields return `false`, `field == null` returns `true`
   - Error on referencing removed/unknown properties

2. **Predicate Extraction (`pelago-query::planner`):**
   - Walk AST, extract conjuncts from top-level AND
   - Classify each predicate: field, operator, value
   - For Phase 1: single-predicate index matching only (no intersection/union)
   - Match predicate → available index: unique/equality → PointLookup, range → RangeScan
   - Unmatched predicates → residual filter

3. **Query Plan Generation (`pelago-query::plan`):**
   - `QueryPlan` enum: `PointLookup`, `RangeScan`, `FullScan`
   - `ExecutionPlan`: primary plan + compiled residual CEL + projection + limit + cursor
   - Plan selection: most selective indexed predicate wins (heuristic-based for Phase 1, no statistics)

4. **FindNodes Execution (`pelago-query::executor`):**
   - Execute primary plan (index scan or full scan)
   - For each candidate: load node data, apply residual filter
   - Stream results via tokio channel with backpressure
   - Pagination: cursor = last scanned index key, returned to client for continuation
   - Limit enforcement

5. **Explain Endpoint:**
   - Run pipeline stages 1-6 (parse → type-check → normalize → extract → match → plan)
   - Return `ExplainResponse` with plan details and human-readable explanation steps
   - No execution

### Phase 1 Query Limitations (Documented, Not Bugs)
- Single-predicate index selection only. Multi-predicate queries use best single index + residual.
- No index intersection or union (Phase 2).
- No cost-based selection — heuristic only (unique > range > equality > full scan).
- No traversal (Phase 2 scope — listed in spec but deferring multi-hop to keep Phase 1 focused).
- No cursor for complex queries (simple cursor on index position only).

### Exit Criteria
- `FindNodes("Person", "age >= 30")` → range scan on age index, returns matching nodes
- `FindNodes("Person", "email == 'a@b.com'")` → unique index point lookup
- `FindNodes("Person", "bio.contains('engineer')")` → full scan with residual filter
- `FindNodes("Person", "age >= 30 && name == 'Andrew'")` → best index (name unique) + residual (age)
- `FindNodes("Person", "salary > 0")` where salary not in schema → CEL type-check error
- `Explain` returns plan without executing
- Null field comparisons return false (node excluded from results)
- `FindNodes("Person", "age == null")` → full scan, returns nodes without age

---

## Milestone 5: gRPC API + Background Jobs

**Goal:** Wire everything into tonic service handlers. Implement background job framework for index backfill and property stripping.

**Depends on:** M2, M3, M4

### Deliverables

1. **gRPC Service Handlers:**
   - `SchemaService`: `RegisterSchema`, `GetSchema`, `ListSchemas`
   - `NodeService`: `CreateNode`, `GetNode`, `UpdateNode`, `DeleteNode`
   - `EdgeService`: `CreateEdge`, `DeleteEdge`, `ListEdges` (server-streaming)
   - `QueryService`: `FindNodes` (server-streaming), `Explain`
   - `AdminService`: `DropIndex`, `StripProperty`, `GetJobStatus`, `DropEntityType`, `DropNamespace`
   - All RPCs include `datastore` and `namespace` fields
   - Error mapping: `PelagoError` → gRPC status codes

2. **Background Job Framework (`pelago-storage::jobs`):**
   ```rust
   struct BackgroundJob {
       job_id: String,
       job_type: JobType,
       status: JobStatus,        // Pending, Running, Completed, Failed
       progress_cursor: Vec<u8>,
       created_at: i64,
       last_updated: i64,
       error: Option<String>,
       datastore: DatastoreId,
       namespace: NamespaceId,
   }
   ```
   - Job storage in `(ds, ns, _jobs, job_type, job_id)` → CBOR
   - Worker loop: find next pending/running job → process batch → advance cursor → repeat
   - Crash recovery: on startup, scan `_jobs` for `Running` status, resume from cursor

3. **Index Backfill Job:**
   - Triggered when `RegisterSchema` adds a new indexed property
   - Scans all nodes of entity type from cursor
   - For each node: extract property value, write index entry
   - Batch size configurable (e.g., 100 nodes per FDB transaction)
   - Idempotent: writing an index entry that already exists is a no-op

4. **StripProperty Job:**
   - Triggered by `StripProperty` RPC
   - Scans all nodes of entity type
   - For each: deserialize CBOR, remove property key, re-serialize, write back
   - Idempotent: stripping a property that's already absent is a no-op

5. **DropIndex (Synchronous):**
   - `clearRange` on `(ds, ns, entities, type, idx, field, ...)` — deletes all index entries for the field
   - Immediate, no background job

6. **DropEntityType:**
   - `clearRange` on `(ds, ns, entities, type, ...)` — all data, indexes, edges
   - Enqueue orphaned edge cleanup job for other entity types

7. **DropNamespace:**
   - `clearRange` on `(ds, ns, ...)` — everything

8. **Server Binary (`pelago-server`):**
   - Load config (site_id, FDB cluster file, listen address)
   - Initialize FDB connection
   - Start background job worker (tokio task)
   - Start tonic gRPC server with all services
   - Graceful shutdown

### Exit Criteria
- All RPCs callable via `grpcurl` or a test client
- `RegisterSchema` → `CreateNode` → `GetNode` → `FindNodes` works end-to-end over gRPC
- `StripProperty` enqueues job, job runs to completion, property removed from CBOR blobs
- `DropIndex` immediately clears index entries
- `DropNamespace` removes everything under the namespace
- Background worker resumes jobs after restart
- Server starts and serves on configured port

---

## Milestone 6: Integration Testing + Validation

**Goal:** End-to-end test suite at two levels: Rust integration tests against FDB internals, and a Python gRPC test client validating the API contract from the outside.

**Depends on:** All milestones

### Testing Layers

**Layer 1: Rust integration tests** (`tests/integration/`)
Direct FDB access. Validate storage-level correctness: key layout, atomicity, index entries, CDC entries. These tests use the storage/query crates directly without gRPC.

**Layer 2: Python gRPC test client** (`tests/python/`)
Generated from `pelago.proto` via `grpcio-tools`. Validates the full API contract over the wire. This is what a real client sees.

### Python Test Client Setup

```
tests/
└── python/
    ├── requirements.txt       # grpcio, grpcio-tools, pytest, pytest-asyncio
    ├── generate_stubs.sh      # python -m grpc_tools.protoc ... → pelago_pb2.py, pelago_pb2_grpc.py
    ├── conftest.py            # fixtures: server process, channel, stub clients
    ├── test_schema.py         # RegisterSchema, GetSchema, ListSchemas
    ├── test_nodes.py          # CreateNode, GetNode, UpdateNode, DeleteNode
    ├── test_edges.py          # CreateEdge, DeleteEdge, ListEdges
    ├── test_queries.py        # FindNodes (streaming), Explain
    ├── test_admin.py          # DropIndex, StripProperty, DropNamespace
    └── test_lifecycle.py      # Full workflow: schema → nodes → edges → query → cleanup
```

**Key fixtures (`conftest.py`):**
- `pelago_server`: starts `pelago-server` binary as subprocess, waits for port ready, tears down after test session
- `grpc_channel`: creates channel to running server
- `schema_stub`, `node_stub`, `edge_stub`, `query_stub`, `admin_stub`: typed service stubs
- `fresh_namespace`: creates a unique `(datastore, namespace)` per test, drops it in teardown

**Stub generation:** `generate_stubs.sh` runs `python -m grpc_tools.protoc` against `proto/pelago.proto`. Stubs committed to repo (or generated in CI).

### Test Scenarios

1. **Schema lifecycle:**
   - Register → retrieve → re-register with new version → retrieve version history
   - Forward references → resolve after target type registered

2. **Node lifecycle:**
   - Create → read → update → delete
   - Unique constraint enforcement
   - Required field enforcement
   - Default value application
   - extras_policy enforcement

3. **Index correctness:**
   - Create nodes → verify index entries exist (Rust layer)
   - Update indexed property → verify old entry removed, new entry added
   - Delete node → verify all index entries removed
   - Null values → verify no index entry created

4. **Edge integrity:**
   - Create edge → verify OUT + IN entries
   - Delete source node → verify all edges (both directions) cleaned up
   - Bidirectional edge lifecycle
   - Sort key ordering in edge listing

5. **Query correctness:**
   - FindNodes with each operator type (==, >=, <, startsWith, contains)
   - Multi-predicate with residual filtering
   - Null handling in queries
   - Pagination via cursor
   - Empty result sets

6. **CDC completeness:** (Rust layer — CDC is internal)
   - Every mutation produces exactly one CDC entry
   - CDC entries contain correct operation details
   - CDC stream is ordered by versionstamp
   - CDC entries readable via range scan from high-water mark

7. **Atomicity:** (Rust layer)
   - Simulated crash mid-transaction → verify no partial writes
   - Concurrent unique index writes → exactly one succeeds

8. **Namespace isolation:** (Python — tests API boundary)
   - Same entity type in different namespaces → independent data
   - DropNamespace → verify complete cleanup

9. **Background jobs:** (Mixed — Rust for internals, Python for RPC trigger)
   - Index backfill on schema evolution → all existing nodes get index entries
   - StripProperty → property removed from all CBOR blobs
   - Job status queryable via GetJobStatus RPC
   - Job resume after simulated crash (Rust layer)

### Exit Criteria
- All Rust integration tests pass
- All Python gRPC tests pass against a running server
- Basic throughput numbers: point lookups/sec, node creates/sec, FindNodes latency
- No data corruption under concurrent operations
- Python test suite runnable via `pytest tests/python/` with a single command

---

## Dependency Graph

```
M0 (Scaffolding)
 │
 ├── M1 (Schema Registry)
 │    │
 │    ├── M2 (Node CRUD + Indexes)
 │    │    │
 │    │    └── M3 (Edge CRUD)
 │    │         │
 │    │         └── M5 (gRPC API + Jobs) ──→ M6 (Integration)
 │    │              ▲
 │    └── M4 (CEL Query) ───┘
 │         (can start parallel with M3)
```

## What's Deferred to Phase 2

- Multi-predicate index intersection / union strategies
- Cost-based query planning with statistics
- Multi-hop traversal engine
- RocksDB cache layer (all consistency levels)
- Read coalescing
- Traversal depth/timeout/result limits (no traversal in Phase 1)
- Cursor pagination for complex queries
- Watch/reactive subscriptions (Phase 3)
- Multi-site replication (Phase 3)

## Open Items for Phase 1

| Item | Status | Notes |
|---|---|---|
| `cel-rust` crate evaluation | TODO | Verify it supports AST access, type-checking, custom environments. Fallback: `cel-interpreter` or minimal hand-rolled predicate engine. |
| `fdb` crate compatibility | TODO | Verify FDB 7.1 support, tuple layer API, versionstamp support |
| CBOR library choice | Decided: ciborium | Successor to unmaintained serde_cbor |
| Edge ID allocation | Decided: same as NodeId | `[site_id:u8][counter:u64]`, separate counter from node IDs |
| Sort-order float encoding | TODO | IEEE 754 order-preserving encoding impl needed (flip sign bit + conditional invert) |
| Proto message details | TODO | Full message definitions for all RPCs (fields, types, oneof patterns) |
| CDC entry CBOR schema | Decided: per spec | `CdcEntry { site, namespace, timestamp, batch_id, operations }` |
| Config file format | Decided: clap + env vars | CLI flags with env var fallback via clap derive. No config file for Phase 1. |

---

*This plan is a living document. Update as implementation progresses and decisions are made.*
