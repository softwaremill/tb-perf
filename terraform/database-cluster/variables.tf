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
  description = "Zones to spread DB replicas across (one instance per zone, cycling if node_count > length(zones))"
  type        = list(string)
  default     = ["europe-central2-a", "europe-central2-b", "europe-central2-c"]
}

variable "database_type" {
  description = "Which database this cluster hosts: \"postgresql\" or \"tigerbeetle\""
  type        = string

  validation {
    condition     = contains(["postgresql", "tigerbeetle"], var.database_type)
    error_message = "database_type must be \"postgresql\" or \"tigerbeetle\"."
  }
}

variable "node_count" {
  description = "Number of DB replicas (symmetric 3-node cluster for both DB types, per PLAN.md §3.1)"
  type        = number
  default     = 3
}

variable "machine_type" {
  description = "GCE machine type for DB nodes"
  type        = string
  default     = "n2-highmem-4" # 4 vCPU, 32 GB RAM - matches AWS i4i.xlarge spec used in earlier design
}

variable "local_ssd_count" {
  description = "Number of 375 GB Local SSD (NVMe) disks to attach per DB node"
  type        = number
  default     = 1
}

variable "boot_image" {
  description = "Boot disk image (needs a modern kernel for TigerBeetle's io_uring usage)"
  type        = string
  default     = "ubuntu-os-cloud/ubuntu-2204-lts"
}

variable "boot_disk_size_gb" {
  description = "Boot disk size in GB (OS only - DB data lives on Local SSD)"
  type        = number
  default     = 50
}

variable "state_bucket" {
  description = "GCS bucket used for Terraform state (must match scripts/gcp-bootstrap-tfstate.sh output)"
  type        = string
  default     = "tigerbettle-sandbox-tfstate"
}
