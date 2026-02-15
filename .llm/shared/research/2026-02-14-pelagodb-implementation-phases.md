# PelagoDB v1 Implementation Phases

**Status:** Research Document (Revised)
**Date:** 2026-02-14
**Last Updated:** 2026-02-14
**Purpose:** Break down the v1 specification into implementation phases for LLM/AI implementation

---

## Executive Summary

This document defines a **6-phase implementation plan** for PelagoDB v1, where each phase:
1. Ends with a **working, usable system**
2. **Builds incrementally** on previous phases
3. Exposes functionality through **tools and APIs**
4. **Validates specific concerns** (storage, querying, replication, etc.)
5. Is **atomic** and can be reverted to a working state

The phases are designed for LLM/AI implementation where timing/effort is not a concern—what matters is clarity, completeness, and validation at each step.

> **Validation Status:** This document has been validated against the v1 spec. See [Critique Document](.llm/shared/research/2026-02-14-implementation-phases-critique.md) for detailed analysis.

---

## Phase Overview

| Phase | Name | Validates | Working Deliverable |
|-------|------|-----------|---------------------|
| **1** | Storage Foundation | FDB storage, schema, CRUD, indexes | gRPC API for node/edge CRUD + CEL queries |
| **2** | Change Data Capture | Event streaming, CDC consumers | CDC log + consumer framework |
| **3** | Query & Cache + CLI | PQL language, RocksDB cache, CLI | PQL REPL + CLI + cached reads |
| **4** | Real-Time Features | Watch system, subscriptions | Reactive subscriptions via gRPC streaming |
| **5** | Multi-Site Replication | Distributed consistency, ownership | Full multi-site deployment |
| **6** | Security & Authorization | Authentication, ACLs, audit | Production-ready with auth/authz |

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        IMPLEMENTATION DEPENDENCY GRAPH                       │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│   PHASE 1: Foundation                                                        │
│   ├── M0: Project Scaffolding (types, encoding, FDB connection)              │
│   ├── M1: Schema Registry (EntitySchema, validation, versioning)             │
│   ├── M2: Node Operations (CRUD + CDC entry write)                           │
│   ├── M3: Edge Operations (bidirectional, cross-namespace)                   │
│   ├── M4: Query System (CEL compilation, index selection, traversal)         │
│   ├── M5: gRPC API (all Phase 1 services)                                    │
│   └── M6: Testing (Rust integration + Python gRPC)                           │
│           │                                                                  │
│           ▼                                                                  │
│   PHASE 2: CDC System                                                        │
│   ├── M7: CDC entry format (versionstamp ordering)                           │
│   ├── M8: Consumer framework (HWM tracking, checkpointing)                   │
│   ├── M9: Background jobs (index backfill, retention)                        │
│   └── ReplicationService proto (PullCdcEvents)                               │
│           │                                                                  │
│           ├──────────────────────────┬───────────────────────┐               │
│           ▼                          ▼                       ▼               │
│   PHASE 3: Query/Cache/CLI   PHASE 4: Watch System    PHASE 5: Multi-Site   │
│   ├── M10: PQL parser        ├── M14: WatchService    ├── M18: Site registry│
│   ├── M11: PQL compiler      ├── M15: CDC dispatcher  ├── M19: Ownership    │
│   ├── M12: RocksDB cache     ├── M16: Subscriptions   ├── M20: Replicator   │
│   ├── M13: Traversal cache   └── M17: Client resume   └── M21: Conflicts    │
│   └── M13.5-13.7: CLI                │                                      │
│           │                          │                                      │
│           │    ┌─────────────────────┘                                      │
│           │    │  (PQL watches require M10-M11)                             │
│           ▼    ▼                                                            │
│   PHASE 6: Security & Authorization                                         │
│   ├── M22: Authentication (JWT, mTLS)                                       │
│   ├── M23: Authorization (Path-based ACLs)                                  │
│   └── M24: Audit Logging                                                    │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Dependency Notes

