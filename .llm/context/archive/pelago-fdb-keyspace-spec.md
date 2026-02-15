# Pelago FDB Keyspace Specification

**Version:** 1.0  
**Status:** Draft  
**Last Updated:** 2026-02-12

---

## Overview

This document specifies the FoundationDB keyspace layout for Pelago, a multi-tenant distributed database designed for VFX production pipelines. The design supports:

- Global replication across 5-6 datacenters
- Lifecycle-based storage optimization (hot → warm → cold)
- Locality-based conflict resolution
- Client-side schema versioning
- Cross-project data references
- 100:1 read:write ratio optimization

### Design Principles

1. **Locality as first-class concept** — Every entity has an owning datacenter; only that DC can mutate it
2. **Lifecycle-aware storage** — Projects transition from active (hot) to inactive (warm/cold) with appropriate storage tiers
3. **Namespace isolation** — Namespaces can be deleted atomically via range clear
4. **Edges in source namespace** — Bidirectional edges stored in creating namespace for clean deletion semantics
5. **Soft deletes only** — Individual entities are tombstoned, never hard-deleted; namespace deletion is the only hard delete

---

## Directory Structure

```
/pelago/
  /_sys/                     → System-wide configuration
  /_auth/                    → Global authorization
  /<db>/
    /_db/                    → DB-level metadata
    /<ns>/
      /_ns/                  → Namespace metadata
      /_schema/              → Schema definitions
      /_types/               → Entity type registry
      /data/                 → Entity storage
      /loc/                  → Locality index
      /idx/                  → Secondary indices
      /edge/                 → Edge storage
      /xref/                 → Cross-project reference tracking
```

### Naming Conventions

- Underscored prefixes (`_sys`, `_db`, `_ns`, etc.) are reserved system subspaces
- User-created namespaces cannot begin with underscore
- DB names map to projects (e.g., `starwars_sequel`, `avatar_2`, `library_characters`)
- Namespace names map to data spaces (e.g., `core`, `statistics`, `render_data`)

---

## System Configuration

**Subspace:** `/pelago/_sys/`

### Keys

| Key | Value | Description |
|-----|-------|-------------|
| `(config)` | System config object | Global system settings |
| `(dc, <dc_id>)` | DC config object | Datacenter registry |
| `(dc_topo, <dc_a>, <dc_b>)` | Topology object | DC-to-DC network characteristics |
| `(repl, <lifecycle>)` | String array | Replication set per lifecycle state |
| `(migration, <id>)` | Migration object | Active migration tracking |

### Value Schemas

**System Config:**
```json
{
  "version": 1,
  "created_at": "<timestamp>",
  "default_replication": 6,
  "default_lifecycle": "hot"
}
```

**DC Config:**
```json
{
  "name": "San Francisco",
  "region": "us-west",
  "tier": "primary",
  "fdb_cluster": "sf-prod-001",
  "s3_bucket": "pelago-cold-sf",
  "active": true
}
```

**DC Topology:**
```json
{
  "latency_ms": 85,
  "bandwidth_gbps": 10
}
```

**Replication Sets:**
```json
["SF", "LON", "SYD", "VAN", "MTL", "SNG"]
```

**Migration:**
```json
{
  "type": "lifecycle",
  "db": "starwars",
  "ns": null,
  "from": "hot",
  "to": "warm",
  "status": "in_progress",
  "started_at": "<timestamp>",
  "completed_at": null,
  "progress": {"total": 50000, "done": 12000},
  "error": null
}
```

| Status | Description |
|--------|-------------|
| `pending` | Migration queued but not started |
| `in_progress` | Migration actively running |
| `complete` | Migration finished successfully |
| `failed` | Migration failed (see `error` field) |

---

## Authorization

**Subspace:** `/pelago/_auth/`

### Keys

| Key | Value | Description |
|-----|-------|-------------|
| `(scope, <name>)` | Scope definition | Named permission bundle |
| `(user, <user_id>)` | User permissions | User's scopes and direct grants |
| `(cache, <user_id>, <db>, <ns>)` | Access cache | Precomputed access for fast checks |

### Value Schemas

**Scope:**
```json
{
  "patterns": [
    {"path": "*/core/*", "access": "read"},
    {"path": "library_*/**", "access": "read"}
  ],
  "desc": "Basic read access"
}
```

**User:**
```json
{
  "scopes": ["read_basic", "library"],
  "direct": [
    {"path": "starwars/core/*", "access": "write"},
    {"path": "starwars/statistics/*", "access": "write"}
  ]
}
```

**Access Cache:**
```json
{
  "r": true,
  "w": false,
  "expires": "<timestamp>"
}
```

### Permission Path Patterns

| Pattern | Matches |
|---------|---------|
| `starwars/core/*` | Specific DB and namespace |
| `*/core/*` | All DBs, specific namespace |
| `library_*/**` | DB prefix, all namespaces |
| `starwars/**` | Specific DB, all namespaces |

---

## DB Metadata

**Subspace:** `/pelago/<db>/_db/`

### Keys

