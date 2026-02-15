---
date: 2026-02-13T16:30:00-08:00
researcher: Claude
git_commit: HEAD (initial commit pending)
branch: main
repository: pelagodb
topic: "V1 Spec Gap Analysis - Consolidation of All Design Documents"
tags: [research, spec-consolidation, phase-1, implementation-readiness, gap-analysis]
status: complete
last_updated: 2026-02-13
last_updated_by: Claude
---

# Research: V1 Spec Gap Analysis — What's Missing for Implementation

**Date**: 2026-02-13T16:30:00-08:00
**Researcher**: Claude
**Git Commit**: HEAD (initial commit pending)
**Branch**: main
**Repository**: pelagodb

## Research Question

Analyze all existing design documents and identify what's missing to create a comprehensive, implementation-ready V1 specification for PelagoDB Phase 1.

---

## Documents Analyzed

| Document | Purpose | Status |
|----------|---------|--------|
| `graph-db-spec-v0.3.md` | Core architecture, user stories, CEL pipeline | Complete |
| `pelago-fdb-keyspace-spec.md` | FDB key layout, VFX production focus | Complete |
| `graph-db-edge-spec.md` | Edge operations, bidirectional edges, ownership | Complete |
| `phase-1-plan.md` | Implementation milestones, crate layout | Complete |
| `2026-02-13-query-api-cli-design-v2.md` | PQL language, gRPC API, CLI design | Complete |
| `2026-02-13-traversal-query-language.md` | PQL grammar, directives, compilation | Draft |
| `research-phase1-outcome-clarity.md` | Cross-reference gap analysis | Complete |
| `research-undocumented-gaps.md` | Snapshot reads, auth, error model | Complete |
| `research-c1-index-intersection.md` | Query planning, selectivity | Complete |
| `research-i1-null-handling.md` | Null semantics in CEL | Complete |

---

## Summary: What V1 Spec Needs

A consolidated V1 spec must combine all documents and **resolve these critical gaps**:

### Already Resolved (Documented Across Files)

1. **Keyspace layout** — `pelago-fdb-keyspace-spec.md` ✓
2. **Edge storage model** — `graph-db-edge-spec.md` ✓
3. **PQL language** — `2026-02-13-query-api-cli-design-v2.md` ✓
4. **Null semantics** — `research-i1-null-handling.md` → incorporated into v0.3 ✓
5. **Index intersection** — `research-c1-index-intersection.md` → incorporated into v0.3 ✓
6. **Property removal** — incorporated into v0.3 Section 4.4 ✓
7. **Snapshot reads** — `research-undocumented-gaps.md` recommendation ✓
8. **Traversal limits** — spec v0.3 Section 9 ✓

### Gaps Requiring Resolution for V1 Spec

---

## Gap 1: Two Keyspace Specs with Conflicts

### Problem

`graph-db-spec-v0.3.md` (Section 6) and `pelago-fdb-keyspace-spec.md` define DIFFERENT key hierarchies:

**Spec v0.3:**
```
(namespace)
  (_meta)
    (schemas, {entity_type})
  (_cdc)
  (_jobs)
  (entities)
    ({entity_type})
      (data, {node_id})
      (idx)
      (edges)
```

**FDB Keyspace Spec:**
```
/pelago/
  /_sys/                 → System-wide config
  /_auth/                → Global authorization
  /<db>/                 → "datastore" level
    /_db/                → DB-level metadata
    /<ns>/               → namespace level
      /_ns/
      /_schema/
      /_types/
      /data/
      /loc/              → Locality index
      /idx/
      /edge/
      /xref/             → Cross-project refs
```

**Key differences:**
1. **Hierarchy depth**: FDB spec has `/pelago/<db>/<ns>/` (3 levels), v0.3 has `(namespace)/` (1 level)
2. **System config**: FDB spec has `/_sys/` and `/_auth/` at root
3. **Locality index**: FDB spec has dedicated `/loc/` subspace, v0.3 doesn't
4. **Cross-project refs**: FDB spec has `/xref/`, v0.3 doesn't
5. **Edge prefix**: FDB spec uses `f/fm/r` prefixes, v0.3 uses `(edges, type, direction, ...)`

