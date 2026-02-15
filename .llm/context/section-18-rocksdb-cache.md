## 18. RocksDB Cache Layer

The RocksDB cache layer provides high-throughput reads for hot data by maintaining a CDC-driven projection of frequently accessed entities. This implements the CQRS (Command Query Responsibility Segregation) pattern where writes flow through FDB (system of record) and reads are served from RocksDB when consistency requirements permit.

### 18.1 Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           Cache Architecture                                  │
└─────────────────────────────────────────────────────────────────────────────┘

                              WRITE PATH
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                            FoundationDB                                     │
│    ┌─────────────┐    ┌─────────────┐    ┌─────────────┐                   │
│    │  /data/     │    │  /idx/      │    │  /_cdc/     │                   │
│    │  (entities) │    │  (indexes)  │    │  (log)      │                   │
│    └─────────────┘    └─────────────┘    └──────┬──────┘                   │
│                                                 │                           │
└─────────────────────────────────────────────────┼───────────────────────────┘
                                                  │
                            CDC Consumer          │
                                  │◄──────────────┘
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                         CDC Projector Process                              │
│    ┌─────────────────────────────────────────────────────────────┐       │
│    │  consume CDC → transform → write RocksDB → update HWM        │       │
│    └─────────────────────────────────────────────────────────────┘       │
│                                  │                                         │
└──────────────────────────────────┼─────────────────────────────────────────┘
                                   │
                                   ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                             RocksDB                                         │
│    ┌─────────────┐    ┌─────────────┐    ┌─────────────┐                   │
│    │  node:*     │    │  edge:*     │    │  meta:hwm   │                   │
│    │  (data)     │    │  (adjacency)│    │  (tracking) │                   │
│    └─────────────┘    └─────────────┘    └─────────────┘                   │
└──────────────────────────────────────────────────────────────────────────┘
                                   ▲
                                   │
                              READ PATH
```

**Data Flow:**

1. Mutations commit to FDB (entities, indexes, CDC entry atomically)
2. CDC Projector consumes entries from `/_cdc/` subspace
3. Projector transforms CDC operations into RocksDB writes
4. Projector advances high-water mark (HWM) after successful projection
5. Query path checks RocksDB, verifies HWM freshness, falls back to FDB

### 18.2 RocksDB Keyspace Layout

RocksDB keys are derived from FDB keys with a simplified prefix structure optimized for point lookups and adjacency traversal.

#### Key Prefixes

| Prefix | Purpose | Key Format | Value |
|--------|---------|------------|-------|
| `n:` | Node data | `n:<db>:<ns>:<type>:<node_id>` | CBOR entity |
| `e:` | Edge forward | `e:<db>:<ns>:<src_type>:<src_id>:<label>:<tgt_ns>:<tgt_type>:<tgt_id>` | Edge metadata |
| `r:` | Edge reverse | `r:<db>:<ns>:<tgt_type>:<tgt_id>:<label>:<src_ns>:<src_type>:<src_id>` | `0x01` |
| `m:` | Metadata | `m:<key>` | Varies |

#### Key Encoding

Keys use a colon-separated string format (not FDB tuple layer) for RocksDB:

```rust
fn encode_node_key(db: &str, ns: &str, entity_type: &str, node_id: &NodeId) -> Vec<u8> {
    format!("n:{}:{}:{}:{}", db, ns, entity_type, node_id.to_string())
        .into_bytes()
}

fn encode_edge_forward_key(
    db: &str,
    ns: &str,
    src_type: &str,
    src_id: &NodeId,
    label: &str,
    tgt_ns: &str,
    tgt_type: &str,
    tgt_id: &NodeId,
) -> Vec<u8> {
    format!(
        "e:{}:{}:{}:{}:{}:{}:{}:{}",
        db, ns, src_type, src_id.to_string(), label, tgt_ns, tgt_type, tgt_id.to_string()
    ).into_bytes()
}
```

**FDB to RocksDB Key Mapping:**

| FDB Key | RocksDB Key |
|---------|-------------|
| `(db, ns, data, Person, 1_42)` | `n:db:ns:Person:1_42` |
| `(db, ns, edge, f, Person, 1_42, KNOWS, db, ns, Person, 1_43)` | `e:db:ns:Person:1_42:KNOWS:ns:Person:1_43` |
| `(db, ns, edge, r, db, ns, Person, 1_43, KNOWS, Person, 1_42)` | `r:db:ns:Person:1_43:KNOWS:ns:Person:1_42` |

#### Metadata Keys

| Key | Value | Description |
|-----|-------|-------------|
| `m:hwm` | 10-byte versionstamp | Global high-water mark (last projected CDC entry) |
| `m:hwm:<db>:<ns>` | 10-byte versionstamp | Per-namespace high-water mark |
| `m:stats:nodes` | u64 | Cached node count |
| `m:stats:edges` | u64 | Cached edge count |
| `m:rebuild:status` | JSON | Rebuild job status |

### 18.3 CDC Projector

The CDC Projector is a background process that consumes CDC entries and projects them to RocksDB. It runs as a singleton per PelagoDB instance.

#### Projector Structure

```rust
pub struct CdcProjector {
    /// FDB database handle
    fdb: FdbDatabase,

