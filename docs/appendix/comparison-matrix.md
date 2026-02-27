# Comparison Matrix

Detailed comparison of PelagoDB vs Neo4j, Dgraph, and JanusGraph across key architectural dimensions.

## Architecture and Storage

| Dimension | PelagoDB | Neo4j | Dgraph | JanusGraph |
|---|---|---|---|---|
| Storage engine | FoundationDB (distributed KV) | Custom native store | Badger (distributed) | Pluggable (Cassandra, HBase, etc.) |
| Transaction model | ACID via FDB | ACID (single instance), causal cluster | Distributed ACID | Depends on backend |
| Consistency | Strong (FDB guarantees) | Strong (single), eventual (cluster) | Strong (Raft-based) | Eventual (typically) |

## Data Model

| Dimension | PelagoDB | Neo4j | Dgraph | JanusGraph |
|---|---|---|---|---|
| Model | Schema-first property graph | Property graph | Graph with GraphQL schema | Property graph |
| Schema enforcement | Required before writes | Optional constraints | Optional GraphQL schema | Backend-dependent |
| Property types | 6 types + null | Rich type system | Scalar + list types | Backend-dependent |
| Edge properties | Supported with schema | Supported | Facets | Supported |

## Query

| Dimension | PelagoDB | Neo4j | Dgraph | JanusGraph |
|---|---|---|---|---|
| Primary query language | CEL + PQL | Cypher | GraphQL / DQL | Gremlin |
| Secondary query surfaces | gRPC Traverse, SQL-like REPL | APOC procedures | HTTP/gRPC | REST API |
| Explain plans | Yes (CEL + PQL) | Yes (Cypher) | Yes | Yes (Gremlin profile) |
| Pagination | Cursor-based (FindNodes, Traverse) | SKIP/LIMIT | Pagination tokens | Range queries |

## Multi-Site and Replication

| Dimension | PelagoDB | Neo4j | Dgraph | JanusGraph |
|---|---|---|---|---|
| Multi-site model | Pull-based CDC replication | Causal Cluster / Aura | Raft groups, read replicas | Backend-dependent |
| Conflict resolution | Owner-wins + LWW for new data | Leader-based | Raft consensus | Backend-dependent |
| Cross-site writability | Yes (all sites writable) | Read replicas + routing | Write to leader | Backend-dependent |
| Replication control | Explicit per-namespace scopes | Cluster-level | Group-level | Backend-level |

## Operations

| Dimension | PelagoDB | Neo4j | Dgraph | JanusGraph |
|---|---|---|---|---|
| Auth model | API keys, bearer tokens, mTLS, policies | Native auth + LDAP/SSO | ACL, namespace auth | Backend-dependent |
| Audit logging | Built-in, queryable | Enterprise feature | Limited | Application-level |
| Watch / streaming | CDC-backed watch subscriptions | Change Data Capture (Enterprise) | GraphQL subscriptions | Not built-in |
| CLI | Full CRUD + admin + REPL | Cypher Shell | Dgraph CLI | gremlin.sh |

## Ecosystem

| Dimension | PelagoDB | Neo4j | Dgraph | JanusGraph |
|---|---|---|---|---|
| Client SDKs | Python, Elixir, Rust, Swift | 10+ languages | GraphQL clients, Go, Java | Gremlin language drivers |
| Visualization | Embedded web console | Neo4j Browser, Bloom | Ratel | Various Gremlin tools |
| Managed offering | Not yet | Aura (cloud) | Dgraph Cloud | Not built-in |
| Community size | Early stage | Very large | Medium | Medium |

## When to Choose Each

### Choose PelagoDB when:
- You need schema-first enforcement and explicit ownership semantics
- You need multi-site writability with conflict-aware replication
- You need cross-tenant graph relationships in a controlled model
- You can own operational complexity

### Choose Neo4j when:
- You need the broadest graph ecosystem and tooling
- Cypher language compatibility is important
- Managed cloud (Aura) is preferred
- Deep graph analytics is a primary use case

### Choose Dgraph when:
- GraphQL is your primary API surface
- Distributed-first architecture is required
- Strong consistency via Raft is preferred over LWW
- You want built-in GraphQL schema management

### Choose JanusGraph when:
- You need pluggable storage backends
- Gremlin/TinkerPop compatibility is required
- You want to leverage existing Cassandra/HBase infrastructure
- Maximum deployment flexibility is valued

## Related

- [Positioning](../concepts/positioning.md) — when to use PelagoDB
- [Architecture](../concepts/architecture.md) — PelagoDB system design
