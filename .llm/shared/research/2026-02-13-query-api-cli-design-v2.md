---
date: 2026-02-13T15:30:00-08:00
researcher: Claude
git_commit: HEAD (initial commit pending)
branch: main
repository: pelagodb
topic: "Query API and CLI Design for PelagoDB (v2 - PQL Integrated)"
tags: [research, api-design, cli, grpc, query-layer, pql, traversal]
status: complete
last_updated: 2026-02-13
last_updated_by: Claude
supersedes: 2026-02-13-query-api-cli-design.md
---

# Research: Query API and CLI Design for PelagoDB (v2 - PQL Integrated)

**Date**: 2026-02-13T15:30:00-08:00
**Researcher**: Claude
**Git Commit**: HEAD (initial commit pending)
**Branch**: main
**Repository**: pelagodb
**Supersedes**: `2026-02-13-query-api-cli-design.md`

## Research Question

Consolidate the Query API, CLI, and PQL (Pelago Query Language) designs into a unified specification. PQL becomes the native query language for both the REPL and API, replacing ad-hoc command syntax.

---

## Summary

This document presents the unified query layer design for PelagoDB:

1. **PQL as the single query language** — DQL-inspired nested blocks with CEL predicates
2. **REPL is PQL-native** — `pelago repl` opens a REPL where users type PQL directly
3. **Client-side round-trips for multi-block queries** — Parser in client, uses existing protos
4. **Full PQL in Phase 1** — Including variables, @cascade, @facets, @recurse, aggregations
5. **Upserts deferred to Phase 3** — Focus on read queries first
6. **Migration path to server-side execution** — Architecture supports future `ExecutePql` RPC

---

## 1. Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                        CLI Layer                             │
│  pelago repl (PQL REPL), pelago schema, pelago node, etc.    │
│  PQL Parser → AST → Proto Compilation                        │
└──────────────────────────┬──────────────────────────────────┘
                           │ gRPC (FindNodesRequest, TraverseRequest)
┌──────────────────────────▼──────────────────────────────────┐
│                      API Layer                               │
│  SchemaService, NodeService, EdgeService, QueryService       │
│  gRPC handlers, validation, CEL execution                    │
└──────────────────────────┬──────────────────────────────────┘
                           │ Rust API
┌──────────────────────────▼──────────────────────────────────┐
│                    Core Engine                               │
│  Query planner, FDB transactions, index management           │
└─────────────────────────────────────────────────────────────┘
```

**Key insight**: PQL parsing and multi-block execution happen in the CLI/client layer. The server only sees individual `FindNodesRequest` and `TraverseRequest` calls — no new execution engine required.

---

## 2. PQL Language Specification

### 2.1 Query Structure

```
query [query_name] {
  block_name(func: root_function) [@directive ...] {
    field_projection
    -[EDGE_TYPE]-> [@directive ...] {
      nested_projection
    }
  }
}
```

### 2.2 Root Functions

| Function | Meaning | Compiles To |
|---|---|---|
| `uid(Person:p_42)` | Single node by type:id | `NodeRef` |
| `uid(var_name)` | Nodes from captured variable | Variable reference |
| `eq(field, value)` | Equality on indexed field | CEL `field == value` |
| `ge(field, value)` / `le(field, value)` | Range comparisons | CEL `field >= value` |
| `between(field, lo, hi)` | Range | CEL `field >= lo && field <= hi` |
| `has(field)` | Field exists | CEL `field != null` |
| `type(EntityType)` | All nodes of type | `FindNodesRequest` with empty filter |

### 2.3 Directives

| Directive | Purpose | Applies To |
|---|---|---|
| `@filter(cel_expr)` | Filter target nodes | Blocks, edge traversals |
| `@edge(cel_expr)` | Filter edges by edge properties | Edge traversals only |
| `@limit(first: N, offset: M)` | Pagination / fan-out control | Blocks, edge traversals |
| `@sort(field: asc/desc)` | Order results | Blocks, edge traversals |
| `@cascade` | Require all nested paths exist | Blocks |
| `@facets(prop1, prop2)` | Project edge properties | Edge traversals |
| `@recurse(depth: N)` | Variable-depth traversal | Blocks (single edge type) |

### 2.4 Edge Traversal Syntax

```
-[EDGE_TYPE]->      # outgoing
-[EDGE_TYPE]<-      # incoming
-[EDGE_TYPE]<->     # bidirectional
```

**Explicit edge vs node filtering:**
```
-[WORKS_AT]-> @edge(role == "Engineer") @filter(name.startsWith("Tech")) {
  name
  founded
}
```
- `@edge(...)` filters on edge properties (role, started, weight)
- `@filter(...)` filters on target node properties (name, age)

### 2.5 Variables

**UID Variables** — capture node sets for use in later blocks:
```
query {
  friends as start(func: uid(Person:p_42)) {
    -[KNOWS]-> { uid }
  }

  # Use captured nodes
  mutual(func: uid(friends)) @filter(age >= 30) {
    name
  }
}
```

**Value Variables** — capture scalar aggregations:
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

**Set Operations:**
```
uid(a, b)              # Union: nodes in a OR b
uid(a, b, intersect)   # Intersection: nodes in a AND b
uid(a, b, difference)  # Difference: nodes in a but NOT b
```

### 2.6 Aggregations

```
query {
  companies(func: type(Company)) {
    name
    employee_count: count(-[WORKS_AT]<-)
    avg_tenure: avg(-[WORKS_AT]<-.started)
  }
}
```

Supported functions: `count`, `sum`, `avg`, `min`, `max`

### 2.7 @groupby

```
query {
  by_department(func: type(Person)) @groupby(department) {
    count(uid)
    avg(age)
  }
}
```

---

## 3. CLI Design

### 3.1 Command Hierarchy

```
pelago
├── repl                              # PQL REPL (primary interface)
│
├── schema
│   ├── register <file.json>          # Register/update schema
│   ├── get <type>                    # Show schema for entity type
│   ├── list                          # List all schemas
│   └── diff <type> <v1> <v2>         # Compare schema versions
│
├── node
│   ├── create <type> [--props JSON]  # Create node
│   ├── get <type> <id>               # Get node by ID
│   ├── update <type> <id> [--props]  # Update node
│   ├── delete <type> <id>            # Delete node
│   └── list <type> [--limit N]       # List nodes of type
│
├── edge
│   ├── create <src> <label> <tgt>    # Create edge
│   ├── delete <edge-id>              # Delete edge
│   └── list <type> <id> [--dir]      # List edges
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