### Recommendation

**Adopt the FDB keyspace spec** — it's more complete for multi-site VFX production. Update v0.3 to reference it as authoritative.

**Migration needed:** Update `phase-1-plan.md` M2/M3 key layouts to match FDB spec.

---

## Gap 2: Proto Message Definitions Missing

### Problem

`phase-1-plan.md` line 756 marks this as TODO. No complete protobuf definitions exist.

### What's Documented

`2026-02-13-query-api-cli-design-v2.md` has proto sketches:
- `QueryService`: FindNodes, Traverse, Explain
- `TraverseRequest` with `cascade`, `RecurseConfig`, `per_node_limit`
- `TraversalStep` with `edge_fields`, `SortSpec`

But missing:
- Full `RequestContext` definition
- All Request/Response field types
- Pagination fields (`cursor`, `limit`)
- ErrorDetail proto

### Recommendation

Create complete `proto/pelago.proto` as standalone deliverable BEFORE M0 code begins. Include:

```protobuf
// RequestContext (every request)
message RequestContext {
  string datastore = 1;
  string namespace = 2;
  string site_id = 3;        // For multi-site
  string request_id = 4;     // For tracing
}

// Pagination
message Pagination {
  uint32 limit = 1;
  bytes cursor = 2;          // Opaque, base64
}

// Node reference
message NodeRef {
  string entity_type = 1;
  string node_id = 2;
}

// ErrorDetail (from research-undocumented-gaps.md)
message ErrorDetail {
  string error_code = 1;
  string message = 2;
  string category = 3;
  map<string, string> metadata = 4;
  repeated FieldError field_errors = 5;
}
```

---

## Gap 3: CLI Design Not Integrated with Phase 1

### Problem

`2026-02-13-query-api-cli-design-v2.md` defines a comprehensive CLI:

```
pelago
├── repl                     # PQL REPL
├── schema                   # register, get, list, diff
├── node                     # create, get, update, delete, list
├── edge                     # create, delete, list
├── index                    # create, drop, list
├── admin                    # job status, drop-type, drop-namespace
├── connect
└── config
```

But `phase-1-plan.md` only defines the server binary (`pelago-server`). No CLI crate exists.

### Recommendation

Add `pelago-cli` crate to Phase 1:

```
crates/
  └── pelago-cli/            # CLI binary
      ├── Cargo.toml
      └── src/
          ├── main.rs        # clap CLI with subcommands
          ├── repl.rs        # PQL REPL (Phase 1: basic REPL)
          ├── schema.rs      # schema subcommands
          ├── node.rs        # node subcommands
          └── connect.rs     # connection management
```

**Phase 1 scope for CLI:**
- `pelago connect <address>` — set server address
- `pelago schema register <file.json>` — register schema
- `pelago schema get <type>` — get schema
- `pelago node create/get/update/delete` — CRUD
- `pelago edge create/delete/list` — edge operations
- `pelago repl` — basic PQL REPL (single-block queries only)

**Deferred to Phase 2:**
- Full PQL with variables, multi-block
- `:explain` command
- History persistence

---

## Gap 4: Phase 1 PQL Scope Unclear

### Problem

Two documents define different Phase 1 scopes:

**`phase-1-plan.md` M4:**
> "Single-predicate index selection only. No traversal."

**`2026-02-13-query-api-cli-design-v2.md` Phase 1:**
> "Full PQL in Phase 1 — Including variables, @cascade, @facets, @recurse, aggregations"

These are contradictory. The API design doc is much more ambitious.

### Recommendation

**Keep Phase 1 minimal** (per `phase-1-plan.md`):

