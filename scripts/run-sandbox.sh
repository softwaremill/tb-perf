#!/bin/bash
set -e

echo "Starting Claude sandbox..."
docker sandbox run --template claude-rust-sandbox claude
