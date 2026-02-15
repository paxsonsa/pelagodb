---
date: 2026-02-13T14:00:00-08:00
researcher: Claude
git_commit: HEAD (initial commit pending)
branch: main
repository: pelagodb
topic: "Pelago Query Language (PQL) — Traversal Query Language Design"
tags: [research, query-language, traversal, dql-inspired, cel-integration, cross-namespace]
status: draft
last_updated: 2026-02-13
last_updated_by: Claude
last_updated_note: "Added cross-namespace traversal syntax (Section 2.5) with namespace-qualified edge types and target type constraints"
---

# Research: Pelago Query Language (PQL) — Traversal Query Language Design

**Date**: 2026-02-13T14:00:00-08:00
**Researcher**: Claude
**Inspiration**: Dgraph DQL (nested blocks, variables, directives)
**Foundation**: PelagoDB CEL pipeline, existing TraverseRequest proto

## Research Question

Design a declarative traversal query language for PelagoDB that:

1. Borrows DQL's nesting, variable capture, and directive model
2. Preserves CEL as the predicate language (type-checked, cost-based planning)
3. Compiles to existing `TraverseRequest`/`TraversalStep` proto messages
4. Supports the REPL (`pelago sql`) and gRPC text-field transport
5. Handles polymorphic edges, bidirectional traversal, and vertex-centric indexes

---

## Summary

PQL is a declarative graph query language designed for PelagoDB. It combines DQL-style nested block traversal with CEL predicate expressions. Every PQL query compiles to either a `FindNodesRequest` or a `TraverseRequest` protobuf message — PQL is syntax sugar over the existing gRPC API, not a separate execution engine.

Key design choices:

1. **CEL stays** — all predicates are CEL expressions, type-checked against schema
2. **Nested blocks from DQL** — multi-hop traversals expressed as nested curly-brace blocks
3. **Variables from DQL** — capture node sets and scalar aggregates for multi-block queries
4. **Directives from DQL** — `@filter`, `@cascade`, `@limit`, `@sort` modify traversal behavior
5. **No new execution engine** — PQL parses to an AST that compiles to the existing proto API

---

## 1. Language Grammar

### 1.1 Query Structure

```
query [query_name] {
  root_block(func: root_function) [@directive ...] {
    field_projection
    edge_traversal [@directive ...] {
      nested_projection
      deeper_edge [@directive ...] { ... }
    }
  }
}
```

A PQL query is one or more **named blocks**. Each block starts with a **root function** that selects starting nodes, then nests **edge traversals** to arbitrary depth.

### 1.2 Formal Grammar (EBNF)

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

**Namespace qualification examples:**

| Syntax | Meaning |
|--------|---------|
| `-[KNOWS]->` | Edge type KNOWS in default namespace |
| `-[job_history:HAD]->` | Edge type HAD in job_history namespace |
| `-[KNOWS]-> Person` | Target type Person in default namespace |
| `-[job_history:HAD]-> job_history:Job` | Edge HAD in job_history, target Job in job_history |
| `uid(Person:p_42)` | Node Person:p_42 in default namespace |
| `uid(default:Person:p_42)` | Node Person:p_42 explicitly in default namespace |

### 1.3 Root Functions