**Note**: No `pelago query` subcommand — queries go through `pelago repl` REPL.

### 3.2 Global Flags

```
--datastore, -d     Datastore name (default: from config)
--namespace, -n     Namespace name (default: from config)
--site-id, -s       Site identifier for locality
--format            Output format: auto, json, table, csv
--pretty            Pretty-print output
--quiet, -q         Suppress non-essential output
--server            Server address (default: localhost:27615)
```

### 3.3 PQL REPL (`pelago repl`)

```
$ pelago repl
Connected to pelago://localhost:27615
Datastore: starwars | Namespace: default | Site: SF

pelago> query { people(func: eq(name, "Luke")) { name, age } }
┌────────┬──────────┬─────┐
│ block  │ name     │ age │
├────────┼──────────┼─────┤
│ people │ Luke     │ 23  │
└────────┴──────────┴─────┘
1 result (2.1ms)

pelago> :explain Person @filter(age >= 30) { name }
Query Plan:
  Block: result
  Root: type(Person)
  Strategy: range_scan(age)
  Estimated rows: ~200

pelago> :use graphics                  # Switch namespace
Switched to namespace: graphics

pelago> :format json                   # Change output format
Output format: json

pelago> :help
Commands:
  <PQL query>                          Execute PQL query
  :explain <PQL>                       Show query plan
  :use <namespace>                     Switch namespace
  :format <json|table|csv>             Set output format
  :param $name = value                 Set query parameter
  :history                             Show command history
  :quit                                Exit REPL
```

### 3.4 Short-Form PQL in REPL

For simple single-block queries, the `query { }` wrapper is optional:

```
pelago> Person @filter(age >= 30) { name, age }
```

Equivalent to:
```
query { result(func: type(Person)) @filter(age >= 30) { name, age } }
```

---

## 4. gRPC API Design

### 4.1 Service Overview

| Service | Purpose | Phase |
|---------|---------|-------|
| `SchemaService` | Schema registration and retrieval | 1 |
| `NodeService` | Node CRUD operations | 1 |
| `EdgeService` | Edge CRUD and listing | 1 |
| `QueryService` | FindNodes, Traverse, Explain | 1 |
| `AdminService` | Index management, jobs, lifecycle | 1 |
| `WatchService` | Reactive subscriptions | 3 |

### 4.2 QueryService Proto (Updated)

