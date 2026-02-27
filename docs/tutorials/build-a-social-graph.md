# Tutorial: Build a Social Graph

Build a complete social graph from scratch: schema registration, node creation, edge relationships, and querying — in about 20 minutes.

## Prerequisites

- PelagoDB server running (see [Quickstart](../getting-started/quickstart.md))
- CLI available (`pelago` or `cargo run -p pelago-cli --`)

## Step 1: Design the Schema

We'll model people who follow each other and belong to groups.

### Person Schema

```bash
pelago schema register --inline '{
  "name": "Person",
  "properties": {
    "name": {"type": "string", "required": true},
    "email": {"type": "string", "index": "unique"},
    "city": {"type": "string", "index": "equality"},
    "age": {"type": "int", "index": "range"},
    "active": {"type": "bool"}
  },
  "edges": {
    "FOLLOWS": {"target": "Person", "direction": "outgoing"},
    "MEMBER_OF": {"target": "Group", "direction": "outgoing"}
  },
  "meta": {"allow_undeclared_edges": false, "extras_policy": "reject"}
}'
```

### Group Schema

```bash
pelago schema register --inline '{
  "name": "Group",
  "properties": {
    "name": {"type": "string", "required": true},
    "topic": {"type": "string", "index": "equality"},
    "member_count": {"type": "int", "index": "range"}
  },
  "meta": {"allow_undeclared_edges": true, "extras_policy": "allow"}
}'
```

Verify:
```bash
pelago schema list
pelago schema get Person
pelago schema get Group
```

## Step 2: Create People

```bash
pelago node create Person name=Alice email=alice@example.com city=Portland age=32 active=true
pelago node create Person name=Bob email=bob@example.com city=Portland age=29 active=true
pelago node create Person name=Carol email=carol@example.com city=Seattle age=35 active=true
pelago node create Person name=Dave email=dave@example.com city=Seattle age=27 active=false
```

Note the node IDs returned (e.g., `1_0`, `1_1`, `1_2`, `1_3`). You'll need them for edges.

## Step 3: Create Groups

```bash
pelago node create Group name=Hikers topic=outdoors member_count=2
pelago node create Group name=Coders topic=technology member_count=3
```

## Step 4: Create Relationships

Replace the IDs below with the actual IDs from your node creation:

```bash
# Alice follows Bob and Carol
pelago edge create Person:1_0 FOLLOWS Person:1_1
pelago edge create Person:1_0 FOLLOWS Person:1_2

# Bob follows Alice
pelago edge create Person:1_1 FOLLOWS Person:1_0

# Carol follows Alice and Dave
pelago edge create Person:1_2 FOLLOWS Person:1_0
pelago edge create Person:1_2 FOLLOWS Person:1_3

# Group memberships
pelago edge create Person:1_0 MEMBER_OF Group:1_4
pelago edge create Person:1_1 MEMBER_OF Group:1_4
pelago edge create Person:1_0 MEMBER_OF Group:1_5
pelago edge create Person:1_2 MEMBER_OF Group:1_5
pelago edge create Person:1_3 MEMBER_OF Group:1_5
```

Verify edges:
```bash
pelago edge list Person 1_0 --dir out
```

## Step 5: Query with CEL

Find active people in Portland:

```bash
pelago query find Person --filter 'city == "Portland" && active == true' --limit 10
```

Find people over 30:

```bash
pelago query find Person --filter 'age >= 30' --limit 10
```

## Step 6: Traverse Relationships

Who does Alice follow?

```bash
pelago query traverse Person:1_0 FOLLOWS --max-depth 1 --max-results 20
```

Two-hop traversal — who do Alice's follows follow?

```bash
pelago query traverse Person:1_0 FOLLOWS --max-depth 2 --max-results 50
```

## Step 7: Query with PQL

Find all active people with their names:

```bash
pelago query pql --query 'Person @filter(active == true) { uid name city age }'
```

Advanced: Find people Alice follows who are over 30:

```bash
pelago query pql --query '
query {
  seed(func: uid(Person:1_0)) {
    name
    -[FOLLOWS]-> @filter(age >= 30) { uid name city age }
  }
}'
```

## Step 8: Explore with Explain

Check how your queries use indexes:

```bash
pelago query pql --query 'Person @filter(age >= 30) { uid name }' --explain
```

## What You've Learned

- **Schema-first modeling** with typed properties, indexes, and edge contracts
- **Node creation** with site-aware IDs
- **Edge creation** for directed relationships
- **Three query surfaces:** CEL for filtering, traversal for graph paths, PQL for expressive queries
- **Explain plans** for query optimization

## Exercises

1. Add a `bio` property to Person (string, no index) using a new schema version. Check with `schema diff`.
2. Create a query that finds all members of the "Coders" group using traversal.
3. Write a PQL query using variable captures to find mutual followers.

## Related

- [Core Workflow](../getting-started/core-workflow.md) — the lifecycle explained
- [Learn CEL Queries](learn-cel-queries.md) — deep dive on CEL
- [Learn PQL](learn-pql.md) — deep dive on PQL
- [Data Modeling Patterns](../guides/data-modeling-patterns.md) — best practices
