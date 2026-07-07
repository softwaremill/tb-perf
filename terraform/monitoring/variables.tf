variable "project_id" {
  description = "GCP project ID"
  type        = string
  default     = "tigerbettle-sandbox"
}

variable "region" {
  description = "GCP region"
  type        = string
  default     = "europe-central2"
}

variable "zone" {
  description = "Zone for the monitoring instance"
  type        = string
  default     = "europe-central2-a"
}

variable "machine_type" {
  description = "GCE machine type for the monitoring instance"
  type        = string
  default     = "n2-standard-2"
}

variable "boot_image" {
  description = "Boot disk image"
  type        = string
  default     = "ubuntu-os-cloud/ubuntu-2204-lts"
}

variable "boot_disk_size_gb" {
  description = "Boot disk size in GB (Prometheus TSDB + Grafana data live here)"
  type        = number
  default     = 50
}

variable "state_bucket" {
  description = "GCS bucket used for Terraform state (must match scripts/gcp-bootstrap-tfstate.sh output)"
  type        = string
  default     = "tigerbettle-sandbox-tfstate"
}
