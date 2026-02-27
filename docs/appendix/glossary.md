# Glossary

Key terms and definitions used throughout PelagoDB documentation.

## Core Concepts

**Node**
: An instance of a schema-defined entity type. Each node has a type, site-aware ID, and typed properties.

**Edge**
: A labeled, directed relationship between two nodes. Can be outgoing (one-way) or bidirectional.

**Entity Type**
: A named schema definition (e.g., `Person`, `Order`). All nodes of the same type share the same property/edge contracts.

**Property**
: A typed key-value pair on a node or edge, defined by the schema.

**Schema**
: The definition of an entity type's structure: properties, edges, indexes, and validation policy.

## Identity

**Node ID**
: Site-aware identifier in the format `<site_id>_<sequence>` (e.g., `1_0`, `2_42`).

**Edge Label**
: The name of a relationship type (e.g., `FOLLOWS`, `HAS_ORDER`). PQL expects uppercase-first labels.

## Organization

**Database**
: Top-level container for data (default: `default`).

**Namespace**
: Operational partition within a database. Controls isolation, replication scope, and lifecycle.

**Tenant**
: Business/account boundary — who owns data logically. Not a PelagoDB primitive; implemented through namespaces and authorization.

**Ownership**
: Per-entity write authority that controls which site can mutate a node. Enforced below the namespace level.

## Query

**CEL (Common Expression Language)**
: Property-based filter language used in `FindNodes`, traverse node filters, and watch queries.

**PQL (Pelago Query Language)**
: Graph-native query language supporting multi-block queries, variable captures, traversals, and set operations.

**Explain Plan**
: Query execution plan showing index selection and scan strategy. Available for both CEL and PQL.

**FindNodes**
: gRPC endpoint for CEL-filtered node retrieval with cursor-based pagination.

**Traverse**
: gRPC endpoint for graph path exploration from a starting node with depth/result bounds.

## Replication

**CDC (Change Data Capture)**
: Event log appended on every mutation. Powers replication, watch, and cache projection.

**Versionstamp**
: FoundationDB-assigned ordered identifier for CDC events. Used for replication checkpoints and watch resume.

**LWW (Last-Write-Wins)**
: Conflict resolution strategy for concurrent writes during partitions. The most recent write (by versionstamp) wins.

**Replicator**
: A PelagoDB process that pulls CDC from remote sites and applies changes locally.

**Lease**
: Lock mechanism ensuring only one replicator actively pulls per scope. Prevents duplicate processing.

**Pull Replication**
: PelagoDB's replication model where receiving sites actively pull CDC events from source sites.

## Caching

**Cache Projector**
: Background process that reads CDC and projects node/edge state into RocksDB for fast reads.

**RocksDB Cache**
: Optional read cache that accelerates queries. Always derivative state — FDB is source of truth.

## Security

**API Key**
: Static credential for authentication, mapped to a principal via `PELAGO_API_KEYS`.

**Bearer Token**
: Short-lived access token obtained by exchanging an API key through `AuthService.Authenticate`.

**Principal**
: Identity associated with a request (e.g., `admin-user`, `app-service`). Derived from API key, token, or mTLS certificate.

**Policy**
: Authorization rule binding a principal to specific permissions (actions, databases, namespaces, entity types).

**Audit Event**
: Logged record of security and administrative actions (auth, authz, replication conflicts).

## Operations

**Site**
: A PelagoDB deployment identified by a unique `PELAGO_SITE_ID`. Each site has its own FDB cluster.

**Smoke Check**
: Quick validation that server and data are healthy (`scripts/presentation-smoke.sh`).

**Benchmark**
: Performance measurement harness tracking p50/p95/p99 latency (`scripts/perf-benchmark.py`).

**DR Rehearsal**
: Disaster recovery practice run using `scripts/dr-rehearsal.sh`.

## Schema

**PropertyDef**
: Schema definition for a node property: type, required flag, index type, default value.

**EdgeDef**
: Schema definition for an edge: target type, direction, properties, sort key, ownership.

**SchemaMeta**
: Schema-level policies: `allow_undeclared_edges` and `extras_policy`.

**Index Backfill**
: Background job that creates index entries for existing nodes when a new indexed property is added.

## Related

- [Data Model](../concepts/data-model.md) — detailed data model explanation
- [Architecture](../concepts/architecture.md) — system overview