| Feature | Phase 1 | Phase 2 |
|---------|---------|---------|
| Single-predicate CEL queries | ✓ | |
| Multi-predicate (residual filter) | ✓ | |
| Index intersection | | ✓ |
| Multi-hop traversal | | ✓ |
| PQL parser | | ✓ |
| Variables | | ✓ |
| @cascade, @recurse | | ✓ |
| Aggregations | | ✓ |

**Phase 1 query interface:**
- `FindNodes(entity_type, cel_expression)` — CEL queries, no PQL
- `ListEdges(source_type, source_id, edge_type)` — edge listing
- No traversal engine

**PQL becomes Phase 2** — move to a separate milestone after basic query works.

---

## Gap 5: Error Codes Not Enumerated

### Problem

`research-undocumented-gaps.md` provides a gRPC status mapping table, but no canonical list of error codes exists.

### Recommendation

Create error code registry in `pelago-core`:

```rust
// pelago-core/src/errors.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    // Schema errors
    SchemaNotFound,
    SchemaValidation,
    SchemaVersionConflict,

    // Node errors
    NodeNotFound,
    NodeAlreadyExists,
    UniqueConstraintViolation,

    // Edge errors
    EdgeNotFound,
    TargetNodeNotFound,
    EdgeTypeNotDeclared,

    // Query errors
    CelSyntaxError,
    CelTypeError,
    QueryTimeout,
    ResultLimitExceeded,

    // System errors
    FdbUnavailable,
    Internal,
}

impl ErrorCode {
    pub fn grpc_code(&self) -> tonic::Code {
        match self {
            Self::SchemaNotFound | Self::NodeNotFound | Self::EdgeNotFound => Code::NotFound,
            Self::SchemaValidation | Self::CelSyntaxError | Self::CelTypeError => Code::InvalidArgument,
            Self::UniqueConstraintViolation | Self::NodeAlreadyExists => Code::AlreadyExists,
            Self::QueryTimeout => Code::DeadlineExceeded,
            Self::ResultLimitExceeded => Code::ResourceExhausted,
            _ => Code::Internal,
        }
    }
}
```

---

## Gap 6: ID Format Disagreement

### Problem

**`phase-1-plan.md`:** `[site_id: u8][counter: u64]` = 9 bytes

**`graph-db-edge-spec.md`:** Recommends UUID v7 (16 bytes, time-ordered)

**`pelago-fdb-keyspace-spec.md`:** Uses string UUIDs (`"p_42"`, `"c_7"`)

### Recommendation

**Adopt the phase-1-plan.md format** — it's compact and extractable:

```rust
#[derive(Clone, Copy, Debug)]
pub struct NodeId {
    pub site: u8,    // Owning site (0-255)
    pub seq: u64,    // Monotonic counter per site
}

impl NodeId {
    pub fn to_bytes(&self) -> [u8; 9] {
        let mut buf = [0u8; 9];
        buf[0] = self.site;
        buf[1..9].copy_from_slice(&self.seq.to_be_bytes());
        buf
    }

    pub fn to_string(&self) -> String {
        // Human-readable: site_seq (e.g., "1_12345")
        format!("{}_{}", self.site, self.seq)
    }
}
```

**Update FDB keyspace spec examples** to use this format instead of string IDs.

---

## Gap 7: Site ID Configuration

### Problem

`_home_site` appears in node CBOR but no spec defines:
- How site IDs are assigned
- What valid values are (u8? string?)
- How site ID is configured at server startup

### Documented

`phase-1-plan.md` CLI:
```rust
#[arg(long, env = "PELAGO_SITE_ID")]
site_id: u8,
```

### Recommendation

Add to V1 spec:

> **Site ID** is a unique u8 identifier (0-255) per physical deployment site. Configured via `PELAGO_SITE_ID` environment variable or `--site-id` CLI flag. All node/edge IDs created by this instance include the site ID prefix. Phase 1 is single-site (site_id=0 default); multi-site coordination is Phase 3.

---

## Gap 8: Auth Model Entirely Missing from Phase 1

