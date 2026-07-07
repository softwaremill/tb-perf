# Terraform Infrastructure

Infrastructure as Code for GCP cloud deployments.

## Structure:

- `modules/` - Reusable Terraform modules
- `network/` - VPC, subnet, firewall rules
- `database-cluster/` - Database cluster (PostgreSQL or TigerBeetle)
- `client-cluster/` - Client instances

## Modules to create:

### Network Module
- Single VPC with one subnet spanning all 3 zones used
- Firewall rules (database internal/client access, client egress-only, monitoring access, IAP ingress for SSH)
- No Cloud NAT / Cloud Router - instances use external IPs directly since infrastructure is ephemeral and access is locked down via firewall rules instead of network isolation

### Database Cluster Module
- 3x `n2-highmem-4` instances (4 vCPU, 32 GB RAM), one per zone, for 3-node clusters
- 1x 375 GB Local SSD per instance for database storage
- PostgreSQL or TigerBeetle configuration via startup-script
- Node exporter for metrics
- Dedicated service account per instance, scoped to Cloud Storage access only

### Client Cluster Module
- Configurable number of `n2-standard-2` instances
- Docker and Rust toolchain pre-installed via startup-script
- Client binary deployment via a Cloud Storage (GCS) bucket

## State backend

Terraform state is stored in a GCS bucket (`gcs` backend), which provides native state locking - no separate lock table (e.g. DynamoDB) is required.

Before the first `terraform init`, run `../scripts/gcp-bootstrap-tfstate.sh` once to create the state bucket (the `gcs` backend requires the bucket to already exist).

## Remote access

Remote command execution and debugging use `gcloud compute ssh --tunnel-through-iap` (Identity-Aware Proxy) rather than direct SSH. This requires:
- The invoking identity to have `roles/iap.tunnelResourceAccessor`
- A firewall rule allowing TCP/22 from Google's IAP range (`35.235.240.0/20`)
- No manual SSH key management - GCP OS Login provisions ephemeral keys tied to IAM identity

## Usage:

```bash
cd terraform/network
terraform init
terraform apply

cd ../database-cluster
terraform init
terraform apply -var="database_type=tigerbeetle"

cd ../client-cluster
terraform init
terraform apply -var="num_instances=5"
```
