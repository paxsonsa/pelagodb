# Installation

This guide covers prerequisites, platform-specific setup, and building PelagoDB from source.

## Prerequisites

- **Rust toolchain** (stable) — install via [rustup](https://rustup.rs)
- **FoundationDB 7.3.x** client libraries and server
- A valid FDB cluster file at one of:
  - `/usr/local/etc/foundationdb/fdb.cluster` (macOS Intel)
  - `/opt/homebrew/etc/foundationdb/fdb.cluster` (macOS Apple Silicon)
  - `/etc/foundationdb/fdb.cluster` (Linux packages)
  - `./fdb.cluster` (containerized FDB for tests)

## Platform Setup

### macOS Apple Silicon

Homebrew prefix is `/opt/homebrew`, not `/usr/local`. If `fdbcli` is installed but the cluster path is wrong, startup fails with FDB connection errors.

Quick check:

```bash
which fdbcli
ls -l /opt/homebrew/etc/foundationdb/fdb.cluster
```

### macOS Intel

Standard Homebrew prefix at `/usr/local`. Verify cluster file location:

```bash
ls -l /usr/local/etc/foundationdb/fdb.cluster
fdbcli --exec "status minimal"
```

### Docker on macOS

Docker Desktop networking can differ from Linux host networking. If `scripts/start-fdb.sh` cannot expose FDB reliably to host clients, prefer a native local FDB install for day-to-day development.

### Linux

Ensure your user can read the cluster file (often root-owned by package defaults). If using systemd services, verify the FoundationDB daemon is healthy before starting PelagoDB.

```bash
fdbcli --exec "status minimal"
```

## FoundationDB Setup

### Option A: Use an Existing Local Cluster

If you already have a local cluster running, reuse it:

```bash
fdbcli --exec "status minimal"
```

### Option B: Start Test FDB via Docker

```bash
./scripts/start-fdb.sh
```

If this fails due to Docker networking behavior, use Option A instead.

## Building from Source

Clone the repository and build:

```bash
git clone <repo-url> pelagodb
cd pelagodb
cargo build --release
```

The server binary is built from the `pelago-server` crate:

```bash
cargo build -p pelago-server --release
```

The CLI binary is built from the `pelago-cli` crate:

```bash
cargo build -p pelago-cli --release
```

## Verifying the Installation

Start the server:

```bash
export PELAGO_FDB_CLUSTER=/usr/local/etc/foundationdb/fdb.cluster
# Apple Silicon: /opt/homebrew/etc/foundationdb/fdb.cluster
export PELAGO_SITE_ID=1
export PELAGO_SITE_NAME=dc1
export PELAGO_LISTEN_ADDR=127.0.0.1:27615
cargo run -p pelago-server --bin pelago-server
```

In a second terminal, verify connectivity:

```bash
cargo run -p pelago-cli -- --server http://127.0.0.1:27615 schema list
```

## Related

- [Quickstart](quickstart.md) — first server to first query in 10 minutes
- [Configuration Reference](../reference/configuration.md) — all server settings
- [Troubleshooting](../operations/troubleshooting.md) — platform-specific fixes
