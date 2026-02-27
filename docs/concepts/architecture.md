# Architecture

How PelagoDB is put together, why key design decisions were made, and what tradeoffs they introduce.

## Design Goals

- Strong correctness and transactional consistency on core graph data
- Schema-first modeling with explicit data-shape contracts
- Multi-site replication via CDC pull workers
- Operational surfaces for auth, audit, watch, and administration
- A practical developer interface: gRPC + CLI + REPL + SDKs

## Non-Goals

- A pure in-memory graph engine optimized only for deep analytic traversals
- Lowest-latency single-node graph database at all costs
- Feature parity with every mature graph ecosystem out of the box

## Crate Structure

| Crate | Responsibility |
|---|---|
| `pelago-server` | gRPC server, service wiring, runtime lifecycle |
| `pelago-api` | Service handler implementations |
| `pelago-core` | Types, schema, config, errors, encoding |
| `pelago-storage` | FoundationDB storage, CDC, caching, subspaces |
| `pelago-query` | CEL evaluator, PQL parser/compiler/executor |
| `pelago-cli` | CLI binary, REPL, output formatting |

## High-Level Runtime

```
Clients (CLI/SDKs/grpcurl)
        |
        v
pelago-server (gRPC services)
  - SchemaService
  - NodeService / EdgeService
  - QueryService
  - AdminService
  - WatchService
  - ReplicationService
  - AuthService / HealthService
        |
        v
Storage + Query Core
  - FoundationDB transactional store
  - Schema registry + ID allocator
  - Query planner/executor + traversal engine
  - CDC log + replication checkpoints
  - Optional RocksDB read cache/projector
```

## Data Flow

### Write Path

1. Request hits service handler (schema/node/edge/admin).
2. AuthZ checks run (if enabled).
3. Storage layer validates schema and applies mutation in FDB transaction.
4. CDC entry is appended for downstream consumers (replication, watch, cache projector).
5. Response returns with persisted entity state.

**Tradeoff:** Strong transactional behavior with explicit consistency. Higher operational complexity than single-process embedded stores.

### Read Path

1. Query/Node/Edge request enters gRPC service.
2. Planner chooses indexed lookup / scan / traversal strategy.
3. Reads are served from FDB and optionally accelerated by cache.
4. Results stream to client with continuation support.

**Tradeoff:** Predictable semantics and unified APIs. Some query patterns require modeling/index tuning for target p99 latency.

### Replication Path

1. Dedicated replicator workers pull remote CDC (`ReplicationService.PullCdcEvents`).
2. Replica-safe apply methods enforce ownership and conflict policy.
3. Applied remote changes are mirrored into local CDC.
4. Local caches/watch streams converge through the same CDC backbone.

**Tradeoff:** Clear operational separation of API nodes and replication workers. Requires explicit topology/configuration and monitoring.

### Watch Path

1. Client subscribes via point/query/namespace watch.
2. Service validates scope, limits, and resume position.
3. Watch loop reads CDC increments and filters into watch events.
4. Stream continues until cancelled/expired/error.

**Tradeoff:** Unified change-stream model with resume semantics. Requires queue/TTL tuning for high-churn namespaces.

## Storage and Consistency

- FoundationDB is the transactional source of truth
- Schema definitions are versioned and validated server-side
- IDs are allocated with site-aware semantics
- Namespace boundaries are first-class for isolation and lifecycle

## Conflict and Availability

PelagoDB stays writable under cross-DC partitions:

1. Existing entity updates/mutations are guarded by ownership semantics.
2. Create-time conflicts for logically new data can occur under concurrent writes.
3. Replica apply resolves new-data conflicts with deterministic LWW.
4. Conflict signals are surfaced through audit/replication paths.

**Tradeoff:** Better availability during partitions. Potential reconciliation work for create-time conflicts.

## Deployment Model

Recommended split:
- **API/query nodes:** `PELAGO_REPLICATION_ENABLED=false`
- **Replicator nodes:** `PELAGO_REPLICATION_ENABLED=true`

This separates client-serving capacity from replication throughput.

## Design Tradeoffs Summary

| Benefit | Cost |
|---|---|
| Strong transactional core | More moving parts than single-node graph databases |
| Schema and policy guardrails | Requires operational discipline |
| Multi-site replication with ownership | Replication/cache/watch tuning needed |
| Cross-tenant relationships | Deliberate modeling required |
| Unified CDC backbone | Monitoring multiple downstream consumers |

## Related

- [Data Model](data-model.md) — nodes, edges, properties
- [CDC and Event Model](cdc-and-event-model.md) — CDC backbone
- [Replication](replication.md) — multi-site design
- [Caching and Consistency](caching-and-consistency.md) — read acceleration
- [Security Model](security-model.md) — auth and authorization