Root functions select the starting node set. Only ONE root function per block (matching DQL's constraint — keeps query planning deterministic).

| Function | Meaning | Compiles To |
|---|---|---|
| `uid(Person:p_42)` | Single node by type:id | `NodeRef { entity_type: "Person", node_id: "p_42" }` |
| `uid(var_name)` | Nodes captured by a variable | Variable reference (see Section 3) |
| `eq(field, value)` | Equality on indexed field | CEL `field == value` → index point lookup |
| `ge(field, value)` | Greater-than-or-equal | CEL `field >= value` → index range scan |
| `le(field, value)` | Less-than-or-equal | CEL `field <= value` → index range scan |
| `between(field, lo, hi)` | Range | CEL `field >= lo && field <= hi` |
| `has(field)` | Field exists (not null) | CEL `has(field)` or `field != null` |
| `type(EntityType)` | All nodes of type | `FindNodesRequest { entity_type, cel_expression: "" }` |
| `allofterms(field, "a b")` | All terms present | CEL `field.contains("a") && field.contains("b")` |

Root functions compile to CEL expressions + `FindNodesRequest` on the server. They're syntactic convenience — the planner sees CEL.

---

## 2. Traversal Blocks

### 2.1 Edge Traversal Syntax

```
-[EDGE_TYPE]->     outgoing traversal
-[EDGE_TYPE]<-     incoming traversal
-[EDGE_TYPE]<->    bidirectional traversal
```

Each edge block specifies the edge type and direction, then nests the projection/filtering for target nodes.

### 2.2 Example: Two-Hop Traversal

**PQL:**
```
query friends_companies {
  start(func: uid(Person:p_42)) {
    name
    age
    -[KNOWS]-> @filter(strength > 0.8) {
      name
      age
      -[WORKS_AT]-> @filter(started > timestamp("2020-01-01")) {
        name
        industry
      }
    }
  }
}
```

**Compiles to TraverseRequest:**
```protobuf
TraverseRequest {
  context: { datastore: "prod", namespace: "default" },
  start: { entity_type: "Person", node_id: "p_42" },
  steps: [
    TraversalStep {
      edge_namespace: "",           // Empty = use request context namespace
      edge_type: "KNOWS",
      direction: EDGE_DIRECTION_OUT,
      target_namespace: "",         // Empty = infer from edge schema
      target_type: "",              // Empty = infer from edge schema
      edge_filter: "strength > 0.8",
      node_filter: "",
      fields: ["name", "age"]
    },
    TraversalStep {
      edge_namespace: "",
      edge_type: "WORKS_AT",
      direction: EDGE_DIRECTION_OUT,
      target_namespace: "",
      target_type: "",
      edge_filter: "started > timestamp('2020-01-01')",
      node_filter: "",
      fields: ["name", "industry"]
    }
  ],
  max_depth: 2,
  timeout_ms: 5000,
  max_results: 10000
}
```

### 2.3 Distinguishing Edge Filters vs Node Filters

DQL uses `@filter` for both edge and node predicates — which creates ambiguity. PQL separates them explicitly:

```
-[EDGE_TYPE]-> @edge(cel_on_edge_props) @filter(cel_on_target_node_props) {
  ...
}
```

- **`@edge(...)`** — CEL expression evaluated against edge properties (role, started, strength, etc.)
- **`@filter(...)`** — CEL expression evaluated against target node properties (name, age, etc.)

Both compile to `TraversalStep.edge_filter` and `TraversalStep.node_filter` respectively.

**Example:**
```
-[WORKS_AT]-> @edge(role == "Engineer") @filter(name.startsWith("Tech")) {
  name
  founded
}
```

Compiles to:
```protobuf
TraversalStep {
  edge_type: "WORKS_AT",
  direction: OUT,
  edge_filter: "role == 'Engineer'",
  node_filter: "name.startsWith('Tech')",
  fields: ["name", "founded"]
}
```

### 2.4 Vertex-Centric Index Optimization

When `@edge(...)` references a sort_key property, the PQL compiler emits a range scan hint:

```
-[WORKS_AT]-> @edge(started >= timestamp("2020-01-01")) {
  ...
}
```

The query planner recognizes `started` is the sort_key for WORKS_AT edges and compiles to an FDB range scan on the edge subspace rather than a full adjacency list scan + residual.

This is transparent to the user — the planner makes the decision based on edge schema metadata.

### 2.5 Cross-Namespace Traversal

Edges may be defined in a different namespace than their source or target nodes. PQL supports **namespace-qualified edge types** and **explicit target type constraints** for cross-namespace traversal.

#### 2.5.1 Namespace-Qualified Edge Types

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

#### 2.5.2 Explicit Target Type Constraints

For polymorphic edges or explicit type safety, specify the target type after the direction arrow:

```
-[namespace:EDGE_TYPE]-> namespace:TargetType { ... }
```

**Example:** Cross-namespace with explicit target:

```
query {
  start(func: uid(default:Person:p_42)) {
    name
    -[job_history:HAD]-> job_history:Job {
      title
      company
    }
  }
}
```

**Example:** Polymorphic edge constrained to specific types:

If `TAGGED_WITH` is declared as `target: "*"` (any type):

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

#### 2.5.3 Namespace Resolution Rules

| Syntax | Edge Namespace | Target Namespace |
|--------|----------------|------------------|
| `-[EDGE]->` | Query context default | Inferred from edge schema |
| `-[ns:EDGE]->` | `ns` | Inferred from edge schema |
| `-[EDGE]-> Type` | Query context default | Query context default |
| `-[ns:EDGE]-> Type` | `ns` | Query context default |
| `-[ns:EDGE]-> ns2:Type` | `ns` | `ns2` |

**Same-namespace shorthand:** When target is in the same namespace as the query context, the namespace prefix can be omitted:

```
query @context(namespace: "default") {
  start(func: type(Person)) {
    name
    -[KNOWS]-> Person {           # Implicitly default:Person
      name
    }
    -[job_history:HAD]-> job_history:Job {  # Cross-namespace, explicit
      title
    }
  }
}
```

#### 2.5.4 Compilation to Proto

```
-[job_history:HAD]-> job_history:Job @edge(role == "Lead") { title }
```

Compiles to:

```protobuf
TraversalStep {
  edge_namespace: "job_history",
  edge_type: "HAD",
  direction: EDGE_DIRECTION_OUT,
  target_namespace: "job_history",
  target_type: "Job",
  edge_filter: "role == 'Lead'",
  fields: ["title"]
}
```

#### 2.5.5 Validation Rules

1. **If target type specified**: Server validates it matches edge schema declaration (or is a valid polymorphic target)
2. **If target type omitted**: Server infers from edge schema; error if edge is polymorphic (`target: "*"`) and ambiguous
3. **Cross-namespace edges**: Target namespace is required if different from query context
4. **Edge namespace resolution**: If edge type exists in multiple namespaces, explicit qualification is required

---

## 3. Variables

### 3.1 UID Variables (Node Set Capture)

Capture the node set produced by a block and reference it in a later block.

**Syntax:**
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

`friends as` captures all nodes reached by the KNOWS traversal. The second block `mutual` starts from those captured nodes.

**Compilation strategy:**

Variables are a client-side or server-side intermediate result. Two execution options:

1. **Server-side pipeline** (preferred): First block streams results; server captures UIDs in a temporary set; second block reads from that set. Requires server support for multi-block queries.

2. **Client-side round-trip**: First block returns results to client; client extracts UIDs; client sends second query with `uid(id1, id2, ...)`. Works with existing single-TraverseRequest API.

**Proto extension needed for server-side:**
```protobuf
message PqlQuery {
  repeated QueryBlock blocks = 1;
  uint32 timeout_ms = 2;
  uint32 max_results = 3;
  ReadConsistency consistency = 4;
}

message QueryBlock {
  string name = 1;
  oneof root {
    NodeRef start_node = 2;
    FindNodesRequest find = 3;
    string variable_ref = 4;          // References a previous block's output
  }
  repeated TraversalStep steps = 5;
  repeated string capture_at_depth = 6;  // Which depths to capture as variable
}
```

### 3.2 Value Variables (Scalar Capture)

Capture scalar aggregations for use in filters or projections.

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

`friend_count as count(...)` computes the count of KNOWS edges per person. The second block filters to persons with 10+ friends and projects the count.

**Implementation:**

Value variables require a map of `node_id → scalar_value` computed during the first block. The second block's `ge(val(friend_count), 10)` becomes a lookup into that map.

This is expensive for large node sets. Server-side implementation uses an in-memory HashMap; client-side uses returned results.

### 3.3 Variable Scoping Rules

| Rule | Behavior |
|---|---|
| Variables are query-scoped | Visible to all blocks within the same `query { }` |
| Block execution order | Blocks execute top-to-bottom; a block can only reference variables from earlier blocks |
| UID variables capture leaf nodes | The variable captures nodes at the innermost traversal depth of the capturing block |
| Value variables are per-node | Each node gets its own computed value in the variable map |
| Variable naming | Must be unique within a query; alphanumeric + underscores |

---

## 4. Directives

### 4.1 @filter — Node Predicate

Filters target nodes using CEL expression. Applied after edge traversal, before result emission.

```
@filter(age >= 30 && department == "rendering")
```

Compiles to `TraversalStep.node_filter` or `FindNodesRequest.cel_expression`.

### 4.2 @edge — Edge Predicate

Filters edges during traversal using CEL expression against edge properties.

```
@edge(role == "Lead" && started > timestamp("2023-01-01"))
```

Compiles to `TraversalStep.edge_filter`.

### 4.3 @cascade

**Borrowed from DQL.** Removes nodes that don't have all requested sub-traversals.

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

Without `@cascade`: returns all Person nodes, with empty nested blocks for those without KNOWS or WORKS_AT edges.

With `@cascade`: only returns Person nodes that have at least one KNOWS edge to a Person who has at least one WORKS_AT edge. Pattern matching.

**Implementation:**

Cascade is a post-filter on the result tree. After traversal completes at all depths, prune any root node whose subtree is incomplete. This can be done:

- **Server-side**: Buffer results per root node; emit only if all nested blocks produced at least one result.
- **Client-side**: Post-process the streamed result set.

**Proto extension:**
```protobuf
message TraverseRequest {
  // ... existing fields ...
  bool cascade = 8;  // If true, only emit paths where all hops matched
}
```

### 4.4 @limit and @offset — Pagination

```
people(func: type(Person)) @limit(first: 20, offset: 40) {
  name
}
```

Compiles to `FindNodesRequest.limit = 20` with cursor-based pagination. The `offset` is sugar for cursor advancement (the actual implementation skips N results).

Per-hop limits:
```
-[KNOWS]-> @limit(first: 5) {
  name
}
```

Limits the fan-out at each hop — return at most 5 KNOWS edges per source node. This is critical for controlling traversal explosion.

**Proto extension:**
```protobuf
message TraversalStep {
  // ... existing fields ...
  uint32 per_node_limit = 6;  // Max edges to traverse per source node at this hop
}
```

### 4.5 @sort — Ordering

```
-[WORKS_AT]-> @sort(started: desc) {
  name
}
```

Sort results by a field at this traversal depth. If the sort field matches the edge's sort_key, this is free (FDB returns edges in sort_key order). Otherwise, server-side sort on the result set.

**Sort on edge properties:**
```
-[WORKS_AT]-> @sort(edge.started: desc) {
  name
}
```

The `edge.` prefix references the edge property rather than target node property.

### 4.6 @facets — Edge Property Projection

**Borrowed from DQL.** Projects edge properties alongside target node data.

```
-[WORKS_AT]-> @facets(role, started) {
  name
}
```

Returns:
```json
{
  "name": "ILM",
  "WORKS_AT|role": "Engineer",
  "WORKS_AT|started": "2020-01-15T00:00:00Z"
}
```

**Implementation:**

Edge properties are already available in `EdgeData.properties`. The `@facets` directive tells the result serializer to include them in the output alongside node properties, namespaced by edge type.

**Proto extension:**
```protobuf
message TraversalStep {
  // ... existing fields ...
  repeated string edge_fields = 7;  // Edge properties to project in results
}
```

### 4.7 @recurse — Recursive Traversal

For variable-depth traversal of the same edge type (social graphs, org hierarchies).

```
query {
  reports(func: uid(Person:p_ceo)) @recurse(depth: 5) {
    name
    -[MANAGES]->
  }
}
```

Equivalent to 5 hops of `-[MANAGES]->` with the same projection at each level. Cycle detection is mandatory (visited-node tracking).

**Implementation considerations:**

- Server tracks visited UIDs in a `HashSet<(String, String)>` (entity_type, node_id)
- If a node is already visited, skip it (break cycles)
- `depth` is mandatory — no unbounded recursion
- Results are streamed depth-first

**Proto extension:**
```protobuf
message TraverseRequest {
  // ... existing fields ...
  RecurseConfig recurse = 9;
}

message RecurseConfig {
  uint32 max_depth = 1;
  bool detect_cycles = 2;  // default: true
}
```

---

## 5. Multi-Block Queries (Joins)

### 5.1 Parallel Independent Blocks

Multiple root blocks in one query execute in parallel (like DQL):

```
query {
  alice(func: eq(name, "Alice")) {
    name
    age
    -[WORKS_AT]-> { name }
  }

  bob(func: eq(name, "Bob")) {
    name
    age
    -[WORKS_AT]-> { name }
  }
}
```

Both blocks execute concurrently. Results are keyed by block name. No dependency between them.

### 5.2 Variable-Linked Blocks (Sequential)

When a block references a variable from a prior block, execution is sequential:

```
query {
  alice_friends as start(func: uid(Person:p_alice)) {
    -[KNOWS]-> {
      uid  # capture these node IDs
    }
  }

  # This block depends on alice_friends
  shared(func: uid(alice_friends)) {
    -[KNOWS]-> @filter(name == "Bob") {
      name
    }
  }
}
```

Block `shared` waits for `alice_friends` to complete. The planner builds a DAG of block dependencies and executes in topological order, parallelizing independent branches.

### 5.3 Intersection and Difference

Variable sets support set operations:

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
    age
  }

  # Difference: alice's friends who aren't bob's friends
  alice_only(func: uid(alice_friends, bob_friends, difference)) {
    name
  }
}
```

Set operations:

| Operation | Syntax | Meaning |
|---|---|---|
| Union | `uid(a, b)` | Nodes in a OR b |
| Intersection | `uid(a, b, intersect)` | Nodes in a AND b |
| Difference | `uid(a, b, difference)` | Nodes in a but NOT b |

---

## 6. Mutations in PQL

### 6.1 Upsert Blocks

**Borrowed from DQL.** Atomic query-then-mutate in a single request.

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

**Compilation:**

The upsert block compiles to a server-side transaction:

1. Execute the `query` block within an FDB transaction snapshot
2. Evaluate `@if` conditions against query results
3. Execute matching mutation(s) within the same transaction
4. Commit atomically

**Proto:**
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

### 6.2 Conditional Edge Creation

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

Atomically: find Alice, find ILM, check if edge exists, create if not. Single FDB transaction.

---

## 7. Aggregations

### 7.1 Inline Aggregations

```
query {
  companies(func: type(Company)) {
    name
    employee_count: count(-[WORKS_AT]<-)
    avg_tenure: avg(-[WORKS_AT]<- @facets(started) { started })
  }
}
```

Aggregation functions compute over the edge set at each node:

| Function | Input | Output |
|---|---|---|
| `count(edge_block)` | Number of edges matching block | int |
| `sum(edge_block.field)` | Sum of a numeric field on targets or edges | float |
| `avg(edge_block.field)` | Average | float |
| `min(edge_block.field)` | Minimum | same type |
| `max(edge_block.field)` | Maximum | same type |

### 7.2 @groupby

```
query {
  by_department(func: type(Person)) @groupby(department) {
    count(uid)
    avg(age)
  }
}
```

Groups results by the specified field and applies aggregation functions within each group.

**Compiles to:** Full scan of Person type → group by department → compute aggregates per group. This is a server-side operation that requires materializing all matching nodes.

**Performance consideration:** @groupby is expensive. It should always use an indexed field for grouping to avoid full scans. The planner should warn (via EXPLAIN) if the groupby field isn't indexed.

---

## 8. Compilation Pipeline

### 8.1 PQL → Proto Compilation Stages

```
PQL text
  │
  ▼
