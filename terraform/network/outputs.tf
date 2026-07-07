output "network_self_link" {
  value = google_compute_network.tb_perf.self_link
}

output "network_name" {
  value = google_compute_network.tb_perf.name
}

output "subnet_self_link" {
  value = google_compute_subnetwork.tb_perf.self_link
}

output "subnet_name" {
  value = google_compute_subnetwork.tb_perf.name
}

output "region" {
  value = var.region
}

output "artifacts_bucket_name" {
  value = google_storage_bucket.artifacts.name
}
