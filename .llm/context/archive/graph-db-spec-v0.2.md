# Graph Database Engine: Architecture Spec
## Working Title: TBD

**Status:** Draft v0.2 — Architecture and Design
**Author:** Andrew + Claude
**Date:** 2026-02-07
**Changelog:**
- v0.2: Replaced WAL with CDC stream + Saga coordinator. Rationale in Section 7.

---

## 1. Motivation

Existing graph databases force a choice: either you get a purpose-built graph engine with a proprietary storage layer (Neo4j) or you get a graph abstraction over a general-purpose store that carries significant overhead and complexity (JanusGraph). Both approaches have limitations:

- **JanusGraph** is Java-heavy, has a complex plugin architecture, and the query optimization layer is coupled to TinkerPop/Gremlin semantics in ways that limit performance tuning.
- **Neo4j** locks you into a single-node or enterprise-only clustering model with a proprietary query language.
- **Neither** provides a clean multi-site replication story with ownership-based conflict resolution.
- **Neither** cleanly separates schema definition from storage enforcement in a way that allows schema-from-client patterns.

### Goals

1. **Point lookups are fast.** Single-node retrieval by ID should be sub-millisecond at the storage layer.
2. **Criteria/scan lookups are performant.** Property-based queries against indexed fields should compile to FDB range scans, not in-memory filtering.
3. **Graph traversal is first-class.** Multi-hop traversal with per-hop filtering should stream results asynchronously with backpressure.
4. **Schema is a client concern.** The API layer declares schema. The storage layer enforces and indexes based on those declarations. New entity types don't require server restarts or recompilation.
5. **Multi-site replication with ownership semantics.** Nodes have home sites. Conflict-free replication via CDC-driven event streaming.
6. **Lifecycle management via namespacing.** Entity types, logical databases, and tenants can be independently managed, cleared, and replicated.

### Non-Goals (for v1)

- Full-text search (delegate to external search engine)
- Graph analytics / batch processing (OLAP workloads)
- SQL compatibility
- Multi-model (document, relational) — graph-first

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
│  gRPC service handlers · Auth · Rate limiting           │
└───────┬───────────────────────────────┬─────────────────┘
        │                               │
        ▼                               ▼
┌───────────────────┐    ┌──────────────────────────────┐
│  Query Executor   │    │     Mutation Pipeline         │
│  Stream assembly  │    │  Validate → FDB write + CDC  │
│  Index selection  │    │  (or Saga for multi-txn ops)  │
│  Traversal engine │    │                               │
└───────┬───────┬───┘    └──────┬───────────────────────┘
        │       │               │
        ▼       ▼               ▼
┌────────────┐ ┌─────────────────────────┐
│  RocksDB   │ │  FoundationDB           │
│  (cache/   │ │  (system of record)     │
│  matviews/ │ │  Entities, edges,       │
│  temporal  │ │  indexes, CDC stream,   │
│  indexes)  │ │  schema registry,       │
└────────────┘ │  ID alloc, sagas        │
               └─────────────────────────┘