┌──────────────────┐
│ 1. Parse          │  pest/nom parser → PQL AST
│    (syntax check) │
└───────┬──────────┘
        │
        ▼
┌──────────────────┐
│ 2. Resolve        │  Resolve entity types, edge types, field names
│    (schema check) │  Validate edge declarations exist
│                   │  Resolve variable references (check DAG)
└───────┬──────────┘
        │
        ▼
┌──────────────────┐
│ 3. CEL Compile    │  Extract all @filter/@edge directives
│    (type check)   │  Compile each CEL expression against schema
│                   │  Type-check field references
└───────┬──────────┘
        │
        ▼
┌──────────────────┐
│ 4. Plan           │  Build execution DAG (block dependencies)
│    (optimize)     │  Merge sequential hops into TraversalSteps
│                   │  Select root function → index strategy
│                   │  Apply @limit hints for fan-out control
│                   │  Emit EXPLAIN data if requested
└───────┬──────────┘
        │
        ▼
┌──────────────────┐
│ 5. Emit Proto     │  Generate FindNodesRequest / TraverseRequest
│    (codegen)      │  Wire up variable references as block deps
│                   │  Set consistency, timeout, max_results
└──────────────────┘
```

### 8.2 Parser Implementation

Recommended: `pest` crate for PEG parser. PQL grammar is simple enough for a PEG and pest generates the parser at compile time.

```rust
// pelago-query/src/pql/parser.rs

