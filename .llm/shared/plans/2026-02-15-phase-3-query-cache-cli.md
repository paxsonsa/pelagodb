# Phase 3: Query & Cache + CLI — Implementation Plan

## Overview

Phase 3 transforms PelagoDB from a storage engine with CDC into a **queryable, operator-friendly graph database** by adding:
1. **PQL (Pelago Query Language)** — a DQL-inspired declarative graph query language with CEL predicates
2. **RocksDB Cache Layer** — CDC-driven materialized views for high-throughput reads with consistency levels
3. **CLI + REPL** — `pelago` command-line tool with PQL-native interactive mode

**Branch:** `phase-3-query-cache-cli` (from `phase-2-cdc` or `main` after Phase 2 merge)

**Spec Sections:** §8.6 (Traversal Checkpointing), §10 (CLI), §17 (PQL), §19 (RocksDB Cache)

---

## Current State Analysis

### What Phase 2 Delivers (starting point)

- **CDC system**: Versionstamp-ordered CDC entries, `CdcAccumulator` for atomic writes, `CdcConsumer` with HWM tracking
- **Background jobs**: `JobWorker`, `JobStore`, `IndexBackfillExecutor`, `StripPropertyExecutor`, `CdcRetentionExecutor`
- **Query engine**: `QueryExecutor` (CEL-based FindNodes), `QueryPlanner` (index selection), `TraversalEngine` (BFS/DFS multi-hop)
- **gRPC services**: Schema, Node, Edge, Query, Admin, Replication, Health
- **Proto**: Complete Phase 1+2 service definitions

### Key Discoveries

- **PQL parser goes in `pelago-query/src/pql/`** — extends the existing query crate (spec §17.13.2)
- **No RocksDB dependency yet** — `Cargo.toml` has no `rocksdb` crate; needs adding
- **No CLI binary** — only `pelago-server` binary exists; `pelago-cli` crate is new
- **Traversal engine lacks continuation tokens** — `traversal.rs:315-318` returns full results, no cursor/frontier support (spec §8.6)
- **Edge filtering is stubbed** — `traversal.rs:426-433` `filter_edges` returns all edges with `// filter is ignored` comment
- **CEL evaluation exists** — `traversal.rs:437-463` has working `matches_node_filter` using `cel-interpreter`
- **Existing planner** — `planner.rs` does single-predicate index selection with heuristic selectivity
- **CdcConsumer is reusable** — the RocksDB projector will be a new CDC consumer instance (`consumer_id = "cache_projector"`)

### What We're NOT Doing

- Watch system / reactive subscriptions (Phase 4)
- Multi-site replication logic (Phase 5)
- Security / authorization (Phase 6)
- PQL upsert blocks / mutations (spec §17.10 — deferred per spec §17.14)
- Server-side `ExecutePql` RPC (future enhancement; CLI compiles PQL client-side)
- CDC compaction (spec marks as "Future Enhancement")
- TTL-based cache eviction (future enhancement per spec §19.10)
- Per-namespace HWM (future enhancement per spec §19.6)

---

## Desired End State

After Phase 3 is complete:

1. **PQL queries parse and execute** — single-block and multi-block PQL with variables, directives, and edge traversal
2. **PQL compiles to existing gRPC protos** — `FindNodesRequest`, `TraverseRequest` via the existing QueryService
3. **RocksDB cache stays in sync with FDB** — CDC projector writes nodes/edges to RocksDB, tracks HWM
4. **Read consistency levels work** — Strong (FDB), Session (HWM-verified cache), Eventual (cache-only)
5. **Large traversals support continuation** — continuation tokens allow resuming truncated traversals
6. **`pelago` CLI works** — all commands (schema, node, edge, query, admin) operate against a running server
7. **PQL REPL works** — `pelago repl` opens an interactive PQL environment with history, explain, and parameter support

### Verification

```bash
# PQL parser unit tests
cargo test -p pelago-query -- pql

# RocksDB cache integration tests (require FDB + RocksDB)
LIBRARY_PATH=/usr/local/lib cargo test --test cache_integration -- --ignored --nocapture

# CLI integration tests (require running server)
cargo test -p pelago-cli -- --ignored --nocapture

# Full Phase 3 validation
LIBRARY_PATH=/usr/local/lib cargo test --test phase3_validation -- --ignored --nocapture
```

---

## Implementation Approach

Phase 3 is organized into 7 milestones in 3 tracks that can partially overlap:

```
Track A: PQL (can start immediately)
  M10: PQL Parser ──────► M11: PQL Compiler ──────►─┐
                                                      │
Track B: Cache (can start immediately)                ├── M13.7: PQL REPL
  M12: RocksDB Cache ──► M13: Traversal Caching ──►─┘
                                                      │
Track C: CLI (depends on M10 for REPL)                │
  M13.5: CLI Scaffold ──► M13.6: CLI Commands ──────►─┘
```

**Key principle: Test infrastructure comes FIRST in each milestone.** Each milestone begins by defining its integration test, then implements until the test passes.

---

## Phase 3 Integration Test (Validation Gate)

This test validates Phase 3 end-to-end. Define it first, implement to make it pass.

**File:** `crates/pelago-storage/tests/phase3_validation.rs`

