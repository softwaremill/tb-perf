#!/bin/bash
set -e

cd "$(dirname "$0")/.."

echo "Removing existing sandbox template..."
# Remove any sandbox templates for this project directory
PROJECT_DIR="$(pwd)"
SANDBOX_IDS=$(docker sandbox ls 2>/dev/null | grep "$PROJECT_DIR" | awk '{print $1}' || true)
if [ -n "$SANDBOX_IDS" ]; then
    echo "$SANDBOX_IDS" | xargs docker sandbox rm
fi

echo "Building sandbox image..."
docker build -t claude-rust-sandbox -f Dockerfile.claude .

echo "Done! Run './scripts/run-sandbox.sh' to start the sandbox."
