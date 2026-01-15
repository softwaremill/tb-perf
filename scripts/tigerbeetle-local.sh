#!/bin/bash
# Manage a local TigerBeetle instance (for macOS where Docker io_uring doesn't work)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
TIGERBEETLE_BIN="$PROJECT_DIR/tigerbeetle"

DATA_DIR="${TIGERBEETLE_DATA_DIR:-/tmp/tb-perf}"
DATA_FILE="$DATA_DIR/0_0.tigerbeetle"
PID_FILE="$DATA_DIR/tigerbeetle.pid"
LOG_FILE="$DATA_DIR/tigerbeetle.log"
ADDRESS="${TIGERBEETLE_ADDRESS:-3000}"

usage() {
    echo "Usage: $0 {start|stop|wipe|status}"
    echo ""
    echo "Commands:"
    echo "  start   Start TigerBeetle (formats data file if needed)"
    echo "  stop    Stop TigerBeetle"
    echo "  wipe    Stop TigerBeetle and delete all data"
    echo "  status  Check if TigerBeetle is running"
    echo ""
    echo "Environment variables:"
    echo "  TIGERBEETLE_DATA_DIR  Data directory (default: /tmp/tb-perf)"
    echo "  TIGERBEETLE_ADDRESS   Listen port (default: 3000)"
    exit 1
}

check_tigerbeetle() {
    if [ ! -x "$TIGERBEETLE_BIN" ]; then
        echo "Error: tigerbeetle binary not found at $TIGERBEETLE_BIN"
        echo "Download it from: https://tigerbeetle.com/install.sh"
        exit 1
    fi
}

start_tigerbeetle() {
    check_tigerbeetle

    if [ -f "$PID_FILE" ] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
        echo "TigerBeetle is already running (PID: $(cat "$PID_FILE"))"
        return 0
    fi

    mkdir -p "$DATA_DIR"

    # Format data file if it doesn't exist
    if [ ! -f "$DATA_FILE" ]; then
        echo "Formatting TigerBeetle data file..."
        "$TIGERBEETLE_BIN" format --cluster=0 --replica=0 --replica-count=1 "$DATA_FILE"
    fi

    echo "Starting TigerBeetle on port $ADDRESS..."
    "$TIGERBEETLE_BIN" start --addresses="$ADDRESS" "$DATA_FILE" > "$LOG_FILE" 2>&1 &
    echo $! > "$PID_FILE"

    # Wait a moment and verify it started
    sleep 1
    if kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
        echo "TigerBeetle started (PID: $(cat "$PID_FILE"))"
        echo "Logs: $LOG_FILE"
    else
        echo "Error: TigerBeetle failed to start. Check logs: $LOG_FILE"
        cat "$LOG_FILE"
        rm -f "$PID_FILE"
        exit 1
    fi
}

stop_tigerbeetle() {
    if [ ! -f "$PID_FILE" ]; then
        echo "TigerBeetle is not running (no PID file)"
        return 0
    fi

    PID=$(cat "$PID_FILE")
    if kill -0 "$PID" 2>/dev/null; then
        echo "Stopping TigerBeetle (PID: $PID)..."
        kill "$PID"
        # Wait for graceful shutdown
        for i in {1..10}; do
            if ! kill -0 "$PID" 2>/dev/null; then
                break
            fi
            sleep 0.5
        done
        # Force kill if still running
        if kill -0 "$PID" 2>/dev/null; then
            echo "Force killing TigerBeetle..."
            kill -9 "$PID" 2>/dev/null || true
        fi
        echo "TigerBeetle stopped"
    else
        echo "TigerBeetle process not found (stale PID file)"
    fi
    rm -f "$PID_FILE"
}

wipe_tigerbeetle() {
    stop_tigerbeetle
    if [ -d "$DATA_DIR" ]; then
        echo "Wiping TigerBeetle data directory: $DATA_DIR"
        rm -rf "$DATA_DIR"
        echo "Data wiped"
    else
        echo "No data directory to wipe"
    fi
}

status_tigerbeetle() {
    if [ -f "$PID_FILE" ] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
        echo "TigerBeetle is running (PID: $(cat "$PID_FILE"))"
        echo "Data directory: $DATA_DIR"
        echo "Address: $ADDRESS"
        return 0
    else
        echo "TigerBeetle is not running"
        return 1
    fi
}

case "${1:-}" in
    start)
        start_tigerbeetle
        ;;
    stop)
        stop_tigerbeetle
        ;;
    wipe)
        wipe_tigerbeetle
        ;;
    status)
        status_tigerbeetle
        ;;
    *)
        usage
        ;;
esac