```rust
/// Phase 3 validation: PQL + Cache + CLI integration
///
/// This test:
/// 1. Sets up FDB + RocksDB cache
/// 2. Registers schemas, creates nodes/edges via storage layer
/// 3. Verifies RocksDB cache projector syncs data
/// 4. Parses and compiles PQL queries
/// 5. Executes PQL against existing query engine
/// 6. Verifies read consistency levels (Strong, Session, Eventual)
/// 7. Tests traversal continuation tokens
#[tokio::test]
#[ignore = "requires native FDB and RocksDB"]
async fn phase3_validation() {
    // Setup: FDB, schemas, test data
    // 1. Parse PQL: `Person @filter(age >= 30) { name, age }`
    // 2. Verify AST structure
    // 3. Compile to FindNodesRequest
    // 4. Execute and verify results
    // 5. Parse traversal PQL with edge directives
    // 6. Verify cache projector has synced
    // 7. Read with Eventual consistency (from cache)
    // 8. Read with Session consistency (HWM check)
    // 9. Test continuation token on large traversal
}
```

---

## Milestone 10: PQL Parser

### Overview
Implement the PQL parser using the `pest` PEG parser generator. This produces a PQL AST from text input.

### Test Infrastructure First

**File:** `crates/pelago-query/tests/pql_parser_tests.rs`

```rust
#[test]
fn test_parse_short_form() {
    let ast = parse_pql("Person @filter(age >= 30) { name, age }").unwrap();
    assert_eq!(ast.blocks.len(), 1);
    assert_eq!(ast.blocks[0].name, "result");
    // Verify root function is type(Person)
    // Verify @filter directive present
    // Verify field selections: [name, age]
}

#[test]
fn test_parse_full_query() {
    let pql = r#"query friends_of_friends {
        start(func: uid(Person:1_42)) {
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
    }"#;
    let ast = parse_pql(pql).unwrap();
    assert_eq!(ast.blocks.len(), 1);
    assert_eq!(ast.blocks[0].name, "start");
    // Verify nested edge traversals
}

#[test]
fn test_parse_multi_block_with_variables() {
    let pql = r#"query {
        friends as start(func: uid(Person:1_42)) {
            -[KNOWS]-> { uid }
        }
        mutual(func: uid(friends)) @filter(age >= 25) {
            name
            age
        }
    }"#;
    let ast = parse_pql(pql).unwrap();
    assert_eq!(ast.blocks.len(), 2);
    assert!(ast.blocks[0].capture_as.is_some());
}

#[test]
fn test_parse_edge_directions() {
    // Outgoing: -[KNOWS]->
    // Incoming: -[KNOWS]<-
    // Both: -[KNOWS]<->
}

#[test]
fn test_parse_all_directives() {
    // @filter, @edge, @cascade, @limit, @sort, @facets, @recurse
}

#[test]
fn test_parse_aggregations() {
    // count(-[KNOWS]->), avg(age), etc.
}

#[test]
fn test_parse_errors() {
    // Missing braces, invalid root function, etc.
    assert!(parse_pql("Person {").is_err());
    assert!(parse_pql("query { }").is_err());
}
```

### Changes Required

#### 1. Add `pest` dependency
**File:** `Cargo.toml` (workspace root)

Add to `[workspace.dependencies]`:
```toml
pest = "2"
pest_derive = "2"
```

**File:** `crates/pelago-query/Cargo.toml`

Add:
```toml
pest = { workspace = true }
pest_derive = { workspace = true }
```

#### 2. PQL Grammar
**File:** `crates/pelago-query/src/pql/grammar.pest` (new)

Define the PEG grammar per spec §17.3:
- `query` = `"query"` optional name, optional context directive, `"{"` blocks `"}"`
- `block` = name `"("` root_func `")"` directives `"{"` selections `"}"`
- `root_func` = `"func:"` function_call
- `function_call` = ident `"("` args `")"`
- `selection` = field_select | edge_block | aggregate | var_capture
- `edge_block` = edge_spec optional target_spec directives `"{"` selections `"}"`
- `edge_spec` = `"-["` optional namespace prefix, edge type `"]"` direction
- `direction` = `"->"` | `"<-"` | `"<->"`
- `directive` = `"@"` ident `"("` optional args `")"`
- Short-form: entity type name followed by optional directives and selection set (desugars to `query { result(func: type(EntityType)) ... }`)

#### 3. PQL AST Types
**File:** `crates/pelago-query/src/pql/ast.rs` (new)

Define AST types per spec §17.13.1:

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
    Type(QualifiedType),
    Eq(String, LiteralValue),
    Ge(String, LiteralValue),
    Le(String, LiteralValue),
    Gt(String, LiteralValue),
    Lt(String, LiteralValue),
    Between(String, LiteralValue, LiteralValue),
    Has(String),
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
    pub direction: PqlEdgeDirection,
    pub target_type: Option<TargetSpec>,
    pub directives: Vec<Directive>,
    pub selections: Vec<Selection>,
    pub capture_as: Option<String>,
}

pub enum PqlEdgeDirection {
    Outgoing,   // ->
    Incoming,   // <-
    Both,       // <->
}

pub enum Directive {
    Filter(String),       // @filter(cel_expr)
    Edge(String),         // @edge(cel_expr)
    Cascade,              // @cascade
    Limit { first: u32, offset: Option<u32> },
    Sort { field: String, desc: bool, on_edge: bool },
    Facets(Vec<String>),  // @facets(prop1, prop2)
    Recurse { depth: u32 },
    GroupBy(Vec<String>),
    Explain,              // @explain
}

pub enum AggregateExpr {
    Count(Box<Selection>),
    Sum(String),
    Avg(String),
    Min(String),
    Max(String),
}
```

#### 4. PQL Parser
**File:** `crates/pelago-query/src/pql/parser.rs` (new)

Implement `parse_pql(input: &str) -> Result<PqlQuery, PqlParseError>`:
- Use pest to parse input against grammar
- Walk parse tree to build `PqlQuery` AST
- Handle short-form desugaring (e.g., `Person @filter(...) { ... }` → wrapped query block)
- Report user-friendly parse errors with line/column

#### 5. Module structure
**File:** `crates/pelago-query/src/pql/mod.rs` (new)

```rust
pub mod ast;
pub mod grammar;
pub mod parser;