> **Soft Dependency:** PQL query watches (§18.3.2) require Phase 3 PQL parser (M10-M11).
> CEL query watches work with Phase 2 only. Phases 3/4 can run in parallel if
> PQL watches are initially disabled and enabled after M11 completion.

> **Phase 5 Independence:** Multi-site replication requires Phases 1-2 only.
> It does NOT require Phases 3 or 4 per the spec (verified against §12).

---

## Phase 1: Storage Foundation

**Goal:** Establish core storage primitives, schema system, CRUD operations, and basic query capabilities.

**Validates:**
- FDB transaction model and performance characteristics
- Schema-first design with validation and indexing
- CEL predicate compilation and execution
- gRPC API contract and error model

**Spec Sections:** §1-9, §13-16

> **Scope Decision:** Traversal is included in Phase 1 (moved from original Phase 2 in gap analysis)
> based on v0.4 critique feedback: "provides immediate graph database value."
> This increases Phase 1 scope but delivers a more complete MVP that validates
> graph traversal capabilities early.

### Milestone Breakdown

#### M0: Project Scaffolding
**Spec References:** §1.3 (Technology Stack), §2.2 (Crate Structure), §3 (Keyspace Layout), §4 (Data Model)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| Workspace setup | Create `pelago/` workspace with all crates | `cargo build` succeeds |
| Core types | `NodeId`, `EdgeId`, `Value`, `PropertyType` | Types round-trip through CBOR |
| Encoding layer | Tuple encoding, sort-order encoding for indexes | Index keys sort correctly |
| FDB connection | `FdbDatabase` initialization, basic txn helpers | Test key written/read from FDB |
| Config system | `ServerConfig` from env vars | Site ID loaded from `PELAGO_SITE_ID` |
| Error model | `PelagoError` enum with gRPC mapping | Errors map to correct gRPC codes |

**Files Created:**
```
crates/pelago-core/src/{lib.rs, types.rs, encoding.rs, errors.rs, config.rs}
crates/pelago-storage/src/{lib.rs, subspace.rs}
crates/pelago-proto/src/lib.rs, build.rs
proto/pelago.proto
```

#### M1: Schema Registry
**Spec References:** §5 (Schema System), §5.6 (Schema Registry Storage)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| Schema structures | `EntitySchema`, `PropertyDef`, `EdgeDef`, `IndexType` | Schemas serialize to CBOR |
| Schema validation | Type checking, required fields, extras policy | Invalid schemas rejected |
| FDB schema storage | `(_meta, schemas, entity_type)` layout | Schema round-trips through FDB |
| Version history | `(_meta, schema_versions, entity_type, version)` | Version history queryable |
| In-memory cache | `HashMap<(ds, ns, type), Arc<EntitySchema>>` | Cache invalidates on update |
| Schema evolution | Detect new indexes, enqueue backfill job | New indexed property triggers job |

**gRPC Services Added:**
- `SchemaService.RegisterSchema`
- `SchemaService.GetSchema`
- `SchemaService.ListSchemas`

#### M2: Node Operations
**Spec References:** §6 (Node Operations), §3.3 (Namespace Subspaces)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| ID allocation | Atomic counter per entity type: `(_meta, id_seq, entity_type)` | IDs are unique, site-prefixed |
| Node CRUD | Create, Read, Update, Delete with schema validation | All CRUD operations work |
| Index maintenance | Write index entries atomically with node | Index entries match node data |
| Locality tracking | Store `locality` field, `(loc, entity_type, node_id)` index | Locality queryable |
| CDC entry write | Append `NodeCreate`, `NodeUpdate`, `NodeDelete` to CDC log | CDC entries written atomically |

**gRPC Services Added:**
- `NodeService.CreateNode`
- `NodeService.GetNode`
- `NodeService.UpdateNode`
- `NodeService.DeleteNode`