```

### Technology Stack

| Component | Choice | Rationale |
|---|---|---|
| Language | Rust | Sub-millisecond hot paths, zero-copy serialization, tokio async runtime |
| Async runtime | tokio | Task groups for traversal fan-out, mature ecosystem |
| Primary storage | FoundationDB | Ordered KV with strict serializable ACID, automatic sharding |
| Cache layer | RocksDB | Materialized views, temporal indexes, hot data cache |
| Query predicates | CEL (cel-rust) | Portable expression language, type-checkable, AST-decomposable |
| Wire transport | gRPC (tonic) | Streaming RPCs, protobuf framing, wide client language support |
| FDB client | `fdb` crate | Tokio-native, FDB 7.1, actively maintained with BindingTester CI |
| Serialization | Custom varint + CBOR | Varint for keys/IDs, CBOR for property values |

---

## 3. User Stories

### 3.1 Schema Management

**US-1: Register an entity type**
> As a developer, I want to register a new entity type (e.g., "Person") with its property definitions and edge declarations so that the system knows how to validate, index, and query nodes of that type.

Acceptance criteria:
- Entity schema is defined as JSON and sent via gRPC
- Server validates the schema structure
- Server stores the schema in FDB metadata subspace
- Server creates index rules for annotated properties
- Server builds CEL type environment for the entity
- Entity type becomes immediately available for node creation
- Schema can reference edge targets that are not yet registered (forward declaration)

**US-2: Schema forward references**
> As a developer, I want to declare that Person has a "WORKS_AT" edge targeting "Company" even before the Company schema is registered, so that I can build schemas incrementally across teams.

Acceptance criteria:
- Schema registration succeeds even if edge target types don't exist yet
- Creating actual edges to unregistered types is rejected
- When the target type is later registered, edges become creatable with no migration

**US-3: Schema evolution**
> As a developer, I want to add new properties or edge types to an existing entity schema without downtime or data migration.

Acceptance criteria:
- Adding a new non-indexed property: immediate, no backfill
- Adding a new indexed property: triggers async background index backfill (via saga)
- Schema version is tracked; CEL cache invalidates on version change
- Removing a property: TBD (soft deprecation? hard removal?)

### 3.2 Node Operations

**US-4: Create a node**
> As a developer, I want to create a node of a registered type with validated properties.

Acceptance criteria:
- Request specifies entity type and properties
- Server validates all properties against schema (types, required fields)
- Rejects if entity type is not registered
- Assigns a unique ID (or accepts client-provided ID)
- Writes node data + all index entries + CDC entry in a single FDB transaction
- CDC entry represents the committed, complete mutation

**US-5: Point lookup by ID**
> As a developer, I want to retrieve a node by its type and ID in sub-millisecond time.

Acceptance criteria:
- Single FDB `get()` call: `(namespace, entities, type, data, id)`
- Returns deserialized properties
- Optional: check RocksDB cache first, fall through to FDB on miss

**US-6: Lookup by unique property**
> As a developer, I want to find a Person by email (unique indexed field).

Acceptance criteria:
- Compiles to unique index lookup: `(namespace, entities, Person, idx, email, value)` returns ID
- Then point lookup for full node data
- Two FDB reads, both sub-millisecond

### 3.3 Edge Operations

**US-7: Create an edge**
> As a developer, I want to create a typed edge between two existing nodes.

Acceptance criteria:
- Both source and target nodes must exist (referential integrity)
- Both source and target types must be registered
- If edge type is declared in source schema, validates target type matches declaration
- Undeclared edge types are allowed (untyped, no target constraint)
- Edge stored at source node (outgoing) AND target node (incoming) in a single transaction
- Edge properties validated if edge type has a property schema
- CDC entry appended in the same transaction

**US-8: Bidirectional edge semantics**
> As a developer, I want to declare that "KNOWS" is bidirectional so that traversing in either direction from either node works identically.

Acceptance criteria:
- Schema declares `direction: "bidirectional"` on edge type
- Single `CreateEdge` call writes both directions
- Traversal from either endpoint with direction OUT or IN returns the relationship

### 3.4 Query Operations

**US-9: Find nodes by property criteria**
> As a developer, I want to find all Person nodes where age >= 30 and name starts with "A".

Acceptance criteria:
- CEL expression `"age >= 30 && name.startsWith('A')"` sent in FindNodes request
- Server compiles CEL against Person schema, validates types
- Decomposes into: range scan on age index (index-backed) + residual startsWith filter (in-memory)
- Returns streaming results with backpressure
- Pagination via cursor token

**US-10: Multi-hop graph traversal**
> As a developer, I want to traverse from a Person through KNOWS edges to other Persons, then through WORKS_AT edges to Companies, filtering at each hop.

Acceptance criteria:
- TraverseRequest specifies start node/query + ordered list of TraversalSteps
- Each step specifies: edge type, direction, edge filter (CEL), node filter (CEL), field projection
- Results stream back with depth indicator, path breadcrumb, and the node/edge at each hop
- Traversal is async streaming — results from hop 1 feed into hop 2 without materializing the full intermediate set
- Client can cancel the stream at any point

**US-11: Traversal with mixed entity types**
> As a developer, starting from a Person node, I want to traverse a polymorphic edge (e.g., TAGGED_WITH -> *) and then filter the target nodes regardless of their type.

Acceptance criteria:
- Polymorphic edges encode target entity type in the edge key
- Traversal resolves target type from edge, loads correct schema for filtering
- CEL filter on target node is compiled against the actual target type's schema
- If target type has no matching field for the CEL predicate, that node is excluded (not an error)

### 3.5 Lifecycle Management

**US-12: Drop all nodes of an entity type**
> As an operator, I want to delete all Person nodes, their edges, and their indexes in one operation.

Acceptance criteria:
- FDB `clearRange` on the Person entity subspace removes all data, indexes, and edges where Person is the source
- Edges pointing TO Person nodes from other entity types are orphaned and must be cleaned up (background job or lazy cleanup on traversal)
- Schema registration remains (can be explicitly removed separately)

**US-13: Namespace isolation**
> As an operator, I want multiple logical databases (namespaces) that are fully isolated in storage.

Acceptance criteria:
- Each namespace is a top-level FDB subspace prefix
- clearRange on a namespace removes everything: entities, edges, indexes, CDC stream, schema registry
- No cross-namespace queries (enforced at API layer)
- Namespaces have independent schema registries

### 3.6 CDC Stream and Caching

**US-14: CDC-driven cache projection**
> As the system, committed mutations should asynchronously project into RocksDB for fast cached reads.

Acceptance criteria:
- Every mutation appends a CDC entry in the same FDB transaction as the data write
- CDC entries only exist for fully committed changes — no partial or pending state
- An async CDC consumer reads new entries and writes corresponding cache entries to RocksDB
- RocksDB projector tracks a high-water mark (last processed CDC versionstamp)
- Read path: check RocksDB first, verify freshness against CDC high-water mark, fall through to FDB on miss or stale
- Cache eviction driven by CDC entries (exact invalidation)

**US-15: CDC replay for cache rebuild**
> As an operator, I want to rebuild the RocksDB cache from scratch by replaying the CDC stream.

Acceptance criteria:
- CDC entries are retained for a configurable duration (default: 7 days)
- Replay process reads CDC from beginning (or checkpoint) and rebuilds RocksDB state
- System remains available during replay (reads fall through to FDB)

**US-19: Bulk import via saga**
> As a developer, I want to import a large batch of nodes/edges that exceeds FDB's single-transaction limits.

Acceptance criteria:
- System creates a saga record with the full operation description
- Data is applied in chunks across multiple FDB transactions
- Each chunk updates the saga progress cursor
- On completion, CDC entries are written and the saga record is deleted
- If the process crashes, a recovery process resumes from the last completed chunk
- Each chunk is idempotent — safe to re-apply on crash recovery
- System remains available during bulk import (other reads/writes unaffected)

### 3.7 Multi-Site Replication

**US-16: Node ownership and site locality**
> As a developer, I want each node to have a home site, and mutations to a node should be authoritative from that site.

Acceptance criteria:
- Every node has a `_home_site` metadata field set at creation
- Mutations to a node from its home site are authoritative
- Mutations from non-home sites are rejected (or routed to home site — TBD)
- Ownership transfer is an explicit operation with a protocol

**US-17: CDC-driven replication between sites**
> As an operator, I want sites to replicate data to each other via CDC streaming.

Acceptance criteria:
- Each site streams its CDC entries to remote sites
- Every CDC entry is a committed fact — no partial or pending state to handle
- Receiving site applies CDC entries only for data owned by the sending site
- Each site tracks a high-water mark per remote site
- Replication is per logical database (namespace)

**US-18: Edge ownership**
> As a developer, I want edges to be owned by the source node's home site by default, with opt-in independent ownership for specific edge types.

Acceptance criteria:
- Default: edge owned by source node's home site
- Schema can declare an edge type as `ownership: "independent"` — edge is owned by the site that created it
- Ownership determines which site is authoritative for the edge
- Replication respects ownership (only the owning site's CDC entries for an edge are applied)

---

## 4. Schema Definition Format

### JSON Schema Structure

```json
{
  "namespace": "myapp",
  "entity": {
    "name": "Person",
    "version": 1,
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
      },
      "bio": {
        "type": "string"
      },
      "created_at": {
        "type": "timestamp",
        "index": "range",
        "default": "now"
      }
    },
    "edges": {
      "KNOWS": {
        "target": "Person",
        "direction": "bidirectional",
        "properties": {
          "since": { "type": "timestamp", "sort_key": true },
          "strength": { "type": "float" }
        }
      },
      "WORKS_AT": {
        "target": "Company",
        "direction": "outgoing",
        "properties": {
          "role": { "type": "string", "index": "equality" },
          "started": { "type": "timestamp", "sort_key": true }
        }
      },
      "TAGGED_WITH": {
        "target": "*",
        "direction": "outgoing",
        "properties": {
          "relevance": { "type": "float" }
        }
      }
    },
    "meta": {
      "default_ownership": "source_site",
      "allow_undeclared_edges": true,
      "extras_policy": "reject"
    }
  }
}
```

### Property Types

| Type | Rust Mapping | FDB Encoding | CEL Type |
|---|---|---|---|
| `string` | `String` | UTF-8 bytes | `string` |
| `int` | `i64` | Big-endian 8 bytes (for sort order) | `int` |
| `float` | `f64` | IEEE 754 order-preserving | `double` |
| `bool` | `bool` | Single byte | `bool` |
| `timestamp` | `i64` (unix micros) | Big-endian 8 bytes | `timestamp` |
| `bytes` | `Vec<u8>` | Raw bytes | `bytes` |

### Index Types

| Index Type | FDB Pattern | Supports |
|---|---|---|
| `unique` | `(ns, entities, Type, idx, field, value)` -> `id` | Equality only. Enforces uniqueness on write. |
| `equality` | `(ns, entities, Type, idx, field, value, id)` -> empty | Equality lookups. Multiple nodes per value. |
| `range` | `(ns, entities, Type, idx, field, value, id)` -> empty | Equality, >, >=, <, <=. Values must be sort-order encoded. |

### Schema Registration Rules

| Rule | Behavior |
|---|---|
| Node creation without registered type | **REJECT** |
| Edge declaration to unregistered target type | **ALLOW** (forward declaration) |
| Edge creation to unregistered target type | **REJECT** (target nodes can't exist) |
| Edge creation to non-existent target node | **REJECT** (referential integrity) |
| Undeclared edge type on a node | Depends on `allow_undeclared_edges` flag |
| Property not in schema | Depends on `extras_policy`: "reject", "allow", "warn" |

---

## 5. CEL to Query Plan Pipeline

This is the core query compilation path. A CEL expression enters as a string and exits as a set of FDB operations plus optional residual predicates.

### Pipeline Stages

```
         CEL String
             │
    ┌────────▼────────┐
    │  1. Parse        │  CEL grammar → AST
    └────────┬────────┘
             │
    ┌────────▼────────┐
    │  2. Type-Check   │  Validate against entity schema
    │                  │  "age >= 30" → age is Int ✓, 30 is Int ✓
    │                  │  "salary > 0" → salary not in schema ✗ REJECT
    └────────┬────────┘
             │
    ┌────────▼────────────────┐
    │  3. Normalize            │  Flatten nested logic:
    │                          │  !(age < 30) → age >= 30
    │                          │  age > 29 → age >= 30 (integer)
    └────────┬────────────────┘
             │
    ┌────────▼────────────────┐
    │  4. Predicate Extraction │  Split AST into individual predicates:
    │                          │  "age >= 30 && name == 'A'"
    │                          │  → [age >= 30] AND [name == 'A']
    └────────┬────────────────┘
             │
    ┌────────▼────────────────┐
    │  5. Index Matching       │  For each predicate, check if an index exists:
    │                          │  [age >= 30] → age has Range index → ✓ INDEX SCAN
    │                          │  [name == 'A'] → name has Unique index → ✓ POINT LOOKUP
    │                          │  [bio.contains('X')] → bio not indexed → RESIDUAL
    └────────┬────────────────┘
             │
    ┌────────▼────────────────┐
    │  6. Plan Selection       │  Choose execution strategy:
    │                          │  • If any POINT LOOKUP: start there (most selective)
    │                          │  • Else if RANGE SCAN: use narrowest range
    │                          │  • Combine with AND: intersect result sets
    │                          │  • Remaining predicates → residual filter
    └────────┬────────────────┘
             │
    ┌────────▼────────────────┐
    │  7. Plan Output          │  QueryPlan {
    │                          │    primary: PointLookup(name, "A")
    │                          │    verify: [RangeScan(age, >=, 30)]
    │                          │    residual: [contains(bio, "X")]
    │                          │  }
    └─────────────────────────┘