use pest::Parser;
use pest_derive::Parser;

#[derive(Parser)]
#[grammar = "pql/grammar.pest"]
pub struct PqlParser;

pub fn parse_query(input: &str) -> Result<PqlAst, PqlError> {
    let pairs = PqlParser::parse(Rule::query, input)?;
    build_ast(pairs)
}
```

### 8.3 AST Types

```rust
// pelago-query/src/pql/ast.rs

pub struct PqlQuery {
    pub name: Option<String>,
    pub default_namespace: Option<String>,  // From @context(namespace: "...")
    pub blocks: Vec<QueryBlock>,
}

pub struct QueryBlock {
    pub name: String,
    pub root: RootFunction,
    pub directives: Vec<Directive>,
    pub selections: Vec<Selection>,
    pub capture_as: Option<String>,  // Variable capture
}

/// Qualified reference with optional namespace
pub struct QualifiedRef {
    pub namespace: Option<String>,   // None = use query default
    pub entity_type: String,
    pub node_id: String,
}

pub enum RootFunction {
    Uid(QualifiedRef),               // uid(ns:Type:id) or uid(Type:id)
    UidVar(String),                  // Reference to captured variable
    UidSet(Vec<String>, SetOp),      // Set operation on variables
    Eq(String, Value),
    Ge(String, Value),
    Le(String, Value),
    Between(String, Value, Value),
    Has(String),
    Type(QualifiedType),             // type(ns:Type) or type(Type)
    AllOfTerms(String, String),
}

