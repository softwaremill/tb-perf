#!/bin/bash
# tb-perf client node startup-script.
#
# Installs Docker (for consistency/debugging use), Rust toolchain (in case
# the client binary is built on-instance rather than pulled from GCS - see
# PLAN.md §3.1), and the Google Cloud CLI (to pull the pre-built binary from
# the artifacts bucket).
#
# GCP re-runs startup-scripts on every boot, so everything here is
# idempotent. This script is static (not templated) since client nodes are
# interchangeable - it takes no per-instance parameters.

set -euo pipefail
exec > >(tee -a /var/log/tb-perf-startup.log) 2>&1

echo "=== tb-perf client node startup ==="

# --- Install Docker ---
if ! command -v docker >/dev/null 2>&1; then
  echo "Installing Docker..."
  apt-get update
  apt-get install -y ca-certificates curl gnupg build-essential
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

# --- Install Rust toolchain (used only if building the client on-instance) ---
# Installed to a shared, world-readable location rather than the default
# $HOME/.cargo - this startup-script runs as root, but the client binary is
# built later over SSH as the OS-Login user, not root, so a root-only
# /root/.cargo would be inaccessible to it.
export RUSTUP_HOME=/opt/rust/rustup
export CARGO_HOME=/opt/rust/cargo

if [ ! -x /opt/rust/cargo/bin/cargo ]; then
  echo "Installing Rust toolchain..."
  mkdir -p "$RUSTUP_HOME" "$CARGO_HOME"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
fi

chmod -R a+rwX /opt/rust

# Make cargo/rustc available on PATH for interactive login shells (SSH
# --command invocations reference the full path directly instead, since
# non-interactive shells don't source /etc/profile.d).
cat >/etc/profile.d/tb-perf-cargo.sh <<'EOF'
export RUSTUP_HOME=/opt/rust/rustup
export CARGO_HOME=/opt/rust/cargo
export PATH="/opt/rust/cargo/bin:$PATH"
EOF
chmod +r /etc/profile.d/tb-perf-cargo.sh

echo "=== tb-perf client node startup complete ==="
