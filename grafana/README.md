# Grafana Provisioning

This directory contains Grafana dashboard and datasource provisioning files.

## Structure:

- `dashboards/` - Dashboard JSON files
- `datasources/` - Datasource configuration files

## Dashboards to create:

- `performance-overview.json` - Main performance dashboard with:
  - Test phase indicator (warmup/measurement)
  - Test mode indicator (max_throughput/fixed_rate)
  - Throughput (requests/sec)
  - Latency percentiles (p50, p95, p99, p999)
  - Error rate
  - Resource usage (CPU, memory, disk, network)
  - TigerBeetle batch sizes (if applicable)

## Datasources:

- Prometheus - configured to point to Prometheus instance