```protobuf
service QueryService {
  // Single-type node queries (existing)
  rpc FindNodes(FindNodesRequest) returns (stream NodeResult);

  // Multi-hop graph traversal (existing)
  rpc Traverse(TraverseRequest) returns (stream TraverseResult);

  // Query plan explanation (existing)
  rpc Explain(ExplainRequest) returns (ExplainResponse);
}

// FindNodesRequest - unchanged from v1
message FindNodesRequest {
  RequestContext context = 1;
  string entity_type = 2;
  string cel_expression = 3;
  ReadConsistency consistency = 4;
  repeated string fields = 5;
  int32 limit = 6;
  bytes cursor = 7;
}

// TraverseRequest - extended for PQL features
message TraverseRequest {
  RequestContext context = 1;
  NodeRef start = 2;
  repeated TraversalStep steps = 3;
  uint32 max_depth = 4;
  uint32 timeout_ms = 5;
  uint32 max_results = 6;
  ReadConsistency consistency = 7;
  bool cascade = 8;                    // NEW: @cascade support
  RecurseConfig recurse = 9;           // NEW: @recurse support
}

message TraversalStep {
  string edge_type = 1;
  EdgeDirection direction = 2;
  string edge_filter = 3;              // CEL on edge properties (@edge)
  string node_filter = 4;              // CEL on target node properties (@filter)
  repeated string fields = 5;          // Node field projection
  uint32 per_node_limit = 6;           // NEW: Fan-out control
  repeated string edge_fields = 7;     // NEW: @facets projection
  SortSpec sort = 8;                   // NEW: @sort support
}

message SortSpec {
  string field = 1;
  bool descending = 2;
  bool on_edge = 3;                    // Sort by edge property vs node property
}

message RecurseConfig {
  uint32 max_depth = 1;
  bool detect_cycles = 2;              // Default: true
}

// TraverseResult - extended for edge facets
message TraverseResult {
  int32 depth = 1;
  repeated NodeRef path = 2;
  NodeRef node = 3;
  map<string, Value> properties = 4;
  EdgeRef edge = 5;
  map<string, Value> edge_facets = 6;  // NEW: Edge properties if @facets
}
```

### 4.3 Future: Server-Side PQL Execution

When migrating from client round-trips to server-side execution:

```protobuf
service QueryService {
  // ... existing RPCs ...

  // Future: Execute raw PQL on server
  rpc ExecutePql(PqlRequest) returns (stream PqlResult);
  rpc ExplainPql(PqlRequest) returns (PqlExplainResponse);
}

message PqlRequest {
  RequestContext context = 1;
  string pql = 2;                      // Raw PQL text
  ReadConsistency consistency = 3;
  uint32 timeout_ms = 4;
  uint32 max_results = 5;
  map<string, Value> parameters = 6;   // $param substitution
}
```

**Migration cost estimate**: 2-3 weeks. Parser, AST, and compilation logic are reusable — we add server-side block scheduler and connection pooling benefits.

---

## 5. PQL Compilation Pipeline

### 5.1 Client-Side Execution Flow

```
PQL text (from REPL or file)
  │
  ▼
┌──────────────────────┐
│ 1. Parse             │  pest parser → PqlAst
│    (syntax check)    │
└───────┬──────────────┘
        │
        ▼
┌──────────────────────┐
│ 2. Resolve           │  Validate entity types, edge types, fields
│    (schema check)    │  Build variable dependency DAG
└───────┬──────────────┘
        │
        ▼
┌──────────────────────┐
│ 3. CEL Compile       │  Extract @filter/@edge directives
│    (type check)      │  Type-check each CEL expression
└───────┬──────────────┘
        │
        ▼
┌──────────────────────┐
│ 4. Plan & Execute    │  For each block in DAG order:
│                      │    - Compile to FindNodesRequest or TraverseRequest
│                      │    - Execute via gRPC
│                      │    - Capture variable outputs
│                      │    - Feed into dependent blocks
└───────┬──────────────┘
        │
        ▼
┌──────────────────────┐
│ 5. Aggregate Results │  Merge results from all blocks
│                      │  Apply @cascade post-filter
│                      │  Format for output
└──────────────────────┘
```

### 5.2 Crate Organization

```
pelago-query/src/
  ├─ cel.rs              ← CEL parsing, type-checking, cost model
  ├─ planner.rs          ← Index selection, predicate extraction
  ├─ executor.rs         ← FindNodes/Traverse execution
  └─ pql/
      ├─ mod.rs
      ├─ grammar.pest    ← PEG grammar
      ├─ parser.rs       ← pest parser → PqlAst
      ├─ ast.rs          ← AST type definitions
      ├─ resolver.rs     ← Schema resolution, variable DAG
      ├─ compiler.rs     ← PqlAst → proto messages
      ├─ executor.rs     ← Multi-block execution with variable capture
      └─ explain.rs      ← EXPLAIN output for PQL
```

---

## 6. Implementation Phases

### Phase 1: Full PQL (This Phase)

**Parser & AST:**
- [ ] PEG grammar for PQL (pest crate)
- [ ] Parser → PqlAst
- [ ] Schema resolver (validate types, fields, edges)

**Single-Block Queries:**
- [ ] Root functions: `uid`, `eq`, `ge`, `le`, `between`, `has`, `type`
- [ ] Field projection
- [ ] Edge traversal with `@filter` and `@edge`
- [ ] `@limit` and `@sort` directives
- [ ] Short-form syntax support

