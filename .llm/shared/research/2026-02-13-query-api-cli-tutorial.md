# PelagoDB Query API & CLI — The 10-Minute Walkthrough

Everything you need to understand the query system without reading the full research doc.

---

## The Mental Model

Three layers, top to bottom:

```
CLI (pelago)  →  gRPC API  →  Core Engine (FDB)
```

Every CLI command maps to a gRPC call. Every gRPC call compiles to FDB operations. There's no magic — the CLI is sugar over proto, proto is sugar over FDB key scans.

The query language is **CEL** (Common Expression Language). Not SQL, not Cypher, not Gremlin. CEL is a type-checked expression language that decomposes into index operations. You write `age >= 30 && name.startsWith("A")`, the planner splits it into an index range scan + in-memory residual filter.

---

## CLI Structure

Noun-verb ordering (inspired by EdgeDB). Every command follows `pelago <noun> <verb>`.

```bash
# Schema
pelago schema register person.json       # Register entity type from file
pelago schema get Person                  # Show schema
pelago schema list                        # All schemas in namespace

# Nodes
pelago node create Person --props '{"name":"Andrew","age":38}'
pelago node get Person p_42
pelago node update Person p_42 --props '{"age":39}'
pelago node delete Person p_42

# Edges
pelago edge create Person:p_42 WORKS_AT Company:c_7 --props '{"role":"Lead"}'
pelago edge list Person p_42 --dir out
pelago edge delete e_101

# Queries
pelago query find Person 'age >= 30'
pelago query explain Person 'age >= 30 && department == "rendering"'
pelago query traverse Person:p_42 '-[KNOWS]-> -[WORKS_AT]-> Company'

# Indexes
pelago index create Person age range
pelago index list Person
pelago index drop Person age

# Admin
pelago admin job status job_abc123
pelago admin drop-type Person            # Nuclear option — clearRange
```

### Global flags that go everywhere:

```bash
-d, --datastore starwars     # Which datastore
-n, --namespace graphics     # Which namespace
-s, --site-id SF             # Site for writes
--format json|table|csv      # Output format
```

---

## The REPL

`pelago sql` drops you into an interactive session:

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

pelago> :use graphics        # switch namespace
pelago> :format json         # switch output
pelago> :history             # see what you ran
pelago> :quit
```

The `find` and `explain` commands are the same CEL under the hood. `:commands` are REPL meta-commands (inspired by Neo4j's cypher-shell).

---

## gRPC Services

Six services, phased rollout:

| Service | What it does | Phase |
|---|---|---|
| **SchemaService** | Register/get/list entity schemas | 1 |
| **NodeService** | CRUD nodes + batch create | 1 |
| **EdgeService** | CRUD edges + list with CEL filters | 1 |
| **QueryService** | FindNodes, Explain, Traverse | 1-2 |
| **AdminService** | Indexes, jobs, drop type/namespace | 1 |
| **WatchService** | Reactive subscriptions | 3 |

### The request envelope

Every RPC takes a `RequestContext`:

```protobuf
message RequestContext {
  string datastore = 1;     // logical database
  string namespace = 2;     // partition within datastore
  string site_id = 3;       // which site is making the write
  string request_id = 4;    // for distributed tracing
}
```

### Consistency levels

Every read can specify how fresh you need the data:

| Level | What happens | Latency |
|---|---|---|
| `STRONG` | Direct FDB read | Higher (network to FDB) |
| `SESSION` (default) | RocksDB cache + freshness check | Low |
| `EVENTUAL` | RocksDB only, no freshness check | Lowest |

---

## Key RPCs by Example

### Schema Registration

```protobuf
RegisterSchemaRequest {
  context: { datastore: "vfx", namespace: "shots" },
  entity_type: "Person",
  properties: [
    { name: "name", type: "string", required: true, index: "equality" },
    { name: "age", type: "int", index: "range" },
    { name: "email", type: "string", index: "unique" }
  ],
  edges: [
    { name: "WORKS_AT", target: "Company", direction: "outgoing",
      properties: [
        { name: "role", type: "string", index: "equality" },
        { name: "started", type: "timestamp" }
      ],
      sort_key: "started"   // vertex-centric index
    },
    { name: "KNOWS", target: "Person", direction: "bidirectional" }
  ]
}
```

Response tells you the schema version and whether it triggered an index backfill job:

```protobuf
RegisterSchemaResponse {
  version: 1,
  created: true,
  job_id: ""    // empty = no backfill needed (new type)
}
```

### FindNodes (streaming)

The core query RPC. CEL expression goes in, streaming results come out.

```protobuf
FindNodesRequest {
  context: { datastore: "vfx", namespace: "shots" },
  entity_type: "Person",
  cel_expression: "age >= 30 && name.startsWith('A')",
  consistency: SESSION,
  fields: ["name", "age"],   // projection — omit for all fields
  limit: 100,
  cursor: bytes("")           // empty = start from beginning
}
```

Server streams back `NodeResult` messages:

```protobuf
NodeResult { entity_type: "Person", node_id: "p_42",
             properties: { "name": "Andrew", "age": 38 } }
