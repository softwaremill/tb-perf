#!/bin/bash
# tb-perf: bring up one PostgreSQL node as part of a 3-node synchronous
# replication cluster (1 primary + 2 standbys), for the initial "Pre-Test
# Setup" phase (see PLAN.md §3.2/§3.4). This is a ONE-TIME cluster bring-up
# script, not a between-run reset (resets between runs reuse the existing
# TRUNCATE-based logic from coordinator/src/postgres_setup.rs, run against
# the primary over a remote connection instead of `docker compose exec`).
#
# This is copied to each DB node and run by the coordinator over
# `gcloud compute ssh --tunnel-through-iap` (see coordinator/src/gcp_setup.rs).
# Not meant to be run manually except for debugging.
#
# NOTE on quorum: PLAN.md §3.1 specifies `synchronous_standby_names =
# 'ANY 2 (*)'` for "2-of-3 quorum", but with only 2 standbys configured,
# "ANY 2 (*)" actually requires BOTH standbys to ack (i.e. 3-of-3, stricter
# than TigerBeetle's leader+1-of-3 majority quorum in the same topology).
# To match TigerBeetle's actual write quorum (leader + 1 other node = 2 of 3
# total), this script uses `ANY 1 (...)` instead. Flagging this explicitly -
# override to `ANY 2 (...)` here if you specifically want the stricter,
# non-equivalent guarantee instead.
#
# Usage:
#   gcp-setup-postgresql.sh primary <standby_name1> <standby_name2>
#   gcp-setup-postgresql.sh standby <primary_internal_ip> <own_standby_name>

set -euo pipefail

ROLE="${1:?Usage: gcp-setup-postgresql.sh {primary|standby} ...}"

DATA_DIR="/mnt/tb-perf-data/postgresql"
IMAGE="postgres:16" # keep in sync with docker/docker-compose.postgresql.yml
CONTAINER_NAME="tb-perf-postgres"
REPL_USER="tbperf_repl"
REPL_PASSWORD="tbperf_repl_password" # benchmark-only cluster; not reachable outside the VPC (see terraform/network firewall rules)

mkdir -p "$DATA_DIR"
docker rm -f "$CONTAINER_NAME" >/dev/null 2>&1 || true

case "$ROLE" in
  primary)
    STANDBY1_NAME="${2:?standby_name1 required}"
    STANDBY2_NAME="${3:?standby_name2 required}"

    echo "=== Setting up PostgreSQL PRIMARY (standbys: $STANDBY1_NAME, $STANDBY2_NAME) ==="
    rm -rf "${DATA_DIR:?}"
    mkdir -p "$DATA_DIR"

    docker run -d \
      --name "$CONTAINER_NAME" \
      --network host \
      --restart unless-stopped \
      -e POSTGRES_USER=postgres \
      -e POSTGRES_PASSWORD=postgres \
      -e POSTGRES_DB=tbperf \
      -v "$DATA_DIR:/var/lib/postgresql/data" \
      "$IMAGE" \
      postgres \
        -c listen_addresses='*' \
        -c max_connections=200 \
        -c shared_buffers=256MB \
        -c wal_level=replica \
        -c max_wal_senders=10 \
        -c max_replication_slots=10 \
        -c hot_standby=on \
        -c synchronous_commit=on \
        -c "synchronous_standby_names=ANY 1 ($STANDBY1_NAME, $STANDBY2_NAME)"

    echo "Waiting for primary to accept connections..."
    for _ in $(seq 1 30); do
      if docker exec "$CONTAINER_NAME" pg_isready -U postgres >/dev/null 2>&1; then
        break
      fi
      sleep 1
    done

    echo "Creating replication role..."
    docker exec "$CONTAINER_NAME" psql -U postgres -c \
      "CREATE ROLE $REPL_USER WITH REPLICATION LOGIN PASSWORD '$REPL_PASSWORD';"

    echo "Allowing replication connections from within the VPC subnet..."
    docker exec "$CONTAINER_NAME" bash -c \
      "echo 'host replication $REPL_USER 10.10.0.0/20 md5' >> /var/lib/postgresql/data/pg_hba.conf"
    docker exec "$CONTAINER_NAME" bash -c \
      "echo 'host all all 10.10.0.0/20 md5' >> /var/lib/postgresql/data/pg_hba.conf"
    docker exec "$CONTAINER_NAME" psql -U postgres -c "SELECT pg_reload_conf();"

    echo "=== PostgreSQL primary ready ==="
    ;;

  standby)
    PRIMARY_IP="${2:?primary_internal_ip required}"
    STANDBY_NAME="${3:?own_standby_name required}"

    echo "=== Setting up PostgreSQL STANDBY '$STANDBY_NAME' (primary=$PRIMARY_IP) ==="
    rm -rf "${DATA_DIR:?}"
    mkdir -p "$DATA_DIR"

    echo "Waiting for primary to become reachable..."
    for _ in $(seq 1 60); do
      if docker run --rm --network host \
           -e PGPASSWORD="$REPL_PASSWORD" "$IMAGE" \
           pg_isready -h "$PRIMARY_IP" -p 5432 -U "$REPL_USER" >/dev/null 2>&1; then
        break
      fi
      sleep 2
    done

    echo "Taking base backup from primary..."
    # -d with a full conninfo string lets us set application_name, which must
    # match one of the names listed in the primary's synchronous_standby_names.
    docker run --rm --network host \
      -e PGPASSWORD="$REPL_PASSWORD" \
      -v "$DATA_DIR:/var/lib/postgresql/data" \
      "$IMAGE" \
      pg_basebackup \
        -d "host=$PRIMARY_IP port=5432 user=$REPL_USER application_name=$STANDBY_NAME" \
        -D /var/lib/postgresql/data -Fp -Xs -P -R

    docker run -d \
      --name "$CONTAINER_NAME" \
      --network host \
      --restart unless-stopped \
      -v "$DATA_DIR:/var/lib/postgresql/data" \
      "$IMAGE" \
      postgres -c listen_addresses='*' -c hot_standby=on

    echo "Waiting for standby to accept read-only connections..."
    for _ in $(seq 1 30); do
      if docker exec "$CONTAINER_NAME" pg_isready -U postgres >/dev/null 2>&1; then
        break
      fi
      sleep 1
    done

    echo "=== PostgreSQL standby '$STANDBY_NAME' ready ==="
    ;;

  *)
    echo "Error: unknown role '$ROLE' (expected primary|standby)" >&2
    exit 1
    ;;
esac
