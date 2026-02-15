# Research: Schema Lifecycle and Property Removal (Gap C6)

**Status:** Research Document
**Author:** Andrew + Claude
**Date:** 2026-02-10
**Scope:** Design for client-controlled schema lifecycle and explicit property removal without lifecycle state machines.

---

## Executive Summary

Gap C6 identifies undefined behavior when properties are removed from entity schemas. The spec supports additive schema evolution but has no mechanism for removing fields cleanly. This document proposes a drastically simplified model:

**Client-owned schema lifecycle. NoSQL YOLO.**

- **Schemas are client-defined state.** The client sends a schema definition via gRPC. The server stores it, enforces it, and that's it.
- **Indexes are explicit RPCs.** If the client wants an index, they request it. The server creates it (with background backfill for existing data).
- **Property removal is the client's problem.** When a property is removed from the schema, the system provides tools (optional background cleanup, index deletion) but doesn't enforce anything. The client decides when and how to clean up.

No three-state lifecycle. No migration machinery. No deprecated states. Just straightforward operations.

---

## 1. Core Model

### 1.1 Schema Registration

The client registers a schema by sending a JSON definition. The server stores it as the current schema version.

```json
{
  "namespace": "payroll",
  "entity": {
    "name": "Person",
    "version": 1,
    "properties": {
      "name": { "type": "string", "required": true },
      "email": { "type": "string", "required": true, "index": "unique" },
      "department": { "type": "string" }
    }
  }
}
```

Each schema registration increments the version number. Old versions are retained in FDB for reference but do not affect current behavior. The latest version is active.

**Schema versioning:**
- Schema version increments on any property or edge change.
- Old versions are stored at `(ns, "_meta", "schema_versions", entity_type, version)` as read-only history.
- Current schema is stored at `(ns, "_meta", "schemas", entity_type)`.
- No schema deprecation, no activation state, no lifecycle machinery. It's history, not state.

### 1.2 Explicit Index Creation

Properties do NOT automatically create indexes when declared in the schema. The client explicitly requests indexes via RPC.

```protobuf
message CreateIndexRequest {
  string namespace = 1;
  string entity_type = 2;
  string property_name = 3;
  IndexType type = 4;  // unique, equality, range
}

enum IndexType {
  UNIQUE = 0;
  EQUALITY = 1;
  RANGE = 2;
}
```

When a client requests an index on `salary`:
1. Server creates the index FDB key structure: `(ns, entities, Type, idx, salary, *)`
2. If existing data exists, server enqueues a background job to backfill the index over all nodes of that type
3. New writes immediately create index entries for `salary`
4. The client can query before backfill completes (results are incomplete until backfill done, but queries don't fail)

**No schema field for indexing.** The schema only defines properties and types. Indexes are a separate concern managed via explicit RPCs.

### 1.3 Property Removal: Simple

When the client removes a property from the schema (registers a new version without it):

1. **Existing nodes retain the property in CBOR.** The system does not automatically clean up old data.
2. **Queries for the removed property fail at CEL type-check.** The field is not in the schema's CEL environment, so `salary >= 100000` is rejected: "salary not in schema".
3. **Existing indexes remain orphaned.** If there was an index on `salary`, it still exists in FDB. The client must explicitly request that the index be dropped.
4. **The property is invisible to queries.** Even though nodes have `salary` in their CBOR, they won't appear in index-backed queries (the index is unused) or CEL queries (type-check rejects it).
5. **The client can optionally clean up:**
   - **Drop the index:** `DropIndex(entity_type="Person", property="salary")`
   - **Strip the property from all nodes:** `StripProperty(entity_type="Person", property="salary")` — background job that rewrites CBOR to remove the field

**This is entirely the client's responsibility.** The system provides the tools; the client decides whether and when to use them.

---

## 2. The Concrete Scenario

### Initial State

Person schema v1 with 100,000 existing nodes:

```json
{
  "namespace": "payroll",
  "entity": {
    "name": "Person",
    "version": 1,
    "properties": {
      "name": { "type": "string", "required": true },
      "email": { "type": "string", "required": true },
      "salary": { "type": "int" },
      "department": { "type": "string" }
    }
  }
}
```

Suppose the client previously created an index on salary via `CreateIndex(entity_type="Person", property="salary", type="range")`. Index entries populate `(payroll, entities, Person, idx, salary, *)` for all 100,000 nodes.

### Schema Update: Remove Salary

Client registers schema v2:

```json
{
  "namespace": "payroll",
  "entity": {
    "name": "Person",
    "version": 2,
    "properties": {
      "name": { "type": "string", "required": true },
      "email": { "type": "string", "required": true },
      "department": { "type": "string" }
    }
  }
}
```

What happens:

1. **Schema v2 is registered.** Schema v1 remains in history. No node migration, no automatic cleanup.
2. **Existing nodes keep salary in CBOR.** `(payroll, entities, Person, data, p_42)` still contains `{name: "Alice", email: "alice@...", salary: 120000, department: "Eng", ...}`. It's there, but invisible.
3. **Queries referencing salary now fail.** A query like `salary >= 100000` is rejected at CEL type-check: "salary not in schema."
4. **The salary index is orphaned.** Index entries in `(payroll, entities, Person, idx, salary, *)` still exist but are never used (because the query would fail at compile time anyway).
5. **New nodes do not have salary.** When a client creates a new Person node via the v2 schema, they don't provide salary (it's not in the schema), so new nodes don't have it in CBOR.

