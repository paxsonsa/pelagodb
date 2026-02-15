---
date: 2026-02-13T10:30:00-08:00
researcher: Claude
git_commit: HEAD (initial commit pending)
branch: main
repository: pelagodb
topic: "Query API and CLI Design for PelagoDB"
tags: [research, api-design, cli, grpc, query-layer]
status: complete
last_updated: 2026-02-13
last_updated_by: Claude
---

# Research: Query API and CLI Design for PelagoDB

**Date**: 2026-02-13T10:30:00-08:00
**Researcher**: Claude
**Git Commit**: HEAD (initial commit pending)
**Branch**: main
**Repository**: pelagodb

## Research Question

Design the Query API layer and CLI for PelagoDB, building on the existing specs (`graph-db-spec-v0.3.md` and `pelago-fdb-keyspace-spec.md`). Define how users interact with the system through both programmatic gRPC and command-line interfaces.

---

## Summary

This document proposes a cohesive Query API and CLI design for PelagoDB that:

1. **Leverages existing work** from `phase-1-plan.md` (gRPC services) and enhances it
2. **Adopts modern CLI patterns** from EdgeDB/Gel (noun-verb hierarchy) and SurrealDB (interactive REPL)
3. **Implements CEL-based queries** with progressive enhancement (basic filtering → full traversals)
4. **Supports three interaction modes**: embedded library, gRPC client, CLI

---

## Detailed Design

### 1. Three-Layer Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        CLI Layer                             │
│  pelago schema register, pelago query, pelago node create   │
└──────────────────────────┬──────────────────────────────────┘
                           │ gRPC (or embedded)
┌──────────────────────────▼──────────────────────────────────┐
│                      API Layer                               │
│  SchemaService, NodeService, EdgeService, QueryService       │
│  gRPC handlers, validation, CEL compilation                  │
└──────────────────────────┬──────────────────────────────────┘
                           │ Rust API
┌──────────────────────────▼──────────────────────────────────┐
│                    Core Engine                               │
│  Query planner, FDB transactions, index management           │
└─────────────────────────────────────────────────────────────┘
```

---

### 2. CLI Design

#### 2.1 Design Philosophy

Inspired by **EdgeDB/Gel** (noun-verb hierarchy, consistent patterns) and **SurrealDB** (interactive REPL, `--pretty` output):

- **Noun-verb ordering**: `pelago schema register` not `pelago register-schema`
- **Interactive REPL**: `pelago sql` for ad-hoc querying
- **Consistent flags**: `--datastore`, `--namespace` everywhere
- **Progressive output**: `--json` for machines, `--pretty` for humans

#### 2.2 Command Hierarchy

```
pelago
├── schema
│   ├── register <file.json>        # Register/update schema from file
│   ├── get <type>                  # Show schema for entity type
│   ├── list                        # List all schemas in namespace
│   └── diff <type> <v1> <v2>       # Compare schema versions
│
├── node
│   ├── create <type> [--props JSON]  # Create node
│   ├── get <type> <id>               # Get node by ID
│   ├── update <type> <id> [--props JSON]
│   ├── delete <type> <id>
│   └── list <type> [--limit N]       # List nodes of type
│
├── edge
│   ├── create <src> <label> <tgt>    # Create edge
│   ├── delete <edge-id>
│   └── list <type> <id> [--dir in|out|both]
│
├── query
│   ├── find <type> <cel-expr>        # Find nodes by CEL
│   ├── traverse <start> <steps>      # Multi-hop traversal
│   └── explain <type> <cel-expr>     # Show query plan
│
├── sql                               # Interactive REPL
│
├── index
│   ├── create <type> <field> <kind>  # Create index
│   ├── drop <type> <field>           # Drop index
│   └── list <type>                   # List indexes
│
├── admin
│   ├── job status <job-id>           # Check job progress
│   ├── job list                      # List active jobs
│   ├── drop-type <type>              # Drop entity type
│   └── drop-namespace <ns>           # Drop namespace
│
├── connect <address>                 # Connect to server
├── config                            # Manage local config
└── version
```

#### 2.3 Global Flags

```
--datastore, -d     Datastore name (default: from config)
--namespace, -n     Namespace name (default: from config)
--site-id, -s       Site identifier for locality
--format            Output format: auto, json, table, csv
--pretty            Pretty-print output
--quiet, -q         Suppress non-essential output
```

#### 2.4 Interactive REPL (`pelago sql`)

```
$ pelago sql
Connected to pelago://localhost:27615
Datastore: starwars | Namespace: core | Site: SF