NodeResult { entity_type: "Person", node_id: "p_108",
             properties: { "name": "Alicia", "age": 45 },
             next_cursor: bytes("...") }  // last message carries cursor
```

Empty `next_cursor` = no more results. Non-empty = pass it back to get the next page.

### Explain (query plan inspection)

Same input as FindNodes, but returns the execution plan instead of data:

```protobuf
ExplainRequest {
  entity_type: "Person",
  cel_expression: "age >= 30 && department == 'rendering'"
}

ExplainResponse {
  plan: {
    strategy: "index_scan",
    index_ops: [
      { field: "department", operator: "==", value: "rendering",
        estimated_rows: 200 }
    ],
    residual_filter: "age >= 30"
  },
  steps: [
    "Scan equality index on department (== 'rendering'): ~200 rows",
    "Apply residual filter age >= 30 in memory"
  ],
  estimated_cost: 201,
  estimated_rows: 100
}
```

The planner picked `department` (200 rows) over `age` (8000 rows) because it's more selective. `age >= 30` becomes a residual filter evaluated in memory against the 200 candidates.

### Traverse (multi-hop, Phase 2)

Start at a node, walk edges with per-hop filtering:

```protobuf
TraverseRequest {
  context: { datastore: "vfx", namespace: "shots" },
  start: { entity_type: "Person", node_id: "p_42" },
  steps: [
    { edge_type: "KNOWS", direction: OUT,
      edge_filter: "strength > 0.8",
      node_filter: "age >= 30",
      fields: ["name", "age"] },
    { edge_type: "WORKS_AT", direction: OUT,
      edge_filter: "",
      node_filter: "name.startsWith('Tech')",
      fields: ["name", "industry"] }
  ],
  max_depth: 2,
  timeout_ms: 5000,
  max_results: 10000,
  consistency: SESSION
}
```

This says: "Start at Person p_42. Walk outgoing KNOWS edges where strength > 0.8 to people aged 30+. From each of those people, walk outgoing WORKS_AT edges to companies whose name starts with 'Tech'."

Results stream back with breadcrumb paths:

```protobuf
TraverseResult {
  depth: 1,
  path: [{ entity_type: "Person", node_id: "p_42" }],
  node: { entity_type: "Person", node_id: "p_99" },
  properties: { "name": "Jamie", "age": 34 },
  edge: { edge_id: "e_305" }
}

TraverseResult {
  depth: 2,
  path: [{ "Person", "p_42" }, { "Person", "p_99" }],
  node: { entity_type: "Company", node_id: "c_12" },
  properties: { "name": "TechCorp", "industry": "VFX" },
  edge: { edge_id: "e_410" }
}
```

### Edge Operations

Create an edge with properties:

```protobuf
CreateEdgeRequest {
  context: { datastore: "vfx", namespace: "shots", site_id: "SF" },
  source: { entity_type: "Person", node_id: "p_42" },
  target: { entity_type: "Company", node_id: "c_7" },
  edge_type: "WORKS_AT",
  properties: { "role": "Lead", "started": 1705363200 }
}

