# CLI Reference

The CLI binary is `pelago` (crate: `crates/pelago-cli`).

## Interface Coverage Matrix

This matrix shows which capabilities are available on each surface.

| Capability | gRPC | CLI (`pelago`) | REPL | SDKs |
|---|---|---|---|---|
| Schema register/get/list | Yes | Yes | Yes | Partial |
| Node create/get/update/delete/list | Yes | Yes | Partial | Partial |
| Node transfer ownership | Yes | No | No | Partial |
| Edge create/delete/list | Yes | Yes | Partial | Partial |
| Query find (CEL) | Yes | Yes | Yes | Partial |
| Query traverse | Yes | Yes (single-step CLI shape) | Partial | Partial |
| Query explain (CEL) | Yes | No | No | Partial |
| Execute PQL | Yes | Yes | Yes | Partial |
| PQL explain | Yes | Yes (`query pql --explain`) | Yes (`:explain`) | Partial |
| Admin drop index | Yes | No | No | Partial |
| Admin strip property | Yes | No | No | Partial |
| Admin jobs/sites/replication/audit | Yes | Yes | Partial | Partial |
| Admin drop type/namespace | Yes | Yes | Partial | Partial |
| Watch point/query/namespace | Yes | No | No | Partial |
| Watch list/cancel | Yes | No | No | Partial |
| Auth authenticate/token/policy APIs | Yes | No | No | Partial |
| Health check | Yes | No | No | Partial |

Notes:
- `Partial` in SDK column means generated stubs/examples exist, but first-class ergonomic wrappers may vary by language.
- For missing CLI coverage, use `grpcurl` or SDK stubs directly (see [gRPC API](grpc-api.md)).

## Global Flags

| Flag | Env Var | Default |
|---|---|---|
| `--server` | `PELAGO_SERVER` | `http://localhost:27615` |
| `--database` | `PELAGO_DATABASE` | `default` |
| `--namespace` | `PELAGO_NAMESPACE` | `default` |
| `--api-key` | `PELAGO_API_KEY` | — |
| `--bearer-token` | `PELAGO_BEARER_TOKEN` | — |
| `--format` | — | `table` (options: `table`, `json`, `csv`) |

## Configuration Precedence

Current behavior:
1. Explicit CLI flags (highest precedence)
2. Environment variables
3. Built-in defaults

CLI config file: `~/.pelago/config.toml`

> **Note:** The CLI currently parses `~/.pelago/config.toml` but does not yet apply it to effective runtime options. For deterministic scripts and automation, rely on flags and env vars.

Example config file shape (for future compatibility):

```toml
server = "http://127.0.0.1:27615"
database = "default"
namespace = "default"
format = "table"
```

## Schema Commands

Register from file:
```bash
pelago schema register ./schema.json
pelago schema register ./schema.sql --input-format sql
```

Register inline:
```bash
pelago schema register --inline '{"name":"Person","properties":{"name":{"type":"string","required":true}}}'
pelago schema register --inline 'CREATE TYPE Person (name STRING REQUIRED, age INT INDEX RANGE);' --input-format sql
```

`pelago schema register` accepts:
- Compact JSON shape
- Protobuf-shaped JSON (enum-style values and full edge/default metadata)
- SQL-like DDL (`CREATE TYPE ...` / `CREATE SCHEMA ...`)

Get and list:
```bash
pelago schema get Person
pelago schema list
pelago schema diff Person 1 2
```

## Node Commands

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

## Edge Commands

Create/delete/list:
```bash
pelago edge create Person:1_0 follows Person:1_1
pelago edge delete Person:1_0 follows Person:1_1
pelago edge list Person 1_0 --dir out --label follows
```

## Query Commands

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

## Admin Commands

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

Interactive UX features:
- Syntax highlighting for meta commands, SQL keywords, params, and PQL directives
- Tab completion for meta commands, SQL/PQL phrases, schema entity types, schema fields/edges, and `$params`
- History hints (`Ctrl-R` for reverse search) and smart multiline entry for open braces/quotes
- Fuzzy-ranked completion with runtime controls:
  - `:set completion fuzzy|prefix`
  - `:set color on|off`

See the [Interactive REPL Tutorial](../tutorials/interactive-repl.md) for a hands-on guide.

## Missing CLI Operations

Use `grpcurl` for currently uncovered operations:
- `QueryService.Explain`
- `WatchService` (all 5 RPCs)
- `AuthService` methods
- `HealthService.Check`
- `NodeService.TransferOwnership`
- `AdminService.DropIndex` and `AdminService.StripProperty`

See [gRPC API](grpc-api.md), [Watch API](watch-api.md), and [Auth API](auth-api.md) for request examples.

## Related

- [gRPC API Reference](grpc-api.md) — full API surface
- [Configuration Reference](configuration.md) — server settings
- [Interactive REPL Tutorial](../tutorials/interactive-repl.md) — hands-on REPL guide