pub struct QualifiedType {
    pub namespace: Option<String>,
    pub entity_type: String,
}

pub enum SetOp {
    Union,
    Intersect,
    Difference,
}

pub enum Selection {
    Field(String),                   // Property projection
    Edge(EdgeTraversal),             // Nested edge block
    Aggregate(AggregateExpr),        // count, sum, etc.
    ValueVar(String, AggregateExpr), // value variable capture
}

pub struct EdgeTraversal {
    pub edge_namespace: Option<String>,   // None = use query default
    pub edge_type: String,
    pub direction: EdgeDirection,
    pub target_type: Option<TargetSpec>,  // Explicit target type constraint
    pub directives: Vec<Directive>,
    pub selections: Vec<Selection>,
    pub capture_as: Option<String>,
}

/// Target type specification with optional namespace
pub struct TargetSpec {
    pub namespace: Option<String>,   // None = use query default
    pub entity_type: String,
}

pub enum Directive {
    Filter(String),            // CEL on target node props
    Edge(String),              // CEL on edge props
    Cascade,
    Limit { first: u32, offset: Option<u32> },
    Sort { field: String, desc: bool, on_edge: bool },
    Facets(Vec<String>),       // Edge property projection
    Recurse { depth: u32 },
    GroupBy(Vec<String>),
}