```

### Query Plan Representation

```rust
enum QueryPlan {
    PointLookup {
        index_field: String,
        value: Value,
    },
    RangeScan {
        index_field: String,
        lower: Option<Bound>,
        upper: Option<Bound>,
    },
    Intersection {
        plans: Vec<QueryPlan>,  // AND: intersect result sets
    },
    Union {
        plans: Vec<QueryPlan>,  // OR: merge result sets
    },
    FullScan {
        entity_type: String,    // worst case: scan all nodes of type
    },
}

struct ExecutionPlan {
    primary: QueryPlan,                // drives the main scan/lookup
    verify: Vec<IndexCheck>,           // additional index checks on candidates
    residual: Option<CompiledCelExpr>, // in-memory filter for non-indexed predicates
    projection: Vec<String>,           // which fields to return
    limit: Option<u32>,
    cursor: Option<Cursor>,
}
```

### CEL Operator to Index Operation Mapping

| CEL Expression | Index Type Required | FDB Operation |
|---|---|---|
| `field == value` | unique or equality | Single key get |
| `field != value` | range | Full index scan minus one key (expensive) |
| `field > value` | range | Range scan: (field, value+1) to (field, MAX) |
| `field >= value` | range | Range scan: (field, value) to (field, MAX) |
| `field < value` | range | Range scan: (field, MIN) to (field, value) |
| `field <= value` | range | Range scan: (field, MIN) to (field, value+1) |
| `field.startsWith(prefix)` | range (string) | Range scan: (field, prefix) to (field, prefix+\xFF) |
| `field.contains(substr)` | NONE | Residual: full scan + in-memory filter |
| `field in [a, b, c]` | equality or unique | Multiple point lookups, union results |

### AND/OR Handling

| Expression Pattern | Strategy |
|---|---|
| `A && B` (both indexed) | Execute most selective first, verify second against candidates |
| `A && B` (one indexed) | Execute indexed predicate, apply non-indexed as residual |
| `A && B` (neither indexed) | Full scan with combined residual filter |
| `A \|\| B` (both indexed) | Execute both, union results, deduplicate |
| `A \|\| B` (one indexed) | Full scan with combined residual (can't avoid it) |

### Edge Filter CEL

Edge property filters work identically but compile against the edge's property schema instead of the node schema. The CEL environment is built from the edge type's declared properties:

```
Edge type: WORKS_AT { role: string, started: timestamp }
CEL environment: { role: String, started: Timestamp }
Expression: "role == 'Engineer' && started > timestamp('2020-01-01')"
```

If the edge type has `sort_key: true` on a property (e.g., `started`), range predicates on that field compile to subspace range scans within the adjacency list — this is the vertex-centric index pattern from JanusGraph.

---

## 6. FDB Subspace Layout

### Key Hierarchy

```
(tenant)                                          ← optional multi-tenancy prefix
  (namespace)                                     ← logical database
    (_meta)
      (schemas, {entity_type})                    → EntitySchema bytes
      (schema_versions, {entity_type}, {version}) → EntitySchema bytes
      (replication, {remote_site})                → high-water mark (versionstamp)
    (_cdc)
      ({versionstamp})                            → CDC entry bytes (committed facts only)
    (_sagas)
      ({saga_id})                                 → Saga state bytes (in-progress multi-txn ops)
    (_ids)
      ({entity_type})                             → next ID block start
    (entities)
      ({entity_type})
        (data, {node_id})                         → serialized properties
        (idx)
          ({field_name}, {value}, {node_id})       → empty (or node_id for unique)
        (edges)
          ({edge_type}, OUT, {sort_key}, {target_type}, {target_id}, {edge_id})
                                                  → edge properties
          ({edge_type}, IN, {sort_key}, {source_type}, {source_id}, {edge_id})
                                                  → edge properties (or empty)