pelago> find Person where age >= 30 && name.startsWith("A")
┌────────────┬──────────────┬─────┐
│ id         │ name         │ age │
├────────────┼──────────────┼─────┤
│ p_42       │ Andrew       │ 38  │
│ p_108      │ Alicia       │ 45  │
└────────────┴──────────────┴─────┘
2 results (3.2ms)

pelago> explain Person where age >= 30
Strategy: range_scan
Index: age (range)
Estimated rows: 8000
Cost: 8000

pelago> traverse Person:p_42 -[KNOWS]-> -[WORKS_AT]-> Company
...

pelago> :use graphics                  # Switch namespace
Switched to namespace: graphics

pelago> :format json                   # Change output format
Output format: json

pelago> :help
Commands:
  find <type> where <cel>              Find nodes matching CEL expression
  get <type> <id>                      Get node by ID
  traverse <start> <steps>             Graph traversal
  explain <type> where <cel>           Show query plan
  :use <namespace>                     Switch namespace
  :format <json|table|csv>             Set output format
  :history                             Show command history
  :quit                                Exit REPL
```

#### 2.5 CLI Implementation (Clap)

```rust
use clap::{Parser, Subcommand, Args};

#[derive(Parser)]
#[command(name = "pelago", version, about = "PelagoDB command-line interface")]
struct Cli {
    #[clap(flatten)]
    global: GlobalOpts,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Args)]
struct GlobalOpts {
    /// Datastore name
    #[arg(short = 'd', long, env = "PELAGO_DATASTORE")]
    datastore: Option<String>,

    /// Namespace name
    #[arg(short = 'n', long, env = "PELAGO_NAMESPACE")]
    namespace: Option<String>,

    /// Server address
    #[arg(long, default_value = "localhost:27615", env = "PELAGO_SERVER")]
    server: String,

    /// Output format
    #[arg(long, value_enum, default_value = "auto")]
    format: OutputFormat,
}

#[derive(Subcommand)]
enum Commands {
    /// Schema management
    Schema(SchemaCmd),
    /// Node operations
    Node(NodeCmd),
    /// Edge operations
    Edge(EdgeCmd),
    /// Query operations
    Query(QueryCmd),
    /// Interactive query REPL
    Sql,
    /// Index management
    Index(IndexCmd),
    /// Administrative operations
    Admin(AdminCmd),
}

#[derive(Args)]
struct SchemaCmd {
    #[command(subcommand)]
    action: SchemaAction,
}

#[derive(Subcommand)]
enum SchemaAction {
    /// Register or update a schema from file
    Register {
        /// Path to schema JSON file
        file: PathBuf,
    },
    /// Get schema for entity type
    Get {
        /// Entity type name
        entity_type: String,
        /// Schema version (default: latest)
        #[arg(short, long)]
        version: Option<u32>,
    },
    /// List all schemas
    List,
}

#[derive(Subcommand)]
enum QueryAction {
    /// Find nodes matching CEL expression
    Find {
        /// Entity type
        entity_type: String,
        /// CEL filter expression
        #[arg(name = "cel")]
        cel_expression: String,
        /// Maximum results
        #[arg(short, long, default_value = "100")]
        limit: u32,
        /// Consistency level
        #[arg(short, long, value_enum, default_value = "session")]
        consistency: Consistency,
    },
    /// Explain query plan without executing
    Explain {
        /// Entity type
        entity_type: String,
        /// CEL filter expression
        cel_expression: String,
    },
}
```

---

### 3. gRPC API Design

#### 3.1 Service Overview

| Service | Purpose | Phase |
|---------|---------|-------|
| `SchemaService` | Schema registration and retrieval | 1 |
| `NodeService` | Node CRUD operations | 1 |
| `EdgeService` | Edge CRUD and listing | 1 |
| `QueryService` | CEL-based queries, traversals, EXPLAIN | 1-2 |
| `AdminService` | Index management, jobs, lifecycle | 1 |
| `WatchService` | Reactive subscriptions | 3 |

#### 3.2 Complete Proto Definitions

```protobuf
syntax = "proto3";
package pelago.v1;

