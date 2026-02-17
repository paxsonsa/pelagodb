#!/bin/bash
set -euo pipefail

# Start FDB container for testing
cd "$(dirname "$0")/.."

echo "Starting FDB container..."
docker-compose -f docker-compose.test.yml up -d

echo "Waiting for FDB to be healthy..."
for i in {1..60}; do
    health_status="$(docker inspect -f '{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}' pelago-fdb-test 2>/dev/null || true)"
    if [[ "$health_status" == "healthy" ]]; then
        echo "FDB is healthy."
        # Keep a host-side cluster file for test commands.
        echo "docker:docker@127.0.0.1:4500" > fdb.cluster
        echo "Cluster file written to ./fdb.cluster"

        # Verify host-side CLI can reach the cluster.
        if fdbcli -C ./fdb.cluster --exec "status minimal" 2>/dev/null | grep -Eq "The database is available|Healthy"; then
            echo "Host connectivity check passed."
            exit 0
        fi

        echo "FDB container is healthy, but host connectivity check failed for ./fdb.cluster."
        echo "If you're on macOS Docker Desktop, host networking may not be routable for FDB."
        echo "Recommended: use a native local FoundationDB install, or run this test harness on Linux."
        exit 1
    fi
    echo "Waiting... ($i/60) health=$health_status"
    sleep 2
done

echo "FDB failed to become healthy"
echo "Recent container logs:"
docker logs --tail 80 pelago-fdb-test || true
exit 1