```

### Key Encoding Details

All tuple elements are encoded using FDB's tuple layer, which preserves lexicographic ordering:

- **Strings**: null-terminated with escape bytes
- **Integers**: big-endian with sign handling for correct sort order
- **Timestamps**: encoded as int64 (unix microseconds) — sorts chronologically
- **Node IDs**: could be UUIDs (16 bytes) or sequential int64 (varint)
- **Sort keys**: encoded in the key position to enable range scans within edge subspaces
- **Versionstamps**: 10-byte server-assigned values (8 bytes commit version + 2 bytes batch order)

### Example Keys

```
Person node:
  ("myapp", "entities", "Person", "data", "p_42")
  → CBOR { "name": "Andrew", "age": 38, "_home_site": "site_a" }

Person name index:
  ("myapp", "entities", "Person", "idx", "name", "Andrew", "p_42")
  → empty bytes

Person age index:
  ("myapp", "entities", "Person", "idx", "age", 38, "p_42")
  → empty bytes

Outgoing edge:
  ("myapp", "entities", "Person", "p_42", "edges", "WORKS_AT", 0, 1705363200, "Company", "c_7", "e_101")
  → CBOR { "role": "Engineer", "_owner": "site_a" }
  (0 = OUT direction, 1705363200 = sort key timestamp)