#### M3: Edge Operations
**Spec References:** §7 (Edge Operations), §5.3 (Edge Declarations)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| Edge CRUD | Create, Delete edges with bidirectional pairs | Forward + reverse edges written |
| Edge validation | Target type checking, undeclared edge policy | Invalid edges rejected |
| Cross-namespace | Support `ns2::TargetType` edge targets | Cross-namespace edges work |
| Cascade delete | Delete edges when node deleted | No orphaned edges |
| Edge properties | Store properties on edge (if declared) | Edge properties queryable |
| CDC entry write | Append `EdgeCreate`, `EdgeDelete` to CDC log | CDC entries written atomically |

**gRPC Services Added:**
- `EdgeService.CreateEdge`
- `EdgeService.DeleteEdge`
- `EdgeService.ListEdges` (streaming)

#### M4: Query System
**Spec References:** §8 (Query System), §8.1-8.5 (CEL, Index Selection, Traversal)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| CEL compilation | Parse, type-check CEL expressions | Valid CEL compiles |
| Predicate extraction | Extract index-usable predicates from AST | Equality/range predicates extracted |
| Index selection | Choose best index based on predicate | Correct index selected |
| Query execution | Scan + filter + stream results | `FindNodes` returns correct results |
| Explain output | Return query plan without executing | `Explain` shows plan |
| Traversal engine | Multi-hop traversal with per-hop filters | `Traverse` works to max_depth |
| Snapshot isolation | Acquire FDB read version at query start | Multi-batch queries see consistent snapshot |

**gRPC Services Added:**
- `QueryService.FindNodes` (streaming)
- `QueryService.Traverse` (streaming)
- `QueryService.Explain`

#### M5: gRPC API
**Spec References:** §9 (gRPC API), §14 (Error Model)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| Proto definitions | All Phase 1 messages and services | `tonic-build` generates code |
| Request context | `RequestContext` with `database`, `namespace` | Context passed through all calls |
| Error mapping | `PelagoError` → `tonic::Status` with details | Errors include `ErrorDetail` |
| Streaming | Implement server-streaming for queries | Stream backpressure works |
| Health check | `HealthService.Check` | Server reports healthy |
| Job storage | `(_jobs, job_type, job_id)` layout for job state | Jobs persist in FDB |
| Job status API | `GetJobStatus` returns progress | Status queryable via gRPC |

> **Note:** Phase 1 includes job storage and status API but NOT job execution.
> Jobs are created and tracked, but IndexBackfill/StripProperty run in Phase 2.

#### M6: Testing
**Spec References:** §16 (Testing Strategy)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| Rust integration | Tests against real FDB | `cargo test --test integration` passes |
| Python gRPC | API contract tests | `pytest tests/python` passes |
| Test fixtures | Isolated namespaces, cleanup | No test pollution |
| CI setup | Docker Compose with FDB | CI pipeline green |

### Phase 1 Validation Checklist

- [ ] Schema registration and retrieval works via gRPC
- [ ] Node CRUD with property validation
- [ ] Index entries created and queryable
- [ ] Edge creation with bidirectional pairs
- [ ] CEL queries execute with correct index selection
- [ ] Traversal works to configured max depth
- [ ] Multi-batch queries use snapshot isolation
- [ ] All errors return structured `ErrorDetail`
- [ ] CDC entries written (but not consumed yet)
- [ ] Job status API works (jobs created but not executed)

### Performance Targets (Phase 1)

| Operation | Target (p99) |
|-----------|--------------|
| Point lookup (by ID) | < 1ms |
| Index query (single predicate) | < 10ms |
| Traversal (depth 4, 100 nodes) | < 100ms |

### API Availability After Phase 1

| Service | Endpoint | Available |
|---------|----------|-----------|
| SchemaService | RegisterSchema, GetSchema, ListSchemas | ✓ |
| NodeService | CreateNode, GetNode, UpdateNode, DeleteNode | ✓ |
| EdgeService | CreateEdge, DeleteEdge, ListEdges | ✓ |
| QueryService | FindNodes, Traverse, Explain | ✓ |
| AdminService | DropIndex, StripProperty, GetJobStatus | ✓ |
| HealthService | Check | ✓ |
| WatchService | * | ✗ (Phase 4) |
| ReplicationService | * | ✗ (Phase 5) |
| AuthService | * | ✗ (Phase 6) |

