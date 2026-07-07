#!/bin/bash
# Bootstrap the GCS bucket used for Terraform remote state.
#
# The Terraform "gcs" backend requires its bucket to already exist before
# `terraform init` can use it (chicken-and-egg problem), so this one-time
# setup is done here rather than in Terraform itself.
#
# Run this once per project, before the first `terraform init` in terraform/.
#
# Usage:
#   ./scripts/gcp-bootstrap-tfstate.sh
#
# Environment variables (all optional, sensible defaults shown):
#   GCP_PROJECT_ID   GCP project ID              (default: tigerbettle-sandbox)
#   GCP_REGION       Region for the bucket       (default: europe-central2)
#   TF_STATE_BUCKET  Bucket name                 (default: ${GCP_PROJECT_ID}-tfstate)

set -euo pipefail

PROJECT_ID="${GCP_PROJECT_ID:-tigerbettle-sandbox}"
REGION="${GCP_REGION:-europe-central2}"
BUCKET_NAME="${TF_STATE_BUCKET:-${PROJECT_ID}-tfstate}"

echo "Project:      $PROJECT_ID"
echo "Region:       $REGION"
echo "State bucket: gs://$BUCKET_NAME"
echo ""

if ! command -v gcloud >/dev/null 2>&1; then
    echo "Error: gcloud CLI not found. Install it from https://cloud.google.com/sdk/docs/install"
    exit 1
fi

# Confirm gcloud has an active login
if ! gcloud auth list --filter=status:ACTIVE --format="value(account)" | grep -q .; then
    echo "Error: no active gcloud authentication found."
    echo "Run: gcloud auth login && gcloud auth application-default login"
    exit 1
fi

# Confirm the project exists and is accessible
if ! gcloud projects describe "$PROJECT_ID" >/dev/null 2>&1; then
    echo "Error: cannot access project '$PROJECT_ID'."
    echo "Check the project ID and that your account has at least Viewer access."
    exit 1
fi

# Create the bucket if it doesn't already exist (idempotent)
if gcloud storage buckets describe "gs://$BUCKET_NAME" --project="$PROJECT_ID" >/dev/null 2>&1; then
    echo "Bucket gs://$BUCKET_NAME already exists - skipping creation."
else
    echo "Creating bucket gs://$BUCKET_NAME in $REGION..."
    gcloud storage buckets create "gs://$BUCKET_NAME" \
        --project="$PROJECT_ID" \
        --location="$REGION" \
        --uniform-bucket-level-access \
        --default-storage-class=STANDARD

    echo "Enabling versioning (lets you recover/inspect previous state file versions)..."
    gcloud storage buckets update "gs://$BUCKET_NAME" --versioning
fi

echo ""
echo "Done. Reference this bucket in each Terraform module's backend config, e.g.:"
echo ""
cat <<EOF
terraform {
  backend "gcs" {
    bucket = "$BUCKET_NAME"
    prefix = "tb-perf/network"  # use a distinct prefix per module (network, database-cluster, client-cluster)
  }
}
EOF