Incoming edge (at target):
  ("myapp", "entities", "Company", "c_7", "edges", "WORKS_AT", 1, 1705363200, "Person", "p_42", "e_101")
  → CBOR { "role": "Engineer", "_owner": "site_a" }
  (1 = IN direction)

CDC entry:
  ("myapp", "_cdc", <versionstamp>)
  → CBOR { ops: [...], site: "site_a" }

Saga record:
  ("myapp", "_sagas", "saga_bulk_import_20260207_001")
  → CBOR { status: "pending", progress: 23, chunks_total: 50, payload: [...] }

Schema:
  ("myapp", "_meta", "schemas", "Person")
  → CBOR { version: 1, properties: {...}, edges: {...} }
```

---

## 7. CDC + Saga Architecture

### Design Rationale

v0.1 used a single WAL (Write-Ahead Log) for crash recovery, cache projection, replication, and cache rebuild. This conflated two concerns:

1. **Durability for incomplete multi-transaction operations** — requires pending/partial state
2. **Change notification for committed mutations** — should only contain completed facts

A single stream serving both purposes forces consumers to check entry status, handle partial state, and deal with entries that reference data that doesn't yet exist. This is a design smell.

**FDB already provides crash recovery.** A single FDB transaction is atomic — if the process crashes before commit, nothing is written. FDB's internal WAL handles this. We don't need our own WAL for single-transaction operations.

The v0.2 architecture separates these concerns:

- **CDC stream**: Append-only log of committed facts. Versionstamped keys for zero-contention ordering. Every entry represents a fully committed, consistent state change. Consumed by cache projector, replication, audit.
- **Saga coordinator**: Small, bounded set of in-progress records for operations that exceed FDB's single-transaction limits (10MB, 5 seconds). Internal only. Self-deleting on completion.

### Write Paths

#### Single-Transaction Mutations (99%+ of operations)

```
Client mutation arrives
    │
    ▼
┌─────────────────────────────┐
│ Validate against schema      │
│ (type exists, props valid,   │
│  edge targets exist)         │
└──────────────┬──────────────┘
               │
               ▼
┌─────────────────────────────┐
│ Single FDB Transaction:      │
│  1. Write node/edge data     │
│  2. Write index entries      │
│  3. Append CDC entry         │
│     (versionstamped key)     │
│  All atomic.                 │
└──────────────┬──────────────┘
               │
               ▼
       ACK to client
               │
               ▼ (async, non-blocking)
┌─────────────────────────────┐
│ CDC Consumer (background)    │
│  Reads new CDC entries       │
│  Projects to RocksDB cache   │
│  Updates replication stream  │
└─────────────────────────────┘
```

This is one round-trip to FDB. The CDC entry is marginal cost — one additional KV pair in an already-open transaction. The client gets immediate read-after-write consistency because data and indexes are written atomically.

#### Multi-Transaction Mutations (bulk import, index backfill)

```
Client bulk mutation arrives
    │
    ▼
┌──────────────────────────────────┐
│ FDB Txn 1: Create saga record    │
│  { saga_id, status: "pending",   │
│    operation: <full description>, │
│    chunks_total: N,               │
│    chunks_completed: 0 }          │
└──────────────┬───────────────────┘
               │
               ▼
┌──────────────────────────────────┐
│ FDB Txn 2..N: Apply data chunks  │
│  Each chunk:                      │
│   - Write node/edge data          │
│   - Write index entries           │
│   - Update saga progress cursor   │
│  Each chunk is idempotent.        │
└──────────────┬───────────────────┘
               │
               ▼
