# When to Use PelagoDB

This page is intentionally candid. PelagoDB is a strong fit for some teams and a poor fit for others.

## Why PelagoDB Exists

PelagoDB was designed for global data modeling across multiple data centers where all of these are simultaneously true:

- You need high availability and continued writability during partitions.
- You need relationship-rich graph queries across organizational boundaries.
- You cannot rely on only database- or namespace-level write partitioning.
- You need mutation safety at a lower level (entity ownership), not just tenant isolation.

**Design intent:** Keep updates/mutations safe and conflict-aware, keep global relationships queryable, keep the system writable under partition, and surface conflicts explicitly instead of silently corrupting state.

## Short Positioning

PelagoDB is best viewed as:
- A schema-first, FoundationDB-backed graph platform
- With explicit multi-site replication and ownership controls
- Plus operationally integrated auth/audit/watch surfaces
- And support for cross-tenant relationship modeling when required

It is **not** trying to be:
- The most mature graph analytics ecosystem
- The easiest zero-ops graph database for all workloads

## Tradeoffs vs Other Graph Databases

| Dimension | PelagoDB | Neo4j | Dgraph | JanusGraph |
|---|---|---|---|---|
| Core posture | Schema-first graph on transactional KV foundation | Mature property graph platform | Distributed graph with GraphQL/DQL focus | Graph layer over pluggable backends |
| Data-shape governance | Strong schema contracts and validation | Flexible model with constraints/indexes as needed | Schema-driven GraphQL model is common | Depends on backend/config choices |
| Query surfaces | CEL + PQL + gRPC | Cypher ecosystem depth | GraphQL/DQL workflows | Gremlin/TinkerPop ecosystem |
| Multi-site model | Explicit CDC pull replication + ownership controls | Varies by edition/deployment | Different replication semantics | Depends on backend topology |
| Cross-tenant relationships | First-class when modeled intentionally | Possible, often app/policy-dependent | Possible, model-dependent | Possible, backend/model-dependent |
| Operational model | More explicit infra control, more knobs | Broad operational tooling ecosystem | Strong distributed-first posture | High flexibility, higher assembly/ops complexity |
| Ecosystem maturity | Growing | Very mature | Mature in distributed GraphQL use cases | Mature in custom stack deployments |

## Who Should Use PelagoDB

PelagoDB is a strong fit if most are true:
- You need strict schema and policy control around graph data
- You need global graph relationships that can cross tenant boundaries
- You need multi-site replication with clear ownership behavior
- You accept explicit tradeoffs during partitions to preserve writability
- Your team can own operational complexity (replication, cache, watch limits)

## Who Should Not Use PelagoDB

Prefer alternatives if most are true:
- You need the deepest out-of-the-box graph analytics ecosystem right now
- You want the broadest prebuilt query IDE and visualization tooling
- You want minimal platform ownership and mostly managed operations
- You require immediate Cypher/Gremlin compatibility with existing estates
- You cannot accept LWW resolution for conflicting new-entity creation during partitions

## Honest Benefits

- Strong correctness posture with transactional core
- Schema-first guardrails reduce drift in large teams
- Unified CDC backbone powers replication, watch, and cache convergence
- Cross-tenant relationship modeling is possible without giving up tenant-aware operations
- Security/authorization/audit are part of the runtime model, not afterthoughts

## Honest Costs

- Steeper operational learning curve than simpler graph offerings
- Fewer ecosystem conveniences than longest-established graph products
- Requires intentional data modeling and capacity planning for strong p99 behavior
- Under partition, conflicting new-data creation is resolved by LWW and may require downstream reconciliation

## Decision Checklist

**Use PelagoDB** if most answers are "yes":
- Do we need strict schema and policy enforcement?
- Do we need explicit multi-site data control and per-entity ownership semantics?
- Do we need cross-tenant graph relationships in one global model?
- Are we comfortable operating a system with replication/cache/watch tuning?
- Can our clients own reconciliation for create-time conflict edge cases?

**Prefer alternatives** if most answers are "yes":
- Do we prioritize prebuilt ecosystem depth over platform control?
- Do we need a fully managed, lowest-effort graph runtime first?
- Is our team standardized on Cypher or Gremlin workflows?
- Do we require strong consistency for all conflict classes without reconciliation?

## Suggested Evaluation Plan

1. Model one production-like namespace with real tenant boundaries and cross-tenant edges.
2. Run expected write/query/replication/watch traffic on representative datasets.
3. Run partition/failover drills and record conflict rates, lag, and operational toil.
4. Verify that create-time conflict handling and reconciliation flows are acceptable.

If PelagoDB wins on control, global modeling fit, and acceptable operations burden, it is likely the right choice.

## Related

- [Architecture](architecture.md) — system design and tradeoffs
- [Data Model](data-model.md) — nodes, edges, properties
- [Replication](replication.md) — multi-site design
- [Comparison Matrix](../appendix/comparison-matrix.md) — detailed comparison