### Client Cleanup

If the client wants to reclaim storage, they have two options:

**Option A: Drop the index**
```protobuf
DropIndexRequest {
  namespace: "payroll"
  entity_type: "Person"
  property_name: "salary"
}
```
This removes the FDB entries at `(payroll, entities, Person, idx, salary, *)`. Old nodes still have salary in CBOR, but that's fine — it's not indexed, and queries don't use it.

**Option B: Strip the property**
```protobuf
StripPropertyRequest {
  namespace: "payroll"
  entity_type: "Person"
  property_name: "salary"
  trigger_background_job: true
}
```
Server enqueues a background job that:
- Scans all Person nodes
- For each node: deserialize CBOR, remove `salary`, re-serialize and write back
- This reclaims storage (no more salary values in the blobs)
- This is **optional and asynchronous** — the client requests it, but the system works in the background

**Option C: Do nothing**
The salary data remains in CBOR. It's dead weight in storage, but queries work fine (they can't reference salary anyway). If the client never writes to those nodes, the data persists indefinitely. This is acceptable for many deployments.

---

## 3. Property Lifecycle: No State Machine

Unlike the three-state proposal (Active → Deprecated → Removed), this model has no state machine at all.

**There is only one state: Active.**

A property is in a schema (active) or not in a schema (removed). That's it. No intermediate "Deprecated" state. No minimum deprecation window. No automatic transitions.