import "google/protobuf/struct.proto";
import "google/protobuf/timestamp.proto";

// ============================================================================
// Common Types
// ============================================================================

message NodeRef {
  string entity_type = 1;
  string node_id = 2;
}

message EdgeRef {
  string edge_id = 1;
}

message Value {
  oneof kind {
    string string_value = 1;
    int64 int_value = 2;
    double float_value = 3;
    bool bool_value = 4;
    int64 timestamp_value = 5;  // Unix microseconds
    bytes bytes_value = 6;
  }
}

enum ReadConsistency {
  READ_CONSISTENCY_UNSPECIFIED = 0;
  READ_CONSISTENCY_STRONG = 1;    // Direct FDB read
  READ_CONSISTENCY_SESSION = 2;   // RocksDB with freshness check (default)
  READ_CONSISTENCY_EVENTUAL = 3;  // RocksDB only
}

message RequestContext {
  string datastore = 1;
  string namespace = 2;
  string site_id = 3;        // For write operations
  string request_id = 4;     // For tracing
}

// ============================================================================
// Schema Service
// ============================================================================

service SchemaService {
  rpc RegisterSchema(RegisterSchemaRequest) returns (RegisterSchemaResponse);
  rpc GetSchema(GetSchemaRequest) returns (GetSchemaResponse);
  rpc ListSchemas(ListSchemasRequest) returns (ListSchemasResponse);
}

message PropertyDef {
  string name = 1;
  string type = 2;           // string, int, float, bool, timestamp, bytes
  bool required = 3;
  string index = 4;          // unique, equality, range, or empty
  Value default_value = 5;
}

message EdgeDef {
  string name = 1;
  string target = 2;         // Entity type name or "*" for polymorphic
  string direction = 3;      // outgoing, bidirectional
  repeated PropertyDef properties = 4;
  string sort_key = 5;       // Property name for vertex-centric index
}

message RegisterSchemaRequest {
  RequestContext context = 1;
  string entity_type = 2;
  repeated PropertyDef properties = 3;
  repeated EdgeDef edges = 4;
  bool allow_undeclared_edges = 5;
  string extras_policy = 6;  // reject, allow, warn
}

message RegisterSchemaResponse {
  uint32 version = 1;
  bool created = 2;          // true if new, false if updated
  string job_id = 3;         // If index backfill started
}

message GetSchemaRequest {
  RequestContext context = 1;
  string entity_type = 2;
  uint32 version = 3;        // 0 = latest
}

message GetSchemaResponse {
  string entity_type = 1;
  uint32 version = 2;
  repeated PropertyDef properties = 3;
  repeated EdgeDef edges = 4;
  google.protobuf.Timestamp created_at = 5;
}

message ListSchemasRequest {
  RequestContext context = 1;
}

message ListSchemasResponse {
  repeated SchemaInfo schemas = 1;
}

message SchemaInfo {
  string entity_type = 1;
  uint32 latest_version = 2;
  int64 entity_count = 3;
}

// ============================================================================
// Node Service
// ============================================================================

service NodeService {
  rpc CreateNode(CreateNodeRequest) returns (CreateNodeResponse);
  rpc GetNode(GetNodeRequest) returns (GetNodeResponse);
  rpc UpdateNode(UpdateNodeRequest) returns (UpdateNodeResponse);
  rpc DeleteNode(DeleteNodeRequest) returns (DeleteNodeResponse);
  rpc BatchCreateNodes(stream CreateNodeRequest) returns (BatchCreateNodesResponse);
}

message CreateNodeRequest {
  RequestContext context = 1;
  string entity_type = 2;
  string node_id = 3;        // Optional, server-generated if empty
  map<string, Value> properties = 4;
  string batch_id = 5;       // Optional, for bulk import tracking
}

message CreateNodeResponse {
  string node_id = 1;
  uint64 version = 2;        // Optimistic concurrency version
}

