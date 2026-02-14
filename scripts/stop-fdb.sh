#!/bin/bash
set -e

cd "$(dirname "$0")/.."

echo "Stopping FDB container..."
docker-compose -f docker-compose.test.yml down

echo "FDB stopped."
