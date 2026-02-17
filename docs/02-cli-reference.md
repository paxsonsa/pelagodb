# CLI Reference

The CLI binary is `pelago` (crate: `crates/pelago-cli`).

## Interface Coverage Matrix

This matrix is intentionally explicit so users can pick the right surface without guessing.

| Capability | gRPC | CLI (`pelago`) | REPL | SDKs |
|---|---|---|---|---|
| Schema register/get/list | Yes | Yes | No | Partial |
| Node create/get/update/delete/list | Yes | Yes | No | Partial |
| Node transfer ownership | Yes | No | No | Partial |
| Edge create/delete/list | Yes | Yes | No | Partial |
| Query find (CEL) | Yes | Yes | No | Partial |
| Query traverse | Yes | Yes (single-step CLI shape) | No | Partial |
| Query explain (CEL) | Yes | No | No | Partial |
| Execute PQL | Yes | Yes | Yes | Partial |
| PQL explain | Yes | Yes (`query pql --explain`) | Yes (`:explain`) | Partial |
| Admin drop index | Yes | No | No | Partial |
| Admin strip property | Yes | No | No | Partial |
| Admin jobs/sites/replication/audit | Yes | Yes | No | Partial |
| Admin drop type/namespace | Yes | Yes | No | Partial |
| Watch point/query/namespace | Yes | No | No | Partial |
| Watch list/cancel | Yes | No | No | Partial |
| Auth authenticate/token/policy APIs | Yes | No | No | Partial |
| Health check | Yes | No | No | Partial |

Notes:
- `Partial` in SDK column means generated stubs/examples exist, but first-class ergonomic wrappers may vary by language.
- For missing CLI coverage, use `grpcurl` or SDK stubs directly (`docs/03-api-grpc.md`).

## Global Flags

- `--server` (env: `PELAGO_SERVER`, default: `http://localhost:27615`)
- `--database` (env: `PELAGO_DATABASE`, default: `default`)
- `--namespace` (env: `PELAGO_NAMESPACE`, default: `default`)
- `--api-key` (env: `PELAGO_API_KEY`)
- `--bearer-token` (env: `PELAGO_BEARER_TOKEN`)
- `--format table|json|csv`

## CLI Configuration Behavior and Precedence

Current behavior (as of 2026-02-17):
1. Explicit CLI flags (highest precedence)
2. Environment variables
3. Built-in defaults

CLI config file location:
- `~/.pelago/config.toml`

Important:
- The CLI currently parses `~/.pelago/config.toml`, but does not yet apply it to effective runtime options.
- For deterministic scripts and automation, rely on flags and env vars today.

Planned target behavior:
1. Built-in defaults
2. `~/.pelago/config.toml`
3. Environment variables
4. CLI flags

Example config file shape for future compatibility:

```toml
server = "http://127.0.0.1:27615"
database = "default"
namespace = "default"
format = "table"
```

## Schema

Register from file:
```bash
pelago schema register ./schema.json
```

Register inline:
```bash
pelago schema register --inline '{"name":"Person","properties":{"name":{"type":"string","required":true}}}'
```

Get and list:
```bash
pelago schema get Person
pelago schema list
pelago schema diff Person 1 2
```

## Nodes

Create:
```bash
pelago node create Person name=Alice age=31
pelago node create Person --props '{"name":"Bob","age":29}'
```

Get/update/delete:
```bash
pelago node get Person 1_0
pelago node update Person 1_0 --props '{"age":32}'
pelago node delete Person 1_0 --force
```

List:
```bash
pelago node list Person --limit 200 --filter 'age >= 30'
```

## Edges

Create/delete/list:
```bash
pelago edge create Person:1_0 follows Person:1_1
pelago edge delete Person:1_0 follows Person:1_1
pelago edge list Person 1_0 --dir out --label follows
```

## Queries

CEL find:
```bash
pelago query find Person --filter 'age >= 30' --limit 100
```

Traversal:
```bash
pelago query traverse Person:1_0 follows --max-depth 2 --max-results 200
```

PQL:
```bash
pelago query pql --query 'Person @filter(age >= 30) { uid name age }'
pelago query pql --file ./query.pql --explain
```

## Admin

Jobs:
```bash
pelago admin job list
pelago admin job status <job_id>
```

Sites, replication, audit:
```bash
pelago admin sites
pelago admin replication-status
pelago admin audit --principal user-1 --action authz.denied --limit 100
```

Drops:
```bash
pelago admin drop-type Person
pelago admin drop-namespace default
```

## REPL

```bash
pelago repl
```

Use REPL for iterative PQL during demos.

## Missing CLI Operations (Current Workarounds)

Use `grpcurl` for currently uncovered operations:
- `QueryService.Explain`
- `WatchService` (`WatchPoint`, `WatchQuery`, `WatchNamespace`, `ListSubscriptions`, `CancelSubscription`)
- `AuthService` methods
- `HealthService.Check`
- `NodeService.TransferOwnership`
- `AdminService.DropIndex` and `AdminService.StripProperty`

Detailed request examples are in:
- `docs/03-api-grpc.md`
- `docs/05-security-and-access.md`
- `docs/17-watch-and-streaming.md`
