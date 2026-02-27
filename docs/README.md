# PelagoDB Documentation

Welcome to PelagoDB — a schema-first, FoundationDB-backed graph database with multi-site replication, ownership semantics, and cross-tenant relationship modeling.

## Learning Paths

### New Developer (30 min)
Get productive quickly with PelagoDB's core features.
1. [Quickstart](getting-started/quickstart.md) — first server to first query
2. [Data Model](concepts/data-model.md) — nodes, edges, properties
3. [Schema System](concepts/schema-system.md) — schema-first philosophy
4. [Learn CEL Queries](tutorials/learn-cel-queries.md) — property filtering
5. [CLI Reference](reference/cli.md) — command reference

**Completion signal:** You can register schemas, create data, and query with CEL.

### Application Developer (2-3 hrs)
Full deep dive into building applications on PelagoDB.
1. [Installation](getting-started/installation.md) → [Quickstart](getting-started/quickstart.md) → [Core Workflow](getting-started/core-workflow.md)
2. [Data Model](concepts/data-model.md) → [Schema System](concepts/schema-system.md) → [Query Model](concepts/query-model.md)
3. [Build a Social Graph](tutorials/build-a-social-graph.md) → [Learn CEL](tutorials/learn-cel-queries.md) → [Learn PQL](tutorials/learn-pql.md)
4. [Data Modeling Patterns](guides/data-modeling-patterns.md) → [Schema Design Strategy](guides/schema-design-strategy.md)
5. [Python SDK](tutorials/python-sdk.md) or [Elixir SDK](tutorials/elixir-sdk.md)

**Completion signal:** You can design schemas, build graph data models, write queries, and integrate via SDKs.

### Operator (2-3 hrs)
Everything needed to deploy and operate PelagoDB in production.
1. [Installation](getting-started/installation.md) → [Architecture](concepts/architecture.md)
2. [Replication](concepts/replication.md) → [Caching and Consistency](concepts/caching-and-consistency.md)
3. [Deployment](operations/deployment.md) → [Kubernetes](operations/kubernetes.md)
4. [Security Setup](operations/security-setup.md) → [Monitoring](operations/monitoring.md)
5. [Production Checklist](operations/production-checklist.md) → [Backup and Recovery](operations/backup-and-recovery.md)

**Completion signal:** You can deploy, secure, monitor, and maintain PelagoDB in production.

### Evaluator (45 min)
Assess whether PelagoDB fits your use case.
1. [Positioning](concepts/positioning.md) — honest tradeoffs
2. [Architecture](concepts/architecture.md) — system design
3. [Data Model](concepts/data-model.md) — data capabilities
4. [Replication](concepts/replication.md) — multi-site design
5. [Comparison Matrix](appendix/comparison-matrix.md) — vs Neo4j, Dgraph, JanusGraph

**Completion signal:** You have a clear decision framework for PelagoDB adoption.

### Presenter (30 min prep)
Prepare for live demos and presentations.
1. [Quickstart](getting-started/quickstart.md) — environment setup
2. [Onboarding Course](tutorials/onboarding-course.md) — 60-min guided course
3. [Presentation Runbook](operations/presentation-runbook.md) — demo delivery
4. [Datasets and Loading](guides/datasets-and-loading.md) — demo data

**Completion signal:** Demo environment is ready, rehearsed, and fallback plans are in place.

---

## Documentation Sections

### [Getting Started](getting-started/)
- [Installation](getting-started/installation.md) — prerequisites and setup
- [Quickstart](getting-started/quickstart.md) — first query in 10 minutes
- [Core Workflow](getting-started/core-workflow.md) — schema → node → edge → query

### [Concepts](concepts/)
- [Architecture](concepts/architecture.md) — system design and crates
- [Data Model](concepts/data-model.md) — nodes, edges, properties, IDs
- [Schema System](concepts/schema-system.md) — schema-first philosophy
- [Query Model](concepts/query-model.md) — CEL vs PQL
- [Namespaces and Tenancy](concepts/namespaces-and-tenancy.md) — partitioning
- [CDC and Event Model](concepts/cdc-and-event-model.md) — change data capture
- [Replication](concepts/replication.md) — multi-site design
- [Caching and Consistency](concepts/caching-and-consistency.md) — read acceleration
- [Security Model](concepts/security-model.md) — auth and authorization
- [Positioning](concepts/positioning.md) — when to use PelagoDB

### [Tutorials](tutorials/)
- [Build a Social Graph](tutorials/build-a-social-graph.md) — full hands-on tutorial
- [Learn CEL Queries](tutorials/learn-cel-queries.md) — CEL filtering exercises
- [Learn PQL](tutorials/learn-pql.md) — PQL from basics to advanced
- [Interactive REPL](tutorials/interactive-repl.md) — REPL guide
- [Python SDK](tutorials/python-sdk.md) — end-to-end Python
- [Elixir SDK](tutorials/elixir-sdk.md) — end-to-end Elixir
- [Watch Live Changes](tutorials/watch-live-changes.md) — watch subscriptions
- [Set Up Multi-Site](tutorials/set-up-multi-site.md) — replication with docker-compose
- [Onboarding Course](tutorials/onboarding-course.md) — 60-min guided course

### [Guides](guides/)
- [Data Modeling Patterns](guides/data-modeling-patterns.md) — best practices
- [Schema Design Strategy](guides/schema-design-strategy.md) — index and evolution
- [Query Optimization](guides/query-optimization.md) — explain plans and tuning
- [Datasets and Loading](guides/datasets-and-loading.md) — bundled datasets
- [Using the Console](guides/using-the-console.md) — web UI guide
- [Cross-Tenant Modeling](guides/cross-tenant-modeling.md) — cross-boundary patterns
- [Capacity Planning](guides/capacity-planning.md) — sizing and budgets

### [Reference](reference/)
- [CLI](reference/cli.md) — complete CLI commands
- [gRPC API](reference/grpc-api.md) — core services
- [Watch API](reference/watch-api.md) — watch subscriptions
- [Auth API](reference/auth-api.md) — authentication and authorization
- [PQL](reference/pql.md) — query language grammar
- [CEL Filters](reference/cel-filters.md) — expression syntax
- [Schema Spec](reference/schema-spec.md) — schema format
- [Configuration](reference/configuration.md) — server settings
- [Data Types](reference/data-types.md) — value types and encoding
- [Errors](reference/errors.md) — error types and status codes

### [Operations](operations/)
- [Deployment](operations/deployment.md) — single-site, multi-site, Docker
- [Kubernetes](operations/kubernetes.md) — K8s manifests and HPA
- [Security Setup](operations/security-setup.md) — auth configuration
- [Replication Operations](operations/replication-operations.md) — peer management
- [Monitoring](operations/monitoring.md) — metrics and health checks
- [Backup and Recovery](operations/backup-and-recovery.md) — DR procedures
- [Production Checklist](operations/production-checklist.md) — pre-production readiness
- [Troubleshooting](operations/troubleshooting.md) — common issues
- [Daily Operations](operations/daily-operations.md) — operational rhythm
- [Presentation Runbook](operations/presentation-runbook.md) — demo delivery

### [Appendix](appendix/)
- [Glossary](appendix/glossary.md) — key terms and definitions
- [Comparison Matrix](appendix/comparison-matrix.md) — vs Neo4j, Dgraph, JanusGraph

---

## Source of Truth

- Protocol/API contract: `proto/pelago.proto`
- Server runtime: `crates/pelago-server/src/main.rs`
- Storage semantics: `crates/pelago-storage/src/`
- Query engine: `crates/pelago-query/src/`