### Problem

`research-undocumented-gaps.md` recommends API keys + gRPC interceptor, but `phase-1-plan.md` says nothing about auth.

### Recommendation

**Document explicitly:** Phase 1 has NO authentication.

Add to `phase-1-plan.md`:

> **Phase 1 Auth**: None. All requests are trusted. The API layer does not validate credentials. This is acceptable for single-site development/testing environments. Auth (API keys, gRPC interceptor) is Phase 3 scope.

---

## Gap 9: Snapshot Read Implementation Missing

### Problem

`research-undocumented-gaps.md` Gap I4 recommends:
- Acquire FDB read version at query start
- Pin all reads to that version
- Enforce 5-second transaction window

`phase-1-plan.md` M4 doesn't mention this.

### Recommendation

Add to M4 (CEL Query Pipeline):

> **Snapshot reads**: All FindNodes/ListEdges queries acquire a read version at start (`fdb.get_read_version()`) and use it for all FDB reads within that query. This ensures snapshot consistency within a single query invocation. Queries exceeding 5 seconds are rejected with `QueryTimeout`.

---

## Gap 10: CDC Schema Not Defined

### Problem

`graph-db-spec-v0.3.md` Section 7 has a CdcEntry struct, but:
- Field names differ from FDB keyspace spec
- No versioning for CDC entry format
- No specification for how site ID is captured

### FDB Keyspace Spec Edge Keys:

```
(f, <src_type>, <src_id>, <label>, <tgt_db>, <tgt_ns>, <tgt_type>, <tgt_id>)
```

This includes `tgt_db` and `tgt_ns` for cross-project edges. The CDC entry schema doesn't capture this.

### Recommendation

Unify CDC schema:

```rust
struct CdcEntry {
    site: String,           // Originating site
    timestamp: i64,         // Unix microseconds
    batch_id: Option<String>,
    operations: Vec<CdcOperation>,
}

enum CdcOperation {
    NodeCreate {
        entity_type: String,
        node_id: NodeId,     // [site:u8][seq:u64]
        properties: HashMap<String, Value>,
    },
    NodeUpdate { ... },
    NodeDelete { ... },
    EdgeCreate {
        source: NodeRef,
        target: NodeRef,     // Includes target db/ns for cross-project
        edge_type: String,
        edge_id: EdgeId,
        properties: HashMap<String, Value>,
        pair_id: Option<String>,
    },
    EdgeDelete { edge_id: EdgeId, pair_id: Option<String> },
    SchemaRegister { ... },
}
```

---

## Gap 11: Background Job Types Incomplete

### Problem

`phase-1-plan.md` M5 lists:
- IndexBackfill
- StripProperty
- OrphanedEdgeCleanup (deferred)

But `graph-db-spec-v0.3.md` Section 7 mentions:
- CDC retention/compaction (no spec)
- Statistics maintenance (for query planner)

### Recommendation

Add to V1 spec job types:

```rust
enum JobType {
    // Phase 1
    IndexBackfill { entity_type: String, field: String },
    StripProperty { entity_type: String, property: String },

    // Phase 2
    OrphanedEdgeCleanup { dropped_entity_type: String },
    StatisticsRefresh { entity_type: String },
    CdcRetention { retention_days: u32 },
}
```

**Phase 1 jobs:** IndexBackfill, StripProperty only.

---

## Gap 12: Test Strategy Scope

### Problem

`phase-1-plan.md` M6 defines test layers but doesn't specify:
- Which edge cases to test
- Performance benchmarks
- Concurrent operation tests

### Recommendation

Add test matrix to V1 spec:

| Category | Tests |
|----------|-------|
| Schema | Register, evolve, forward refs, validation errors |
| Node CRUD | Create, read, update, delete, required fields, defaults |
| Index | Unique constraint, range queries, null handling |
| Edge | Create, delete, bidirectional, referential integrity |
| Query | Single-predicate, multi-predicate (residual), cursor pagination |
| CDC | Entry per mutation, versionstamp ordering |
| Concurrency | Parallel creates, unique index race, read-write contention |

