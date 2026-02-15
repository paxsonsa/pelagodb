# PelagoDB Specification v1.0

**Status:** Draft
**Date:** 2026-02-14
**Authors:** Andrew + Claude

---

## Version History

| Version | Date | Summary |
|---------|------|---------|
| v0.3 | 2026-01 | User stories and high-level architecture. Established CDC-driven design, ownership model, and background job framework. Removed saga coordinator in favor of FDB-native transactions. |
| v0.4 | 2026-02 | Phase 1 implementation spec. Detailed keyspace layout, CRUD operations, query system, gRPC API, and error model. Multi-site replication deferred to Phase 3. |
| v1.0 | 2026-02-14 | **Canonical implementation spec.** Consolidated v0.3/v0.4 content. Added: Multi-Site Replication (Section 12), PQL query language (Section 17), Site ID collision guard, ownership transfer protocol, Reactive Subscriptions/Watch System (Section 18), RocksDB Cache Layer (Section 19), Security and Authorization with path-based ACLs (Section 20). |

**Lineage:** v0.3 → v0.4 → v1.0

Previous versions are archived in `.llm/context/archive/` for reference.

---

## Table of Contents

1. [Introduction](#1-introduction)
   - 1.1 [Goals](#11-goals)
   - 1.2 [Non-Goals](#12-non-goals)
   - 1.3 [Technology Stack](#13-technology-stack)
2. [Architecture Overview](#2-architecture-overview)
   - 2.1 [System Diagram](#21-system-diagram)
   - 2.2 [Crate Structure](#22-crate-structure)
   - 2.3 [Request Flow](#23-request-flow)
3. [Keyspace Layout](#3-keyspace-layout)
   - 3.1 [Directory Hierarchy](#31-directory-hierarchy)
   - 3.2 [System Subspaces](#32-system-subspaces)
   - 3.3 [Namespace Subspaces](#33-namespace-subspaces)
   - 3.4 [Key Encoding](#34-key-encoding)
   - 3.5 [Value Encoding (CBOR)](#35-value-encoding-cbor)
4. [Data Model](#4-data-model)
   - 4.1 [Node ID Format](#41-node-id-format)
   - 4.2 [Edge ID Format](#42-edge-id-format)
   - 4.3 [Property Types](#43-property-types)
   - 4.4 [Null Semantics](#44-null-semantics)
5. [Schema System](#5-schema-system)
   - 5.1 [Entity Schema Structure](#51-entity-schema-structure)
   - 5.2 [Property Definitions](#52-property-definitions)
   - 5.3 [Edge Declarations](#53-edge-declarations)
   - 5.4 [Index Types](#54-index-types)
   - 5.5 [Schema Evolution Rules](#55-schema-evolution-rules)
   - 5.6 [Schema Registry Storage](#56-schema-registry-storage)
6. [Node Operations](#6-node-operations)
7. [Edge Operations](#7-edge-operations)
8. [Query System](#8-query-system)
9. [gRPC API](#9-grpc-api)
10. [CLI Reference](#10-cli-reference)
11. [CDC System](#11-cdc-system)
12. [Multi-Site Replication](#12-multi-site-replication)
    - 12.1 [Overview](#121-overview)
    - 12.2 [Ownership Model](#122-ownership-model)
    - 12.3 [Ownership Transfer](#123-ownership-transfer)
    - 12.4 [Replication Architecture](#124-replication-architecture)
    - 12.5 [Event Projection](#125-event-projection)
    - 12.6 [Conflict Resolution](#126-conflict-resolution)
    - 12.7 [Replication Scope](#127-replication-scope)
    - 12.8 [Consistency Guarantees](#128-consistency-guarantees)
    - 12.9 [Failure Modes and Recovery](#129-failure-modes-and-recovery)
13. [Background Jobs](#13-background-jobs)
14. [Error Model](#14-error-model)
15. [Configuration](#15-configuration)
16. [Testing Strategy](#16-testing-strategy)
17. [Pelago Query Language (PQL)](#17-pelago-query-language-pql)
    - 17.1 [Overview](#171-overview)
    - 17.2 [Query Structure](#172-query-structure)
    - 17.3 [Grammar (EBNF)](#173-grammar-ebnf)
    - 17.4 [Root Functions](#174-root-functions)
    - 17.5 [Edge Traversal Syntax](#175-edge-traversal-syntax)
    - 17.6 [Cross-Namespace Traversal](#176-cross-namespace-traversal)
    - 17.7 [Directives](#177-directives)
    - 17.8 [Variables](#178-variables)
    - 17.9 [Aggregations](#179-aggregations)
    - 17.10 [Mutations (Upsert Blocks)](#1710-mutations-upsert-blocks)
    - 17.11 [REPL Integration](#1711-repl-integration)
    - 17.12 [Wire Transport](#1712-wire-transport)
    - 17.13 [Compilation Pipeline](#1713-compilation-pipeline)
    - 17.14 [Implementation Phases](#1714-implementation-phases)
    - 17.15 [Comparison: PQL vs DQL](#1715-comparison-pql-vs-dql)
18. [Reactive Subscriptions (Watch System)](#18-reactive-subscriptions-watch-system)
    - 18.1 [Overview](#181-overview)
    - 18.2 [WatchService gRPC API](#182-watchservice-grpc-api)
    - 18.3 [Subscription Types](#183-subscription-types)
    - 18.4 [Watch Events](#184-watch-events)
    - 18.5 [Subscription Options](#185-subscription-options)
    - 18.6 [CDC Integration Design](#186-cdc-integration-design)
    - 18.7 [Server-Side Subscription Registry](#187-server-side-subscription-registry)
    - 18.8 [Client Reconnection and Resume Semantics](#188-client-reconnection-and-resume-semantics)
    - 18.9 [Resource Management](#189-resource-management)
    - 18.10 [Complete Proto Definitions](#1810-complete-proto-definitions)
    - 18.11 [Error Codes](#1811-error-codes)
    - 18.12 [Configuration](#1812-configuration)
    - 18.13 [Use Case Examples](#1813-use-case-examples)
19. [RocksDB Cache Layer](#19-rocksdb-cache-layer)
    - 19.1 [Architecture Overview](#191-architecture-overview)
    - 19.2 [RocksDB Keyspace Layout](#192-rocksdb-keyspace-layout)
    - 19.3 [CDC Projector](#193-cdc-projector)
    - 19.4 [Cache Invalidation Strategy](#194-cache-invalidation-strategy)
    - 19.5 [Read Path](#195-read-path)
    - 19.6 [High-Water Mark Tracking](#196-high-water-mark-tracking)
    - 19.7 [Traversal Caching](#197-traversal-caching)
    - 19.8 [Cache Warm-up and Rebuild](#198-cache-warm-up-and-rebuild)
    - 19.9 [Configuration](#199-configuration)
    - 19.10 [Eviction Policy](#1910-eviction-policy)
    - 19.11 [Metrics](#1911-metrics)
20. [Security and Authorization](#20-security-and-authorization)
    - 20.1 [Security Architecture Overview](#201-security-architecture-overview)
    - 20.2 [Authentication Mechanisms](#202-authentication-mechanisms)
    - 20.3 [Principal Model](#203-principal-model)
    - 20.4 [Path-Based Permission Model](#204-path-based-permission-model)
    - 20.5 [Policy Definition](#205-policy-definition)
    - 20.6 [Built-in Roles](#206-built-in-roles)
    - 20.7 [Policy Storage in FDB](#207-policy-storage-in-fdb)
    - 20.8 [RequestContext Integration](#208-requestcontext-integration)
    - 20.9 [Token and Credential Management](#209-token-and-credential-management)
    - 20.10 [Audit Logging](#2010-audit-logging)
    - 20.11 [gRPC Auth Service](#2011-grpc-auth-service)
    - 20.12 [Security Configuration](#2012-security-configuration)
    - 20.13 [Error Codes](#2013-error-codes)
    - 20.14 [Security Considerations](#2014-security-considerations)

---

## 1. Introduction

PelagoDB is a distributed graph database engine designed for high-performance graph operations with multi-site replication capabilities. Built on FoundationDB for transactional guarantees and horizontal scalability, it provides first-class support for schema validation, property indexing, and reactive subscriptions.

### 1.1 Goals

1. **Point lookups are fast.** Single-node retrieval by ID should be sub-millisecond at the storage layer.

2. **Criteria/scan lookups are performant.** Property-based queries against indexed fields compile to FDB range scans, not in-memory filtering.

3. **Graph traversal is first-class.** Multi-hop traversal with per-hop filtering streams results asynchronously with backpressure.

4. **Schema is a client concern.** The API layer declares schema. The storage layer enforces and indexes based on those declarations. New entity types require no server restarts or recompilation.

5. **Multi-site replication with ownership semantics.** Nodes have home sites. Conflict-free replication via CDC-driven event streaming.

6. **Lifecycle management via namespacing.** Entity types, logical databases, and tenants can be independently managed, cleared, and replicated.

7. **Reactive subscriptions.** Clients can watch individual nodes or query result sets for real-time change notifications.

8. **High-throughput reads.** The read path scales horizontally with configurable consistency levels and read coalescing for hot data.

### 1.2 Non-Goals

The following are explicitly out of scope for v1:

- **Full-text search** — Delegate to external search engine (Elasticsearch, Meilisearch)
- **Graph analytics / batch processing** — OLAP workloads are not supported
- **SQL compatibility** — No SQL dialect; uses CEL for predicates
- **Multi-model** — Graph-first; not a document or relational database

### 1.3 Technology Stack

| Component | Choice | Rationale |
|-----------|--------|-----------|
| Language | Rust | Sub-millisecond hot paths, zero-copy serialization, tokio async runtime |
| Async runtime | tokio | Task groups for traversal fan-out, mature ecosystem |
| Primary storage | FoundationDB | Ordered KV with strict serializable ACID, automatic sharding |
| Cache layer | RocksDB | Materialized views, temporal indexes, hot data cache (Phase 2) |
| Query predicates | CEL (cel-rust) | Portable expression language, type-checkable, AST-decomposable |
| Wire transport | gRPC (tonic) | Streaming RPCs, protobuf framing, wide client language support |
| FDB client | `fdb` crate | Tokio-native, FDB 7.1, actively maintained with BindingTester CI |
| Serialization | CBOR (ciborium) | Compact binary format for property values; FDB tuple layer for keys |

---

## 2. Architecture Overview

### 2.1 System Diagram

```
┌─────────────────────────────────────────────────────────────┐
│                       Client SDK                             │
│    Schema registration · Typed queries · Stream consumer     │
└───────────────────────────┬─────────────────────────────────┘
                            │ gRPC (protobuf frames)
                            │
┌───────────────────────────▼─────────────────────────────────┐
│                        API Layer                             │
│    Schema validation · CEL compilation · Query planning      │
│    gRPC service handlers · Rate limiting                     │
│    Watch registry · Traversal limit enforcement              │
└───────┬───────────────────────────────────┬─────────────────┘
        │                                   │
        ▼                                   ▼
┌───────────────────────┐    ┌────────────────────────────────┐
│    Query Executor     │    │      Mutation Pipeline          │
│    Stream assembly    │    │   Validate → FDB write + CDC    │
│    Index selection    │    │   (unified path for all ops)    │
│    Traversal engine   │    │                                 │
│    Watch Registry     │    │                                 │
└───────┬───────┬───────┘    └──────┬───────────────────────── ┘
        │       │                   │
        ▼       ▼                   ▼
┌────────────┐ ┌─────────────────────────────┐
│  RocksDB   │ │       FoundationDB          │
│  (cache/   │ │    (system of record)       │
│  matviews/ │ │    Entities, edges,         │
│  temporal  │ │    indexes, CDC stream,     │
│  indexes,  │ │    schema registry,         │
│  read-thru)│ │    ID alloc, jobs           │
└────────────┘ │                             │
               └─────────────────────────────┘
```

### 2.2 Crate Structure

```
pelago/
├── Cargo.toml                 # workspace root
├── Dockerfile                 # multi-stage: builder + slim runtime with FDB client libs
├── docker-compose.yml         # FDB + pelago server + test runner
├── proto/
│   └── pelago.proto           # gRPC service + message definitions
├── crates/
│   ├── pelago-proto/          # generated protobuf/tonic code
│   │   ├── Cargo.toml
│   │   ├── build.rs
│   │   └── src/lib.rs
│   ├── pelago-core/           # shared types, encoding, errors
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── types.rs       # NodeId, EdgeId, Value, PropertyType
│   │       ├── encoding.rs    # tuple encoding, CBOR helpers, sort-order encoding
│   │       ├── schema.rs      # EntitySchema, PropertyDef, EdgeDef, IndexType
│   │       ├── errors.rs      # PelagoError enum (thiserror)
│   │       └── config.rs      # ServerConfig (site_id, fdb_cluster_file, etc.)
│   ├── pelago-storage/        # FDB interactions, subspace layout
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── subspace.rs    # Subspace builder: db/namespace/entities/...
│   │       ├── schema.rs      # Schema CRUD in FDB _schema subspace
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
    ├── integration/           # Rust tests against real FDB
    └── python/                # Python gRPC test client
```

### 2.3 Request Flow

#### Read Path (Point Lookup)

```
Client: GetNode(entity_type="Person", node_id="1_42")
    │
    ▼
API Layer
    ├── Validate request fields
    ├── Parse node_id → NodeId { site: 1, seq: 42 }
    │
    ▼
Storage Layer
    ├── Build key: (db, ns, data, "Person", <node_id_bytes>)
    ├── FDB get() → CBOR blob
    ├── Deserialize CBOR → property map
    │
    ▼
Response: Node { id, entity_type, properties }
```

#### Write Path (Node Create)

```
Client: CreateNode(entity_type="Person", properties={name: "Alice", age: 30})
    │
    ▼
API Layer
    ├── Load schema for Person
    ├── Validate properties against schema
    │   ├── Check required fields
    │   ├── Type-check values
    │   └── Apply defaults
    │
    ▼
Storage Layer (single FDB transaction)
    ├── Allocate NodeId from counter
    ├── Write data key: (db, ns, data, "Person", <id>) → CBOR
    ├── Write index entries for indexed properties
    │   ├── Unique: (db, ns, idx, unique, "Person", "email", value) → node_id
    │   └── Range: (db, ns, idx, range, "Person", "age", encoded_value, node_id) → empty
    ├── Write CDC entry: (db, ns, _cdc, <versionstamp>) → CdcEntry
    └── Commit
    │
    ▼
Response: Node { id: "1_42", ... }
```

---

## 3. Keyspace Layout

This section defines the authoritative FoundationDB keyspace layout for PelagoDB.

### 3.1 Directory Hierarchy

```
/pelago/
  /_sys/                     → System-wide configuration
  /<db>/                     → Database (logical isolation unit)
    /_db/                    → DB-level metadata
    /<ns>/                   → Namespace (entity grouping within db)
      /_ns/                  → Namespace metadata
      /_schema/              → Schema definitions
      /_types/               → Entity type registry
      /_cdc/                 → Change data capture stream
      /_jobs/                → Background job state
      /_ids/                 → ID allocation counters
      /data/                 → Entity storage
      /loc/                  → Locality index (owning site)
      /idx/                  → Secondary indices
      /edge/                 → Edge storage
```

**Naming Conventions:**

- Underscored prefixes (`_sys`, `_db`, `_ns`, etc.) are reserved system subspaces
- User-created databases and namespaces cannot begin with underscore
- Database names map to logical isolation units (e.g., `production`, `staging`)
- Namespace names group related entity types (e.g., `core`, `analytics`)

### 3.2 System Subspaces

#### `/pelago/_sys/` — System Configuration

| Key | Value | Description |
|-----|-------|-------------|
| `(config)` | System config object | Global system settings |
| `(site_claim, <site_id>)` | UTF-8 site name | Site ID ownership claim (collision guard) |
| `(dc, <dc_id>)` | DC config object | Datacenter registry |
| `(dc_topo, <dc_a>, <dc_b>)` | Topology object | DC-to-DC network characteristics |

**System Config:**
```json
{
  "version": 1,
  "created_at": "<timestamp>",
  "default_replication": 3
}
```

**DC Config:**
```json
{
  "name": "San Francisco",
  "region": "us-west",
  "tier": "primary",
  "fdb_cluster": "sf-prod-001",
  "active": true
}
```

#### `/pelago/<db>/_db/` — Database Metadata

| Key | Value | Description |
|-----|-------|-------------|
| `(config)` | DB config object | Database configuration |
| `(lock)` | Lock object or null | Write lock state |
| `(ns, <ns_name>)` | Namespace info | Namespace registry entry |

**DB Config:**
```json
{
  "locality_default": "SF",
  "created_at": "<timestamp>",
  "created_by": "user_123"
}
```

### 3.3 Namespace Subspaces

Each namespace contains the following subspaces:

#### `/_ns/` — Namespace Metadata

| Key | Value | Description |
|-----|-------|-------------|
| `(config)` | NS config object | Namespace configuration |
| `(stats)` | Statistics object | Entity/edge counts (async updated) |

#### `/_schema/` — Schema Definitions

| Key | Value | Description |
|-----|-------|-------------|
| `(<type>, v, <version>)` | Schema definition | Versioned schema CBOR |
| `(<type>, latest)` | Integer | Current version pointer |

The `v` separator prevents collision between version integers and the `latest` marker.

#### `/_types/` — Entity Type Registry

| Key | Value | Description |
|-----|-------|-------------|
| `(<type>)` | Type info object | Entity type metadata + count |

```json
{
  "created_at": "<timestamp>",
  "entity_count": 45000,
  "latest_schema": 3
}
```

#### `/_cdc/` — Change Data Capture

| Key | Value | Description |
|-----|-------|-------------|
| `(<versionstamp>)` | CdcEntry CBOR | Committed mutation record |

CDC keys use FDB versionstamps for zero-contention, globally-ordered append.

#### `/_jobs/` — Background Jobs

| Key | Value | Description |
|-----|-------|-------------|
| `(<job_type>, <job_id>)` | Job state CBOR | Progress cursor, status |

#### `/_ids/` — ID Allocation

| Key | Value | Description |
|-----|-------|-------------|
| `(<type>, <site_id>)` | u64 counter | Next ID block start |

#### `/data/` — Entity Storage

| Key | Value | Description |
|-----|-------|-------------|
| `(<type>, <node_id>)` | Entity CBOR | Node properties |

**Entity Record:**
```json
{
  "p": { "name": "Alice", "age": 30 },
  "l": "SF",
  "c": "<created_timestamp>",
  "u": "<updated_timestamp>"
}
```

| Field | Full Name | Description |
|-------|-----------|-------------|
| `p` | payload | Client properties |
| `l` | locality | Owning site (3-char) |
| `c` | created_at | Creation timestamp |
| `u` | updated_at | Last update timestamp |

#### `/loc/` — Locality Index

| Key | Value | Description |
|-----|-------|-------------|
| `(<type>, <node_id>)` | 3-char string | Owning datacenter |

Separated from entity data for minimal read on write-path locality validation.

**Invariant:** `loc/(<type>, <node_id>)` always equals `data/(<type>, <node_id>).l`

#### `/idx/` — Secondary Indices

| Key | Value | Description |
|-----|-------|-------------|
| `(unique, <type>, <field>, <value>)` | node_id bytes | Unique index |
| `(eq, <type>, <field>, <value>, <node_id>)` | empty | Equality index |
| `(range, <type>, <field>, <encoded_value>, <node_id>)` | empty | Range index |
| `(byloc, <dc>, <type>, <node_id>)` | `0x01` | Entities by locality |
| `(byupd, <type>, <versionstamp>)` | node_id | Entities by update time |

#### `/edge/` — Edge Storage

| Key | Value | Description |
|-----|-------|-------------|
| `(f, <src_type>, <src_id>, <label>, <tgt_db>, <tgt_ns>, <tgt_type>, <tgt_id>)` | timestamp | Forward edge |
| `(fm, <src_type>, <src_id>, <label>, <tgt_db>, <tgt_ns>, <tgt_type>, <tgt_id>)` | metadata CBOR | Forward edge metadata |
| `(r, <tgt_db>, <tgt_ns>, <tgt_type>, <tgt_id>, <label>, <src_type>, <src_id>)` | `0x01` | Reverse edge |

**Prefix meanings:**

| Prefix | Meaning |
|--------|---------|
| `f` | Forward edge (existence + created_at timestamp) |
| `fm` | Forward edge metadata (optional, only if edge has properties) |
| `r` | Reverse edge (existence only) |

### 3.4 Key Encoding

All keys use FDB's tuple layer for encoding, providing:

- Lexicographic ordering
- Type-aware comparison
- Efficient prefix matching
- Versionstamp support

**Data Type Encoding:**

| Element | Encoding | Size |
|---------|----------|------|
| `db`, `ns`, `type` | UTF-8 string | Variable |
| `node_id` | 9 bytes: `[site:u8][seq:u64 BE]` | 9 bytes |
| `edge_id` | 9 bytes: `[site:u8][seq:u64 BE]` | 9 bytes |
| `dc` (datacenter) | UTF-8 3-char | 3 bytes |
| `version` | Big-endian int32 | 4 bytes |
| `timestamp` | Int64 microseconds since epoch | 8 bytes |
| `versionstamp` | FDB versionstamp | 10 bytes |
| Counter values | Little-endian int64 | 8 bytes |
| Existence markers | `0x01` | 1 byte |

**Sort-Order Encoding for Index Values:**

For range indexes to work correctly, values must be encoded in sort order:

| Type | Encoding |
|------|----------|
| `int` (i64) | Flip sign bit, big-endian 8 bytes |
| `float` (f64) | IEEE 754 order-preserving (flip sign bit + conditional invert) |
| `string` | UTF-8 bytes (natural lexicographic order) |
| `timestamp` | Same as int (unix microseconds) |
| `bytes` | Raw bytes |
| `bool` | 0x00 (false) or 0x01 (true) |

### 3.5 Value Encoding (CBOR)

All structured values use CBOR (Concise Binary Object Representation):

- Compact binary format (smaller than JSON)
- Schema-flexible (no compilation required)
- Self-describing types
- Fast encode/decode
- Wide language support via ciborium (Rust), cbor2 (Python), etc.

**Example Node CBOR:**

```rust
// Rust struct
struct NodeRecord {
    p: HashMap<String, Value>,  // payload (client properties)
    l: String,                   // locality (owning site)
    c: i64,                      // created_at (unix micros)
    u: i64,                      // updated_at (unix micros)
}

// Encoded as CBOR map:
// { "p": { "name": "Alice", "age": 30 }, "l": "SF", "c": 1707840000000000, "u": 1707840000000000 }
```

**Example CDC Entry CBOR:**

```rust
struct CdcEntry {
    site: String,
    timestamp: i64,
    batch_id: Option<String>,
    operations: Vec<CdcOperation>,
}

enum CdcOperation {
    NodeCreate { entity_type: String, node_id: NodeId, properties: HashMap<String, Value> },
    NodeUpdate { entity_type: String, node_id: NodeId, changed: HashMap<String, Value> },
    NodeDelete { entity_type: String, node_id: NodeId },
    EdgeCreate { source: NodeRef, target: NodeRef, edge_type: String, edge_id: EdgeId, properties: HashMap<String, Value> },
    EdgeDelete { edge_id: EdgeId },
}
```

---

## 4. Data Model

### 4.1 Node ID Format

Node IDs use a compact 9-byte format that encodes the originating site:

```
┌─────────┬──────────────────────────────────────┐
│ site_id │              counter                  │
│  (u8)   │              (u64 BE)                 │
├─────────┼──────────────────────────────────────┤
│ 1 byte  │              8 bytes                  │
└─────────┴──────────────────────────────────────┘
```

**Rust Definition:**

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId {
    pub site: u8,    // Owning site (0-255)
    pub seq: u64,    // Monotonic counter per site
}

impl NodeId {
    /// Serialize to 9 bytes for FDB key
    pub fn to_bytes(&self) -> [u8; 9] {
        let mut buf = [0u8; 9];
        buf[0] = self.site;
        buf[1..9].copy_from_slice(&self.seq.to_be_bytes());
        buf
    }

    /// Deserialize from 9 bytes
    pub fn from_bytes(bytes: &[u8; 9]) -> Self {
        Self {
            site: bytes[0],
            seq: u64::from_be_bytes(bytes[1..9].try_into().unwrap()),
        }
    }

    /// Human-readable format: "site_seq" (e.g., "1_42")
    pub fn to_string(&self) -> String {
        format!("{}_{}", self.site, self.seq)
    }

    /// Parse from human-readable format
    pub fn from_str(s: &str) -> Result<Self, ParseError> {
        let parts: Vec<&str> = s.split('_').collect();
        if parts.len() != 2 {
            return Err(ParseError::InvalidFormat);
        }
        Ok(Self {
            site: parts[0].parse()?,
            seq: parts[1].parse()?,
        })
    }

    /// Extract the originating site from an ID
    pub fn origin_site(&self) -> u8 {
        self.site
    }
}
```

**Properties:**

- **Compact:** 9 bytes vs 16 bytes for UUID
- **Extractable origin:** Site ID can be read without database lookup
- **No cross-site coordination:** Each site has independent counter
- **Sortable:** Big-endian encoding preserves sort order within a site

**ID Allocation:**

IDs are allocated per-site using an atomic counter in FDB:

```
Key: (db, ns, _ids, entity_type, site_id)
Value: u64 (next available sequence number)
```

Batch allocation grabs blocks of N IDs (e.g., 100) to reduce FDB transaction overhead.

#### Site ID Assignment

Site IDs are explicitly configured via environment variable or configuration file. Each site must have a unique `site_id` (u8, 0-255).

**Configuration:**

```bash
# Environment variable
PELAGO_SITE_ID=1
PELAGO_SITE_NAME=sf

# Or in config file
[site]
id = 1
name = "sf"
```

**Collision Guard:**

To prevent catastrophic data corruption from misconfigured site IDs (e.g., copied config files), PelagoDB enforces a **startup collision check**:

```rust
async fn validate_site_id(
    db: &FdbDatabase,
    site_id: u8,
    site_name: &str,
) -> Result<()> {
    let tx = db.create_transaction()?;
    let claim_key = ("_sys", "site_claim", site_id);

    match tx.get(&claim_key).await? {
        Some(existing_name) => {
            let existing = String::from_utf8_lossy(&existing_name);
            if existing != site_name {
                // FATAL: Another site already owns this ID
                panic!(
                    "FATAL: Site ID {} already claimed by '{}', refusing to start as '{}'. \
                     This prevents silent data corruption from duplicate site IDs. \
                     Either use a different site_id or investigate why two sites have the same ID.",
                    site_id, existing, site_name
                );
            }
            // We already own this ID, continue
        }
        None => {
            // First time this site_id is used, claim it
            tx.set(&claim_key, site_name.as_bytes());
            tx.commit().await?;
        }
    }
    Ok(())
}
```

**Behavior:**

| Scenario | Result |
|----------|--------|
| First startup, ID unclaimed | Claim succeeds, site starts |
| Restart, same ID and name | Validation passes, site starts |
| Different site, same ID | **FATAL ERROR**, site refuses to start |
| Same site, different name | **FATAL ERROR**, name mismatch detected |

**Rationale:**

This design combines the simplicity of explicit IDs with automatic collision detection:

- **No complex self-registration:** Site ID is deterministic from config
- **No FDB dependency for ID value:** ID is known before database connection
- **Loud failure:** Misconfiguration causes immediate crash, not silent corruption
- **Operational clarity:** The site→ID mapping is visible in `/_sys/site_claim/`

**Reclaiming a Site ID:**

If a site is permanently decommissioned and its ID needs reassignment:

```bash
# Clear the claim (admin operation)
pelago admin clear-site-claim --site-id 3
```

This removes the `(_sys, site_claim, 3)` key, allowing the ID to be claimed by a new site.

### 4.2 Edge ID Format

Edge IDs use the same 9-byte format as Node IDs:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EdgeId {
    pub site: u8,    // Site that created the edge
    pub seq: u64,    // Monotonic counter per site
}
```

Edge IDs are allocated from a separate counter than Node IDs:

```
Key: (db, ns, _ids, "_edges", site_id)
Value: u64
```

### 4.3 Property Types

PelagoDB supports the following property types:

| Type | Rust Mapping | CBOR Encoding | CEL Type | Index Support |
|------|--------------|---------------|----------|---------------|
| `string` | `String` | Text string | `string` | unique, equality, range |
| `int` | `i64` | Integer | `int` | unique, equality, range |
| `float` | `f64` | Float | `double` | equality, range |
| `bool` | `bool` | Boolean | `bool` | equality |
| `timestamp` | `i64` (unix micros) | Integer | `timestamp` | unique, equality, range |
| `bytes` | `Vec<u8>` | Byte string | `bytes` | equality |

**Rust Value Enum:**

```rust
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Timestamp(i64),    // Unix microseconds
    Bytes(Vec<u8>),
    Null,
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::String(_) => "string",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Bool(_) => "bool",
            Value::Timestamp(_) => "timestamp",
            Value::Bytes(_) => "bytes",
            Value::Null => "null",
        }
    }
}
```

### 4.4 Null Semantics

PelagoDB uses a unified null model: **missing properties and null values are indistinguishable**. Both represent "this property has no value."

#### Core Rules

1. **Missing = Null:** If a property is not present in a node's CBOR blob, it is null. There is no distinction between "field absent" and "field set to null."

2. **Null not stored:** Null values are not stored in CBOR. The absence of a key IS the null representation.

3. **Required fields:** Properties marked `required: true` cannot be null. The API layer enforces this at write time.

4. **Default values:** If a property has a `default` value, it is applied at node creation time if the field is omitted. After defaults are applied, the field is present (not null).

5. **Null in responses:** Missing/null fields are absent from API responses. Client SDKs represent them using language-native optional types.

#### Null in CEL Expressions

All comparison operators have defined behavior when a field is null:

| CEL Expression | Field is Null | Result | Rationale |
|----------------|---------------|--------|-----------|
| `age > 30` | yes | `false` | Null does not satisfy any comparison |
| `age >= 30` | yes | `false` | Null does not satisfy any comparison |
| `age < 30` | yes | `false` | Null does not satisfy any comparison |
| `age <= 30` | yes | `false` | Null does not satisfy any comparison |
| `age == 30` | yes | `false` | Null is not equal to any value |
| `age == null` | yes | `true` | This is how to query for missing fields |
| `age != null` | yes | `false` | Null is equal to null |
| `age != 30` | yes | `false` | Null is not equal to 30, but `!=` on null is false |
| `name.startsWith("A")` | yes | `false` | String operations on null return false |
| `bio.contains("X")` | yes | `false` | String operations on null return false |

**Boolean logic with null:**

| Expression | Result | Rationale |
|------------|--------|-----------|
| `null && true` | `false` | Null is falsy |
| `null && false` | `false` | AND with false is false |
| `null \|\| true` | `true` | OR with true is true |
| `null \|\| false` | `false` | Null is falsy, false is falsy |

#### Null in Indexes

**Null values are NOT indexed.** If a property is indexed and a node has no value for that property, no index entry is created.

Consequences:

1. **Range scans exclude null nodes correctly.** A query like `age >= 30` naturally excludes nodes without an age index entry.

2. **Null equality queries require full scan.** A query like `age == null` cannot use the age index and must scan all nodes with a residual filter.

3. **Unique index allows multiple nulls.** Multiple nodes can have null for a unique-indexed field (no constraint on null values).

#### Query Examples

```cel
// Find people aged 30 or older (excludes null age)
age >= 30

// Find people without an age set
age == null

// Find people with an age set (any value)
age != null

// Find active people or people older than 30
// (null active treated as false)
active || age > 30
```

---

## 5. Schema System

### 5.1 Entity Schema Structure

Every entity type in PelagoDB has a schema that defines its properties, edges, and validation rules.

**Rust Definition:**

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EntitySchema {
    /// Database this schema belongs to
    pub db: String,
    /// Namespace this schema belongs to
    pub namespace: String,
    /// Entity type name (e.g., "Person", "Company")
    pub name: String,
    /// Schema version (monotonically increasing)
    pub version: u32,
    /// Property definitions
    pub properties: HashMap<String, PropertyDef>,
    /// Edge type declarations
    pub edges: HashMap<String, EdgeDef>,
    /// Schema-level configuration
    pub meta: SchemaMeta,
    /// Creation timestamp
    pub created_at: i64,
    /// Creating user/service
    pub created_by: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SchemaMeta {
    /// Allow edge types not declared in schema
    pub allow_undeclared_edges: bool,
    /// Policy for properties not in schema: "reject", "allow", "warn"
    pub extras_policy: ExtrasPolicy,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ExtrasPolicy {
    Reject,  // Reject writes with unknown properties
    Allow,   // Accept and store unknown properties
    Warn,    // Accept but log warning
}
```

**JSON Schema Example:**

```json
{
  "db": "production",
  "namespace": "core",
  "name": "Person",
  "version": 1,
  "properties": {
    "name": {
      "type": "string",
      "required": true,
      "index": "unique"
    },
    "email": {
      "type": "string",
      "required": true,
      "index": "unique"
    },
    "age": {
      "type": "int",
      "index": "range"
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
        "since": { "type": "timestamp", "sort_key": true }
      }
    },
    "WORKS_AT": {
      "target": "Company",
      "direction": "outgoing",
      "properties": {
        "role": { "type": "string", "index": "equality" },
        "started": { "type": "timestamp", "sort_key": true }
      }
    }
  },
  "meta": {
    "allow_undeclared_edges": true,
    "extras_policy": "reject"
  }
}
```

### 5.2 Property Definitions

Each property in a schema has the following attributes:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PropertyDef {
    /// Property type
    pub property_type: PropertyType,
    /// Whether the property is required (cannot be null)
    pub required: bool,
    /// Index type (if indexed)
    pub index: Option<IndexType>,
    /// Default value (applied at creation if omitted)
    pub default: Option<DefaultValue>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PropertyType {
    String,
    Int,
    Float,
    Bool,
    Timestamp,
    Bytes,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum DefaultValue {
    /// Literal value
    Literal(Value),
    /// Special "now" for timestamps (current time at creation)
    Now,
}
```

**Property Attributes:**

| Attribute | Type | Description |
|-----------|------|-------------|
| `type` | PropertyType | The data type of the property |
| `required` | bool | If true, property must be present (cannot be null) |
| `index` | IndexType? | Optional index: "unique", "equality", or "range" |
| `default` | DefaultValue? | Value to use if property is omitted at creation |

**Validation Rules:**

1. **Type checking:** Property values must match the declared type
2. **Required fields:** Must be present in create/update requests
3. **Default application:** Applied at creation time only, not retroactively

### 5.3 Edge Declarations

Edge declarations define the relationships an entity type can have:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EdgeDef {
    /// Target entity type: specific type name or "*" for polymorphic
    pub target: EdgeTarget,
    /// Edge direction: outgoing or bidirectional
    pub direction: EdgeDirection,
    /// Properties on the edge itself
    pub properties: HashMap<String, PropertyDef>,
    /// Property to use as sort key in adjacency list
    pub sort_key: Option<String>,
    /// Ownership mode for multi-site replication
    pub ownership: OwnershipMode,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum EdgeTarget {
    /// Specific entity type (e.g., "Company")
    Specific(String),
    /// Any entity type (polymorphic edge)
    Polymorphic,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum EdgeDirection {
    /// Edge goes from source to target only
    Outgoing,
    /// Edge is bidirectional (creates paired edges)
    Bidirectional,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum OwnershipMode {
    /// Edge owned by source node's home site (default)
    SourceSite,
    /// Edge owned by the site that created it
    Independent,
}
```

**Edge Declaration Examples:**

```json
{
  "WORKS_AT": {
    "target": "Company",
    "direction": "outgoing",
    "properties": {
      "role": { "type": "string" },
      "started": { "type": "timestamp", "sort_key": true }
    }
  },
  "KNOWS": {
    "target": "Person",
    "direction": "bidirectional",
    "properties": {
      "since": { "type": "timestamp", "sort_key": true }
    }
  },
  "TAGGED_WITH": {
    "target": "*",
    "direction": "outgoing",
    "properties": {
      "relevance": { "type": "float" }
    }
  }
}
```

**Forward References:**

Edge declarations can reference entity types that are not yet registered. This enables incremental schema development across teams:

- Schema registration succeeds even if target type doesn't exist
- Creating actual edges to unregistered types is rejected
- When target type is later registered, edges become creatable

### 5.4 Index Types

PelagoDB supports three index types:

#### Unique Index

Enforces that no two nodes have the same value for the indexed property.

**Key Pattern:**
```
(db, ns, idx, unique, <entity_type>, <field>, <value>) → <node_id>
```

**Behavior:**
- Write-time: Read index key first; reject if exists
- Query: Single key get returns node ID
- Null handling: Null values are not indexed; multiple nodes can have null

**Use Cases:** Email, username, SKU, external IDs

#### Equality Index

Enables fast lookup by exact value match. Multiple nodes can share the same value.

**Key Pattern:**
```
(db, ns, idx, eq, <entity_type>, <field>, <value>, <node_id>) → empty
```

**Behavior:**
- Write-time: Append node ID to value's entry list
- Query: Range scan on `(type, field, value)` prefix returns all matching node IDs
- Null handling: Null values are not indexed

**Use Cases:** Status, category, type, boolean flags

#### Range Index

Enables range queries (>, >=, <, <=) and equality. Values must be sort-order encoded.

**Key Pattern:**
```
(db, ns, idx, range, <entity_type>, <field>, <encoded_value>, <node_id>) → empty
```

**Behavior:**
- Write-time: Encode value in sort order; append node ID
- Query: Range scan with bounds on encoded value
- Null handling: Null values are not indexed

**Encoding:** Values are encoded to preserve sort order:

```rust
fn encode_for_range_index(value: &Value) -> Vec<u8> {
    match value {
        Value::Int(n) => {
            // Flip sign bit for correct signed integer ordering
            let mut bytes = n.to_be_bytes();
            bytes[0] ^= 0x80;  // Flip sign bit
            bytes.to_vec()
        }
        Value::Float(f) => {
            // IEEE 754 order-preserving encoding
            let bits = f.to_bits();
            let encoded = if bits & 0x8000_0000_0000_0000 != 0 {
                !bits  // Negative: invert all bits
            } else {
                bits ^ 0x8000_0000_0000_0000  // Positive: flip sign bit
            };
            encoded.to_be_bytes().to_vec()
        }
        Value::String(s) => s.as_bytes().to_vec(),
        Value::Timestamp(t) => {
            // Same as Int encoding
            let mut bytes = t.to_be_bytes();
            bytes[0] ^= 0x80;
            bytes.to_vec()
        }
        _ => panic!("Type not supported for range index"),
    }
}
```

**Use Cases:** Age, price, dates, scores, any numeric/temporal data

### 5.5 Schema Evolution Rules

Schemas evolve over time. PelagoDB supports additive changes with explicit cleanup.

#### Adding Properties

| Change | Behavior |
|--------|----------|
| Add non-indexed property | Immediate. Old nodes return null for new property. |
| Add indexed property | Triggers background index backfill job. Index is incomplete until job finishes. |
| Add required property | Only affects new nodes. Old nodes are not validated. |
| Add property with default | Default applied to new nodes only, not retroactively. |

#### Removing Properties

When a property is removed from the schema:

1. **Server stops validating the property.** New writes cannot include it.

2. **Old data remains in CBOR.** The property stays in existing nodes' serialized blobs but is invisible to queries.

3. **CEL type-check fails.** Queries referencing the removed property fail at compile time.

4. **Indexes become orphaned.** Index entries remain but are never used.

**Explicit Cleanup RPCs:**

```
DropIndex(namespace, entity_type, property_name)
  → Synchronously deletes all index entries for the property
  → FDB clearRange on (db, ns, idx, *, entity_type, property)

StripProperty(namespace, entity_type, property_name)
  → Enqueues background job to remove property from CBOR blobs
  → For each node: deserialize, remove key, re-serialize, write back
  → Reclaims storage; optional cleanup
```

#### Version Management

- Schema version is a monotonically increasing u32
- Each registration must increment the version
- Old versions are retained in `(_schema, type, v, version)` for reference
- Current version pointer is `(_schema, type, latest)`

#### Multi-Site Schema Compatibility

When replicating CDC entries across sites with different schema versions:

1. Load local schema for the entity type
2. Deserialize CDC entry
3. **Strip fields not in local schema** (silently ignored)
4. Apply the operation

Sites converge gracefully. Old CDC entries with removed fields work fine.

### 5.6 Schema Registry Storage

Schemas are stored in FDB under the namespace's `_schema` subspace:

**Current Schema:**
```
Key: (db, ns, _schema, <entity_type>, latest)
Value: u32 (current version number)

Key: (db, ns, _schema, <entity_type>, v, <version>)
Value: CBOR { EntitySchema }
```

**Schema Lookup Flow:**

```
1. Read (db, ns, _schema, entity_type, latest) → version number
2. Read (db, ns, _schema, entity_type, v, version) → schema CBOR
3. Cache in memory with version check
4. Invalidate on RegisterSchema
```

**In-Memory Cache:**

```rust
struct SchemaCache {
    schemas: HashMap<(String, String, String), Arc<EntitySchema>>,
    // Key: (db, namespace, entity_type)
    // Value: Arc<EntitySchema> with version embedded
}

impl SchemaCache {
    fn get(&self, db: &str, ns: &str, entity_type: &str) -> Option<Arc<EntitySchema>> {
        // Return cached if present
    }

    fn invalidate(&mut self, db: &str, ns: &str, entity_type: &str) {
        // Remove from cache; next access will reload from FDB
    }
}
```

**Type Registry:**

The `_types` subspace provides fast enumeration of entity types:

```
Key: (db, ns, _types, <entity_type>)
Value: CBOR { created_at, entity_count, latest_schema }
```

Updated when:
- Schema is registered (creates/updates type entry)
- Entity count changes (async background update)

---

## 6. Node Operations

This section defines the complete lifecycle of node operations including validation, storage, indexing, and CDC emission.

### 6.1 Create Node

Node creation follows a single-transaction write path that atomically performs validation, storage, indexing, and CDC emission.

#### Validation Pipeline

```
CreateNode request arrives
    │
    ▼
┌─────────────────────────────────────────────┐
│ 1. Load schema for entity type               │
│    Cache hit → use cached schema             │
│    Cache miss → load from FDB → populate     │
└──────────────────┬──────────────────────────┘
    │
    ▼
┌─────────────────────────────────────────────┐
│ 2. Validate entity type is registered        │
│    Not registered → REJECT (NOT_FOUND)       │
└──────────────────┬──────────────────────────┘
    │
    ▼
┌─────────────────────────────────────────────┐
│ 3. Validate properties against schema        │
│    a. Type check each property value         │
│    b. Check required fields are present      │
│    c. Check extras_policy for unknown props  │
└──────────────────┬──────────────────────────┘
    │
    ▼
┌─────────────────────────────────────────────┐
│ 4. Apply default values                      │
│    For each property with default:           │
│      if missing → apply default              │
│      if "now" → apply current timestamp      │
└──────────────────┬──────────────────────────┘
    │
    ▼
┌─────────────────────────────────────────────┐
│ 5. Single FDB Transaction:                   │
│    a. Allocate NodeId from counter           │
│    b. Write data key                         │
│    c. Write index entries (all indexed props)│
│    d. Write CDC entry                        │
│    e. Commit                                 │
└──────────────────┬──────────────────────────┘
    │
    ▼
   ACK to client with NodeId
```

#### ID Allocation (Batch Strategy)

Node IDs are allocated using atomic counters in FDB with batch allocation to reduce contention:

```
Key: (db, ns, _ids, entity_type, site_id)
Value: u64 (next available sequence number)
```

**Batch Allocation Strategy:**

```rust
struct IdAllocator {
    site_id: u8,
    current_block: (u64, u64),  // (start, end) of current block
    block_size: u64,            // Default: 100
}

impl IdAllocator {
    async fn allocate(&mut self, db: &FdbDatabase, entity_type: &str) -> Result<NodeId> {
        // Check if current block is exhausted
        if self.current_block.0 >= self.current_block.1 {
            // Allocate new block atomically
            let tx = db.create_transaction()?;
            let counter_key = (db_id, ns, "_ids", entity_type, self.site_id);

            let current = tx.get(&counter_key).await?
                .map(|v| u64::from_le_bytes(v.try_into().unwrap()))
                .unwrap_or(0);

            let new_start = current;
            let new_end = current + self.block_size;

            tx.set(&counter_key, &new_end.to_le_bytes());
            tx.commit().await?;

            self.current_block = (new_start, new_end);
        }

        let seq = self.current_block.0;
        self.current_block.0 += 1;

        Ok(NodeId { site: self.site_id, seq })
    }
}
```

**Properties:**
- **No cross-site coordination:** Each site has independent counters
- **Low contention:** Batch allocation reduces FDB transaction frequency
- **Crash safety:** Unused IDs in a block are lost on crash (acceptable)
- **Block size:** Configurable; default 100 balances contention vs. waste

#### Index Entry Creation

For each indexed property in the schema, create the appropriate index entry:

```rust
fn create_index_entries(
    tx: &FdbTransaction,
    schema: &EntitySchema,
    node_id: &NodeId,
    properties: &HashMap<String, Value>,
) -> Result<()> {
    for (prop_name, prop_def) in &schema.properties {
        if let Some(index_type) = &prop_def.index {
            let value = properties.get(prop_name);

            // Null values are NOT indexed
            if value.is_none() || matches!(value, Some(Value::Null)) {
                continue;
            }

            let value = value.unwrap();

            match index_type {
                IndexType::Unique => {
                    // Unique: (db, ns, idx, unique, type, field, value) → node_id
                    let key = (db, ns, "idx", "unique", &schema.name, prop_name,
                               encode_for_index(value));

                    // Check-then-write: reject if exists
                    if tx.get(&key).await?.is_some() {
                        return Err(PelagoError::UniqueConstraintViolation {
                            entity_type: schema.name.clone(),
                            field: prop_name.clone(),
                            value: value.clone(),
                        });
                    }

                    tx.set(&key, &node_id.to_bytes());
                }
                IndexType::Equality => {
                    // Equality: (db, ns, idx, eq, type, field, value, node_id) → empty
                    let key = (db, ns, "idx", "eq", &schema.name, prop_name,
                               encode_for_index(value), node_id.to_bytes());
                    tx.set(&key, &[]);
                }
                IndexType::Range => {
                    // Range: (db, ns, idx, range, type, field, encoded_value, node_id) → empty
                    let key = (db, ns, "idx", "range", &schema.name, prop_name,
                               encode_for_range_index(value), node_id.to_bytes());
                    tx.set(&key, &[]);
                }
            }
        }
    }
    Ok(())
}
```

#### CDC Entry

Every node creation appends a CDC entry in the same transaction:

```rust
fn append_cdc_entry(
    tx: &FdbTransaction,
    site: &str,
    operation: CdcOperation,
) -> Result<()> {
    let entry = CdcEntry {
        site: site.to_string(),
        timestamp: current_time_micros(),
        batch_id: None,
        operations: vec![operation],
    };

    // Key uses versionstamp for zero-contention ordering
    let key = (db, ns, "_cdc", tx.get_versionstamp());
    tx.set(&key, &encode_cbor(&entry));

    Ok(())
}

// CdcOperation for node creation
CdcOperation::NodeCreate {
    entity_type: schema.name.clone(),
    node_id: node_id.clone(),
    properties: properties.clone(),
    home_site: site.to_string(),
}
```

### 6.2 Get Node (Point Lookup)

Point lookup is the fastest read path: a single FDB `get()` operation.

```rust
async fn get_node(
    db: &FdbDatabase,
    entity_type: &str,
    node_id: &NodeId,
    consistency: ReadConsistency,
) -> Result<Option<Node>> {
    // Build data key
    let key = (db_id, ns, "data", entity_type, node_id.to_bytes());

    match consistency {
        ReadConsistency::Strong => {
            // Direct FDB read at latest committed version
            let tx = db.create_transaction()?;
            let read_version = tx.get_read_version().await?;
            tx.set_read_version(read_version);

            let value = tx.get(&key).await?;
            value.map(|v| deserialize_node(&v, entity_type, node_id))
        }
        ReadConsistency::Session => {
            // Try RocksDB cache first, verify freshness
            if let Some(cached) = rocksdb_cache.get(&key) {
                let read_version = db.get_read_version().await?;
                let cache_hwm = get_cache_high_water_mark().await?;
                if cache_hwm >= read_version {
                    return Ok(Some(cached));
                }
            }
            // Fall through to FDB
            let tx = db.create_transaction()?;
            tx.get(&key).await?.map(|v| deserialize_node(&v, entity_type, node_id))
        }
        ReadConsistency::Eventual => {
            // RocksDB cache only, fall back to FDB on miss
            if let Some(cached) = rocksdb_cache.get(&key) {
                return Ok(Some(cached));
            }
            let tx = db.create_transaction()?;
            tx.get(&key).await?.map(|v| deserialize_node(&v, entity_type, node_id))
        }
    }
}

fn deserialize_node(
    cbor_bytes: &[u8],
    entity_type: &str,
    node_id: &NodeId,
) -> Node {
    let record: NodeRecord = decode_cbor(cbor_bytes);

    Node {
        id: node_id.to_string(),
        entity_type: entity_type.to_string(),
        properties: record.p,  // payload
        locality: record.l,
        created_at: record.c,
        updated_at: record.u,
    }
}
```

**Performance:** Sub-millisecond at the storage layer (single FDB network round-trip).

### 6.3 Update Node

Node updates follow a read-modify-write pattern with optimistic concurrency.

#### Index Maintenance (Delete Old, Add New)

When updating indexed properties, the old index entries must be deleted and new ones created:

```rust
async fn update_node(
    db: &FdbDatabase,
    entity_type: &str,
    node_id: &NodeId,
    changed_properties: HashMap<String, Value>,
) -> Result<Node> {
    let schema = get_schema(entity_type).await?;

    loop {
        let tx = db.create_transaction()?;

        // Read current node data
        let key = (db_id, ns, "data", entity_type, node_id.to_bytes());
        let current = tx.get(&key).await?
            .ok_or(PelagoError::NodeNotFound)?;
        let current_props: HashMap<String, Value> = decode_cbor(&current);

        // Validate changed properties against schema
        validate_properties(&schema, &changed_properties)?;

        // Compute index diff
        let index_diff = compute_index_diff(&schema, &current_props, &changed_properties);

        // Delete old index entries
        for (prop_name, old_value) in &index_diff.removed {
            delete_index_entry(&tx, &schema, prop_name, old_value, node_id)?;
        }

        // Add new index entries
        for (prop_name, new_value) in &index_diff.added {
            create_index_entry(&tx, &schema, prop_name, new_value, node_id)?;
        }

        // Merge properties
        let mut new_props = current_props.clone();
        for (k, v) in changed_properties {
            if matches!(v, Value::Null) {
                new_props.remove(&k);  // Null = remove property
            } else {
                new_props.insert(k, v);
            }
        }

        // Write updated data
        let record = NodeRecord {
            p: new_props.clone(),
            l: current_locality,
            c: current_created_at,
            u: current_time_micros(),
        };
        tx.set(&key, &encode_cbor(&record));

        // Append CDC entry
        append_cdc_entry(&tx, CdcOperation::NodeUpdate {
            entity_type: entity_type.to_string(),
            node_id: node_id.clone(),
            changed_properties: changed_properties.clone(),
            previous_version: current_version,
        })?;

        // Commit with optimistic concurrency
        match tx.commit().await {
            Ok(_) => break,
            Err(e) if e.is_retryable() => continue,  // Retry on conflict
            Err(e) => return Err(e.into()),
        }
    }

    Ok(node)
}
```

#### Optimistic Concurrency

FDB provides automatic conflict detection through read conflict ranges:

1. **Read current state:** Transaction reads the node data key
2. **Apply changes:** Compute new state, write updates
3. **Commit:** If another transaction modified the same key, commit fails
4. **Retry:** Loop retries with fresh read

This ensures serializable isolation without explicit locking.

### 6.4 Delete Node

Node deletion removes all data, indexes, and edges, then emits a CDC entry.

```rust
async fn delete_node(
    db: &FdbDatabase,
    entity_type: &str,
    node_id: &NodeId,
) -> Result<()> {
    let schema = get_schema(entity_type).await?;
    let tx = db.create_transaction()?;

    // 1. Read current node data (need property values for index cleanup)
    let key = (db_id, ns, "data", entity_type, node_id.to_bytes());
    let current = tx.get(&key).await?
        .ok_or(PelagoError::NodeNotFound)?;
    let current_props: HashMap<String, Value> = decode_cbor(&current);

    // 2. Delete data key
    tx.clear(&key);

    // 3. Delete all index entries for this node
    for (prop_name, prop_def) in &schema.properties {
        if let Some(index_type) = &prop_def.index {
            if let Some(value) = current_props.get(prop_name) {
                delete_index_entry(&tx, &schema, prop_name, value, node_id)?;
            }
        }
    }

    // 4. Delete locality index entry
    let loc_key = (db_id, ns, "loc", entity_type, node_id.to_bytes());
    tx.clear(&loc_key);

    // 5. Delete all outgoing edges
    let out_prefix = (db_id, ns, "edge", "f", entity_type, node_id.to_bytes());
    tx.clear_range(&out_prefix.range());
    let out_meta_prefix = (db_id, ns, "edge", "fm", entity_type, node_id.to_bytes());
    tx.clear_range(&out_meta_prefix.range());

    // 6. Handle incoming edge cleanup (edge cascade)
    delete_incoming_edges(&tx, entity_type, node_id).await?;

    // 7. Append CDC entry
    append_cdc_entry(&tx, CdcOperation::NodeDelete {
        entity_type: entity_type.to_string(),
        node_id: node_id.clone(),
    })?;

    tx.commit().await?;
    Ok(())
}
```

#### Edge Cascade (Incoming Edge Cleanup)

When a node is deleted, all edges pointing TO it must be cleaned up:

```rust
async fn delete_incoming_edges(
    tx: &FdbTransaction,
    entity_type: &str,
    node_id: &NodeId,
) -> Result<()> {
    // Scan reverse edge index to find all incoming edges
    let reverse_prefix = (db_id, ns, "edge", "r");

    // Range scan for edges targeting this node
    // Key: (r, tgt_db, tgt_ns, tgt_type, tgt_id, label, src_type, src_id)
    let range = tx.get_range(&reverse_prefix.subrange(
        (db_id, ns, entity_type, node_id.to_bytes())
    )).await?;

    for (key, _) in range {
        // Extract source info from reverse edge key
        let (_, _, _, _, _, label, src_type, src_id) = decode_tuple(&key);

        // Delete the forward edge at source
        let fwd_key = (db_id, ns, "edge", "f", src_type, src_id, label,
                       db_id, ns, entity_type, node_id.to_bytes());
        tx.clear(&fwd_key);

        // Delete the forward edge metadata if present
        let fwd_meta_key = (db_id, ns, "edge", "fm", src_type, src_id, label,
                           db_id, ns, entity_type, node_id.to_bytes());
        tx.clear(&fwd_meta_key);

        // Delete the reverse edge entry
        tx.clear(&key);
    }

    Ok(())
}
```

**Complexity:** O(number of incoming edges). For high-degree nodes, this may be scheduled as a background job instead of inline deletion.

---

## 7. Edge Operations

This section defines edge storage, creation, deletion, and querying patterns.

### 7.1 Edge Storage Model

Edges are stored as paired entries: a forward edge at the source and a reverse edge for index lookup.

#### OUT/IN Paired Entries

Every edge creates two FDB entries:

```
Forward Edge (f):
  Key:   (db, ns, edge, f, <src_type>, <src_id>, <label>, <tgt_db>, <tgt_ns>, <tgt_type>, <tgt_id>)
  Value: timestamp (i64, creation time in microseconds)

Forward Edge Metadata (fm) [optional, only if edge has properties]:
  Key:   (db, ns, edge, fm, <src_type>, <src_id>, <label>, <tgt_db>, <tgt_ns>, <tgt_type>, <tgt_id>)
  Value: CBOR { properties, pair_id?, owner }

Reverse Edge (r):
  Key:   (db, ns, edge, r, <tgt_db>, <tgt_ns>, <tgt_type>, <tgt_id>, <label>, <src_type>, <src_id>)
  Value: 0x01 (existence marker)
```

#### Sort Key in Key Position

When an edge type declares a `sort_key` property, that value is encoded in the forward edge key for efficient range scans:

```
Key: (db, ns, edge, f, <src_type>, <src_id>, <label>, <sort_key_value>, <tgt_db>, <tgt_ns>, <tgt_type>, <tgt_id>)
```

This enables vertex-centric indexes: scanning edges from a node sorted by the sort key property (e.g., all WORKS_AT edges sorted by `started` timestamp).

### 7.2 Create Edge

Edge creation validates referential integrity, creates paired entries, and emits CDC.

#### Referential Integrity Validation

```rust
async fn create_edge(
    db: &FdbDatabase,
    source: &NodeRef,
    target: &NodeRef,
    label: &str,
    properties: HashMap<String, Value>,
) -> Result<EdgeId> {
    // 1. Validate source node exists
    let src_key = (db_id, ns, "data", &source.entity_type, source.id.to_bytes());
    let tx = db.create_transaction()?;
    if tx.get(&src_key).await?.is_none() {
        return Err(PelagoError::SourceNotFound {
            entity_type: source.entity_type.clone(),
            node_id: source.id.clone(),
        });
    }

    // 2. Validate target node exists
    let tgt_key = (target.db, target.ns, "data", &target.entity_type, target.id.to_bytes());
    if tx.get(&tgt_key).await?.is_none() {
        return Err(PelagoError::TargetNotFound {
            entity_type: target.entity_type.clone(),
            node_id: target.id.clone(),
        });
    }

    // 3. Load source schema and validate edge type
    let schema = get_schema(&source.entity_type).await?;
    if let Some(edge_def) = schema.edges.get(label) {
        // Validate target type matches declaration
        match &edge_def.target {
            EdgeTarget::Specific(expected_type) => {
                if &target.entity_type != expected_type {
                    return Err(PelagoError::TargetTypeMismatch {
                        edge_type: label.to_string(),
                        expected: expected_type.clone(),
                        actual: target.entity_type.clone(),
                    });
                }
            }
            EdgeTarget::Polymorphic => {
                // Any type allowed
            }
        }

        // Validate edge properties
        validate_edge_properties(edge_def, &properties)?;
    } else {
        // Undeclared edge type
        if !schema.meta.allow_undeclared_edges {
            return Err(PelagoError::UndeclaredEdgeType {
                entity_type: source.entity_type.clone(),
                edge_type: label.to_string(),
            });
        }
    }

    // 4. Allocate edge ID
    let edge_id = allocate_edge_id(db, &source.entity_type).await?;

    // 5. Create edge entries
    create_edge_entries(&tx, source, target, label, &properties, &edge_id).await?;

    // 6. Append CDC entry
    append_cdc_entry(&tx, CdcOperation::EdgeCreate {
        source: source.clone(),
        target: target.clone(),
        edge_type: label.to_string(),
        edge_id: edge_id.clone(),
        properties: properties.clone(),
        owner_site: get_current_site().to_string(),
    })?;

    tx.commit().await?;
    Ok(edge_id)
}
```

#### Bidirectional Edge Pairs (pair_id)

Bidirectional edges are implemented as two paired directed edges sharing a `pair_id`:

```rust
async fn create_bidirectional_edge(
    tx: &FdbTransaction,
    source: &NodeRef,
    target: &NodeRef,
    label: &str,
    properties: &HashMap<String, Value>,
) -> Result<String> {
    // Generate pair_id
    let pair_id = generate_uuid_v7();

    // Create forward edge: source → target
    let edge_id_out = format!("{}_out", pair_id);
    create_single_edge(tx, source, target, label, properties, &edge_id_out, Some(&pair_id))?;

    // Create reverse edge: target → source (same label, same properties)
    let edge_id_in = format!("{}_in", pair_id);
    create_single_edge(tx, target, source, label, properties, &edge_id_in, Some(&pair_id))?;

    Ok(pair_id)
}
```

**Ownership:** Each direction is owned by its source node's home site. For a bidirectional KNOWS edge between Person_42 (site_a) and Person_99 (site_b):
- Edge from Person_42 → Person_99: owned by site_a
- Edge from Person_99 → Person_42: owned by site_b

### 7.3 Delete Edge

#### Single Edge Deletion

```rust
async fn delete_edge(
    db: &FdbDatabase,
    source: &NodeRef,
    target: &NodeRef,
    label: &str,
) -> Result<()> {
    let tx = db.create_transaction()?;

    // Build forward edge key
    let fwd_key = (db_id, ns, "edge", "f", &source.entity_type, source.id.to_bytes(),
                   label, target.db, target.ns, &target.entity_type, target.id.to_bytes());

    // Check edge exists and get metadata
    let fwd_value = tx.get(&fwd_key).await?
        .ok_or(PelagoError::EdgeNotFound)?;

    // Delete forward edge
    tx.clear(&fwd_key);

    // Delete forward edge metadata if present
    let fwd_meta_key = (db_id, ns, "edge", "fm", &source.entity_type, source.id.to_bytes(),
                        label, target.db, target.ns, &target.entity_type, target.id.to_bytes());
    tx.clear(&fwd_meta_key);

    // Delete reverse edge
    let rev_key = (target.db, target.ns, "edge", "r", target.db, target.ns,
                   &target.entity_type, target.id.to_bytes(),
                   label, &source.entity_type, source.id.to_bytes());
    tx.clear(&rev_key);

    // Append CDC entry
    append_cdc_entry(&tx, CdcOperation::EdgeDelete {
        source: source.clone(),
        target: target.clone(),
        edge_type: label.to_string(),
    })?;

    tx.commit().await?;
    Ok(())
}
```

#### Paired Bidirectional Deletion

When deleting a bidirectional edge, delete both directions:

```rust
async fn delete_bidirectional_edge(
    db: &FdbDatabase,
    source: &NodeRef,
    target: &NodeRef,
    label: &str,
) -> Result<()> {
    let tx = db.create_transaction()?;

    // Get edge metadata to find pair_id
    let fwd_meta_key = (db_id, ns, "edge", "fm", &source.entity_type, source.id.to_bytes(),
                        label, target.db, target.ns, &target.entity_type, target.id.to_bytes());
    let metadata = tx.get(&fwd_meta_key).await?;

    if let Some(meta_bytes) = metadata {
        let meta: EdgeMetadata = decode_cbor(&meta_bytes);

        if let Some(pair_id) = meta.pair_id {
            // Delete both directions
            delete_single_edge(&tx, source, target, label)?;
            delete_single_edge(&tx, target, source, label)?;

            // Append CDC entry with pair_id
            append_cdc_entry(&tx, CdcOperation::EdgeDelete {
                source: source.clone(),
                target: target.clone(),
                edge_type: label.to_string(),
                pair_id: Some(pair_id),
            })?;
        }
    }

    tx.commit().await?;
    Ok(())
}
```

### 7.4 List Edges

#### Adjacency List Scan

List all edges of a specific type from a node:

```rust
async fn list_edges(
    db: &FdbDatabase,
    node: &NodeRef,
    label: Option<&str>,
    direction: EdgeDirection,
    limit: u32,
    cursor: Option<&[u8]>,
) -> Result<(Vec<Edge>, Option<Vec<u8>>)> {
    let tx = db.create_transaction()?;

    let prefix = match direction {
        EdgeDirection::Outgoing => {
            if let Some(l) = label {
                (db_id, ns, "edge", "f", &node.entity_type, node.id.to_bytes(), l)
            } else {
                (db_id, ns, "edge", "f", &node.entity_type, node.id.to_bytes())
            }
        }
        EdgeDirection::Incoming => {
            // Use reverse index
            if let Some(l) = label {
                (db_id, ns, "edge", "r", db_id, ns, &node.entity_type, node.id.to_bytes(), l)
            } else {
                (db_id, ns, "edge", "r", db_id, ns, &node.entity_type, node.id.to_bytes())
            }
        }
    };

    let range_start = cursor.unwrap_or(&prefix.pack());
    let range_end = prefix.range_end();

    let results = tx.get_range(
        &range_start,
        &range_end,
        RangeOption { limit: limit + 1, ..default() }
    ).await?;

    let has_more = results.len() > limit as usize;
    let edges: Vec<Edge> = results.iter()
        .take(limit as usize)
        .map(|(key, value)| decode_edge(key, value, direction))
        .collect();

    let next_cursor = if has_more {
        Some(results[limit as usize].0.clone())
    } else {
        None
    };

    Ok((edges, next_cursor))
}
```

#### Vertex-Centric Indexes (Sort Key Range Scans)

When an edge type has a `sort_key` property, range queries on that property compile to FDB range scans:

```rust
async fn list_edges_with_sort_key_filter(
    db: &FdbDatabase,
    node: &NodeRef,
    label: &str,
    sort_key_lower: Option<Value>,
    sort_key_upper: Option<Value>,
    limit: u32,
) -> Result<Vec<Edge>> {
    let tx = db.create_transaction()?;

    // Build range based on sort key bounds
    let prefix = (db_id, ns, "edge", "f", &node.entity_type, node.id.to_bytes(), label);

    let range_start = match sort_key_lower {
        Some(v) => prefix.extend(encode_for_range_index(&v)),
        None => prefix.pack(),
    };

    let range_end = match sort_key_upper {
        Some(v) => prefix.extend(encode_for_range_index(&v)).next(),
        None => prefix.range_end(),
    };

    let results = tx.get_range(&range_start, &range_end, RangeOption { limit, ..default() }).await?;

    results.iter().map(|(key, value)| decode_edge(key, value, EdgeDirection::Outgoing)).collect()
}
```

**Example:** Find all WORKS_AT edges from Person_42 where `started >= 2020-01-01`:

```
Range scan:
  Start: (db, ns, edge, f, Person, p_42, WORKS_AT, <encoded_2020-01-01>)
  End:   (db, ns, edge, f, Person, p_42, WORKS_AT, \xFF)
```

### 7.5 Cross-Namespace Edges

Edges can span namespaces within the same database. The target coordinates include the target database and namespace:

```
Forward Edge Key:
  (db, ns, edge, f, <src_type>, <src_id>, <label>, <tgt_db>, <tgt_ns>, <tgt_type>, <tgt_id>)
```

**Constraints:**
- Both source and target nodes must exist at edge creation time
- Cross-namespace edges are subject to the same ownership rules as same-namespace edges
- Deleting a node still cascades to delete all edges (including cross-namespace)
- CDC entries for cross-namespace edges include full target coordinates

**Note:** Cross-database edges are not supported in v1. All edges must be within the same logical database.

---

## 8. Query System

This section defines the query compilation pipeline, index selection strategy, and execution model.

### 8.1 CEL Expression Pipeline

Queries use CEL (Common Expression Language) for predicates. The compilation pipeline transforms CEL text into executable query plans.

```
         CEL String (e.g., "age >= 30 && name.startsWith('A')")
             │
    ┌────────▼────────┐
    │  1. Parse        │  CEL grammar → AST
    │                  │  Syntax errors fail here
    └────────┬────────┘
             │
    ┌────────▼────────┐
    │  2. Type-Check   │  Validate against entity schema
    │                  │  Build CEL type environment from schema
    │                  │  Type errors fail here (field not in schema)
    └────────┬────────┘
             │
    ┌────────▼──────────────────┐
    │  3. Normalize              │  Flatten nested logic:
    │                            │  !(age < 30) → age >= 30
    │                            │  age > 29 → age >= 30 (integer)
    └────────┬──────────────────┘
             │
    ┌────────▼──────────────────┐
    │  4. Extract Predicates     │  Split AST into conjuncts:
    │                            │  "age >= 30 && name == 'A'"
    │                            │  → [age >= 30] AND [name == 'A']
    └────────┬──────────────────┘
             │
    ┌────────▼──────────────────┐
    │  5. Match Indexes          │  For each predicate, check index:
    │                            │  [age >= 30] → age has range index ✓
    │                            │  [name == 'A'] → name has unique index ✓
    │                            │  [bio.contains('X')] → bio not indexed → RESIDUAL
    └────────┬──────────────────┘
             │
    ┌────────▼──────────────────┐
    │  6. Select Plan            │  Choose execution strategy:
    │                            │  - Most selective index drives scan
    │                            │  - Other predicates become residual
    │                            │  - Emit QueryPlan + residual CEL
    └────────┬──────────────────┘
             │
    ┌────────▼──────────────────┐
    │  7. Execute                │  Run plan against FDB
    │                            │  Apply residual filter to candidates
    │                            │  Stream results
    └─────────────────────────── ┘
```

### 8.2 Index Matching Strategy

#### Single-Predicate Selection (Phase 1)

In Phase 1, the query planner selects a single index to drive the query. Multi-predicate queries use the best index plus residual filtering.

```rust
fn select_index(
    schema: &EntitySchema,
    predicates: &[Predicate],
) -> (Option<IndexPlan>, Vec<Predicate>) {
    let mut best_plan: Option<IndexPlan> = None;
    let mut best_selectivity = 1.0;

    for pred in predicates {
        if let Some(prop_def) = schema.properties.get(&pred.field) {
            if let Some(index_type) = &prop_def.index {
                let selectivity = estimate_selectivity(index_type, &pred.operator);

                if selectivity < best_selectivity {
                    best_selectivity = selectivity;
                    best_plan = Some(IndexPlan {
                        field: pred.field.clone(),
                        index_type: index_type.clone(),
                        operator: pred.operator.clone(),
                        value: pred.value.clone(),
                    });
                }
            }
        }
    }

    // Remaining predicates become residual
    let residual: Vec<Predicate> = predicates.iter()
        .filter(|p| best_plan.as_ref().map_or(true, |bp| bp.field != p.field))
        .cloned()
        .collect();

    (best_plan, residual)
}
```

#### Heuristic Selectivity (unique=0.01, equality=0.10, range=0.50)

Without statistics, use heuristic estimates:

| Index Type | Operator | Estimated Selectivity |
|------------|----------|----------------------|
| Unique | `==` | 0.01 (1 row expected) |
| Unique | `!=` | 0.99 (all but 1 row) |
| Equality | `==` | 0.10 (10% of rows) |
| Equality | `!=` | 0.90 (90% of rows) |
| Range | `>=`, `<=`, `>`, `<` | 0.50 (50% of rows) |
| Range | `==` | 0.10 (10% of rows) |
| None | any | 1.00 (full scan) |

```rust
fn estimate_selectivity(index_type: &IndexType, operator: &Operator) -> f64 {
    match (index_type, operator) {
        (IndexType::Unique, Operator::Eq) => 0.01,
        (IndexType::Unique, Operator::Ne) => 0.99,
        (IndexType::Equality, Operator::Eq) => 0.10,
        (IndexType::Equality, Operator::Ne) => 0.90,
        (IndexType::Range, Operator::Eq) => 0.10,
        (IndexType::Range, Operator::Ge | Operator::Le | Operator::Gt | Operator::Lt) => 0.50,
        _ => 1.0,
    }
}
```

### 8.3 Query Plan Types

```rust
pub enum QueryPlan {
    /// Single key lookup via unique index
    PointLookup {
        entity_type: String,
        index_field: String,
        value: Value,
    },

    /// Range scan via range or equality index
    RangeScan {
        entity_type: String,
        index_field: String,
        lower_bound: Option<Bound>,
        upper_bound: Option<Bound>,
    },

    /// Full scan of all nodes of type (no usable index)
    FullScan {
        entity_type: String,
    },
}

pub struct Bound {
    pub value: Value,
    pub inclusive: bool,
}

pub struct ExecutionPlan {
    /// Primary plan drives the data retrieval
    pub primary: QueryPlan,

    /// Residual filter applied to candidates
    pub residual: Option<CompiledCelExpr>,

    /// Fields to project (empty = all fields)
    pub projection: Vec<String>,

    /// Maximum results to return
    pub limit: Option<u32>,

    /// Pagination cursor
    pub cursor: Option<Vec<u8>>,
}
```

**Plan Selection:**

```rust
fn create_execution_plan(
    entity_type: &str,
    predicates: &[Predicate],
    projection: Vec<String>,
    limit: Option<u32>,
    cursor: Option<Vec<u8>>,
) -> ExecutionPlan {
    let schema = get_schema(entity_type)?;
    let (index_plan, residual_predicates) = select_index(&schema, predicates);

    let primary = match index_plan {
        Some(plan) => {
            match (&plan.index_type, &plan.operator) {
                (IndexType::Unique, Operator::Eq) => {
                    QueryPlan::PointLookup {
                        entity_type: entity_type.to_string(),
                        index_field: plan.field,
                        value: plan.value,
                    }
                }
                (_, Operator::Eq) => {
                    QueryPlan::RangeScan {
                        entity_type: entity_type.to_string(),
                        index_field: plan.field,
                        lower_bound: Some(Bound { value: plan.value.clone(), inclusive: true }),
                        upper_bound: Some(Bound { value: plan.value, inclusive: true }),
                    }
                }
                (_, Operator::Ge) => {
                    QueryPlan::RangeScan {
                        entity_type: entity_type.to_string(),
                        index_field: plan.field,
                        lower_bound: Some(Bound { value: plan.value, inclusive: true }),
                        upper_bound: None,
                    }
                }
                // ... other operators
            }
        }
        None => {
            QueryPlan::FullScan {
                entity_type: entity_type.to_string(),
            }
        }
    };

    let residual = if residual_predicates.is_empty() {
        None
    } else {
        Some(compile_residual_filter(&residual_predicates))
    };

    ExecutionPlan { primary, residual, projection, limit, cursor }
}
```

### 8.4 Snapshot Reads

All queries use snapshot reads for consistency within the query execution.

#### Acquire Read Version at Query Start

```rust
async fn execute_query(
    db: &FdbDatabase,
    plan: &ExecutionPlan,
) -> Result<impl Stream<Item = Node>> {
    // Acquire snapshot read version at query start
    let read_version = db.get_read_version().await?;

    // All subsequent FDB reads use this version
    let tx = db.create_transaction()?;
    tx.set_read_version(read_version);

    // Execute plan with pinned version
    execute_plan_with_snapshot(&tx, plan).await
}
```

#### Pin All Reads to Same Version

Every read operation within a query uses the same FDB read version:

```rust
async fn execute_plan_with_snapshot(
    tx: &FdbTransaction,  // Has read_version already set
    plan: &ExecutionPlan,
) -> Result<impl Stream<Item = Node>> {
    match &plan.primary {
        QueryPlan::PointLookup { entity_type, index_field, value } => {
            // Index lookup at snapshot version
            let index_key = build_unique_index_key(entity_type, index_field, value);
            let node_id = tx.get(&index_key).await?;

            if let Some(id) = node_id {
                // Data fetch at same snapshot version
                let data_key = build_data_key(entity_type, &id);
                let data = tx.get(&data_key).await?;
                // ... stream single node
            }
        }
        QueryPlan::RangeScan { entity_type, index_field, lower_bound, upper_bound } => {
            // Range scan at snapshot version
            let range = build_index_range(entity_type, index_field, lower_bound, upper_bound);
            let results = tx.get_range(&range).await?;

            // For each candidate, fetch data at same snapshot version
            for (key, _) in results {
                let node_id = extract_node_id(&key);
                let data_key = build_data_key(entity_type, &node_id);
                let data = tx.get(&data_key).await?;  // Same snapshot
                // ... apply residual filter and stream
            }
        }
        // ... FullScan similar
    }
}
```

#### 5-Second Transaction Window

FDB snapshot reads are only valid for 5 seconds. Queries that exceed this window must paginate:

```rust
async fn execute_long_query(
    db: &FdbDatabase,
    plan: &ExecutionPlan,
) -> Result<impl Stream<Item = Node>> {
    let start_time = Instant::now();
    let mut cursor = plan.cursor.clone();
    let mut results_count = 0;

    loop {
        // Check if approaching 5-second window
        if start_time.elapsed() > Duration::from_secs(4) {
            // Return current results with cursor for continuation
            return Ok(StreamResult {
                nodes: collected_nodes,
                next_cursor: Some(cursor),
                truncated: true,
            });
        }

        // Acquire fresh snapshot for this batch
        let read_version = db.get_read_version().await?;
        let tx = db.create_transaction()?;
        tx.set_read_version(read_version);

        // Execute batch
        let batch = execute_batch(&tx, plan, &cursor, BATCH_SIZE).await?;

        // Collect results
        for node in batch.nodes {
            collected_nodes.push(node);
            results_count += 1;

            if plan.limit.map_or(false, |l| results_count >= l) {
                return Ok(StreamResult {
                    nodes: collected_nodes,
                    next_cursor: None,
                    truncated: false,
                });
            }
        }

        cursor = batch.next_cursor;
        if cursor.is_none() {
            break;  // No more results
        }
    }

    Ok(StreamResult {
        nodes: collected_nodes,
        next_cursor: None,
        truncated: false,
    })
}
```

**Note:** Results within each batch are consistent (same snapshot). Across batch boundaries, the snapshot may advance, potentially causing slight inconsistencies. This is documented behavior.

### 8.5 Pagination

#### Cursor Format (Opaque Bytes)

Cursors encode position for resumption. The format is opaque to clients:

```rust
#[derive(Serialize, Deserialize)]
struct Cursor {
    /// Plan type indicator
    plan_type: CursorPlanType,

    /// Last seen index key (for range scans)
    last_key: Vec<u8>,

    /// Last seen node_id (for full scans)
    last_node_id: Option<NodeId>,

    /// Scan direction
    reverse: bool,

    /// Original query hash (for validation)
    query_hash: u64,
}

impl Cursor {
    fn encode(&self) -> Vec<u8> {
        // Binary encode, then base64 for wire transport
        let cbor = encode_cbor(self);
        cbor
    }

    fn decode(bytes: &[u8]) -> Result<Self> {
        decode_cbor(bytes)
    }
}
```

#### Limit Enforcement

```rust
async fn execute_with_limit(
    tx: &FdbTransaction,
    plan: &ExecutionPlan,
) -> Result<(Vec<Node>, Option<Cursor>)> {
    let limit = plan.limit.unwrap_or(1000);  // Default limit
    let mut results = Vec::with_capacity(limit as usize);
    let mut last_key: Option<Vec<u8>> = None;

    let range = build_range_for_plan(plan);
    let iterator = tx.get_range(&range, RangeOption {
        limit: limit + 1,  // Fetch one extra to detect more results
        ..default()
    }).await?;

    for (key, value) in iterator.take(limit as usize) {
        let node = decode_and_filter(&key, &value, plan)?;
        if let Some(n) = node {
            results.push(n);
            last_key = Some(key);
        }
    }

    // Check if there are more results
    let next_cursor = if iterator.len() > limit as usize {
        Some(Cursor {
            plan_type: plan.primary.cursor_type(),
            last_key: last_key.unwrap(),
            last_node_id: None,
            reverse: false,
            query_hash: plan.query_hash(),
        })
    } else {
        None
    };

    Ok((results, next_cursor))
}
```

---

## 9. gRPC API

This section defines the complete gRPC service interfaces and protobuf message definitions.

### 9.1 Service Overview

| Service | Purpose | RPCs |
|---------|---------|------|
| `SchemaService` | Schema registration and retrieval | `RegisterSchema`, `GetSchema`, `ListSchemas` |
| `NodeService` | Node CRUD operations | `CreateNode`, `GetNode`, `UpdateNode`, `DeleteNode` |
| `EdgeService` | Edge CRUD and listing | `CreateEdge`, `DeleteEdge`, `ListEdges` |
| `QueryService` | Node queries and traversal | `FindNodes`, `Traverse`, `Explain` |
| `AdminService` | Index management, jobs, lifecycle | `DropIndex`, `StripProperty`, `GetJobStatus`, `DropEntityType`, `DropNamespace` |

### 9.2 RequestContext

Every RPC includes a `RequestContext` for routing and authorization:

```protobuf
message RequestContext {
  // Database identifier (logical isolation unit)
  string database = 1;

  // Namespace within database (entity grouping)
  string namespace = 2;

  // Site identifier for locality-aware routing (optional)
  string site_id = 3;

  // Request ID for tracing (optional, server-generated if absent)
  string request_id = 4;
}
```

### 9.3 SchemaService

```protobuf
service SchemaService {
  // Register or update an entity schema
  rpc RegisterSchema(RegisterSchemaRequest) returns (RegisterSchemaResponse);

  // Get schema for an entity type
  rpc GetSchema(GetSchemaRequest) returns (GetSchemaResponse);

  // List all schemas in namespace
  rpc ListSchemas(ListSchemasRequest) returns (ListSchemasResponse);
}
```

### 9.4 NodeService

```protobuf
service NodeService {
  // Create a new node
  rpc CreateNode(CreateNodeRequest) returns (CreateNodeResponse);

  // Get node by ID (point lookup)
  rpc GetNode(GetNodeRequest) returns (GetNodeResponse);

  // Update node properties
  rpc UpdateNode(UpdateNodeRequest) returns (UpdateNodeResponse);

  // Delete a node
  rpc DeleteNode(DeleteNodeRequest) returns (DeleteNodeResponse);
}
```

### 9.5 EdgeService

```protobuf
service EdgeService {
  // Create an edge between nodes
  rpc CreateEdge(CreateEdgeRequest) returns (CreateEdgeResponse);

  // Delete an edge
  rpc DeleteEdge(DeleteEdgeRequest) returns (DeleteEdgeResponse);

  // List edges from a node (streaming)
  rpc ListEdges(ListEdgesRequest) returns (stream EdgeResult);
}
```

### 9.6 QueryService

```protobuf
service QueryService {
  // Find nodes matching criteria (streaming)
  rpc FindNodes(FindNodesRequest) returns (stream NodeResult);

  // Multi-hop graph traversal (streaming)
  rpc Traverse(TraverseRequest) returns (stream TraverseResult);

  // Explain query plan without execution
  rpc Explain(ExplainRequest) returns (ExplainResponse);
}
```

### 9.7 AdminService

```protobuf
service AdminService {
  // Drop index for a property (synchronous)
  rpc DropIndex(DropIndexRequest) returns (DropIndexResponse);

  // Strip property from all nodes (async job)
  rpc StripProperty(StripPropertyRequest) returns (StripPropertyResponse);

  // Get background job status
  rpc GetJobStatus(GetJobStatusRequest) returns (GetJobStatusResponse);

  // Drop all data for an entity type
  rpc DropEntityType(DropEntityTypeRequest) returns (DropEntityTypeResponse);

  // Drop entire namespace
  rpc DropNamespace(DropNamespaceRequest) returns (DropNamespaceResponse);
}
```

### 9.8 Complete Proto Definitions

```protobuf
syntax = "proto3";
package pelago.v1;

import "google/protobuf/struct.proto";
import "google/protobuf/timestamp.proto";

// ═══════════════════════════════════════════════════════════════════════════
// COMMON TYPES
// ═══════════════════════════════════════════════════════════════════════════

message RequestContext {
  string database = 1;
  string namespace = 2;
  string site_id = 3;
  string request_id = 4;
}

message NodeRef {
  string entity_type = 1;
  string node_id = 2;
  string database = 3;   // For cross-namespace edges
  string namespace = 4;  // For cross-namespace edges
}

message EdgeRef {
  NodeRef source = 1;
  NodeRef target = 2;
  string label = 3;
  string edge_id = 4;
}

message Value {
  oneof kind {
    string string_value = 1;
    int64 int_value = 2;
    double float_value = 3;
    bool bool_value = 4;
    int64 timestamp_value = 5;  // Unix microseconds
    bytes bytes_value = 6;
    bool null_value = 7;        // If true, value is null
  }
}

enum ReadConsistency {
  READ_CONSISTENCY_UNSPECIFIED = 0;
  READ_CONSISTENCY_STRONG = 1;    // Direct FDB read
  READ_CONSISTENCY_SESSION = 2;   // Verified cache
  READ_CONSISTENCY_EVENTUAL = 3;  // Cache only
}

enum EdgeDirection {
  EDGE_DIRECTION_UNSPECIFIED = 0;
  EDGE_DIRECTION_OUTGOING = 1;
  EDGE_DIRECTION_INCOMING = 2;
  EDGE_DIRECTION_BOTH = 3;
}

// ═══════════════════════════════════════════════════════════════════════════
// SCHEMA SERVICE
// ═══════════════════════════════════════════════════════════════════════════

service SchemaService {
  rpc RegisterSchema(RegisterSchemaRequest) returns (RegisterSchemaResponse);
  rpc GetSchema(GetSchemaRequest) returns (GetSchemaResponse);
  rpc ListSchemas(ListSchemasRequest) returns (ListSchemasResponse);
}

message RegisterSchemaRequest {
  RequestContext context = 1;
  EntitySchema schema = 2;
}

message RegisterSchemaResponse {
  uint32 version = 1;
  bool created = 2;  // true if new type, false if update
}

message GetSchemaRequest {
  RequestContext context = 1;
  string entity_type = 2;
  uint32 version = 3;  // 0 = latest
}

message GetSchemaResponse {
  EntitySchema schema = 1;
}

message ListSchemasRequest {
  RequestContext context = 1;
}

message ListSchemasResponse {
  repeated EntitySchema schemas = 1;
}

message EntitySchema {
  string name = 1;
  uint32 version = 2;
  map<string, PropertyDef> properties = 3;
  map<string, EdgeDef> edges = 4;
  SchemaMeta meta = 5;
  int64 created_at = 6;
  string created_by = 7;
}

message PropertyDef {
  PropertyType type = 1;
  bool required = 2;
  IndexType index = 3;
  Value default_value = 4;
}

enum PropertyType {
  PROPERTY_TYPE_UNSPECIFIED = 0;
  PROPERTY_TYPE_STRING = 1;
  PROPERTY_TYPE_INT = 2;
  PROPERTY_TYPE_FLOAT = 3;
  PROPERTY_TYPE_BOOL = 4;
  PROPERTY_TYPE_TIMESTAMP = 5;
  PROPERTY_TYPE_BYTES = 6;
}

enum IndexType {
  INDEX_TYPE_NONE = 0;
  INDEX_TYPE_UNIQUE = 1;
  INDEX_TYPE_EQUALITY = 2;
  INDEX_TYPE_RANGE = 3;
}

message EdgeDef {
  EdgeTarget target = 1;
  EdgeDirectionDef direction = 2;
  map<string, PropertyDef> properties = 3;
  string sort_key = 4;
  OwnershipMode ownership = 5;
}

message EdgeTarget {
  oneof kind {
    string specific_type = 1;  // e.g., "Company"
    bool polymorphic = 2;      // true = any type (*)
  }
}

enum EdgeDirectionDef {
  EDGE_DIRECTION_DEF_UNSPECIFIED = 0;
  EDGE_DIRECTION_DEF_OUTGOING = 1;
  EDGE_DIRECTION_DEF_BIDIRECTIONAL = 2;
}

enum OwnershipMode {
  OWNERSHIP_MODE_UNSPECIFIED = 0;
  OWNERSHIP_MODE_SOURCE_SITE = 1;
  OWNERSHIP_MODE_INDEPENDENT = 2;
}

message SchemaMeta {
  bool allow_undeclared_edges = 1;
  ExtrasPolicy extras_policy = 2;
}

enum ExtrasPolicy {
  EXTRAS_POLICY_UNSPECIFIED = 0;
  EXTRAS_POLICY_REJECT = 1;
  EXTRAS_POLICY_ALLOW = 2;
  EXTRAS_POLICY_WARN = 3;
}

// ═══════════════════════════════════════════════════════════════════════════
// NODE SERVICE
// ═══════════════════════════════════════════════════════════════════════════

service NodeService {
  rpc CreateNode(CreateNodeRequest) returns (CreateNodeResponse);
  rpc GetNode(GetNodeRequest) returns (GetNodeResponse);
  rpc UpdateNode(UpdateNodeRequest) returns (UpdateNodeResponse);
  rpc DeleteNode(DeleteNodeRequest) returns (DeleteNodeResponse);
}

message CreateNodeRequest {
  RequestContext context = 1;
  string entity_type = 2;
  map<string, Value> properties = 3;
}

message CreateNodeResponse {
  Node node = 1;
}

message GetNodeRequest {
  RequestContext context = 1;
  string entity_type = 2;
  string node_id = 3;
  ReadConsistency consistency = 4;
  repeated string fields = 5;  // Empty = all fields
}

message GetNodeResponse {
  Node node = 1;
}

message UpdateNodeRequest {
  RequestContext context = 1;
  string entity_type = 2;
  string node_id = 3;
  map<string, Value> properties = 4;  // Merged with existing
}

message UpdateNodeResponse {
  Node node = 1;
}

message DeleteNodeRequest {
  RequestContext context = 1;
  string entity_type = 2;
  string node_id = 3;
}

message DeleteNodeResponse {
  bool deleted = 1;
}

message Node {
  string id = 1;
  string entity_type = 2;
  map<string, Value> properties = 3;
  string locality = 4;
  int64 created_at = 5;
  int64 updated_at = 6;
}

// ═══════════════════════════════════════════════════════════════════════════
// EDGE SERVICE
// ═══════════════════════════════════════════════════════════════════════════

service EdgeService {
  rpc CreateEdge(CreateEdgeRequest) returns (CreateEdgeResponse);
  rpc DeleteEdge(DeleteEdgeRequest) returns (DeleteEdgeResponse);
  rpc ListEdges(ListEdgesRequest) returns (stream EdgeResult);
}

message CreateEdgeRequest {
  RequestContext context = 1;
  NodeRef source = 2;
  NodeRef target = 3;
  string label = 4;
  map<string, Value> properties = 5;
}

message CreateEdgeResponse {
  Edge edge = 1;
}

message DeleteEdgeRequest {
  RequestContext context = 1;
  NodeRef source = 2;
  NodeRef target = 3;
  string label = 4;
}

message DeleteEdgeResponse {
  bool deleted = 1;
}

message ListEdgesRequest {
  RequestContext context = 1;
  string entity_type = 2;
  string node_id = 3;
  string label = 4;           // Empty = all labels
  EdgeDirection direction = 5;
  ReadConsistency consistency = 6;
  uint32 limit = 7;
  bytes cursor = 8;
}

message EdgeResult {
  Edge edge = 1;
  bytes next_cursor = 2;      // Set on last message if more results
}

message Edge {
  string edge_id = 1;
  NodeRef source = 2;
  NodeRef target = 3;
  string label = 4;
  map<string, Value> properties = 5;
  int64 created_at = 6;
}

// ═══════════════════════════════════════════════════════════════════════════
// QUERY SERVICE
// ═══════════════════════════════════════════════════════════════════════════

service QueryService {
  rpc FindNodes(FindNodesRequest) returns (stream NodeResult);
  rpc Traverse(TraverseRequest) returns (stream TraverseResult);
  rpc Explain(ExplainRequest) returns (ExplainResponse);
}

message FindNodesRequest {
  RequestContext context = 1;
  string entity_type = 2;
  string cel_expression = 3;  // e.g., "age >= 30 && active"
  ReadConsistency consistency = 4;
  repeated string fields = 5;
  uint32 limit = 6;
  bytes cursor = 7;
}

message NodeResult {
  Node node = 1;
  bytes next_cursor = 2;
}

message TraverseRequest {
  RequestContext context = 1;
  NodeRef start = 2;
  repeated TraversalStep steps = 3;
  uint32 max_depth = 4;       // Default: 4, max: 10
  uint32 timeout_ms = 5;      // Default: 5000, max: 30000
  uint32 max_results = 6;     // Default: 10000
  ReadConsistency consistency = 7;
  bool cascade = 8;           // Require all nested paths exist
}

message TraversalStep {
  string edge_type = 1;
  EdgeDirection direction = 2;
  string edge_filter = 3;     // CEL on edge properties
  string node_filter = 4;     // CEL on target node properties
  repeated string fields = 5; // Node field projection
  uint32 per_node_limit = 6;  // Fan-out control per source node
  repeated string edge_fields = 7; // Edge property projection (@facets)
  SortSpec sort = 8;
}

message SortSpec {
  string field = 1;
  bool descending = 2;
  bool on_edge = 3;           // true = sort by edge property
}

message TraverseResult {
  int32 depth = 1;
  repeated NodeRef path = 2;
  Node node = 3;
  Edge edge = 4;
  bytes next_cursor = 5;
}

message ExplainRequest {
  RequestContext context = 1;
  string entity_type = 2;
  string cel_expression = 3;
}

message ExplainResponse {
  QueryPlan plan = 1;
  repeated string explanation = 2;  // Human-readable steps
  double estimated_cost = 3;
  double estimated_rows = 4;
}

message QueryPlan {
  string strategy = 1;              // "point_lookup", "range_scan", "full_scan"
  repeated IndexOperation indexes = 2;
  string residual_filter = 3;
}

message IndexOperation {
  string field = 1;
  string operator = 2;              // "==", ">=", ">", "<", "<="
  string value = 3;
  double estimated_rows = 4;
}

// ═══════════════════════════════════════════════════════════════════════════
// ADMIN SERVICE
// ═══════════════════════════════════════════════════════════════════════════

service AdminService {
  rpc DropIndex(DropIndexRequest) returns (DropIndexResponse);
  rpc StripProperty(StripPropertyRequest) returns (StripPropertyResponse);
  rpc GetJobStatus(GetJobStatusRequest) returns (GetJobStatusResponse);
  rpc DropEntityType(DropEntityTypeRequest) returns (DropEntityTypeResponse);
  rpc DropNamespace(DropNamespaceRequest) returns (DropNamespaceResponse);
}

message DropIndexRequest {
  RequestContext context = 1;
  string entity_type = 2;
  string property_name = 3;
}

message DropIndexResponse {
  int64 entries_deleted = 1;
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
  Job job = 1;
}

message Job {
  string job_id = 1;
  string job_type = 2;
  JobStatus status = 3;
  int64 created_at = 4;
  int64 updated_at = 5;
  double progress = 6;        // 0.0 to 1.0
  string error = 7;
}

enum JobStatus {
  JOB_STATUS_UNSPECIFIED = 0;
  JOB_STATUS_PENDING = 1;
  JOB_STATUS_RUNNING = 2;
  JOB_STATUS_COMPLETED = 3;
  JOB_STATUS_FAILED = 4;
}

message DropEntityTypeRequest {
  RequestContext context = 1;
  string entity_type = 2;
}

message DropEntityTypeResponse {
  string cleanup_job_id = 1;  // Job for orphaned edge cleanup
}

message DropNamespaceRequest {
  RequestContext context = 1;
}

message DropNamespaceResponse {
  bool dropped = 1;
}

// ═══════════════════════════════════════════════════════════════════════════
// ERROR DETAILS
// ═══════════════════════════════════════════════════════════════════════════

message ErrorDetail {
  string error_code = 1;      // e.g., "ERR_UNREGISTERED_TYPE"
  string message = 2;
  string category = 3;        // "VALIDATION", "REFERENTIAL", "OWNERSHIP"
  map<string, string> metadata = 4;
  repeated FieldError field_errors = 5;
}

message FieldError {
  string field_name = 1;
  string error_code = 2;
  string message = 3;
}
```

---

## 17. Pelago Query Language (PQL)

This section defines PQL (Pelago Query Language), a declarative graph query language inspired by Dgraph's DQL. PQL provides a human-friendly syntax for complex graph traversals that compiles to the existing gRPC API (`FindNodesRequest`, `TraverseRequest`).

### 17.1 Overview

PQL is designed with the following principles:

1. **DQL-inspired syntax**: Nested block traversal, variables, and directives borrowed from Dgraph
2. **CEL predicates**: All filtering uses CEL expressions, preserving type-checking and cost-based planning
3. **Compilation target**: PQL compiles to existing proto messages — it is syntax sugar over the gRPC API
4. **REPL-first**: Designed for interactive exploration in the `pelago` CLI

**Example Query:**

```
query friends_of_friends {
  start(func: uid(Person:p_42)) {
    name
    age
    -[KNOWS]-> @filter(age >= 30) {
      name
      -[WORKS_AT]-> {
        name
        industry
      }
    }
  }
}
```

This query starts from Person `p_42`, traverses KNOWS edges to find friends aged 30+, then traverses to the companies they work at.

### 17.2 Query Structure

A PQL query consists of one or more **named blocks**. Each block has:

1. **Root function**: Selects starting nodes
2. **Directives**: Modify traversal behavior
3. **Selection set**: Field projections and nested edge traversals

```
query [query_name] {
  block_name(func: root_function) [@directive ...] {
    field_projection
    edge_traversal [@directive ...] {
      nested_projection
      deeper_edge [@directive ...] { ... }
    }
  }
}
```

**Components:**

| Component | Description |
|-----------|-------------|
| `query_name` | Optional name for the query (for caching/debugging) |
| `block_name` | Required name for each block (keys results in response) |
| `root_function` | Function that selects starting nodes |
| `@directive` | Optional modifiers like `@filter`, `@limit` |
| `field_projection` | Property names to include in results |
| `edge_traversal` | Nested edge block with direction and type |

### 17.3 Grammar (EBNF)

```ebnf
query       = "query" [IDENT] [context_directive] "{" block+ "}" ;

context_directive = "@context" "(" "namespace" ":" STRING ")" ;

block       = IDENT "(" root_func ")" directives "{" selection+ "}" ;

root_func   = "func:" function_call ;

function_call = IDENT "(" arg_list ")" ;

arg_list    = arg ("," arg)* ;
arg         = STRING | NUMBER | IDENT | qualified_ref ;

qualified_ref = [IDENT ":"] IDENT ":" STRING ;  (* e.g., ns:Person:p_42 or Person:p_42 *)

selection   = field_select | edge_block | var_capture | aggregate ;

field_select = IDENT ;                 (* property name to project *)

edge_block  = edge_spec [target_spec] directives "{" selection+ "}" ;

edge_spec   = "-[" [IDENT ":"] IDENT "]" direction ;
              (* optional namespace prefix on edge type: -[ns:EDGE]-> *)

target_spec = [IDENT ":"] IDENT ;
              (* optional namespace prefix on target type: -> ns:Type *)

direction   = "->" | "<-" | "<->" ;

directives  = ("@" directive)* ;
directive   = IDENT "(" arg_list? ")" ;

var_capture = IDENT "as" (block | edge_block | "val(" IDENT ")") ;

aggregate   = agg_func "(" IDENT ")" ;
agg_func    = "count" | "sum" | "avg" | "min" | "max" ;
```

**Lexical Rules:**

```ebnf
IDENT       = [a-zA-Z_][a-zA-Z0-9_]* ;
STRING      = '"' [^"]* '"' ;
NUMBER      = [0-9]+ ("." [0-9]+)? ;
WHITESPACE  = [ \t\n\r]+ ;          (* ignored *)
COMMENT     = "#" [^\n]* ;          (* ignored *)
```

### 17.4 Root Functions

Root functions select the starting node set for a block. Only ONE root function is allowed per block.

| Function | Meaning | Compiles To |
|----------|---------|-------------|
| `uid(Type:id)` | Single node by type and ID | `NodeRef { entity_type, node_id }` |
| `uid(ns:Type:id)` | Node with explicit namespace | `NodeRef { namespace, entity_type, node_id }` |
| `uid(var_name)` | Nodes captured by a variable | Variable reference |
| `type(EntityType)` | All nodes of type | `FindNodesRequest { entity_type }` |
| `eq(field, value)` | Equality on indexed field | CEL `field == value` |
| `ge(field, value)` | Greater-than-or-equal | CEL `field >= value` |
| `le(field, value)` | Less-than-or-equal | CEL `field <= value` |
| `gt(field, value)` | Greater-than | CEL `field > value` |
| `lt(field, value)` | Less-than | CEL `field < value` |
| `between(field, lo, hi)` | Range (inclusive) | CEL `field >= lo && field <= hi` |
| `has(field)` | Field exists (not null) | CEL `field != null` |
| `allofterms(field, "a b")` | All terms present | CEL `field.contains("a") && field.contains("b")` |

**Examples:**

```
# Single node lookup
start(func: uid(Person:p_42)) { ... }

# All nodes of type
all_people(func: type(Person)) { ... }

# Index-backed equality
by_email(func: eq(email, "alice@example.com")) { ... }

# Range query
active_users(func: ge(last_login, timestamp("2024-01-01"))) { ... }

# Variable reference (from prior block)
mutual(func: uid(friends_of_alice)) { ... }
```

### 17.5 Edge Traversal Syntax

Edge traversal blocks specify the edge type, direction, and nested selections for target nodes.

**Direction Operators:**

| Syntax | Direction | Meaning |
|--------|-----------|---------|
| `-[EDGE]->` | Outgoing | Traverse edges FROM source TO target |
| `-[EDGE]<-` | Incoming | Traverse edges TO source FROM target |
| `-[EDGE]<->` | Bidirectional | Traverse in both directions |

**Edge Block Structure:**

```
-[EDGE_TYPE]-> @edge(edge_filter) @filter(node_filter) {
  field1
  field2
  -[NESTED_EDGE]-> { ... }
}
```

**Separating Edge vs Node Filters:**

| Directive | Evaluated Against | Example |
|-----------|-------------------|---------|
| `@edge(...)` | Edge properties | `@edge(role == "Lead")` |
| `@filter(...)` | Target node properties | `@filter(age >= 30)` |

**Example:**

```
query {
  start(func: uid(Person:p_42)) {
    name
    -[WORKS_AT]-> @edge(role == "Engineer") @filter(industry == "tech") {
      name
      founded
    }
  }
}
```

**Compilation:**

```protobuf
TraversalStep {
  edge_type: "WORKS_AT",
  direction: EDGE_DIRECTION_OUTGOING,
  edge_filter: "role == 'Engineer'",
  node_filter: "industry == 'tech'",
  fields: ["name", "founded"]
}
```

### 17.6 Cross-Namespace Traversal

Edges may be defined in a different namespace than their source or target nodes. PQL supports namespace-qualified edge types and explicit target type constraints.

#### 17.6.1 Namespace-Qualified Edge Types

When an edge type is defined in a different namespace, qualify it with the namespace prefix:

```
-[namespace:EDGE_TYPE]->
```

**Example:** Person (in `default` namespace) traverses to Job (in `job_history` namespace) via HAD edge (defined in `job_history`):

```
query {
  start(func: uid(default:Person:p_42)) {
    name
    -[job_history:HAD]-> {
      title
      company
      started
    }
  }
}
```

#### 17.6.2 Explicit Target Type Constraints

For polymorphic edges or explicit type safety, specify the target type after the direction:

```
-[namespace:EDGE_TYPE]-> namespace:TargetType { ... }
```

**Example:** Polymorphic edge constrained to specific types:

```
query {
  start(func: uid(default:Asset:a_123)) {
    name
    -[TAGGED_WITH]-> default:Project {   # Only traverse to Projects
      name
      status
    }
    -[TAGGED_WITH]-> default:Sequence {  # Separately traverse to Sequences
      name
      frame_range
    }
  }
}
```

#### 17.6.3 Namespace Resolution Rules

| Syntax | Edge Namespace | Target Namespace |
|--------|----------------|------------------|
| `-[EDGE]->` | Query context default | Inferred from edge schema |
| `-[ns:EDGE]->` | `ns` | Inferred from edge schema |
| `-[EDGE]-> Type` | Query context default | Query context default |
| `-[ns:EDGE]-> Type` | `ns` | Query context default |
| `-[ns:EDGE]-> ns2:Type` | `ns` | `ns2` |

**Setting Default Namespace:**

```
query @context(namespace: "core") {
  start(func: type(Person)) {
    name
    -[KNOWS]-> Person {           # Implicitly core:Person
      name
    }
    -[external:LINKED]-> external:Entity {  # Cross-namespace
      id
    }
  }
}
```

#### 17.6.4 Compilation to Proto

```
-[job_history:HAD]-> job_history:Job @edge(role == "Lead") { title }
```

Compiles to:

```protobuf
TraversalStep {
  edge_namespace: "job_history",
  edge_type: "HAD",
  direction: EDGE_DIRECTION_OUTGOING,
  target_namespace: "job_history",
  target_type: "Job",
  edge_filter: "role == 'Lead'",
  fields: ["title"]
}
```

### 17.7 Directives

Directives modify traversal and result behavior. They can be applied at the block level or edge traversal level.

#### 17.7.1 @filter — Node Predicate

Filters nodes using a CEL expression against node properties.

```
@filter(age >= 30 && department == "rendering")
```

**Placement:**
- On root block: filters starting nodes
- On edge block: filters target nodes

**Compiles to:** `FindNodesRequest.cel_expression` or `TraversalStep.node_filter`

#### 17.7.2 @edge — Edge Predicate

Filters edges using a CEL expression against edge properties.

```
@edge(role == "Lead" && started > timestamp("2023-01-01"))
```

**Compiles to:** `TraversalStep.edge_filter`

**Vertex-Centric Optimization:** When `@edge` references the edge's `sort_key` property, the planner emits a range scan hint for optimized FDB lookup.

#### 17.7.3 @cascade

Removes nodes that don't have all requested sub-traversals. Borrowed from DQL.

```
query {
  people(func: type(Person)) @cascade {
    name
    -[KNOWS]-> {
      name
      -[WORKS_AT]-> {
        name
      }
    }
  }
}
```

**Without @cascade:** Returns all Person nodes, with empty nested blocks for those without matching edges.

**With @cascade:** Only returns Person nodes that have at least one KNOWS edge to someone who has at least one WORKS_AT edge.

**Compiles to:** `TraverseRequest.cascade = true`

#### 17.7.4 @limit and @offset — Pagination

```
# Global limit on block
people(func: type(Person)) @limit(first: 20, offset: 40) { ... }

# Per-hop fan-out limit
-[KNOWS]-> @limit(first: 5) { ... }
```

**Global limit:** Limits total results from the block.

**Per-hop limit:** Limits edges traversed per source node — critical for controlling traversal explosion.

**Compiles to:**
- Global: `FindNodesRequest.limit`
- Per-hop: `TraversalStep.per_node_limit`

#### 17.7.5 @sort — Ordering

```
-[WORKS_AT]-> @sort(started: desc) { ... }

# Sort by edge property
-[WORKS_AT]-> @sort(edge.started: desc) { ... }
```

**Compiles to:** `TraversalStep.sort { field, descending, on_edge }`

**Optimization:** If sort field matches the edge's `sort_key`, the sort is free (FDB returns in sort_key order).

#### 17.7.6 @facets — Edge Property Projection

Projects edge properties alongside target node data.

```
-[WORKS_AT]-> @facets(role, started) {
  name
}
```

**Result:**
```json
{
  "name": "Acme Corp",
  "WORKS_AT|role": "Engineer",
  "WORKS_AT|started": "2020-01-15T00:00:00Z"
}
```

**Compiles to:** `TraversalStep.edge_fields`

#### 17.7.7 @recurse — Recursive Traversal

For variable-depth traversal of the same edge type.

```
query {
  reports(func: uid(Person:p_ceo)) @recurse(depth: 5) {
    name
    -[MANAGES]->
  }
}
```

Equivalent to 5 hops of `-[MANAGES]->` with same projection at each level.

**Properties:**
- `depth` is mandatory (no unbounded recursion)
- Server tracks visited UIDs in a HashSet for cycle detection
- Results are streamed depth-first

**Compiles to:**
```protobuf
TraverseRequest {
  recurse: { max_depth: 5, detect_cycles: true }
}
```

### 17.8 Variables

Variables capture intermediate results for use in subsequent blocks.

#### 17.8.1 UID Variables (Node Set Capture)

Capture the node set produced by a block:

```
query {
  friends as start(func: uid(Person:p_42)) {
    -[KNOWS]-> {
      name
    }
  }

  # Use captured nodes as root of second block
  mutual(func: uid(friends)) @filter(age >= 30) {
    name
    age
  }
}
```

`friends as` captures all nodes reached by the KNOWS traversal. The second block starts from those captured nodes.

**Scoping Rules:**

| Rule | Behavior |
|------|----------|
| Query-scoped | Variables visible to all blocks within the same `query { }` |
| Top-to-bottom | A block can only reference variables from earlier blocks |
| Capture at depth | Variable captures nodes at the innermost traversal depth |
| Unique names | Variable names must be unique within a query |

#### 17.8.2 Value Variables (Scalar Capture)

Capture scalar aggregations for use in filters or projections:

```
query {
  var(func: type(Person)) {
    friend_count as count(-[KNOWS]->)
  }

  popular(func: ge(val(friend_count), 10)) {
    name
    fc: val(friend_count)
  }
}
```

`friend_count as count(...)` computes the KNOWS edge count per person. The second block filters to persons with 10+ friends.

#### 17.8.3 Set Operations

Variable sets support union, intersection, and difference:

```
query {
  alice_friends as a(func: uid(Person:p_alice)) {
    -[KNOWS]-> { uid }
  }

  bob_friends as b(func: uid(Person:p_bob)) {
    -[KNOWS]-> { uid }
  }

  # Intersection: mutual friends
  mutual(func: uid(alice_friends, bob_friends, intersect)) {
    name
  }

  # Difference: alice's friends who aren't bob's friends
  alice_only(func: uid(alice_friends, bob_friends, difference)) {
    name
  }
}
```

| Operation | Syntax | Meaning |
|-----------|--------|---------|
| Union | `uid(a, b)` | Nodes in a OR b |
| Intersection | `uid(a, b, intersect)` | Nodes in a AND b |
| Difference | `uid(a, b, difference)` | Nodes in a but NOT b |

### 17.9 Aggregations

#### 17.9.1 Inline Aggregations

Aggregation functions compute over edge sets:

```
query {
  companies(func: type(Company)) {
    name
    employee_count: count(-[WORKS_AT]<-)
    avg_tenure: avg(-[WORKS_AT]<- @facets(started) { started })
  }
}
```

| Function | Input | Output |
|----------|-------|--------|
| `count(edge_block)` | Number of matching edges | int |
| `sum(edge_block.field)` | Sum of numeric field | float |
| `avg(edge_block.field)` | Average | float |
| `min(edge_block.field)` | Minimum | same type |
| `max(edge_block.field)` | Maximum | same type |

#### 17.9.2 @groupby

Groups results by a field and applies aggregations:

```
query {
  by_department(func: type(Person)) @groupby(department) {
    count(uid)
    avg(age)
  }
}
```

**Compiles to:** Full scan of Person type, group by department, compute aggregates per group.

**Performance Note:** `@groupby` is expensive. It should use an indexed field for grouping. The planner warns via EXPLAIN if the groupby field isn't indexed.

### 17.10 Mutations (Upsert Blocks)

PQL supports atomic query-then-mutate operations within a single FDB transaction.

#### 17.10.1 Upsert Block Syntax

```
upsert {
  query {
    existing as find(func: eq(email, "alice@ilm.com")) {
      uid
    }
  }

  mutation @if(len(existing) == 0) {
    create Person {
      email: "alice@ilm.com",
      name: "Alice",
      age: 35
    }
  }

  mutation @if(len(existing) > 0) {
    update uid(existing) {
      name: "Alice Updated"
    }
  }
}
```

**Execution:**
1. Execute `query` block within FDB transaction snapshot
2. Evaluate `@if` conditions against query results
3. Execute matching mutation(s) within same transaction
4. Commit atomically

#### 17.10.2 Conditional Edge Creation

```
upsert {
  query {
    alice as a(func: eq(email, "alice@ilm.com")) { uid }
    ilm as i(func: eq(name, "ILM")) { uid }
    edge_exists as e(func: uid(alice)) {
      -[WORKS_AT]-> @filter(uid == uid(ilm)) { uid }
    }
  }

  mutation @if(len(alice) > 0 && len(ilm) > 0 && len(edge_exists) == 0) {
    create edge uid(alice) -[WORKS_AT]-> uid(ilm) {
      role: "Engineer",
      started: timestamp("2024-01-15")
    }
  }
}
```

#### 17.10.3 Proto Extension

```protobuf
message UpsertRequest {
  RequestContext context = 1;
  PqlQuery query = 2;
  repeated ConditionalMutation mutations = 3;
}

message ConditionalMutation {
  string condition = 1;             // e.g., "len(existing) == 0"
  oneof operation {
    CreateNodeRequest create = 2;
    UpdateNodeRequest update = 3;
    DeleteNodeRequest delete = 4;
    CreateEdgeRequest create_edge = 5;
    DeleteEdgeRequest delete_edge = 6;
  }
}
```

### 17.11 REPL Integration

#### 17.11.1 Full Query Syntax

```
pelago> query { people(func: eq(name, "Andrew")) { name, age } }
+--------+----------+-----+
| block  | name     | age |
+--------+----------+-----+
| people | Andrew   | 38  |
+--------+----------+-----+
1 result (2.1ms)
```

#### 17.11.2 Short-Form Syntax

For simple queries, PQL supports a shortened single-block form:

```
pelago> Person @filter(age >= 30) { name, age }
```

Equivalent to:
```
query { result(func: type(Person)) @filter(age >= 30) { name, age } }
```

**Short-form rules:**
- Starts with entity type name (capitalized)
- Implicit `result` block name
- Implicit `type()` root function

#### 17.11.3 Explain Mode

```
pelago> :explain Person @filter(age >= 30 && department == "rendering") { name }

Query Plan:
  Block: result
  Root: type(Person)
  Strategy: index_scan(department, equality) + residual(age >= 30)
  Estimated rows: ~200
  Cost: 201

  Alternative considered:
    index_scan(age, range) + residual(department == "rendering")
    Estimated rows: ~8000
    Cost: 8001
    Rejected: higher cost
```

#### 17.11.4 Parameterized Queries

```
pelago> :param $min_age = 30
pelago> Person @filter(age >= $min_age) { name, age }
```

Parameters enable server-side query plan caching.

### 17.12 Wire Transport

#### 17.12.1 PQL over gRPC

```protobuf
service QueryService {
  // ... existing RPCs ...
  rpc ExecutePql(PqlRequest) returns (stream PqlResult);
  rpc ExplainPql(PqlRequest) returns (PqlExplainResponse);
}

message PqlRequest {
  RequestContext context = 1;
  string pql = 2;                   // Raw PQL text
  ReadConsistency consistency = 3;
  uint32 timeout_ms = 4;
  uint32 max_results = 5;
  map<string, Value> variables = 6;  // Parameterized queries
}

message PqlResult {
  string block_name = 1;
  int32 depth = 2;
  repeated NodeRef path = 3;
  NodeRef node = 4;
  map<string, Value> properties = 5;
  map<string, Value> edge_facets = 6;  // Edge properties if @facets
  map<string, Value> aggregates = 7;   // Computed aggregates
}

message PqlExplainResponse {
  repeated BlockPlan blocks = 1;
}

message BlockPlan {
  string block_name = 1;
  string strategy = 2;
  repeated string steps = 3;
  double estimated_cost = 4;
  double estimated_rows = 5;
  repeated BlockPlan alternatives = 6;
}
```

### 17.13 Compilation Pipeline

PQL text is compiled through a multi-stage pipeline:

```
PQL text
    |
    v
+------------------+
| 1. Parse         |  pest parser -> PQL AST
|    (syntax)      |  Syntax errors fail here
+--------+---------+
         |
         v
+------------------+
| 2. Resolve       |  Resolve entity types, edge types, field names
|    (schema)      |  Validate edge declarations exist
|                  |  Resolve variable references (check DAG)
+--------+---------+
         |
         v
+------------------+
| 3. CEL Compile   |  Extract all @filter/@edge directives
|    (type-check)  |  Compile each CEL expression against schema
|                  |  Type-check field references
+--------+---------+
         |
         v
+------------------+
| 4. Plan          |  Build execution DAG (block dependencies)
|    (optimize)    |  Merge sequential hops into TraversalSteps
|                  |  Select root function -> index strategy
|                  |  Apply @limit hints for fan-out control
+--------+---------+
         |
         v
+------------------+
| 5. Emit Proto    |  Generate FindNodesRequest / TraverseRequest
|    (codegen)     |  Wire up variable references as block deps
|                  |  Set consistency, timeout, max_results
+------------------+
```

#### 17.13.1 AST Types

```rust
pub struct PqlQuery {
    pub name: Option<String>,
    pub default_namespace: Option<String>,
    pub blocks: Vec<QueryBlock>,
}

pub struct QueryBlock {
    pub name: String,
    pub root: RootFunction,
    pub directives: Vec<Directive>,
    pub selections: Vec<Selection>,
    pub capture_as: Option<String>,
}

pub enum RootFunction {
    Uid(QualifiedRef),
    UidVar(String),
    UidSet(Vec<String>, SetOp),
    Eq(String, Value),
    Ge(String, Value),
    Le(String, Value),
    Between(String, Value, Value),
    Has(String),
    Type(QualifiedType),
    AllOfTerms(String, String),
}

pub enum Selection {
    Field(String),
    Edge(EdgeTraversal),
    Aggregate(AggregateExpr),
    ValueVar(String, AggregateExpr),
}

pub struct EdgeTraversal {
    pub edge_namespace: Option<String>,
    pub edge_type: String,
    pub direction: EdgeDirection,
    pub target_type: Option<TargetSpec>,
    pub directives: Vec<Directive>,
    pub selections: Vec<Selection>,
    pub capture_as: Option<String>,
}

pub enum Directive {
    Filter(String),
    Edge(String),
    Cascade,
    Limit { first: u32, offset: Option<u32> },
    Sort { field: String, desc: bool, on_edge: bool },
    Facets(Vec<String>),
    Recurse { depth: u32 },
    GroupBy(Vec<String>),
}
```

#### 17.13.2 Crate Location

```
pelago-query/src/
  +-- cel.rs            <- existing: CEL parse, type-check
  +-- planner.rs        <- existing: predicate extraction, index selection
  +-- executor.rs       <- existing: FindNodes execution
  +-- plan.rs           <- existing: QueryPlan enum
  +-- pql/
      +-- mod.rs
      +-- grammar.pest  <- PEG grammar for PQL
      +-- parser.rs     <- pest parser -> PqlAst
      +-- resolver.rs   <- schema resolution, variable DAG
      +-- compiler.rs   <- PqlAst -> proto messages
      +-- explain.rs    <- EXPLAIN output for PQL queries
```

### 17.14 Implementation Phases

#### Phase 2a: PQL Parser + Single-Block Queries

- [ ] PEG grammar for PQL (pest crate)
- [ ] Parser -> PqlAst
- [ ] Schema resolver (validate types, fields, edges)
- [ ] CEL extraction from directives
- [ ] Compiler: single-block PQL -> FindNodesRequest or TraverseRequest
- [ ] REPL integration (replace ad-hoc `find` / `traverse` commands)
- [ ] `@filter`, `@edge`, `@limit`, `@sort` directives
- [ ] Short-form syntax in REPL
- [ ] ExplainPql endpoint

#### Phase 2b: Variables + Multi-Block

- [ ] Variable capture (UID variables)
- [ ] Block dependency DAG + execution ordering
- [ ] `uid(var)` root function
- [ ] Value variables with aggregation
- [ ] Set operations (union, intersect, difference)
- [ ] PqlQuery proto + ExecutePql RPC
- [ ] Parameterized queries + plan caching

#### Phase 2c: Advanced Directives

- [ ] `@cascade` (subtree completeness filter)
- [ ] `@facets` (edge property projection)
- [ ] `@recurse` (variable-depth traversal with cycle detection)
- [ ] `@groupby` with aggregation functions
- [ ] `per_node_limit` on TraversalStep

#### Phase 3: Upserts

- [ ] Upsert block parsing
- [ ] Conditional mutation evaluation
- [ ] UpsertRequest proto
- [ ] Atomic query-then-mutate in single FDB transaction

### 17.15 Comparison: PQL vs DQL

| Feature | DQL | PQL | Rationale |
|---------|-----|-----|-----------|
| Predicate language | Custom functions | CEL | Type-checked, cost-based planner, portable |
| Edge vs node filter | Both in `@filter` | Separate `@edge` + `@filter` | Explicit; maps to proto fields cleanly |
| Root function | `func:` with index functions | Same | Direct inspiration |
| Variables | `varName as` | Same | Direct inspiration |
| @cascade | Yes | Yes | Direct inspiration |
| @facets | Yes (inline) | Yes (with directive) | Direct inspiration |
| @recurse | Yes (depth-limited) | Yes (with cycle detection) | Added cycle detection |
| @groupby | Yes | Yes | Direct inspiration |
| Upsert blocks | Yes | Yes | Direct inspiration, single FDB txn |
| Set operations | No (manual) | Yes | Useful for mutual-friends pattern |
| Sort | `orderasc`/`orderdesc` | `@sort(field: asc/desc)` | Cleaner directive syntax |
| Per-hop limit | No built-in | `@limit` on edge blocks | Critical for traversal explosion |

---

## 10. CLI Reference

The PelagoDB CLI (`pelago`) provides command-line access to all database operations, schema management, and a PQL-native REPL for interactive queries.

### 10.1 Command Hierarchy

```
pelago
├── connect <address>                    # Connect to server
├── config                               # Manage local configuration
│   ├── get <key>                        # Get config value
│   ├── set <key> <value>                # Set config value
│   ├── list                             # List all config
│   └── path                             # Show config file path
│
├── schema                               # Schema management
│   ├── register <file.json>             # Register/update schema
│   ├── get <type>                       # Show schema for entity type
│   ├── list                             # List all schemas
│   └── diff <type> <v1> <v2>            # Compare schema versions
│
├── node                                 # Node operations
│   ├── create <type> [--props JSON]     # Create node
│   ├── get <type> <id>                  # Get node by ID
│   ├── update <type> <id> [--props]     # Update node
│   ├── delete <type> <id>               # Delete node
│   └── list <type> [--limit N]          # List nodes of type
│
├── edge                                 # Edge operations
│   ├── create <src> <label> <tgt>       # Create edge
│   ├── delete <edge-id>                 # Delete edge
│   └── list <type> <id> [--dir]         # List edges
│
├── index                                # Index management
│   ├── create <type> <field> <kind>     # Create index
│   ├── drop <type> <field>              # Drop index
│   └── list <type>                      # List indexes
│
├── admin                                # Administrative operations
│   ├── job status <job-id>              # Check job progress
│   ├── job list                         # List active jobs
│   ├── drop-type <type>                 # Drop entity type
│   └── drop-namespace <ns>              # Drop namespace
│
├── repl                                 # PQL interactive REPL
└── version                              # Show version info
```

### 10.2 Connection Management

#### Connect Command

```bash
# Connect to a specific server
pelago connect localhost:27615

# Connect with explicit database and namespace
pelago connect localhost:27615 --database production --namespace core

# Store connection as default
pelago connect localhost:27615 --save
```

#### Configuration File

Configuration is stored in `~/.pelago/config.toml`:

```toml
[default]
server = "localhost:27615"
database = "production"
namespace = "default"
site_id = "SF"
format = "table"

[servers.production]
address = "pelago.prod.example.com:27615"
database = "production"
tls = true
ca_cert = "/path/to/ca.pem"

[servers.staging]
address = "pelago.staging.example.com:27615"
database = "staging"
```

#### Global Flags

| Flag | Short | Environment Variable | Description |
|------|-------|---------------------|-------------|
| `--server` | `-s` | `PELAGO_SERVER` | Server address |
| `--database` | `-d` | `PELAGO_DATABASE` | Database name |
| `--namespace` | `-n` | `PELAGO_NAMESPACE` | Namespace name |
| `--site-id` | | `PELAGO_SITE_ID` | Site identifier |
| `--format` | `-f` | | Output format: `table`, `json`, `csv`, `auto` |
| `--pretty` | | | Pretty-print JSON output |
| `--quiet` | `-q` | | Suppress non-essential output |

### 10.3 Schema Commands

#### Register Schema

```bash
# Register from JSON file
pelago schema register person.json

# Register with inline JSON
pelago schema register --inline '{
  "name": "Person",
  "version": 1,
  "properties": {
    "name": {"type": "string", "required": true, "index": "unique"},
    "age": {"type": "int", "index": "range"}
  }
}'

# Update existing schema (must increment version)
pelago schema register person-v2.json
```

#### Get Schema

```bash
# Get current schema
pelago schema get Person

# Get specific version
pelago schema get Person --version 1

# Output as JSON
pelago schema get Person --format json
```

#### List Schemas

```bash
# List all schemas in namespace
pelago schema list

# Filter by prefix
pelago schema list --prefix User
```

#### Diff Schemas

```bash
# Compare two versions
pelago schema diff Person 1 2

# Output:
# Person v1 -> v2:
#   + Added property: phone (string, optional)
#   ~ Changed property: email (added index: unique)
#   - Removed property: legacy_id
```

### 10.4 Node Commands

#### Create Node

```bash
# Create with JSON properties
pelago node create Person --props '{"name": "Alice", "age": 30}'

# Create with key=value pairs
pelago node create Person name="Alice" age:int=30

# Create and output ID only
pelago node create Person --props '{"name": "Alice"}' --id-only
# Output: 1_42
```

#### Get Node

```bash
# Get by type and ID
pelago node get Person 1_42

# Get specific fields only
pelago node get Person 1_42 --fields name,age

# Output as JSON
pelago node get Person 1_42 --format json
```

#### Update Node

```bash
# Update properties
pelago node update Person 1_42 --props '{"age": 31}'

# Set property to null (removes it)
pelago node update Person 1_42 --props '{"nickname": null}'
```

#### Delete Node

```bash
# Delete node (cascades to edges)
pelago node delete Person 1_42

# Force delete without confirmation
pelago node delete Person 1_42 --force
```

#### List Nodes

```bash
# List nodes of type
pelago node list Person --limit 10

# With filter
pelago node list Person --filter "age >= 30"

# Stream all (for scripting)
pelago node list Person --format jsonl
```

### 10.5 Edge Commands

#### Create Edge

```bash
# Create edge
pelago edge create Person:1_42 WORKS_AT Company:1_7 --props '{"role": "Engineer"}'

# Create bidirectional edge
pelago edge create Person:1_42 KNOWS Person:1_99 --props '{"since": 1705363200}'
```

#### Delete Edge

```bash
# Delete by source, label, and target
pelago edge delete Person:1_42 WORKS_AT Company:1_7
```

#### List Edges

```bash
# List outgoing edges
pelago edge list Person 1_42 --dir out

# List specific edge type
pelago edge list Person 1_42 --label WORKS_AT

# List incoming edges
pelago edge list Person 1_42 --dir in
```

### 10.6 REPL

The REPL provides an interactive PQL (Pelago Query Language) environment.

#### Starting the REPL

```bash
$ pelago repl
Connected to pelago://localhost:27615
Database: production | Namespace: default | Site: SF

pelago>
```

#### REPL Commands

| Command | Description |
|---------|-------------|
| `<PQL query>` | Execute a PQL query |
| `:explain <PQL>` | Show query plan without execution |
| `:use <namespace>` | Switch namespace |
| `:db <database>` | Switch database |
| `:format <json\|table\|csv>` | Set output format |
| `:param $name = value` | Set query parameter |
| `:params` | Show all parameters |
| `:clear` | Clear parameters |
| `:history` | Show command history |
| `:help` | Show help |
| `:quit` | Exit REPL |

#### PQL Query Examples

```
pelago> query { people(func: type(Person)) @filter(age >= 30) { name, age } }
┌────────┬──────────┬─────┐
│ block  │ name     │ age │
├────────┼──────────┼─────┤
│ people │ Alice    │ 30  │
│ people │ Bob      │ 35  │
└────────┴──────────┴─────┘
2 results (3.2ms)

pelago> Person @filter(age >= 30) { name, age }
# Short-form: equivalent to above

pelago> :explain Person @filter(age >= 30) { name }
Query Plan:
  Block: result
  Root: type(Person)
  Strategy: range_scan(age)
  Filter: age >= 30
  Estimated rows: ~200
```

#### Multi-Block Queries

```
pelago> query {
  friends as start(func: uid(Person:1_42)) {
    -[KNOWS]-> { uid }
  }

  mutual(func: uid(friends)) @filter(age >= 25) {
    name
    age
  }
}
```

#### Parameter Substitution

```
pelago> :param $min_age = 30
Set $min_age = 30

pelago> Person @filter(age >= $min_age) { name }
```

---

## 11. CDC System

The Change Data Capture (CDC) system provides an ordered, durable log of all mutations for replication, event sourcing, and cache synchronization.

### 11.1 CDC Entry Schema

Each mutation appends a CDC entry to the namespace's `_cdc` subspace.

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CdcEntry {
    /// Site that performed the mutation
    pub site: String,

    /// Timestamp of the mutation (unix microseconds)
    pub timestamp: i64,

    /// Optional batch identifier for grouped operations
    pub batch_id: Option<String>,

    /// List of operations in this entry
    pub operations: Vec<CdcOperation>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum CdcOperation {
    NodeCreate {
        entity_type: String,
        node_id: NodeId,
        properties: HashMap<String, Value>,
        home_site: String,
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
        owner_site: String,
        pair_id: Option<String>,
    },

    EdgeDelete {
        source: NodeRef,
        target: NodeRef,
        edge_type: String,
        pair_id: Option<String>,
    },

    SchemaRegister {
        entity_type: String,
        version: u32,
        schema: EntitySchema,
    },
}
```

**FDB Key Layout:**

```
Key:   (db, ns, _cdc, <versionstamp>)
Value: CBOR-encoded CdcEntry
```

### 11.2 Versionstamp Ordering

CDC entries use FDB versionstamps for zero-contention, globally-ordered append.

**Versionstamp Properties:**

- **10 bytes:** 8-byte transaction version + 2-byte batch ordering
- **Monotonically increasing:** Within a cluster, versionstamps are strictly ordered
- **Commit-time assigned:** Value is set atomically at commit
- **No coordination:** Writers do not need to read before writing

**Usage Pattern:**

```rust
async fn append_cdc_entry(
    tx: &FdbTransaction,
    entry: &CdcEntry,
) -> Result<()> {
    // Key contains versionstamp placeholder
    let key_template = (db, ns, "_cdc");

    // FDB fills in versionstamp at commit time
    let key = tx.pack_with_versionstamp(&key_template)?;
    let value = encode_cbor(entry);

    tx.set_versionstamped_key(&key, &value);
    Ok(())
}
```

### 11.3 Consumer Pattern

CDC consumers maintain a high-water mark (HWM) indicating the last processed entry.

#### High-Water Mark Tracking

```rust
struct CdcConsumer {
    consumer_id: String,
    checkpoint_key: Vec<u8>,  // (db, ns, _meta, cdc_checkpoints, consumer_id)
}

impl CdcConsumer {
    /// Load checkpoint from FDB
    async fn load_hwm(&self, db: &FdbDatabase) -> Result<Versionstamp> {
        let tx = db.create_transaction()?;
        tx.get(&self.checkpoint_key)
            .await?
            .map(|bytes| Versionstamp::from_bytes(&bytes))
            .unwrap_or_else(|| Versionstamp::zero())
    }

    /// Save checkpoint to FDB
    async fn save_hwm(&self, db: &FdbDatabase, hwm: Versionstamp) -> Result<()> {
        let tx = db.create_transaction()?;
        tx.set(&self.checkpoint_key, &hwm.to_bytes());
        tx.commit().await?;
        Ok(())
    }
}
```

#### Consumer Loop

```rust
async fn consume_cdc_entries(
    db: &FdbDatabase,
    consumer: &CdcConsumer,
    handler: impl Fn(&CdcEntry) -> Result<()>,
) -> Result<()> {
    let mut hwm = consumer.load_hwm(db).await?;
    let mut checkpoint_interval = Instant::now();

    loop {
        let tx = db.create_transaction()?;

        // Range scan from HWM to current
        let range_start = (db_id, ns, "_cdc", hwm.next());
        let range_end = (db_id, ns, "_cdc").range_end();

        let entries = tx.get_range(&range_start, &range_end, RangeOption {
            limit: 1000,
            ..default()
        }).await?;

        if entries.is_empty() {
            // No new entries, wait and retry
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        }

        // Process entries
        for (key, value) in &entries {
            let entry: CdcEntry = decode_cbor(value)?;
            handler(&entry)?;
            hwm = extract_versionstamp(key);
        }

        // Periodic checkpoint
        if checkpoint_interval.elapsed() > Duration::from_secs(5) {
            consumer.save_hwm(db, hwm).await?;
            checkpoint_interval = Instant::now();
        }
    }
}
```

#### Checkpoint Semantics

**At-least-once delivery:** After crash recovery, consumers may replay entries between the last checkpoint and the crash point. Handlers must be idempotent.

**Checkpoint Storage:**

```
Key:   (db, ns, _meta, cdc_checkpoints, <consumer_id>)
Value: 10-byte versionstamp
```

### 11.4 Retention and Compaction

CDC entries are retained for a configurable duration (default: 7 days).

#### Retention Policy

```rust
struct CdcRetentionConfig {
    /// Minimum retention period
    min_retention: Duration,  // Default: 1 day

    /// Maximum retention period
    max_retention: Duration,  // Default: 7 days

    /// Checkpoint-based retention: keep entries since oldest consumer checkpoint
    checkpoint_aware: bool,   // Default: true
}
```

#### Truncation Job

A background job periodically deletes expired CDC entries:

```rust
async fn truncate_expired_cdc(
    db: &FdbDatabase,
    config: &CdcRetentionConfig,
) -> Result<u64> {
    let now = SystemTime::now();
    let max_cutoff = now - config.max_retention;

    // If checkpoint-aware, respect consumer positions
    let effective_cutoff = if config.checkpoint_aware {
        let checkpoints = fetch_all_cdc_checkpoints(db).await?;
        let oldest_checkpoint = checkpoints.values().min()
            .map(|vs| versionstamp_to_time(vs))
            .unwrap_or(max_cutoff);

        // Do not delete entries newer than the oldest checkpoint
        oldest_checkpoint.min(max_cutoff)
    } else {
        max_cutoff
    };

    let cutoff_versionstamp = time_to_versionstamp(effective_cutoff);

    // Delete in batches to avoid large transactions
    let mut deleted_count = 0;
    loop {
        let tx = db.create_transaction()?;

        let range = (db_id, ns, "_cdc").range_to(cutoff_versionstamp);
        let batch = tx.get_range(&range, RangeOption { limit: 10000, ..default() }).await?;

        if batch.is_empty() {
            break;
        }

        for (key, _) in &batch {
            tx.clear(key);
            deleted_count += 1;
        }

        tx.commit().await?;
    }

    Ok(deleted_count)
}
```

#### CDC Compaction (Future Enhancement)

For high-throughput systems, CDC entries can be compacted by consolidating multiple operations on the same entity:

```
Before compaction:
  vs_1001: create node_123 { age: 30 }
  vs_1002: update node_123 { age: 31 }
  vs_1003: update node_123 { age: 32 }

After compaction:
  vs_1001: create node_123 { age: 32 }
```

**Note:** Compaction loses historical state and is not recommended for audit trails.

---

## 12. Multi-Site Replication

This section defines the multi-site replication architecture, ownership model, and conflict resolution strategy.

### 12.1 Overview

PelagoDB supports multi-site deployments where each site maintains a full replica of the data. Replication is:

- **CDC-driven:** Changes flow between sites via CDC event streams
- **Ownership-based:** Each node/edge has an owning site that is authoritative for mutations
- **Eventually consistent:** Sites converge asynchronously with sub-second latency in normal operation
- **HA-prioritized:** Availability during network partitions takes precedence over strict consistency

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           Multi-Site Topology                                │
└─────────────────────────────────────────────────────────────────────────────┘

   Site: SF (site_id=1)                      Site: SYD (site_id=2)
  ┌─────────────────────┐                   ┌─────────────────────┐
  │    PelagoDB Node    │                   │    PelagoDB Node    │
  │  ┌───────────────┐  │                   │  ┌───────────────┐  │
  │  │  API Layer    │  │                   │  │  API Layer    │  │
  │  └───────┬───────┘  │                   │  └───────┬───────┘  │
  │          │          │                   │          │          │
  │  ┌───────▼───────┐  │                   │  ┌───────▼───────┐  │
  │  │  FoundationDB │  │                   │  │  FoundationDB │  │
  │  │  (local FDB   │  │                   │  │  (local FDB   │  │
  │  │   cluster)    │  │                   │  │   cluster)    │  │
  │  └───────────────┘  │                   │  └───────────────┘  │
  │                     │                   │                     │
  │  ┌───────────────┐  │   CDC Events      │  ┌───────────────┐  │
  │  │  Replicator   │◄─┼───────────────────┼──│  Replicator   │  │
  │  │  (for SYD)    │  │                   │  │  (for SF)     │  │
  │  └───────┬───────┘  │                   │  └───────┬───────┘  │
  │          │          │   CDC Events      │          │          │
  │          └──────────┼───────────────────┼─►        │          │
  │                     │                   │          ▼          │
  └─────────────────────┘                   └─────────────────────┘

  Each site has a replicator process for each remote site.
  Replicators pull CDC events and project them locally.
```

### 12.2 Ownership Model

Every node and edge has an **owning site** that is authoritative for mutations.

#### Node Ownership

Nodes are owned by the site specified in their `locality` field:

```rust
struct NodeRecord {
    p: HashMap<String, Value>,  // payload
    l: String,                   // locality (owning site): "SF", "SYD", "SING"
    c: i64,                      // created_at
    u: i64,                      // updated_at
}
```

**Ownership Rules:**

| Operation | Locality Matches Current Site | Result |
|-----------|------------------------------|--------|
| Create | Always (new node) | Node created with `locality = current_site` |
| Read | Any | Allowed (read local replica) |
| Update | Yes | Allowed |
| Update | No | **REJECTED** — must transfer ownership first |
| Delete | Yes | Allowed |
| Delete | No | **REJECTED** — must transfer ownership first |

**Write Rejection Response:**

```protobuf
ErrorDetail {
  error_code: "ERR_LOCALITY_MISMATCH",
  category: "OWNERSHIP",
  message: "Cannot mutate node owned by 'SYD' from site 'SF'. Transfer ownership first.",
  metadata: {
    "node_id": "2_42",
    "owner_site": "SYD",
    "current_site": "SF"
  }
}
```

#### Edge Ownership

Edges have two ownership modes, declared in the schema:

```rust
pub enum OwnershipMode {
    /// Edge owned by source node's home site (default)
    SourceSite,
    /// Edge owned by the site that created it
    Independent,
}
```

**SourceSite (default):**
- Edge is owned by the same site that owns the source node
- If source node ownership transfers, edge ownership transfers implicitly
- Simplifies reasoning about data locality

**Independent:**
- Edge is owned by the site that created it
- Useful for cross-site relationships where neither endpoint "owns" the relationship
- Ownership tracked in edge metadata: `owner_site` field

**Bidirectional Edge Ownership:**

For bidirectional edges (e.g., KNOWS between Person A and Person B):
- Two directed edges are created (A→B and B→A)
- Each direction is owned by its source node's home site
- If A is at SF and B is at SYD:
  - Edge A→B is owned by SF
  - Edge B→A is owned by SYD

### 12.3 Ownership Transfer

To mutate a node from a non-owning site, ownership must be explicitly transferred.

#### Transfer Protocol

```
1. Client requests: TransferOwnership(node_id, new_owner_site)
2. Current owner site validates request
3. Current owner site:
   a. Updates node locality: l = new_owner_site
   b. Emits CDC entry: OwnershipTransfer { node_id, from_site, to_site, timestamp }
   c. Commits transaction
4. CDC replicates to all sites
5. New owner site can now accept mutations
```

**Rust Implementation:**

```rust
async fn transfer_ownership(
    db: &FdbDatabase,
    entity_type: &str,
    node_id: &NodeId,
    new_owner: &str,
    current_site: &str,
) -> Result<()> {
    let tx = db.create_transaction()?;

    // Read current node
    let data_key = (db_id, ns, "data", entity_type, node_id.to_bytes());
    let current = tx.get(&data_key).await?
        .ok_or(PelagoError::NodeNotFound)?;
    let mut record: NodeRecord = decode_cbor(&current);

    // Verify current site owns the node
    if record.l != current_site {
        return Err(PelagoError::NotOwner {
            node_id: node_id.clone(),
            owner: record.l.clone(),
            requester: current_site.to_string(),
        });
    }

    // Update locality
    let old_owner = record.l.clone();
    record.l = new_owner.to_string();
    record.u = current_time_micros();

    // Write updated record
    tx.set(&data_key, &encode_cbor(&record));

    // Update locality index
    let loc_key = (db_id, ns, "loc", entity_type, node_id.to_bytes());
    tx.set(&loc_key, new_owner.as_bytes());

    // Emit CDC entry
    append_cdc_entry(&tx, CdcOperation::OwnershipTransfer {
        entity_type: entity_type.to_string(),
        node_id: node_id.clone(),
        from_site: old_owner,
        to_site: new_owner.to_string(),
        timestamp: current_time_micros(),
    })?;

    tx.commit().await?;
    Ok(())
}
```

**CDC Operation:**

```rust
CdcOperation::OwnershipTransfer {
    entity_type: String,
    node_id: NodeId,
    from_site: String,
    to_site: String,
    timestamp: i64,
}
```

#### Transfer Constraints

| Constraint | Behavior |
|------------|----------|
| Only owner can transfer | Non-owner transfer requests are rejected |
| Target site must exist | Transfer to unknown site is rejected |
| Atomic | Transfer completes fully or not at all |
| Idempotent | Repeated transfer to same site is no-op |

### 12.4 Replication Architecture

Each site runs a **replicator process** for every remote site in the cluster.

#### Site Registry

Sites are discovered via the `/_sys/` subspace:

```
Key: (_sys, site_claim, <site_id>)
Value: UTF-8 site name (e.g., "SF", "SYD", "SING")
```

On startup, each site:
1. Reads all `site_claim` entries to discover registered sites
2. Spawns a replicator for each remote site
3. Periodically re-scans for new sites (autodiscovery)

#### Replicator Design

Each replicator is responsible for:
1. Pulling CDC events from a remote site
2. Projecting those events into the local FDB instance
3. Tracking position (high-water mark) per remote site

```rust
struct Replicator {
    local_site: String,
    remote_site: String,
    remote_endpoint: String,  // gRPC endpoint for remote site
    position_key: Vec<u8>,    // FDB key for HWM: (_sys, repl_position, remote_site)
}

impl Replicator {
    async fn run(&self, db: &FdbDatabase) -> Result<()> {
        loop {
            // Load last processed position
            let hwm = self.load_position(db).await?;

            // Request CDC events from remote site
            let events = self.fetch_cdc_events(hwm).await?;

            if events.is_empty() {
                // No new events, sleep and retry
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }

            // Project events into local FDB
            for event in &events {
                self.project_event(db, event).await?;
            }

            // Update position to last processed event
            let new_hwm = events.last().unwrap().versionstamp.clone();
            self.save_position(db, &new_hwm).await?;
        }
    }
}
```

#### Position Tracking

Each replicator maintains a high-water mark (HWM) indicating the last CDC event processed from a remote site:

```
Key: (_sys, repl_position, <remote_site_name>)
Value: 10-byte versionstamp of last processed CDC entry
```

**Position Storage:**

```rust
impl Replicator {
    async fn load_position(&self, db: &FdbDatabase) -> Result<Versionstamp> {
        let tx = db.create_transaction()?;
        let key = ("_sys", "repl_position", &self.remote_site);

        tx.get(&key).await?
            .map(|bytes| Versionstamp::from_bytes(&bytes))
            .ok_or(Versionstamp::zero())  // Start from beginning if no position
    }

    async fn save_position(&self, db: &FdbDatabase, hwm: &Versionstamp) -> Result<()> {
        let tx = db.create_transaction()?;
        let key = ("_sys", "repl_position", &self.remote_site);
        tx.set(&key, &hwm.to_bytes());
        tx.commit().await?;
        Ok(())
    }
}
```

#### CDC Pull API

Remote sites expose a gRPC endpoint for CDC retrieval:

```protobuf
service ReplicationService {
  // Stream CDC events starting after the given position
  rpc PullCdcEvents(PullCdcRequest) returns (stream CdcEvent);
}

message PullCdcRequest {
  // Start position (exclusive) — events after this versionstamp
  bytes after_position = 1;

  // Maximum events to return per batch
  uint32 batch_size = 2;

  // Optional: filter by database/namespace
  string database = 3;
  string namespace = 4;
}

message CdcEvent {
  bytes versionstamp = 1;
  CdcEntry entry = 2;
}
```

### 12.5 Event Projection

When a replicator receives CDC events from a remote site, it **projects** them into the local FDB instance.

#### Projection Rules

| CDC Operation | Projection Action |
|---------------|-------------------|
| `NodeCreate` | Insert node data + indexes (if owner matches remote site) |
| `NodeUpdate` | Update node data + indexes (if owner matches remote site) |
| `NodeDelete` | Delete node data + indexes (if owner matches remote site) |
| `EdgeCreate` | Insert edge entries (if owner matches remote site) |
| `EdgeDelete` | Delete edge entries (if owner matches remote site) |
| `OwnershipTransfer` | Update locality field and index |
| `SchemaRegister` | Register schema locally (schemas replicate globally) |

#### Ownership Filtering

Replicators only apply events for data owned by the remote site:

```rust
async fn project_event(&self, db: &FdbDatabase, event: &CdcEvent) -> Result<()> {
    for op in &event.entry.operations {
        match op {
            CdcOperation::NodeCreate { home_site, .. } |
            CdcOperation::NodeUpdate { .. } |
            CdcOperation::NodeDelete { .. } => {
                // Get the owning site for this node
                let owner = self.get_node_owner(db, op).await?;

                // Only apply if the remote site owns this node
                if owner == self.remote_site {
                    self.apply_node_operation(db, op).await?;
                }
                // Otherwise, ignore — this site will receive the event
                // from the actual owner
            }

            CdcOperation::EdgeCreate { owner_site, .. } |
            CdcOperation::EdgeDelete { .. } => {
                // Check edge ownership
                let owner = self.get_edge_owner(op);

                if owner == self.remote_site {
                    self.apply_edge_operation(db, op).await?;
                }
            }

            CdcOperation::OwnershipTransfer { from_site, .. } => {
                // Apply if the transfer is FROM the remote site
                if from_site == &self.remote_site {
                    self.apply_ownership_transfer(db, op).await?;
                }
            }

            CdcOperation::SchemaRegister { .. } => {
                // Schemas replicate globally
                self.apply_schema_registration(db, op).await?;
            }
        }
    }
    Ok(())
}
```

#### Idempotent Projection

All projection operations must be idempotent to handle:
- Replicator restarts
- Duplicate event delivery
- Out-of-order processing during recovery

```rust
async fn apply_node_operation(&self, db: &FdbDatabase, op: &CdcOperation) -> Result<()> {
    match op {
        CdcOperation::NodeCreate { entity_type, node_id, properties, home_site } => {
            let tx = db.create_transaction()?;
            let key = (db_id, ns, "data", entity_type, node_id.to_bytes());

            // Check if node already exists (idempotent)
            if tx.get(&key).await?.is_some() {
                return Ok(());  // Already exists, skip
            }

            // Create node record
            let record = NodeRecord {
                p: properties.clone(),
                l: home_site.clone(),
                c: current_time_micros(),
                u: current_time_micros(),
            };

            tx.set(&key, &encode_cbor(&record));
            // ... create indexes ...
            tx.commit().await?;
        }
        // ... other operations ...
    }
    Ok(())
}
```

### 12.6 Conflict Resolution

Conflicts can occur in edge cases such as network partitions or ownership transfer races.

#### Conflict Scenarios

| Scenario | Resolution |
|----------|------------|
| Two sites modify same node | **Owner wins** — non-owner's write was already rejected |
| Ownership transfer race | **Owner wins** — only current owner can transfer |
| Stale write after transfer | **New owner wins** — old owner no longer authoritative |
| Split-brain (both think they own) | **LWW fallback** — timestamp-based resolution |

#### Owner-Wins Resolution

Under normal operation, conflicts cannot occur because:
1. Only the owning site can mutate a node
2. Ownership transfer is atomic and requires current owner
3. All sites receive the same CDC stream from each owner

```
Site SF owns Person_42
├── SF can mutate → Success, CDC emitted
├── SYD tries to mutate → Rejected immediately (ERR_LOCALITY_MISMATCH)
└── SING tries to mutate → Rejected immediately (ERR_LOCALITY_MISMATCH)
```

#### LWW Fallback (Split-Brain Recovery)

In rare split-brain scenarios (network partition where ownership state diverges), use Last-Write-Wins based on timestamps:

```rust
async fn apply_with_lww(&self, db: &FdbDatabase, incoming: &CdcOperation) -> Result<()> {
    let tx = db.create_transaction()?;

    let key = (db_id, ns, "data", entity_type, node_id.to_bytes());
    let existing = tx.get(&key).await?;

    if let Some(existing_bytes) = existing {
        let existing_record: NodeRecord = decode_cbor(&existing_bytes);
        let incoming_timestamp = incoming.timestamp();

        // LWW: Only apply if incoming is newer
        if incoming_timestamp <= existing_record.u {
            log::warn!(
                "LWW conflict: Dropping older event (incoming={}, existing={})",
                incoming_timestamp, existing_record.u
            );
            return Ok(());  // Drop older event
        }
    }

    // Apply incoming event
    self.apply_event(db, incoming).await
}
```

**When LWW Applies:**

LWW is only used when ownership validation fails to prevent the conflict. This indicates a bug or severe network partition. The system logs a warning when LWW resolution occurs.

### 12.7 Replication Scope

#### Current: Full Replication

In the initial implementation, all data replicates to all sites:

```
Site SF ──CDC──► Site SYD
Site SF ──CDC──► Site SING
Site SYD ──CDC──► Site SF
Site SYD ──CDC──► Site SING
Site SING ──CDC──► Site SF
Site SING ──CDC──► Site SYD
```

**Advantages:**
- Simple to reason about
- All reads are local
- No cross-site read latency

**Disadvantages:**
- Storage scales with number of sites
- Replication bandwidth grows with data volume

#### Future: Selective Replication

Future versions will support replication control at the database and namespace level:

```
Database Config:
  production:
    replication_sites: [SF, SYD, SING]  # Full replication

  analytics:
    replication_sites: [SF]             # Single-site only

  regional_eu:
    replication_sites: [FRA, LON]       # EU sites only
```

**Namespace-Level Control:**

```json
{
  "namespace": "user_data",
  "replication": {
    "mode": "selective",
    "sites": ["SF", "SYD"],
    "exclude_sites": ["SING"]
  }
}
```

**Read-Through for Non-Local Data:**

When selective replication is enabled, reads for non-local data will use read-through:

```rust
async fn get_node_with_read_through(
    db: &FdbDatabase,
    entity_type: &str,
    node_id: &NodeId,
) -> Result<Node> {
    // Try local read first
    if let Some(node) = get_node_local(db, entity_type, node_id).await? {
        return Ok(node);
    }

    // Node not replicated locally — read from owner site
    let owner_site = extract_site_from_node_id(node_id);
    let remote = get_remote_client(owner_site)?;

    remote.get_node(entity_type, node_id).await
}
```

### 12.8 Consistency Guarantees

#### Within a Site

All operations within a single site have **strict serializability** via FDB:
- Reads see a consistent snapshot
- Writes are atomic
- No anomalies possible

#### Across Sites

Cross-site consistency is **eventual** with the following properties:

| Property | Guarantee |
|----------|-----------|
| **Convergence** | All sites eventually reach the same state |
| **Causal ordering** | Events from the same site are applied in order |
| **No lost writes** | All committed writes eventually replicate |
| **Bounded lag** | Sub-second in normal operation |

**Replication Lag:**

```
Typical: < 100ms (direct network path)
Degraded: < 5s (congestion, retries)
Partitioned: Unbounded (queues until reconnection)
```

**Monitoring Replication Lag:**

```rust
struct ReplicationMetrics {
    /// Versionstamp of last local CDC entry
    local_hwm: Versionstamp,

    /// Per-remote-site: versionstamp of last received event
    remote_hwms: HashMap<String, Versionstamp>,

    /// Per-remote-site: estimated lag in milliseconds
    estimated_lag_ms: HashMap<String, u64>,
}
```

### 12.9 Failure Modes and Recovery

#### Replicator Crash

**Impact:** Replication pauses for that remote site.

**Recovery:**
1. Replicator restarts
2. Loads last position from `(_sys, repl_position, remote_site)`
3. Resumes pulling from that position
4. Catches up on missed events

**Data Impact:** None. All events are durably stored in CDC and replayed on recovery.

#### Network Partition

**Impact:** Sites cannot communicate; replication stops.

**Behavior:**
- Each site continues accepting local writes
- CDC events queue locally
- Replicators retry connection with exponential backoff

**Recovery:**
1. Network restored
2. Replicators reconnect
3. Position-based pull resumes from last HWM
4. Queued events drain to remote sites

**Conflict Risk:** Minimal. Ownership enforcement prevents conflicting writes. LWW handles edge cases.

#### Site Recovery After Extended Outage

If a site is down for an extended period:

1. **CDC Retention:** Ensure CDC retention period exceeds expected outage duration
2. **Replay:** Recovering site replays all CDC events since last position
3. **Catch-up:** May take time for large backlogs

**CDC Retention Configuration:**

```toml
[cdc]
retention_days = 7  # Keep CDC entries for 7 days
```

If outage exceeds retention, full resync from a healthy site is required.

#### Split-Brain Detection

Split-brain occurs when sites disagree about ownership. Detection:

```rust
async fn detect_split_brain(&self, node_id: &NodeId) -> bool {
    let local_owner = get_local_owner(node_id).await;
    let remote_owner = get_remote_owner(node_id).await;

    if local_owner != remote_owner {
        log::error!(
            "Split-brain detected for {}: local={}, remote={}",
            node_id, local_owner, remote_owner
        );
        return true;
    }
    false
}
```

**Resolution:** Manual intervention or automatic LWW fallback based on configuration.

---

## 13. Background Jobs

Background jobs handle long-running operations like index backfill, property stripping, and orphaned edge cleanup.

### 13.1 Job Types

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum JobType {
    /// Backfill index entries for existing nodes
    IndexBackfill {
        entity_type: String,
        property_name: String,
        index_type: IndexType,
    },

    /// Remove a property from all nodes of a type
    StripProperty {
        entity_type: String,
        property_name: String,
    },

    /// Clean up orphaned edges after entity type deletion (Phase 2)
    OrphanedEdgeCleanup {
        deleted_entity_type: String,
    },

    /// Truncate expired CDC entries
    CdcRetention {
        cutoff_versionstamp: Vec<u8>,
    },
}
```

### 13.2 Job Storage

Jobs are stored in the `_jobs` subspace:

```
Key:   (db, ns, _jobs, <job_type>, <job_id>)
Value: CBOR-encoded JobState
```

**JobState Structure:**

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobState {
    /// Unique job identifier
    pub job_id: String,

    /// Job type and parameters
    pub job_type: JobType,

    /// Current status
    pub status: JobStatus,

    /// Resume cursor (opaque bytes)
    pub progress_cursor: Option<Vec<u8>>,

    /// Progress percentage (0.0 to 1.0)
    pub progress_pct: f64,

    /// Total items to process (if known)
    pub total_items: Option<u64>,

    /// Items processed so far
    pub processed_items: u64,

    /// Creation timestamp
    pub created_at: i64,

    /// Last update timestamp
    pub updated_at: i64,

    /// Error message if failed
    pub error: Option<String>,

    /// Database and namespace
    pub database: String,
    pub namespace: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
}
```

### 13.3 Progress Tracking

Jobs use cursor-based resumption for crash recovery.

#### Cursor-Based Resumption

```rust
impl IndexBackfillJob {
    async fn execute(&mut self, db: &FdbDatabase) -> Result<()> {
        // Resume from cursor if present
        let start_key = self.state.progress_cursor.clone()
            .unwrap_or_else(|| self.build_start_key());

        loop {
            let tx = db.create_transaction()?;

            // Scan batch of nodes from cursor
            let range = (db_id, ns, "data", &self.entity_type)
                .range_from(&start_key);
            let batch = tx.get_range(&range, RangeOption {
                limit: BATCH_SIZE,
                ..default()
            }).await?;

            if batch.is_empty() {
                // Job complete
                self.state.status = JobStatus::Completed;
                self.state.progress_pct = 1.0;
                self.save_state(db).await?;
                return Ok(());
            }

            // Process batch: create index entries
            for (key, value) in &batch {
                let node_id = extract_node_id(&key);
                let props: HashMap<String, Value> = decode_cbor(value)?;

                if let Some(prop_value) = props.get(&self.property_name) {
                    self.create_index_entry(&tx, &node_id, prop_value)?;
                }

                self.state.processed_items += 1;
            }

            // Update cursor to last processed key
            self.state.progress_cursor = Some(batch.last().unwrap().0.clone());
            self.state.updated_at = current_time_micros();

            // Commit batch
            tx.commit().await?;

            // Save state periodically
            if self.state.processed_items % 10000 == 0 {
                self.save_state(db).await?;
            }
        }
    }
}
```

### 13.4 Crash Recovery

On server startup, the job worker scans for interrupted jobs:

```rust
async fn recover_jobs(db: &FdbDatabase) -> Result<Vec<JobState>> {
    let tx = db.create_transaction()?;

    // Scan all jobs with Running status
    let range = (db_id, ns, "_jobs").range();
    let all_jobs = tx.get_range(&range, RangeOption::default()).await?;

    let mut to_resume = Vec::new();

    for (_, value) in all_jobs {
        let job: JobState = decode_cbor(&value)?;

        match job.status {
            JobStatus::Running => {
                // Job was interrupted, resume from cursor
                to_resume.push(job);
            }
            JobStatus::Pending => {
                // Job never started, start fresh
                to_resume.push(job);
            }
            _ => {}
        }
    }

    Ok(to_resume)
}

async fn job_worker_loop(db: &FdbDatabase) -> Result<()> {
    // Recover interrupted jobs on startup
    let interrupted = recover_jobs(db).await?;
    for job in interrupted {
        spawn_job(job).await;
    }

    // Poll for new jobs
    loop {
        let pending = find_pending_jobs(db).await?;
        for job in pending {
            spawn_job(job).await;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
```

**Recovery Guarantees:**

- Jobs resume from the last saved cursor
- Already-processed items may be reprocessed (idempotent operations)
- Failed jobs remain in Failed status for operator inspection
- Completed jobs are retained for 24 hours, then deleted

---

## 14. Error Model

PelagoDB uses a structured error model that maps to gRPC status codes with detailed error information.

### 14.1 Error Categories

| Category | Description | Examples |
|----------|-------------|----------|
| **Validation** | Client input is invalid | Missing required field, type mismatch, CEL syntax error |
| **Referential** | Reference constraint violation | Target node not found, source node not found |
| **Ownership** | Multi-site ownership violation | Mutation from non-home site |
| **Consistency** | Data integrity violation | Unique constraint conflict |
| **Timeout** | Operation exceeded time limit | Query timeout, traversal depth exceeded |
| **Resource** | System resource limit | CDC retention expired, storage quota |
| **Internal** | Server-side failure | FDB unavailable, codec failure |

### 14.2 Error Code Registry

#### Validation Errors (ERR_VALIDATION_*)

| Code | Message | Details |
|------|---------|---------|
| `ERR_VALIDATION_UNREGISTERED_TYPE` | Entity type not registered | entity_type |
| `ERR_VALIDATION_MISSING_REQUIRED` | Required property missing | entity_type, field_name |
| `ERR_VALIDATION_TYPE_MISMATCH` | Property type mismatch | field_name, expected, actual |
| `ERR_VALIDATION_EXTRA_PROPERTY` | Unexpected property (extras_policy=reject) | field_name |
| `ERR_VALIDATION_CEL_SYNTAX` | CEL expression syntax error | expression, position, message |
| `ERR_VALIDATION_CEL_TYPE` | CEL expression type error | expression, field, message |
| `ERR_VALIDATION_INVALID_ID` | Malformed node/edge ID | id_value |
| `ERR_VALIDATION_INVALID_VALUE` | Invalid property value | field_name, reason |

#### Referential Errors (ERR_REF_*)

| Code | Message | Details |
|------|---------|---------|
| `ERR_REF_SOURCE_NOT_FOUND` | Source node does not exist | entity_type, node_id |
| `ERR_REF_TARGET_NOT_FOUND` | Target node does not exist | entity_type, node_id |
| `ERR_REF_NODE_NOT_FOUND` | Node does not exist | entity_type, node_id |
| `ERR_REF_EDGE_NOT_FOUND` | Edge does not exist | source, target, label |
| `ERR_REF_TYPE_NOT_REGISTERED` | Referenced type not registered | entity_type |
| `ERR_REF_TARGET_TYPE_MISMATCH` | Edge target type mismatch | edge_type, expected, actual |
| `ERR_REF_UNDECLARED_EDGE` | Undeclared edge type | entity_type, edge_type |

#### Ownership Errors (ERR_OWNERSHIP_*)

| Code | Message | Details |
|------|---------|---------|
| `ERR_OWNERSHIP_WRONG_SITE` | Mutation from non-home site | node_id, home_site, request_site |
| `ERR_OWNERSHIP_EDGE_WRONG_SITE` | Edge mutation from non-owner site | edge_id, owner_site |

#### Consistency Errors (ERR_CONSISTENCY_*)

| Code | Message | Details |
|------|---------|---------|
| `ERR_CONSISTENCY_UNIQUE_VIOLATION` | Unique constraint violation | entity_type, field, value |
| `ERR_CONSISTENCY_VERSION_CONFLICT` | Optimistic lock conflict | entity_type, node_id |
| `ERR_CONSISTENCY_SCHEMA_MISMATCH` | Schema version incompatible | entity_type, expected, actual |

#### Timeout Errors (ERR_TIMEOUT_*)

| Code | Message | Details |
|------|---------|---------|
| `ERR_TIMEOUT_QUERY` | Query execution timeout | timeout_ms, elapsed_ms |
| `ERR_TIMEOUT_TRAVERSAL` | Traversal timeout | max_depth, reached_depth |
| `ERR_TIMEOUT_TRANSACTION` | FDB transaction too old | |

#### Resource Errors (ERR_RESOURCE_*)

| Code | Message | Details |
|------|---------|---------|
| `ERR_RESOURCE_TRAVERSAL_LIMIT` | Traversal result limit exceeded | max_results, current_count |
| `ERR_RESOURCE_CDC_EXPIRED` | CDC entry expired (retention) | requested_versionstamp |
| `ERR_RESOURCE_QUOTA_EXCEEDED` | Storage quota exceeded | namespace |

#### Internal Errors (ERR_INTERNAL_*)

| Code | Message | Details |
|------|---------|---------|
| `ERR_INTERNAL_FDB_UNAVAILABLE` | FDB cluster unavailable | |
| `ERR_INTERNAL_CODEC_FAILURE` | Serialization/deserialization failure | |
| `ERR_INTERNAL_UNEXPECTED` | Unexpected internal error | message |

### 14.3 gRPC Status Code Mapping

| PelagoDB Category | gRPC Code | Numeric |
|-------------------|-----------|---------|
| Validation | `INVALID_ARGUMENT` | 3 |
| Referential (not found) | `NOT_FOUND` | 5 |
| Referential (type mismatch) | `INVALID_ARGUMENT` | 3 |
| Ownership | `PERMISSION_DENIED` | 7 |
| Consistency (unique) | `ALREADY_EXISTS` | 6 |
| Consistency (conflict) | `ABORTED` | 10 |
| Timeout (query) | `DEADLINE_EXCEEDED` | 4 |
| Timeout (transaction) | `ABORTED` | 10 |
| Resource (limit) | `RESOURCE_EXHAUSTED` | 8 |
| Resource (CDC expired) | `OUT_OF_RANGE` | 11 |
| Internal (FDB unavailable) | `UNAVAILABLE` | 14 |
| Internal (other) | `INTERNAL` | 13 |

### 14.4 ErrorDetail Proto

```protobuf
message ErrorDetail {
  // Unique error code (e.g., "ERR_VALIDATION_MISSING_REQUIRED")
  string error_code = 1;

  // Human-readable error message
  string message = 2;

  // Error category (e.g., "VALIDATION", "REFERENTIAL")
  string category = 3;

  // Contextual metadata
  map<string, string> metadata = 4;

  // Field-level errors (for multi-field validation)
  repeated FieldError field_errors = 5;
}

message FieldError {
  // Field that has the error
  string field_name = 1;

  // Field-specific error code
  string error_code = 2;

  // Error message for this field
  string message = 3;
}
```

**Usage in gRPC:**

Errors are returned via gRPC's standard error metadata mechanism:

```rust
fn to_grpc_status(error: &PelagoError) -> tonic::Status {
    let code = error.grpc_code();
    let message = error.message();

    let detail = ErrorDetail {
        error_code: error.code().to_string(),
        message: message.clone(),
        category: error.category().to_string(),
        metadata: error.metadata(),
        field_errors: error.field_errors(),
    };

    let mut status = tonic::Status::new(code, message);

    // Attach ErrorDetail as binary metadata
    let detail_bytes = encode_proto(&detail);
    status.metadata_mut().insert_bin(
        "pelago-error-detail-bin",
        MetadataValue::from_bytes(&detail_bytes)
    );

    status
}
```

---

## 15. Configuration

### 15.1 Server Configuration

PelagoDB server configuration uses environment variables with CLI flag overrides.

| Environment Variable | CLI Flag | Default | Description |
|---------------------|----------|---------|-------------|
| `PELAGO_FDB_CLUSTER` | `--fdb-cluster` | `/etc/foundationdb/fdb.cluster` | FDB cluster file path |
| `PELAGO_SITE_ID` | `--site-id` | (required) | Site identifier (u8, 0-255) |
| `PELAGO_LISTEN_ADDR` | `--listen-addr` | `[::1]:27615` | gRPC listen address |
| `PELAGO_LOG_LEVEL` | `--log-level` | `info` | Log level filter |
| `PELAGO_LOG_FORMAT` | `--log-format` | `pretty` | Log format: `pretty`, `json` |
| `PELAGO_WORKER_THREADS` | `--workers` | (num CPUs) | Tokio worker thread count |
| `PELAGO_CDC_RETENTION_DAYS` | `--cdc-retention` | `7` | CDC retention in days |
| `PELAGO_JOB_BATCH_SIZE` | `--job-batch` | `1000` | Background job batch size |

**Example Startup:**

```bash
# Minimal
export PELAGO_SITE_ID=1
pelago-server

# Full configuration
PELAGO_FDB_CLUSTER=/etc/fdb/prod.cluster \
PELAGO_SITE_ID=1 \
PELAGO_LISTEN_ADDR=0.0.0.0:27615 \
PELAGO_LOG_LEVEL=pelago=debug,fdb=warn \
pelago-server
```

### 15.2 Site ID

Site ID is a u8 (0-255) that uniquely identifies this server instance for:

1. **Node ID allocation:** IDs are prefixed with site_id
2. **Locality routing:** Queries route to home site for lowest latency
3. **CDC attribution:** Mutations are tagged with originating site
4. **Ownership enforcement:** Only the home site can mutate a node

**Site ID Assignment:**

- Must be unique across all sites in the deployment
- Configured at server startup, immutable for the server lifetime
- Typically matches datacenter identifier (e.g., 1=SF, 2=NYC, 3=LON)

### 15.3 Logging and Tracing

PelagoDB uses the `tracing` crate for structured logging with span-based context.

#### Log Levels

| Level | Usage |
|-------|-------|
| `error` | Unrecoverable errors, failed transactions |
| `warn` | Recoverable issues, deprecation warnings |
| `info` | Request lifecycle, job completion |
| `debug` | Query plans, index selection, CDC entries |
| `trace` | FDB operations, CBOR encoding details |

#### Span Context

Every significant operation creates a tracing span:

```rust
#[instrument(
    skip(db, properties),
    fields(
        entity_type = %entity_type,
        request_id = %request_id,
    )
)]
async fn create_node(
    db: &FdbDatabase,
    entity_type: &str,
    properties: HashMap<String, Value>,
    request_id: &str,
) -> Result<Node> {
    // Nested spans for sub-operations
    let node_id = {
        let _span = tracing::info_span!("allocate_id").entered();
        allocate_node_id(db, entity_type).await?
    };

    // ...
}
```

#### Log Output Formats

**Pretty (development):**

```
2026-02-13T10:30:45.123Z INFO create_node{entity_type=Person request_id=abc123}: pelago_api::node_service: Node created node_id=1_42
```

**JSON (production):**

```json
{
  "timestamp": "2026-02-13T10:30:45.123Z",
  "level": "INFO",
  "target": "pelago_api::node_service",
  "message": "Node created",
  "span": {
    "entity_type": "Person",
    "request_id": "abc123"
  },
  "fields": {
    "node_id": "1_42"
  }
}
```

---

## 16. Testing Strategy

### 16.1 Test Layers

PelagoDB uses a two-layer testing strategy:

```
┌─────────────────────────────────────────────────────────────┐
│ Layer 2: Python gRPC Tests (API Contract)                    │
│ - Client perspective                                         │
│ - Proto message validation                                   │
│ - Error code verification                                    │
│ - End-to-end workflows                                       │
└────────────────────────────┬────────────────────────────────┘
                             │ gRPC over network
┌────────────────────────────▼────────────────────────────────┐
│ Layer 1: Rust Integration Tests (Storage Layer)              │
│ - Direct FDB access                                          │
│ - Key layout verification                                    │
│ - Transaction atomicity                                      │
│ - Index correctness                                          │
│ - CDC entry format                                           │
└─────────────────────────────────────────────────────────────┘
```

### 16.2 Test Categories

#### Storage Layer Tests (Rust)

| Category | Tests |
|----------|-------|
| **Schema** | Registration, versioning, forward references, evolution |
| **Node CRUD** | Create, read, update, delete with validation |
| **Index** | Entry creation, deletion, range scans, null handling |
| **Edge** | Bidirectional pairs, cascade delete, cross-namespace |
| **CDC** | Entry format, versionstamp ordering, consumer pattern |
| **Query** | CEL compilation, index selection, plan execution |
| **Atomicity** | Transaction rollback, conflict detection |

#### API Contract Tests (Python)

| Category | Tests |
|----------|-------|
| **Schema API** | Register, get, list, version history |
| **Node API** | CRUD operations, validation errors, field projection |
| **Edge API** | Create/delete, listing with filters, streaming |
| **Query API** | FindNodes with CEL, Traverse, Explain |
| **Admin API** | DropIndex, StripProperty, job status |
| **Error Handling** | Error codes, metadata, field errors |
| **Lifecycle** | Full workflow: schema -> nodes -> edges -> query -> cleanup |

### 16.3 Test Fixtures

#### Rust Test Fixtures

```rust
/// Test fixture that creates an isolated namespace
pub struct TestNamespace {
    db: FdbDatabase,
    database: String,
    namespace: String,
}

impl TestNamespace {
    pub async fn new() -> Self {
        let db = fdb::open(test_cluster_file())?;
        let database = "test".to_string();
        let namespace = format!("test_{}", uuid::Uuid::new_v4());

        Self { db, database, namespace }
    }

    /// Clean up namespace after test
    pub async fn cleanup(&self) {
        let tx = self.db.create_transaction()?;
        let range = (self.database.as_str(), self.namespace.as_str()).range();
        tx.clear_range(&range);
        tx.commit().await.ok();
    }
}

impl Drop for TestNamespace {
    fn drop(&mut self) {
        // Synchronous cleanup in drop
        futures::executor::block_on(self.cleanup());
    }
}
```

#### Python Test Fixtures

```python
# tests/python/conftest.py

import pytest
import grpc
import subprocess
import time
import uuid

@pytest.fixture(scope="session")
def pelago_server():
    """Start pelago-server for test session."""
    proc = subprocess.Popen(
        ["cargo", "run", "--release", "--bin", "pelago-server"],
        env={
            "PELAGO_SITE_ID": "1",
            "PELAGO_LISTEN_ADDR": "127.0.0.1:27616",
            "PELAGO_FDB_CLUSTER": "/etc/foundationdb/fdb.cluster",
        }
    )

    # Wait for server ready
    for _ in range(30):
        try:
            channel = grpc.insecure_channel("127.0.0.1:27616")
            grpc.channel_ready_future(channel).result(timeout=1)
            break
        except grpc.FutureTimeoutError:
            time.sleep(0.5)

    yield proc

    proc.terminate()
    proc.wait()


@pytest.fixture
def grpc_channel(pelago_server):
    """Create gRPC channel."""
    return grpc.insecure_channel("127.0.0.1:27616")


@pytest.fixture
def fresh_namespace(grpc_channel):
    """Create unique namespace for test isolation."""
    namespace = f"test_{uuid.uuid4().hex[:8]}"

    yield {
        "database": "test",
        "namespace": namespace,
    }

    # Cleanup: drop namespace
    admin_stub = AdminServiceStub(grpc_channel)
    admin_stub.DropNamespace(DropNamespaceRequest(
        context=RequestContext(database="test", namespace=namespace)
    ))


@pytest.fixture
def person_schema(fresh_namespace, grpc_channel):
    """Register Person schema in fresh namespace."""
    schema_stub = SchemaServiceStub(grpc_channel)

    schema_stub.RegisterSchema(RegisterSchemaRequest(
        context=RequestContext(**fresh_namespace),
        schema=EntitySchema(
            name="Person",
            version=1,
            properties={
                "name": PropertyDef(type=PROPERTY_TYPE_STRING, required=True, index=INDEX_TYPE_UNIQUE),
                "age": PropertyDef(type=PROPERTY_TYPE_INT, index=INDEX_TYPE_RANGE),
                "email": PropertyDef(type=PROPERTY_TYPE_STRING, index=INDEX_TYPE_UNIQUE),
            },
            meta=SchemaMeta(extras_policy=EXTRAS_POLICY_REJECT),
        )
    ))

    return fresh_namespace
```

#### Test Execution

```bash
# Run Rust integration tests
cargo test --test integration

# Run specific test category
cargo test --test integration schema_

# Run Python API tests
cd tests/python
pytest -v

# Run with coverage
pytest --cov=pelago --cov-report=html

# Run specific test file
pytest test_nodes.py -v
```

---

## 18. Reactive Subscriptions (Watch System)

The Watch System provides real-time change notifications via server-streaming gRPC. Clients subscribe to data changes using point watches, query watches, or namespace watches. All subscriptions are powered by the CDC infrastructure defined in Section 11.

### 18.1 Overview

The Watch System enables three primary use cases:

1. **Reactive UIs:** Watch specific nodes/edges and receive updates as they change
2. **Materialized Views:** Subscribe to query results and maintain external projections (Flink, Kafka, etc.)
3. **Coordination Primitives:** Wait for a specific condition to become true ("watch until X happens")

**Design Principles:**

- **CDC-powered:** All watch events are derived from CDC entries, sharing infrastructure with replication
- **gRPC streaming:** Server-streaming RPCs minimize connection overhead compared to WebSocket
- **Position-based resume:** Clients track versionstamps to resume after disconnection
- **Server-side filtering:** CDC events are filtered server-side to reduce network traffic
- **Resource-bounded:** Subscription limits and TTLs prevent unbounded resource consumption

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           Watch System Architecture                          │
└─────────────────────────────────────────────────────────────────────────────┘

                                  CDC Log (FDB)
                                       │
                                       ▼
                        ┌──────────────────────────────┐
                        │     CDC Consumer Pool        │
                        │  (shared with replication)   │
                        └──────────────────────────────┘
                                       │
                          CDC entries flow to...
                                       │
                                       ▼
                        ┌──────────────────────────────┐
                        │    Subscription Registry     │
                        │  ┌────────────────────────┐  │
                        │  │ Point Watch: node_42   │──┼──► gRPC stream → Client A
                        │  ├────────────────────────┤  │
                        │  │ Query Watch: age > 30  │──┼──► gRPC stream → Client B
                        │  ├────────────────────────┤  │
                        │  │ NS Watch: user_data/*  │──┼──► gRPC stream → Client C
                        │  └────────────────────────┘  │
                        └──────────────────────────────┘
```

### 18.2 WatchService gRPC API

```protobuf
service WatchService {
  // Subscribe to changes on specific nodes, edges, or properties
  rpc WatchPoint(WatchPointRequest) returns (stream WatchEvent);

  // Subscribe to changes matching a CEL or PQL query
  rpc WatchQuery(WatchQueryRequest) returns (stream WatchEvent);

  // Subscribe to all changes within a namespace
  rpc WatchNamespace(WatchNamespaceRequest) returns (stream WatchEvent);

  // List active subscriptions for this connection
  rpc ListSubscriptions(ListSubscriptionsRequest) returns (ListSubscriptionsResponse);

  // Cancel a subscription by ID
  rpc CancelSubscription(CancelSubscriptionRequest) returns (CancelSubscriptionResponse);
}
```

### 18.3 Subscription Types

#### 18.3.1 Point Watch

Point watches monitor specific nodes, edges, or properties. They are the most efficient subscription type with O(1) matching cost per CDC event.

```protobuf
message WatchPointRequest {
  RequestContext context = 1;
  repeated NodeWatch nodes = 2;
  repeated EdgeWatch edges = 3;
  bytes resume_position = 4;
  WatchOptions options = 5;
}

message NodeWatch {
  string entity_type = 1;
  string node_id = 2;
  repeated string properties = 3;  // Optional: specific properties only
}

message EdgeWatch {
  NodeRef source = 1;
  string label = 2;
  NodeRef target = 3;              // Optional: specific target
  EdgeDirection direction = 4;
}
```

**Use Cases:**

| Pattern | Example | Description |
|---------|---------|-------------|
| Single node | Watch `Person:p_42` | Notify on any change to node |
| Property subset | Watch `Person:p_42.{name,email}` | Notify only when name or email change |
| Node edges | Watch `Person:p_42 -[KNOWS]->` | Notify when KNOWS edges are added/removed |

#### 18.3.2 Query Watch

Query watches monitor all entities matching a CEL or PQL predicate. Events are emitted when entities enter, update within, or exit the result set.

```protobuf
message WatchQueryRequest {
  RequestContext context = 1;
  oneof query {
    string cel_expression = 2;
    string pql_query = 3;
  }
  string entity_type = 4;
  bool include_data = 5;
  bytes resume_position = 6;
  WatchOptions options = 7;
}
```

**Query Watch Semantics:**

- `Enter`: Entity now matches query (created or updated to match)
- `Update`: Entity still matches but properties changed
- `Exit`: Entity no longer matches (deleted or updated to not match)

#### 18.3.3 Namespace Watch

Namespace watches monitor all changes within a namespace for audit trails and external projections.

```protobuf
message WatchNamespaceRequest {
  RequestContext context = 1;
  repeated string entity_types = 2;     // Filter by types (optional)
  repeated OperationType operation_types = 3;
  bytes resume_position = 4;
  WatchOptions options = 5;
}
```

### 18.4 Watch Events

```protobuf
message WatchEvent {
  bytes event_id = 1;
  bytes position = 2;                   // Resume position
  WatchEventType event_type = 3;
  int64 timestamp = 4;
  string subscription_id = 5;
  string entity_type = 6;
  string node_id = 7;
  map<string, Value> data = 8;
  map<string, Value> changed_properties = 9;
  EdgeEventData edge = 10;
  QueryWatchMeta query_meta = 11;
  SubscriptionEndReason end_reason = 12;
}

enum WatchEventType {
  WATCH_EVENT_TYPE_UNSPECIFIED = 0;
  WATCH_EVENT_TYPE_NODE_CREATED = 1;
  WATCH_EVENT_TYPE_NODE_UPDATED = 2;
  WATCH_EVENT_TYPE_NODE_DELETED = 3;
  WATCH_EVENT_TYPE_EDGE_CREATED = 4;
  WATCH_EVENT_TYPE_EDGE_DELETED = 5;
  WATCH_EVENT_TYPE_QUERY_ENTER = 6;
  WATCH_EVENT_TYPE_QUERY_EXIT = 7;
  WATCH_EVENT_TYPE_HEARTBEAT = 8;
  WATCH_EVENT_TYPE_SUBSCRIPTION_ENDED = 9;
}
```

### 18.5 Subscription Options

```protobuf
message WatchOptions {
  uint32 ttl_seconds = 1;              // Subscription TTL (0 = server default)
  uint32 heartbeat_seconds = 2;         // Heartbeat interval
  uint32 rate_limit = 3;                // Max events per second
  BatchOptions batch = 4;               // Event batching
  bool initial_snapshot = 5;            // Emit current state first
}
```

### 18.6 CDC Integration Design

The Watch Dispatcher is a CDC consumer that routes events to active subscriptions:

```rust
struct WatchDispatcher {
    namespace: String,
    consumer_id: String,
    point_watches: RwLock<HashMap<SubscriptionId, PointWatchSubscription>>,
    query_watches: RwLock<HashMap<SubscriptionId, QueryWatchSubscription>>,
    namespace_watches: RwLock<HashMap<SubscriptionId, NamespaceWatchSubscription>>,
    node_index: RwLock<HashMap<(String, NodeId), Vec<SubscriptionId>>>,
    edge_index: RwLock<HashMap<(NodeRef, String), Vec<SubscriptionId>>>,
}
```

Watch consumers track position separately from replication consumers using key `(db, ns, _meta, cdc_checkpoints, watch_dispatcher)`.

### 18.7 Server-Side Subscription Registry

```rust
struct SubscriptionRegistry {
    dispatchers: RwLock<HashMap<String, Arc<WatchDispatcher>>>,
    subscriptions: RwLock<HashMap<SubscriptionId, SubscriptionMeta>>,
    connections: RwLock<HashMap<ConnectionId, Vec<SubscriptionId>>>,
    config: RegistryConfig,
}

struct RegistryConfig {
    max_subscriptions_per_connection: usize,  // Default: 100
    max_subscriptions_per_namespace: usize,   // Default: 10,000
    max_query_watches_per_namespace: usize,   // Default: 1,000
    default_ttl: Duration,                    // Default: 1 hour
    max_ttl: Duration,                        // Default: 24 hours
    heartbeat_interval: Duration,             // Default: 30 seconds
}
```

### 18.8 Client Reconnection and Resume Semantics

Every `WatchEvent` includes a `position` field (opaque bytes containing the versionstamp). Clients store this position to resume after disconnection.

**Resume Protocol:**

1. Client stores last received position
2. On reconnection, client passes `resume_position` in request
3. Server validates position exists in CDC log
4. Server streams from position (exclusive) forward

**Handling Expired Positions:**

If the resume position is expired (older than CDC retention), server sends `SUBSCRIPTION_ENDED` with `POSITION_EXPIRED` reason. Client should start fresh subscription.

### 18.9 Resource Management

| Resource | Limit | Scope | Behavior |
|----------|-------|-------|----------|
| Subscriptions per connection | 100 | Connection | New subscription rejected |
| Total subscriptions per namespace | 10,000 | Namespace | New subscription rejected |
| Query watches per namespace | 1,000 | Namespace | Query watch rejected |
| Matching nodes per query watch | 10,000 | Subscription | Events dropped, warning emitted |
| Event queue depth | 1,000 | Subscription | Backpressure, events dropped |

### 18.10 Error Codes

| Code | gRPC Code | Details |
|------|-----------|---------|
| `ERR_WATCH_SUBSCRIPTION_LIMIT` | `RESOURCE_EXHAUSTED` | Maximum subscriptions reached |
| `ERR_WATCH_QUERY_LIMIT` | `RESOURCE_EXHAUSTED` | Maximum query watches reached |
| `ERR_WATCH_POSITION_EXPIRED` | `OUT_OF_RANGE` | Resume position expired |
| `ERR_WATCH_INVALID_QUERY` | `INVALID_ARGUMENT` | Invalid CEL or PQL query |
| `ERR_WATCH_QUERY_TOO_COMPLEX` | `INVALID_ARGUMENT` | Query exceeds complexity limit |

### 18.11 Configuration

| Environment Variable | Default | Description |
|---------------------|---------|-------------|
| `PELAGO_WATCH_MAX_SUBS_CONNECTION` | `100` | Max subscriptions per connection |
| `PELAGO_WATCH_MAX_SUBS_NAMESPACE` | `10000` | Max subscriptions per namespace |
| `PELAGO_WATCH_MAX_QUERY_WATCHES` | `1000` | Max query watches per namespace |
| `PELAGO_WATCH_DEFAULT_TTL` | `3600` | Default TTL in seconds |
| `PELAGO_WATCH_MAX_TTL` | `86400` | Maximum TTL in seconds |
| `PELAGO_WATCH_HEARTBEAT_INTERVAL` | `30` | Heartbeat interval in seconds |

---

## 19. RocksDB Cache Layer

The RocksDB cache layer provides high-throughput reads for hot data by maintaining a CDC-driven projection of frequently accessed entities. This implements the CQRS (Command Query Responsibility Segregation) pattern where writes flow through FDB (system of record) and reads are served from RocksDB when consistency requirements permit.

### 19.1 Architecture Overview

```
┌──────────────────────────────────────────────────────────────────────────┐
│                           Cache Architecture                              │
└──────────────────────────────────────────────────────────────────────────┘

                              WRITE PATH
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                            FoundationDB                                   │
│    ┌─────────────┐    ┌─────────────┐    ┌─────────────┐                 │
│    │  /data/     │    │  /idx/      │    │  /_cdc/     │                 │
│    │  (entities) │    │  (indexes)  │    │  (log)      │                 │
│    └─────────────┘    └─────────────┘    └──────┬──────┘                 │
└─────────────────────────────────────────────────┼─────────────────────────┘
                                                  │
                            CDC Consumer          │
                                  │◄──────────────┘
                                  ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                         CDC Projector Process                             │
│    ┌─────────────────────────────────────────────────────────────┐       │
│    │  consume CDC → transform → write RocksDB → update HWM        │       │
│    └─────────────────────────────────────────────────────────────┘       │
└──────────────────────────────────┼────────────────────────────────────────┘
                                   ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                             RocksDB                                       │
│    ┌─────────────┐    ┌─────────────┐    ┌─────────────┐                 │
│    │  node:*     │    │  edge:*     │    │  meta:hwm   │                 │
│    │  (data)     │    │  (adjacency)│    │  (tracking) │                 │
│    └─────────────┘    └─────────────┘    └─────────────┘                 │
└──────────────────────────────────────────────────────────────────────────┘
                                   ▲
                              READ PATH
```

**Data Flow:**

1. Mutations commit to FDB (entities, indexes, CDC entry atomically)
2. CDC Projector consumes entries from `/_cdc/` subspace
3. Projector transforms CDC operations into RocksDB writes
4. Projector advances high-water mark (HWM) after successful projection
5. Query path checks RocksDB, verifies HWM freshness, falls back to FDB

### 19.2 RocksDB Keyspace Layout

| Prefix | Purpose | Key Format | Value |
|--------|---------|------------|-------|
| `n:` | Node data | `n:<db>:<ns>:<type>:<node_id>` | CBOR entity |
| `e:` | Edge forward | `e:<db>:<ns>:<src_type>:<src_id>:<label>:<tgt_ns>:<tgt_type>:<tgt_id>` | Edge metadata |
| `r:` | Edge reverse | `r:<db>:<ns>:<tgt_type>:<tgt_id>:<label>:<src_ns>:<src_type>:<src_id>` | `0x01` |
| `m:` | Metadata | `m:<key>` | Varies |

**Metadata Keys:**

| Key | Value | Description |
|-----|-------|-------------|
| `m:hwm` | 10-byte versionstamp | Global high-water mark |
| `m:hwm:<db>:<ns>` | 10-byte versionstamp | Per-namespace HWM |
| `m:rebuild:status` | JSON | Rebuild job status |

### 19.3 CDC Projector

The CDC Projector is a background process that consumes CDC entries and projects them to RocksDB.

```rust
pub struct CdcProjector {
    fdb: FdbDatabase,
    rocksdb: Arc<DB>,
    consumer_id: String,
    batch_size: usize,
    current_hwm: Versionstamp,
    metrics: ProjectorMetrics,
}

impl CdcProjector {
    pub async fn run(&mut self, shutdown: CancellationToken) -> Result<()> {
        loop {
            if shutdown.is_cancelled() {
                return Ok(());
            }
            match self.project_batch().await {
                Ok(count) if count > 0 => { /* continue */ }
                Ok(_) => { tokio::time::sleep(backoff).await; }
                Err(e) => { log_error(e); tokio::time::sleep(1s).await; }
            }
        }
    }
}
```

### 19.4 Cache Invalidation Strategy

Cache invalidation is **exact-key** based on CDC operations. There is no TTL-based expiration.

**Key Properties:**

1. **Exact invalidation:** Only keys affected by the CDC operation are modified
2. **No stale reads:** HWM tracking ensures readers detect stale cache state
3. **Atomic projection:** Data writes and HWM update are in the same RocksDB batch
4. **Idempotent replay:** Re-projecting the same CDC entry produces identical results

### 19.5 Read Path

| Consistency | HWM Check | Behavior |
|-------------|-----------|----------|
| Strong | None | Always read from FDB |
| Session | `cache_hwm >= fdb_read_version` | Serve from cache if HWM is fresh |
| Eventual | None | Serve from cache unconditionally |

```rust
pub async fn get_node_cached(
    fdb: &FdbDatabase,
    rocksdb: &DB,
    entity_type: &str,
    node_id: &NodeId,
    consistency: ReadConsistency,
) -> Result<Option<Node>> {
    match consistency {
        ReadConsistency::Strong => {
            get_node_from_fdb(fdb, entity_type, node_id).await
        }
        ReadConsistency::Session => {
            if let Some(cached) = rocksdb.get(&key)? {
                if cache_hwm >= fdb.get_read_version().await? {
                    return Ok(Some(decode(cached)?));
                }
            }
            get_node_from_fdb(fdb, entity_type, node_id).await
        }
        ReadConsistency::Eventual => {
            if let Some(cached) = rocksdb.get(&key)? {
                return Ok(Some(decode(cached)?));
            }
            // Cache miss: read from FDB and populate cache
            let node = get_node_from_fdb(fdb, entity_type, node_id).await?;
            if let Some(ref n) = node {
                rocksdb.put(&key, &encode(n))?;
            }
            Ok(node)
        }
    }
}
```

### 19.6 High-Water Mark Tracking

The high-water mark (HWM) is a FDB versionstamp representing the last CDC entry successfully projected to RocksDB.

**Session Consistency Guarantee:**

1. Client writes node via FDB (CDC entry at version V1)
2. CDC Projector processes entry, updates HWM to V1
3. Client reads with Session consistency
4. Read acquires FDB read version (V2, where V2 >= V1)
5. If HWM >= V2, cache is fresh — return cached value
6. If HWM < V2, cache is stale — read from FDB

### 19.7 Traversal Caching

Edge adjacency information is cached in RocksDB to accelerate multi-hop traversals.

Forward edges support efficient "find all targets from source" queries:
```
e:<db>:<ns>:<src_type>:<src_id>:<label>:*
```

Reverse edges support efficient "find all sources to target" queries:
```
r:<db>:<ns>:<tgt_type>:<tgt_id>:<label>:*
```

### 19.8 Cache Warm-up and Rebuild

**Rebuild from CDC Replay:**

On startup or after cache corruption, the projector can replay the full CDC stream starting from zero versionstamp.

**Warm-up Strategy:**

Optional warm-up populates the cache with frequently accessed data:
- Recently updated nodes (hot data)
- Specific entity types (schema-driven)

### 19.9 Configuration

| Environment Variable | Default | Description |
|---------------------|---------|-------------|
| `PELAGO_CACHE_ENABLED` | `true` | Enable RocksDB cache layer |
| `PELAGO_CACHE_PATH` | `./data/cache` | RocksDB data directory |
| `PELAGO_CACHE_SIZE_MB` | `1024` | Block cache size (MB) |
| `PELAGO_CACHE_WRITE_BUFFER_MB` | `64` | Write buffer size (MB) |
| `PELAGO_CACHE_PROJECTOR_BATCH` | `1000` | CDC projector batch size |
| `PELAGO_CACHE_WARM_ON_START` | `false` | Enable cache warm-up |

### 19.10 Eviction Policy

RocksDB manages eviction automatically via LRU block cache:
- Hot data remains in cache
- Cold data is evicted and re-read from disk or FDB on demand
- Block size is 4KB by default

### 19.11 Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `pelago_cache_hits_total` | Counter | Cache hit count |
| `pelago_cache_misses_total` | Counter | Cache miss count |
| `pelago_cache_hit_ratio` | Gauge | Hit ratio |
| `pelago_cache_hwm_lag_ms` | Gauge | HWM lag behind FDB |
| `pelago_cache_projector_entries` | Counter | CDC entries projected |
| `pelago_cache_projector_errors` | Counter | Projection errors |

---

## 20. Security and Authorization

This section defines the authentication, authorization, and audit model for PelagoDB. The design follows HashiCorp Vault's path-based ACL paradigm, mapping permission paths directly to the FDB keyspace structure.

### 20.1 Security Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              Client Request                                  │
│                    (API Key / mTLS / Service Token)                         │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                          Authentication Layer                                │
│   • API Key validation (Argon2id hash)                                       │
│   • mTLS certificate verification                                            │
│   • Service account token validation (JWT)                                   │
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
│   • Evaluate capabilities (read, write, admin, deny)                        │
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

### 20.2 Authentication Mechanisms

PelagoDB supports three authentication methods:

#### 20.2.1 API Keys

**Format:** `plg_<version>_<random_bytes>`
```
plg_v1_a3f8c9d2e1b4a5f6c7d8e9f0a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8
└─┬─┘└┬┘└──────────────────────────────────────────────────────────────┘
prefix ver                    32 random bytes (base62)
```

**Header Transport:** `Authorization: Bearer plg_v1_...`

#### 20.2.2 mTLS (Mutual TLS)

Client certificates are mapped to principals via fingerprint:
- Certificate fingerprint → Principal lookup in FDB
- Subject CN can be used for service identification

#### 20.2.3 Service Account Tokens

JWT tokens with PelagoDB-specific claims:
```json
{
  "iss": "pelago:site_1",
  "sub": "svc:etl-pipeline",
  "aud": "pelago",
  "exp": 1709912345,
  "jti": "unique-token-id",
  "pelago": {
    "principal_id": "svc_12345",
    "capabilities": ["read", "write"]
  }
}
```

### 20.3 Principal Model

```rust
struct Principal {
    principal_id: String,
    principal_type: PrincipalType,  // user, service, operator
    name: String,
    policies: Vec<String>,          // policy names
    inline_policy: Option<Policy>,
    metadata: HashMap<String, String>,
    disabled: bool,
}
```

### 20.4 Path-Based Permission Model

Paths map directly to the FDB keyspace hierarchy:

```
<database>/<namespace>/<subspace>/<entity_type>/<node_id>
```

**Path Components:**

| Segment | FDB Mapping | Example Values |
|---------|-------------|----------------|
| `<database>` | Database ID | `production`, `staging` |
| `<namespace>` | Namespace | `core`, `analytics` |
| `<subspace>` | Data subspace | `data`, `edge`, `_schema`, `_cdc` |
| `<entity_type>` | Entity type name | `Person`, `Order` |
| `<node_id>` | Node ID | `1_42` |

**Wildcards:**

| Pattern | Meaning | Example |
|---------|---------|---------|
| `*` | Single segment | `*/core/data/*` matches `prod/core/data/Person` |
| `+` | One or more segments | `production/+` matches `production/core/data/Person/1_42` |

**Example Paths:**

| Path | Matches |
|------|---------|
| `*/*/data/*` | All data in all databases/namespaces |
| `production/core/data/Person/*` | All Person nodes in production/core |
| `*/analytics/+` | Full access to analytics namespace |
| `*/*/edge/*` | All edges in all namespaces |
| `*/*/_schema/*` | Read schemas across all namespaces |

### 20.5 Policy Definition

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

**Capabilities:**

| Capability | Description | Implies |
|------------|-------------|---------|
| `read` | Read data | - |
| `write` | Create, update, delete data | `read` |
| `admin` | Schema changes, index management, drops | `read`, `write` |
| `deny` | Explicit deny (overrides other rules) | - |

**Evaluation Algorithm:**

1. Collect all matching rules (policy rules that match the request path)
2. Sort by specificity (more specific paths first, then by rule order)
3. For each matching rule:
   - If `deny` is present, return Deny
   - If required capability is present or implied, return Allow
4. If no matching rule allows, return Deny (implicit deny)

### 20.6 Built-in Roles

| Role | Policy | Description |
|------|--------|-------------|
| `pelago:admin` | Full admin | Complete access to all paths |
| `pelago:operator` | System operator | CDC, replication, job management |
| `pelago:reader` | Global read | Read-only access to all data |
| `pelago:schema-admin` | Schema management | Register/modify schemas, no data access |

### 20.7 Policy Storage in FDB

Security objects are stored in a dedicated system subspace:

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

### 20.8 RequestContext Integration

```protobuf
message RequestContext {
  string database = 1;
  string namespace = 2;
  string site_id = 3;
  string request_id = 4;
  AuthContext auth = 10;  // Set by auth interceptor
}

message AuthContext {
  string principal_id = 1;
  PrincipalType principal_type = 2;
  repeated string effective_policies = 3;
  string credential_id = 4;
  CredentialType credential_type = 5;
  string client_ip = 6;
  string user_agent = 7;
}
```

### 20.9 Token and Credential Management

**API Key Lifecycle:**

| Operation | Permission Required |
|-----------|---------------------|
| Create API key | `_sys/auth/credentials/*:admin` |
| Rotate API key | `_sys/auth/credentials/*:admin` |
| Revoke API key | `_sys/auth/credentials/*:admin` |

**Key Rotation:**

1. Generate new key with same `key_id` suffix
2. New key becomes active immediately
3. Old key remains valid for grace period (default: 24 hours)
4. Old key hash moved to `revoked` subspace after grace period

### 20.10 Audit Logging

All security-relevant operations are logged:

| Event Type | Trigger |
|------------|---------|
| `auth.success` | Successful authentication |
| `auth.failure` | Failed authentication attempt |
| `authz.allowed` | Authorization check passed |
| `authz.denied` | Authorization check failed |
| `principal.created` | New principal created |
| `credential.revoked` | Credential revoked |

Audit entries use FDB versionstamps for ordering:
```
/pelago/_sys/auth/audit/(<versionstamp>) → AuditEntry CBOR
```

**Retention:** Configurable, default 90 days.

### 20.11 gRPC Auth Service

```protobuf
service AuthService {
  // API Key management
  rpc CreateApiKey(CreateApiKeyRequest) returns (CreateApiKeyResponse);
  rpc RotateApiKey(RotateApiKeyRequest) returns (RotateApiKeyResponse);
  rpc RevokeApiKey(RevokeApiKeyRequest) returns (RevokeApiKeyResponse);

  // Principal management
  rpc CreatePrincipal(CreatePrincipalRequest) returns (CreatePrincipalResponse);
  rpc UpdatePrincipal(UpdatePrincipalRequest) returns (UpdatePrincipalResponse);
  rpc DisablePrincipal(DisablePrincipalRequest) returns (DisablePrincipalResponse);

  // Policy management
  rpc CreatePolicy(CreatePolicyRequest) returns (CreatePolicyResponse);
  rpc UpdatePolicy(UpdatePolicyRequest) returns (UpdatePolicyResponse);
  rpc DeletePolicy(DeletePolicyRequest) returns (DeletePolicyResponse);

  // Introspection
  rpc WhoAmI(WhoAmIRequest) returns (WhoAmIResponse);
  rpc CheckPermission(CheckPermissionRequest) returns (CheckPermissionResponse);
}
```

### 20.12 Security Configuration

| Environment Variable | Default | Description |
|---------------------|---------|-------------|
| `PELAGO_AUTH_ENABLED` | `true` | Enable authentication |
| `PELAGO_AUTH_ALLOW_ANONYMOUS` | `false` | Allow unauthenticated requests |
| `PELAGO_AUTH_TOKEN_SIGNING_KEY` | (required) | Ed25519 private key for JWT |
| `PELAGO_AUDIT_ENABLED` | `true` | Enable audit logging |
| `PELAGO_AUDIT_RETENTION_DAYS` | `90` | Audit log retention |

### 20.13 Error Codes

| Error Code | gRPC Status | Description |
|------------|-------------|-------------|
| `ERR_UNAUTHENTICATED` | `UNAUTHENTICATED` | No valid credentials provided |
| `ERR_INVALID_API_KEY` | `UNAUTHENTICATED` | API key not found or invalid |
| `ERR_EXPIRED_API_KEY` | `UNAUTHENTICATED` | API key has expired |
| `ERR_PERMISSION_DENIED` | `PERMISSION_DENIED` | Insufficient permissions |
| `ERR_PRINCIPAL_DISABLED` | `PERMISSION_DENIED` | Principal account is disabled |
| `ERR_POLICY_NOT_FOUND` | `NOT_FOUND` | Referenced policy does not exist |

### 20.14 Security Considerations

#### Credential Storage

- API key secrets are **never stored in plaintext**; only Argon2id hashes
- JWT signing keys should be stored in secure key management (HSM, Vault)
- Certificate private keys never touch PelagoDB servers

#### Transport Security

- All client connections should use TLS 1.3
- mTLS required for service-to-service in production
- Internal replication uses separate mTLS certificates

#### Rate Limiting

| Operation | Default Limit |
|-----------|---------------|
| Failed auth attempts | 10/minute per IP |
| Token issuance | 100/minute per principal |
| Policy updates | 10/minute per principal |

---

## Appendix A: Error Code Registry

Complete reference of all PelagoDB error codes.

### Validation Errors

| Code | gRPC | Message Template |
|------|------|------------------|
| `ERR_VALIDATION_UNREGISTERED_TYPE` | `INVALID_ARGUMENT` | Entity type '{entity_type}' is not registered |
| `ERR_VALIDATION_MISSING_REQUIRED` | `INVALID_ARGUMENT` | Required property '{field}' is missing |
| `ERR_VALIDATION_TYPE_MISMATCH` | `INVALID_ARGUMENT` | Property '{field}' expected {expected}, got {actual} |
| `ERR_VALIDATION_EXTRA_PROPERTY` | `INVALID_ARGUMENT` | Unexpected property '{field}' (extras_policy=reject) |
| `ERR_VALIDATION_CEL_SYNTAX` | `INVALID_ARGUMENT` | CEL syntax error at position {pos}: {message} |
| `ERR_VALIDATION_CEL_TYPE` | `INVALID_ARGUMENT` | CEL type error: {message} |
| `ERR_VALIDATION_INVALID_ID` | `INVALID_ARGUMENT` | Invalid ID format: '{id}' |
| `ERR_VALIDATION_INVALID_VALUE` | `INVALID_ARGUMENT` | Invalid value for '{field}': {reason} |
| `ERR_VALIDATION_SCHEMA_VERSION` | `INVALID_ARGUMENT` | Schema version must increment (current: {current}) |

### Referential Errors

| Code | gRPC | Message Template |
|------|------|------------------|
| `ERR_REF_NODE_NOT_FOUND` | `NOT_FOUND` | Node {entity_type}:{node_id} not found |
| `ERR_REF_EDGE_NOT_FOUND` | `NOT_FOUND` | Edge from {source} to {target} via {label} not found |
| `ERR_REF_SOURCE_NOT_FOUND` | `NOT_FOUND` | Source node {entity_type}:{node_id} not found |
| `ERR_REF_TARGET_NOT_FOUND` | `NOT_FOUND` | Target node {entity_type}:{node_id} not found |
| `ERR_REF_TYPE_NOT_REGISTERED` | `NOT_FOUND` | Referenced type '{entity_type}' not registered |
| `ERR_REF_TARGET_TYPE_MISMATCH` | `INVALID_ARGUMENT` | Edge {edge_type} expects target type {expected}, got {actual} |
| `ERR_REF_UNDECLARED_EDGE` | `INVALID_ARGUMENT` | Edge type '{edge_type}' not declared on {entity_type} |
| `ERR_REF_SCHEMA_NOT_FOUND` | `NOT_FOUND` | Schema for '{entity_type}' not found |

### Ownership Errors

| Code | gRPC | Message Template |
|------|------|------------------|
| `ERR_OWNERSHIP_WRONG_SITE` | `PERMISSION_DENIED` | Node {node_id} owned by site {home_site}, request from {request_site} |
| `ERR_OWNERSHIP_EDGE_WRONG_SITE` | `PERMISSION_DENIED` | Edge owned by site {owner_site}, request from {request_site} |

### Consistency Errors

| Code | gRPC | Message Template |
|------|------|------------------|
| `ERR_CONSISTENCY_UNIQUE_VIOLATION` | `ALREADY_EXISTS` | Unique constraint violated: {entity_type}.{field}={value} |
| `ERR_CONSISTENCY_VERSION_CONFLICT` | `ABORTED` | Optimistic lock conflict on {entity_type}:{node_id} |
| `ERR_CONSISTENCY_SCHEMA_MISMATCH` | `FAILED_PRECONDITION` | Schema version mismatch for {entity_type} |

### Timeout Errors

| Code | gRPC | Message Template |
|------|------|------------------|
| `ERR_TIMEOUT_QUERY` | `DEADLINE_EXCEEDED` | Query timeout after {elapsed_ms}ms (limit: {timeout_ms}ms) |
| `ERR_TIMEOUT_TRAVERSAL` | `DEADLINE_EXCEEDED` | Traversal timeout at depth {depth} |
| `ERR_TIMEOUT_TRANSACTION` | `ABORTED` | FDB transaction too old (>5s) |

### Resource Errors

| Code | gRPC | Message Template |
|------|------|------------------|
| `ERR_RESOURCE_TRAVERSAL_LIMIT` | `RESOURCE_EXHAUSTED` | Traversal result limit exceeded: {count}/{max_results} |
| `ERR_RESOURCE_CDC_EXPIRED` | `OUT_OF_RANGE` | CDC entry expired (retention: {retention_days} days) |
| `ERR_RESOURCE_QUOTA_EXCEEDED` | `RESOURCE_EXHAUSTED` | Storage quota exceeded for namespace '{namespace}' |

### Internal Errors

| Code | gRPC | Message Template |
|------|------|------------------|
| `ERR_INTERNAL_FDB_UNAVAILABLE` | `UNAVAILABLE` | FDB cluster unavailable |
| `ERR_INTERNAL_CODEC_FAILURE` | `INTERNAL` | Serialization error: {message} |
| `ERR_INTERNAL_UNEXPECTED` | `INTERNAL` | Internal error: {message} |

---

## Appendix B: Key Layout Quick Reference

Summary of all FDB key patterns used by PelagoDB.

### System Keys

| Subspace | Key Pattern | Value |
|----------|-------------|-------|
| System Config | `(_sys, config)` | System config CBOR |
| DC Registry | `(_sys, dc, <dc_id>)` | DC config CBOR |
| DC Topology | `(_sys, dc_topo, <dc_a>, <dc_b>)` | Topology CBOR |

### Database Keys

| Subspace | Key Pattern | Value |
|----------|-------------|-------|
| DB Config | `(<db>, _db, config)` | DB config CBOR |
| DB Lock | `(<db>, _db, lock)` | Lock state or null |
| Namespace Registry | `(<db>, _db, ns, <ns_name>)` | NS info CBOR |

### Namespace Keys

| Subspace | Key Pattern | Value |
|----------|-------------|-------|
| NS Config | `(<db>, <ns>, _ns, config)` | NS config CBOR |
| NS Stats | `(<db>, <ns>, _ns, stats)` | Statistics CBOR |

### Schema Keys

| Subspace | Key Pattern | Value |
|----------|-------------|-------|
| Schema Version | `(<db>, <ns>, _schema, <type>, v, <ver>)` | Schema CBOR |
| Latest Pointer | `(<db>, <ns>, _schema, <type>, latest)` | u32 version |
| Type Registry | `(<db>, <ns>, _types, <type>)` | Type info CBOR |

### Data Keys

| Subspace | Key Pattern | Value |
|----------|-------------|-------|
| Node Data | `(<db>, <ns>, data, <type>, <node_id>)` | Node CBOR |
| Locality Index | `(<db>, <ns>, loc, <type>, <node_id>)` | 3-char site |

### Index Keys

| Index Type | Key Pattern | Value |
|------------|-------------|-------|
| Unique | `(<db>, <ns>, idx, unique, <type>, <field>, <value>)` | node_id bytes |
| Equality | `(<db>, <ns>, idx, eq, <type>, <field>, <value>, <node_id>)` | empty |
| Range | `(<db>, <ns>, idx, range, <type>, <field>, <enc_value>, <node_id>)` | empty |
| By Locality | `(<db>, <ns>, idx, byloc, <dc>, <type>, <node_id>)` | 0x01 |
| By Update | `(<db>, <ns>, idx, byupd, <type>, <versionstamp>)` | node_id |

### Edge Keys

| Edge Type | Key Pattern | Value |
|-----------|-------------|-------|
| Forward | `(<db>, <ns>, edge, f, <src_type>, <src_id>, <label>, [<sort_key>,] <tgt_db>, <tgt_ns>, <tgt_type>, <tgt_id>)` | timestamp |
| Forward Meta | `(<db>, <ns>, edge, fm, <src_type>, <src_id>, <label>, <tgt_db>, <tgt_ns>, <tgt_type>, <tgt_id>)` | Edge CBOR |
| Reverse | `(<db>, <ns>, edge, r, <tgt_db>, <tgt_ns>, <tgt_type>, <tgt_id>, <label>, <src_type>, <src_id>)` | 0x01 |

### CDC Keys

| Subspace | Key Pattern | Value |
|----------|-------------|-------|
| CDC Entry | `(<db>, <ns>, _cdc, <versionstamp>)` | CdcEntry CBOR |
| CDC Checkpoint | `(<db>, <ns>, _meta, cdc_checkpoints, <consumer_id>)` | Versionstamp |

### Job Keys

| Subspace | Key Pattern | Value |
|----------|-------------|-------|
| Job State | `(<db>, <ns>, _jobs, <job_type>, <job_id>)` | JobState CBOR |

### ID Allocation Keys

| Subspace | Key Pattern | Value |
|----------|-------------|-------|
| Node Counter | `(<db>, <ns>, _ids, <entity_type>, <site_id>)` | u64 LE |
| Edge Counter | `(<db>, <ns>, _ids, _edges, <site_id>)` | u64 LE |

---

*End of PelagoDB Specification v1.0*