| Key | Value | Description |
|-----|-------|-------------|
| `(config)` | DB config object | Database configuration |
| `(lock)` | Lock object or null | Write lock state |
| `(hist, <ts>)` | History event | Audit history |
| `(ns, <ns_name>)` | Namespace info | Namespace registry entry |

### Value Schemas

**DB Config:**
```json
{
  "lifecycle": "hot",
  "replicas": 6,
  "repl_set": ["SF", "LON", "SYD", "VAN", "MTL", "SNG"],
  "locality_default": "SF",
  "project_type": "feature_film",
  "created_at": "<timestamp>",
  "created_by": "user_123"
}
```

| Project Type | Description |
|--------------|-------------|
| `feature_film` | Movie production |
| `series` | TV series |
| `ride` | Theme park attraction |
| `library` | Shared asset library |

**Lock:**
```json
{
  "locked_at": "<timestamp>",
  "locked_by": "migration_456",
  "reason": "lifecycle_transition",
  "unlock_token": "abc123"
}
```

When `(lock)` key is absent or null, writes are permitted. When present, all writes are rejected unless the unlock token is provided.

**History Event:**
```json
{
  "event": "lifecycle_change",
  "from": "hot",
  "to": "warm",
  "by": "user_456",
  "reason": "project_complete"
}
```

**Namespace Registry Entry:**
```json
{
  "created_at": "<timestamp>",
  "lifecycle": "hot"
}
```

---

## Namespace Metadata

**Subspace:** `/pelago/<db>/<ns>/_ns/`

### Keys

| Key | Value | Description |
|-----|-------|-------------|
| `(config)` | NS config object | Namespace configuration |
| `(lock)` | Lock object or null | Write lock state |
| `(stats)` | Statistics object | Namespace statistics (async updated) |

### Value Schemas

**NS Config:**
```json
{
  "lifecycle": "hot",
  "desc": "Core production entities",
  "created_at": "<timestamp>"
}
```

Namespace lifecycle can be colder than DB lifecycle, never hotter.

**Lock:** Same schema as DB lock.

**Statistics:**
```json
{
  "entities": 150000,
  "edges": 890000,
  "tombstoned": 2300,
  "bytes": 4500000000,
  "updated_at": "<timestamp>"
}
```

Statistics are updated asynchronously by background processes.

---

## Entity Type Registry

**Subspace:** `/pelago/<db>/<ns>/_types/`

### Keys

| Key | Value | Description |
|-----|-------|-------------|
| `(<type>)` | Type info object | Entity type metadata |

### Value Schema

```json
{
  "created_at": "<timestamp>",
  "entity_count": 45000,
  "latest_schema": 3
}
```

### Purpose

- Enumerate all entity types in a namespace without scanning `data/`
- Quick count per type for monitoring
- Validate type existence before queries

Updated when schemas are registered and asynchronously for entity counts.

---

## Schema Definitions

**Subspace:** `/pelago/<db>/<ns>/_schema/`

### Keys

| Key | Value | Description |
|-----|-------|-------------|
| `(<type>, v, <version>)` | Schema definition | Versioned schema |
| `(<type>, latest)` | Integer | Current version pointer |

The `v` separator prevents collision between version integers and the `latest` marker.

### Value Schema

**Schema Definition:**
```json
{
  "fields": [
    {"name": "title", "type": "string", "required": true},
    {"name": "status", "type": "enum", "values": ["active", "complete"]},
    {"name": "frame_count", "type": "int", "min": 0}
  ],
  "validations": [],
  "created_at": "<timestamp>",
  "created_by": "user_123"
}
```

### Field Types

| Type | Options | Description |
|------|---------|-------------|
| `string` | `min_length`, `max_length`, `pattern` | Text field |
| `int` | `min`, `max` | Integer field |
| `float` | `min`, `max` | Floating point field |
| `bool` | — | Boolean field |
| `enum` | `values` | Enumerated values |
| `array` | `items`, `min_items`, `max_items` | Array of typed items |
| `object` | `properties` | Nested object |
| `ref` | `target_type` | Reference to another entity |

### Schema Lookup Flow

1. Read `(<type>, latest)` → get version number
2. Read `(<type>, v, <version>)` → get schema definition
3. Cache at API layer with FDB watch for invalidation

---

## Entity Storage

**Subspace:** `/pelago/<db>/<ns>/data/`

### Keys

| Key | Value | Description |
|-----|-------|-------------|
| `(<type>, <id>)` | Entity record | Primary entity storage |

### Value Schema

```json
{
  "p": {},
  "s": 3,
  "l": "SF",
  "t": null,
  "c": "<timestamp>",
  "u": "<timestamp>",
  "cb": "user_123",
  "ub": "user_456",
  "cold": null
}
```

| Field | Full Name | Type | Description |
|-------|-----------|------|-------------|
| `p` | payload | object | Client data |
| `s` | schema_v | int | Schema version |
| `l` | locality | string | Owning DC (3-char) |
| `t` | tombstoned_at | timestamp/null | Tombstone timestamp |
| `c` | created_at | timestamp | Creation time |
| `u` | updated_at | timestamp | Last update time |
| `cb` | created_by | string | Creating user |
| `ub` | updated_by | string | Last updating user |
| `cold` | cold | object/null | Cold storage info |

