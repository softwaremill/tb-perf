#!/bin/bash
# tb-perf: format and start one TigerBeetle replica as part of a 3-node
# cluster (see PLAN.md §3.1/§3.2).
#
# This is copied to each DB node and run by the coordinator over
# `gcloud compute ssh --tunnel-through-iap` (see coordinator/src/gcp_setup.rs).
# Not meant to be run manually except for debugging.
#
# All replicas are given the SAME address list (in replica-index order);
# each replica binds to the address at its own index and connects out to
# the others - this is standard TigerBeetle multi-replica usage.
#
# Usage:
#   gcp-setup-tigerbeetle.sh <replica_index> <replica_count> <addr1> <addr2> [<addr3> ...]
#
# Addresses should be internal IPs (DB-to-DB traffic stays off the public internet).

set -euo pipefail

REPLICA_INDEX="${1:?Usage: gcp-setup-tigerbeetle.sh <replica_index> <replica_count> <addr1> <addr2> ...}"
REPLICA_COUNT="${2:?replica_count required}"
shift 2
if [ "$#" -lt 1 ]; then
  echo "Error: at least one address is required" >&2
  exit 1
fi
ADDRESSES="$(IFS=,; echo "$*")"

DATA_DIR="/mnt/tb-perf-data/tigerbeetle"
DATA_FILE="$DATA_DIR/${REPLICA_INDEX}_0.tigerbeetle"
IMAGE="ghcr.io/tigerbeetle/tigerbeetle:0.16.78" # keep in sync with docker/docker-compose.tigerbeetle.yml and tigerbeetle-unofficial's wrapped version in Cargo.toml
CONTAINER_NAME="tb-perf-tigerbeetle"

mkdir -p "$DATA_DIR"

echo "=== TigerBeetle replica $REPLICA_INDEX/$REPLICA_COUNT (addresses=$ADDRESSES) ==="

docker rm -f "$CONTAINER_NAME" >/dev/null 2>&1 || true

if [ ! -f "$DATA_FILE" ]; then
  echo "Formatting data file..."
  # --security-opt seccomp=unconfined: Docker's default seccomp profile
  # blocks the io_uring_setup/io_uring_enter/io_uring_register syscalls
  # TigerBeetle requires, independent of (and checked before) the host
  # kernel's io_uring_disabled sysctl. Acceptable tradeoff on a dedicated,
  # ephemeral benchmark VM (see PLAN.md §3.1 - not a shared/production host).
  docker run --rm \
    --security-opt seccomp=unconfined \
    -v "$DATA_DIR:/data" \
    "$IMAGE" \
    format --cluster=0 --replica="$REPLICA_INDEX" --replica-count="$REPLICA_COUNT" \
    "/data/${REPLICA_INDEX}_0.tigerbeetle"
else
  echo "Data file already exists - skipping format (use gcp-wipe-db.sh to reset)"
fi

echo "Starting TigerBeetle..."
docker run -d \
  --name "$CONTAINER_NAME" \
  --network host \
  --restart unless-stopped \
  --security-opt seccomp=unconfined \
  -v "$DATA_DIR:/data" \
  "$IMAGE" \
  start --addresses="$ADDRESSES" "/data/${REPLICA_INDEX}_0.tigerbeetle"

echo "=== TigerBeetle replica $REPLICA_INDEX started ==="
