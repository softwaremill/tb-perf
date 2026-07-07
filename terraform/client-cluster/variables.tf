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

variable "zones" {
  description = "Zones to spread client instances across (cycles if num_instances > length(zones))"
  type        = list(string)
  default     = ["europe-central2-a", "europe-central2-b", "europe-central2-c"]
}

variable "num_instances" {
  description = "Number of client instances (corresponds to config.toml's deployment.num_client_nodes)"
  type        = number
  default     = 3
}

variable "machine_type" {
  description = "GCE machine type for client nodes"
  type        = string
  default     = "n2-standard-2" # 2 vCPU, 8 GB RAM - general purpose, sized for load generation
}

variable "boot_image" {
  description = "Boot disk image"
  type        = string
  default     = "ubuntu-os-cloud/ubuntu-2204-lts"
}

variable "boot_disk_size_gb" {
  description = "Boot disk size in GB"
  type        = number
  default     = 30
}

variable "state_bucket" {
  description = "GCS bucket used for Terraform state (must match scripts/gcp-bootstrap-tfstate.sh output)"
  type        = string
  default     = "tigerbettle-sandbox-tfstate"
}
