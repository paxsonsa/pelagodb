# Core Workflow

The fundamental lifecycle in PelagoDB: **Schema → Node → Edge → Query**.

Every interaction follows this pattern. Understanding it unlocks the rest of the system.

## 1) Define a Schema

Before creating any data, register a schema that defines the entity type, its properties, indexes, and edge contracts.

```bash
pelago schema register --inline '{
  "name": "Person",
  "properties": {
    "name": {"type": "string", "required": true},
    "email": {"type": "string", "index": "unique"},
    "age": {"type": "int", "index": "range"}
  },
  "meta": {
    "allow_undeclared_edges": true,
    "extras_policy": "reject"
  }
}'
```

Why schema-first? Schemas enforce data shape at write time, enable index-backed queries, and prevent drift as teams grow.

Verify:
```bash
pelago schema get Person
```

## 2) Create Nodes

Nodes are instances of a schema type. Each node gets a site-aware ID (e.g., `1_0` meaning site 1, sequence 0).

```bash
pelago node create Person name=Alice email=alice@example.com age=32
pelago node create Person name=Bob email=bob@example.com age=29
```

Verify:
```bash
pelago node get Person 1_0
```

## 3) Create Edges

Edges connect nodes with labeled, directed relationships.

```bash
pelago edge create Person:1_0 follows Person:1_1
```

This creates an edge from Alice to Bob with label `follows`.

List edges:
```bash
pelago edge list Person 1_0 --dir out --label follows
```

## 4) Query Your Graph

PelagoDB offers three query surfaces:

### CEL Filter (property-based)

Find nodes matching a predicate:

```bash
pelago query find Person --filter 'age >= 30' --limit 100
```

### Traversal (graph-based)

Walk relationships from a starting node:

```bash
pelago query traverse Person:1_0 follows --max-depth 2 --max-results 50
```

### PQL (graph-native language)

Express complex queries with the Pelago Query Language:

```bash
pelago query pql --query 'Person @filter(age >= 25) { uid name age }'
```

## The Lifecycle in Context

```
Schema Registration
       ↓
  Node Creation → CDC Event → Cache Projection
       ↓
  Edge Creation → CDC Event → Cache Projection
       ↓
  Query / Watch / Replicate
```

Every mutation emits a CDC event that powers:
- **Replication** — pull to remote sites
- **Watch** — stream to subscribed clients
- **Cache** — project into RocksDB for fast reads

## What's Next

| Goal | Next Step |
|---|---|
| Understand the data model | [Data Model](../concepts/data-model.md) |
| Learn about schemas in depth | [Schema System](../concepts/schema-system.md) |
| Build a complete graph | [Build a Social Graph](../tutorials/build-a-social-graph.md) |
| Learn query languages | [Learn CEL](../tutorials/learn-cel-queries.md) / [Learn PQL](../tutorials/learn-pql.md) |

## Related

- [Quickstart](quickstart.md) — 10-minute fast start
- [Architecture](../concepts/architecture.md) — system design
- [CLI Reference](../reference/cli.md) — all commands
