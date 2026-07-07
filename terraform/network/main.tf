terraform {
  required_version = ">= 1.5"

  required_providers {
    google = {
      source  = "hashicorp/google"
      version = "~> 5.0"
    }
  }

  # Bootstrap the bucket first with ../../scripts/gcp-bootstrap-tfstate.sh
  backend "gcs" {
    bucket = "tigerbettle-sandbox-tfstate"
    prefix = "tb-perf/network"
  }
}

provider "google" {
  project = var.project_id
  region  = var.region
}

# Single VPC for the whole benchmark environment. All instances (DB, client,
# monitoring) live in one subnet and are distinguished only by network tags,
# so firewall rules don't need to know specific IPs or depend on other
# Terraform modules having already run.
resource "google_compute_network" "tb_perf" {
  name                    = "tb-perf-vpc"
  auto_create_subnetworks = false
  description             = "tb-perf: TigerBeetle vs PostgreSQL benchmark environment"
}

# GCP subnets already span all zones in a region, so one subnet covers our
# 3-zone (europe-central2-a/b/c) DB cluster layout.
resource "google_compute_subnetwork" "tb_perf" {
  name                     = "tb-perf-subnet"
  network                  = google_compute_network.tb_perf.id
  region                   = var.region
  ip_cidr_range            = var.subnet_cidr
  private_ip_google_access = true
}

# GCS bucket for the pre-built client binary and exported results/logs
# (the S3-equivalent artifact/results storage described in PLAN.md §3.1).
# force_destroy is enabled since this is ephemeral benchmark infrastructure -
# results should already be synced to your laptop before `terraform destroy`.
resource "google_storage_bucket" "artifacts" {
  name                        = "${var.project_id}-tb-perf-artifacts"
  project                     = var.project_id
  location                    = var.region
  uniform_bucket_level_access = true
  force_destroy               = true

  lifecycle_rule {
    condition {
      age = 30
    }
    action {
      type = "Delete"
    }
  }
}

# --- Firewall rules ---
# Instances get external IPs directly (no Cloud NAT/Router - see PLAN.md
# §3.1); access is controlled entirely through these rules instead of
# network isolation, since the whole environment is ephemeral.

# DB nodes talk to each other (PostgreSQL streaming replication /
# TigerBeetle replica-to-replica traffic).
resource "google_compute_firewall" "allow_db_internal" {
  name    = "tb-perf-allow-db-internal"
  network = google_compute_network.tb_perf.id

  allow {
    protocol = "tcp"
    # 5432: PostgreSQL, 3000: TigerBeetle (also used for its replication traffic)
    ports = ["5432", "3000"]
  }

  source_tags = ["tb-perf-db"]
  target_tags = ["tb-perf-db"]
}

# Client nodes send transfer/query traffic to DB nodes.
resource "google_compute_firewall" "allow_client_to_db" {
  name    = "tb-perf-allow-client-to-db"
  network = google_compute_network.tb_perf.id

  allow {
    protocol = "tcp"
    ports    = ["5432", "3000"]
  }

  source_tags = ["tb-perf-client"]
  target_tags = ["tb-perf-db"]
}

# Monitoring instance scrapes node-exporter on DB and client nodes.
resource "google_compute_firewall" "allow_monitoring_scrape" {
  name    = "tb-perf-allow-monitoring-scrape"
  network = google_compute_network.tb_perf.id

  allow {
    protocol = "tcp"
    ports    = ["9100"] # node-exporter
  }

  source_tags = ["tb-perf-monitoring"]
  target_tags = ["tb-perf-db", "tb-perf-client"]
}

# Client (and DB, for completeness) nodes push metrics to the OTel
# Collector running on the monitoring instance.
resource "google_compute_firewall" "allow_metrics_to_monitoring" {
  name    = "tb-perf-allow-metrics-to-monitoring"
  network = google_compute_network.tb_perf.id

  allow {
    protocol = "tcp"
    ports    = ["4317", "4318"] # OTLP gRPC / HTTP
  }

  source_tags = ["tb-perf-client", "tb-perf-db"]
  target_tags = ["tb-perf-monitoring"]
}

# Operator (you) can reach Grafana/Prometheus directly from your own IP.
resource "google_compute_firewall" "allow_operator_to_monitoring" {
  name    = "tb-perf-allow-operator-monitoring"
  network = google_compute_network.tb_perf.id

  allow {
    protocol = "tcp"
    ports    = ["3000", "3001", "9090"] # Grafana (3000/3001), Prometheus
  }

  source_ranges = [var.operator_ip]
  target_tags   = ["tb-perf-monitoring"]
}

# SSH access for all tb-perf instances is only permitted via IAP tunneling
# (gcloud compute ssh --tunnel-through-iap), never from the open internet.
# 35.235.240.0/20 is Google's fixed IAP forwarding range.
resource "google_compute_firewall" "allow_iap_ssh" {
  name    = "tb-perf-allow-iap-ssh"
  network = google_compute_network.tb_perf.id

  allow {
    protocol = "tcp"
    ports    = ["22"]
  }

  source_ranges = ["35.235.240.0/20"]
  target_tags   = ["tb-perf-db", "tb-perf-client", "tb-perf-monitoring"]
}
