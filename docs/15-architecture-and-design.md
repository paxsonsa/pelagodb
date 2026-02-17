# Architecture and Design

This page explains how PelagoDB is put together, why key design decisions were made, and what tradeoffs they introduce.

Related implementation sources:
- `crates/pelago-server/src/main.rs`
- `crates/pelago-api/src`
- `crates/pelago-storage/src`
- `crates/pelago-query/src`
- `proto/pelago.proto`

## Design Goals

- Strong correctness and transactional consistency on core graph data.
- Schema-first modeling with explicit data-shape contracts.
- Multi-site replication via CDC pull workers.
- Operational surfaces for auth, audit, watch, and administration.
- A practical developer interface: gRPC + CLI + REPL + SDKs.

## Non-Goals

- A pure in-memory graph engine optimized only for deep analytic traversals.
- Lowest-latency single-node graph database at all costs.
- Feature parity with every mature graph ecosystem out of the box.

## High-Level Runtime Architecture

```text
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

## Core Data Flow

## 1) Write Path

1. Request hits service handler (schema/node/edge/admin).
2. AuthZ checks run (if enabled).
3. Storage layer validates schema and applies mutation in FoundationDB transaction.
4. CDC entry is appended for downstream consumers (replication, watch, cache projector).
5. Response returns with persisted entity state.

Key tradeoff:
- Strong transactional behavior with explicit consistency.
- Higher operational complexity than single-process embedded graph stores.

## 2) Read Path

1. Query/Node/Edge request enters gRPC service.
2. Planner chooses indexed lookup / scan / traversal strategy.
3. Reads are served from FoundationDB and optionally accelerated by cache read paths.
4. Results stream to client with continuation support where applicable.

Key tradeoff:
- Predictable semantics and unified APIs.
- Some query patterns may require modeling/index tuning to hit target p99 latency.

## 3) Replication Path

1. Dedicated replicator workers pull remote CDC (`ReplicationService.PullCdcEvents`).
2. Replica-safe apply methods enforce ownership and conflict policy.
3. Applied remote changes are mirrored into local CDC.
4. Local caches/watch streams converge through the same CDC backbone.

Key tradeoff:
- Clear operational separation of API nodes and replication workers.
- Requires explicit replication topology/configuration and monitoring.

## 4) Conflict and Availability Semantics

PelagoDB is designed to stay writable under cross-DC partitions, with explicit conflict behavior:

1. Existing entity updates/mutations are guarded by ownership semantics.
2. Create-time conflicts for logically new data can still happen when partitions allow concurrent writes.
3. Replica apply resolves those new-data conflicts with deterministic latest-write-wins (LWW).
4. Conflict signals are surfaced through audit/replication paths for downstream reconciliation.

Key tradeoff:
- Better availability and continued writes during partition events.
- Potential dead-letter/reconciliation work for create-time conflict cases.

## 5) Watch Path

1. Client subscribes via point/query/namespace watch.
2. Service validates scope, limits, and resume position.
3. Watch loop reads CDC increments and filters into watch events.
4. Stream continues until cancelled/expired/error.

Key tradeoff:
- Unified change-stream model with resume semantics.
- Requires queue/TTL tuning to avoid pressure in high-churn namespaces.

## Storage and Consistency Model

- FoundationDB is the transactional source of truth.
- Schema definitions are versioned and validated server-side.
- IDs are allocated with site-aware semantics.
- Namespace boundaries are first-class for isolation and lifecycle.

Why this choice:
- Favor deterministic correctness and predictable write behavior.
- Accept more infrastructure coupling in exchange for consistency guarantees.

## Query Model

PelagoDB supports two query surfaces:
- CEL-style filtered retrieval.
- PQL graph-oriented syntax and execution.

Execution pipeline:
- Parse -> resolve against schema -> compile -> execute -> stream results.

Why this choice:
- Keep a low-friction filter path and a graph-native path in one system.
- Avoid forcing every use case through one query abstraction.

## Caching Model

- Optional RocksDB read cache sits beside transactional store.
- CDC projector updates cache to converge with committed state.
- Cache is an optimization layer, not system-of-record.

Why this choice:
- Improve read performance for common access paths.
- Preserve correctness by treating cache as derivative state.

## Security Model

- Auth can be optional (dev) or required (production).
- Authorization policy checks run at service boundaries.
- Audit records capture security and admin activities.

Why this choice:
- Make production controls first-class without blocking local prototyping.

## Operational Deployment Model

Recommended split:
- API/query nodes: `PELAGO_REPLICATION_ENABLED=false`
- Replicator nodes: `PELAGO_REPLICATION_ENABLED=true`

This separates client-serving capacity from replication throughput concerns.

## Design Tradeoffs Summary

Benefits:
- Strong transactional core.
- Schema and policy guardrails.
- Multi-site replication design with clear ownership semantics.
- Cross-tenant relationship modeling with explicit mutation safety boundaries.
- Unified CDC backbone for replication, watch, and cache projection.

Costs:
- More moving parts than single-node graph databases.
- Requires operational discipline for replication/cache/watch tuning.
- Ecosystem and tooling depth may be narrower than longest-established graph platforms.