┌──────────────────────────────────┐
│ FDB Txn N+1: Finalize            │
│  - Write CDC entries for all      │
│    changes (or per-chunk)         │
│  - Delete saga record             │
│  Atomic: CDC appears only when    │
│  saga is complete.                │
└──────────────────────────────────┘
```

Recovery: A background process scans `_sagas` for records older than a configurable threshold. For each stale saga, it resumes applying chunks from the last completed position. Idempotent chunk application means re-applying a partially-completed chunk is safe.

### CDC Entry Schema

```rust
struct CdcEntry {
    // Key: (ns, "_cdc", versionstamp) — server-assigned, zero contention
    site: String,
    namespace: String,
    timestamp: i64,
    operations: Vec<CdcOperation>,
}

enum CdcOperation {
    NodeCreate {
        entity_type: String,
        node_id: String,
        properties: HashMap<String, Value>,
        home_site: String,
    },
    NodeUpdate {
        entity_type: String,
        node_id: String,
        changed_properties: HashMap<String, Value>,
        previous_version: u64,
    },
    NodeDelete {
        entity_type: String,
        node_id: String,
    },
    EdgeCreate {
        source: NodeRef,
        target: NodeRef,
        edge_type: String,
        edge_id: String,
        properties: HashMap<String, Value>,
        owner_site: String,
    },
    EdgeDelete {
        edge_id: String,
    },
    SchemaRegister {
        entity_type: String,
        schema_version: u32,
        schema_json: String,
    },
}
```

### Saga Record Schema

```rust
struct SagaRecord {
    // Key: (ns, "_sagas", saga_id)
    saga_id: String,
    operation_type: SagaType,
    status: SagaStatus,           // Pending, InProgress, Failed
    created_at: i64,
    last_updated: i64,
    chunks_total: u32,
    chunks_completed: u32,
    payload: Vec<u8>,             // Operation-specific data (CBOR)
}

enum SagaType {
    BulkImport,
    IndexBackfill { entity_type: String, field: String },
    EntityTypeDrop { entity_type: String },
}

enum SagaStatus {
    Pending,
    InProgress,
    Failed { error: String, retry_count: u32 },
}
```

### CDC Consumers

| Consumer | Purpose | Latency Tolerance |
|---|---|---|
| RocksDB Projector | Cache population, materialized views | Sub-second |
| Replication Streamer | Send to remote sites | Seconds (network-bound) |
| Audit Logger | External audit trail | Seconds |

Note: Index backfill is handled via sagas, not CDC consumers. The saga directly writes index entries. CDC entries for the backfilled data are written when the saga completes.

### Versionstamp Ordering

CDC entries use FDB versionstamps as keys. Key properties:

- **Zero contention**: Server-assigned at commit time. No application-managed counter. No hot-key bottleneck. (Resolves gap C2 from v0.1.)
- **Total order**: Versionstamps are globally ordered across all transactions in the FDB cluster.
- **Not known at write time**: The versionstamp value is assigned at commit, so the write path cannot return a sequence number to the client in the ACK. This is acceptable — clients don't need the CDC position.
- **Consumer pattern**: Forward range scan from last high-water mark to end of CDC subspace. Returns only new entries. Empty result if nothing new. Alternatively, use `txn.watch()` to avoid polling.

```rust
// CDC consumer loop
let hwm: [u8; 10] = load_last_processed_versionstamp();
let new_entries = txn.get_range(
    &cdc_subspace.range_from(hwm),
    RangeOption { limit: 1000, .. }
).await;
for entry in new_entries {
    process(entry);
    update_hwm(entry.versionstamp);
}
```

### RocksDB Cache Strategy

| Data in RocksDB | Source | Eviction |
|---|---|---|
| Hot node data | Projected from CDC or read-through | LRU with size cap |
| Materialized traversal results | Precomputed for common patterns | TTL-based |
| Temporal indexes (e.g., "modified in last hour") | Projected from CDC | Time-window expiry |
| Aggregation caches (counts, sums) | Maintained by CDC consumer | Invalidated by CDC |

### Cache Coherence

```
Read request arrives
    │
    ▼
Check RocksDB for cached result
    │
    ├── HIT: Check high-water mark (stored in FDB, not RocksDB)
    │     │
    │     ├── Cache HWM >= required freshness → return cached
    │     │
    │     └── Cache HWM < required freshness → fall through to FDB
    │
    └── MISS: Read from FDB
              │
              └── Optionally populate RocksDB (read-through caching)
```

Note: The CDC high-water mark is stored in FDB (source of truth), not in RocksDB. This prevents stale-data-served-as-fresh bugs if RocksDB crashes and recovers to an older checkpoint. (Resolves gap I2 from v0.1.)

---

## 8. Replication Model

### Ownership Rules

1. **Nodes** are owned by their `_home_site`.
2. **Edges** are owned by the source node's home site (default), or by the creating site (if `ownership: "independent"` in edge schema).
3. Only the owning site can create, modify, or delete owned data.
4. Remote sites receive owned data via CDC replication.

### Replication Flow

```
Site A (owns Person_42)              Site B (owns Company_7)
┌───────────────────┐                ┌───────────────────┐
│ CDC: <vs_1001>    │ ──replicate──→ │ apply <vs_1001>   │
│   create Person_42│                │ (Person_42 owned   │
│                   │                │  by A → accept)    │
│                   │                │                    │
│ apply <vs_501>    │ ←──replicate── │ CDC: <vs_501>     │
│ (Company_7 owned  │                │   create Company_7 │
│  by B → accept)   │                │                    │
└───────────────────┘                └───────────────────┘
```

Because CDC entries only contain committed facts, the receiving site never encounters partial or pending operations. This simplifies the replication consumer significantly compared to v0.1's WAL-based approach.

### Causal Ordering for Edges

Edge creation requires both source and target nodes to exist. In a multi-site scenario, a CDC entry that creates an edge may arrive before the target node has been replicated. The CDC consumer must handle this:

```
CDC entry arrives: create edge Person_42 → Company_7
    │
    ▼