Field names are shortened to reduce storage overhead at scale.

### Cold Entity

When an entity is migrated to cold storage:

```json
{
  "p": null,
  "s": 3,
  "l": "SF",
  "t": null,
  "c": "<timestamp>",
  "u": "<timestamp>",
  "cb": "user_123",
  "ub": "user_456",
  "cold": {
    "key": "starwars/core/Shot/c2hvdF8xMjM.msgpack",
    "at": "<timestamp>"
  }
}
```

The `p` (payload) field is set to null; data is in S3 at the specified key.

### S3 Key Format

```
s3://<bucket>/<db>/<ns>/<type>/<base64url(id)>.msgpack
```

Entity ID is base64url-encoded to handle special characters safely.

---

## Locality Index

**Subspace:** `/pelago/<db>/<ns>/loc/`

### Keys

| Key | Value | Description |
|-----|-------|-------------|
| `(<type>, <id>)` | DC identifier | Locality for write validation |

### Value

3-character UTF-8 string identifying the owning datacenter (e.g., `SF`, `LON`, `SYD`).

### Purpose

Dedicated subspace for write-path optimization:

- Write validation reads only this key (3 bytes)
- Ownership transfer updates only this key + main record
- Separated from entity data for minimal read on hot path

### Invariant

`loc/(<type>, <id>)` always equals `data/(<type>, <id>).l`. Updated atomically in same transaction.

---

## Secondary Indices

**Subspace:** `/pelago/<db>/<ns>/idx/`

### Keys

| Key | Value | Description |
|-----|-------|-------------|
| `(tomb, <type>, <id>)` | Timestamp | Tombstoned entities |
| `(byloc, <dc>, <type>, <id>)` | `0x01` | Entities by locality |
| `(byschema, <type>, <version>, <id>)` | `0x01` | Entities by schema version |
| `(byupd, <type>, <versionstamp>)` | Entity ID | Entities by update time |

### Index Details

**Tombstone Index:** Created when entity is tombstoned, deleted if un-tombstoned. Value is the tombstone timestamp.

**By-Locality Index:** For DC operations (migration, failover). Enables "list all entities owned by SYD".

**By-Schema Index:** For schema migration tooling. Find all entities on old schema version.

**By-Update Index:** Uses FDB versionstamp for true global ordering. Key contains versionstamp; value is entity ID (versionstamp cannot be in value position in tuple key).

```python
# Write with versionstamp
tr.set_versionstamped_key(
    idx_ss.pack_with_versionstamp(('byupd', type, fdb.tuple.Versionstamp())),
    id.encode()
)
```

### Index Maintenance

All indices are denormalized and can be rebuilt from entity data if corrupted. Trade-off: write amplification for query speed.

---

## Edge Storage

**Subspace:** `/pelago/<db>/<ns>/edge/`

### Keys

| Key | Value | Description |
|-----|-------|-------------|
| `(f, <src_type>, <src_id>, <label>, <tgt_db>, <tgt_ns>, <tgt_type>, <tgt_id>)` | Timestamp | Forward edge |
| `(fm, <src_type>, <src_id>, <label>, <tgt_db>, <tgt_ns>, <tgt_type>, <tgt_id>)` | Metadata object | Forward edge metadata (optional) |
| `(r, <tgt_db>, <tgt_ns>, <tgt_type>, <tgt_id>, <label>, <src_type>, <src_id>)` | `0x01` | Reverse edge |

### Prefix Meanings

| Prefix | Meaning |
|--------|---------|
| `f` | Forward edge (existence + created_at) |
| `fm` | Forward edge metadata (optional) |
| `r` | Reverse edge (existence only) |

### Forward Edge Value

Timestamp of edge creation. Compact representation for existence-only queries.

### Forward Edge Metadata Value

```json
{
  "created_by": "user_123",
  "meta": {}
}
```

Only written if edge has metadata. Separate key avoids reading metadata on existence-only queries.

### Reverse Edge Value

Single byte `0x01` as existence marker. Full data lives in forward edge.

### Edge Types

**Within-namespace edge:**
```
(f, Shot, shot_123, has_task, starwars, core, Task, task_456)
```

Target db/ns are the same as source. No special case in storage.

**Cross-namespace edge:**
```
(f, RenderStat, stat_789, for_shot, starwars, core, Shot, shot_123)
```

Stored in `statistics` namespace, points to `core` namespace.

**Cross-project edge:**
```
(f, Shot, shot_new, references_asset, starwars_original, assets, Asset, asset_old)
```

### Edge Query Patterns

**Forward traversal (what does this entity point to?):**
```python
edge_ss.range(('f', src_type, src_id))
```

**Reverse traversal (what points to this entity?):**
Must query the SOURCE namespace's reverse index:
```python
other_ns_edge_ss.range(('r', my_db, my_ns, my_type, my_id))
```