---

## Phase 2: Change Data Capture

**Goal:** Establish CDC infrastructure that powers replication, watch system, and cache invalidation.

**Validates:**
- Versionstamp-based ordering works correctly
- CDC consumer pattern handles at-least-once delivery
- Background job framework is robust

**Spec Sections:** §11 (CDC System), §13 (Background Jobs)

### Milestone Breakdown

#### M7: CDC Entry Format
**Spec References:** §11.1 (CdcEntry Structure), §11.2 (Versionstamp Ordering)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| CdcEntry struct | Define all operation types | CBOR serializes correctly |
| Versionstamp key | `(_cdc, <versionstamp>)` layout | Versionstamp set at commit |
| Atomic CDC write | CDC entry in same txn as mutation | Mutations + CDC atomic |

#### M8: Consumer Framework
**Spec References:** §11.3 (Consumer Pattern), §11.4 (Retention)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| High-water mark | `(_meta, cdc_checkpoints, consumer_id)` | HWM persists across restarts |
| Consumer loop | Poll, process, checkpoint pattern | Consumer catches up after restart |
| Batch processing | Process entries in configurable batches | Throughput meets targets |
| Retention job | Truncate expired CDC entries | Old entries deleted |

#### M9: Background Jobs
**Spec References:** §13 (Background Jobs)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| Job worker loop | Poll pending jobs, execute in batches | Jobs execute automatically |
| IndexBackfill | Backfill index entries for existing nodes | New indexes populated |
| StripProperty | Remove property from all nodes of type | Property stripped in batches |
| CdcRetention | Truncate expired CDC entries | Old CDC cleaned up |
| Cursor resumption | Jobs resume from cursor after crash | Jobs complete after restart |

### Phase 2 Validation Checklist

- [ ] CDC entries have strictly ordered versionstamps
- [ ] Consumer resumes from checkpoint after restart
- [ ] Background jobs complete and report progress
- [ ] IndexBackfill populates new indexes correctly
- [ ] CDC retention job cleans old entries
- [ ] Multiple consumers can run independently

### API Additions After Phase 2

| Service | Endpoint | Added |
|---------|----------|-------|
| ReplicationService | PullCdcEvents | ✓ (internal) |
| AdminService | GetJobStatus | Enhanced (shows execution progress) |

---

## Phase 3: Query & Cache + CLI

**Goal:** Implement PQL query language, RocksDB cache for high-throughput reads, and CLI for operators.

**Validates:**
- PQL parser and compiler work correctly
- CDC projector keeps RocksDB in sync
- Cache read path with consistency levels
- CLI provides operator access

**Spec Sections:** §17 (PQL), §19 (RocksDB Cache Layer), §8.6 (Traversal Checkpointing), §10 (CLI Reference)

### Milestone Breakdown

#### M10: PQL Parser
**Spec References:** §17.1-17.3 (Overview, Structure, Grammar)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| Lexer | Tokenize PQL input | All token types recognized |
| Parser | Build AST from tokens | Valid PQL parses |
| Root functions | `node()`, `edge()`, `nodes()` | Root functions work |
| Edge traversal | `-[EDGE_TYPE]->` syntax | Traversal syntax parses |
| Cross-namespace | `@namespace::Type` prefix | Cross-namespace refs work |

#### M11: PQL Compiler
**Spec References:** §17.13 (Compilation Pipeline)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| Type checking | Validate against schema | Type errors caught |
| Query planning | Optimize traversal order | Plan generated |
| CEL generation | Convert filters to CEL | CEL predicates work |
| Explain output | Show plan in `@explain` mode | Plan human-readable |

#### M12: RocksDB Cache
**Spec References:** §19.1-19.6 (Architecture, Keyspace, Projector, Read Path)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| RocksDB setup | Column families for nodes, edges, meta | RocksDB initializes |
| CDC projector | Consume CDC, write to RocksDB | Cache stays in sync |
| HWM tracking | Track projection position | HWM queryable |
| Read path | Check HWM, serve from cache or FDB | Consistency levels work |

