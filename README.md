# TigerBeetle vs PostgreSQL Performance Comparison

Performance comparison framework for TigerBeetle vs PostgreSQL on double-entry bookkeeping workloads.

## Project Structure

```
tb-perf/
├── coordinator/         # Test coordinator binary
├── client/              # Test client binary
├── common/              # Shared configuration and types
├── docker/              # Docker Compose for local testing
├── grafana/             # Dashboard and datasource provisioning
├── terraform/           # AWS infrastructure as code
├── scripts/             # Database setup and management scripts
├── config.*.toml        # Example configuration files
└── PLAN.md              # Detailed implementation plan
```

## Quick Start

### Prerequisites

- Rust 1.85+ (edition 2024)
- Docker + Docker Compose (for local testing)
- Terraform (for cloud deployments)
- AWS credentials (for cloud deployments)

### Build

```bash
cargo build --release
```

### Run Local Test

1. Start database and observability stack:
```bash
cd docker
docker-compose -f docker-compose.postgresql.yml up -d
```

2. Run coordinator:
```bash
./target/release/coordinator --config config.local-postgresql.toml
```

The coordinator will:
- Start the client
- Execute warmup and measurement phases
- Aggregate metrics
- Export results to `./results/`

### View Results

Access Grafana at http://localhost:3000 to view live metrics during the test.

## Configuration

The system uses a single TOML configuration file read by both coordinator and clients.

See example configurations:
- `config.local-postgresql.toml` - Local PostgreSQL test
- `config.local-tigerbeetle.toml` - Local TigerBeetle test
- `config.cloud-tigerbeetle-fixedrate.toml` - Cloud test example

### Configuration Sections

- `[workload]` - Test parameters (used by clients)
- `[database]` - Database type selection
- `[postgresql]` / `[tigerbeetle]` - Database-specific settings (used by clients)
- `[deployment]` - Local vs cloud configuration
- `[coordinator]` - Test orchestration settings (used by coordinator)
- `[monitoring]` - Observability stack configuration (used by coordinator)

## Architecture

### Test Coordinator (single instance)
- Orchestrates test execution
- Manages test runs and database resets
- Aggregates metrics from clients
- Exports results (JSON, CSV)

### Test Clients (one or more instances)
- Execute the workload (double-entry transfers)
- Collect local metrics (latency, throughput, errors)
- Export metrics via OpenTelemetry
- In local mode: 1 client instance
- In cloud mode: N client instances

## Test Modes

### max_throughput
- Closed-loop testing
- Finds maximum sustainable TPS
- Each worker continuously sends requests

### fixed_rate
- Open-loop testing
- Requests issued at constant rate
- Coordinated omission correction for accurate tail latencies
- Use to measure latency under various load levels

## Development Status

Phase 1 (Foundation) - **Complete**
- ✅ Project structure
- ✅ Configuration parsing with validation
- ✅ Coordinator skeleton
- ✅ Client skeleton
- ✅ Docker Compose setup (PostgreSQL, OTel Collector, Prometheus, Grafana)
- ✅ Grafana dashboards

Phase 2 (Local Implementation) - **PostgreSQL Complete, TigerBeetle TODO**
- ✅ PostgreSQL schema and stored procedures
- ✅ Client PostgreSQL workload implementation
- ✅ OpenTelemetry metrics collection and export
- ✅ Coordinator test orchestration
- ✅ Prometheus metric querying for results
- ✅ JSON results export
- ⏳ TigerBeetle workload implementation

Phase 3 (Cloud Infrastructure) - **TODO**
- Terraform modules
- AWS deployment automation
- Multi-client coordination

Phase 4 (Testing Scenarios) - **TODO**
- Configuration matrix testing
- Endurance testing
- Result analysis

## License

Apache2