This is why the API requires explicit `from_namespaces` parameter for reverse queries.

### Namespace Deletion Semantics

Edges are stored bidirectionally in the source namespace. When a namespace is deleted (range clear), all edges created in that namespace are removed, including reverse pointers to other namespaces.

---

## Cross-Project Reference Tracking

**Subspace:** `/pelago/<db>/<ns>/xref/`

### Keys

| Key | Value | Description |
|-----|-------|-------------|
| `(out, <tgt_db>)` | Integer count | Outgoing references to other projects |
| `(in, <src_db>, <src_ns>)` | Integer count | Incoming references from other projects |

### Purpose

Fast check during cold transition: "Does anyone reference this project?"

```python
# Before transitioning starwars_original to cold:
xref_ss = directory.open(tr, ('pelago', 'starwars_original', 'core', 'xref'))
incoming = list(tr.get_range(xref_ss.range(('in',))))
if any(decode_int(v) > 0 for _, v in incoming):
    # Has active references from other projects
    # Either: reject transition, or warn and accept slower access
```

### Counter Updates

Updated atomically when creating/deleting cross-project edges:

```python
def create_cross_project_edge(tr, src_db, src_ns, tgt_db, tgt_ns, ...):
    # ... create edge ...
    
    if src_db != tgt_db:
        # Increment source's outgoing count
        src_xref = directory.open(tr, ('pelago', src_db, src_ns, 'xref'))
        atomic_add(tr, src_xref.pack(('out', tgt_db)), 1)
        
        # Increment target's incoming count
        tgt_xref = directory.open(tr, ('pelago', tgt_db, tgt_ns, 'xref'))
        atomic_add(tr, tgt_xref.pack(('in', src_db, src_ns)), 1)
```

---

## Key Encoding

### Data Types

| Element | Encoding | Size |
|---------|----------|------|
| `db`, `ns`, `type` | UTF-8 string | Variable |
| `id` | UTF-8 string (UUID) | ~36 bytes |
| `dc` | UTF-8 3-char | 3 bytes |
| `version` | Big-endian int32 | 4 bytes |
| `ts` (timestamp) | Int64 microseconds | 8 bytes |
| `versionstamp` | FDB versionstamp | 10 bytes |
| `label` | UTF-8 string | Variable |
| Counter values | Little-endian int64 | 8 bytes |
| Existence markers | `0x01` | 1 byte |

### FDB Tuple Layer

All keys use FDB's tuple layer for encoding, providing:

- Lexicographic ordering
- Type-aware comparison
- Efficient prefix matching
- Versionstamp support

### Value Encoding

MessagePack for all structured values:

- Compact binary format
- Schema-flexible (no compilation required)
- Fast encode/decode
- Wide language support

---

## Transaction Patterns

### Create Entity

```python
@fdb.transactional
def create_entity(tr, db, ns, type, id, payload, locality, schema_v, user_id):
    # Open subspaces
    data_ss = directory.open(tr, ('pelago', db, ns, 'data'))
    loc_ss = directory.open(tr, ('pelago', db, ns, 'loc'))
    idx_ss = directory.open(tr, ('pelago', db, ns, 'idx'))
    schema_ss = directory.open(tr, ('pelago', db, ns, '_schema'))
    types_ss = directory.open(tr, ('pelago', db, ns, '_types'))
    sys_ss = directory.open(tr, ('pelago', '_sys'))
    
    # Check write lock
    check_write_allowed(tr, db, ns)
    
    # Verify schema exists
    if tr[schema_ss.pack((type, 'v', schema_v))] is None:
        raise SchemaNotFound(type, schema_v)
    
    # Check entity doesn't exist
    if tr[data_ss.pack((type, id))] is not None:
        raise EntityExists(type, id)
    
    # Validate locality is known DC
    if tr[sys_ss.pack(('dc', locality))] is None:
        raise InvalidLocality(locality)
    
    now = current_timestamp()
    
    # Write entity
    tr[data_ss.pack((type, id))] = msgpack.encode({
        'p': payload,
        's': schema_v,
        'l': locality,
        't': None,
        'c': now,
        'u': now,
        'cb': user_id,
        'ub': user_id,
        'cold': None
    })
    
    # Write locality
    tr[loc_ss.pack((type, id))] = locality.encode('utf-8')
    
    # Write indices
    tr[idx_ss.pack(('byloc', locality, type, id))] = b'\x01'
    tr[idx_ss.pack(('byschema', type, schema_v, id))] = b'\x01'
    tr.set_versionstamped_key(
        idx_ss.pack_with_versionstamp(('byupd', type, fdb.tuple.Versionstamp())),
        id.encode()
    )
    
    # Update type registry (increment count)
    update_type_count(tr, types_ss, type, 1)
```

### Update Entity