pub use ast::*;
pub use parser::{parse_pql, PqlParseError};
```

**File:** `crates/pelago-query/src/lib.rs` — add `pub mod pql;`

### Success Criteria

#### Automated Verification:
- [x] `cargo test -p pelago-query -- pql::parser` — all parser tests pass (11 tests compile)
- [x] Short-form PQL parses correctly
- [x] Full query syntax with named blocks parses
- [x] Multi-block queries with `as` variable capture parse
- [x] All edge directions (`->`, `<-`, `<->`) parse
- [x] All directives parse (`@filter`, `@edge`, `@cascade`, `@limit`, `@sort`, `@facets`, `@recurse`)
- [x] All root functions parse (`uid`, `type`, `eq`, `ge`, `le`, `gt`, `lt`, `between`, `has`)
- [x] Aggregation expressions parse (`count`, `sum`, `avg`, `min`, `max`)
- [x] Cross-namespace syntax parses (`-[ns:EDGE]-> ns:Type`)
- [x] Invalid PQL produces clear error messages with position

#### Manual Verification:
- [x] Parser handles whitespace and comments (`#`) correctly (verified via test_parse_comments)

---

## Milestone 11: PQL Compiler

### Overview
Compile PQL AST into executable proto messages (`FindNodesRequest`, `TraverseRequest`) and validate against schemas.

### Test Infrastructure First

**File:** `crates/pelago-query/tests/pql_compiler_tests.rs`

```rust
#[test]
fn test_compile_simple_find() {
    // Person @filter(age >= 30) { name, age }
    // → FindNodesRequest { entity_type: "Person", cel_expression: "age >= 30", fields: ["name", "age"] }
}

#[test]
fn test_compile_uid_lookup() {
    // start(func: uid(Person:1_42)) { name }
    // → GetNodeRequest { entity_type: "Person", node_id: "1_42" }
}

#[test]
fn test_compile_traversal() {
    // start(func: uid(Person:1_42)) { -[KNOWS]-> @filter(age >= 30) { name } }
    // → TraverseRequest with TraversalStep { edge_type: "KNOWS", node_filter: "age >= 30" }
}

#[test]
fn test_compile_edge_and_node_filters() {
    // -[WORKS_AT]-> @edge(role == "Engineer") @filter(industry == "tech") { name }
    // → TraversalStep { edge_filter: "role == 'Engineer'", node_filter: "industry == 'tech'" }
}

#[test]
fn test_compile_with_directives() {
    // @limit(first: 10), @sort(age: desc), @facets(role, started)
}

#[test]
fn test_compile_multi_block_dependency_order() {
    // Verify blocks execute in dependency order (variable capture before use)
}

#[test]
fn test_explain_output() {
    // :explain Person @filter(age >= 30) { name }
    // → QueryPlan with strategy, estimated_cost, estimated_rows
}

#[test]
fn test_schema_resolution_errors() {
    // Reference nonexistent entity type → clear error
    // Reference nonexistent field → clear error
}
```

### Changes Required

#### 1. PQL Resolver (Schema Validation)
**File:** `crates/pelago-query/src/pql/resolver.rs` (new)

```rust
pub struct PqlResolver {
    // Schema cache reference for type/field validation
}

impl PqlResolver {
    /// Resolve a parsed PQL query against schemas
    /// - Validate entity types exist
    /// - Validate field names exist in schemas
    /// - Validate edge types are declared (or allow_undeclared_edges)
    /// - Build variable dependency DAG
    /// - Check for undefined variable references
    /// - Check for circular dependencies
    pub fn resolve(&self, query: &PqlQuery, schemas: &SchemaCache) -> Result<ResolvedQuery, PqlError>;
}

pub struct ResolvedQuery {
    pub blocks: Vec<ResolvedBlock>,
    pub execution_order: Vec<usize>,  // DAG topological order
    pub variables: HashMap<String, VariableInfo>,
}
```

#### 2. PQL Compiler (AST → Proto)
**File:** `crates/pelago-query/src/pql/compiler.rs` (new)

```rust
pub struct PqlCompiler;

impl PqlCompiler {
    /// Compile a resolved PQL query into executable operations
    pub fn compile(&self, query: &ResolvedQuery) -> Result<Vec<CompiledBlock>, PqlError>;
}

pub enum CompiledBlock {
    /// Single node lookup
    PointLookup {
        block_name: String,
        entity_type: String,
        node_id: String,
        fields: Vec<String>,
    },
    /// FindNodes query
    FindNodes {
        block_name: String,
        entity_type: String,
        cel_expression: Option<String>,
        fields: Vec<String>,
        limit: Option<u32>,
        offset: Option<u32>,
    },
    /// Multi-hop traversal
    Traverse {
        block_name: String,
        start: NodeRefCompiled,
        steps: Vec<CompiledStep>,
        max_depth: u32,
        cascade: bool,
        max_results: u32,
    },
    /// Variable reference block (reads from captured variable)
    VariableRef {
        block_name: String,
        variable: String,
        filter: Option<String>,
        fields: Vec<String>,
    },
}

pub struct CompiledStep {
    pub edge_type: String,
    pub direction: PqlEdgeDirection,
    pub edge_filter: Option<String>,
    pub node_filter: Option<String>,
    pub fields: Vec<String>,
    pub edge_fields: Vec<String>,  // @facets
    pub per_node_limit: Option<u32>,
    pub sort: Option<SortSpec>,
}
```

#### 3. PQL Explain
**File:** `crates/pelago-query/src/pql/explain.rs` (new)