CreateEdgeResponse {
  edge_id: "e_101",
  pair_id: ""       // empty = directed edge (not bidirectional)
}
```

For bidirectional edges (like KNOWS), the response includes a `pair_id` that links the two directions.

List edges with CEL filtering:

```protobuf
ListEdgesRequest {
  node: { entity_type: "Person", node_id: "p_42" },
  edge_type: "WORKS_AT",
  direction: OUT,
  cel_filter: "role == 'Lead'",
  limit: 50
}
```

Streams back `EdgeResult` messages with properties, source/target refs, and pagination cursor.

---

## How the Query Planner Works

When you send a CEL expression, it goes through a 7-stage pipeline:

```
"age >= 30 && department == 'rendering'"
        │
        ▼
   1. Parse → CEL AST
   2. Type-check against Person schema (age: int, department: string)
   3. Normalize (flatten logic)
   4. Extract predicates: [age >= 30, department == "rendering"]
   5. Index match:
      - age: range index → RangeScan(age, >=, 30) → ~8000 rows
      - department: equality index → PointLookup(department, "rendering") → ~200 rows
   6. Plan selection:
      - Option A: Scan department (200), residual age → cost 201
      - Option B: Scan age (8000), residual department → cost 8001
      - Winner: Option A
   7. Output: ExecutionPlan {
        primary: PointLookup(department, "rendering"),
        residual: CompiledCel("age >= 30"),
        projection: ["name", "age", "department"],
        limit: 100
      }
```

Key insight: **the planner always picks the most selective index as the primary scan**, then evaluates remaining predicates as residual filters in memory. For predicates with similar selectivity (within 5x), it can do a merge-join intersection of two index scans instead.

---

## Pagination Model

All streaming RPCs use opaque cursor-based pagination (Google AIP-158):

```
Client                              Server
  │                                    │
  │─── FindNodes(cursor: empty) ──────>│
  │                                    │ scans FDB
  │<── NodeResult { ... } ────────────│
  │<── NodeResult { ... } ────────────│
  │<── NodeResult { next_cursor: X } ──│ last message
  │                                    │
  │─── FindNodes(cursor: X) ──────────>│ continues from X
  │<── NodeResult { ... } ────────────│
  │<── NodeResult { next_cursor: "" } ─│ empty = done
```

Cursors encode the last FDB key position + scan direction. They're opaque bytes — clients should treat them as tokens, not parse them.

---

## Error Codes

| Situation | gRPC Status | When |
|---|---|---|
| Node/edge/schema not found | `NOT_FOUND` | Get/delete on non-existent resource |
| Bad CEL, schema violation | `INVALID_ARGUMENT` | Malformed query or invalid data |
| Write to non-owned node | `PERMISSION_DENIED` | Multi-site: wrong site tried to write |
| Namespace/DB locked | `FAILED_PRECONDITION` | Write during maintenance lock |
| Query too slow | `DEADLINE_EXCEEDED` | Exceeded `timeout_ms` |
| Optimistic concurrency clash | `ABORTED` | `expected_version` didn't match |

---

## Implementation Phases

**Phase 1** (what gets built first):
- Proto definitions + tonic codegen
- CLI scaffolding with clap
- Schema/Node/Edge services (unary RPCs)
- FindNodes with single-predicate CEL
- Explain endpoint

**Phase 2** (query engine):
- Multi-predicate CEL decomposition + index intersection
- Streaming FindNodes with pagination
- ListEdges streaming
- Traverse (multi-hop)
- Interactive REPL (`pelago sql`)

**Phase 3** (advanced):
- WatchService (FDB native watches + CDC-driven query watches)
- Read coalescing for hot data
- RocksDB cache layer

---

## Quick Reference

```bash
# Create a schema
pelago schema register person.json

# Add data
pelago node create Person --props '{"name":"Andrew","age":38,"email":"andrew@ilm.com"}'
pelago edge create Person:p_42 WORKS_AT Company:c_7 --props '{"role":"Lead"}'

# Query
pelago query find Person 'age >= 30'
pelago query find Person 'email == "andrew@ilm.com"'

# See the plan
pelago query explain Person 'age >= 30 && department == "rendering"'

# Traverse
pelago query traverse Person:p_42 '-[KNOWS]-> -[WORKS_AT]-> Company'

# REPL
pelago sql
pelago> find Person where age >= 30
pelago> :use graphics
pelago> :quit
```