message GetNodeRequest {
  RequestContext context = 1;
  string entity_type = 2;
  string node_id = 3;
  ReadConsistency consistency = 4;
  repeated string fields = 5;    // Field projection (empty = all)
}

message GetNodeResponse {
  string entity_type = 1;
  string node_id = 2;
  map<string, Value> properties = 3;
  uint32 schema_version = 4;
  string home_site = 5;
  uint64 version = 6;
  google.protobuf.Timestamp created_at = 7;
  google.protobuf.Timestamp updated_at = 8;
}

message UpdateNodeRequest {
  RequestContext context = 1;
  string entity_type = 2;
  string node_id = 3;
  map<string, Value> properties = 4;  // Merged with existing
  uint64 expected_version = 5;        // Optimistic concurrency (0 = skip check)
}

message UpdateNodeResponse {
  uint64 version = 1;
}

message DeleteNodeRequest {
  RequestContext context = 1;
  string entity_type = 2;
  string node_id = 3;
  bool cascade_edges = 4;    // Also delete all edges (default: false, reject if edges exist)
}

message DeleteNodeResponse {
  int32 edges_deleted = 1;   // If cascade_edges was true
}

message BatchCreateNodesResponse {
  int32 created = 1;
  int32 failed = 2;
  repeated string failed_ids = 3;
}

// ============================================================================
// Edge Service
// ============================================================================

service EdgeService {
  rpc CreateEdge(CreateEdgeRequest) returns (CreateEdgeResponse);
  rpc DeleteEdge(DeleteEdgeRequest) returns (DeleteEdgeResponse);
  rpc ListEdges(ListEdgesRequest) returns (stream EdgeResult);
}

message CreateEdgeRequest {
  RequestContext context = 1;
  NodeRef source = 2;
  NodeRef target = 3;
  string edge_type = 4;
  map<string, Value> properties = 5;
}

message CreateEdgeResponse {
  string edge_id = 1;
  string pair_id = 2;        // For bidirectional edges
}

message DeleteEdgeRequest {
  RequestContext context = 1;
  string edge_id = 1;
}

message DeleteEdgeResponse {
  bool deleted = 1;
}

enum EdgeDirection {
  EDGE_DIRECTION_UNSPECIFIED = 0;
  EDGE_DIRECTION_OUT = 1;
  EDGE_DIRECTION_IN = 2;
  EDGE_DIRECTION_BOTH = 3;
}

message ListEdgesRequest {
  RequestContext context = 1;
  NodeRef node = 2;
  string edge_type = 3;      // Filter by type (empty = all)
  EdgeDirection direction = 4;
  string cel_filter = 5;     // Filter on edge properties
  int32 limit = 6;
  bytes cursor = 7;
}

message EdgeResult {
  string edge_id = 1;
  string edge_type = 2;
  NodeRef source = 3;
  NodeRef target = 4;
  map<string, Value> properties = 5;
  google.protobuf.Timestamp created_at = 6;
  bytes next_cursor = 7;     // In last message only
}

// ============================================================================
// Query Service
// ============================================================================

service QueryService {
  rpc FindNodes(FindNodesRequest) returns (stream NodeResult);
  rpc Explain(ExplainRequest) returns (ExplainResponse);
  rpc Traverse(TraverseRequest) returns (stream TraverseResult);  // Phase 2
}

message FindNodesRequest {
  RequestContext context = 1;
  string entity_type = 2;
  string cel_expression = 3;
  ReadConsistency consistency = 4;
  repeated string fields = 5;    // Field projection
  int32 limit = 6;               // Max results (default: 1000)
  bytes cursor = 7;              // Pagination cursor
}

message NodeResult {
  string entity_type = 1;
  string node_id = 2;
  map<string, Value> properties = 3;
  bytes next_cursor = 4;         // In last message only
}

message ExplainRequest {
  RequestContext context = 1;
  string entity_type = 2;
  string cel_expression = 3;
}

message ExplainResponse {
  QueryPlan plan = 1;
  repeated string steps = 2;     // Human-readable explanation
  double estimated_cost = 3;
  double estimated_rows = 4;
}