#### M13: Traversal Caching
**Spec References:** §19.7 (Traversal Caching), §8.6 (Traversal Checkpointing)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| Edge adjacency | Cache `(src, label) → [targets]` | Traversal faster |
| Continuation tokens | Checkpoint large traversals | Large traversals complete |
| Frontier tracking | Return frontier on truncation | Continuation works |

#### M13.5: CLI Scaffold
**Spec References:** §10 (CLI Reference)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| CLI framework | clap-based `pelago` binary | `pelago --help` works |
| Connection config | `--endpoint`, `--database`, `--namespace` flags | CLI connects to server |
| Output formatting | Table, JSON, CBOR output modes | `--format` flag works |
| Shell completion | Bash/Zsh completion scripts | Completions install |

#### M13.6: CLI Commands
**Spec References:** §10 (CLI Reference)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| Schema commands | `pelago schema register/get/list` | Schema CLI works |
| Node commands | `pelago node create/get/update/delete` | Node CLI works |
| Edge commands | `pelago edge create/delete/list` | Edge CLI works |
| Query commands | `pelago query find/traverse` | Query CLI works |
| Admin commands | `pelago admin job-status/drop-index` | Admin CLI works |

#### M13.7: PQL REPL
**Spec References:** §10 (CLI Reference), §17.11 (REPL Integration)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| REPL mode | `pelago repl` interactive mode | REPL starts |
| Query execution | Execute PQL queries interactively | Queries return results |
| History | Command history with up/down arrows | History persists |
| Multiline | Support multiline queries | `{` enters multiline mode |

### Phase 3 Validation Checklist

- [ ] PQL queries parse and execute correctly
- [ ] RocksDB cache stays in sync with FDB via CDC
- [ ] Read consistency levels (strong, session, eventual) work
- [ ] Large traversals complete with continuation
- [ ] `pelago --help` shows all commands
- [ ] All CLI commands work against running server
- [ ] PQL REPL interactive mode works

### API Additions After Phase 3

| Service | Endpoint | Added |
|---------|----------|-------|
| QueryService | ExecutePQL | ✓ |
| QueryService | Traverse (enhanced) | Continuation support |

---

## Phase 4: Real-Time Features

**Goal:** Implement reactive subscriptions via the Watch System.

**Validates:**
- Server-streaming gRPC works reliably
- CDC dispatcher routes events efficiently
- Client reconnection/resume works correctly

**Spec Sections:** §18 (Watch System)

> **Soft Dependency on Phase 3:** PQL query watches require M10-M11 (PQL parser/compiler).
> CEL query watches work immediately. If Phase 4 starts before Phase 3 completes,
> implement PQL query watch support as a follow-up when M11 is done.

### Milestone Breakdown

#### M14: WatchService Proto
**Spec References:** §18.2 (WatchService gRPC API), §18.10 (Proto Definitions)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| Proto definitions | WatchPoint, WatchQuery, WatchNamespace | Proto compiles |
| WatchEvent | All event types defined | Events serialize |
| Subscription types | Point, Query, Namespace watches | All types supported |

#### M15: CDC Dispatcher
**Spec References:** §18.6 (CDC Integration Design)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| WatchDispatcher | Consume CDC, route to subscriptions | Events routed correctly |
| Node/edge index | O(1) lookup for point watches | Point watches efficient |
| Query evaluation | Evaluate CEL for query watches | CEL query watches filter correctly |
| PQL query watches | Evaluate PQL for query watches | PQL query watches work (after M11) |

#### M16: Subscription Management
**Spec References:** §18.7 (Subscription Registry), §18.9 (Resource Management)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| Subscription registry | Track active subscriptions | Subscriptions managed |
| Resource limits | Per-connection, per-namespace limits | Limits enforced |
| TTL management | Expire old subscriptions | Subscriptions expire |
| Backpressure | Handle slow consumers | Events dropped gracefully |

