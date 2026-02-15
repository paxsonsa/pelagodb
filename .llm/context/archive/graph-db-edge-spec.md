# Graph Database Engine: Edge Management Specification

**Status:** Draft v0.1 — Edge Management
**Author:** Andrew + Claude
**Date:** 2026-02-10

---

## Overview

This document is a companion to the main Graph Database Engine spec (v0.3). It provides a comprehensive specification for edge operations, storage, replication, and lifecycle management. Where sections reference the main spec, they reference "Section N" of `graph-db-spec-v0.3.md`.

The main spec covers the high-level architecture, user stories, schema definition, CEL query pipeline, FDB subspace layout, CDC + background jobs, watches, and multi-site replication. This edge spec focuses specifically on:

- Edge types and schema declarations
- Edge storage model (FDB key layout, paired bidirectional edges)
- Edge creation, update, deletion workflows
- Edge querying and traversal semantics
- Edge ownership in multi-site scenarios
- Orphaned edge detection and cleanup

**Key design decisions summarized:**

1. **Bidirectional edges are implemented as paired directed edges**, each with unambiguous ownership (resolves gap C3 from main spec).
2. **Orphaned edge cleanup** uses lazy deletion on traversal + background scan for post-entity-type-drop cleanup (resolves gap C5).
3. **Edge ownership defaults to source node's home site**, with opt-in independent ownership override.

---

## 1. Edge Types and Schema Declaration

### 1.1 Declared Edges

Entity schema may declare edge types with target type constraints. Example from main spec Section 4:

```json
{
  "entity": "Person",
  "edges": {
    "WORKS_AT": {
      "target": "Company",
      "direction": "outgoing",
      "properties": {
        "role": { "type": "string", "index": "equality" },
        "started": { "type": "timestamp", "sort_key": true }
      }
    }
  }
}
```

| Property | Meaning |
|---|---|
| `target` | Target entity type. Can be `"*"` for polymorphic edges (any entity type). Forward references allowed (target type need not exist at schema registration). |
| `direction` | `"outgoing"`, `"incoming"`, or `"bidirectional"`. Default: `"outgoing"`. |
| `properties` | Optional property schema for the edge. Validated on creation. |
| `ownership` | `"source_site"` (default) or `"independent"`. See Section 7. |
| `sort_key` | Mark one property as the sort key for vertex-centric range scans (details in Section 6.2). |

### 1.2 Undeclared Edges

If entity schema has `allow_undeclared_edges: true` at the meta level, nodes can create edges to types and with properties not explicitly declared in the schema.

```json
{
  "entity": "Person",
  "meta": {
    "allow_undeclared_edges": true
  }
}
```

Rules:

| Scenario | Behavior |
|---|---|
| Create declared edge type | Validate properties against declared schema. |
| Create undeclared edge type (allowed) | No property validation. Target type must still be registered (referential integrity). |
| Create undeclared edge type (not allowed) | REJECT with error. |

Undeclared edges are useful for exploratory graph patterns but complicate query optimization (no schema for CEL type-checking).

### 1.3 Polymorphic Edges

An edge declared with `target: "*"` can point to any entity type.

```json
{
  "edges": {
    "TAGGED_WITH": {
      "target": "*",
      "direction": "outgoing",
      "properties": {
        "relevance": { "type": "float" }
      }
    }
  }
}
```

Storage encodes the target type in the edge key (details in Section 2). Traversal resolves target type from the edge, loads the appropriate schema, and applies type-specific filters (US-11, main spec Section 3.4).

### 1.4 Edge Direction Semantics

| Direction | Meaning | Storage |
|---|---|---|
| `"outgoing"` | Edge is stored at source node only. Traversal with `direction=OUT` returns outgoing edges. | Single entry per edge in source's edge subspace. |
| `"incoming"` | Edge is stored at target node only. Traversal with `direction=IN` returns incoming edges. | Single entry per edge in target's edge subspace. |
| `"bidirectional"` | Edge exists in both directions. Schema declares once; implementation uses paired directed edges. | Two linked entries: one at source (OUT), one at target (IN). Both share `pair_id` for lifecycle management. |

**Bidirectional edges (detailed below in Section 2.3):**

A single `CreateEdge` call for a bidirectional edge creates two paired directed edges:

```
Person_42 --[KNOWS]--> Person_99   [pair_id="pair_001"]
Person_99 --[KNOWS]<-- Person_42   [pair_id="pair_001"]
```

The pair_id ties them together: deleting one triggers deletion of the pair. Each direction is owned by its source node's home site (see Section 7 for multi-site ownership).

---

## 2. Edge Storage Model

### 2.1 FDB Key Layout

From main spec Section 6, edges are stored in the entity's edge subspace. Full key path:

```
(namespace)
  (entities)
    ({entity_type})
      (edges)
        ({edge_type}, {direction}, {sort_key}, {target_type}, {target_id}, {edge_id})
```

Breaking down each component:

| Component | Type | Purpose |
|---|---|---|
| `namespace` | string | Logical database. |
| `entity_type` | string | Source node's entity type. |
| `edge_type` | string | E.g., "WORKS_AT", "KNOWS". User-declared or undeclared. |
| `direction` | int | 0 for OUT, 1 for IN. Enables prefix scans by direction. |
| `sort_key` | value | Optional. If edge type has a sort_key property (e.g., timestamp), encoded here for range scans. Otherwise, constant (e.g., 0). |
| `target_type` | string | Target entity type (for polymorphic edge resolution). |
| `target_id` | string | Target node ID. |
| `edge_id` | string | Globally unique edge identifier. |

