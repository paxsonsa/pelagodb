# Graph Database Engine: Architecture Spec
## Working Title: TBD

**Status:** Draft v0.3 — Reactive Subscriptions, High-Throughput Reads, Query Optimization
**Author:** Andrew + Claude
**Date:** 2026-02-10
**Changelog:**
- v0.3 (updated): Folded C6 research (property removal) into schema evolution design. Properties are client-owned lifecycle: removal stops schema validation but leaves old data in CBOR; explicit client RPCs (DropIndex, StripProperty) for cleanup; no deprecation state machine. Resolved gap C6. See Section 4.4 (Schema Evolution) and API sections.
- v0.3 (updated): Added null semantics design: missing properties equal null with no distinction, null values not indexed, null handled in CEL expressions and required field validation. Resolved gap I1. See Section 4 (Null Semantics subsection) and Section 5 (CEL null handling rules).
- v0.3 (updated): Added index intersection execution strategy with cost-based selectivity estimation. Single-index selection as baseline, merge-join intersection for similar-selectivity predicates, composite index preference, and EXPLAIN endpoint. Background job maintains per-field statistics (eventually consistent). Resolved gaps C1, I6. See Section 5 expansion and new Section 5.3 (Query Statistics).
- v0.3: Removed saga coordinator — FDB is the WAL, all mutations follow unified write path. Background jobs replace sagas for index backfill and cleanup. Added watch/reactive subscriptions (point watches via FDB native, query watches via CDC filtering). Added high-throughput read design: consistency levels, read coalescing, traversal limits. Resolved gaps C4, M4.
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
7. **Reactive subscriptions.** Clients can watch individual nodes or query result sets for real-time change notifications.
8. **High-throughput reads.** The read path scales horizontally with configurable consistency levels and read coalescing for hot data.

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
│  Watch registry · Traversal limit enforcement           │
└───────┬───────────────────────────────┬─────────────────┘
        │                               │
        ▼                               ▼
┌───────────────────┐    ┌──────────────────────────────┐
│  Query Executor   │    │     Mutation Pipeline         │
│  Stream assembly  │    │  Validate → FDB write + CDC  │
│  Index selection  │    │  (unified path for all ops)  │
│  Traversal engine │    │                               │
│  Watch Registry   │    │                               │
└───────┬───────┬───┘    └──────┬───────────────────────┘
        │       │               │
        ▼       ▼               ▼
┌────────────┐ ┌─────────────────────────┐
│  RocksDB   │ │  FoundationDB           │
│  (cache/   │ │  (system of record)     │
│  matviews/ │ │  Entities, edges,       │
│  temporal  │ │  indexes, CDC stream,   │
│  indexes,  │ │  schema registry,       │
│ read-thru) │ │  ID alloc, jobs         │
└────────────┘ │                         │
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
- Adding a new indexed property: triggers async background index backfill (via background job)
- Schema version is tracked; CEL cache invalidates on version change
- Removing a property: server drops it from current schema; old data stays in CBOR; queries referencing removed property fail at CEL type-check
- Client explicitly drops orphaned indexes via `DropIndex` RPC (synchronous)
- Client optionally strips old data via `StripProperty` RPC (background job, optional for cleanup)
- No deprecation lifecycle states—schema defines current state only

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
- Single `CreateEdge` call writes both directions as paired directed edges with a shared `pair_id`
- Each direction is owned by its source node's home site
- Traversal from either endpoint with direction OUT or IN returns the relationship
- Deleting the pair deletes both directions atomically

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
- Edges pointing TO Person nodes from other entity types are orphaned and must be cleaned up (background job)
- Schema registration remains (can be explicitly removed separately)

**US-13: Namespace isolation**
> As an operator, I want multiple logical databases (namespaces) that are fully isolated in storage.

Acceptance criteria:
- Each namespace is a top-level FDB subspace prefix
- clearRange on a namespace removes everything: entities, edges, indexes, CDC stream, schema registry
- No cross-namespace queries (enforced at API layer)
- Namespaces have independent schema registries

**US-25: Drop an index on a removed property**
> As a developer, after removing a property from a schema, I want to drop the orphaned index to reclaim storage.

Acceptance criteria:
- DropIndex RPC accepts (namespace, entity_type, property_name)
- Synchronously deletes all index entries: `(namespace, entities, Type, idx, property, *)`
- Future writes do not create entries for this property
- Old nodes still have the property in CBOR, but it is no longer indexed
- RPC completes immediately; no background job needed

**US-26: Strip a removed property from all nodes**
> As a developer, to fully reclaim storage, I want to remove a property from CBOR blobs of all existing nodes.

Acceptance criteria:
- StripProperty RPC accepts (namespace, entity_type, property_name)
- Enqueues a background job that scans all nodes of the entity type
- Background job deserializes CBOR, removes the property, re-serializes, and writes back
- Job tracks progress and is resumable after crashes (idempotent)
- Client can query job status to check completion
- Operation is optional—old data can remain in CBOR indefinitely if not stripped

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

**US-16: Bulk import via chunked mutations**
> As a developer, I want to import a large batch of nodes/edges that exceeds FDB's single-transaction limits.

Acceptance criteria:
- Client sends chunks via normal mutation API, each chunk is a separate transaction
- Each chunk is validated and written through the unified write path
- CDC entries are written per-chunk (or per-batch if client specifies batch_id tag)
- System remains available during import (other reads/writes unaffected)
- If the client crashes at chunk 30/50, 30 chunks are committed and visible
- Client resumes from chunk 31; no re-application of already-committed chunks

### 3.7 Watch and Reactive Subscriptions

**US-17: Watch a single node for changes**
> As a developer, I want to subscribe to changes on a single node and receive real-time notifications when it is modified.

Acceptance criteria:
- WatchNode(type, id) establishes a server-streaming gRPC RPC
- Server uses FDB txn.watch() on the node data key
- When the watch fires, server reads new state and pushes to client
- Multiple clients watching the same node share a single FDB watch with server-side fan-out
- Watch auto-resubscribes on FDB transaction reset
- Sub-millisecond latency after commit (FDB native watch latency)