Check: does Company_7 exist locally?
    │
    ├── YES → apply the edge write
    │
    └── NO → queue the entry in a dependency buffer
              (keyed by the missing node ref)
              │
              ▼
         When Company_7 arrives via replication
              → drain the dependency buffer
              → apply queued edge writes
```

Note: This dependency buffering is strictly for cross-site node references (node doesn't exist locally yet). It is NOT caused by incomplete multi-txn operations — sagas resolve locally before emitting CDC entries.

### Conflict Scenarios

| Scenario | Resolution |
|---|---|
| Two sites modify same owned node | Can't happen — only home site can modify |
| Two sites create edge to same target | Allowed — edges have unique IDs, no conflict |
| Ownership transfer race | Explicit protocol: request → ack → transfer. Only one in-flight transfer per node. |
| Schema registered differently at two sites | Schema changes replicate via CDC. Last-writer-wins by versionstamp. Recommend: single schema-authority site. |

---

## 9. Namespace Hierarchy

```
                    ┌─────────────┐
                    │   Tenant    │  Optional. Isolation boundary for
                    │  (prefix)   │  multi-tenant SaaS deployments.
                    └──────┬──────┘
                           │
                    ┌──────▼──────┐
                    │  Namespace   │  Logical database. Replication unit.
                    │  ("myapp")   │  CDC scope. Schema registry scope.
                    └──────┬──────┘
                           │
              ┌────────────┼────────────┐
              │            │            │
       ┌──────▼──────┐ ┌──▼───┐ ┌──────▼──────┐
       │  Entity Type │ │ _meta│ │    _cdc     │
       │  ("Person")  │ │      │ │             │
       └──────┬──────┘ └──────┘ └─────────────┘
              │
    ┌─────────┼──────────┐
    │         │          │