---

## Consolidated V1 Spec Outline

A complete V1 spec should have these sections:

```
1. Introduction
   - Goals and non-goals
   - Technology stack

2. Architecture Overview
   - Diagram with crate boundaries
   - Request flow

3. Keyspace Layout (from pelago-fdb-keyspace-spec.md)
   - Directory structure
   - Key encoding
   - Value encoding (CBOR)

4. Data Model
   - Node ID format
   - Edge ID format
   - Property types
   - Null semantics

5. Schema System
   - Entity schema structure
   - Edge declarations
   - Schema evolution rules
   - Index types

6. Node Operations
   - Create with validation
   - Point lookup
   - Update with index maintenance
   - Delete with cascade

7. Edge Operations
   - Create with referential integrity
   - Bidirectional edge pairs
   - Delete (single and paired)
   - Listing

8. Query System
   - CEL expression handling
   - Index matching
   - Query plan generation
   - Snapshot reads

9. gRPC API (full proto file)
   - SchemaService
   - NodeService
   - EdgeService
   - QueryService
   - AdminService

10. CLI Reference
    - Command hierarchy
    - Connection management

11. CDC System
    - Entry schema
    - Versionstamp ordering
    - Consumer pattern

12. Background Jobs
    - Job types
    - Progress tracking
    - Crash recovery

13. Error Model
    - Error codes
    - gRPC mapping
    - Client handling

14. Configuration
    - Server config
    - Site ID
    - FDB cluster file

15. Testing Strategy
    - Unit tests
    - Integration tests
    - Python gRPC tests

Appendix A: Full Proto Definitions
Appendix B: Error Code Registry
Appendix C: Key Layout Reference
```

---

## Priority Actions

### Before Any Code (Blocking)

| # | Action | Source |
|---|--------|--------|
| 1 | Write complete `proto/pelago.proto` | Gap 2 |
| 2 | Resolve keyspace conflict — adopt FDB spec | Gap 1 |
| 3 | Define NodeId/EdgeId format | Gap 6 |
| 4 | Document "no auth in Phase 1" | Gap 8 |

### Before M4 (Query Milestone)

| # | Action | Source |
|---|--------|--------|
| 5 | Add snapshot read version pinning | Gap 9 |
| 6 | Clarify Phase 1 PQL scope (no PQL, CEL only) | Gap 4 |

### Before M5 (gRPC Milestone)

| # | Action | Source |
|---|--------|--------|
| 7 | Define error code registry | Gap 5 |
| 8 | Add CLI crate to plan | Gap 3 |

### Documentation Only (No Code Change)

| # | Action | Source |
|---|--------|--------|
| 9 | Document site ID configuration | Gap 7 |
| 10 | Unify CDC entry schema | Gap 10 |

---

## Code References

- `llm/context/graph-db-spec-v0.3.md` — Main architecture spec
- `llm/context/pelago-fdb-keyspace-spec.md` — FDB keyspace layout
- `llm/context/graph-db-edge-spec.md` — Edge management
- `llm/context/phase-1-plan.md` — Implementation milestones
- `.llm/shared/research/2026-02-13-query-api-cli-design-v2.md` — PQL and API design
- `.llm/shared/research/2026-02-13-traversal-query-language.md` — PQL grammar
- `llm/context/research-phase1-outcome-clarity.md` — Gap analysis
- `llm/context/research-undocumented-gaps.md` — Auth, errors, snapshots

---

## Related Documents

- `llm/context/janusgraph-architecture-deep-dive.md` — Reference architecture
- `llm/context/research-c1-index-intersection.md` — Query planning
- `llm/context/research-i1-null-handling.md` — Null semantics
- `llm/context/research-c6-property-removal.md` — Schema evolution

---

*Research complete. Ready for V1 spec consolidation.*
