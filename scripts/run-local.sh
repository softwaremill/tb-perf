#!/bin/bash
# Parametrized local test runner
#
# Usage:
#   ./scripts/run-local.sh <mode> <executor> [concurrency] [pool_size]
#
# Modes:
#   sanity  - Quick test (~30s) for verification
#   quick   - Medium test (~5min) with 3 iterations
#   normal  - Full test (~25min) for benchmarking
#
# Executors:
#   postgres-standard  - PostgreSQL with SELECT FOR UPDATE locks
#   postgres-atomic    - PostgreSQL with atomic UPDATE (no explicit locks)
#   postgres-batched   - PostgreSQL with batched transfers
#   tigerbeetle        - TigerBeetle
#
# Output (to stdout):
#   Results: <results_directory>
#   TPS: <throughput_tps>
#   Error rate: <error_rate_percent>
#
# All other output is saved to <results_directory>/console.log

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

# Default values
DEFAULT_CONCURRENCY=4
DEFAULT_POOL_SIZE=8

# Parse arguments
MODE="${1:-}"
EXECUTOR="${2:-}"
CONCURRENCY="${3:-$DEFAULT_CONCURRENCY}"
POOL_SIZE="${4:-$DEFAULT_POOL_SIZE}"

if [ -z "$MODE" ] || [ -z "$EXECUTOR" ]; then
    echo "Usage: $0 <mode> <executor> [concurrency] [pool_size]" >&2
    echo "Modes: sanity, quick, normal" >&2
    echo "Executors: postgres-standard, postgres-atomic, postgres-batched, tigerbeetle" >&2
    exit 1
fi

# Validate mode and set config prefix
case "$MODE" in
    sanity)
        CONFIG_PREFIX="config.sanity"
        ;;
    quick)
        CONFIG_PREFIX="config.quick"
        ;;
    normal|full)
        CONFIG_PREFIX="config.local"
        ;;
    *)
        echo "Error: Unknown mode '$MODE' (use: sanity, quick, normal)" >&2
        exit 1
        ;;
esac

# Validate executor and set config details
case "$EXECUTOR" in
    postgres-standard|pg-standard|pg)
        BASE_CONFIG="${CONFIG_PREFIX}-postgresql.toml"
        DB_TYPE="postgresql"
        ;;
    postgres-atomic|pg-atomic)
        BASE_CONFIG="${CONFIG_PREFIX}-postgresql-atomic.toml"
        DB_TYPE="postgresql"
        ;;
    postgres-batched|pg-batched)
        BASE_CONFIG="${CONFIG_PREFIX}-postgresql-batched.toml"
        DB_TYPE="postgresql"
        ;;
    tigerbeetle|tb)
        BASE_CONFIG="${CONFIG_PREFIX}-tigerbeetle.toml"
        DB_TYPE="tigerbeetle"
        ;;
    *)
        echo "Error: Unknown executor '$EXECUTOR'" >&2
        exit 1
        ;;
esac

# Verify config exists
if [ ! -f "$PROJECT_DIR/$BASE_CONFIG" ]; then
    echo "Error: Config file not found: $BASE_CONFIG" >&2
    exit 1
fi

# Create temporary files
TEMP_CONFIG=$(mktemp /tmp/tb-perf-XXXXXX.toml)
TEMP_LOG=$(mktemp /tmp/tb-perf-console-XXXXXX.log)

# Copy base config and override parameters
cp "$PROJECT_DIR/$BASE_CONFIG" "$TEMP_CONFIG"
sed -i.bak "s/^concurrency = .*/concurrency = $CONCURRENCY/" "$TEMP_CONFIG"
sed -i.bak "s/^connection_pool_size = .*/connection_pool_size = $POOL_SIZE/" "$TEMP_CONFIG"
rm -f "${TEMP_CONFIG}.bak"

# Cleanup function
cleanup() {
    local exit_code=$?

    # Cleanup infrastructure
    if [ "$DB_TYPE" = "tigerbeetle" ]; then
        "$SCRIPT_DIR/tigerbeetle-local.sh" wipe >> "$TEMP_LOG" 2>&1 || true
        "$SCRIPT_DIR/stop-docker.sh" tigerbeetle >> "$TEMP_LOG" 2>&1 || true
    else
        "$SCRIPT_DIR/stop-docker.sh" postgresql >> "$TEMP_LOG" 2>&1 || true
    fi

    # Move log to results directory if it exists
    if [ -n "$RESULTS_DIR" ] && [ -d "$RESULTS_DIR" ]; then
        mv "$TEMP_LOG" "$RESULTS_DIR/console.log"
    else
        rm -f "$TEMP_LOG"
    fi

    rm -f "$TEMP_CONFIG"
    exit $exit_code
}

trap cleanup EXIT

# Start logging
exec 3>&1 4>&2
exec 1>>"$TEMP_LOG" 2>&1

echo "TB-Perf Local Test"
echo "Mode:        $MODE"
echo "Executor:    $EXECUTOR"
echo "Concurrency: $CONCURRENCY"
echo "Pool size:   $POOL_SIZE"
echo "Config:      $BASE_CONFIG"
echo ""

# Build the project
echo "Building project..."
cd "$PROJECT_DIR"
cargo build --release

# Run the test based on database type
if [ "$DB_TYPE" = "tigerbeetle" ]; then
    echo "Starting TigerBeetle locally..."
    "$SCRIPT_DIR/tigerbeetle-local.sh" start

    echo "Starting monitoring stack..."
    docker compose -f "$PROJECT_DIR/docker/docker-compose.tigerbeetle.yml" -p tbperf up -d otel-collector prometheus grafana

    echo "Running test..."
    ./target/release/coordinator -c "$TEMP_CONFIG" --no-docker
else
    echo "Running test..."
    ./target/release/coordinator -c "$TEMP_CONFIG"
fi

# Find the most recent results directory
RESULTS_DIR=$(ls -td "$PROJECT_DIR/results/run_"* 2>/dev/null | head -1)

if [ -z "$RESULTS_DIR" ] || [ ! -d "$RESULTS_DIR" ]; then
    echo "Error: Could not find results directory" >&2
    exec 1>&3 2>&4
    echo "ERROR"
    echo "0"
    echo "100"
    exit 1
fi

# Extract metrics from results.json
RESULTS_JSON="$RESULTS_DIR/results.json"
if [ -f "$RESULTS_JSON" ]; then
    TPS=$(jq -r '.runs[0].throughput_tps // 0' "$RESULTS_JSON" 2>/dev/null || echo "0")
    ERROR_RATE=$(jq -r '
        .runs[0] |
        if (.completed_transfers + .rejected_transfers + .failed_transfers) > 0 then
            (100 * .failed_transfers / (.completed_transfers + .rejected_transfers + .failed_transfers))
        else
            0
        end
    ' "$RESULTS_JSON" 2>/dev/null || echo "0")
else
    TPS="0"
    ERROR_RATE="100"
fi

# Restore stdout/stderr and output results
exec 1>&3 2>&4

echo "Results: $RESULTS_DIR"
echo "TPS: $TPS"
echo "Error rate: $ERROR_RATE"
