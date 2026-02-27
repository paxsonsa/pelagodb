# CDC and Event Model

Change Data Capture (CDC) is the backbone that powers replication, watch subscriptions, and cache projection in PelagoDB.

## What CDC Does

Every mutation (node create/update/delete, edge create/delete, schema change) appends a CDC entry in the FoundationDB transaction. This single event stream drives three downstream systems:

```
Mutation → FDB Transaction → CDC Entry
                                  |
                    ┌─────────────┼─────────────┐
                    v             v             v
              Replication     Watch          Cache
              (pull CDC)    (stream CDC)   (project CDC)
```

## CDC Entry Structure

Each CDC entry is ordered by FoundationDB versionstamp and contains:
- The mutation type (create, update, delete)
- The affected entity (node or edge)
- The namespace and database scope
- The originating site ID

## How Replication Uses CDC

Replicator workers on remote sites pull CDC events via `ReplicationService.PullCdcEvents`:

1. Pull with `source_site` filter and resume from last checkpoint.
2. Apply replica-safe operations with ownership/LWW enforcement.
3. Mirror successfully applied operations into **local** CDC.
4. Checkpoint the remote versionstamp position.

This "mirror into local CDC" step is critical — it ensures cache projectors and watch streams on receiving sites also converge with replicated data without needing their own pull replicators.

## How Watch Uses CDC

Watch subscriptions (`WatchPoint`, `WatchQuery`, `WatchNamespace`) read CDC increments in a loop:

1. Subscribe with optional `resume_after` versionstamp.
2. Watch loop reads new CDC entries since last position.
3. Filter entries against subscription scope (entity, query match, namespace).
4. Emit matching entries as `WatchEvent` on the gRPC stream.

Event types: `ENTER`, `UPDATE`, `EXIT`, `DELETE`

## How Cache Projection Uses CDC

The optional RocksDB cache projector:

1. Consumes local CDC entries (including replicated ones).
2. Projects node/edge state into RocksDB for fast read paths.
3. Processes in batches controlled by `PELAGO_CACHE_PROJECTOR_BATCH_SIZE`.
4. Scopes can be limited with `PELAGO_CACHE_PROJECTOR_SCOPES`.

Cache is always derivative state — FoundationDB remains the source of truth.

## Ordering Guarantees

- CDC entries within a single FDB transaction are atomically ordered.
- Versionstamps provide total ordering across transactions on the same cluster.
- Cross-site ordering relies on replication checkpoints — events may arrive with variable lag but are applied in source order per scope.

## Operational Implications

- **Replication lag** is the delay between a remote CDC entry and its local application. Monitor via `pelago admin replication-status`.
- **Watch pressure** occurs when CDC volume exceeds consumer processing speed. Tune `PELAGO_WATCH_MAX_QUEUE_SIZE` and alert on dropped events.
- **Cache staleness** depends on projector throughput. Increase batch size or reduce scope for high-volume namespaces.

## Related

- [Replication](replication.md) — multi-site replication design
- [Caching and Consistency](caching-and-consistency.md) — read cache model
- [Watch API Reference](../reference/watch-api.md) — subscription RPCs
- [Architecture](architecture.md) — system overview