**Key examples:**

```
Outgoing edge (OUT):
  ("myapp", "entities", "Person", "edges", "WORKS_AT", 0, 1705363200, "Company", "c_7", "e_101")

Incoming edge (IN, at target):
  ("myapp", "entities", "Company", "c_7", "edges", "WORKS_AT", 1, 1705363200, "Person", "p_42", "e_101")

Bidirectional edge (both directions share pair_id):
  ("myapp", "entities", "Person", "edges", "KNOWS", 0, <sort_key>, "Person", "p_99", "e_001_out")
  ("myapp", "entities", "Person", "edges", "KNOWS", 1, <sort_key>, "Person", "p_42", "e_001_in")
```

**Order preservation:**

- Edges within an entity type are ordered by: edge_type → direction → sort_key → target_type → target_id.
- This enables prefix scans (all outgoing WORKS_AT edges) and range scans on sort_key (all WORKS_AT edges started after 2020-01-01).

### 2.2 Edge Data Serialization

Edge values are stored in CBOR format (same as node properties, main spec Section 6). The value includes:

```rust
struct EdgeData {
    properties: HashMap<String, Value>,   // User-defined properties (role, started, etc.)
    _owner: String,                       // Owning site (default: source node's home_site)
    _pair_id: Option<String>,             // For bidirectional edges: ID linking to paired direction
    _created_at: i64,                     // Timestamp (unix microseconds)
}
```

Example CBOR serialization:

```
Edge key: ("myapp", "entities", "Person", "edges", "WORKS_AT", 0, 1705363200, "Company", "c_7", "e_101")
Edge value (CBOR):
{
  "properties": {
    "role": "Engineer",
    "started": 1705363200
  },
  "_owner": "site_a",
  "_created_at": 1705363200000000
}
```

### 2.3 Paired Bidirectional Edge Storage

Bidirectional edges are stored as TWO directed edges, each owned by its source node's home site.

**Single-site scenario (both nodes homed at same site):**

CreateEdge call:
```
CreateEdge(
  source: Person_42 (site_a),
  target: Person_99 (site_a),
  edge_type: "KNOWS",
  direction: "bidirectional",
  properties: { "since": 1705363200 }
)
```

Single FDB transaction writes:

```
Key 1 (outgoing at Person_42):
  ("myapp", "entities", "Person", "edges", "KNOWS", 0, 1705363200, "Person", "p_99", "e_bid_001_out")
  Value: CBOR { properties: {...}, _owner: "site_a", _pair_id: "e_bid_001" }

Key 2 (incoming at Person_99):
  ("myapp", "entities", "Person", "edges", "KNOWS", 1, 1705363200, "Person", "p_42", "e_bid_001_in")
  Value: CBOR { properties: {...}, _owner: "site_a", _pair_id: "e_bid_001" }
```

Both edges share `_pair_id = "e_bid_001"` and have the same owner. Deletion of either edge triggers deletion of both (see Section 5.2).

**Multi-site scenario (nodes homed at different sites):**

CreateEdge(
  source: Person_42 (site_a),
  target: Person_99 (site_b),
  edge_type: "KNOWS"
)

Flow:

