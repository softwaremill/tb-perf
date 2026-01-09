# Terraform Infrastructure

Infrastructure as Code for AWS cloud deployments.

## Structure:

- `modules/` - Reusable Terraform modules
- `network/` - VPC, subnets, security groups
- `database-cluster/` - Database cluster (PostgreSQL or TigerBeetle)
- `client-cluster/` - Client instances

## Modules to create:

### Network Module
- VPC with public/private subnets across 3 AZs
- Internet Gateway
- NAT Gateways
- Security Groups

### Database Cluster Module
- 3x i4i.xlarge instances (for 3-node clusters)
- NVMe instance storage setup
- PostgreSQL or TigerBeetle configuration
- Node exporter for metrics

### Client Cluster Module
- Configurable number of c5.large instances
- Docker and Rust toolchain pre-installed
- Client binary deployment

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