**What happens when a property is removed:**
- Old nodes may have it in CBOR (invisible to queries)
- The property is gone from the schema (new nodes don't have it)
- Old indexes remain (orphaned and unused)
- CEL queries fail at type-check (non-breaking — clients must migrate anyway)
- The client can clean up via explicit RPCs

**Why no deprecation state?**
- It adds complexity (lifecycle transitions, minimum duration windows, API endpoints to check deprecation status).
- Most clients have their own migration processes and timelines. Adding a server-side enforced window doesn't help.
- If a client needs a grace period, they can run it themselves: keep the property in the schema for as long as they want, then remove it.

---

## 4. Index Management

### Creating Indexes

When the client declares a property in the schema, it does NOT automatically create an index. The property is declared for type-checking and CEL compilation, but not indexed.

If the client wants an index:
```protobuf
CreateIndexRequest {
  namespace: "payroll"
  entity_type: "Person"
  property_name: "age"
  type: RANGE
}
```

Server actions:
1. Validates that the property exists in the current schema
2. Creates the FDB key structure for the index
3. **Enqueues a background job** to backfill the index over all existing nodes
4. New writes immediately create index entries (for nodes created after the RPC, or updates to nodes)
5. Backfill job scans existing nodes, extracts the property, and writes index entries

**Before backfill completes:**
- Queries using the index may return incomplete results
- CEL type-check succeeds (property exists in schema)
- Clients should not rely on the index until the backfill job completes

**After backfill completes:**
- Index is complete
- All queries are consistent

### Deleting Indexes

If the client removes a property from the schema OR simply wants to drop an index:

```protobuf
DropIndexRequest {
  namespace: "payroll"
  entity_type: "Person"
  property_name: "salary"
}
```

Server actions:
1. **Immediately** deletes all entries in `(ns, entities, Type, idx, property, *)`
2. Future writes do not create entries for this property
3. Queries can no longer use the index (CEL planner will do full scans or residual filters)

**Synchronous operation.** No background job needed — FDB range delete is atomic and fast.

---

## 5. CEL Type-Checking

### Active Property (In Schema)

```cel
age >= 30
```
- Type-check succeeds. `age` exists in the schema, type matches.
- Query compiles and executes.

### Removed Property (Not In Schema)

```cel
salary >= 100000
```
- Type-check fails. Error: `ERR_UNKNOWN_FIELD: "salary"`.
- Query is rejected before execution.
- **This is correct behavior.** The schema explicitly says salary is not a property. Clients must rewrite queries to match the current schema.

### No Warnings

Unlike the deprecation model (which emits warnings), there are no special warnings for removed properties. The type-check simply fails, and the client sees the error and fixes their query.

---

## 6. Nodes Without a Value Don't Get Indexed

If the schema says `salary` is an `int` and a Person node doesn't have a `salary` value, no index entry is created for that node.

**This applies whether the node was created before salary was added to the schema, or after salary was removed.**

When a client queries for `salary >= 100000`:
- Server scans the salary index
- Only nodes with a salary value have index entries
- Nodes without salary don't appear in the index
- No artificial null/missing entries, no three-valued logic — just simple: if the property exists in the node's CBOR, it's indexed; if not, it's not

This simplifies indexing and makes the behavior predictable.

---

## 7. CDC and Replication

### CDC Entries Include Current Properties

A CDC entry reflects the committed state at write time:

```
CDC entry (when salary was in schema):
  NodeCreate {
    entity_type: "Person",
    node_id: "p_99",
    properties: { name, email, salary: 98500, department }
  }
```

After salary is removed from the schema:

```
CDC entry (after schema change):
  NodeUpdate {
    entity_type: "Person",
    node_id: "p_99",
    changed_properties: { department: "Finance" }
  }
```

The receiving site applies these entries regardless of schema version mismatches:

1. Deserialize the entry
2. Load the local schema for the entity type
3. **Strip any fields not in the local schema**
4. Apply the operation

So if a receiving site has schema v2 (without salary) and receives a pre-removal CDC entry with salary, the receiving site silently ignores the salary field. Information is not lost at the application layer — the data is still in the FDB replicated node's CBOR — but from the query perspective, salary is not accessible.

**Replication is robust to schema mismatches.** Old CDC entries with removed fields work fine. New CDC entries generated after the schema change don't include the removed fields.

---

## 8. Comparison to Other Systems

### MongoDB

**Approach:** Schema validation is optional. Remove a field from the validation schema; existing documents retain it. Optional cleanup via `updateMany({}, [{ $unset: "field" }])`.

**Similarity:** No required deprecation phase. Cleanup is optional.

**Difference:** MongoDB has no explicit index lifecycle. This design separates index management from schema definition.

### Cassandra

**Approach:** `ALTER TABLE DROP COLUMN salary` immediately removes the column from the schema. Existing SSTables may retain it until compaction (lazy cleanup).

**Similarity:** Removal is immediate, not gradual. Lazy CBOR cleanup (equivalent to Cassandra's compaction).

**Difference:** This design provides an optional eager cleanup tool if the client wants it.

### Elasticsearch

**Approach:** Remove field from mapping. New documents don't have it. Old documents retain it until index is reindexed.

**Similarity:** Lazy cleanup, optional explicit cleanup (reindex).

**Difference:** Elasticsearch doesn't have the concept of property schemas in the same way; this design is simpler.

### PostgreSQL

**Approach:** `ALTER TABLE DROP COLUMN salary` removes the column. Cleanup happens during VACUUM (lazy).

**Similarity:** Removal is immediate; physical cleanup is lazy.

**Difference:** No explicit option for eager cleanup (the client can't request it).

---

## 9. Implementation

### Schema Registration

- Client sends schema JSON with no index information
- Server validates structure and property types
- Server stores as current version at `(ns, "_meta", "schemas", entity_type)`
- Server stores version history at `(ns, "_meta", "schema_versions", entity_type, version)`
- CEL environment is rebuilt based on the new schema

### Index Creation (Background Job)

```
Client: CreateIndexRequest { entity_type: "Person", property: "age", type: "range" }
  │
  ▼
Server validates property exists in schema
  │
  ▼
Server creates index key structure
  │
  ▼
Server enqueues background job: IndexBackfill { entity_type, property, index_type }
  │
  ▼
ACK to client (index creation started)
  │
  ▼ (async, non-blocking)
Background job:
  ├─ Scan all Person nodes
  ├─ Extract property value
  ├─ Write index entry: (ns, entities, Person, idx, age, value, node_id)
  ├─ Track progress cursor
  └─ Mark job completed when done
```

### Index Deletion

```
Client: DropIndexRequest { entity_type: "Person", property: "salary" }
  │
  ▼
Server validates (optional: check that property not in schema)
  │
  ▼
Server issues FDB clearRange on (ns, entities, Person, idx, salary, *)
  │
  ▼
ACK to client (index deleted)
```

Synchronous and atomic. No background job.

### Property Stripping (Optional Background Job)

```
Client: StripPropertyRequest { entity_type: "Person", property: "salary" }
  │
  ▼
Server enqueues background job: StripProperty { entity_type, property }
  │
  ▼
ACK to client
  │
  ▼ (async, non-blocking)
Background job:
  ├─ Scan all Person nodes
  ├─ Deserialize CBOR
  ├─ Remove "salary" field
  ├─ Re-serialize and write back
  ├─ Track progress cursor
  └─ Mark job completed when done
```

Optional. Client can leave old data in place if they prefer.

### Write Path (No Schema State)

```
Client write arrives (e.g., CreateNode)
  │
  ▼
Validate against current schema:
  ├─ All required properties provided
  ├─ All properties in the request exist in current schema
  └─ All property types match
  │
  ▼
Single FDB transaction:
  ├─ Write node data
  ├─ Write index entries (for properties that have indexes)
  └─ Append CDC entry
  │
  ▼
ACK to client
```

No schema version check. Just validate against current.

---

## 10. API Endpoints

### Schema Management

```protobuf
service Schema {
  // Register a new schema version
  rpc RegisterSchema(RegisterSchemaRequest) returns (RegisterSchemaResponse);

  // Get current schema
  rpc GetSchema(GetSchemaRequest) returns (EntitySchema);

  // Get specific version
  rpc GetSchemaVersion(GetSchemaVersionRequest) returns (EntitySchema);

  // List all versions
  rpc ListSchemaVersions(ListSchemaVersionsRequest) returns (ListSchemaVersionsResponse);
}

message RegisterSchemaRequest {
  string namespace = 1;
  string schema_json = 2;  // Full schema definition
}

message RegisterSchemaResponse {
  uint32 version = 1;  // New version number
  int64 timestamp = 2; // When registered
}

message GetSchemaRequest {
  string namespace = 1;
  string entity_type = 2;
}

message GetSchemaVersionRequest {
  string namespace = 1;
  string entity_type = 2;
  uint32 version = 3;
}
```

### Index Management

```protobuf
service Indexes {
  // Create an index on a property
  rpc CreateIndex(CreateIndexRequest) returns (CreateIndexResponse);

  // Drop an index
  rpc DropIndex(DropIndexRequest) returns (DropIndexResponse);

  // List indexes for a type
  rpc ListIndexes(ListIndexesRequest) returns (ListIndexesResponse);

  // Check index backfill status
  rpc GetIndexStatus(GetIndexStatusRequest) returns (GetIndexStatusResponse);
}

message CreateIndexRequest {
  string namespace = 1;
  string entity_type = 2;
  string property_name = 3;
  IndexType type = 4;  // UNIQUE, EQUALITY, RANGE
}

message CreateIndexResponse {
  string job_id = 1;  // For tracking backfill progress
}

message DropIndexRequest {
  string namespace = 1;
  string entity_type = 2;
  string property_name = 3;
}

message DropIndexResponse {
  bool deleted = 1;
}

message ListIndexesResponse {
  repeated IndexInfo indexes = 1;
}

message IndexInfo {
  string property_name = 1;
  IndexType type = 2;
  string status = 3;  // BACKFILLING, COMPLETE
  float progress = 4; // 0.0 to 1.0 if backfilling
}
```

### Cleanup Operations

```protobuf
service Cleanup {
  // Strip property from all nodes of a type
  rpc StripProperty(StripPropertyRequest) returns (StripPropertyResponse);

  // Check cleanup job status
  rpc GetCleanupJobStatus(GetCleanupJobStatusRequest) returns (CleanupJobStatus);
}

message StripPropertyRequest {
  string namespace = 1;
  string entity_type = 2;
  string property_name = 3;
}

message StripPropertyResponse {
  string job_id = 1;  // For tracking progress
}

message CleanupJobStatus {
  string job_id = 1;
  string status = 2;  // PENDING, RUNNING, COMPLETED, FAILED
  float progress = 3; // 0.0 to 1.0
  int64 nodes_processed = 4;
}
```

---

## 11. CLI Tools

Simple, no state machine complexity:

```bash
# Register a new schema
pelagodb schema register --namespace payroll --file person_v2.json

# Check current schema
pelagodb schema current --namespace payroll --entity Person

# Get full version history
pelagodb schema history --namespace payroll --entity Person

# Create an index
pelagodb index create --namespace payroll --entity Person --property age --type range

# Check index status
pelagodb index status --namespace payroll --entity Person --property age

# Drop an index
pelagodb index drop --namespace payroll --entity Person --property salary

# Strip a property (background job)
pelagodb property strip --namespace payroll --entity Person --property salary

# Check cleanup status
pelagodb job status --job-id cleanup_Person_salary_20260210_001
```

No deprecation commands. No removal-eligibility checks. No minimum-duration enforcement.

---

## 12. Why This Works

### Simplicity

No state machine. No transitions. No minimum windows. No automatic lifecycle. The system does one thing well: the client defines the schema, and the system enforces it.

### Control

The client controls when indexes are created, when they're dropped, when old data is cleaned up. The server provides the tools; the client decides.

### Robustness

CEL type-check fails cleanly on removed properties. No silent failures. No data loss. If a client queries for a removed property, they get an error and fix their query.

### Backward Compatibility

Old nodes retain their data in CBOR. No forced migration. No data scrubbing. If a client removes a property and later wants it back, they can re-add it to the schema and re-index. Old nodes already have the data.

### Multi-Site Friendly

CDC entries strip unknown fields on receipt. Sites with different schema versions converge gracefully. No schema synchronization machinery needed.

---

## 13. Gaps Resolved

**C6: Property removal is undefined behavior.**

RESOLVED. When a property is removed:
1. It's no longer in the schema
2. Existing nodes keep the value in CBOR
3. Queries fail at type-check
4. Indexes become orphaned (client can drop them)
5. Client can optionally strip the property from all nodes

No state machine. No lifecycle. Clean semantics.

**M2: Schema version management is incomplete.**

RESOLVED. Schema versions are immutable history. Current version is active. Clients discover the current version via `GetSchema()`. Old versions are accessible via `GetSchemaVersion(version_num)` for reference only.

---

## 14. Risk Mitigation

| Risk | Mitigation |
|------|-----------|
| **Dead weight in CBOR from removed properties** | Client can optionally call `StripProperty()` to clean it up. System provides the tool. |
| **Orphaned index entries** | Client must explicitly call `DropIndex()`. No automatic cleanup, so no surprise deletions. |
| **Queries fail on removed properties** | Type-check error makes it obvious. Client sees the error and rewrites the query. |
| **Schema divergence between sites** | CDC strips unknown fields. Sites converge as they upgrade schemas. |
| **Old nodes without new properties** | New properties are optional on creation. Queries handle missing values naturally (not in index, not in result set). |

---

## 15. Example Workflow

### Day 1: Initial Schema and Index

Client registers Person schema with `salary` and `department` properties. No indexes by default.

Client explicitly requests an index on `salary`:
```
CreateIndexRequest { entity_type: "Person", property: "salary", type: "range" }
```
Background job backfills the index over 100,000 existing Person nodes.

### Day 2: Add New Property

Client registers Person schema v2 with new property `start_date` added.

No automatic index. Client can optionally request one:
```
CreateIndexRequest { entity_type: "Person", property: "start_date", type: "range" }
```
Background job backfills.

### Day 30: Remove Old Property

Client realizes `salary` is no longer used. Registers Person schema v3 without `salary`.

Existing nodes still have `salary` in CBOR (invisible to queries). Index entries still exist (orphaned).

Client drops the index:
```
DropIndexRequest { entity_type: "Person", property: "salary" }
```

Old nodes still have salary in CBOR, but it's not indexed. Client can leave it as-is (dead weight) or clean it up:
```
StripPropertyRequest { entity_type: "Person", property: "salary" }
```
Background job rewrites all nodes to remove salary from CBOR.

### Queries

Before `salary` was removed, queries worked:
```cel
salary >= 100000 && department == "Finance"
```

After removal, the same query fails at type-check:
```
ERR_UNKNOWN_FIELD: "salary"
```

Client updates their query logic to not reference `salary`.

---

## 16. Conclusion

Property removal is simple: define what's in the schema, and the system enforces it. Old data is left alone. Cleanup is optional. No state machine. No migration machinery.

The client owns the schema and the evolution timeline. The system provides clear semantics: in schema = queryable; not in schema = not queryable. Clients decide when and how to clean up.

This is dramatically simpler than the three-state deprecation model and better aligns with how real systems (MongoDB, Cassandra) handle schema evolution.

---

*Research document complete. Ready for implementation.*
