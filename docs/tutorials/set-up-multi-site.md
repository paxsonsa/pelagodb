# Tutorial: Set Up Multi-Site Replication

Set up a two-site PelagoDB topology with docker-compose and verify replication.

## Prerequisites

- Docker and docker-compose installed
- PelagoDB repository cloned

## Step 1: Start the Multi-Site Topology

```bash
docker compose -f docker-compose.multisite.yml up --build -d
```

This starts:
- **Site 1** API server at `127.0.0.1:27615`
- **Site 2** API server at `127.0.0.1:27616`
- FoundationDB instances for each site
- Replicator workers for cross-site CDC pull

## Step 2: Verify Both Sites

```bash
pelago --server http://127.0.0.1:27615 admin sites
pelago --server http://127.0.0.1:27616 admin sites
```

Both commands should show the registered sites.

## Step 3: Register Schema on Site 1

```bash
pelago --server http://127.0.0.1:27615 schema register --inline '{
  "name": "Person",
  "properties": {
    "name": {"type": "string", "required": true},
    "city": {"type": "string", "index": "equality"},
    "age": {"type": "int", "index": "range"}
  },
  "meta": {"allow_undeclared_edges": true, "extras_policy": "allow"}
}'
```

## Step 4: Create Data on Site 1

```bash
pelago --server http://127.0.0.1:27615 node create Person name=Alice city=Portland age=32
pelago --server http://127.0.0.1:27615 node create Person name=Bob city=Seattle age=29
```

## Step 5: Verify Replication to Site 2

After a short delay (replication poll interval), query Site 2:

```bash
pelago --server http://127.0.0.1:27616 query find Person --filter 'age >= 25' --limit 10
```

You should see Alice and Bob replicated from Site 1.

## Step 6: Check Replication Status

```bash
pelago --server http://127.0.0.1:27615 admin replication-status
pelago --server http://127.0.0.1:27616 admin replication-status
```

Verify:
- Peers are listed
- `last_applied_versionstamp` is advancing
- Lag values are stable or decreasing

## Step 7: Create Data on Site 2

```bash
pelago --server http://127.0.0.1:27616 node create Person name=Carol city=Denver age=35
```

Verify it replicates back to Site 1:

```bash
pelago --server http://127.0.0.1:27615 query find Person --filter 'name == "Carol"'
```

## Step 8: Observe Conflict Behavior

If you update the same node from both sites simultaneously, the replication system resolves conflicts using LWW (last-write-wins) and logs audit events:

```bash
pelago --server http://127.0.0.1:27615 admin audit --action replication.conflict --limit 20
```

## Cleanup

```bash
docker compose -f docker-compose.multisite.yml down -v
```

## What You've Learned

- How to deploy a multi-site PelagoDB topology
- How replication propagates data between sites
- How to verify replication health
- How conflict resolution works

## Related

- [Replication Concepts](../concepts/replication.md) — design and model
- [Replication Operations](../operations/replication-operations.md) — production setup
- [Deployment Guide](../operations/deployment.md) — all deployment topologies
- [Configuration Reference](../reference/configuration.md) — replication settings