message QueryPlan {
  string strategy = 1;           // point_lookup, range_scan, merge_join, full_scan
  repeated IndexOperation index_ops = 2;
  string residual_filter = 3;    // CEL for non-indexed predicates
}

message IndexOperation {
  string field = 1;
  string operator = 2;           // ==, >=, >, <, <=
  string value = 3;
  double estimated_rows = 4;
}

// Phase 2: Traversal
message TraverseRequest {
  RequestContext context = 1;
  NodeRef start = 2;
  repeated TraversalStep steps = 3;
  uint32 max_depth = 4;          // Default: 4, server max enforced
  uint32 timeout_ms = 5;         // Default: 5000
  uint32 max_results = 6;        // Default: 10000
  ReadConsistency consistency = 7;
}

message TraversalStep {
  string edge_type = 1;          // Edge type to traverse
  EdgeDirection direction = 2;
  string edge_filter = 3;        // CEL on edge properties
  string node_filter = 4;        // CEL on target node properties
  repeated string fields = 5;    // Node field projection
}

message TraverseResult {
  int32 depth = 1;
  repeated NodeRef path = 2;     // Breadcrumb from start
  NodeRef node = 3;
  map<string, Value> properties = 4;
  EdgeRef edge = 5;              // Edge that led here
}

// ============================================================================
// Admin Service
// ============================================================================

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
  string field = 3;
  string index_type = 4;         // unique, equality, range
}

message CreateIndexResponse {
  string job_id = 1;             // Backfill job
}

message DropIndexRequest {
  RequestContext context = 1;
  string entity_type = 2;
  string field = 3;
}

message DropIndexResponse {
  bool dropped = 1;
}

message ListIndexesRequest {
  RequestContext context = 1;
  string entity_type = 2;
}

message ListIndexesResponse {
  repeated IndexInfo indexes = 1;
}

message IndexInfo {
  string field = 1;
  string index_type = 2;
  string status = 3;             // active, backfilling
}

message StripPropertyRequest {
  RequestContext context = 1;
  string entity_type = 2;
  string property = 3;
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
  string status = 3;             // pending, running, completed, failed
  int64 total = 4;
  int64 processed = 5;
  string error = 6;
  google.protobuf.Timestamp created_at = 7;
  google.protobuf.Timestamp updated_at = 8;
}

message ListJobsRequest {
  RequestContext context = 1;
  string status_filter = 2;      // Optional: pending, running, etc.
}

message ListJobsResponse {
  repeated GetJobStatusResponse jobs = 1;
}

message DropEntityTypeRequest {
  RequestContext context = 1;
  string entity_type = 2;
}

message DropEntityTypeResponse {
  string cleanup_job_id = 1;     // Orphan edge cleanup job
}

message DropNamespaceRequest {
  RequestContext context = 1;
  bool confirm = 2;              // Must be true to proceed
}

message DropNamespaceResponse {
  bool dropped = 1;
}
```

---

### 4. Query Language Design

#### 4.1 CEL as the Foundation

PelagoDB uses CEL (Common Expression Language) for all predicate expressions. This provides:

- **Type safety**: Expressions validated against schema at compile time
- **Portability**: Same expressions work in gRPC, CLI, and watches
- **Performance**: AST can be decomposed for index optimization

#### 4.2 Query Syntax in REPL

The REPL uses a simplified syntax that compiles to CEL:

```
# REPL syntax
find Person where age >= 30 && name.startsWith("A")

# Compiles to CEL
cel_expression = "age >= 30 && name.startsWith(\"A\")"
entity_type = "Person"
```

#### 4.3 Traversal Syntax

```
# REPL traversal syntax
traverse Person:p_42 -[KNOWS]-> -[WORKS_AT]-> Company

# Parsed to TraverseRequest
start = NodeRef("Person", "p_42")
steps = [
  TraversalStep { edge_type: "KNOWS", direction: OUT },
  TraversalStep { edge_type: "WORKS_AT", direction: OUT,
                  node_filter: "entity_type == 'Company'" }
]
```

#### 4.4 CEL Extensions

Custom functions added to the CEL environment:

```cel
# Standard CEL (built-in)
field == value
field >= value
field.startsWith("prefix")
field.contains("substr")