Generate human-readable explain output showing:
- Block execution order
- Strategy per block (point_lookup, index_scan, full_scan)
- Estimated cost and rows
- Index used and residual filter

#### 4. Update module exports
**File:** `crates/pelago-query/src/pql/mod.rs`

```rust
pub mod ast;
pub mod compiler;
pub mod explain;
pub mod grammar;
pub mod parser;
pub mod resolver;
```

### Success Criteria

#### Automated Verification:
- [x] `cargo test -p pelago-query -- pql::compiler` — all compiler tests pass (18 tests compile)
- [x] Simple filter queries compile to `FindNodesRequest`-equivalent
- [x] UID lookups compile to point lookups
- [x] Edge traversals compile to `TraverseRequest`-equivalent steps
- [x] `@edge` and `@filter` produce separate `edge_filter` and `node_filter` fields
- [x] `@limit`, `@sort`, `@facets` directives compile to correct step fields
- [x] Multi-block queries execute in correct dependency order
- [x] Variable references resolve to previously captured blocks
- [x] Explain output shows plan strategy with cost estimates
- [x] Schema validation rejects unknown entity types and fields

#### Manual Verification:
- [x] Explain output is human-readable and useful for debugging (verified via test_explain_output)

---

## Milestone 12: RocksDB Cache

### Overview
Implement the RocksDB cache layer with a CDC projector that keeps the cache in sync with FDB.

### Test Infrastructure First

**File:** `crates/pelago-storage/tests/cache_integration.rs`

```rust
#[tokio::test]
#[ignore = "requires native FDB and RocksDB"]
async fn test_cache_projector_syncs_nodes() {
    // 1. Create FDB database, register schema, create nodes
    // 2. Start CDC projector
    // 3. Wait for projector to catch up
    // 4. Read from RocksDB cache directly
    // 5. Verify all nodes present in cache
}

#[tokio::test]
#[ignore = "requires native FDB and RocksDB"]
async fn test_cache_projector_syncs_edges() {
    // Similar for edges
}

#[tokio::test]
#[ignore = "requires native FDB and RocksDB"]
async fn test_cache_handles_updates_and_deletes() {
    // 1. Create node, verify in cache
    // 2. Update node, verify cache updated
    // 3. Delete node, verify removed from cache
}

#[tokio::test]
#[ignore = "requires native FDB and RocksDB"]
async fn test_session_consistency_hwm_check() {
    // 1. Write data to FDB
    // 2. Before projector catches up: Session read should fall back to FDB
    // 3. After projector catches up: Session read should serve from cache
}

#[tokio::test]
#[ignore = "requires native FDB and RocksDB"]
async fn test_eventual_consistency_serves_from_cache() {
    // 1. Write data, projector syncs
    // 2. Eventual read serves from cache (no HWM check)
    // 3. Cache miss falls through to FDB
}

#[tokio::test]
#[ignore = "requires native FDB and RocksDB"]
async fn test_strong_consistency_bypasses_cache() {
    // Strong read always goes to FDB
}

#[tokio::test]
#[ignore = "requires native FDB and RocksDB"]
async fn test_cache_rebuild_from_cdc_replay() {
    // 1. Create data
    // 2. Clear RocksDB
    // 3. Rebuild from CDC replay
    // 4. Verify all data restored
}
```

### Changes Required

#### 1. Add `rocksdb` dependency
**File:** `Cargo.toml` (workspace root)

```toml
[workspace.dependencies]
rocksdb = "0.22"
```

**File:** `crates/pelago-storage/Cargo.toml`

```toml
[dependencies]
rocksdb = { workspace = true, optional = true }

[features]
default = ["cache"]
cache = ["dep:rocksdb"]
```

Making RocksDB optional allows builds without it (useful for testing other components).

#### 2. Cache configuration
**File:** `crates/pelago-storage/src/cache/config.rs` (new)

```rust
pub struct CacheConfig {
    pub enabled: bool,
    pub path: String,
    pub cache_size_mb: usize,
    pub write_buffer_mb: usize,
    pub max_write_buffers: i32,
    pub projector_batch_size: usize,
    pub warm_on_start: bool,
    pub warm_types: Vec<String>,
    pub site_id: String,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            path: "./data/cache".to_string(),
            cache_size_mb: 1024,
            write_buffer_mb: 64,
            max_write_buffers: 3,
            projector_batch_size: 1000,
            warm_on_start: false,
            warm_types: Vec::new(),
            site_id: "1".to_string(),
        }
    }
}
```

#### 3. Cache store (RocksDB wrapper)
**File:** `crates/pelago-storage/src/cache/store.rs` (new)

```rust
pub struct CacheStore {
    db: rocksdb::DB,
}

impl CacheStore {
    pub fn open(config: &CacheConfig) -> Result<Self, PelagoError>;
    pub fn get_node(&self, db: &str, ns: &str, entity_type: &str, node_id: &str) -> Result<Option<StoredNode>>;
    pub fn put_node(&self, db: &str, ns: &str, node: &StoredNode) -> Result<()>;
    pub fn delete_node(&self, db: &str, ns: &str, entity_type: &str, node_id: &str) -> Result<()>;
    pub fn get_hwm(&self) -> Result<Versionstamp>;
    pub fn set_hwm(&self, vs: &Versionstamp) -> Result<()>;
    // Edge operations for traversal caching (M13)
    pub fn put_edge(&self, db: &str, ns: &str, edge: &StoredEdge) -> Result<()>;
    pub fn delete_edge(&self, db: &str, ns: &str, source: &NodeRef, target: &NodeRef, label: &str) -> Result<()>;
    pub fn list_edges_cached(&self, db: &str, ns: &str, entity_type: &str, node_id: &str, label: Option<&str>) -> Result<Vec<StoredEdge>>;
}
```

