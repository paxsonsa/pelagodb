# Caching and Consistency

PelagoDB uses an optional RocksDB read cache alongside FoundationDB's transactional store, with configurable consistency levels per request.

## Cache Architecture

- **FoundationDB** is the transactional source of truth.
- **RocksDB cache** sits beside the transactional store as an optimization layer.
- **CDC projector** updates the cache to converge with committed state.

The cache is always derivative state, never system-of-record.

## Cache Projection

The CDC projector consumes local CDC entries (including replicated ones) and projects node/edge state into RocksDB:

1. Read new CDC entries since last projection checkpoint.
2. Apply state changes to RocksDB.
3. Advance checkpoint.

This means replicated data from remote sites also appears in the local cache after replication and projection converge.

### Cache Configuration

| Variable | Notes |
|---|---|
| `PELAGO_CACHE_ENABLED` | Enable/disable cache (default: `true`) |
| `PELAGO_CACHE_PATH` | Cache directory |
| `PELAGO_CACHE_SIZE_MB` | Maximum cache size |
| `PELAGO_CACHE_WRITE_BUFFER_MB` | RocksDB write buffer |
| `PELAGO_CACHE_MAX_WRITE_BUFFERS` | Concurrent write buffers |
| `PELAGO_CACHE_PROJECTOR_BATCH_SIZE` | CDC entries per projection batch |
| `PELAGO_CACHE_PROJECTOR_SCOPES` | Optional CSV scopes to project |

## Read Consistency Levels

Per-request controls:

| Level | Behavior |
|---|---|
| `READ_CONSISTENCY_STRONG` | Read from FDB, guaranteed latest committed state |
| `READ_CONSISTENCY_SESSION` | Session-consistent reads |
| `READ_CONSISTENCY_EVENTUAL` | May read from cache, may lag behind latest commit |

## Snapshot Modes

| Mode | Behavior |
|---|---|
| `SNAPSHOT_MODE_STRICT` | Enforces guardrails on elapsed time, scanned keys, and result bytes |
| `SNAPSHOT_MODE_BEST_EFFORT` | Returns what's available within budget |

Default snapshot modes when unspecified:
- `FindNodes`: `STRICT`
- `Traverse`: `BEST_EFFORT`
- `ExecutePQL`: `BEST_EFFORT`

### Strict Snapshot Guardrails

| Limit | Value |
|---|---|
| Max elapsed | ~2000 ms |
| Max scanned keys | ~50,000 |
| Max result bytes | ~8 MiB |

When strict limits are exceeded:
- If `allow_degrade_to_best_effort=true`: response is marked degraded
- Otherwise: request fails with snapshot budget error

## Cache Convergence Under Replication

Cache convergence across sites follows this path:

1. Remote mutation → remote CDC
2. Local replicator pulls remote CDC
3. Replicator mirrors applied operations into local CDC
4. Local cache projector consumes local CDC
5. Cache reflects replicated state

Read nodes remain coherent without running their own pull replicators — they only need the cache projector consuming local CDC.

## When to Use Each Consistency Level

| Use Case | Recommended |
|---|---|
| User-facing reads requiring latest state | `STRONG` |
| Dashboard queries tolerating slight delay | `EVENTUAL` |
| Traversals with bounded latency budgets | `BEST_EFFORT` snapshot |
| Background analytics | `EVENTUAL` with `BEST_EFFORT` |

## Related

- [CDC and Event Model](cdc-and-event-model.md) — how CDC drives cache projection
- [Architecture](architecture.md) — read path design
- [Query Optimization](../guides/query-optimization.md) — tuning strategies
- [Configuration Reference](../reference/configuration.md) — cache settings