# PelagoDB extensions
field in ["a", "b", "c"]        # Multi-value equality
exists(field)                    # Field is not null
```

---

### 5. Wire Protocol Details

#### 5.1 Pagination

All list/find operations use cursor-based pagination (per AIP-158):

- `cursor`: Opaque bytes, encodes last position + scan direction
- `limit`: Client-requested page size (server may cap)
- Response includes `next_cursor` in the last streamed message
- Empty `next_cursor` indicates end of results

#### 5.2 Streaming

Server-streaming RPCs (`FindNodes`, `ListEdges`, `Traverse`) use:

- `tokio::sync::mpsc` channels for backpressure
- Client can cancel stream at any point
- Server respects `timeout_ms` and `max_results` limits

#### 5.3 Error Handling

gRPC status codes mapping:

| Error | gRPC Code | Description |
|-------|-----------|-------------|
| Entity not found | `NOT_FOUND` | Node/edge doesn't exist |
| Schema not found | `NOT_FOUND` | Entity type not registered |
| Validation failed | `INVALID_ARGUMENT` | CEL syntax error, schema violation |
| Locality violation | `PERMISSION_DENIED` | Write from non-owning site |
| Write locked | `FAILED_PRECONDITION` | DB/namespace locked |
| Timeout | `DEADLINE_EXCEEDED` | Query exceeded timeout |
| Conflict | `ABORTED` | Optimistic concurrency failure |

---

### 6. Implementation Phases

#### Phase 1: Foundation
- [ ] Proto definitions finalized
- [ ] `pelago-proto` crate with tonic codegen
- [ ] Basic CLI scaffolding (clap)
- [ ] SchemaService, NodeService, EdgeService (unary only)
- [ ] FindNodes with single-predicate CEL
- [ ] Explain endpoint

#### Phase 2: Query Engine
- [ ] Multi-predicate CEL decomposition
- [ ] Index intersection strategies
- [ ] ListEdges streaming
- [ ] FindNodes streaming with pagination
- [ ] Traverse (multi-hop)
- [ ] Interactive REPL (`pelago sql`)

#### Phase 3: Advanced Features
- [ ] WatchService (FDB native watches)
- [ ] Query watches via CDC
- [ ] Read coalescing
- [ ] RocksDB cache layer

---

## Code References

- `llm/context/graph-db-spec-v0.3.md` - Main architecture spec
- `llm/context/pelago-fdb-keyspace-spec.md` - FDB keyspace layout
- `llm/context/phase-1-plan.md:192-232` - Existing gRPC service stubs
- `llm/context/research-c1-index-intersection.md` - Query planning strategy

---

## Architecture Insights

### Design Decisions Made

1. **CEL over custom query language**: Leverages existing tooling, type-checked, portable
2. **Noun-verb CLI pattern**: Following EdgeDB/Gel for consistency
3. **Cursor-based pagination**: Follows Google AIP-158, handles concurrent mutations
4. **Server-streaming for results**: Enables backpressure and partial results
5. **Interactive REPL**: Lower barrier to entry for exploration

### Patterns from Research

| Pattern | Source | Application |
|---------|--------|-------------|
| Superflags | Dgraph | Potential for complex index configs |
| `:command` in REPL | Neo4j cypher-shell | REPL meta-commands |
| `--format` flag | SurrealDB | Output format selection |
| Cursor pagination | Google AIP-158 | All streaming RPCs |
| Noun-verb hierarchy | EdgeDB/Gel | CLI command structure |

---

## Open Questions

1. **Composite index syntax**: How to specify multi-field composite indexes in schema?
2. **Batch mutations in CLI**: Stream from file or inline JSON array?
3. **REPL history persistence**: Store in `~/.pelago/history` or XDG dirs?
4. **Auth integration**: API keys, mTLS, or both?
5. **Query caching**: Cache compiled CEL expressions? Cache query plans?

---

## Related Documents

- `llm/context/graph-db-spec-v0.3.md` - Full architecture specification
- `llm/context/pelago-fdb-keyspace-spec.md` - FDB keyspace layout
- `llm/context/phase-1-plan.md` - Implementation plan
- `llm/context/research-c1-index-intersection.md` - Query optimization research