```python
@fdb.transactional
def update_entity(tr, db, ns, type, id, payload, user_id, requesting_dc):
    loc_ss = directory.open(tr, ('pelago', db, ns, 'loc'))
    data_ss = directory.open(tr, ('pelago', db, ns, 'data'))
    idx_ss = directory.open(tr, ('pelago', db, ns, 'idx'))
    
    # Check write lock
    check_write_allowed(tr, db, ns)
    
    # Fast locality check
    owner = tr[loc_ss.pack((type, id))]
    if owner is None:
        raise EntityNotFound(type, id)
    if owner.decode('utf-8') != requesting_dc:
        raise NotLocalityOwner(owner=owner.decode(), requester=requesting_dc)
    
    # Load current entity
    entity = msgpack.decode(tr[data_ss.pack((type, id))])
    
    if entity['t']:  # tombstoned_at
        raise EntityTombstoned(type, id)
    
    if entity['cold']:
        raise EntityCold(type, id)  # must rehydrate first
    
    now = current_timestamp()
    
    # Update entity
    entity['p'] = payload
    entity['u'] = now
    entity['ub'] = user_id
    tr[data_ss.pack((type, id))] = msgpack.encode(entity)
    
    # Update time index (versionstamped)
    tr.set_versionstamped_key(
        idx_ss.pack_with_versionstamp(('byupd', type, fdb.tuple.Versionstamp())),
        id.encode()
    )
```

### Transfer Ownership

```python
@fdb.transactional
def transfer_ownership(tr, db, ns, type, id, new_dc, requesting_dc):
    """Must execute at CURRENT owner DC."""
    loc_ss = directory.open(tr, ('pelago', db, ns, 'loc'))
    data_ss = directory.open(tr, ('pelago', db, ns, 'data'))
    idx_ss = directory.open(tr, ('pelago', db, ns, 'idx'))
    sys_ss = directory.open(tr, ('pelago', '_sys'))
    
    # Verify new DC exists
    if tr[sys_ss.pack(('dc', new_dc))] is None:
        raise InvalidLocality(new_dc)
    
    # Check current ownership
    current_dc = tr[loc_ss.pack((type, id))]
    if current_dc is None:
        raise EntityNotFound(type, id)
    current_dc = current_dc.decode('utf-8')
    
    if current_dc != requesting_dc:
        raise NotLocalityOwner(owner=current_dc, requester=requesting_dc)
    
    if current_dc == new_dc:
        return  # no-op
    
    # Update locality key
    tr[loc_ss.pack((type, id))] = new_dc.encode('utf-8')
    
    # Update entity record
    entity = msgpack.decode(tr[data_ss.pack((type, id))])
    entity['l'] = new_dc
    entity['u'] = current_timestamp()
    tr[data_ss.pack((type, id))] = msgpack.encode(entity)
    
    # Update locality index
    tr.clear(idx_ss.pack(('byloc', current_dc, type, id)))
    tr[idx_ss.pack(('byloc', new_dc, type, id))] = b'\x01'
```

### Create Edge

```python
@fdb.transactional
def create_edge(tr, db, ns, src_type, src_id, label, 
                tgt_db, tgt_ns, tgt_type, tgt_id, user_id, metadata=None):
    edge_ss = directory.open(tr, ('pelago', db, ns, 'edge'))
    
    now = current_timestamp()
    
    # Forward edge
    fwd_key = edge_ss.pack(('f', src_type, src_id, label, 
                            tgt_db, tgt_ns, tgt_type, tgt_id))
    tr[fwd_key] = msgpack.encode(now)
    
    # Forward edge metadata (if provided)
    if metadata:
        fm_key = edge_ss.pack(('fm', src_type, src_id, label,
                               tgt_db, tgt_ns, tgt_type, tgt_id))
        tr[fm_key] = msgpack.encode({
            'created_by': user_id,
            'meta': metadata
        })
    
    # Reverse edge
    rev_key = edge_ss.pack(('r', tgt_db, tgt_ns, tgt_type, tgt_id,
                            label, src_type, src_id))
    tr[rev_key] = b'\x01'
    
    # Update cross-project reference counts
    if db != tgt_db:
        src_xref = directory.open(tr, ('pelago', db, ns, 'xref'))
        atomic_add(tr, src_xref.pack(('out', tgt_db)), 1)
        
        tgt_xref = directory.open(tr, ('pelago', tgt_db, tgt_ns, 'xref'))
        atomic_add(tr, tgt_xref.pack(('in', db, ns)), 1)
```

### Tombstone Entity

```python
@fdb.transactional
def tombstone_entity(tr, db, ns, type, id, user_id, requesting_dc):
    loc_ss = directory.open(tr, ('pelago', db, ns, 'loc'))
    data_ss = directory.open(tr, ('pelago', db, ns, 'data'))
    idx_ss = directory.open(tr, ('pelago', db, ns, 'idx'))
    
    check_write_allowed(tr, db, ns)
    
    # Locality check
    owner = tr[loc_ss.pack((type, id))]
    if owner is None:
        raise EntityNotFound(type, id)
    if owner.decode('utf-8') != requesting_dc:
        raise NotLocalityOwner(owner=owner.decode(), requester=requesting_dc)
    
    entity = msgpack.decode(tr[data_ss.pack((type, id))])
    
    if entity['t']:
        raise EntityAlreadyTombstoned(type, id)
    
    now = current_timestamp()
    
    entity['t'] = now
    entity['u'] = now
    entity['ub'] = user_id
    tr[data_ss.pack((type, id))] = msgpack.encode(entity)
    
    # Add to tombstone index
    tr[idx_ss.pack(('tomb', type, id))] = msgpack.encode(now)
```