pub enum AggregateExpr {
    Count(Box<Selection>),
    Sum(String),
    Avg(String),
    Min(String),
    Max(String),
}
```

### 8.4 Crate Location

```
pelago-query/src/
  ├─ cel.rs            ← existing: CEL parse, type-check
  ├─ planner.rs        ← existing: predicate extraction, index selection
  ├─ executor.rs       ← existing: FindNodes execution
  ├─ plan.rs           ← existing: QueryPlan enum
  └─ pql/
      ├─ mod.rs
      ├─ grammar.pest  ← PEG grammar for PQL
      ├─ parser.rs     ← pest parser → PqlAst
      ├─ resolver.rs   ← schema resolution, variable DAG
      ├─ compiler.rs   ← PqlAst → proto messages
      └─ explain.rs    ← EXPLAIN output for PQL queries
```

---

## 9. REPL Integration

### 9.1 PQL in the REPL

PQL replaces the current ad-hoc `traverse` and `find` REPL commands with a unified language:

```
pelago> query { people(func: eq(name, "Andrew")) { name, age } }
┌────────┬──────────┬─────┐
│ block  │ name     │ age │
├────────┼──────────┼─────┤
│ people │ Andrew   │ 38  │
└────────┴──────────┴─────┘
1 result (2.1ms)
```

### 9.2 Short-Form Syntax

For simple queries, PQL supports a shortened single-block form (no `query { }` wrapper):

```
pelago> Person @filter(age >= 30) { name, age }
```

Equivalent to:
```
query { result(func: type(Person)) @filter(age >= 30) { name, age } }
```

### 9.3 Explain Mode

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

---

## 10. Wire Transport

### 10.1 PQL over gRPC

PQL queries can be sent as text in a new `PqlQuery` RPC:

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

### 10.2 Parameterized Queries

For safety and performance (compiled query caching):

```
pelago> :param $min_age = 30
pelago> Person @filter(age >= $min_age) { name, age }
```

gRPC:
```protobuf
PqlRequest {
  pql: "query { result(func: type(Person)) @filter(age >= $min_age) { name, age } }",
  variables: { "min_age": Value { int_value: 30 } }
}
```

Parameterized queries enable server-side query plan caching — same PQL text with different parameters reuses the compiled plan.

---

## 11. Comparison: PQL vs DQL

| Feature | DQL | PQL | Rationale |
|---|---|---|---|
| Predicate language | Custom functions | CEL | Type-checked, cost-based planner, portable |
| Edge vs node filter | Both in `@filter` | Separate `@edge` + `@filter` | Explicit; maps to proto fields cleanly |
| Root function | `func:` with index functions | Same (`func:` with typed functions) | Direct inspiration |
| Variables | `varName as` | Same | Direct inspiration |
| @cascade | Yes | Yes | Direct inspiration |
| @facets | Yes (edge properties inline) | Yes (with `@facets` directive) | Direct inspiration |
| @recurse | Yes (depth-limited) | Yes (with cycle detection) | Added cycle detection |
| @groupby | Yes | Yes | Direct inspiration |
| Upsert blocks | Yes (query + conditional mutation) | Yes | Direct inspiration, single FDB txn |
| Set operations on variables | No (manual in client) | Yes (intersect, difference) | Useful for mutual-friends pattern |
| Sort | `orderasc` / `orderdesc` | `@sort(field: asc/desc)` | Cleaner directive syntax |
| Per-hop fan-out limit | No built-in | `@limit(first: N)` on edge blocks | Critical for traversal explosion control |
| GraphQL endpoint | Yes (translates to DQL) | Future consideration | PQL is the native language |

---

## 12. Implementation Phases

### Phase 2a: PQL Parser + Single-Block Queries
- [ ] PEG grammar for PQL (pest crate)
- [ ] Parser → PqlAst
- [ ] Schema resolver (validate types, fields, edges)
- [ ] CEL extraction from directives
- [ ] Compiler: single-block PQL → FindNodesRequest or TraverseRequest
- [ ] REPL integration (replace ad-hoc `find` / `traverse` commands)
- [ ] `@filter`, `@edge`, `@limit`, `@sort` directives
- [ ] Short-form syntax in REPL
- [ ] ExplainPql endpoint

### Phase 2b: Variables + Multi-Block
- [ ] Variable capture (UID variables)
- [ ] Block dependency DAG + execution ordering
- [ ] `uid(var)` root function
- [ ] Value variables with aggregation
- [ ] Set operations (union, intersect, difference)
- [ ] PqlQuery proto + ExecutePql RPC
- [ ] Parameterized queries + plan caching

### Phase 2c: Advanced Directives
- [ ] `@cascade` (subtree completeness filter)
- [ ] `@facets` (edge property projection)
- [ ] `@recurse` (variable-depth traversal with cycle detection)
- [ ] `@groupby` with aggregation functions
- [ ] `per_node_limit` on TraversalStep

### Phase 3: Upserts
- [ ] Upsert block parsing
- [ ] Conditional mutation evaluation
- [ ] UpsertRequest proto
- [ ] Atomic query-then-mutate in single FDB transaction

---

## 13. Open Questions

1. **Aggregation push-down**: Should aggregations (count, sum) be computed during traversal (streaming) or after materialization? Streaming is more efficient but complex.

2. **Cycle detection strategy**: HashSet of visited UIDs works for single-query scope. For @recurse across large graphs, memory could be an issue. Bloom filter alternative?

3. **Query plan caching**: Cache compiled PQL → proto mapping keyed by PQL text hash? Invalidate on schema change?

4. **Multi-site variable resolution**: If a variable-linked block spans sites (variable captured at site A, used at site B), how do we handle latency? Force all blocks to execute at the coordinator site?

5. **Streaming @cascade**: Can we detect incomplete subtrees during streaming (before all results arrive), or must we buffer the entire result set? DQL buffers — can we do better?

6. **Cost model for traversals**: The existing CEL cost model handles single-hop index selection. Multi-hop traversal cost estimation is harder — need to estimate fan-out at each hop based on edge degree statistics.

7. **PQL over WebSocket**: For long-running traversals, should we support WebSocket transport in addition to gRPC streaming? Or is gRPC streaming sufficient?

---

## Code References

- `llm/context/graph-db-spec-v0.3.md` — Main architecture spec (CEL pipeline Section 5)
- `llm/context/graph-db-edge-spec.md` — Edge storage, traversal semantics
- `.llm/shared/research/2026-02-13-query-api-cli-design.md` — Existing gRPC API and CLI design
- `llm/context/research-c1-index-intersection.md` — Query planning and cost estimation
- `llm/context/phase-1-plan.md` — Crate layout and implementation roadmap

## External References

- [DQL Query Syntax](https://dgraph.io/docs/dql/dql-syntax/dql-query/) — Root functions, nested blocks
- [DQL Variables](https://dgraph.io/docs/query-language/query-variables/) — UID and value variables
- [DQL Aggregation](https://dgraph.io/docs/query-language/aggregation/) — @groupby, count, sum
- [DQL Conditional Upsert](https://dgraph.io/docs/v21.03/mutations/conditional-upsert/) — Upsert blocks
- [GraphQL vs DQL](https://dgraph.io/blog/post/graphql-vs-dql/) — Why DQL diverged from GraphQL

---

## Related Documents

- `llm/context/graph-db-spec-v0.3.md` — Full architecture specification
- `llm/context/graph-db-edge-spec.md` — Edge management specification
- `.llm/shared/research/2026-02-13-query-api-cli-design.md` — Query API and CLI design
- `llm/context/research-c1-index-intersection.md` — Index intersection strategy
- `llm/context/pelago-fdb-keyspace-spec.md` — FDB keyspace layout
