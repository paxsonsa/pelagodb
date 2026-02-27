# UI Console (Embedded)

PelagoDB can serve an embedded web console for graph exploration, query workflows,
operational state, and guided admin actions.

The V2 console uses a command-center layout with guided workflows for:
- Explorer: schema-backed entity selection, graph canvas, and node/edge inspectors
- Query: CEL/PQL studio with explain plans and degradation visibility
- Ops: KPI cards, typed state tables, filterable audit stream, and metrics fallback
- Admin: risk-tiered mutation cards with confirmation dialogs
- Watch: mode-driven stream setup, connection state badges, and filtered event feed

## Enable

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

Open:
- `http://127.0.0.1:4080/ui/`

## API Surface

The embedded console uses a versioned BFF API:
- `/ui/api/v1/auth/*`
- `/ui/api/v1/schema/*`
- `/ui/api/v1/query/*`
- `/ui/api/v1/graph/*`
- `/ui/api/v1/state/*`
- `/ui/api/v1/watch/*`
- `/ui/api/v1/admin/*`
- `/ui/api/v1/metrics/raw`

## Auth

When `PELAGO_AUTH_REQUIRED=true`, provide either:
- `x-api-key`
- `authorization: Bearer <token>`

Context headers:
- `x-pelago-database` (default `default`)
- `x-pelago-namespace` (default `default`)

The UI persists auth and scope state in browser `sessionStorage` for the active tab/session.
Closing the tab clears persisted credentials.

## Frontend Stack

- React 19 + React Router
- Tailwind CSS v4
- Radix primitives + shadcn-style component layer
- Sonner toast notifications
- Vitest + Testing Library for frontend tests

## Notes

- Destructive namespace/type drops are intentionally excluded from the UI.
- Watch streams are proxied via SSE endpoints.
- Metrics endpoint reflects `PELAGO_METRICS_ENABLED`; when disabled, UI shows non-fatal fallback state.
