output "instance_names" {
  value = google_compute_instance.db[*].name
}

output "internal_ips" {
  value = [for i in google_compute_instance.db : i.network_interface[0].network_ip]
}

output "external_ips" {
  value = [for i in google_compute_instance.db : i.network_interface[0].access_config[0].nat_ip]
}

output "zones" {
  value = google_compute_instance.db[*].zone
}

output "service_account_email" {
  value = google_service_account.db_node.email
}
