# Capacity Planning

Sizing, namespace partitioning, and performance budgets for PelagoDB deployments.

## Sizing Dimensions

### Storage

PelagoDB storage is FoundationDB-backed. Key sizing factors:

| Factor | Impact |
|---|---|
| Node count per namespace | FDB key-value pairs per node (properties + indexes) |
| Edge count per node | Additional KV pairs per edge (forward + optional reverse) |
| Index count per property | Write amplification proportional to indexed properties |
| Property sizes | Larger values (bytes, long strings) increase storage per node |

Rule of thumb: Each indexed property adds ~1 additional KV pair per node. A node with 5 indexed properties generates ~6 KV writes.

### Throughput

| Workload | Bottleneck | Scaling Lever |
|---|---|---|
| Point reads | FDB read latency + optional cache | Add API nodes, enable cache |
| Indexed queries | FDB range scan + index selectivity | Better indexes, cache |
| Writes | FDB transaction throughput | Namespace partitioning |
| Traversals | Fan-out per hop × depth | Bound depth/results, better modeling |
| Replication | CDC volume × poll frequency | Batch size, dedicated replicators |

### Latency Budgets

Define p50/p95/p99 targets per operation type:

| Operation | Suggested p99 Target |
|---|---|
| Node point read | 1 ms |
| Indexed query (small result) | 10 ms |
| Traversal (bounded) | 100 ms |
| Write (single node) | 5 ms |

Validate with `scripts/perf-benchmark.py --enforce-targets`.

## Namespace Partitioning Strategy

### Why Partition

Namespaces are the primary unit of:
- Replication scope
- Cache projector scope
- Performance isolation
- Lifecycle management

### Partitioning Guidelines

| Pattern | When |
|---|---|
| One namespace per tenant | Clear tenant isolation needed |
| Shared + per-tenant namespaces | Global reference data + tenant-specific data |
| Hot/cold namespace split | Separate high-churn operational data |
| Per-environment namespaces | Dev/staging/production isolation |

### Namespace Sizing

Aim for namespaces that:
- Fit within FDB transaction size limits for bulk operations
- Have manageable replication lag (< 1s under steady state)
- Don't create hotspot partitions in FDB

## Cache Sizing

| Parameter | Guidance |
|---|---|
| `PELAGO_CACHE_SIZE_MB` | Size for working set of hot reads |
| `PELAGO_CACHE_WRITE_BUFFER_MB` | 64 MB is typical |
| `PELAGO_CACHE_PROJECTOR_BATCH_SIZE` | Increase for high CDC volume |
| `PELAGO_CACHE_PROJECTOR_SCOPES` | Limit to hot namespaces |

## Replication Sizing

| Parameter | Guidance |
|---|---|
| `PELAGO_REPLICATION_BATCH_SIZE` | 512 default; increase for high volume |
| `PELAGO_REPLICATION_POLL_MS` | 300 ms default; decrease for lower lag |
| Replicator count | 1 active per scope (lease-gated); standby for failover |

## Monitoring Checkpoints

Regularly check:
- Replication lag per scope
- Cache projector lag
- Watch queue pressure and dropped events
- FDB transaction conflict rate
- Query p99 latency trends

## Related

- [Data Modeling Patterns](data-modeling-patterns.md) — modeling for performance
- [Query Optimization](query-optimization.md) — tuning reads
- [Configuration Reference](../reference/configuration.md) — all settings
- [Production Checklist](../operations/production-checklist.md) — pre-production validation
- [Daily Operations](../operations/daily-operations.md) — benchmark procedures