#### M17: Client Resume
**Spec References:** §18.8 (Client Reconnection)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| Position tracking | Return versionstamp in events | Position available |
| Resume validation | Validate position exists | Invalid positions rejected |
| Resume streaming | Continue from position | Resume works |

### Phase 4 Validation Checklist

- [ ] Point watches deliver node/edge changes
- [ ] CEL query watches emit enter/update/exit events
- [ ] PQL query watches work (after Phase 3 complete)
- [ ] Namespace watches stream all changes
- [ ] Client resumes after disconnection
- [ ] Resource limits prevent runaway subscriptions

### API Additions After Phase 4

| Service | Endpoint | Added |
|---------|----------|-------|
| WatchService | WatchPoint | ✓ |
| WatchService | WatchQuery | ✓ |
| WatchService | WatchNamespace | ✓ |
| WatchService | ListSubscriptions | ✓ |
| WatchService | CancelSubscription | ✓ |

---

## Phase 5: Multi-Site Replication

**Goal:** Enable multi-site deployments with ownership-based conflict resolution.

**Validates:**
- Ownership model prevents conflicts
- Replicator stays in sync across sites
- LWW fallback handles edge cases

**Spec Sections:** §12 (Multi-Site Replication)

### Milestone Breakdown

#### M18: Site Registry
**Spec References:** §12.4 (Replication Architecture)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| Site claims | `(_sys, site_claim, site_id)` | Sites discoverable |
| Site ID collision | Guard against duplicate site IDs | Collisions detected |
| Autodiscovery | Detect new sites on startup | New sites discovered |

#### M19: Ownership Model
**Spec References:** §12.2 (Ownership Model), §12.3 (Ownership Transfer)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| Node ownership | Enforce locality-based ownership | Non-owner writes rejected |
| Edge ownership | SourceSite vs Independent modes | Edge ownership works |
| Ownership transfer | TransferOwnership RPC | Ownership transfers |
| CDC operation | OwnershipTransfer CDC entry | Transfer replicates |

#### M20: Replicator
**Spec References:** §12.4 (Replication Architecture), §12.5 (Event Projection)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| Position tracking | `(_sys, repl_position, remote_site)` | Position persists |
| CDC pull | Fetch events from remote via gRPC | Events pulled |
| Event projection | Apply remote events locally | Projection works |
| Ownership filtering | Only apply events from owner | Filtering works |

#### M21: Conflict Resolution
**Spec References:** §12.6 (Conflict Resolution), §12.9 (Failure Modes)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| Owner-wins | Reject non-owner writes | No conflicts in normal operation |
| LWW fallback | Timestamp-based resolution | Split-brain handled |
| Split-brain detection | Detect ownership divergence | Detection alerts |

### Phase 5 Validation Checklist

- [ ] Sites discover each other via registry
- [ ] Ownership enforcement prevents conflicts
- [ ] Replicator syncs data across sites
- [ ] Ownership transfer works correctly
- [ ] LWW handles split-brain scenarios

### API Additions After Phase 5

| Service | Endpoint | Added |
|---------|----------|-------|
| ReplicationService | PullCdcEvents | ✓ (external) |
| NodeService | TransferOwnership | ✓ |
| AdminService | ListSites | ✓ |
| AdminService | GetReplicationStatus | ✓ |

---

## Phase 6: Security & Authorization

**Goal:** Implement authentication, authorization, and audit logging for production readiness.

**Validates:**
- Authentication mechanisms work correctly
- Path-based ACLs enforce access control
- Audit logging captures security events

**Spec Sections:** §20 (Security and Authorization)

> **Why Phase 6?** Security is placed last to avoid blocking feature development,
> but is REQUIRED before production deployment. All prior phases can run in
> development mode without auth, but production deployments must complete Phase 6.

### Milestone Breakdown

#### M22: Authentication
**Spec References:** §20.2 (Authentication Mechanisms), §20.8 (RequestContext Integration)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| JWT validation | Validate JWT tokens in gRPC interceptor | Valid tokens authenticate |
| mTLS support | Client certificate authentication | mTLS connections work |
| API keys | Static API key authentication (dev/test) | API keys authenticate |
| RequestContext | Extract principal from auth credentials | Principal in context |
| Token refresh | Support token refresh flow | Tokens refresh cleanly |

