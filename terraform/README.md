# Terraform Infrastructure

Infrastructure as Code for the GCP cloud deployment (project `tigerbettle-sandbox`, region `europe-central2`).

Scope is intentionally limited to **networking, compute, storage, and baseline OS setup** (Docker/Rust/Local SSD mounting). Database replication configuration, TigerBeetle cluster formatting, and the observability stack's actual startup are handled separately by the coordinator over `gcloud compute ssh --tunnel-through-iap`, since those need to be reconfigurable per test run (see `PLAN.md` §3.2/§3.4).

## Structure

- `network/` - VPC, subnet, firewall rules (applies first - other modules read its state)
- `database-cluster/` - 3-node DB cluster (PostgreSQL or TigerBeetle, selected via `database_type`)
- `client-cluster/` - Client load-generator instances
- `monitoring/` - Single instance for OTel Collector + Prometheus + Grafana

Each module is a fully independent Terraform root (own state, own `terraform init`/`apply`). `database-cluster`, `client-cluster`, and `monitoring` read the `network` module's outputs via a `terraform_remote_state` data source, so `network` must be applied first, but the others can be applied/destroyed independently of each other.

## One-time setup

Before the first `terraform init` anywhere in this directory, bootstrap the GCS state bucket:

```bash
cd ..
./scripts/gcp-bootstrap-tfstate.sh
```

## Usage

```bash
# 1. Network (always first)
cd network
cp terraform.tfvars.example terraform.tfvars   # fill in your operator_ip
terraform init
terraform apply

# 2. Database cluster (pick one database_type per test session)
cd ../database-cluster
terraform init
terraform apply -var="database_type=tigerbeetle"
# ...or: terraform apply -var="database_type=postgresql"

# 3. Client cluster
cd ../client-cluster
terraform init
terraform apply -var="num_instances=5"

# 4. Monitoring stack instance
cd ../monitoring
terraform init
terraform apply
```

To tear everything down (recommended after each test session, to control cost):

```bash
cd monitoring && terraform destroy
cd ../client-cluster && terraform destroy
cd ../database-cluster && terraform destroy
cd ../network && terraform destroy
```

## Design notes

- **No Cloud NAT / Cloud Router.** All instances get external IPs directly; access is locked down via firewall rules (network tags) rather than network isolation. This is a deliberate simplification since the whole environment is ephemeral (provisioned per test session, destroyed after) - see `PLAN.md` §3.1.
- **Firewall rules use network tags**, not IP lists, so modules don't need to share instance IPs with each other - `network` defines rules once (`tb-perf-db`, `tb-perf-client`, `tb-perf-monitoring` tags), and every module just tags its instances accordingly.
- **State backend**: GCS bucket (`gcs` backend), which natively supports state locking - no separate lock table (e.g. DynamoDB) is required.
- **Remote access**: `gcloud compute ssh --tunnel-through-iap` (Identity-Aware Proxy) instead of direct SSH or AWS-SSM-style tooling. Requires:
  - Your account/user to have `roles/iap.tunnelResourceAccessor` on the project
  - The `tb-perf-allow-iap-ssh` firewall rule (already defined in `network/main.tf`), which only allows TCP/22 from Google's fixed IAP range (`35.235.240.0/20`)
  - No manual SSH key management - GCP OS Login provisions ephemeral keys tied to your IAM identity

## Machine types

| Module | Machine type | Notes |
|---|---|---|
| `database-cluster` | `n2-highmem-4` (4 vCPU, 32 GB RAM) + 1x 375 GB Local SSD (NVMe) | One per zone (`europe-central2-a/b/c`) |
| `client-cluster` | `n2-standard-2` (2 vCPU, 8 GB RAM) | General-purpose, sized for load generation |
| `monitoring` | `n2-standard-2` (2 vCPU, 8 GB RAM) | Runs OTel Collector + Prometheus + Grafana via Docker Compose |
