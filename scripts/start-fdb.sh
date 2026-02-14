#!/bin/bash
set -e

# Start FDB container for testing
cd "$(dirname "$0")/.."

# Create cluster file if it doesn't exist
if [ ! -f fdb.cluster ]; then
    echo "docker:docker@127.0.0.1:4500" > fdb.cluster
fi

echo "Starting FDB container..."
docker-compose -f docker-compose.test.yml up -d

echo "Waiting for FDB to be ready..."
for i in {1..30}; do
    if docker exec pelago-fdb-test fdbcli --exec "status minimal" 2>/dev/null | grep -q "Healthy"; then
        echo "FDB is ready!"
        exit 0
    fi
    echo "Waiting... ($i/30)"
    sleep 2
done

echo "FDB failed to become ready"
exit 1
