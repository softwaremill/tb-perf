#!/bin/bash
# Stop Docker infrastructure for tb-perf
#
# Usage:
#   ./scripts/stop-docker.sh postgresql   # Stop PostgreSQL stack
#   ./scripts/stop-docker.sh tigerbeetle  # Stop TigerBeetle stack
#   ./scripts/stop-docker.sh all          # Stop both stacks

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
DOCKER_DIR="$PROJECT_DIR/docker"

stop_postgresql() {
    echo "Stopping PostgreSQL stack..."
    docker compose -f "$DOCKER_DIR/docker-compose.postgresql.yml" -p tbperf down -v
    echo "PostgreSQL stack stopped."
}

stop_tigerbeetle() {
    echo "Stopping TigerBeetle stack..."
    docker compose -f "$DOCKER_DIR/docker-compose.tigerbeetle.yml" -p tbperf down -v
    echo "TigerBeetle stack stopped."
}

case "${1:-}" in
    postgresql|pg)
        stop_postgresql
        ;;
    tigerbeetle|tb)
        stop_tigerbeetle
        ;;
    all)
        stop_postgresql 2>/dev/null || true
        stop_tigerbeetle 2>/dev/null || true
        echo "All stacks stopped."
        ;;
    *)
        echo "Usage: $0 {postgresql|tigerbeetle|all}"
        echo ""
        echo "Examples:"
        echo "  $0 postgresql   # Stop PostgreSQL stack"
        echo "  $0 pg           # Stop PostgreSQL stack (short)"
        echo "  $0 tigerbeetle  # Stop TigerBeetle stack"
        echo "  $0 tb           # Stop TigerBeetle stack (short)"
        echo "  $0 all          # Stop both stacks"
        exit 1
        ;;
esac
