# Using the Console

PelagoDB includes an embedded web console for graph exploration, query workflows, operational state, and guided admin actions.

## Enabling the Console

Build UI assets:

```bash
cd ui
npm install
npm run typecheck
npm run test
npm run build
cd ..
```

Start server with UI enabled:

```bash
export PELAGO_UI_ENABLED=true
export PELAGO_UI_ADDR=127.0.0.1:4080
export PELAGO_UI_ASSETS_DIR=ui/dist
export PELAGO_UI_TITLE='PelagoDB Console'
cargo run -p pelago-server --bin pelago-server
```

Open: `http://127.0.0.1:4080/ui/`

## Console Features

The V2 console uses a command-center layout with five guided workflows:

### Explorer

Schema-backed entity selection, graph canvas, and node/edge inspectors. Browse entity types, select nodes, and visualize relationships.

### Query

CEL/PQL studio with explain plans and degradation visibility. Write queries, see results, and inspect execution plans.

### Ops

KPI cards, typed state tables, filterable audit stream, and metrics fallback. Monitor system health, replication status, and audit events.

### Admin

Risk-tiered mutation cards with confirmation dialogs. Perform administrative operations with safety guardrails. Destructive namespace/type drops are intentionally excluded from the UI.

### Watch

Mode-driven stream setup, connection state badges, and filtered event feed. Set up point, query, or namespace watches and observe live changes.

## Authentication

When `PELAGO_AUTH_REQUIRED=true`, provide either:
- `x-api-key` header
- `authorization: Bearer <token>` header

Context headers:
- `x-pelago-database` (default: `default`)
- `x-pelago-namespace` (default: `default`)

The UI persists auth and scope state in browser `sessionStorage`. Closing the tab clears credentials.

## API Surface

The console uses a versioned BFF API:

| Endpoint Group | Purpose |
|---|---|
| `/ui/api/v1/auth/*` | Authentication |
| `/ui/api/v1/schema/*` | Schema operations |
| `/ui/api/v1/query/*` | Query execution |
| `/ui/api/v1/graph/*` | Graph exploration |
| `/ui/api/v1/state/*` | System state |
| `/ui/api/v1/watch/*` | Watch streams (SSE proxy) |
| `/ui/api/v1/admin/*` | Admin operations |
| `/ui/api/v1/metrics/raw` | Raw metrics |

## Frontend Stack

- React 19 + React Router
- Tailwind CSS v4
- Radix primitives + shadcn-style component layer
- Sonner toast notifications
- Vitest + Testing Library for frontend tests

## Notes

- Watch streams are proxied via SSE endpoints.
- Metrics endpoint reflects `PELAGO_METRICS_ENABLED`; when disabled, UI shows non-fatal fallback state.
- Use a private/internal bind address unless the console should be publicly reachable.

## Related

- [Configuration Reference](../reference/configuration.md) — UI settings
- [Quickstart](../getting-started/quickstart.md) — getting started
- [Presentation Runbook](../operations/presentation-runbook.md) — demo delivery