    /// RocksDB handle
    rocksdb: Arc<DB>,

    /// Consumer identity for checkpoint tracking
    consumer_id: String,

    /// Batch size for CDC consumption
    batch_size: usize,

    /// Current high-water mark (in memory)
    current_hwm: Versionstamp,

    /// Metrics collector
    metrics: ProjectorMetrics,
}

impl CdcProjector {
    pub async fn new(
        fdb: FdbDatabase,
        rocksdb: Arc<DB>,
        config: &CacheConfig,
    ) -> Result<Self> {
        let consumer_id = format!("cache_projector_{}", config.site_id);

        // Load HWM from RocksDB (survives restarts)
        let current_hwm = Self::load_hwm(&rocksdb)?;

        Ok(Self {
            fdb,
            rocksdb,
            consumer_id,
            batch_size: config.projector_batch_size,
            current_hwm,
            metrics: ProjectorMetrics::default(),
        })
    }

    fn load_hwm(rocksdb: &DB) -> Result<Versionstamp> {
        rocksdb.get(b"m:hwm")?
            .map(|bytes| Versionstamp::from_bytes(&bytes))
            .unwrap_or_else(|| Ok(Versionstamp::zero()))
    }
}
```

#### Projection Loop

```rust
impl CdcProjector {
    pub async fn run(&mut self, shutdown: CancellationToken) -> Result<()> {
        let mut idle_backoff = Duration::from_millis(10);
        let max_backoff = Duration::from_millis(100);

        loop {
            if shutdown.is_cancelled() {
                tracing::info!("CDC Projector shutting down");
                return Ok(());
            }

            match self.project_batch().await {
                Ok(count) if count > 0 => {
                    // Reset backoff on successful work
                    idle_backoff = Duration::from_millis(10);
                    self.metrics.entries_projected.add(count as u64);
                }
                Ok(_) => {
                    // No entries, back off
                    tokio::time::sleep(idle_backoff).await;
                    idle_backoff = (idle_backoff * 2).min(max_backoff);
                }
                Err(e) => {
                    tracing::error!(error = %e, "CDC projection failed");
                    self.metrics.projection_errors.inc();
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }

    async fn project_batch(&mut self) -> Result<usize> {
        // Read batch from FDB CDC stream
        let tx = self.fdb.create_transaction()?;

        let range_start = (db_id, ns, "_cdc", self.current_hwm.next());
        let range_end = (db_id, ns, "_cdc").range_end();

        let entries = tx.get_range(&range_start, &range_end, RangeOption {
            limit: self.batch_size,
            ..default()
        }).await?;

        if entries.is_empty() {
            return Ok(0);
        }

        // Build RocksDB write batch
        let mut batch = WriteBatch::default();
        let mut last_versionstamp = self.current_hwm;

        for (key, value) in &entries {
            let entry: CdcEntry = decode_cbor(value)?;
            last_versionstamp = extract_versionstamp(key);

            for op in &entry.operations {
                self.project_operation(&mut batch, op)?;
            }
        }

        // Write HWM in same batch (atomic)
        batch.put(b"m:hwm", &last_versionstamp.to_bytes());

        // Commit to RocksDB
        self.rocksdb.write(batch)?;
        self.current_hwm = last_versionstamp;

        Ok(entries.len())
    }
}
```

#### Operation Projection

```rust
impl CdcProjector {
    fn project_operation(&self, batch: &mut WriteBatch, op: &CdcOperation) -> Result<()> {
        match op {
            CdcOperation::NodeCreate { entity_type, node_id, properties, home_site } => {
                let key = encode_node_key(&self.db, &self.ns, entity_type, node_id);
                let record = NodeRecord {
                    p: properties.clone(),
                    l: home_site.clone(),
                    c: current_time_micros(),
                    u: current_time_micros(),
                };
                batch.put(&key, &encode_cbor(&record));
            }

            CdcOperation::NodeUpdate { entity_type, node_id, changed_properties, .. } => {
                // Read-modify-write for partial updates
                let key = encode_node_key(&self.db, &self.ns, entity_type, node_id);

                if let Some(existing) = self.rocksdb.get(&key)? {
                    let mut record: NodeRecord = decode_cbor(&existing)?;

                    for (k, v) in changed_properties {
                        if matches!(v, Value::Null) {
                            record.p.remove(k);
                        } else {
                            record.p.insert(k.clone(), v.clone());
                        }
                    }
                    record.u = current_time_micros();

                    batch.put(&key, &encode_cbor(&record));
                }
                // Note: If not in cache, we skip. Cache is not guaranteed complete.
            }

            CdcOperation::NodeDelete { entity_type, node_id } => {
                let key = encode_node_key(&self.db, &self.ns, entity_type, node_id);
                batch.delete(&key);
            }

            CdcOperation::EdgeCreate { source, target, edge_type, properties, .. } => {
                // Forward edge
                let fwd_key = encode_edge_forward_key(
                    &self.db, &self.ns,
                    &source.entity_type, &source.node_id,
                    edge_type,
                    &target.namespace, &target.entity_type, &target.node_id,
                );
                batch.put(&fwd_key, &encode_cbor(properties));

                // Reverse edge
                let rev_key = encode_edge_reverse_key(
                    &self.db, &self.ns,
                    &target.entity_type, &target.node_id,
                    edge_type,
                    &source.namespace, &source.entity_type, &source.node_id,
                );
                batch.put(&rev_key, &[0x01]);
            }

            CdcOperation::EdgeDelete { source, target, edge_type, .. } => {
                let fwd_key = encode_edge_forward_key(
                    &self.db, &self.ns,
                    &source.entity_type, &source.node_id,
                    edge_type,
                    &target.namespace, &target.entity_type, &target.node_id,
                );
                batch.delete(&fwd_key);

                let rev_key = encode_edge_reverse_key(
                    &self.db, &self.ns,
                    &target.entity_type, &target.node_id,
                    edge_type,
                    &source.namespace, &source.entity_type, &source.node_id,
                );
                batch.delete(&rev_key);
            }

            CdcOperation::SchemaRegister { .. } => {
                // Schema not cached in RocksDB (uses in-memory cache)
            }
        }

        Ok(())
    }
}
```

### 18.4 Cache Invalidation Strategy

Cache invalidation is **exact-key** based on CDC operations. There is no TTL-based expiration or probabilistic invalidation.

#### Invalidation Flow

```
CDC Operation Committed (FDB)
         │
         ▼
┌─────────────────────────────────────┐
│ CDC Projector consumes operation     │
└─────────────────────┬───────────────┘
                      │
         ┌────────────┼────────────────┐
         │            │                │
         ▼            ▼                ▼
    NodeCreate    NodeUpdate      NodeDelete
         │            │                │
         ▼            ▼                ▼
   PUT to cache   PUT to cache    DELETE from cache
         │            │                │
         └────────────┼────────────────┘
                      │
                      ▼
         Update HWM (atomic with data)
```

**Key Properties:**

1. **Exact invalidation:** Only keys affected by the CDC operation are modified
2. **No stale reads:** HWM tracking ensures readers detect stale cache state
3. **Atomic projection:** Data writes and HWM update are in the same RocksDB batch
4. **Idempotent replay:** Re-projecting the same CDC entry produces identical results

#### Handling Missed Updates

If a node exists in FDB but not in RocksDB (cache miss), reads fall back to FDB. The cache is **not guaranteed to be complete**. This is by design:

- New nodes may not be cached until accessed
- Evicted nodes are re-populated on demand
- Rebuild populates the cache from CDC replay

### 18.5 Read Path

The read path implements consistency-aware cache lookup with HWM verification.

#### Read Path Flowchart

```
                              GetNode(entity_type, node_id, consistency)
                                              │
                                              ▼
                                   ┌──────────────────┐
                                   │ Check Consistency │
                                   │      Level        │
                                   └────────┬─────────┘
                                            │
              ┌─────────────────────────────┼─────────────────────────────┐
              │                             │                             │
              ▼                             ▼                             ▼
         STRONG                        SESSION                       EVENTUAL
              │                             │                             │
              ▼                             ▼                             ▼
   ┌──────────────────┐        ┌───────────────────────┐      ┌──────────────────┐
   │  Direct FDB read  │        │  RocksDB lookup       │      │  RocksDB lookup   │
   │  at latest version │        │                       │      │                   │
   └──────────────────┘        └───────────┬───────────┘      └─────────┬─────────┘
                                           │                            │
                                           ▼                            │
                               ┌───────────────────────┐                │
                               │  Cache hit?           │                │
                               └───────────┬───────────┘                │
                                    │           │                       │
                                   YES          NO                      │
                                    │           │                       │
                                    ▼           │                       │
                         ┌──────────────────┐   │                       │
                         │  Get FDB read    │   │                       │
                         │  version (GRV)   │   │                       │
                         └────────┬─────────┘   │                       │
                                  │             │                       │
                                  ▼             │                       │
                         ┌──────────────────┐   │                       │
                         │  Get cache HWM   │   │                       │
                         └────────┬─────────┘   │                       │
                                  │             │                       │
                                  ▼             │                       │
                         ┌──────────────────┐   │                       │
                         │  HWM >= GRV?     │   │                       │
                         └────────┬─────────┘   │                       │
                              │       │         │                       │
                             YES      NO        │                       │
                              │       │         │                       │
                              ▼       ▼         │                       │
                         Return   ┌─────────┐   │                       │
                         cached   │ FDB read│◄──┘                       │
                         value    └────┬────┘                           │
                                       │                                │
                                       ▼                                │
                                  Return FDB                            │
                                  value                                 │
                                                                        │
                                                              ┌─────────▼─────────┐
                                                              │  Cache hit?        │
                                                              └─────────┬─────────┘
                                                                   │         │
                                                                  YES        NO
                                                                   │         │
                                                                   ▼         ▼
                                                              Return     FDB read
                                                              cached     (populate
                                                              value      cache)
```

#### Implementation

```rust
pub async fn get_node_cached(
    fdb: &FdbDatabase,
    rocksdb: &DB,
    db: &str,
    ns: &str,
    entity_type: &str,
    node_id: &NodeId,
    consistency: ReadConsistency,
) -> Result<Option<Node>> {
    let key = encode_node_key(db, ns, entity_type, node_id);

    match consistency {
        ReadConsistency::Strong => {
            // Bypass cache entirely
            get_node_from_fdb(fdb, db, ns, entity_type, node_id).await
        }

        ReadConsistency::Session => {
            // Check cache with HWM verification
            if let Some(cached_bytes) = rocksdb.get(&key)? {
                let fdb_version = fdb.get_read_version().await?;
                let cache_hwm = get_cache_hwm(rocksdb)?;

                if cache_hwm >= fdb_version {
                    // Cache is fresh enough
                    let record: NodeRecord = decode_cbor(&cached_bytes)?;
                    return Ok(Some(to_node(entity_type, node_id, record)));
                }
                // Cache is stale, fall through to FDB
            }

            get_node_from_fdb(fdb, db, ns, entity_type, node_id).await
        }

        ReadConsistency::Eventual => {
            // Check cache, no HWM verification
            if let Some(cached_bytes) = rocksdb.get(&key)? {
                let record: NodeRecord = decode_cbor(&cached_bytes)?;
                return Ok(Some(to_node(entity_type, node_id, record)));
            }

            // Cache miss, read from FDB and optionally populate cache
            let node = get_node_from_fdb(fdb, db, ns, entity_type, node_id).await?;

            if let Some(ref n) = node {
                // Populate cache on miss (read-through)
                let record = NodeRecord {
                    p: n.properties.clone(),
                    l: n.locality.clone(),
                    c: n.created_at,
                    u: n.updated_at,
                };
                rocksdb.put(&key, &encode_cbor(&record))?;
            }

            Ok(node)
        }
    }
}

fn get_cache_hwm(rocksdb: &DB) -> Result<Versionstamp> {
    rocksdb.get(b"m:hwm")?
        .map(|bytes| Versionstamp::from_bytes(&bytes))
        .unwrap_or_else(|| Ok(Versionstamp::zero()))
}
```

### 18.6 High-Water Mark Tracking

The high-water mark (HWM) is a FDB versionstamp that represents the last CDC entry successfully projected to RocksDB.

#### HWM Semantics

| Consistency | HWM Check | Behavior |
|-------------|-----------|----------|
| Strong | None | Always read from FDB |
| Session | `cache_hwm >= fdb_read_version` | Serve from cache if HWM is at or past current FDB version |
| Eventual | None | Serve from cache unconditionally |

#### Session Consistency Guarantee

Session consistency ensures "read-your-writes" semantics:

1. Client writes node via FDB (CDC entry at version V1)
2. CDC Projector processes entry, updates HWM to V1
3. Client reads with Session consistency
4. Read acquires FDB read version (V2, where V2 >= V1)
5. If HWM >= V2, cache is fresh — return cached value
6. If HWM < V2, cache is stale — read from FDB

```rust
async fn check_session_freshness(
    fdb: &FdbDatabase,
    rocksdb: &DB,
) -> Result<bool> {
    // Get current FDB read version (cheap, no transaction needed)
    let fdb_version = fdb.get_read_version().await?;

    // Get cache HWM from RocksDB
    let cache_hwm = get_cache_hwm(rocksdb)?;

    // Cache is fresh if HWM >= FDB version
    Ok(cache_hwm.to_u64() >= fdb_version)
}
```

#### Per-Namespace HWM (Future Enhancement)

For deployments with many namespaces, per-namespace HWM tracking reduces false staleness:

```rust
fn get_namespace_hwm(rocksdb: &DB, db: &str, ns: &str) -> Result<Versionstamp> {
    let key = format!("m:hwm:{}:{}", db, ns);
    rocksdb.get(key.as_bytes())?
        .map(|bytes| Versionstamp::from_bytes(&bytes))
        .unwrap_or_else(|| Ok(Versionstamp::zero()))
}
```

### 18.7 Traversal Caching

Edge adjacency information is cached in RocksDB to accelerate multi-hop traversals.

#### Adjacency Key Layout

Forward edges support efficient "find all targets from source" queries:

```
e:<db>:<ns>:<src_type>:<src_id>:<label>:*
```

Reverse edges support efficient "find all sources to target" queries:

```
r:<db>:<ns>:<tgt_type>:<tgt_id>:<label>:*
```

#### Cached Traversal

```rust
pub async fn traverse_cached(
    fdb: &FdbDatabase,
    rocksdb: &DB,
    start: &NodeRef,
    direction: EdgeDirection,
    label: &str,
    consistency: ReadConsistency,
) -> Result<Vec<NodeRef>> {
    // Build prefix for RocksDB range scan
    let prefix = match direction {
        EdgeDirection::Outgoing => {
            format!("e:{}:{}:{}:{}:{}:",
                start.db, start.ns, start.entity_type, start.node_id, label)
        }
        EdgeDirection::Incoming => {
            format!("r:{}:{}:{}:{}:{}:",
                start.db, start.ns, start.entity_type, start.node_id, label)
        }
    };

    match consistency {
        ReadConsistency::Strong => {
            // Bypass cache, read from FDB
            traverse_from_fdb(fdb, start, direction, label).await
        }

        ReadConsistency::Session | ReadConsistency::Eventual => {
            // Check cache HWM for Session, skip for Eventual
            if consistency == ReadConsistency::Session {
                if !check_session_freshness(fdb, rocksdb).await? {
                    return traverse_from_fdb(fdb, start, direction, label).await;
                }
            }

            // Range scan in RocksDB
            let mut results = Vec::new();
            let iter = rocksdb.prefix_iterator(prefix.as_bytes());

            for item in iter {
                let (key, _value) = item?;
                let key_str = String::from_utf8_lossy(&key);

                // Parse target from key suffix
                let target = parse_edge_target(&key_str, direction)?;
                results.push(target);
            }

            Ok(results)
        }
    }
}
```

### 18.8 Cache Warm-up and Rebuild

The cache can be rebuilt from CDC replay or warm-up from targeted reads.

#### Rebuild from CDC Replay

On startup or after cache corruption, the projector can replay the full CDC stream:

```rust
pub async fn rebuild_cache(
    fdb: &FdbDatabase,
    rocksdb: &DB,
    config: &RebuildConfig,
) -> Result<RebuildStats> {
    // Record rebuild start
    let status = RebuildStatus {
        state: "running",
        started_at: current_time_micros(),
        progress_pct: 0.0,
        entries_processed: 0,
    };
    rocksdb.put(b"m:rebuild:status", &serde_json::to_vec(&status)?)?;

    // Clear existing cache data (keep metadata)
    clear_cache_data(rocksdb)?;

    // Start from zero versionstamp
    let mut hwm = Versionstamp::zero();
    let mut total_entries = 0;

    loop {
        let tx = fdb.create_transaction()?;

        let range_start = (db_id, ns, "_cdc", hwm.next());
        let range_end = (db_id, ns, "_cdc").range_end();

        let entries = tx.get_range(&range_start, &range_end, RangeOption {
            limit: config.rebuild_batch_size,
            ..default()
        }).await?;

        if entries.is_empty() {
            break;
        }

        let mut batch = WriteBatch::default();

        for (key, value) in &entries {
            let entry: CdcEntry = decode_cbor(value)?;
            hwm = extract_versionstamp(key);

            for op in &entry.operations {
                project_operation_to_batch(&mut batch, op)?;
            }
            total_entries += 1;
        }

        batch.put(b"m:hwm", &hwm.to_bytes());
        rocksdb.write(batch)?;

        // Update progress
        if total_entries % 10000 == 0 {
            let status = RebuildStatus {
                state: "running",
                started_at: status.started_at,
                progress_pct: 0.0, // Unknown total
                entries_processed: total_entries,
            };
            rocksdb.put(b"m:rebuild:status", &serde_json::to_vec(&status)?)?;
        }
    }

    // Mark rebuild complete
    let final_status = RebuildStatus {
        state: "complete",
        started_at: status.started_at,
        progress_pct: 1.0,
        entries_processed: total_entries,
    };
    rocksdb.put(b"m:rebuild:status", &serde_json::to_vec(&final_status)?)?;

    Ok(RebuildStats {
        entries_processed: total_entries,
        duration: Instant::now().duration_since(start),
    })
}

fn clear_cache_data(rocksdb: &DB) -> Result<()> {
    // Delete all node and edge keys, preserve metadata
    let mut batch = WriteBatch::default();

    // Delete node data
    let node_iter = rocksdb.prefix_iterator(b"n:");
    for item in node_iter {
        let (key, _) = item?;
        batch.delete(&key);
    }

    // Delete edge data
    let edge_iter = rocksdb.prefix_iterator(b"e:");
    for item in edge_iter {
        let (key, _) = item?;
        batch.delete(&key);
    }

    let rev_iter = rocksdb.prefix_iterator(b"r:");
    for item in rev_iter {
        let (key, _) = item?;
        batch.delete(&key);
    }

    rocksdb.write(batch)?;
    Ok(())
}
```

#### Warm-up Strategy

Optional warm-up populates the cache with frequently accessed data on startup:

```rust
pub async fn warm_cache(
    fdb: &FdbDatabase,
    rocksdb: &DB,
    config: &WarmupConfig,
) -> Result<WarmupStats> {
    let mut warmed_nodes = 0;

    // Strategy 1: Recently updated nodes (hot data)
    if config.warm_recent {
        let recent = query_recent_nodes(fdb, config.recent_window).await?;
        for node in recent {
            cache_node(rocksdb, &node)?;
            warmed_nodes += 1;
        }
    }

    // Strategy 2: Specific entity types (schema-driven)
    for entity_type in &config.warm_types {
        let nodes = query_all_nodes(fdb, entity_type, config.warm_limit).await?;
        for node in nodes {
            cache_node(rocksdb, &node)?;
            warmed_nodes += 1;
        }
    }

    Ok(WarmupStats { warmed_nodes })
}
```

### 18.9 Configuration

Cache layer configuration is part of the server configuration.

#### Configuration Options

| Environment Variable | CLI Flag | Default | Description |
|---------------------|----------|---------|-------------|
| `PELAGO_CACHE_ENABLED` | `--cache-enabled` | `true` | Enable RocksDB cache layer |
| `PELAGO_CACHE_PATH` | `--cache-path` | `./data/cache` | RocksDB data directory |
| `PELAGO_CACHE_SIZE_MB` | `--cache-size` | `1024` | RocksDB block cache size (MB) |
| `PELAGO_CACHE_WRITE_BUFFER_MB` | `--cache-write-buffer` | `64` | Write buffer size (MB) |
| `PELAGO_CACHE_MAX_WRITE_BUFFERS` | `--cache-max-buffers` | `3` | Max write buffer count |
| `PELAGO_CACHE_PROJECTOR_BATCH` | `--cache-batch` | `1000` | CDC projector batch size |
| `PELAGO_CACHE_WARM_ON_START` | `--cache-warm` | `false` | Enable cache warm-up on startup |
| `PELAGO_CACHE_WARM_TYPES` | `--cache-warm-types` | (none) | Entity types to warm (comma-separated) |

#### RocksDB Tuning

```rust
pub fn create_cache_options(config: &CacheConfig) -> Options {
    let mut opts = Options::default();

    // Create if not exists
    opts.create_if_missing(true);

    // Block cache (read cache)
    let cache = Cache::new_lru_cache(config.cache_size_mb * 1024 * 1024);
    let mut block_opts = BlockBasedOptions::default();
    block_opts.set_block_cache(&cache);
    block_opts.set_bloom_filter(10.0, false); // 10 bits per key
    opts.set_block_based_table_factory(&block_opts);

    // Write buffer
    opts.set_write_buffer_size(config.write_buffer_mb * 1024 * 1024);
    opts.set_max_write_buffer_number(config.max_write_buffers);

    // Compaction
    opts.set_level_compaction_dynamic_level_bytes(true);
    opts.set_max_background_jobs(4);

    // Compression
    opts.set_compression_type(DBCompressionType::Lz4);

    opts
}
```

#### Example Configuration

```bash
# Production configuration
PELAGO_CACHE_ENABLED=true \
PELAGO_CACHE_PATH=/var/lib/pelago/cache \
PELAGO_CACHE_SIZE_MB=4096 \
PELAGO_CACHE_WRITE_BUFFER_MB=128 \
PELAGO_CACHE_PROJECTOR_BATCH=2000 \
PELAGO_CACHE_WARM_ON_START=true \
PELAGO_CACHE_WARM_TYPES=User,Account,Session \
pelago-server
```

### 18.10 Eviction Policy

RocksDB manages eviction automatically via LRU block cache. Additional application-level eviction is not implemented in Phase 1.

#### Block Cache LRU

RocksDB's block cache evicts least-recently-used blocks when the cache is full:

- Hot data remains in cache
- Cold data is evicted and re-read from disk or FDB on demand
- Block size is 4KB by default

#### Future: TTL-Based Eviction

Phase 2 may add TTL-based eviction for specific use cases:

```rust
// Future enhancement: TTL column family
fn create_ttl_column_family(config: &CacheConfig) -> Result<()> {
    // Entries automatically expire after TTL
    // Useful for session data, temporary caches
}
```

### 18.11 Metrics

The cache layer exposes metrics for monitoring.

| Metric | Type | Description |
|--------|------|-------------|
| `pelago_cache_hits_total` | Counter | Cache hit count |
| `pelago_cache_misses_total` | Counter | Cache miss count |
| `pelago_cache_hit_ratio` | Gauge | Hit ratio (hits / total) |
| `pelago_cache_hwm_lag_ms` | Gauge | HWM lag behind FDB (milliseconds) |
| `pelago_cache_projector_entries` | Counter | CDC entries projected |
| `pelago_cache_projector_errors` | Counter | Projection errors |
| `pelago_cache_rocksdb_size_bytes` | Gauge | RocksDB disk usage |
| `pelago_cache_rocksdb_block_cache_usage` | Gauge | Block cache memory usage |

```rust
pub struct CacheMetrics {
    pub hits: Counter,
    pub misses: Counter,
    pub hwm_lag_ms: Gauge,
    pub projector_entries: Counter,
    pub projector_errors: Counter,
}

impl CacheMetrics {
    pub fn record_hit(&self) {
        self.hits.inc();
    }

    pub fn record_miss(&self) {
        self.misses.inc();
    }

    pub fn hit_ratio(&self) -> f64 {
        let hits = self.hits.get();
        let total = hits + self.misses.get();
        if total == 0 { 0.0 } else { hits as f64 / total as f64 }
    }
}
```

---
