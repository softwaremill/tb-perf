terraform {
  required_version = ">= 1.5"

  required_providers {
    google = {
      source  = "hashicorp/google"
      version = "~> 5.0"
    }
  }

  backend "gcs" {
    bucket = "tigerbettle-sandbox-tfstate"
    prefix = "tb-perf/database-cluster"
  }
}

provider "google" {
  project = var.project_id
  region  = var.region
}

# Read the network module's outputs instead of hardcoding IDs, so this
# module can be applied/destroyed independently of the network module.
data "terraform_remote_state" "network" {
  backend = "gcs"

  config = {
    bucket = var.state_bucket
    prefix = "tb-perf/network"
  }
}

# Least-privilege service account: DB nodes only need to read/write to
# Cloud Storage (client binary pull is not needed here, but results/logs
# and version-pinned artifacts may be staged through GCS).
resource "google_service_account" "db_node" {
  account_id   = "tb-perf-db-node"
  display_name = "tb-perf database node (${var.database_type})"
}

resource "google_storage_bucket_iam_member" "db_node_artifacts_admin" {
  bucket = data.terraform_remote_state.network.outputs.artifacts_bucket_name
  role   = "roles/storage.objectAdmin"
  member = "serviceAccount:${google_service_account.db_node.email}"
}

resource "google_compute_instance" "db" {
  count        = var.node_count
  name         = "tb-perf-db-${var.database_type}-${count.index}"
  machine_type = var.machine_type
  zone         = var.zones[count.index % length(var.zones)]

  tags   = ["tb-perf-db"]
  labels = {
    app        = "tb-perf"
    role       = "db"
    db-type    = var.database_type
    replica-id = tostring(count.index)
  }

  boot_disk {
    initialize_params {
      image = var.boot_image
      size  = var.boot_disk_size_gb
      type  = "pd-balanced"
    }
  }

  dynamic "scratch_disk" {
    for_each = range(var.local_ssd_count)
    content {
      interface = "NVME"
    }
  }

  network_interface {
    subnetwork = data.terraform_remote_state.network.outputs.subnet_self_link
    access_config {} # ephemeral external IP (see PLAN.md §3.1 - no NAT/Cloud Router)
  }

  service_account {
    email  = google_service_account.db_node.email
    scopes = ["cloud-platform"]
  }

  metadata = {
    startup-script = templatefile("${path.module}/startup-script.sh.tpl", {
      database_type = var.database_type
      replica_index = count.index
      node_count    = var.node_count
    })
  }

  # Local SSD data is ephemeral by design (PLAN.md §3.1) - stopping this
  # instance wipes the DB data, which is expected/acceptable for a
  # benchmark environment that gets reset between runs anyway.
  allow_stopping_for_update = true
}