### Delete Namespace

```python
@fdb.transactional
def delete_namespace(tr, db, ns):
    # Clear all subspaces
    for subspace in ['_ns', '_schema', '_types', 'data', 'loc', 'idx', 'edge', 'xref']:
        ss = directory.open(tr, ('pelago', db, ns, subspace))
        tr.clear_range(ss.range())
    
    # Remove directory entries
    directory.remove(tr, ('pelago', db, ns))
    
    # Update DB namespace registry
    db_ss = directory.open(tr, ('pelago', db, '_db'))
    tr.clear(db_ss.pack(('ns', ns)))

# After transaction commits:
def post_delete_cleanup(db, ns):
    # Queue async S3 cleanup
    enqueue_cold_cleanup(f"{db}/{ns}/")
```

---

## Query Patterns

### Get Entity

```python
def get_entity(tr, db, ns, type, id, include_tombstoned=False):
    data_ss = directory.open(tr, ('pelago', db, ns, 'data'))
    
    raw = tr[data_ss.pack((type, id))]
    if raw is None:
        raise EntityNotFound(type, id)
    
    entity = msgpack.decode(raw)
    
    if entity['t'] and not include_tombstoned:
        raise EntityNotFound(type, id)
    
    # Handle cold entity
    if entity['cold']:
        cached = cache.get(entity['cold']['key'])
        if cached:
            return cached
        
        data = s3.get(entity['cold']['key'])
        cache.set(entity['cold']['key'], data, ttl=3600)
        return data
    
    return entity['p']
```

### List Entity Types

```python
def list_types(tr, db, ns):
    types_ss = directory.open(tr, ('pelago', db, ns, '_types'))
    
    result = []
    for k, v in tr.get_range(types_ss.range()):
        type_name = types_ss.unpack(k)[0]
        info = msgpack.decode(v)
        result.append({
            'type': type_name,
            'count': info['entity_count'],
            'latest_schema': info['latest_schema']
        })
    return result
```

### Query Edges

```python
def get_edges(tr, db, ns, type, id, from_namespaces=None, skip_tombstoned=False):
    edges = []
    
    # Forward edges (from this entity)
    edge_ss = directory.open(tr, ('pelago', db, ns, 'edge'))
    for k, v in tr.get_range(edge_ss.range(('f', type, id))):
        _, _, _, label, tgt_db, tgt_ns, tgt_type, tgt_id = edge_ss.unpack(k)
        
        if skip_tombstoned:
            if is_tombstoned(tr, tgt_db, tgt_ns, tgt_type, tgt_id):
                continue
        
        edges.append({
            'direction': 'outgoing',
            'label': label,
            'target': {'db': tgt_db, 'ns': tgt_ns, 'type': tgt_type, 'id': tgt_id},
            'created_at': msgpack.decode(v)
        })
    
    # Reverse edges (to this entity, from other namespaces)
    if from_namespaces:
        for other_ns in from_namespaces:
            other_edge_ss = directory.open(tr, ('pelago', db, other_ns, 'edge'))
            for k, _ in tr.get_range(other_edge_ss.range(('r', db, ns, type, id))):
                _, _, _, _, _, label, src_type, src_id = other_edge_ss.unpack(k)
                
                if skip_tombstoned:
                    if is_tombstoned(tr, db, other_ns, src_type, src_id):
                        continue
                
                edges.append({
                    'direction': 'incoming',
                    'label': label,
                    'source': {'db': db, 'ns': other_ns, 'type': src_type, 'id': src_id}
                })
    
    return edges
```

### Find Entities by Locality

```python
def find_by_locality(tr, db, ns, dc, type=None):
    idx_ss = directory.open(tr, ('pelago', db, ns, 'idx'))
    
    if type:
        range_start = idx_ss.pack(('byloc', dc, type))
        range_end = idx_ss.pack(('byloc', dc, type, b'\xff'))
    else:
        range_start = idx_ss.pack(('byloc', dc))
        range_end = idx_ss.pack(('byloc', dc, b'\xff'))
    
    result = []
    for k, _ in tr.get_range(range_start, range_end):
        parts = idx_ss.unpack(k)
        result.append({'type': parts[2], 'id': parts[3]})
    return result
```

### Find Entities Updated Since

```python
def find_updated_since(tr, db, ns, type, since_versionstamp):
    idx_ss = directory.open(tr, ('pelago', db, ns, 'idx'))
    
    start = idx_ss.pack(('byupd', type, since_versionstamp))
    end = idx_ss.pack(('byupd', type, b'\xff'))
    
    result = []
    for k, v in tr.get_range(start, end):
        entity_id = v.decode()
        result.append(entity_id)
    return result
```