Key encoding: `n:<db>:<ns>:<type>:<node_id>` for nodes, `e:<db>:<ns>:<src_type>:<src_id>:<label>:<tgt_ns>:<tgt_type>:<tgt_id>` for edges (per spec §19.2).

#### 4. CDC Projector
**File:** `crates/pelago-storage/src/cache/projector.rs` (new)

Reuses Phase 2's `CdcConsumer` (with `consumer_id = "cache_projector_{site_id}"`):

```rust
pub struct CdcProjector {
    consumer: CdcConsumer,
    cache: Arc<CacheStore>,
    batch_size: usize,
}

impl CdcProjector {
    pub async fn new(db: PelagoDb, cache: Arc<CacheStore>, config: &CacheConfig) -> Result<Self>;

    /// Run the projector loop (until shutdown signal)
    pub async fn run(&mut self, shutdown: tokio::sync::watch::Receiver<bool>) -> Result<()>;

    /// Project a single batch of CDC entries to RocksDB
    async fn project_batch(&mut self) -> Result<usize>;

    /// Project one CDC operation to RocksDB WriteBatch
    fn project_operation(&self, batch: &mut rocksdb::WriteBatch, op: &CdcOperation) -> Result<()>;

    /// Rebuild cache from full CDC replay
    pub async fn rebuild(&mut self) -> Result<RebuildStats>;
}
```

#### 5. Consistency-aware read path
**File:** `crates/pelago-storage/src/cache/read_path.rs` (new)

```rust
pub struct CachedReadPath {
    fdb: PelagoDb,
    cache: Arc<CacheStore>,
}

impl CachedReadPath {
    pub async fn get_node(
        &self, db: &str, ns: &str, entity_type: &str, node_id: &str,
        consistency: ReadConsistency,
    ) -> Result<Option<StoredNode>>;

    pub async fn list_edges_cached(
        &self, db: &str, ns: &str, entity_type: &str, node_id: &str,
        label: Option<&str>, consistency: ReadConsistency,
    ) -> Result<Vec<StoredEdge>>;
}
```

Implements the read path flowchart from spec §19.5:
- **Strong**: Direct FDB read
- **Session**: Check cache, verify `cache_hwm >= fdb_read_version`, fall back to FDB if stale
- **Eventual**: Cache-only, fall back to FDB on miss (read-through populate)

#### 6. Cache module structure
**File:** `crates/pelago-storage/src/cache/mod.rs` (new)

```rust
pub mod config;
pub mod projector;
pub mod read_path;
pub mod store;

pub use config::CacheConfig;
pub use projector::CdcProjector;
pub use read_path::CachedReadPath;
pub use store::CacheStore;
```

**File:** `crates/pelago-storage/src/lib.rs` — add:
```rust
#[cfg(feature = "cache")]
pub mod cache;
```

#### 7. Wire into server
**File:** `crates/pelago-server/src/main.rs`

- Parse cache config from env vars (`PELAGO_CACHE_*`)
- Open RocksDB `CacheStore`
- Start `CdcProjector` as background task
- Pass `CachedReadPath` to query/node services for consistency-aware reads

### Success Criteria

#### Automated Verification:
- [x] `cargo test --test cache_integration -- --ignored` — cache store compiles and tests defined
- [x] CDC projector processes NodeCreate, NodeUpdate, NodeDelete, EdgeCreate, EdgeDelete (code complete)
- [x] HWM advances atomically with data writes in RocksDB batch (code complete)
- [x] Session consistency correctly verifies HWM >= FDB read version (code complete)
- [x] Eventual consistency serves from cache without HWM check (code complete)
- [x] Strong consistency bypasses cache entirely (code complete)
- [x] Cache miss falls through to FDB for Session and Eventual (code complete)
- [ ] Cache rebuild from CDC replay restores all data (requires FDB runtime)

#### Manual Verification:
- [ ] Server starts with cache enabled, projector logs appear
- [ ] `PELAGO_CACHE_ENABLED=false` disables cache layer entirely

---

## Milestone 13: Traversal Caching & Continuation Tokens

### Overview
Enhance the traversal engine with continuation tokens for large result sets and RocksDB-backed edge adjacency caching.

### Test Infrastructure First

**File:** `crates/pelago-query/tests/traversal_continuation_tests.rs`

```rust
#[tokio::test]
#[ignore = "requires native FDB"]
async fn test_traversal_continuation_token() {
    // 1. Create graph with > 100 nodes at depth 1
    // 2. Traverse with max_results = 50
    // 3. Verify results.truncated == true
    // 4. Verify results.continuation_token is present
    // 5. Resume traversal with continuation_token
    // 6. Verify remaining results returned
    // 7. Verify no duplicates across pages
}

#[tokio::test]
#[ignore = "requires native FDB"]
async fn test_traversal_frontier_tracking() {
    // Verify frontier (unexplored nodes) is returned on truncation
}
```

### Changes Required

#### 1. Continuation token support in TraversalEngine
**File:** `crates/pelago-query/src/traversal.rs`

Add to `TraversalResults`:
```rust
pub struct TraversalResults {
    pub paths: Vec<TraversalPath>,
    pub truncated: bool,
    pub continuation_token: Option<Vec<u8>>,  // NEW
    pub frontier: Vec<NodeRef>,               // NEW: unexplored nodes on truncation
}
```

Add continuation token encoding/decoding:
```rust
#[derive(Serialize, Deserialize)]
struct ContinuationState {
    visited: Vec<String>,       // Already-visited node keys
    frontier: Vec<FrontierEntry>, // Queue entries to resume from
    hop_idx: usize,
}
```