**gRPC Services Added:**
- `AuthService.Authenticate`
- `AuthService.RefreshToken`
- `AuthService.ValidateToken`

#### M23: Authorization
**Spec References:** §20.3 (Principal Model), §20.4-20.7 (Permissions, Policies, Storage)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| Principal model | User, Service, System principal types | Principal types work |
| Path-based ACLs | `(database, namespace, entity_type, *)` paths | ACLs check correctly |
| Policy storage | `(_sys, policies, policy_id)` in FDB | Policies persist |
| Permission checks | Check ACLs in all operations | Unauthorized rejected |
| Built-in roles | `admin`, `read-only`, `namespace-admin` | Roles grant correct permissions |
| Policy API | CRUD for authorization policies | Policies manageable |

**gRPC Services Added:**
- `AuthService.CreatePolicy`
- `AuthService.GetPolicy`
- `AuthService.ListPolicies`
- `AuthService.DeletePolicy`
- `AuthService.CheckPermission`

#### M24: Audit Logging
**Spec References:** §20.10 (Audit Logging)

| Task | Description | Exit Criteria |
|------|-------------|---------------|
| Audit events | Log auth success/failure, policy changes | Audit events captured |
| CDC integration | Audit events as CDC entries | Audit in CDC log |
| Audit queries | Query audit log by principal, time, action | Audit queryable |
| Retention policy | Configurable audit retention | Old audit cleaned up |

### Phase 6 Validation Checklist

- [ ] JWT authentication works
- [ ] mTLS authentication works
- [ ] Unauthenticated requests rejected
- [ ] Unauthorized requests rejected (403)
- [ ] Path-based ACLs enforce correctly
- [ ] Built-in roles work
- [ ] Audit log captures events
- [ ] Audit log is queryable

### API Additions After Phase 6

| Service | Endpoint | Added |
|---------|----------|-------|
| AuthService | Authenticate | ✓ |
| AuthService | RefreshToken | ✓ |
| AuthService | ValidateToken | ✓ |
| AuthService | CreatePolicy | ✓ |
| AuthService | GetPolicy | ✓ |
| AuthService | ListPolicies | ✓ |
| AuthService | DeletePolicy | ✓ |
| AuthService | CheckPermission | ✓ |
| AdminService | QueryAuditLog | ✓ |

---

## Complete API Availability Matrix

| Service | Endpoint | Phase |
|---------|----------|-------|
| **HealthService** | Check | 1 |
| **SchemaService** | RegisterSchema | 1 |
| | GetSchema | 1 |
| | ListSchemas | 1 |
| **NodeService** | CreateNode | 1 |
| | GetNode | 1 |
| | UpdateNode | 1 |
| | DeleteNode | 1 |
| | TransferOwnership | 5 |
| **EdgeService** | CreateEdge | 1 |
| | DeleteEdge | 1 |
| | ListEdges | 1 |
| **QueryService** | FindNodes | 1 |
| | Traverse | 1 (enhanced in 3) |
| | Explain | 1 |
| | ExecutePQL | 3 |
| **AdminService** | DropIndex | 1 |
| | StripProperty | 1 |
| | DropEntityType | 1 |
| | DropNamespace | 1 |
| | GetJobStatus | 1 (enhanced in 2) |
| | ListSites | 5 |
| | GetReplicationStatus | 5 |
| | QueryAuditLog | 6 |
| **WatchService** | WatchPoint | 4 |
| | WatchQuery | 4 |
| | WatchNamespace | 4 |
| | ListSubscriptions | 4 |
| | CancelSubscription | 4 |
| **ReplicationService** | PullCdcEvents | 2 (internal), 5 (external) |
| **AuthService** | Authenticate | 6 |
| | RefreshToken | 6 |
| | ValidateToken | 6 |
| | CreatePolicy | 6 |
| | GetPolicy | 6 |
| | ListPolicies | 6 |
| | DeletePolicy | 6 |
| | CheckPermission | 6 |

