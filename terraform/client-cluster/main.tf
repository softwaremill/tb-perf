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
    prefix = "tb-perf/client-cluster"
  }
}

provider "google" {
  project = var.project_id
  region  = var.region
}

data "terraform_remote_state" "network" {
  backend = "gcs"

  config = {
    bucket = var.state_bucket
    prefix = "tb-perf/network"
  }
}

resource "google_service_account" "client_node" {
  account_id   = "tb-perf-client-node"
  display_name = "tb-perf client node"
}

# Client nodes need read access to the artifacts bucket to pull the
# pre-built client binary (see PLAN.md §3.1 - "Client binary deployment via
# a Cloud Storage (GCS) bucket, or built on instance from source").
resource "google_storage_bucket_iam_member" "client_node_artifacts_read" {
  bucket = data.terraform_remote_state.network.outputs.artifacts_bucket_name
  role   = "roles/storage.objectViewer"
  member = "serviceAccount:${google_service_account.client_node.email}"
}

resource "google_compute_instance" "client" {
  count        = var.num_instances
  name         = "tb-perf-client-${count.index}"
  machine_type = var.machine_type
  zone         = var.zones[count.index % length(var.zones)]

  tags   = ["tb-perf-client"]
  labels = {
    app        = "tb-perf"
    role       = "client"
    client-id  = tostring(count.index)
  }

  boot_disk {
    initialize_params {
      image = var.boot_image
      size  = var.boot_disk_size_gb
      type  = "pd-balanced"
    }
  }

  network_interface {
    subnetwork = data.terraform_remote_state.network.outputs.subnet_self_link
    access_config {} # ephemeral external IP (see PLAN.md §3.1 - no NAT/Cloud Router)
  }

  service_account {
    email  = google_service_account.client_node.email
    scopes = ["cloud-platform"]
  }

  metadata = {
    startup-script = file("${path.module}/startup-script.sh")
  }

  allow_stopping_for_update = true
}