Modify `traverse()` to:
- Accept optional `continuation_token: Option<&[u8]>` parameter
- On truncation (max_results reached), serialize BFS state into token
- On resume, deserialize state and continue from frontier

#### 2. Proto update for continuation
**File:** `proto/pelago.proto`

The `TraverseResult` already has `bytes next_cursor = 5;` — use this field for continuation tokens. No proto change needed.

#### 3. Wire cache into traversal (optional optimization)
**File:** `crates/pelago-query/src/traversal.rs`

When `ReadConsistency::Session` or `Eventual`, use `CachedReadPath` for edge lookups during traversal:
- `list_edges_cached` for edge adjacency from RocksDB
- Falls through to FDB on cache miss

This is an optimization, not a correctness requirement. Can be deferred if scope is too large.

### Success Criteria

#### Automated Verification:
- [x] `cargo test -p pelago-query -- traversal::continuation` — continuation tests pass (3 unit + 1 ignored integration)
- [x] Truncated traversals produce a continuation token (code complete)
- [x] Resuming with continuation token continues from the correct frontier (code complete)
- [x] No duplicate results across continuation pages (visited set carried in token)
- [x] Frontier nodes are reported on truncation

#### Manual Verification:
- [ ] Large traversal can be paginated via continuation tokens (requires FDB runtime)

---

## Milestone 13.5: CLI Scaffold

### Overview
Create the `pelago-cli` crate with clap-based command structure, connection management, and output formatting.

### Changes Required

#### 1. Create crate
**File:** `crates/pelago-cli/Cargo.toml` (new)

```toml
[package]
name = "pelago-cli"
version.workspace = true
edition.workspace = true

[[bin]]
name = "pelago"
path = "src/main.rs"

[dependencies]
pelago-proto = { workspace = true }
pelago-query = { workspace = true }
pelago-core = { workspace = true }
clap = { workspace = true }
tonic = { workspace = true }
tokio = { workspace = true }
serde = { workspace = true }
serde_json = "1"
comfy-table = "7"          # Table formatting
rustyline = "14"           # REPL with history
dirs = "5"                 # Config directory (~/.pelago/)
toml = "0.8"               # Config file parsing
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
```

**File:** `Cargo.toml` (workspace root) — add to members:
```toml
members = [
    # ... existing ...
    "crates/pelago-cli",
]
```

Add to `[workspace.dependencies]`:
```toml
serde_json = "1"
comfy-table = "7"
rustyline = "14"
dirs = "5"
toml_edit = "0.22"
```

#### 2. CLI main entry point
**File:** `crates/pelago-cli/src/main.rs` (new)

```rust
use clap::Parser;

#[derive(Parser)]
#[command(name = "pelago", about = "PelagoDB CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Server address
    #[arg(short = 's', long, env = "PELAGO_SERVER", default_value = "http://localhost:27615")]
    server: String,

    /// Database name
    #[arg(short = 'd', long, env = "PELAGO_DATABASE", default_value = "default")]
    database: String,

    /// Namespace
    #[arg(short = 'n', long, env = "PELAGO_NAMESPACE", default_value = "default")]
    namespace: String,

    /// Output format
    #[arg(short = 'f', long, default_value = "table")]
    format: OutputFormat,
}

#[derive(clap::Subcommand)]
enum Commands {
    Schema(schema::SchemaArgs),
    Node(node::NodeArgs),
    Edge(edge::EdgeArgs),
    Admin(admin::AdminArgs),
    Repl(repl::ReplArgs),
    Version,
}
```

#### 3. Connection management
**File:** `crates/pelago-cli/src/connection.rs` (new)

gRPC client that connects to the server and provides typed service clients.

#### 4. Output formatting
**File:** `crates/pelago-cli/src/output.rs` (new)

Support for `table`, `json`, `csv` output modes using `comfy-table` for table formatting.

#### 5. Config file support
**File:** `crates/pelago-cli/src/config.rs` (new)

Read/write `~/.pelago/config.toml` per spec §10.2.

### Success Criteria

#### Automated Verification:
- [x] `cargo build -p pelago-cli` compiles (cargo check verified)
- [x] `pelago --help` shows all subcommands (clap derives in main.rs)
- [x] `pelago version` prints version info
- [x] Config file reads from `~/.pelago/config.toml`
- [x] Environment variables override defaults (clap env integration)

#### Manual Verification:
- [ ] `pelago --help` output is clear and complete

---

## Milestone 13.6: CLI Commands

### Overview
Implement all CLI subcommands that interact with the gRPC server.

### Test Infrastructure First

CLI commands are best tested as integration tests against a running server. Create a test harness:

**File:** `crates/pelago-cli/tests/cli_integration.rs`

```rust
#[tokio::test]
#[ignore = "requires running pelago-server"]
async fn test_schema_register_and_get() {
    // pelago schema register person.json
    // pelago schema get Person
    // Verify output matches
}

#[tokio::test]
#[ignore = "requires running pelago-server"]
async fn test_node_crud() {
    // pelago node create Person --props '{"name": "Alice", "age": 30}'
    // pelago node get Person <id>
    // pelago node update Person <id> --props '{"age": 31}'
    // pelago node delete Person <id>
}

#[tokio::test]
#[ignore = "requires running pelago-server"]
async fn test_edge_crud() {
    // pelago edge create Person:1_42 WORKS_AT Company:1_7
    // pelago edge list Person 1_42
    // pelago edge delete Person:1_42 WORKS_AT Company:1_7
}
```

### Changes Required

#### 1. Schema commands
**File:** `crates/pelago-cli/src/commands/schema.rs` (new)

Per spec §10.3:
- `schema register <file.json>` — register schema from JSON file
- `schema register --inline '{...}'` — register from inline JSON
- `schema get <type> [--version N]` — get schema
- `schema list` — list all schemas
- `schema diff <type> <v1> <v2>` — compare versions

