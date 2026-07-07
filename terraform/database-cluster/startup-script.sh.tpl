#!/bin/bash
# tb-perf DB node startup-script.
#
# Scope is intentionally limited to baseline OS setup: mounting the Local
# SSD and installing Docker/tooling. It does NOT configure PostgreSQL
# replication or format the TigerBeetle cluster - that happens later, driven
# by the coordinator over `gcloud compute ssh --tunnel-through-iap`, since it
# needs to be reconfigurable per test config and reset between runs
# (see PLAN.md §3.2 / §3.4).
#
# GCP re-runs startup-scripts on every boot, so everything here must be
# idempotent.

set -euo pipefail
exec > >(tee -a /var/log/tb-perf-startup.log) 2>&1

echo "=== tb-perf DB node startup: database_type=${database_type} replica_index=${replica_index} node_count=${node_count} ==="

# --- Mount Local SSD ---
DEVICE="/dev/disk/by-id/google-local-ssd-0"
MOUNT_POINT="/mnt/tb-perf-data"

mkdir -p "$MOUNT_POINT"

if [ -e "$DEVICE" ]; then
  if ! mountpoint -q "$MOUNT_POINT"; then
    # Only format if it doesn't already look like an ext4 filesystem -
    # avoids wiping data on a startup-script re-run after a reboot.
    if ! blkid "$DEVICE" >/dev/null 2>&1; then
      echo "Formatting Local SSD at $DEVICE..."
      mkfs.ext4 -F "$DEVICE"
    fi
    mount -o discard,defaults "$DEVICE" "$MOUNT_POINT"
    chmod 777 "$MOUNT_POINT"
    echo "Local SSD mounted at $MOUNT_POINT"
  fi
else
  echo "WARNING: Local SSD device not found at $DEVICE - falling back to boot disk for data"
fi

# --- Install Docker ---
if ! command -v docker >/dev/null 2>&1; then
  echo "Installing Docker..."
  apt-get update
  apt-get install -y ca-certificates curl gnupg
  install -m 0755 -d /etc/apt/keyrings
  curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc
  chmod a+r /etc/apt/keyrings/docker.asc
  ARCH="$(dpkg --print-architecture)"
  CODENAME="$(. /etc/os-release && echo "$VERSION_CODENAME")"
  echo "deb [arch=$ARCH signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu $CODENAME stable" \
    > /etc/apt/sources.list.d/docker.list
  apt-get update
  apt-get install -y docker-ce docker-ce-cli containerd.io docker-compose-plugin
  systemctl enable docker
  systemctl start docker
fi

# --- Install Google Cloud CLI (for gcloud/gsutil access to GCS) ---
if ! command -v gcloud >/dev/null 2>&1; then
  echo "Installing Google Cloud CLI..."
  echo "deb [signed-by=/usr/share/keyrings/cloud.google.gpg] https://packages.cloud.google.com/apt cloud-sdk main" \
    > /etc/apt/sources.list.d/google-cloud-sdk.list
  curl -fsSL https://packages.cloud.google.com/apt/doc/apt-key.gpg \
    | gpg --dearmor -o /usr/share/keyrings/cloud.google.gpg
  apt-get update
  apt-get install -y google-cloud-cli
fi

# --- Node exporter (scraped by Prometheus on the monitoring instance) ---
if [ ! -f /usr/local/bin/node_exporter ]; then
  echo "Installing node_exporter..."
  id -u node_exporter >/dev/null 2>&1 || useradd --no-create-home --shell /usr/sbin/nologin node_exporter

  NODE_EXPORTER_VERSION="1.7.0"
  curl -fsSL "https://github.com/prometheus/node_exporter/releases/download/v$${NODE_EXPORTER_VERSION}/node_exporter-$${NODE_EXPORTER_VERSION}.linux-amd64.tar.gz" \
    -o /tmp/node_exporter.tar.gz
  tar -xzf /tmp/node_exporter.tar.gz -C /tmp
  mv "/tmp/node_exporter-$${NODE_EXPORTER_VERSION}.linux-amd64/node_exporter" /usr/local/bin/node_exporter
  rm -rf /tmp/node_exporter.tar.gz "/tmp/node_exporter-$${NODE_EXPORTER_VERSION}.linux-amd64"

  cat >/etc/systemd/system/node_exporter.service <<'UNIT'
[Unit]
Description=Node Exporter
After=network.target

[Service]
User=node_exporter
ExecStart=/usr/local/bin/node_exporter

[Install]
WantedBy=multi-user.target
UNIT

  systemctl daemon-reload
  systemctl enable --now node_exporter
fi

echo "=== tb-perf DB node startup complete (database_type=${database_type}) ==="
echo "Database software setup is handled separately by the coordinator."
