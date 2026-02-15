# JanusGraph Architecture Deep Dive
## How a Graph Database Works on Top of FoundationDB / Cassandra

---

## The Big Picture

JanusGraph doesn't store graphs in some magical native format. It decomposes graph structures (vertices, edges, properties) into rows of key-value pairs and stores them in a **wide-column / ordered key-value backend**. The entire system is a set of layered abstractions that translate graph semantics into storage operations.

```
                    ┌──────────────────────────────┐
                    │     Gremlin Query Language     │
                    │       (TinkerPop API)          │
                    └──────────────┬───────────────┘
                                   │
                    ┌──────────────▼───────────────┐
                    │   TraversalStrategy Engine     │
                    │   (Query Optimization)         │
                    └──────────────┬───────────────┘
                                   │
                    ┌──────────────▼───────────────┐
                    │    StandardJanusGraphTx        │
                    │  (Transaction + Mutation Log)  │
                    ├───────────────────────────────┤
                    │  L1 Cache: Vertex Cache        │
                    │  L1 Cache: Index Result Cache  │
                    └──────────────┬───────────────┘
                                   │
                    ┌──────────────▼───────────────┐
                    │    Backend Orchestrator        │
                    │  (KeyColumnValueStoreManager)  │
                    └──────┬───────────┬───────────┘
                           │           │
              ┌────────────▼──┐  ┌─────▼──────────┐
              │  Edge Store   │  │  Index Backend  │
              │  (graph data) │  │ (ES/Solr/Lucene)│
              └───────┬───────┘  └────────────────┘
                      │
        ┌─────────────▼──────────────┐
        │  KeyColumnValueStore       │
        │  (Storage Abstraction)     │
        └─────┬──────┬──────┬───────┘
              │      │      │
         ┌────▼┐ ┌───▼──┐ ┌▼──────────┐
         │ C*  │ │HBase │ │FoundationDB│
         └─────┘ └──────┘ └───────────┘
```

---

## Layer 1: Storage Backend Abstraction

### The Core Interface: `KeyColumnValueStore`

Everything bottoms out at one interface: **KeyColumnValueStore**. This models a table where:

- **Row Key** = a byte array (the vertex ID)
- **Column** = a byte array (edge/property identifier)
- **Value** = a byte array (edge/property data)

Rows are ordered. Columns within a row are ordered. This is the Bigtable model.

The **`KeyColumnValueStoreManager`** manages the lifecycle of multiple named stores:

| Store Name | Purpose |
|---|---|
| `edgestore` | All vertex adjacency lists (edges + properties) |
| `graphindex` | Global graph index entries |
| `system_properties` | Schema definitions and system metadata |
| `txlog` | Transaction write-ahead log |
| `janusgraph_ids` | ID block allocation tracking |

### How Backends Map to This

**Cassandra**: Each store becomes a CQL table. Row key = partition key. Columns = clustering keys. Natural fit since Cassandra IS a wide-column store.

**FoundationDB**: Each store becomes a **subspace** (a key prefix namespace). The wide-column model is simulated by composing tuple keys:

```
FDB Key: (subspace_prefix, row_key, column_key) → value

Example:
  ("edgestore", vertex_42, edge_column_bytes) → edge_data
```

Because FoundationDB keys are globally ordered, a range scan on `("edgestore", vertex_42, *)` retrieves the entire adjacency list for vertex 42 in one operation. This is functionally identical to scanning a Cassandra wide row.

**Key difference**: FoundationDB gives you strict serializability (true ACID) out of the box. Cassandra gives you tunable consistency (usually eventual). This matters for the transaction layer.

---

## Layer 2: Data Model — How Graphs Become Bytes

This is the core intellectual insight of JanusGraph's design.

### The Adjacency List Model

Every vertex gets one wide row. That row contains ALL of the vertex's edges and properties as individual column-value pairs.