┌───▼──┐ ┌───▼──┐ ┌─────▼─────┐
│ data │ │  idx │ │   edges   │
└──────┘ └──────┘ └───────────┘
```

### Lifecycle Operations

| Operation | FDB Action | Scope |
|---|---|---|
| Drop entity type | `clearRange(ns, entities, Type, ...)` | All data, indexes, outgoing edges for that type. Incoming edges from other types become orphaned. Uses saga for large types. |
| Drop namespace | `clearRange(ns, ...)` | Everything: entities, indexes, edges, CDC, schema |
| Drop tenant | `clearRange(tenant, ...)` | All namespaces under tenant |
| Purge orphaned edges | Background scan of edge subspaces, check target existence | Triggered after entity type drop |

---

## 10. Gap Analysis

Analysis of missing pieces, inconsistencies, and unresolved design problems. Categorized by severity.

### CRITICAL: Must Resolve Before Implementation

**C1. Index intersection has no execution strategy.**
The query planner says "execute most selective first, verify second against candidates" but never defines HOW. When both `age >= 30` and `salary >= 100000` are range-indexed, verifying candidates from one index against the other requires N point lookups on the node data (not the second index). This is potentially slower than a full scan. The spec needs: a cost model, selectivity estimation (even naive cardinality-based), and a decision algorithm for choosing between index scan + verify, dual index scan + set intersection, or full scan + residual.

~~**C2. WAL sequence allocation is a hot-key bottleneck.**~~
**RESOLVED in v0.2.** CDC entries use FDB versionstamps (server-assigned, zero contention) instead of application-managed monotonic counters. See Section 7.

**C3. Bidirectional edges have ambiguous ownership.**
If Person_42 (owned by site A) has a bidirectional KNOWS edge to Person_99 (owned by site B), which site owns the edge? "Source node's home site" — but in a bidirectional edge, both nodes are sources. Need: designate one node as the canonical source at creation time (e.g., the node specified as `source` in CreateEdge), or treat bidirectional as two independent directed edges with separate ownership.

**C4. No query timeout or traversal depth limits.**
TraverseRequest has no `max_depth`, `timeout_ms`, or `max_results` field. A 4-hop traversal with branching factor 100 produces 100M paths. This is a DoS vector. Required: mandatory limits with server-enforced defaults.

**C5. Orphaned incoming edges have no efficient cleanup path.**
Dropping an entity type clears outgoing edges but orphans incoming edges stored at other entity types. The only cleanup is O(all edges in the database) — scanning every entity type's edge subspace. Need: an incoming edge index `(ns, entities, Type, incoming_refs, source_type, source_id)` that enables targeted cleanup at the cost of additional write amplification.

**C6. Property removal is undefined behavior.**
If "salary" is removed from the Person schema: old nodes still have it in CBOR, orphaned index entries remain, CEL rejects queries referencing it. Need: explicit schema evolution states (active, deprecated, removed) with defined behavior at each state.

**C7. Multi-site unique constraint violation.**
Two sites can each create a node with the same unique-indexed value (e.g., same email). Both succeed locally. On replication, the receiving site detects a unique violation with no resolution strategy. Need: either route unique-constrained writes through a single authority site, or use a reservation protocol where a unique value is claimed globally before the write commits.

### IMPORTANT: Should Resolve Before Phase 2

**I1. CEL type system doesn't handle NULL / missing values.**
Schema says `age: int` but a node may not have `age` set (optional property, schema evolution). CEL expression `age > 30` crashes at eval time. Need: three-valued logic (present/null/missing) or enforce that all schema properties have defaults.

~~**I2. RocksDB cache coherence mechanism is underspecified.**~~
**RESOLVED in v0.2.** CDC high-water mark is stored in FDB (source of truth), not in RocksDB. See Section 7 → Cache Coherence.

**I3. Polymorphic edge traversal has no indexing strategy.**
Traversing `TAGGED_WITH -> *` with a filter on the target node requires loading each target, checking its type, loading its schema, and evaluating the filter. No index optimization is possible because the target type is unknown. For high-degree polymorphic edges, this is O(edge count). Need: either accept this limitation explicitly or add a polymorphic index strategy.

**I4. No transactionality guarantee across multi-step queries.**
A FindNodes query that does an index scan followed by data fetches is not atomic. Concurrent mutations can produce inconsistent results (node exists in index but deleted from data store between the two reads). Need: explicit consistency level in the API (snapshot reads via FDB read versions, or accept eventual consistency).

**I5. Cursor pagination is undefined for complex queries.**
FindNodes pagination needs a cursor that encodes: the last index position scanned, the scan direction, and enough state to resume. Traversal pagination is harder — state includes depth, path, and per-hop position. The spec defines no cursor format.

**I6. No EXPLAIN / query plan inspection.**
CEL expressions are opaque strings. Clients can't see what the server will do. Need: an explain endpoint that returns the QueryPlan without executing it. Critical for debugging slow queries.

### MINOR: Address During Implementation

**M1. ID allocation strategy is undecided.**
UUIDs (16 bytes, no coordination) vs sequential int64 (smaller, requires counter). Affects key size, sort locality, and write throughput. Recommend: ULID or UUIDv7 (time-ordered UUIDs) — gets you temporal locality without coordination.

**M2. Schema version management is incomplete.**
No definition of when versions increment, how clients discover the current version, or how old schema versions interact with new data.

**M3. Edge ID generation is unspecified.**
Globally unique? Per-source-node? Server-assigned? Affects CDC entries, replication, and deduplication.

**M4. No specification for bulk import.**
Partially addressed in v0.2 via saga coordinator (US-19). Chunking strategy, progress tracking, and crash recovery are defined. Still needed: rollback-on-failure semantics (do partial imports remain or get cleaned up?).

**M5. Schema conflict resolution across sites.**
Two sites registering conflicting schemas (different property types for the same field) has no defined resolution beyond "last writer wins." Recommend: single schema-authority site per namespace.

### Original Open Questions (Retained)

- **OR queries across entity types**: Require typed queries? Or support cross-type union?
- **Aggregation primitives**: Server-side or client-side only?
- **Cycle detection in traversal**: Track visited nodes? Cost of visited set?
- **Computed / derived properties**: Out of scope for v1?
- **Nested properties / complex types**: Flat key-value only for v1?
- **CDC retention and compaction**: Checkpointing strategy?
- **Observability**: Metrics, tracing, alerting strategy
- **Backup and restore**: FDB native backup. RocksDB rebuildable from CDC.
- **Resource limits**: Per-query memory, per-namespace quotas, rate limiting

---

## 11. Implementation Phases

### Phase 1: Storage Foundation (MVP)
- FDB subspace layout and tuple encoding
- Node CRUD with schema validation
- Property indexes (unique, equality, range)
- Edge CRUD with referential integrity
- CEL compilation and basic query planning (single-predicate)
- gRPC API with streaming FindNodes
- CDC stream with versionstamped keys (single-txn write path)

### Phase 2: Query Engine
- Multi-predicate CEL decomposition
- Index intersection and union strategies
- Multi-hop traversal with per-hop filtering
- RocksDB cache layer with CDC projection
- Cursor-based pagination

### Phase 3: Replication
- CDC consumers and consumer framework
- Single-site ownership model
- CDC streaming between sites
- Causal ordering and dependency buffer
- Ownership transfer protocol
- Saga coordinator for bulk operations and index backfill

### Phase 4: Operational Maturity
- Schema migration tooling
- Observability and metrics
- Backup/restore integration
- Resource limits and quotas
- Performance benchmarking and tuning

---

*This is a living document. Architecture decisions captured in Apple Notes: "Graph DB - Architecture Decisions", "Graph DB - Schema Enforcement Rules", "Graph DB - Ownership + Replication Model", "Graph DB - API + Wire Protocol Design".*