---

## Validation Gates

Each phase must pass these validation gates before proceeding:

### Phase 1 Gate
- [ ] All gRPC services respond correctly
- [ ] Schema validation rejects invalid inputs
- [ ] Index queries return correct results
- [ ] Snapshot isolation works for multi-batch queries
- [ ] Error responses include structured details
- [ ] Python test suite passes
- [ ] Performance targets met (point lookup < 1ms, index query < 10ms)

### Phase 2 Gate
- [ ] CDC consumer processes all operations
- [ ] Consumer resumes correctly after restart
- [ ] Background jobs complete successfully
- [ ] IndexBackfill populates indexes correctly
- [ ] Multiple consumers operate independently

### Phase 3 Gate
- [ ] PQL queries parse and execute
- [ ] RocksDB cache reflects FDB state
- [ ] Traversal continuation works
- [ ] Read consistency levels enforced
- [ ] CLI commands work for all operations
- [ ] PQL REPL interactive mode works

### Phase 4 Gate
- [ ] Watch subscriptions deliver events
- [ ] Client resume works across disconnects
- [ ] Resource limits prevent abuse
- [ ] CEL query watches filter correctly
- [ ] PQL query watches work (after Phase 3)

### Phase 5 Gate
- [ ] Multi-site deployment works
- [ ] Ownership prevents conflicts
- [ ] Replication stays in sync
- [ ] Ownership transfer succeeds

### Phase 6 Gate
- [ ] Authentication works (JWT + mTLS)
- [ ] Authorization ACLs enforced
- [ ] Audit logging captures events
- [ ] No unauthorized access possible

### Production Readiness Gate
All phases complete, plus:
- [ ] Authentication enabled and enforced
- [ ] Authorization policies configured
- [ ] Audit logging active
- [ ] Performance benchmarks met
- [ ] Security review complete
- [ ] Disaster recovery tested

---

## Implementation Notes for LLM/AI

### Code Organization
- Follow existing patterns in the codebase
- Use `tracing` for all logging (not `log`)
- Implement `thiserror` for error types
- Use `tokio` async runtime throughout

### Testing Strategy
- Write Rust integration tests first
- Add Python gRPC tests for API contract
- Use isolated namespaces for test isolation
- Clean up test data in fixture teardown
- Add security tests in Phase 6

### Proto Style
- Follow existing `pelago.v1` package conventions
- Use `RequestContext` in all requests
- Include `ErrorDetail` in all error responses
- Use streaming for potentially large responses

### Key Dependencies (from spec §1.3)
- `fdb` crate for FoundationDB
- `cel-rust` or `cel-interpreter` for CEL
- `ciborium` for CBOR encoding
- `tonic` for gRPC
- `rocksdb` crate for cache layer
- `clap` for CLI
- `jsonwebtoken` for JWT validation

---

## References

- **Spec Location:** `.llm/context/pelagodb-spec-v1.md`
- **Phase 1 Plan:** `.llm/context/archive/phase-1-plan.md`
- **Gap Analysis:** `.llm/shared/research/2026-02-13-v1-spec-gap-analysis.md`
- **Validation Critique:** `.llm/shared/research/2026-02-14-implementation-phases-critique.md`
- **Watch System Spec:** Spec Section 18
- **Traversal Checkpointing:** Spec Section 8.6
- **Security Spec:** Spec Section 20
- **CLI Reference:** Spec Section 10

---

## Changelog

- **2026-02-14 (v1):** Initial research document created
- **2026-02-14 (v2):** Incorporated critique findings:
  - Added Phase 6 (Security & Authorization) covering §20
  - Added CLI milestones (M13.5-M13.7) covering §10
  - Added snapshot isolation task to M4
  - Clarified background job phasing (storage in Phase 1, execution in Phase 2)
  - Added PQL query watch soft dependency note
  - Added performance targets to Phase 1
  - Added Production Readiness Gate
  - Added traversal scope rationale note