```
Vertex 42's Row:
┌──────────────────────────────────────────────────────────┐
│ Row Key: [vertex_42_id]                                  │
├────────────────────────┬─────────────────────────────────┤
│ Column                 │ Value                           │
├────────────────────────┼─────────────────────────────────┤
│ [KNOWS|OUT|sort|v99|e1]│ {since: 2020, weight: 0.8}     │
│ [KNOWS|OUT|sort|v107|e3│ {since: 2021}                   │
│ [LIKES|OUT|sort|v55|e7]│ {rating: 5}                     │
│ [KNOWS|IN|sort|v12|e2] │ {since: 2019}                   │
│ [prop:name]            │ "Andrew"                        │
│ [prop:age]             │ 38                              │
└────────────────────────┴─────────────────────────────────┘
```

**Critical detail**: Every edge is stored TWICE. If vertex A -KNOWS-> vertex B, there's a column in A's row (outgoing) AND a column in B's row (incoming). This doubles write cost but makes traversal in either direction a single row read.

### Column Key Byte Encoding

The column key is a carefully designed byte sequence that preserves sort order:

```
┌───────────────┬──────────────┬──────────────────┬──────────┐
│ Edge Label ID │ Sort Key     │ Adjacent Vertex   │ Edge ID  │
│ + Direction   │ Properties   │ ID (delta-encoded)│          │
│ (varint)      │ (if defined) │ (varint)          │ (varint) │
└───────────────┴──────────────┴──────────────────┴──────────┘
```

**Edge Label ID + Direction**: The label's numeric ID is varint-encoded. The **least significant bit** stores direction (0 = outgoing, 1 = incoming). So label ID 5 outgoing = byte value 10, label ID 5 incoming = byte value 11.

**Sort Key Properties**: If you defined a sort key on the edge label (e.g., sort KNOWS edges by `since` timestamp), those property values are encoded here. This means the storage backend's native byte ordering gives you pre-sorted edges for free.

**Adjacent Vertex ID (Delta Encoding)**: Instead of storing the absolute 64-bit vertex ID, JanusGraph stores the **difference** between the adjacent vertex ID and the current vertex ID. Why? JanusGraph's ID allocator assigns nearby IDs to co-created vertices, so deltas are typically small numbers that compress well with varint encoding.

**Edge ID**: Unique identifier for this edge instance. Varint-encoded.

### Varint Encoding

All numeric values use variable-length integer encoding (like protobuf):

| Value Range | Bytes Used |
|---|---|
| 0–127 | 1 byte |
| 128–16,383 | 2 bytes |
| 16,384–2,097,151 | 3 bytes |

This means a vertex ID delta of 5 takes 1 byte instead of 8. Massive space savings at scale.

### Properties in the Value

The cell value contains edge properties in two forms:

- **Signature properties**: Referenced in the schema, stored with compressed metadata (just the value, type inferred from schema)
- **Other properties**: Stored with full type information (type tag + value)

### Vertex-Centric Indexes

These are NOT separate index structures. They exploit the column sort order within a vertex's row.

Because columns are sorted by `[label_id | sort_key | ...]`, a query like "get all outgoing KNOWS edges sorted by timestamp" becomes a **byte-range scan** within the row:

```
Scan: row=vertex_42, column_start=[KNOWS|OUT|...], column_end=[KNOWS|OUT|0xFF...]
```

The storage backend returns only matching edges. No full adjacency list scan needed. This is how JanusGraph handles "supernode" vertices with millions of edges.

---

## Layer 3: Caching Architecture

### Transaction-Level Cache (L1)

Every open transaction maintains two in-memory caches:

**Vertex Cache**
- LRU eviction, default capacity: 20,000 entries
- Caches vertex objects and their loaded adjacency list subsets
- **Modified vertices are pinned** — they cannot be evicted (you can't lose uncommitted mutations)
- Weight-based: vertices with large adjacency lists consume proportionally more cache budget
- Config: `cache.tx-vertex-cache-size`

**Index Result Cache**
- Caches results from index backend queries (Elasticsearch/Solr results)
- Weight formula: `2 + result_set_size` per entry
- Total weight capped at 50% of the transaction cache budget
- Prevents repeated expensive external index calls within a single transaction

### Backend-Level Cache (L2)

Each storage backend has its own caching:

- **Cassandra**: Row cache, key cache, block cache (OS page cache)
- **HBase**: BlockCache, BucketCache, Bloom filters
- **FoundationDB**: Storage server caches, OS page cache (FDB relies heavily on SSDs and the OS buffer cache rather than application-level caching)
- **BerkeleyDB**: JE cache (in-process, configurable heap fraction)

### Cache Invalidation

When a vertex is locally modified within a transaction:
1. The vertex is pinned in L1 cache
2. All related backend cache entries are marked expired
3. On next access, expired entries trigger a fresh backend read
4. This ensures read-your-writes consistency within a transaction

---

## Layer 4: Transaction System

### Transaction Lifecycle

```
1. Open      → Thread's first graph op auto-opens a tx
2. Read      → Check L1 cache → miss → read from backend
3. Mutate    → Buffer changes in local mutation log
                (modified vertices pinned in cache)
4. Commit    → Batch all mutations into single backend write
                → If backend supports ACID, this is atomic
                → If Cassandra, it's "best effort atomic"
5. Rollback  → Discard mutation log, release pins
```

The `StandardJanusGraphTx` class orchestrates all of this. It maintains:
- Vertex cache
- Index result cache
- Mutation log (all pending changes)
- Lock set (acquired consistency locks)
- Schema inspector (for type resolution)

### ACID Guarantees Depend on Backend

| Backend | Atomicity | Isolation | Durability |
|---|---|---|---|
| **FoundationDB** | Full (cross-shard) | Strict serializable | Full |
| **Cassandra** | Row-level only | Eventual consistency | Tunable |
| **HBase** | Row-level only | Read-committed | Full |
| **BerkeleyDB** | Full (single-node) | Serializable | Full |

This is why **FoundationDB is architecturally the best backend for JanusGraph** if you need real ACID semantics. You get cross-key atomic transactions for free.

### Locking

By default, JanusGraph does NOT acquire locks (optimistic approach). You can enable explicit locking per schema element:

```java
mgmt.setConsistency(nameProperty, ConsistencyModifier.LOCK);
```

With Cassandra, this uses a distributed lock protocol. With FoundationDB, the inherent serializable isolation makes this less critical (FDB's optimistic concurrency control handles conflicts natively).

### Write-Ahead Log

Optional WAL for crash recovery:
- Configurable via `tx.log-tx`
- Entries expire after 2 days (configurable TTL)
- Adds one extra write per transaction
- Useful for recovery scenarios with non-ACID backends

---

## Layer 5: Query Processing (Gremlin Execution)

### The TinkerPop Stack

JanusGraph implements the Apache TinkerPop provider interface. A Gremlin query flows through:

```
Gremlin String: g.V().has('name','Andrew').out('KNOWS').values('name')
         │
         ▼
   ┌─────────────┐
   │  Parser      │  Gremlin grammar → traversal bytecode
   └──────┬──────┘
          ▼
   ┌─────────────┐
   │  Strategies  │  JanusGraph injects custom TraversalStrategy
   │  (Optimizer) │  instances that rewrite the traversal:
   │              │  • Index lookup detection
   │              │  • has() filter pushdown to storage
   │              │  • Step reordering
   │              │  • Batch query aggregation
   └──────┬──────┘
          ▼
   ┌─────────────┐
   │  Execution   │  Lazy, pull-based iterator model
   │  Engine      │  Each step pulls from previous step
   └──────┬──────┘
          ▼
   ┌─────────────┐
   │  Backend I/O │  Actual storage reads/writes
   └─────────────┘
```

### Batch Processing

When `query.batch.enabled=true` (default since JanusGraph 1.0):

Instead of executing one vertex lookup at a time, compatible steps **aggregate** multiple vertices and fire a single multi-get to the backend. This amortizes:
- Network round-trip latency
- Traversal compilation cost
- Backend query planning overhead

Example: `g.V(1,2,3).out('KNOWS')` → instead of 3 separate adjacency list reads, one batched multi-row fetch.

### Thread Model

JanusGraph Server wraps Gremlin Server with configurable thread pools:
- **Boss threads**: Accept incoming connections (default: 1)
- **Worker threads**: Handle I/O on accepted connections (default: 1)
- **Gremlin pool**: Execute actual traversals (default: 8)

Each Gremlin pool thread gets its own transaction context.

---

## Layer 6: Index Subsystem

### Two Fundamentally Different Index Types

**1. Vertex-Centric Indexes (Local)**
- Stored IN the edgestore alongside vertex data
- Exploit column sort order for range scans within a single vertex
- Handle the "supernode problem" (vertices with millions of edges)
- Zero external dependencies

**2. Mixed/Composite Global Indexes (External)**
- Stored in a separate index backend (Elasticsearch, Solr, or Lucene)
- Enable queries like "find all vertices where name = 'Andrew'"
- Support full-text search, geo queries, fuzzy matching
- JanusGraph coordinates writes to both the primary store AND the index backend

### Index Write Path

```
Mutation (add vertex with indexed property)
    │
    ├──→ Write to edgestore (primary storage)
    │
    └──→ Write to index backend (async or sync)
         └──→ Elasticsearch / Solr / Lucene
```

Index refreshes are NOT instant. Elasticsearch has `refresh_interval` (default 1s), Solr has `maxTime`. There's an eventual consistency window between primary storage and index backend.

---

## Layer 7: ID Management

### Block-Based Allocation

JanusGraph allocates vertex/edge IDs in blocks to minimize coordination:

1. Instance requests a block of IDs from the `janusgraph_ids` store
2. Block acquisition is a distributed consensus operation (expensive)
3. Instance allocates IDs locally from the block (cheap)
4. When block exhausted, request another

### Collision Avoidance

With multiple JanusGraph instances:
- Each instance gets a unique random marker
- ID blocks are partitioned by marker
- No central coordinator needed
- IDs are 64-bit longs with bit patterns encoding type information (vertex vs. edge vs. property)

---

## FoundationDB-Specific Architecture

### Why FoundationDB is Interesting for This

FoundationDB's model maps cleanly to JanusGraph's needs:

| JanusGraph Need | FoundationDB Feature |
|---|---|
| Wide-column rows | Tuple-encoded composite keys with range scan |
| Atomic mutations | Strict serializable transactions (5-second limit) |
| Ordered iteration | Globally ordered keyspace |
| Distributed scale | Automatic sharding and replication |
| ACID transactions | First-class, cross-key ACID |

### Tuple Layer Mapping

```
JanusGraph Wide Row:
  Row: vertex_42
    Column: [edge_label|sort|adj_vid|eid]  → value

FoundationDB Keys:
  ("edgestore", 42, [edge_label|sort|adj_vid|eid]) → value

Range scan for vertex 42's adjacency list:
  getRange(("edgestore", 42, 0x00), ("edgestore", 42, 0xFF))
```

The tuple layer encodes each element with type-aware byte ordering, so integer 42 sorts correctly among other integers, and the composite key preserves left-to-right ordering.

### The 5-Second Transaction Limit

FoundationDB enforces a **5-second transaction time limit**. This means:
- Large batch mutations must be chunked
- Long-running analytical queries need special handling
- The JanusGraph FoundationDB adapter handles this by breaking large operations into sub-transactions
- Read snapshots provide consistent reads without holding transaction locks

### Async Iterators (Opt-In)

The `janusgraph-foundationdb` adapter supports two range query modes via `storage.fdb.get-range-mode`:

- **`list` (default)**: Synchronous iteration. Loads results into a list. Simpler, but loads full result sets into memory.
- **`iterator`**: Async iterator mode. On-demand data pulling — doesn't load entire adjacency lists into memory. Better memory efficiency and thread parallelism for vertices with huge edge counts. Must be explicitly enabled.

---

## If You Built Your Own: Component Checklist

Here's what you'd actually need to build, ordered by dependency:

### Phase 1: Storage Foundation
```
┌─────────────────────────────────────────────┐
│ 1. Subspace Manager                         │
│    Define key prefixes for vertices, edges, │
│    indexes, metadata, ID allocation         │
│                                             │
│ 2. Tuple Encoder / Key Composer             │
│    Serialize (vertex_id, column_data) into  │
│    ordered byte keys                        │
│                                             │
│ 3. Value Serializer                         │
│    Encode/decode property maps, edge data   │
│    with varint compression                  │
│                                             │
│ 4. ID Allocator                             │
│    Block-based, distributed-safe ID         │
│    generation using FDB transactions        │
└─────────────────────────────────────────────┘
```

### Phase 2: Graph Semantics
```
┌─────────────────────────────────────────────┐
│ 5. Vertex Store                             │
│    CRUD for vertices + property storage     │
│    Adjacency list management                │
│                                             │
│ 6. Edge Store                               │
│    Dual-write (both endpoints)              │
│    Direction-aware column encoding          │
│    Sort key property encoding               │
│                                             │
│ 7. Schema Manager                           │
│    Edge label definitions, property keys,   │
│    sort key configurations, cardinality     │
│                                             │
│ 8. Index Manager                            │
│    Vertex-centric indexes (column ordering) │
│    Global composite indexes (separate KV)   │
│    Optional: external index integration     │
└─────────────────────────────────────────────┘
```

### Phase 3: Query Engine
```
┌─────────────────────────────────────────────┐
│ 9. Transaction Manager                      │
│    Mutation buffering, commit batching,     │
│    conflict detection (or delegate to FDB)  │
│                                             │
│ 10. Cache Layer                             │
│     Vertex cache (LRU, pin-on-modify)      │
│     Index result cache                      │
│     Backend result cache                    │
│                                             │
│ 11. Query Parser + Optimizer                │
│     Gremlin bytecode → execution plan       │
│     Index selection, filter pushdown,       │
│     batch aggregation strategies            │
│                                             │
│ 12. Execution Engine                        │
│     Pull-based iterator model               │
│     Step-by-step traversal execution        │
│     Batch multi-vertex fetches              │
└─────────────────────────────────────────────┘
```

### Phase 4: TinkerPop Integration (Optional)
```
┌─────────────────────────────────────────────┐
│ 13. TinkerPop Provider                      │
│     Implement Graph, Vertex, Edge,          │
│     VertexProperty, Property interfaces     │
│     Pass gremlin-test validation suite      │
│                                             │
│ 14. TraversalStrategy Optimizations         │
│     Custom strategies for your backend      │
│     Index detection, step rewriting         │
│                                             │
│ 15. Server / Network Layer                  │
│     WebSocket server for remote queries     │
│     Serialization (GraphSON, Gryo)          │
└─────────────────────────────────────────────┘
```

### Complexity Estimates

| Component | Effort | Why |
|---|---|---|
| Subspace + Tuple Encoding | Low | FDB provides these primitives |
| Value Serializer | Medium | Varint + schema-aware encoding |
| ID Allocator | Medium | Distributed coordination logic |
| Vertex/Edge Store | Medium | Dual-write, delta encoding |
| Schema Manager | Medium | Type system, validation |
| Index Manager | Medium-High | Multiple index types, consistency |
| Transaction Manager | Low with FDB | FDB handles the hard parts |
| Cache Layer | Medium | LRU + pin semantics + invalidation |
| Query Optimizer | **High** | Where most performance comes from |
| TinkerPop Provider | Medium | Large API surface, compliance tests |

### The Key Insight

**FoundationDB eliminates the hardest distributed systems problems.** You don't build consensus, replication, sharding, or distributed transactions. FDB gives you an ordered, transactional key-value store. Your job is purely the graph semantics layer: data model, query optimization, and caching.

With Cassandra, you'd also need to build distributed locking, handle eventual consistency, manage conflict resolution, and implement your own transaction coordinator. FDB removes all of that.

---

## Key Architectural Trade-offs

**Adjacency list (JanusGraph) vs. Separate edge table (some other systems)**:
- Adjacency list: O(1) to find a vertex's edges (single row read). Doubles write cost (dual storage). Best for traversal-heavy workloads.
- Separate edge table: Single write per edge. O(n) edge lookup requires index. Better for edge-centric analytics.

**Embedded indexes vs. External search**:
- Vertex-centric indexes are fast, zero-latency, but limited to equality/range on pre-defined sort keys
- External indexes (ES/Solr) support full-text, geo, fuzzy — but add operational complexity and consistency lag

**Transaction scope with FDB's 5-second limit**:
- Fine for OLTP workloads (single-vertex mutations, short traversals)
- Requires chunking for bulk loads or long analytical traversals
- Read snapshots help for read-heavy analytical queries

**Caching vs. Consistency**:
- Aggressive caching improves read throughput but risks stale reads in multi-instance deployments
- FDB's strict serializability helps here — cache invalidation is simpler when the backend guarantees ordering