**Multi-Block Queries:**
- [ ] UID variable capture (`friends as ...`)
- [ ] Block dependency DAG
- [ ] Client-side round-trip execution
- [ ] Set operations (union, intersect, difference)

**Advanced Directives:**
- [ ] `@cascade` (subtree completeness filter)
- [ ] `@facets` (edge property projection)
- [ ] `@recurse` (variable-depth with cycle detection)
- [ ] `per_node_limit` on TraversalStep

**Aggregations:**
- [ ] Inline aggregations (count, sum, avg, min, max)
- [ ] Value variable capture
- [ ] `@groupby` directive

**REPL:**
- [ ] `pelago repl` command with PQL parsing
- [ ] `:explain` command
- [ ] `:use`, `:format`, `:param` meta-commands
- [ ] History persistence

**Proto Extensions:**
- [ ] `cascade` field on TraverseRequest
- [ ] `RecurseConfig` on TraverseRequest
- [ ] `per_node_limit`, `edge_fields`, `SortSpec` on TraversalStep
- [ ] `edge_facets` on TraverseResult

### Phase 2: Query Optimization

- [ ] Multi-predicate CEL decomposition
- [ ] Index intersection strategies
- [ ] Query plan caching (keyed by PQL hash)
- [ ] Streaming result pagination

### Phase 3: Upserts & Server-Side Execution

- [ ] Upsert block parsing
- [ ] `UpsertRequest` proto
- [ ] Atomic query-then-mutate in single FDB transaction
- [ ] `ExecutePql` RPC for server-side PQL execution
- [ ] Connection pooling benefits

---

## 7. Wire Protocol Details

### 7.1 Pagination

All streaming RPCs use cursor-based pagination:
- `cursor`: Opaque bytes encoding position + direction
- `limit`: Client-requested page size
- Response includes `next_cursor` in last streamed message
- Empty `next_cursor` = end of results

### 7.2 Error Codes

| Error | gRPC Code | Description |
|-------|-----------|-------------|
| Entity not found | `NOT_FOUND` | Node/edge doesn't exist |
| Schema not found | `NOT_FOUND` | Entity type not registered |
| PQL syntax error | `INVALID_ARGUMENT` | Parse failure |
| CEL type error | `INVALID_ARGUMENT` | Type mismatch in expression |
| Schema violation | `INVALID_ARGUMENT` | Field doesn't exist on type |
| Undefined variable | `INVALID_ARGUMENT` | Variable referenced before capture |
| Locality violation | `PERMISSION_DENIED` | Write from non-owning site |
| Timeout | `DEADLINE_EXCEEDED` | Query exceeded timeout |
| Traversal too deep | `RESOURCE_EXHAUSTED` | Exceeded max_depth or max_results |

---

## 8. Open Questions

### Resolved

1. **REPL syntax**: PQL is the only query language. No dual syntax.
2. **Phase 1 scope**: Full PQL including all directives and aggregations.
3. **Execution model**: Client round-trips, with clear migration path to server-side.
4. **Upserts**: Deferred to Phase 3.
5. **CLI structure**: `pelago repl` is PQL-native REPL, no `pelago query` commands.

### Still Open

1. **Composite index syntax**: How to specify multi-field composite indexes in PQL schema?

2. **REPL history location**: `~/.pelago/history` or XDG directories?

3. **Auth integration**: API keys, mTLS, or both?

4. **Aggregation push-down**: Compute during traversal (streaming) or after materialization?

5. **Cycle detection memory**: HashSet for small graphs, but what about large graphs? Bloom filter?

6. **Cost model for traversals**: Need to estimate fan-out at each hop. Edge degree statistics?

---

## Code References

- `llm/context/graph-db-spec-v0.3.md` - Main architecture spec
- `llm/context/pelago-fdb-keyspace-spec.md` - FDB keyspace layout
- `llm/context/phase-1-plan.md` - Implementation plan
- `llm/context/research-c1-index-intersection.md` - Query planning strategy
- `llm/context/graph-db-edge-spec.md` - Edge storage, traversal semantics

---

## Related Documents

- `.llm/shared/research/2026-02-13-query-api-cli-design.md` - Original API design (superseded)
- `.llm/shared/research/2026-02-13-traversal-query-language.md` - PQL language research
- `llm/context/graph-db-spec-v0.3.md` - Full architecture specification

---

## External References

- [DQL Query Syntax](https://dgraph.io/docs/dql/dql-syntax/dql-query/)
- [DQL Variables](https://dgraph.io/docs/query-language/query-variables/)
- [CEL Specification](https://github.com/google/cel-spec)
- [pest PEG Parser](https://pest.rs/)