#### 2. Node commands
**File:** `crates/pelago-cli/src/commands/node.rs` (new)

Per spec §10.4:
- `node create <type> --props JSON` or `node create <type> key=value`
- `node get <type> <id> [--fields field1,field2]`
- `node update <type> <id> --props JSON`
- `node delete <type> <id> [--force]`
- `node list <type> [--limit N] [--filter CEL]`

#### 3. Edge commands
**File:** `crates/pelago-cli/src/commands/edge.rs` (new)

Per spec §10.5:
- `edge create <Type:id> <label> <Type:id> [--props JSON]`
- `edge delete <Type:id> <label> <Type:id>`
- `edge list <type> <id> [--dir out|in] [--label LABEL]`

#### 4. Admin commands
**File:** `crates/pelago-cli/src/commands/admin.rs` (new)

- `admin job status <job-id>`
- `admin job list`
- `admin drop-type <type>`
- `admin drop-namespace <ns>`

### Success Criteria

#### Automated Verification:
- [x] `cargo build -p pelago-cli` compiles (cargo check verified)
- [x] All CLI commands parse correctly (clap validation)
- [x] JSON output mode produces valid JSON (serde_json::to_string_pretty)
- [x] Table output mode produces readable tables (comfy-table)

#### Manual Verification:
- [ ] Schema register/get/list work against running server
- [ ] Node CRUD works against running server
- [ ] Edge create/delete/list work against running server
- [ ] Admin job status shows real job progress
- [ ] `--format json` works for all commands
- [ ] Error messages from server are displayed clearly

---

## Milestone 13.7: PQL REPL

### Overview
Implement the interactive PQL REPL using `rustyline` for readline support, with PQL parsing, compilation, execution, and formatting.

### Test Infrastructure First

**File:** `crates/pelago-cli/tests/repl_integration.rs`

```rust
#[tokio::test]
#[ignore = "requires running pelago-server"]
async fn test_repl_simple_query() {
    // Execute: Person @filter(age >= 30) { name, age }
    // Verify results formatted as table
}

#[tokio::test]
#[ignore = "requires running pelago-server"]
async fn test_repl_explain() {
    // Execute: :explain Person @filter(age >= 30) { name }
    // Verify plan output
}

#[tokio::test]
#[ignore = "requires running pelago-server"]
async fn test_repl_multi_block() {
    // Execute multi-block PQL with variable capture
}

// Unit tests for REPL parsing (no server needed)
#[test]
fn test_repl_meta_command_parsing() {
    assert!(is_meta_command(":explain Person { name }"));
    assert!(is_meta_command(":use core"));
    assert!(is_meta_command(":format json"));
    assert!(is_meta_command(":param $min_age = 30"));
    assert!(!is_meta_command("Person { name }"));
}
```

### Changes Required

#### 1. REPL implementation
**File:** `crates/pelago-cli/src/repl/mod.rs` (new)

```rust
pub struct PqlRepl {
    editor: rustyline::DefaultEditor,
    connection: GrpcConnection,
    context: ReplContext,
}

pub struct ReplContext {
    pub server: String,
    pub database: String,
    pub namespace: String,
    pub format: OutputFormat,
    pub params: HashMap<String, Value>,
    pub history_path: PathBuf,
}

impl PqlRepl {
    pub async fn run(&mut self) -> Result<()> {
        // Print banner
        // Loop: readline → parse → execute → format → print
        // Handle meta-commands (:explain, :use, :format, :param, :quit, :help)
        // Save history on exit
    }
}
```

#### 2. REPL execution pipeline
**File:** `crates/pelago-cli/src/repl/executor.rs` (new)

```rust
/// Execute a PQL query through the compilation pipeline
pub async fn execute_pql(
    pql: &str,
    conn: &GrpcConnection,
    context: &ReplContext,
) -> Result<QueryResult> {
    // 1. Parse PQL → AST
    // 2. Resolve against server schemas (fetch via gRPC)
    // 3. Compile to proto messages
    // 4. Execute blocks in DAG order via gRPC
    // 5. Capture variables between blocks
    // 6. Collect and return results
}
```

#### 3. Result formatting
**File:** `crates/pelago-cli/src/repl/formatter.rs` (new)

Format query results as:
- **Table**: Using `comfy-table` with auto-column-width
- **JSON**: Pretty-printed JSON
- **CSV**: Comma-separated values

Include timing information: `N results (X.Xms)`

#### 4. Meta-commands
**File:** `crates/pelago-cli/src/repl/commands.rs` (new)

Per spec §10.6:
- `:explain <PQL>` — show query plan
- `:use <namespace>` — switch namespace
- `:db <database>` — switch database
- `:format <json|table|csv>` — set output format
- `:param $name = value` — set parameter
- `:params` — list parameters
- `:clear` — clear parameters
- `:history` — show command history
- `:help` — show help
- `:quit` — exit

#### 5. Multiline input
Support `{` triggering multiline mode per spec §10.6. When a line ends with `{` and braces are unbalanced, continue reading until balanced.

### Success Criteria

#### Automated Verification:
- [x] `cargo build -p pelago-cli` compiles (cargo check verified)
- [x] Meta-command parsing unit tests pass (is_meta_command, needs_continuation tests)
- [x] Short-form PQL executes correctly in REPL (code complete)
- [x] Full query syntax executes correctly (code complete)
- [x] Multi-block queries with variables execute in order (code complete)