### Check Cross-Project References

```python
def has_incoming_references(tr, db, ns):
    xref_ss = directory.open(tr, ('pelago', db, ns, 'xref'))
    
    for k, v in tr.get_range(xref_ss.range(('in',))):
        count = decode_int(v)
        if count > 0:
            _, src_db, src_ns = xref_ss.unpack(k)
            return True, {'db': src_db, 'ns': src_ns, 'count': count}
    
    return False, None
```

---

## Lifecycle Operations

### Hot → Warm Transition

```python
def transition_to_warm(db):
    # 1. Acquire write lock
    @fdb.transactional
    def lock_writes(tr):
        db_ss = directory.open(tr, ('pelago', db, '_db'))
        
        token = generate_token()
        tr[db_ss.pack(('lock',))] = msgpack.encode({
            'locked_at': current_timestamp(),
            'locked_by': 'lifecycle_transition',
            'reason': 'hot_to_warm',
            'unlock_token': token
        })
        
        tr[db_ss.pack(('hist', current_timestamp()))] = msgpack.encode({
            'event': 'write_lock',
            'reason': 'hot_to_warm'
        })
        
        return token
    
    token = lock_writes(fdb.db)
    
    # 2. Wait for replication quiescence
    wait_for_replication_sync(db)
    
    # 3. Reduce FDB replicas (external configuration)
    reduce_fdb_replicas(db, target=3, dcs=['SF', 'LON', 'SYD'])
    
    # 4. Update configuration
    @fdb.transactional
    def update_config(tr):
        db_ss = directory.open(tr, ('pelago', db, '_db'))
        
        config = msgpack.decode(tr[db_ss.pack(('config',))])
        config['lifecycle'] = 'warm'
        config['replicas'] = 3
        config['repl_set'] = ['SF', 'LON', 'SYD']
        tr[db_ss.pack(('config',))] = msgpack.encode(config)
        
        tr[db_ss.pack(('hist', current_timestamp()))] = msgpack.encode({
            'event': 'lifecycle_change',
            'from': 'hot',
            'to': 'warm'
        })
    
    update_config(fdb.db)
    
    return token  # Keep for potential unlock
```

### Warm → Cold Transition

```python
def transition_to_cold(db, ns=None):
    # 1. Check for incoming references
    if ns:
        namespaces = [ns]
    else:
        namespaces = list_namespaces(db)
    
    for namespace in namespaces:
        has_refs, ref_info = has_incoming_references(fdb.db, db, namespace)
        if has_refs:
            raise HasActiveReferences(db, namespace, ref_info)
    
    # 2. Start migration
    migration_id = create_migration(db, ns, 'warm', 'cold')
    
    # 3. For each namespace, migrate entities to S3
    for namespace in namespaces:
        migrate_namespace_to_cold(db, namespace, migration_id)
    
    # 4. Update configuration
    @fdb.transactional
    def update_config(tr):
        db_ss = directory.open(tr, ('pelago', db, '_db'))
        
        config = msgpack.decode(tr[db_ss.pack(('config',))])
        config['lifecycle'] = 'cold'
        config['replicas'] = 1
        config['repl_set'] = ['SF']
        tr[db_ss.pack(('config',))] = msgpack.encode(config)
    
    update_config(fdb.db)
    complete_migration(migration_id)


def migrate_namespace_to_cold(db, ns, migration_id):
    data_ss = directory.open(fdb.db, ('pelago', db, ns, 'data'))
    
    cursor = None
    batch_size = 1000
    
    while True:
        @fdb.transactional
        def migrate_batch(tr):
            nonlocal cursor
            
            start = data_ss.range().start if cursor is None else cursor
            end = data_ss.range().stop
            
            entities = list(tr.get_range(start, end, limit=batch_size))
            if not entities:
                return False
            
            for k, v in entities:
                type, id = data_ss.unpack(k)
                entity = msgpack.decode(v)
                
                if entity['cold']:  # already cold
                    continue
                
                if entity['p'] is None:  # no payload (already cleared?)
                    continue
                
                # Upload to S3
                s3_key = f"{db}/{ns}/{type}/{base64url_encode(id)}.msgpack"
                s3.put(s3_key, msgpack.encode(entity['p']))
                
                # Update entity record
                entity['p'] = None
                entity['cold'] = {
                    'key': s3_key,
                    'at': current_timestamp()
                }
                tr[k] = msgpack.encode(entity)
            
            cursor = entities[-1][0] + b'\x00'
            update_migration_progress(tr, migration_id, len(entities))
            return True
        
        if not migrate_batch(fdb.db):
            break
```

### Unlock Writes

```python
@fdb.transactional
def unlock_writes(tr, db, token):
    db_ss = directory.open(tr, ('pelago', db, '_db'))
    
    lock_raw = tr[db_ss.pack(('lock',))]
    if lock_raw is None:
        return  # Already unlocked
    
    lock = msgpack.decode(lock_raw)
    if lock['unlock_token'] != token:
        raise InvalidUnlockToken()
    
    tr.clear(db_ss.pack(('lock',)))
    
    tr[db_ss.pack(('hist', current_timestamp()))] = msgpack.encode({
        'event': 'write_unlock',
        'locked_at': lock['locked_at']
    })
```

