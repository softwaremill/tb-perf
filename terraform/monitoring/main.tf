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
    prefix = "tb-perf/monitoring"
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

resource "google_service_account" "monitoring" {
  account_id   = "tb-perf-monitoring"
  display_name = "tb-perf monitoring node"
}

# One instance running the OTel Collector + Prometheus + Grafana stack via
# Docker Compose (see PLAN.md §3.4). Terraform only installs Docker here;
# the coordinator pushes the actual docker-compose/config files and starts
# the stack over `gcloud compute ssh --tunnel-through-iap` at pre-test setup
# time, reusing the same compose service definitions as the local setup
# (docker/docker-compose.{postgresql,tigerbeetle}.yml's otel-collector,
# prometheus and grafana services).
resource "google_compute_instance" "monitoring" {
  name         = "tb-perf-monitoring"
  machine_type = var.machine_type
  zone         = var.zone

  tags   = ["tb-perf-monitoring"]
  labels = {
    app  = "tb-perf"
    role = "monitoring"
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
    access_config {} # ephemeral external IP - locked down to operator_ip via network module's firewall rules
  }

  service_account {
    email  = google_service_account.monitoring.email
    scopes = ["cloud-platform"]
  }

  metadata = {
    startup-script = file("${path.module}/startup-script.sh")
  }

  allow_stopping_for_update = true
}
