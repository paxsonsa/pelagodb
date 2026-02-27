# Deployment Guide

Single-site, multi-site, and Docker deployment topologies for PelagoDB.

## Single-Site Deployment

The simplest topology: one server process serving both API traffic and (optionally) replication.

```bash
export PELAGO_FDB_CLUSTER=/usr/local/etc/foundationdb/fdb.cluster
export PELAGO_SITE_ID=1
export PELAGO_SITE_NAME=dc1
export PELAGO_LISTEN_ADDR=127.0.0.1:27615
export PELAGO_AUTH_REQUIRED=false
export PELAGO_CACHE_ENABLED=true
cargo run -p pelago-server --bin pelago-server
```

Verify:
```bash
pelago admin sites
```

## Multi-Site Deployment

### Recommended Topology

Per site, split into two tiers:

| Tier | Configuration | Purpose |
|---|---|---|
| API/query nodes | `PELAGO_REPLICATION_ENABLED=false` | Serve client traffic |
| Replicator nodes | `PELAGO_REPLICATION_ENABLED=true` | Pull CDC from remote sites |

This separates client-serving capacity from replication throughput.

### Site Configuration

Each site needs a unique `PELAGO_SITE_ID` and configured peers:

**Site 1:**
```bash
export PELAGO_SITE_ID=1
export PELAGO_SITE_NAME=dc1
export PELAGO_LISTEN_ADDR=0.0.0.0:27615
export PELAGO_REPLICATION_ENABLED=true
export PELAGO_REPLICATION_PEERS='2=dc2.example.com:27616'
export PELAGO_REPLICATION_DATABASE=default
export PELAGO_REPLICATION_NAMESPACE=default
```

**Site 2:**
```bash
export PELAGO_SITE_ID=2
export PELAGO_SITE_NAME=dc2
export PELAGO_LISTEN_ADDR=0.0.0.0:27616
export PELAGO_REPLICATION_ENABLED=true
export PELAGO_REPLICATION_PEERS='1=dc1.example.com:27615'
export PELAGO_REPLICATION_DATABASE=default
export PELAGO_REPLICATION_NAMESPACE=default
```

### Replication Scopes

For multi-namespace replication, use scopes instead of single database/namespace:

```bash
export PELAGO_REPLICATION_SCOPES='default/default,analytics/live'
```

## Docker Compose Multi-Site

Local two-site topology for testing:

```bash
docker compose -f docker-compose.multisite.yml up --build -d
```

Endpoints:
- Site 1 API: `127.0.0.1:27615`
- Site 2 API: `127.0.0.1:27616`

Shutdown:
```bash
docker compose -f docker-compose.multisite.yml down -v
```

## Validation

```bash
pelago admin sites                    # verify site claims
pelago admin replication-status       # verify peer connectivity and lag
pelago admin audit --limit 20         # verify audit activity
```

Expected:
- Peers listed with correct site IDs
- `last_applied_versionstamp` advancing
- Lag values stable or decreasing

## Operational Guidance

- Use unique site IDs globally
- Keep clocks reasonably synchronized for predictable LWW behavior
- For planned maintenance, monitor replication lag to near-zero first
- Use `scripts/production-env.example` for security-first runtime defaults

## Embedded Services

### Docs Server

```bash
export PELAGO_DOCS_ENABLED=true
export PELAGO_DOCS_ADDR=127.0.0.1:4070
```

### UI Console

```bash
export PELAGO_UI_ENABLED=true
export PELAGO_UI_ADDR=127.0.0.1:4080
```

## Related

- [Kubernetes](kubernetes.md) — K8s deployment
- [Configuration Reference](../reference/configuration.md) — all settings
- [Security Setup](security-setup.md) — auth configuration
- [Replication Operations](replication-operations.md) — monitoring and validation
- [Installation](../getting-started/installation.md) — prerequisites