---

## Write Lock Enforcement

```python
def check_write_allowed(tr, db, ns):
    """Called before any write operation. Raises WriteLocked if locked."""
    db_ss = directory.open(tr, ('pelago', db, '_db'))
    ns_ss = directory.open(tr, ('pelago', db, ns, '_ns'))
    
    # Check DB-level lock
    db_lock = tr[db_ss.pack(('lock',))]
    if db_lock:
        lock = msgpack.decode(db_lock)
        raise WriteLocked(
            level='db',
            reason=lock['reason'],
            locked_at=lock['locked_at']
        )
    
    # Check NS-level lock
    ns_lock = tr[ns_ss.pack(('lock',))]
    if ns_lock:
        lock = msgpack.decode(ns_lock)
        raise WriteLocked(
            level='ns', 
            reason=lock['reason'],
            locked_at=lock['locked_at']
        )
```

---

## Appendix A: Complete Keyspace Reference

```
/pelago/
│
├── _sys/
│   ├── (config)                                    → System config
│   ├── (dc, <dc>)                                  → DC registry
│   ├── (dc_topo, <dc>, <dc>)                       → DC topology
│   ├── (repl, <lifecycle>)                         → Replication sets
│   └── (migration, <id>)                           → Active migrations
│
├── _auth/
│   ├── (scope, <name>)                             → Permission bundles
│   ├── (user, <user_id>)                           → User permissions
│   └── (cache, <user>, <db>, <ns>)                 → Auth cache
│
└── <db>/
    ├── _db/
    │   ├── (config)                                → DB config
    │   ├── (lock)                                  → Write lock
    │   ├── (hist, <ts>)                            → History
    │   └── (ns, <ns>)                              → Namespace registry
    │
    └── <ns>/
        ├── _ns/
        │   ├── (config)                            → NS config
        │   ├── (lock)                              → Write lock
        │   └── (stats)                             → Statistics
        │
        ├── _types/
        │   └── (<type>)                            → Type registry
        │
        ├── _schema/
        │   ├── (<type>, v, <ver>)                  → Schema definition
        │   └── (<type>, latest)                    → Current version
        │
        ├── data/
        │   └── (<type>, <id>)                      → Entity record
        │
        ├── loc/
        │   └── (<type>, <id>)                      → Locality
        │
        ├── idx/
        │   ├── (tomb, <type>, <id>)                → Tombstone index
        │   ├── (byloc, <dc>, <type>, <id>)         → By-locality
        │   ├── (byschema, <type>, <ver>, <id>)     → By-schema
        │   └── (byupd, <type>, <versionstamp>)     → By-update (→ id)
        │
        ├── edge/
        │   ├── (f, <src_t>, <src_id>, <lbl>, <tgt_db>, <tgt_ns>, <tgt_t>, <tgt_id>)
        │   ├── (fm, <src_t>, <src_id>, <lbl>, <tgt_db>, <tgt_ns>, <tgt_t>, <tgt_id>)
        │   └── (r, <tgt_db>, <tgt_ns>, <tgt_t>, <tgt_id>, <lbl>, <src_t>, <src_id>)
        │
        └── xref/
            ├── (out, <tgt_db>)                     → Outgoing ref count
            └── (in, <src_db>, <src_ns>)            → Incoming ref count
```

---

## Appendix B: Error Types

| Error | Description |
|-------|-------------|
| `SchemaNotFound` | Referenced schema version does not exist |
| `EntityExists` | Attempted to create entity with existing ID |
| `EntityNotFound` | Entity does not exist or is tombstoned |
| `EntityTombstoned` | Attempted to modify tombstoned entity |
| `EntityCold` | Attempted to modify cold entity without rehydration |
| `InvalidLocality` | Specified DC is not in system registry |
| `NotLocalityOwner` | Write attempted from non-owning DC |
| `WriteLocked` | DB or namespace is locked for writes |
| `InvalidUnlockToken` | Wrong token provided for unlock |
| `HasActiveReferences` | Cannot transition to cold with incoming refs |

---

## Appendix C: Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Entity ID format | String UUID | Debuggability, client control |
| Value encoding | MessagePack | Compact, schema-flexible |
| Schema location | In FDB per-namespace | Matches doc hierarchy, transactional |
| Tombstone GC | Never | "Data exists forever" philosophy |
| Cold storage format | MessagePack | Matches hot format, single-entity access |
| S3 key encoding | Base64url for ID | Safe for any ID characters |
| Write locking | Key in `_db/` and `_ns/` | Simple check on write path |
| Cross-project refs | Counter in `xref/` | Efficient cold transition check |
| Update ordering | FDB versionstamp | True global ordering |
| Edge existence vs metadata | Separate keys | Most traversals need existence only |
