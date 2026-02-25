# Server-Hosted Docs Site

PelagoDB can serve documentation pages from the running `pelago-server` process when enabled.

## Why
- Keep operator docs available next to runtime services.
- Provide a polished docs portal for demos and internal teams.
- Avoid needing a separate docs deployment for presentation environments.

## Enable
Set environment variables before starting `pelago-server`:

```bash
export PELAGO_DOCS_ENABLED=true
export PELAGO_DOCS_ADDR=127.0.0.1:4070
export PELAGO_DOCS_DIR=docs
export PELAGO_DOCS_TITLE='PelagoDB Documentation'
```

Then run server as usual:

```bash
cargo run -p pelago-server --bin pelago-server
```

Open:
- `http://127.0.0.1:4070/docs/` (index)
- `http://127.0.0.1:4070/docs/01-getting-started` (example page)

## Behavior
- Markdown files are read from `PELAGO_DOCS_DIR` (default `docs`).
- Pages are rendered to HTML with a built-in stylesheet.
- The docs site runs in the same process but on its own listener address.
- If docs are disabled (`PELAGO_DOCS_ENABLED=false`), no docs listener is started.

## Operational Notes
- Use a private/internal bind address unless docs should be publicly reachable.
- If auth-required docs are needed, place docs listener behind a reverse proxy with auth.
- Keep docs content in source control for reproducibility.
