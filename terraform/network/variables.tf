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

variable "operator_ip" {
  description = <<-EOT
    CIDR allowed to reach Grafana/Prometheus and SSH (via IAP) consoles,
    e.g. "203.0.113.4/32". Find your current IP with: curl -s ifconfig.me
  EOT
  type        = string
}

variable "subnet_cidr" {
  description = "CIDR range for the single tb-perf subnet"
  type        = string
  default     = "10.10.0.0/20"
}
