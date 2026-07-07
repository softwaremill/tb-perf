#!/bin/bash
# tb-perf monitoring node startup-script.
#
# Installs Docker only. The OTel Collector / Prometheus / Grafana stack
# itself is started later by the coordinator (over IAP-SSH), which pushes
# the same docker-compose service definitions used for local testing - see
# PLAN.md §3.4.
#
# GCP re-runs startup-scripts on every boot, so this is idempotent.

set -euo pipefail
exec > >(tee -a /var/log/tb-perf-startup.log) 2>&1

echo "=== tb-perf monitoring node startup ==="

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

echo "=== tb-perf monitoring node startup complete ==="