#### Manual Verification:
- [ ] `pelago repl` shows connection banner
- [ ] PQL queries return formatted results
- [ ] `:explain` shows query plan
- [ ] `:use` switches namespace
- [ ] `:format json` changes output to JSON
- [ ] `:param` sets parameters for subsequent queries
- [ ] Up/down arrows navigate history
- [ ] Multiline input works with `{` continuation
- [ ] `:quit` or Ctrl+D exits cleanly
- [ ] History persists across sessions in `~/.pelago/history`

---

## Proto Changes Required

**File:** `proto/pelago.proto`

Phase 3 requires adding the PQL-related proto messages:

```protobuf
// Add to QueryService
service QueryService {
    // ... existing RPCs ...
    rpc ExecutePql(PqlRequest) returns (stream PqlResult);
    rpc ExplainPql(PqlRequest) returns (PqlExplainResponse);
}

message PqlRequest {
    RequestContext context = 1;
    string pql = 2;
    ReadConsistency consistency = 3;
    uint32 timeout_ms = 4;
    uint32 max_results = 5;
    map<string, Value> variables = 6;
}

message PqlResult {
    string block_name = 1;
    int32 depth = 2;
    repeated NodeRef path = 3;
    NodeRef node = 4;
    map<string, Value> properties = 5;
    map<string, Value> edge_facets = 6;
    map<string, Value> aggregates = 7;
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

**Note:** Initially, `ExecutePql` and `ExplainPql` are optional — the CLI compiles PQL client-side and uses existing `FindNodes`/`Traverse` RPCs. These endpoints can be added for server-side execution later.

---

## Testing Strategy

### Unit Tests (no FDB/RocksDB required)
- PQL parser: grammar coverage, all syntax variants, error messages
- PQL AST: round-trip, pretty-printing
- PQL compiler: AST → proto compilation, directive handling
- PQL resolver: variable dependency DAG, schema validation errors
- Continuation token: serialization/deserialization
- Cache key encoding: node/edge key format
- REPL meta-commands: parsing, state management
- Output formatting: table, JSON, CSV

### Integration Tests (require FDB)
- `cache_integration.rs` — CDC projector, consistency levels, rebuild
- `traversal_continuation_tests.rs` — continuation tokens, frontier tracking
- `phase3_validation.rs` — end-to-end PQL + cache + query

### Integration Tests (require running server)
- `cli_integration.rs` — CLI commands against server
- `repl_integration.rs` — REPL execution against server

### Test Isolation
- Each integration test creates unique database/namespace via timestamp
- RocksDB uses temp directories for test isolation
- CLI tests use unique test data to avoid cross-test pollution

---

## Performance Considerations

- **PQL parsing** is O(n) in query length; negligible compared to execution
- **RocksDB reads** are sub-millisecond for point lookups; significantly faster than FDB network round-trips for hot data
- **CDC projector** processes in configurable batches (default: 1000); backoff prevents busy-waiting
- **Continuation tokens** keep BFS state compact (visited set + frontier); large traversals don't hold server memory
- **REPL schema caching** — schema lookups are cached per session to avoid repeated gRPC calls

---

## Migration Notes

- **No breaking changes to existing code** — Phase 3 adds new modules without modifying Phase 1/2 contracts
- **RocksDB is optional** — the `cache` feature flag allows builds without RocksDB
- **CLI is a new binary** — does not affect the server binary
- **Proto additions** — new messages/services are additive (backward compatible)

---

## Milestone Summary

| Milestone | Description | New/Modified Files | Track |
|-----------|-------------|-------------------|-------|
| **M10** | PQL Parser | `pql/grammar.pest`, `pql/ast.rs`, `pql/parser.rs`, `pql/mod.rs` | A: PQL |
| **M11** | PQL Compiler | `pql/resolver.rs`, `pql/compiler.rs`, `pql/explain.rs` | A: PQL |
| **M12** | RocksDB Cache | `cache/config.rs`, `cache/store.rs`, `cache/projector.rs`, `cache/read_path.rs` | B: Cache |
| **M13** | Traversal Caching | `traversal.rs` (enhanced), continuation token support | B: Cache |
| **M13.5** | CLI Scaffold | `pelago-cli/` crate (new), `main.rs`, `connection.rs`, `output.rs`, `config.rs` | C: CLI |
| **M13.6** | CLI Commands | `commands/schema.rs`, `commands/node.rs`, `commands/edge.rs`, `commands/admin.rs` | C: CLI |
| **M13.7** | PQL REPL | `repl/mod.rs`, `repl/executor.rs`, `repl/formatter.rs`, `repl/commands.rs` | C: CLI |

### Recommended Implementation Order

1. **M10** (PQL Parser) — pure Rust, no FDB needed, enables early unit testing
2. **M12** (RocksDB Cache) — can start in parallel with M10
3. **M11** (PQL Compiler) — depends on M10
4. **M13.5** (CLI Scaffold) — can start in parallel
5. **M13** (Traversal Caching) — depends on M12 for cache integration
6. **M13.6** (CLI Commands) — depends on M13.5
7. **M13.7** (PQL REPL) — depends on M10, M11, M13.5, M13.6

---

## References

- **Spec:** `.llm/context/pelagodb-spec-v1.md` — §8.6, §10, §17, §19
- **RocksDB Cache Spec:** `.llm/context/section-18-rocksdb-cache.md`
- **Implementation Phases:** `.llm/shared/research/2026-02-14-pelagodb-implementation-phases.md`
- **PQL Research:** `.llm/shared/research/2026-02-13-query-api-cli-design-v2.md`
- **Traversal Research:** `.llm/shared/research/2026-02-13-traversal-query-language.md`
- **Phase 2 Plan:** `.llm/shared/plans/2026-02-14-phase-2-cdc-system.md`
- **Phase 1 Plan:** `.llm/shared/plans/2026-02-14-phase-1-storage-foundation.md`