1. **Site A** (source node's home):
   - Creates outgoing edge at Person_42 immediately in its transaction.
   - Appends CDC entry: EdgeCreate with pair_id and target node reference.

2. **CDC consumer at Site A**:
   - Streams CDC entry to Site B with pair_id and the reverse edge requirement signaled.

3. **Site B** (target node's home):
   - Receives CDC entry, verifies Person_99 exists locally.
   - Creates the incoming edge at Person_99 with same pair_id.
   - Appends its own CDC entry for replication back to Site A (optional, for confirmation).

This ensures both directions are owned by their respective source nodes' home sites, and both have the same pair_id for lifecycle management.

### 2.4 Edge ID Generation

Edge IDs must be globally unique within a namespace. Options:

| Strategy | Pros | Cons |
|---|---|---|
| UUID v4 | No coordination, 16 bytes | No temporal locality |
| UUID v7 (or ULID) | No coordination, 16 bytes, time-ordered | Large keys |
| Sequential int64 | Small (8 bytes varint), temporal | Requires counter, coordination |

**Recommendation:** UUID v7 (16 bytes, time-ordered). Accept the larger key size for coordinate-free ID generation.

For paired bidirectional edges, the pair_id can be a shorter token derived from the edge IDs or generated independently. Example: `_pair_id = hash(min(edge_id_1, edge_id_2))`.

---

## 3. Edge Creation

### 3.1 Validation Steps

Edge creation workflow:

```
CreateEdge request arrives
    │
    ▼
┌─────────────────────────────────────────┐
│ 1. Validate source node exists          │
│    (must exist locally or be replicated) │
└──────────────────┬──────────────────────┘
    │
    ▼
┌─────────────────────────────────────────┐
│ 2. Validate target node exists          │
│    (target must exist before edge)      │
└──────────────────┬──────────────────────┘
    │
    ▼
┌─────────────────────────────────────────┐
│ 3. Validate edge type in schema         │
│    (if declared)                        │
│    - target type matches declaration    │
│    - properties match schema             │
└──────────────────┬──────────────────────┘
    │
    ▼
┌─────────────────────────────────────────┐
│ 4. Validate ownership (multi-site)      │
│    - source node must be owned locally  │
│      (or allow replication-driven       │
│       edge creation via CDC)            │
└──────────────────┬──────────────────────┘
    │
    ▼
┌─────────────────────────────────────────┐
│ 5. Single FDB Transaction:              │
│    - Write OUT entry (at source)        │
│    - Write IN entry (at target)         │
│    - If bidirectional: generate pair_id │
│    - Append CDC entry                   │
└──────────────────┬──────────────────────┘
    │
    ▼
   ACK to client
```

### 3.2 Referential Integrity

Both source and target nodes must exist at creation time.

| Scenario | Behavior |
|---|---|
| Source exists, target missing | REJECT: `TargetNotFound` |
| Source missing, target exists | REJECT: `SourceNotFound` |
| Both exist, different namespaces | REJECT: `NamespaceMismatch` |
| Edge type declared, target type mismatch | REJECT: `TargetTypeMismatch` |

**Multi-site edge creation (target on different site):**

When creating an edge where the source and target nodes are owned by different sites, only the source site's edge creation succeeds immediately. The target site's edge (if direction is bidirectional or if explicitly replicating incoming edges) arrives via CDC:

```
Site A (source):
  Write OUT edge → CDC entry
         │
         └──> Replicate to Site B
              │
              ▼ (Site B's CDC consumer)
              Create IN edge at target
```

The CDC consumer at Site B checks: does the target node exist? If not, queue in dependency buffer (main spec Section 10, "Causal Ordering for Edges"). Once target arrives, drain buffer and apply queued edge writes.

### 3.3 Bidirectional Edge Creation

For a declared bidirectional edge:

```json
{
  "edges": {
    "KNOWS": {
      "target": "Person",
      "direction": "bidirectional"
    }
  }
}
```

**Single-transaction write path (both nodes at same site):**

```rust
fn create_bidirectional_edge(
    fdb_txn: &mut FdbTransaction,
    source: NodeRef,
    target: NodeRef,
    edge_type: &str,
    properties: &Map<String, Value>,
) -> Result<String> {
    // Validate both nodes exist
    assert_node_exists(source)?;
    assert_node_exists(target)?;

    // Generate IDs
    let pair_id = generate_pair_id();
    let edge_id_out = format!("{}_out", pair_id);
    let edge_id_in = format!("{}_in", pair_id);

    // Write outgoing edge at source
    let out_key = make_edge_key(source.type, source.id, edge_type, OUT, target.type, target.id, edge_id_out);
    let out_value = EdgeData {
        properties: properties.clone(),
        _owner: source.home_site.clone(),
        _pair_id: Some(pair_id.clone()),
        _created_at: now_micros(),
    };
    fdb_txn.set(out_key, encode_cbor(&out_value));

    // Write incoming edge at target
    let in_key = make_edge_key(target.type, target.id, edge_type, IN, source.type, source.id, edge_id_in);
    let in_value = EdgeData {
        properties: properties.clone(),
        _owner: source.home_site.clone(),  // Same owner as OUT
        _pair_id: Some(pair_id.clone()),
        _created_at: now_micros(),
    };
    fdb_txn.set(in_key, encode_cbor(&in_value));

    // Append CDC entry
    append_cdc_entry(fdb_txn, CdcOperation::EdgeCreate {
        source: source.clone(),
        target: target.clone(),
        edge_type: edge_type.to_string(),
        pair_id: Some(pair_id.clone()),
        properties: properties.clone(),
        owner_site: source.home_site.clone(),
    });

    Ok(pair_id)
}
```

**Multi-site flow:**

If source and target are at different sites:

1. Source site writes OUT edge in transaction + CDC entry.
2. CDC entry includes `pair_id` and signals remote site to create reverse edge.
3. Target site's CDC consumer receives the entry and creates IN edge with same `pair_id`.

### 3.4 CDC Entry for Edge Creation

From main spec Section 7, edge creation appends a CDC entry:

```rust
CdcOperation::EdgeCreate {
    source: NodeRef {
        entity_type: "Person",
        node_id: "p_42",
        home_site: "site_a",
    },
    target: NodeRef {
        entity_type: "Company",
        node_id: "c_7",
        home_site: "site_b",
    },
    edge_type: "WORKS_AT",
    edge_id: "e_101",
    pair_id: None,  // None for directed edges; Some(pair_id) for bidirectional
    properties: {"role": "Engineer", "started": 1705363200},
    owner_site: "site_a",  // Default: source's home site
}
```

---

## 4. Edge Updates

### 4.1 Which Properties Can Be Updated

Only edge properties (not the edge structure itself) can be updated. Updating the source, target, edge type, or direction is unsupported — delete and recreate instead.

Updatable properties are those declared in the edge schema with no special flags. Example:

```json
{
  "edges": {
    "WORKS_AT": {
      "properties": {
        "role": { "type": "string", "updatable": true },
        "started": { "type": "timestamp" }
      }
    }
  }
}
```

| Property | Updatable? |
|---|---|
| `role` | Yes (declared, no restrictions) |
| `started` | Yes (declared, no restrictions) |
| `_owner` | No (metadata) |
| `_pair_id` | No (metadata) |
| `_created_at` | No (metadata) |

### 4.2 Ownership Check

Edge update must be authorized by the owning site.

| Ownership | Update authorization |
|---|---|
| `source_site` (default) | Only source node's home site can update. |
| `independent` | Only the site that created the edge can update. |

For bidirectional edges, both directions have the same owner (source node's home site), so update is authorized from one site.

### 4.3 Update Semantics

Update path:

```
UpdateEdgeProperties request
    │
    ├─ Identify edge by (source, target, edge_type, sort_key if applicable)
    │
    ├─ Validate ownership
    │  (if multi-site, ensure requester is owning site)
    │
    ├─ Single FDB Transaction:
    │  ├─ Read current edge properties
    │  ├─ Merge with new properties
    │  ├─ Update OUT entry
    │  ├─ Update IN entry (if direction is IN or bidirectional)
    │  └─ Append CDC entry (EdgeUpdate)
    │
    └─> ACK with new properties
```

For bidirectional edges, updating one direction's properties should update both (they share the same properties in storage). Implementation: update both OUT and IN entries in the same transaction.

### 4.4 CDC Entry for Edge Update

```rust
CdcOperation::EdgeUpdate {
    edge_id: "e_101",
    changed_properties: {"role": "Senior Engineer"},
    previous_version: 1,
}
```

CDC consumer uses edge_id to locate and apply the update at remote sites (replication respects ownership).

---

## 5. Edge Deletion

### 5.1 Single Edge Deletion

To delete an edge, identify it by (source, target, edge_type, sort_key). Single FDB transaction:

```rust
fn delete_edge(
    fdb_txn: &mut FdbTransaction,
    source: NodeRef,
    target: NodeRef,
    edge_type: &str,
) -> Result<()> {
    // Lookup edge to get edge_id and metadata
    let out_key = make_edge_key(source.type, source.id, edge_type, OUT, target.type, target.id, "*");
    let edge = fdb_txn.get_range(out_key, LIMIT=1)?;  // Prefix scan for the edge

    if edge.is_empty() {
        return Err("EdgeNotFound");
    }

    let (out_key, out_value) = edge[0];
    let edge_data: EdgeData = decode_cbor(&out_value)?;
    let edge_id = extract_edge_id_from_key(out_key);

    // Verify ownership
    assert_same_site(&edge_data._owner)?;

    // Delete OUT entry
    fdb_txn.delete(&out_key);

    // Delete IN entry (at target node)
    let in_key = make_edge_key(target.type, target.id, edge_type, IN, source.type, source.id, edge_id);
    fdb_txn.delete(&in_key);

    // If bidirectional (pair_id present), delete the pair
    if let Some(pair_id) = &edge_data._pair_id {
        // Pair should be: source <-> target both directions
        // If we're deleting the OUT edge, also delete the IN edge at target
        // (both already deleted above for single directed edge)
        // For true bidirectional, this is handled in section 5.2
    }

    // Append CDC entry
    append_cdc_entry(fdb_txn, CdcOperation::EdgeDelete {
        edge_id: edge_id.clone(),
        pair_id: edge_data._pair_id.clone(),
    });

    Ok(())
}
```

### 5.2 Paired Bidirectional Deletion

For a bidirectional edge with pair_id, deleting one direction triggers deletion of the pair:

```
Delete OUT edge at Person_42 with pair_id="pair_001"
    │
    ├─ Delete OUT entry at Person_42
    │
    ├─ Delete IN entry at Person_99 (if at same site)
    │
    └─ CDC entry includes pair_id and delete signal
         │
         └─> Remote site's CDC consumer:
             Delete the IN entry at Person_99 (if at different site)
```

**Single-site deletion:**

Both edges deleted in same FDB transaction:

```rust
fn delete_bidirectional_edge(
    fdb_txn: &mut FdbTransaction,
    source: NodeRef,
    target: NodeRef,
    edge_type: &str,
) -> Result<()> {
    // ... lookup and verify ownership ...

    let pair_id = edge_data._pair_id.unwrap();

    // Delete both directions in same transaction
    fdb_txn.delete(&out_key);  // OUT at source
    fdb_txn.delete(&in_key);   // IN at target

    // Append CDC entry with pair_id
    append_cdc_entry(fdb_txn, CdcOperation::EdgeDelete {
        edge_id: edge_id.clone(),
        pair_id: Some(pair_id.clone()),
    });

    Ok(())
}
```

**Multi-site deletion:**

When deleting a bidirectional edge where source and target are at different sites:

1. Source site deletes OUT edge, appends CDC entry with pair_id.
2. CDC consumer at target site reads entry, finds matching IN edge by pair_id, deletes it.

The CDC entry includes the pair_id, allowing the remote site to unambiguously find and delete the reverse edge.

### 5.3 Node Deletion Cascade

When a node is deleted, all edges incident to it must be deleted:

- **Outgoing edges**: Scan `(ns, entities, {entity_type}, edges, *, OUT, *, *, *, *)` — find all edges from this node. Delete corresponding IN entries at targets.
- **Incoming edges**: Scan `(ns, entities, {entity_type}, edges, *, IN, *, *, *, *)` — find all edges to this node. Delete corresponding OUT entries at sources.

This is O(degree of node). For high-degree nodes, it's a background job (not inline with node deletion).

**Protocol:**

```
DeleteNode request arrives
    │
    ▼
┌─────────────────────────────────┐
│ Inline: delete node data        │
│         delete node indexes     │
│         append CDC entry        │
└──────────────┬──────────────────┘
    │
    ▼
┌──────────────────────────────────────┐
│ Background job:                      │
│  1. Scan all incoming edges          │
│  2. Delete corresponding OUT entries │
│     at source nodes                  │
│  3. Append CDC for each deletion     │
└──────────────────────────────────────┘
```

For multi-site:

- Source site deletes node, scans its outgoing edges, appends CDC entry (NodeDelete).
- CDC consumer at target sites applies NodeDelete. After receiving it, they can clean up incoming edges lazily or via a background job.

Alternative (eager cleanup): On receiving a NodeDelete via CDC, immediately scan and delete all incoming edges in a background job. This ensures no orphans.

### 5.4 Entity Type Drop

When an entire entity type is dropped (via `clearRange`), all nodes and outgoing edges are removed. However, incoming edges from other entity types are orphaned.

**Cleanup strategy (resolves gap C5 from main spec):**

1. **Lazy cleanup on traversal**: When traversal encounters an edge pointing to a deleted node, detect it and delete the edge inline.
2. **Background job**: After entity type drop, schedule OrphanedEdgeCleanup job that scans all other entity types' edge subspaces for edges targeting the dropped type, and deletes them.

```rust
enum BackgroundJob {
    // ... from main spec Section 7
    OrphanedEdgeCleanup {
        dropped_entity_type: String,
    },
}

fn orphaned_edge_cleanup_worker(
    dropped_type: &str,
) {
    // Scan all entity types
    for entity_type in all_entity_types() {
        if entity_type == dropped_type {
            continue;  // Already cleared
        }

        // Scan edges in this type's edge subspace
        let prefix = (ns, "entities", entity_type, "edges");
        let mut cursor = prefix.clone();

        loop {
            let batch = scan_edges_from(cursor, BATCH_SIZE);
            if batch.is_empty() {
                break;
            }

            for edge_key in batch {
                let target_type = extract_target_type(edge_key);
                if target_type == dropped_type {
                    // This edge points to a dropped type
                    // Verify target doesn't exist
                    let target_id = extract_target_id(edge_key);
                    if !node_exists(dropped_type, target_id) {
                        fdb_txn.delete(edge_key);  // Delete the edge
                    }
                }
            }

            cursor = batch.last();
        }
    }
}
```

This is O(all edges in database) but runs in the background. Entity type drops are rare operations; not worth adding a reverse index to every edge write.

---

## 6. Edge Querying and Traversal

### 6.1 Adjacency List Scan

Retrieve all edges of a specific type from a node:

```
Query: all WORKS_AT edges from Person_42
    │
    ▼
Prefix scan: (ns, entities, Person, edges, WORKS_AT, OUT, *, *, *)
    │
    ├─ Prefix is: ("myapp", "entities", "Person", "edges", "WORKS_AT", 0)
    │
    ▼
Returns all matching (source, target, edge_id, edge_data) tuples
```

FDB range scan with prefix:

```rust
fn get_outgoing_edges(
    fdb_txn: &FdbTransaction,
    source: NodeRef,
    edge_type: &str,
) -> Result<Vec<Edge>> {
    let prefix = (ns, "entities", source.type, "edges", edge_type, 0);  // 0 = OUT
    let range = fdb_txn.get_range(prefix_range(prefix), LIMIT)?;

    range.into_iter().map(|kv| {
        let target_type = extract_target_type(kv.key);
        let target_id = extract_target_id(kv.key);
        let edge_id = extract_edge_id(kv.key);
        let edge_data = decode_cbor(&kv.value)?;

        Ok(Edge {
            source_id: source.id.clone(),
            target_id,
            target_type,
            edge_type: edge_type.to_string(),
            edge_id,
            properties: edge_data.properties,
        })
    }).collect()
}
```

Complexity: O(number of edges of that type from the node) + FDB network roundtrip.

### 6.2 Vertex-Centric Indexes (Sort Key Range Scans)

If an edge type declares a sort_key property, range predicates on that property become subspace range scans.

**Schema example:**

```json
{
  "edges": {
    "WORKS_AT": {
      "properties": {
        "started": { "type": "timestamp", "sort_key": true }
      }
    }
  }
}
```

**Query example:**

```
Find all WORKS_AT edges from Person_42 where started >= 2020-01-01
    │
    ▼
Encoded timestamp for 2020-01-01: 1577836800000000
    │
    ▼
Range scan: (ns, entities, Person, edges, WORKS_AT, 0, [1577836800000000, MAX])
    │
    ├─ Prefix is: ("myapp", "entities", "Person", "edges", "WORKS_AT", 0)
    ├─ Lower bound: ("myapp", "entities", "Person", "edges", "WORKS_AT", 0, 1577836800000000)
    ├─ Upper bound: ("myapp", "entities", "Person", "edges", "WORKS_AT", 0, MAX)
    │
    ▼
Returns edges in temporal order
```

**CEL compilation for sort_key:**

From main spec Section 5, edge filters compile against the edge schema. If the CEL predicate is on the sort_key field:

```
CEL: "started >= timestamp('2020-01-01')"
Edge type: WORKS_AT with properties { started: timestamp }
Sort key: "started"
    │
    ▼
Compile to: RangeScan(started, >=, 1577836800000000)
    │
    ▼
FDB operation: range scan within edge subspace, keyed by sort_key
```

This is the "vertex-centric index" pattern from JanusGraph — efficiently supporting range queries on edges without separate index structures.

### 6.3 Edge Property Filtering via CEL

Edge filters are applied during traversal (see US-10, main spec Section 3.4). The CEL expression compiles against the edge schema:

```
Edge filter: "role == 'Engineer' && started > timestamp('2020-01-01')"
Edge type: WORKS_AT { role: string, started: timestamp }
CEL environment: { role: String, started: Timestamp }
    │
    ▼
Decompose:
  - [role == 'Engineer']: equality predicate, no index → residual
  - [started > ...]: sort_key predicate → RangeScan
    │
    ▼
Execution:
  1. Range scan edges by sort_key (started > 2020-01-01)
  2. For each candidate, evaluate residual filter (role == 'Engineer')
  3. Return matching edges
```

If no sort_key matches the predicate, the query reverts to: adjacency list scan + residual filtering (all edges, in-memory CEL eval).

### 6.4 Polymorphic Edge Traversal

Traversing a polymorphic edge (target: "*") requires resolving the target type from the edge key:

```
Traverse: TAGGED_WITH -> * with filter "price > 100"
    │
    ├─ Start from source node
    │
    ├─ Scan all TAGGED_WITH edges (all target types)
    │
    └─ For each edge:
       ├─ Extract target_type from edge key
       ├─ Load target node schema
       ├─ Evaluate filter "price > 100" against target type
       │  (if "price" not in target schema, exclude)
       └─ Return matching target node
```

No optimization for polymorphic edges (see gap I3 in main spec). Each traversal requires per-edge schema lookup and evaluation. This is acceptable for low-degree polymorphic edges but can be slow for high-degree patterns.

### 6.5 Multi-Hop Traversal with Per-Hop Filtering

From US-10 (main spec):

```
TraverseRequest {
  start: Person_42,
  steps: [
    { edge_type: "KNOWS", direction: OUT, edge_filter: "strength > 0.8", node_filter: "age >= 30" },
    { edge_type: "WORKS_AT", direction: OUT, edge_filter: "nil", node_filter: "name.startsWith('Tech')" }
  ],
  max_depth: 2,
  max_results: 100,
  timeout_ms: 5000
}
```

Execution:

```
Hop 1: From Person_42, traverse KNOWS edges
    │
    ├─ Adjacency scan: get_outgoing_edges(Person_42, "KNOWS")
    ├─ Filter edges: CEL "strength > 0.8"
    ├─ For each matching edge, load target node
    ├─ Filter nodes: CEL "age >= 30"
    └─ Collect results (persons known with strength > 0.8, age >= 30)
       │
       └─> Stream to client

Hop 2: From each result of Hop 1, traverse WORKS_AT edges
    │
    ├─ For each person from Hop 1:
    │  ├─ Adjacency scan: get_outgoing_edges(person_id, "WORKS_AT")
    │  ├─ Load target nodes (companies)
    │  ├─ Filter nodes: CEL "name.startsWith('Tech')"
    │  └─ Stream to client
    │
    └─ Stop when depth >= max_depth or results >= max_results
```

Streaming results with backpressure: results from Hop 1 are streamed back to client immediately; client processes and sends next batch request for Hop 2 (or all at once if client implements full pipeline).

**Depth and traversal limits (from main spec Section 9):**

- `max_depth`: Stop traversal if depth > max_depth. Default: 4, server maximum enforced.
- `timeout_ms`: Cancel traversal if elapsed time > timeout_ms. Default: 5000ms.
- `max_results`: Stop streaming when total results >= max_results. Return cursor for pagination.

---

## 7. Edge Ownership and Multi-Site Replication

### 7.1 Default Ownership Rules

From main spec Section 10, edges have owners:

| Edge Type | Ownership | Rationale |
|---|---|---|
| Directed (OUT/IN) | Source node's home site | Source site is authoritative for outgoing relationships. |
| Bidirectional (paired) | Source node's home site (for OUT direction) | Each direction owned by its source. Single-site creates both; multi-site: each site owns its outgoing. |
| Independent (opt-in) | Creating site | Edge is owned by the site that created it, not by source node's home site. Useful for metadata edges not tied to source's authority. |

### 7.2 Schema Override: Independent Ownership

Edge schema can declare `ownership: "independent"`:

```json
{
  "edges": {
    "ASSIGNED_TO": {
      "target": "User",
      "ownership": "independent"
    }
  }
}
```

When independent ownership is declared:

- Edge is owned by the creating site, not the source node's home site.
- Only the creating site can modify or delete the edge.
- CDC replication respects ownership (only the owning site's CDC entries are applied).

Example: Task (owned by site A) is ASSIGNED_TO a User (owned by site B). The assignment edge is created by an admin site C. It's useful for site C to own the assignment (e.g., assignment can be changed independently of task or user ownership). Set `ownership: "independent"` on ASSIGNED_TO.

### 7.3 Bidirectional Ownership in Multi-Site

For a bidirectional edge spanning two sites:

```
Person_42 (site_a) --[KNOWS]-- Person_99 (site_b)
```

Each direction is owned by its source:

- Outgoing edge at Person_42: owned by site_a (Person_42's home)
- Incoming edge at Person_99: owned by site_b (Person_99's home)

Each site can modify its outgoing edge independently. If site_a wants to update edge properties (strength, since), it updates its OUT edge and appends CDC. site_b receives the CDC and applies it to its IN edge (both share the same properties in CBOR).

**Question: Should properties be consistent?**

Currently, the design assumes both directions have the same properties (updated together). If site_a updates properties, those changes propagate to site_b's IN edge via CDC. If site_b concurrently updates, there's potential for a conflict (last-writer-wins by versionstamp, or merge strategy TBD).

**Recommendation:** For now, require that property updates always come from the source node's home site (site_a in this example). If site_b needs to update edge properties, it sends a request to site_a, which handles the update. This avoids concurrency issues.

### 7.4 CDC Replication of Edge Operations

CDC entries include ownership info:

```rust
CdcOperation::EdgeCreate {
    source: NodeRef { ..., home_site: "site_a" },
    target: NodeRef { ..., home_site: "site_b" },
    edge_type: "KNOWS",
    edge_id: "e_001",
    pair_id: Some("pair_001"),
    properties: {...},
    owner_site: "site_a",  // Who owns the edge
}
```

Receiving site's CDC consumer checks ownership:

```
Receive CDC entry: EdgeCreate(..., owner_site="site_a")
    │
    ├─ If local site == owner_site:
    │  └─ Apply the edge write (create/update)
    │
    └─ Else:
       └─ Ignore (owner is remote; we'll receive a replication of their edge)
```

For bidirectional edges:

- `owner_site` points to the source node's home site.
- Both directions are replicated together.
- Receiving site's CDC consumer knows it's a bidirectional pair (from `pair_id`) and applies both directions with the same owner.

### 7.5 Causal Ordering: Edge Arrives Before Target Node

From main spec Section 10, CDC replication must respect causal ordering:

```
Event 1: Create Company_7 at site_b → CDC entry
Event 2: Create edge Person_42 -> Company_7 at site_a → CDC entry

Site_a's CDC: [Event 2 (depends on Event 1)]
Replicate to site_a:
    │
    ├─ Event 2 arrives at site_a first
    ├─ Check: does Company_7 exist locally?
    ├─ NO → queue in dependency buffer
    │
    └─> Event 1 arrives
        ├─ Create Company_7 at site_a
        ├─ Drain dependency buffer: apply queued Event 2
        └─ Edge is now created
```

**Dependency buffer implementation:**

```rust
struct CdcConsumer {
    dependency_buffer: Map<NodeRef, Vec<CdcOperation>>,  // node -> queued ops
}

fn apply_cdc_entry(entry: CdcEntry) {
    for op in entry.operations {
        match op {
            CdcOperation::EdgeCreate { source, target, ... } => {
                if !node_exists(&target) {
                    // Buffer the operation
                    dependency_buffer[target].push(op);
                } else {
                    // Apply immediately
                    apply_edge_create(op);
                }
            }
            CdcOperation::NodeCreate { node_ref, ... } => {
                apply_node_create(op);

                // Drain dependency buffer for this node
                if let Some(queued) = dependency_buffer.remove(&node_ref) {
                    for queued_op in queued {
                        apply_edge_create(queued_op);
                    }
                }
            }
            // ...
        }
    }
}
```

The dependency buffer ensures no orphaned edges: an edge is only applied after its target node exists.

---

## 8. Edge Lifecycle and Consistency

### 8.1 Orphaned Edge Detection

An edge is orphaned if its target node no longer exists. This can occur:

1. Entity type is dropped: outgoing edges cleared, incoming edges orphaned.
2. Individual node is deleted: edges point to non-existent node.
3. Bug or data corruption: edge data references deleted node.

### 8.2 Lazy Cleanup on Traversal

During traversal, if an edge points to a non-existent target, detect and delete it:

```rust
fn traverse_adjacency_list(
    fdb_txn: &FdbTransaction,
    source: NodeRef,
    edge_type: &str,
) -> Result<Vec<Edge>> {
    let edges = get_outgoing_edges(source, edge_type)?;

    let mut valid_edges = Vec::new();
    for edge in edges {
        // Check if target exists
        if !node_exists(&edge.target_type, &edge.target_id) {
            // Orphaned edge: delete it lazily
            // (In a separate transaction, or batch with other deletions)
            mark_edge_for_deletion(edge.edge_id);
        } else {
            valid_edges.push(edge);
        }
    }

    Ok(valid_edges)
}
```

This is O(edges in adjacency list) + O(target node lookups). For high-degree nodes with many orphaned edges, it's slow. But orphaned edges should be rare in normal operation.

### 8.3 Background Job for Post-Entity-Type-Drop Cleanup

After dropping an entity type, schedule an OrphanedEdgeCleanup background job (see Section 5.4). This scans other entity types' edge subspaces and deletes edges targeting the dropped type.

Job state in FDB:

```
("myapp", "_jobs", "OrphanedEdgeCleanup", "Person_drop_20260208_001")
  → CBOR {
      status: "Running",
      progress_cursor: ("myapp", "entities", "Company", "edges"),
      created_at: 1707432000000000,
      dropped_entity_type: "Person"
    }
```

Worker loop:

```rust
loop {
    let job = get_job_by_id(job_id);
    let cursor = job.progress_cursor.clone();
    let batch = scan_edges_from(cursor, BATCH_SIZE);

    if batch.is_empty() {
        mark_job_completed(job_id);
        break;
    }

    for edge_key in batch {
        let target_type = extract_target_type(edge_key);
        if target_type == job.dropped_entity_type {
            // Delete orphaned edge
            fdb_txn.delete(edge_key);
            // Append CDC entry for replication
            append_cdc_entry(fdb_txn, CdcOperation::EdgeDelete { ... });
        }
    }

    update_job_cursor(job_id, batch.last());
}
```

This is O(all edges) but distributed over time and doesn't block foreground operations.

### 8.4 Edge Consistency Guarantees in Multi-Site

| Guarantee | Status | Notes |
|---|---|---|
| Edges are replicated | Yes | Via CDC, all edges eventually replicate to other sites. |
| Both directions of bidirectional edge replicate together | Yes | Pair_id ties them together; same CDC entry triggers both. |
| Causal ordering (edge applied after target node) | Yes | Dependency buffer ensures this. |
| No orphaned edges (eventually) | Yes | Lazy cleanup on traversal + background job cleans up post-drop. |
| Ownership enforced | Yes | Only owning site can update/delete. CDC replication checks ownership. |
| Concurrent updates to same edge | TBD | Depends on owning site coordination. Last-writer-wins by versionstamp if concurrent updates from same site. Conflict if from different sites (but ownership prevents this). |

---

## 9. Gap Analysis: Edge-Specific Issues

### CRITICAL

**E1. Concurrent property updates to bidirectional edges.**

If site_a and site_b both own a direction of a bidirectional edge (site_a owns OUT, site_b owns IN), they could concurrently update properties. Properties are shared (same CBOR in both directions), so conflict resolution is needed.

**Options:**
- Require property updates always from source node's home site (site_a). Simpler, single authority.
- Implement merge logic (e.g., last-writer-wins, or property-level LWW with clocks). Complex.
- Forbid property updates to bidirectional edges; require delete + recreate. Harsh.

**Recommendation:** Require updates from source node's home site. Document clearly.

**E2. Multi-site unique constraint violation.**

Two sites can each create a node with the same unique-indexed value before replication syncs. Violates the unique constraint (gap C7 from main spec). Not edge-specific, but affects edge creation validation (e.g., creating edges to nodes with duplicate unique fields).

**Recommendation:** Defer to main spec gap C7 resolution. Likely: route unique-constrained writes through a single authority site.

### IMPORTANT

**E3. Sort key property is immutable.**

If sort_key encodes part of the edge's FDB key, changing it requires delete + recreate (can't just update properties). Schema should enforce this (mark sort_key as immutable).

**E4. Orphaned edge cleanup is O(all edges).**

Background job scans entire graph to find edges targeting dropped type. For large databases, this is expensive. Future optimization: incoming edge index. But adds write amplification and complexity.

**E5. Polymorphic edge index optimization.**

No index for polymorphic edges (target: "*"). Traversal is O(edges) per node. Acceptable for most workloads, but high-degree polymorphic edges are slow.

### MINOR

**E6. Edge ID generation strategy unfinalized.**

Recommend UUID v7 (time-ordered, no coordination), but not yet decided. Affects key size and temporal locality.

**E7. Pair ID derivation for bidirectional edges.**

Pair_id can be: one of the two edge IDs, a hash of both, or independently generated. Not critical; recommend hash(min(edge_id_1, edge_id_2)) for determinism (useful for testing/debugging).

---

## 10. Summary Table: Edge Operations

| Operation | Storage | Single-Txn | Multi-Txn | Replication | Ownership Check |
|---|---|---|---|---|---|
| Create directed edge | OUT + IN entries | Yes | No | CDC entry | Verify source ownership (if required) |
| Create bidirectional edge | Paired OUT + IN | Yes | No (both sites write separately for multi-site) | CDC with pair_id | Source node's home site |
| Update edge properties | Modify both directions | Yes | No | CDC entry | Owning site only |
| Delete directed edge | Remove OUT + IN | Yes | No | CDC entry | Owning site only |
| Delete bidirectional edge | Remove both directions | Yes | No | CDC with pair_id | Owning site (both directions) |
| Node deletion cascade | Scan incoming/outgoing | No (background job for large degree) | Yes | CDC per deleted edge | N/A |
| Entity type drop | clearRange + background cleanup | No | Yes | CDC per deleted edge | N/A |
| Query adjacency list | Range scan on edge subspace | N/A | Single FDB read | N/A | N/A |
| Traverse with sort_key filter | Range scan + residual | N/A | Single FDB read | N/A | N/A |
| Traversal (multi-hop) | Multiple adjacency scans | N/A | Streaming FDB reads | N/A | N/A |

---

## 11. Implementation Checklist

### Phase 1 (Storage Foundation)

- [ ] FDB key layout for edges (OUT/IN tuples)
- [ ] Edge serialization (CBOR with ownership/pair_id metadata)
- [ ] CreateEdge: validation, referential integrity, single-txn write
- [ ] DeleteEdge: single edge deletion + IN entry cleanup
- [ ] Edge CDC entries (Create, Update, Delete)
- [ ] GetOutgoingEdges / GetIncomingEdges (adjacency list scan)
- [ ] Bidirectional edge creation (paired edges, pair_id generation)

### Phase 2 (Query Engine)

- [ ] CEL compilation for edge filters
- [ ] Sort_key property support (vertex-centric range scans)
- [ ] Polymorphic edge traversal (target type resolution)
- [ ] Multi-hop traversal with per-hop edge/node filters
- [ ] Traversal depth/timeout/result limits (from main spec Section 9)
- [ ] Edge property filtering in traversal

### Phase 3 (Replication)

- [ ] Edge ownership metadata (default + independent)
- [ ] CDC replication of edge operations
- [ ] Dependency buffer for out-of-order CDC entries
- [ ] Multi-site bidirectional edge creation (paired edges across sites)
- [ ] Ownership enforcement in update/delete

### Phase 4 (Lifecycle Management)

- [ ] Node deletion cascade (background job for large degree)
- [ ] Entity type drop with orphaned edge cleanup (background job)
- [ ] Lazy cleanup on traversal (detect + delete orphaned edges)
- [ ] Edge update operations
- [ ] Edge watch semantics (WatchEdges from main spec US-18)

---

## References

- Main spec Section 3.3 (Edge Operations user stories US-7, US-8)
- Main spec Section 3.4 (Multi-hop traversal US-10, US-11)
- Main spec Section 6 (FDB subspace layout for edges)
- Main spec Section 7 (CDC entries for edge operations)
- Main spec Section 10 (Replication and edge ownership)
- Main spec Section 11 (Namespace hierarchy and lifecycle)
- Main spec Section 12 (Gap analysis C3, C5)

---

*This specification is a living document. Updates and clarifications will be incorporated as implementation proceeds and design decisions are validated.*