**US-18: Watch edges on a node**
> As a developer, I want to receive notifications when edges are added or removed from a node.

Acceptance criteria:
- WatchEdges(type, id, edge_type?, direction?) streams edge events
- Server uses FDB watch on edge subspace prefix
- Events include: edge created, edge deleted, edge properties updated
- Optional edge_type and direction filters reduce notifications to relevant subset
- Multiple clients with same subscription share FDB watch

**US-19: Watch a query result set**
> As a developer, I want to subscribe to a query (e.g., "all active users") and receive events when nodes match or stop matching the criteria.

Acceptance criteria:
- WatchQuery(entity_type, cel_expression) streams query result changes
- Server registers subscription in memory
- CDC consumer filters incoming CDC entries against active subscriptions
- Events: NodeEntered (now matches), NodeLeft (no longer matches), NodeUpdated (still matches, properties changed)
- Server sends initial snapshot before streaming deltas
- Events reflect the result set — order and membership changes
- Subscriptions are server-local (not replicated); reconnect on failover required

**US-20: Consistency levels for reads**
> As a developer, I want to choose read consistency for different query patterns: strong for critical data, session for most queries, eventual for analytics.

Acceptance criteria:
- API parameter: consistency_level ∈ {Strong, Session, Eventual}
- Strong: direct FDB read (serializable, latest committed state)
- Session: RocksDB cache verified against FDB read version (bounded staleness, bounded to CDC consumer lag)
- Eventual: RocksDB cache only (fastest, may be stale by CDC consumer lag)
- Default: Session
- Server enforces consistency level; client cannot request stronger than configured policy

**US-21: Read coalescing for hot data**
> As the system, when multiple concurrent requests for the same uncached node arrive, coalesce into a single FDB read.

Acceptance criteria:
- Per-key in-flight request map tracks pending reads
- Multiple waiters on same key all receive the result from single FDB read
- Prevents thundering herd on popular nodes (e.g., lookup by unique email)
- Transparent to client
- Implementation: per-key tokio::sync::broadcast channel

### 3.8 Multi-Site Replication

**US-22: Node ownership and site locality**
> As a developer, I want each node to have a home site, and mutations to a node should be authoritative from that site.

Acceptance criteria:
- Every node has a `_home_site` metadata field set at creation
- Mutations to a node from its home site are authoritative
- Mutations from non-home sites are rejected (or routed to home site — TBD)
- Ownership transfer is an explicit operation with a protocol

**US-23: CDC-driven replication between sites**
> As an operator, I want sites to replicate data to each other via CDC streaming.

Acceptance criteria:
- Each site streams its CDC entries to remote sites
- Every CDC entry is a committed fact — no partial or pending state to handle
- Receiving site applies CDC entries only for data owned by the sending site
- Each site tracks a high-water mark per remote site
- Replication is per logical database (namespace)

**US-24: Edge ownership**
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

### Null Semantics

Missing properties and null values are not distinguished. Both represent the same concept: "this property has no value."

**Core rule:** If a property is not present in a node's CBOR blob, it is null. Null values are NOT stored in CBOR — the absence of a key IS the null representation. No explicit null encoding is needed.

**Required fields:** Properties marked `required: true` cannot be null. The API layer enforces this at write time — nodes without required fields are rejected, so required fields are always present and never null in storage.

**Default values:** If a property has a `default` value, it is applied at node creation time if the field is omitted. After defaults are applied, the field is present in storage, not null. Defaults are applied only at creation time, not retroactively to old nodes.

**Null in responses:** Missing/null fields are absent from the response — not returned to the client. Client SDKs represent missing fields using language-native optional types (`Option<T>`, `undefined`, `None`, `*T`, etc.).

### Index Types

| Index Type | FDB Pattern | Supports |
|---|---|---|
| `unique` | `(ns, entities, Type, idx, field, value)` -> `id` | Equality only. Enforces uniqueness on write. |
| `equality` | `(ns, entities, Type, idx, field, value, id)` -> empty | Equality lookups. Multiple nodes per value. |
| `range` | `(ns, entities, Type, idx, field, value, id)` -> empty | Equality, >, >=, <, <=. Values must be sort-order encoded. |

**Null values are NOT indexed.** If a property is indexed and a node has no value for that property (null), no index entry is created for that node. This has two consequences:

1. **Range and equality scans exclude null nodes correctly.** A query like `age >= 30` naturally excludes nodes without an age index entry (age is null).
2. **Null equality queries require full scan.** A query like `age == null` cannot use the index (null values don't appear in it) and must use a full scan with a residual filter to check for missing fields.

### Schema Registration Rules

| Rule | Behavior |
|---|---|
| Node creation without registered type | **REJECT** |
| Edge declaration to unregistered target type | **ALLOW** (forward declaration) |
| Edge creation to unregistered target type | **REJECT** (target nodes can't exist) |
| Edge creation to non-existent target node | **REJECT** (referential integrity) |
| Undeclared edge type on a node | Depends on `allow_undeclared_edges` flag |
| Property not in schema | Depends on `extras_policy`: "reject", "allow", "warn" |

### 4.4 Schema Evolution

Schemas are client-defined state. The client sends schema definitions via gRPC, the server stores and enforces them, and the current schema is the source of truth. There is no schema lifecycle state machine (no deprecation phases, no removal states). A property is either in the schema (active) or removed.

#### Adding Properties

When a client registers a new schema version with additional properties:

- **Non-indexed property**: Immediate. No backfill required. New nodes include the property; old nodes without it return `null` when queried.
- **Indexed property**: Server enqueues a background job to backfill the index over all existing nodes of that type. Backfill scans all nodes, extracts the property value, and writes index entries. Until backfill completes, the index is incomplete (queries using it return partial results, which is acceptable—clients should not rely on new indexes until the backfill job completes).

#### Removing Properties

When a client registers a new schema version without a property:

1. **Server stops validating and accepting the property on new writes.** The property is not in the current schema, so CEL type-checking rejects any query that references it (`age > 30` → error: `"age"` not in schema).
2. **Existing nodes retain the property in CBOR.** The system does not automatically clean up old data—it remains in the serialized blob, invisible to queries and index operations.
3. **Old indexes become orphaned.** If an index existed on the removed property, index entries in FDB remain but are never used (queries can't reference the property, so they can't use the index).
4. **CEL type-check fails cleanly.** Clients attempting to query a removed property get a compile error and must rewrite their queries. No data is lost, no corruption occurs.

#### Client-Owned Cleanup

The system provides explicit RPCs for cleanup. The client decides whether and when to use them:

**`DropIndex(namespace, entity_type, property_name)` — RPC**
- Immediately deletes all index entries for the property: `(namespace, entities, Type, idx, property, *)` via FDB range delete.
- Future writes do not create entries for this property.
- Synchronous operation, no background job required.
- Old nodes still have the property in CBOR, but it's no longer indexed.

**`StripProperty(namespace, entity_type, property_name)` — RPC**
- Enqueues a background job that scans all nodes of the entity type.
- For each node: deserialize CBOR, remove the property, re-serialize, and write back.
- Reclaims storage occupied by the removed property.
- Optional and asynchronous—the client requests it, but the system works in the background.
- Idempotent—re-running the job on a node that has already been stripped is safe.

#### No Deprecation State Machine

Unlike traditional deprecation models (Active → Deprecated → Removed), this design has no intermediate states. A property is either in the schema (active) or not. There is:
- No minimum deprecation window enforced by the system.
- No automatic lifecycle transitions.
- No warnings for removed properties—only type-check errors.

If a client needs a grace period before removing a property, they manage it themselves by keeping the property in the schema until they're ready.

#### Multi-Site Schema Compatibility

CDC entries include only the properties that are in the local schema. When a receiving site replicates a CDC entry:
1. Load the local schema for the entity type
2. Deserialize the CDC entry
3. **Strip any fields not in the local schema** (they're silently ignored)
4. Apply the operation

Sites with different schema versions converge gracefully. Old CDC entries with removed fields work fine—the receiving site simply ignores the extra fields. New CDC entries generated after a schema change don't include removed fields.

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
    ┌────────▼────────────────────────┐
    │  6. Plan Selection (Cost-Based)  │
    │                                  │  1. Identify indexed vs unindexed preds
    │                                  │  2. Load per-field statistics from FDB
    │                                  │  3. Estimate selectivity of each pred:
    │                                  │     - eq: total_nodes / distinct_values
    │                                  │     - range: with/without histogram
    │                                  │     - fallback heuristics if no stats
    │                                  │  4. Decide execution strategy:
    │                                  │     - Check for composite index match
    │                                  │     - Single-best: if one pred << others
    │                                  │     - Merge-join: if similar selectivity
    │                                  │     - Full scan: if very few indexed preds
    │                                  │  5. Remaining predicates → residual filter
    └────────┬────────────────────────┘
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
        estimated_cardinality: f64,
        cost: f64,
    },
    RangeScan {
        index_field: String,
        lower: Option<Bound>,
        upper: Option<Bound>,
        estimated_cardinality: f64,
        cost: f64,
    },
    Intersection {
        plans: Vec<QueryPlan>,  // AND: merge-join sorted result sets
        estimated_cardinality: f64,
        cost: f64,
    },
    Union {
        plans: Vec<QueryPlan>,  // OR: merge-sort deduplicated result sets
        estimated_cardinality: f64,
        cost: f64,
    },
    Composite {
        fields: Vec<String>,    // composite index on these fields
        filters: Vec<String>,   // CEL filters per field
        estimated_cardinality: f64,
        cost: f64,
    },
    FullScan {
        entity_type: String,    // worst case: scan all nodes of type
        estimated_cardinality: f64,
        cost: f64,
    },
}

struct ExecutionPlan {
    primary: QueryPlan,                // drives the main scan/lookup
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

### CEL Null Handling Rules

All comparison operators and string operations have defined behavior when a field is null (missing):

| CEL Expression | Field is Null | Result | Rationale |
|---|---|---|---|
| `field > value` | yes | `false` | Null value does not satisfy any comparison |
| `field >= value` | yes | `false` | Null value does not satisfy any comparison |
| `field < value` | yes | `false` | Null value does not satisfy any comparison |
| `field <= value` | yes | `false` | Null value does not satisfy any comparison |
| `field == value` | yes | `false` | Null equals only null, not any value |
| `field == null` | yes | `true` | This is how to query for missing fields |
| `field != value` | yes | `false` | Null is not equal to value, so negation is false |
| `field != null` | yes | `false` | Null is equal to null, so inequality is false |
| `field.startsWith(prefix)` | yes | `false` | String operation on null returns false |
| `field.contains(substr)` | yes | `false` | String operation on null returns false |

**Null in boolean logic:** Null is treated as falsy in AND/OR expressions:
- `null && true` → `false` (null is falsy)
- `null || true` → `true` (OR with true)
- `null || false` → `false` (null is falsy)

**Key insight:** `field == null` and `field != null` are the only operators that produce useful results when a field is null. All other comparisons return `false`, which correctly excludes null nodes from range queries.

### AND/OR Handling with Index Intersection Strategy

| Expression Pattern | Baseline Strategy | Cost Model Decision |
|---|---|---|
| `A && B` (both indexed, A << B selectivity) | Execute A (most selective), residual-filter B on candidates | Single-index dominant: cost(A scan + B residual) << cost(merge-join) |
| `A && B` (both indexed, A ≈ B selectivity) | Merge-join: scan both indexes in parallel, intersect sorted ID lists | Multi-index intersection: cost(A scan + B scan + merge) < cost(single-best + residual) |
| `A && B` (one indexed) | Execute indexed predicate, apply non-indexed as residual | Only option |
| `A && B` (neither indexed) | Full scan with combined residual filter | Only option |
| `A && B && C` (multiple indexed) | Check for composite index match first; if not: apply cost model to top 2 by selectivity, residual-filter others | Composite preferred; else apply cost model iteratively |
| `A \|\| B` (both indexed) | Execute both indexes, union results, deduplicate | Merge sort union with O(N+M) complexity; execute in parallel if feasible |
| `A \|\| B` (one indexed) | Full scan with combined residual (can't avoid it) | Only option |
| No indexed predicates | Full scan with all predicates as residual | Only option |

### Query Statistics Maintenance (Section 5.3)

The query planner uses per-field statistics maintained by a background job to estimate selectivity and choose optimal execution strategies. Statistics are eventually consistent; stale statistics produce suboptimal but correct plans (acceptable; same approach as Postgres ANALYZE).

#### What Statistics Are Maintained

Per-entity-type and per-indexed-field:
- **total_nodes**: Count of all nodes for the entity type
- **index_entries**: Count of entries in the index (may be > total for non-unique indexes)
- **distinct_values**: Count of distinct values in the index (used for equality selectivity: `total_nodes / distinct_values`)
- **histogram**: Optional value distribution buckets for range predicates (advanced; heuristic suffices for MVP)
- **last_updated**: Timestamp of last statistics computation

#### How Statistics Are Collected

**Background job (eventually consistent):**
- Periodic job (e.g., hourly) runs per entity type
- Samples N random nodes (e.g., 1000) to compute distinct value count
- Counts total index entries per field
- Stores result in FDB metadata subspace: `(_meta, _stats, entity_type, field) → statistics_blob`
- No impact on write path; async updates don't block mutations
- Stale statistics produce suboptimal but correct plans

**Query planner caching:**
- Planner loads statistics at planning time from FDB metadata
- In-memory cache per statistics key with TTL (e.g., 5 minutes)
- Cache invalidates on schema change
- Falls back to hardcoded heuristics if statistics unavailable

#### Statistics Usage in Selectivity Estimation

**With statistics (accurate):**
```
Equality predicate (e.g., department == 'rendering'):
  estimated_rows = total_nodes / distinct_values

Range predicate (e.g., age >= 30):
  estimated_rows = total_nodes * 0.5  (or use histogram for accuracy)

Decision: Compare estimated cardinality across indexed predicates.
  If max_cardinality / min_cardinality >= 5.0: single-best index is dominant
  Else: merge-join intersection may be cheaper than single-best + residual
```

**Without statistics (fallback heuristics):**
```
Equality: assume 10% selectivity
Range: assume 50% selectivity
Timestamp range: assume 30% selectivity
```

#### Cost Model Decision

The planner estimates total cost for each viable execution strategy:

**Single-index + residual:**
- Cost = (1 FDB scan + candidate_count * residual_filter_cost)
- Residual filter cost ≈ 0.01 CPU units per candidate

**Merge-join (two indexes):**
- Cost = (FDB scan index 1 + FDB scan index 2 + merge_cost)
- Merge cost ≈ 0.001 CPU units per pointer comparison (very fast)

**Full scan:**
- Cost = (total_nodes / FDB_batch_size) + (total_nodes * residual_filter_cost)

**Decision:** Choose strategy with minimum estimated cost.

Example:
```
Query: age >= 30 AND salary >= 100000
Selectivity estimates: age=8000 rows, salary=2000 rows
Ratio: 8000/2000 = 4 < 5.0 → consider merge-join

Cost(single-best on salary): 1 scan (2000) + 2000 residual filters ≈ 2020
Cost(merge-join): 1 scan (8000) + 1 scan (2000) + 10000 merge ops ≈ 10010

Decision: Single-best is cheaper. Scan salary index, residual-filter age.
```

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
    (_jobs)
      ({job_type}, {job_id})                      → Job state bytes (progress cursor, status)
    (_subscriptions)
      ({subscription_id})                         → Subscription metadata (informational only)
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

Background job:
  ("myapp", "_jobs", "index_backfill", "Person_age_20260208_001")
  → CBOR { status: "running", progress_cursor: [...], entity_type: "Person", field: "age" }

Subscription (informational):
  ("myapp", "_subscriptions", "query_sub_12345")
  → CBOR { entity_type: "Person", cel_expression: "age >= 30", created_at: ... }

Schema:
  ("myapp", "_meta", "schemas", "Person")
  → CBOR { version: 1, properties: {...}, edges: {...} }
```

---

## 7. CDC + Background Jobs Architecture

### Design Rationale

**FDB is the WAL.** A single FDB transaction is atomic — if the process crashes before commit, nothing is written. FDB's internal WAL handles durability. There is no need for an application-level write-ahead log or coordination layer.

This means every operation — whether a single node create or chunk 37 of a bulk import — follows the same write path:

- Validate → single FDB transaction (data + indexes + CDC entry) → ack.
- Multi-transaction operations are sequences of independent transactions, each self-contained.
- If a crash occurs mid-sequence, completed transactions are committed facts in FDB with CDC entries already emitted. Background workers resume from the next logical step. Idempotent and crash-safe.

This gives us:
- One write path for all operations (single-node creates and bulk imports use the same machinery)
- Every committed change is immediately visible in CDC — no hidden intermediate state
- CDC contains only committed facts, which simplifies replication consumers

### Write Paths

#### All Mutations (Single-Transaction Path)

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
│ CDC Consumers (background)   │
│  Reads new CDC entries       │
│  Projects to RocksDB cache   │
│  Updates replication stream  │
│  Dispatches watch events     │
└─────────────────────────────┘
```

This is one round-trip to FDB. The CDC entry is marginal cost — one additional KV pair in an already-open transaction. The client gets immediate read-after-write consistency because data and indexes are written atomically.

#### Bulk Import: Chunked Mutations

```
Client sends batch of mutations (one mutation per chunk)
    │
    ▼
For each chunk:
  ├─ Single FDB Transaction:
  │   1. Write node/edge data
  │   2. Write index entries
  │   3. Append CDC entry (with optional batch_id tag)
  │   ACK to client
  │
If chunk N fails, client:
  ├─ Handles error (retry, log, etc.)
  ├─ Resumes from chunk N+1 on next batch send
  │
Chunks 1..N are committed, CDC'd, and replicated.
No special "bulk import" machinery needed.
```

This is the unified write path applied repeatedly. Optional `batch_id` tag on CDC entries allows client-side tracking of which chunks belong together. If the client crashes at chunk 30/50, chunks 1-30 are committed and visible. The client resumes from chunk 31. No re-application of already-committed chunks.

### Background Jobs

Long-running scans (index backfill, orphaned edge cleanup, etc.) are background jobs — a progress cursor in FDB and a worker loop.

```rust
struct BackgroundJob {
    job_id: String,
    job_type: JobType,
    status: JobStatus,           // Pending, Running, Completed, Failed
    progress_cursor: Vec<u8>,    // opaque cursor into the scan space
    created_at: i64,
    last_updated: i64,
    error: Option<String>,
}

enum JobType {
    IndexBackfill {
        entity_type: String,
        field: String,
        index_type: String,
    },
    OrphanedEdgeCleanup {
        dropped_entity_type: String,
    },
    // future: CDC compaction, cache rebuild, etc.
}

enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
}
```

Worker loop pseudocode:

```rust
loop {
    let job = find_next_pending_or_running_job().await;
    let cursor = job.progress_cursor.clone();
    let batch = read_batch_from(cursor, BATCH_SIZE).await;

    if batch.is_empty() {
        mark_job_completed(job.job_id).await;
        break;
    }

    // Single FDB transaction: process batch
    process_batch(&batch).await;

    // Advance cursor and commit
    advance_cursor(&job.job_id, &batch.last_key()).await;

    // If crash here or after: resume from cursor on next iteration
    // Idempotent — re-processing a batch is safe
}
```

Key properties:
- **Progress cursor**: Opaque byte string. For entity type scans, it's the last seen entity ID or a range key. Worker reads from cursor to cursor+limit, processes, updates cursor, commits. No re-application of already-processed batch.
- **Idempotence**: Each batch is processed in isolation. If a crash occurs mid-batch, the process resumes from the cursor, reading the batch again. The work is idempotent — applying the batch twice produces the same result as once (or at least, no data corruption).
- **No compensation**: If a job fails, it is marked Failed and an operator investigates. No automatic rollback. (Rollback is the client's problem for bulk import — they can delete nodes by batch_id and retry.)
- **Crash safety**: On restart, the worker finds incomplete jobs by scanning `_jobs` for status = Running or Pending. It resumes from the progress cursor.

### CDC Entry Schema

```rust
struct CdcEntry {
    // Key: (ns, "_cdc", versionstamp) — server-assigned, zero contention
    site: String,
    namespace: String,
    timestamp: i64,
    batch_id: Option<String>,  // Optional: for client tracking (e.g., bulk import batch 30/50)
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

### CDC Consumers

| Consumer | Purpose | Latency Tolerance |
|---|---|---|
| RocksDB Projector | Cache population, materialized views | Sub-second |
| Replication Streamer | Send to remote sites | Seconds (network-bound) |
| Watch Dispatcher | Filter CDC entries, dispatch to query subscriptions | Sub-second |
| Audit Logger | External audit trail | Seconds |

### Versionstamp Ordering

CDC entries use FDB versionstamps as keys. Key properties:

- **Zero contention**: Server-assigned at commit time. No application-managed counter. No hot-key bottleneck.
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
| Hot node data | Projected from CDC (proactive warming) or read-through on cache miss (reactive) | LRU with size cap |
| Materialized traversal results | Precomputed for common patterns | TTL-based |
| Temporal indexes (e.g., "modified in last hour") | Projected from CDC | Time-window expiry |
| Aggregation caches (counts, sums) | Maintained by CDC consumer | Invalidated by CDC |

**Dual-population strategy:**
- CDC projector proactively warms the cache from CDC entries (every write eventually reaches RocksDB)
- Read-through cache on miss (reactive): a cache miss on a node triggers an FDB read and populates RocksDB

**Configurable:** Both strategies should be supported. Deployments can choose:
- Projector only (higher CDC consumer load, but all cached data is warm)
- Read-through only (lower CDC load, but popular nodes warm up gradually)
- Both (balance between proactive and reactive)

Note: Evaluate whether the added complexity of a CDC-driven projector is worth the warm cache vs pure read-through. The read-through strategy alone may be sufficient for most workloads.

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

The CDC high-water mark is stored in FDB (source of truth), not in RocksDB. This prevents stale-data-served-as-fresh bugs if RocksDB crashes and recovers to an older checkpoint.

---

## 8. Watch/Reactive Subscriptions Architecture

### Point Watches (FDB Native)

Point watches use FDB's transactional watch mechanism for individual nodes or edge sets. These are high-fidelity, low-latency subscriptions.

```
User registers: WatchNode(type="Person", id="p_42")
    │
    ▼
Server checks watch registry:
    │
    ├── [First watcher] Create new FDB watch on data key
    │   ("myapp", "entities", "Person", "data", "p_42")
    │   Add client to subscribers list
    │
    └── [Additional watchers] Add to existing subscribers list
              (share the one FDB watch, multiple clients)
    │
    ▼
When FDB watch fires (node is modified):
    │
    ├── Server reads new node state
    │
    ├── Fan-out to all clients watching this key
    │   (each gets a server-push on its gRPC stream)
    │
    └── Re-subscribe watch for next change
              (loop continues)
```

**Performance:**
- Sub-millisecond latency: FDB watch fires immediately upon commit
- Scalability: Multiple clients watching same key share one FDB watch + in-memory fan-out
- Crash safety: On server restart, client reconnects and re-subscribes

**Implementation notes:**
- Watch registry: in-memory map `Map<WatchKey, Vec<ClientId>>` + tokio::sync channels for fan-out
- Watch re-subscription: automatic on FDB transaction reset
- Watch key: `(namespace, entity_type, node_id)` for point watches; `(namespace, entity_type, node_id, edge_type, direction)` for edge watches

### Query Watches (CDC-Filtered)

Query watches subscribe to a result set defined by a CEL predicate. The CDC consumer evaluates each entry against active subscriptions and sends deltas.

```
User registers: WatchQuery(entity_type="Person", cel="age >= 30 && active")
    │
    ▼
Server registers subscription in memory:
    ("query_sub_12345", "Person", compiled_cel_expr)
    │
    ▼
[Initial snapshot]
Server queries current matching nodes:
    FindNodes(entity_type="Person", cel="age >= 30 && active")
    Stream results to client
    │
    ▼
CDC consumer loop:
    │
    ├── Read CDC entry
    │
    ├── Iterate active subscriptions for entity_type="Person"
    │
    ├── Evaluate entry against each subscription's CEL
    │
    └── Send appropriate event to subscribed client:
        • NodeEntered: node now matches (was not in previous snapshot)
        • NodeLeft: node no longer matches
        • NodeUpdated: node still matches, properties changed
```

**Performance:**
- Bounded by CDC consumer lag (typically sub-second)
- At 10K active subscriptions with complex CEL, the watch dispatcher becomes a bottleneck
- Future optimization: subscription indexing (bloom filter on entity_type + changed fields)

**Consistency on reconnect:**
- Option A: Always send full snapshot on reconnect (simpler, higher bandwidth)
- Option B: Server tracks per-subscription high-water mark in FDB; replay CDC from there (efficient but complex)
- Recommend starting with A; migrate to B if needed

**Server-local:**
- Subscriptions are not replicated across sites
- On failover, client reconnects and re-registers subscription

### Watch Registry

```rust
struct WatchRegistry {
    // Point watches: shared FDB watches + fan-out
    point_watches: Map<WatchKey, PointWatchState>,

    // Query watches: subscription metadata
    query_subscriptions: Map<SubscriptionId, QuerySubscription>,
}

struct PointWatchState {
    fdb_watch: FdbWatch,                    // single FDB watch per key
    subscribers: Vec<ClientId>,             // clients watching this key
    broadcast_tx: tokio::sync::broadcast,   // send updates to all subscribers
}

struct QuerySubscription {
    subscription_id: String,
    entity_type: String,
    compiled_cel: CompiledCel,
    client_id: ClientId,
    initial_snapshot_sent: bool,
}
```

---

## 9. High-Throughput Read Design

### Consistency Levels

```rust
enum ReadConsistency {
    Strong,
    Session,
    Eventual,
}
```

**Strong:** Direct FDB read
- Serializable isolation
- Latest committed state
- Use case: critical updates, ownership verification
- Latency: network round-trip to FDB

**Session:** RocksDB cache verified against FDB read version
- Bounded staleness: cache is valid if its high-water mark (CDC versionstamp) >= the FDB read version at request time
- Use case: most queries
- Latency: RocksDB hit; or FDB read if stale
- Default level

**Eventual:** RocksDB cache only
- Fastest
- May be stale by CDC consumer lag (typically < 1 second)
- Use case: analytics, non-critical reads
- Latency: RocksDB hit; miss falls back to FDB

API:
```protobuf
message FindNodesRequest {
    string entity_type = 1;
    string cel_expression = 2;
    ReadConsistency consistency = 3;  // optional, default: Session
}

enum ReadConsistency {
    STRONG = 0;
    SESSION = 1;
    EVENTUAL = 2;
}
```

Server enforces: client can request Stronger but not weaker than configured policy. (Config: `max_read_consistency = Session` means client cannot use Eventual.)

### Read Coalescing

When multiple concurrent requests for the same uncached node arrive, coalesce into a single FDB read.

```
Request 1: GetNode(type="Person", id="p_42")  [cache miss]
    ├─ Check in-flight map
    ├─ [Not found] Start FDB read, register in in-flight map
    └─ Broadcast result to Request 1

Request 2: GetNode(type="Person", id="p_42")  [arrives while Request 1 in-flight]
    ├─ Check in-flight map
    ├─ [Found] Subscribe to broadcast channel (tokio::sync::broadcast)
    └─ Wait for result from Request 1's FDB read

Result arrives from FDB
    │
    ├─ Send to Request 1
    ├─ Send to Request 2 (via broadcast)
    └─ Remove from in-flight map

Both requests satisfied by single FDB read.
```

**Implementation:**
```rust
let mut in_flight: Map<NodeKey, broadcast::Sender> = Map::new();

// Request arrives
let key = (entity_type, node_id);
let mut sender = in_flight.entry(key).or_insert_with(|| {
    let (tx, rx) = broadcast::channel(10);
    // Spawn FDB read task
    tokio::spawn(async move {
        let node = fdb.get(&key).await;
        let _ = tx.send(node);  // broadcast to all waiters
    });
    tx
});

let mut rx = sender.subscribe();
let node = rx.recv().await;
```

Prevents thundering herd on popular nodes (e.g., lookups by unique email).

### Traversal Limits

TraverseRequest has mandatory limits, server-enforced.

```protobuf
message TraverseRequest {
    NodeRef start = 1;
    repeated TraversalStep steps = 2;
    uint32 max_depth = 3;        // mandatory, default: 4, cannot exceed server max
    uint32 timeout_ms = 4;       // mandatory, default: 5000ms
    uint32 max_results = 5;      // mandatory, default: 10000
}
```

Server-enforced defaults: `max_depth=4, timeout_ms=5000ms, max_results=10000`. Client can specify lower limits, but cannot exceed server maximums.

**Enforcement:**
- Depth check: on each hop, increment depth counter; fail if depth > max_depth
- Timeout: set tokio::time::timeout on the traversal task
- Results: stop streaming when result count >= max_results; return partial result with cursor

### EXPLAIN Endpoint (Query Plan Inspection)

Clients can inspect the query plan without executing it, useful for debugging slow queries and understanding index selection decisions.

```protobuf
message ExplainRequest {
    string entity_type = 1;
    string cel_expression = 2;
}

message ExplainResponse {
    QueryPlan plan = 1;
    repeated string explanation_steps = 2;  // human-readable breakdown
    double estimated_cost = 3;
    double estimated_cardinality = 4;
}

message QueryPlan {
    string strategy = 1;  // e.g., "point_lookup", "range_scan", "merge_join", "full_scan", "composite"
    repeated IndexOperation index_operations = 2;
    string residual_filter = 3;  // CEL for non-indexed predicates
    double estimated_cost = 4;
    double estimated_cardinality = 5;
}

message IndexOperation {
    string field = 1;
    string operator = 2;  // e.g., "==", ">=", ">", "<", "<="
    string value = 3;
    double estimated_rows = 4;
}
```

**Example response:**
```json
{
  "plan": {
    "strategy": "single_best_index",
    "index_operations": [
      {
        "field": "salary",
        "operator": ">=",
        "value": "100000",
        "estimated_rows": 2000
      }
    ],
    "residual_filter": "age >= 30",
    "estimated_cost": 2020,
    "estimated_cardinality": 1500
  },
  "explanation_steps": [
    "Both age and salary have range indexes",
    "Selectivity estimates: age >= 30 → 8000 rows, salary >= 100000 → 2000 rows",
    "Ratio: 8000/2000 = 4, single-best index is preferred",
    "Cost model: single-best (2020) < merge-join (10010)",
    "Strategy: scan salary index (most selective: 2000 rows), residual-filter age on candidates"
  ],
  "estimated_cost": 2020,
  "estimated_cardinality": 1500
}
```

---

## 10. Replication Model

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

Because CDC entries only contain committed facts, the receiving site never encounters partial or pending operations.

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

This dependency buffering is strictly for cross-site node references where the target node hasn't been replicated yet.

### Conflict Scenarios

| Scenario | Resolution |
|---|---|
| Two sites modify same owned node | Can't happen — only home site can modify |
| Two sites create edge to same target | Allowed — edges have unique IDs, no conflict |
| Ownership transfer race | Explicit protocol: request → ack → transfer. Only one in-flight transfer per node. |
| Schema registered differently at two sites | Schema changes replicate via CDC. Last-writer-wins by versionstamp. Recommend: single schema-authority site. |

---

## 11. Namespace Hierarchy

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
| Drop entity type | `clearRange(ns, entities, Type, ...)` | All data, indexes, outgoing edges for that type. Incoming edges from other types become orphaned. Uses background job for large types. |
| Drop namespace | `clearRange(ns, ...)` | Everything: entities, indexes, edges, CDC, schema |
| Drop tenant | `clearRange(tenant, ...)` | All namespaces under tenant |
| Purge orphaned edges | Background job: scan edge subspaces, check target existence | Triggered after entity type drop |

---

## 12. Gap Analysis

Analysis of missing pieces, inconsistencies, and unresolved design problems. Categorized by severity.

### CRITICAL: Must Resolve Before Implementation

~~**C1. Index intersection has no execution strategy.**~~
**RESOLVED in v0.3 (updated).** Complete strategy defined: cost-based selectivity estimation using background-maintained statistics; single-best index + residual as baseline; merge-join intersection for multi-index AND queries with similar selectivity; composite indexes as preferred path for known patterns; EXPLAIN endpoint for query plan inspection. See Section 5.3 (Query Statistics Maintenance), Section 5 (Plan Selection and AND/OR Handling), and API section (EXPLAIN endpoint).

~~**C3. Bidirectional edges have ambiguous ownership.**~~
**RESOLVED in v0.3.** Bidirectional edges are implemented as two paired directed edges with a shared `pair_id`. Each direction is owned by its source node's home site (unambiguous ownership). The schema declares `direction: "bidirectional"` but the engine implements it as paired directed edges — one outgoing from each endpoint. This eliminates the ownership ambiguity: the edge from A→B is owned by A's home site, and the edge from B→A is owned by B's home site. See companion document: Edge Management Spec.

~~**C4. No query timeout or traversal depth limits.**~~
**RESOLVED in v0.3.** TraverseRequest has mandatory `max_depth`, `timeout_ms`, and `max_results` with server-enforced defaults. See Section 9.

~~**C5. Orphaned incoming edges have efficiency concern.**~~
**RESOLVED in v0.3.** Single node deletion scans IN entries at the deleted node to find all incoming edges, then deletes corresponding OUT entries at source nodes — O(incoming edges). For entity type drops: lazy cleanup (orphaned edges detected and cleaned during traversal) plus a background job that scans other entity types' edge subspaces post-drop. No reverse type index is needed — entity type drops are rare operational events and the write amplification cost is not justified for every edge creation. See companion document: Edge Management Spec.

~~**C6. Property removal is undefined behavior.**~~
**RESOLVED in v0.3.** When a property is removed from the schema: (1) server stops validating/accepting it on new writes; (2) old nodes keep it in CBOR (invisible to queries); (3) old indexes become orphaned (client drops them explicitly with DropIndex RPC); (4) client optionally strips old data with StripProperty RPC (background job). No deprecation state machine—schema IS the current state. See Section 4.4 (Schema Evolution).

~~**C7. Multi-site unique constraint violation.**~~
**RESOLVED in v0.3.** The unique constraint is "best-effort" in a multi-site deployment. The system cannot guarantee global uniqueness in the presence of network partitions between data centers — routing writes to a single authority site fails when that site is unreachable. Resolution strategy: **first-come-first-serve with OBSOLETE marking.** Both sites can create nodes with the same unique-indexed value independently. On CDC replication, when a receiving site detects a unique constraint conflict: the node that arrived FIRST at that site (already committed locally) wins; the conflicting incoming node is marked with `_status: "obsolete"` in CDC. OBSOLETE nodes still exist in storage and are queryable with an explicit `include_obsolete: true` flag, but are excluded from normal queries, cannot be used as edge targets, and cannot be updated (mutations rejected with `ERR_NODE_OBSOLETE`). Existing edges to/from the obsolete node remain valid but the node is effectively frozen. An operator can manually resolve by merging data from obsolete into winner, then deleting obsolete. CDC entry for the obsolete marking is emitted so all sites converge. Note: "first" is determined by local arrival order at each site, which may differ during the conflict window. Sites converge once CDC replication catches up. See companion document: Edge Management Spec.

### IMPORTANT: Should Resolve Before Phase 2

~~**I1. CEL type system doesn't handle NULL / missing values.**~~
**RESOLVED in v0.3.** Missing and null are one concept. If a property is not in CBOR, it is null. Null values are falsy, don't satisfy comparisons (except `== null` and `!= null`), and are not indexed. Required fields are validated at write time; optional fields may be null. Defaults are applied at creation time. See Section 4 (Null Semantics) and Section 5 (CEL Null Handling Rules).

**I3. Polymorphic edge traversal has no indexing strategy.**
Traversing `TAGGED_WITH -> *` with a filter on the target node requires loading each target, checking its type, loading its schema, and evaluating the filter. No index optimization is possible because the target type is unknown. For high-degree polymorphic edges, this is O(edge count). Need: either accept this limitation explicitly or add a polymorphic index strategy.

**I4. No transactionality guarantee across multi-step queries.**
A FindNodes query that does an index scan followed by data fetches is not atomic. Concurrent mutations can produce inconsistent results (node exists in index but deleted from data store between the two reads). Need: explicit consistency level in the API (snapshot reads via FDB read versions, or accept eventual consistency).

**I5. Cursor pagination is undefined for complex queries.**
FindNodes pagination needs a cursor that encodes: the last index position scanned, the scan direction, and enough state to resume. Traversal pagination is harder — state includes depth, path, and per-hop position. The spec defines no cursor format.

~~**I6. No EXPLAIN / query plan inspection.**~~
**RESOLVED in v0.3 (updated).** EXPLAIN endpoint defined: takes entity_type and CEL expression, returns QueryPlan with estimated cost and cardinality without executing. QueryPlan enum updated with cost annotations (estimated_cardinality, cost fields on all variants). Clients can debug slow queries. See Section 5 (QueryPlan Rust enum with cost annotations) and API section (EXPLAIN endpoint).

**I7. Watch subscription scalability.**
Point watches use FDB txn.watch() which has per-transaction limits. Query watches require CDC consumer to evaluate every entry against every active subscription. At 10K active subscriptions with complex CEL expressions, the watch dispatcher becomes a bottleneck. Need: subscription indexing (e.g., bloom filter on entity_type + changed fields) to skip non-matching subscriptions cheaply.

**I8. Query watch consistency on reconnect.**
When a client reconnects after disconnect, the server needs to provide a delta since the client's last-seen state. Options: (a) always send full snapshot on reconnect, (b) server tracks per-subscription high-water mark in FDB and replays CDC from there. (a) is simpler, (b) is more efficient for large result sets.

**I9. Bulk import partial failure semantics.**
If client crashes mid-import, committed chunks are visible and replicated. This is a feature, not a bug — but clients must handle partial imports. The API should provide a way to query "all nodes with batch_id X" for cleanup if needed.

### MINOR: Address During Implementation

**M1. ID allocation strategy is undecided.**
UUIDs (16 bytes, no coordination) vs sequential int64 (smaller, requires counter). Affects key size, sort locality, and write throughput. Recommend: ULID or UUIDv7 (time-ordered UUIDs) — gets you temporal locality without coordination.

**M2. Schema version management is incomplete.**
No definition of when versions increment, how clients discover the current version, or how old schema versions interact with new data.

**M3. Edge ID generation is unspecified.**
Globally unique? Per-source-node? Server-assigned? Affects CDC entries, replication, and deduplication.

~~**M4. No specification for bulk import.**~~
**RESOLVED in v0.3.** Bulk import is chunked mutations through the unified write path. Each chunk is a normal FDB transaction with its own CDC entry. No server-side coordination needed. Optional `batch_id` tag for client-side tracking. See Section 7.

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

## 13. Implementation Phases

### Phase 1: Storage Foundation (MVP)
- FDB subspace layout and tuple encoding
- Node CRUD with schema validation
- Property indexes (unique, equality, range)
- Edge CRUD with referential integrity
- CEL compilation and basic query planning (single-predicate)
- gRPC API with streaming FindNodes
- CDC stream with versionstamped keys (single-txn write path)
- Background job framework (_jobs subspace, worker loop)

### Phase 2: Query Engine + High-Throughput Reads
- Multi-predicate CEL decomposition
- Index intersection and union strategies
- Multi-hop traversal with per-hop filtering
- Configurable consistency levels (strong/session/eventual)
- Read coalescing for hot nodes
- RocksDB cache layer with CDC projection and read-through
- Traversal depth/timeout/result limits
- Cursor-based pagination

### Phase 3: Reactive Subscriptions + Replication
- Point watches (FDB txn.watch) for individual nodes/edges
- Watch registry with fan-out
- Query watches via CDC consumer filtering
- CDC consumers and consumer framework
- Single-site ownership model
- CDC streaming between sites
- Causal ordering and dependency buffer
- Ownership transfer protocol

### Phase 4: Operational Maturity
- Schema migration tooling
- Observability and metrics
- Backup/restore integration
- Resource limits and quotas
- Performance benchmarking and tuning

---

*This is a living document. Architecture decisions captured in Apple Notes: "Graph DB - Architecture Decisions", "Graph DB - Schema Enforcement Rules", "Graph DB - Ownership + Replication Model", "Graph DB - API + Wire Protocol Design".*
