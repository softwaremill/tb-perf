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

## Running Tests

> **Note:** TigerBeetle uses port 3000 for its API, so when running TigerBeetle tests, Grafana is available on port 3001 instead of 3000.

### Sanity Check (Quick 30-second Test)

Use sanity-check configurations to verify your setup works before running longer tests.

**PostgreSQL Sanity Check:**
```bash
# Start the stack
docker compose -f docker/docker-compose.postgresql.yml up -d

# Wait for PostgreSQL to be ready (about 10 seconds)
# Then run the sanity check
cargo run --release --bin coordinator -- -c config.sanity-postgresql.toml

# View results in Grafana at http://localhost:3000
```

**TigerBeetle Sanity Check:**
```bash
# Start the stack
docker compose -f docker/docker-compose.tigerbeetle.yml up -d

# Wait for TigerBeetle to be ready (about 10 seconds)
# Then run the sanity check
cargo run --release --bin coordinator -- -c config.sanity-tigerbeetle.toml

# View results in Grafana at http://localhost:3001
# (TigerBeetle uses port 3000, so Grafana is on 3001)
```

### Proper Local Test (5-minute Measurement, 3 Runs)

**PostgreSQL Full Test:**
```bash
# Start infrastructure
docker compose -f docker/docker-compose.postgresql.yml up -d

# Run the full test suite (takes ~25 minutes: 3 runs x (2min warmup + 5min test))
cargo run --release --bin coordinator -- -c config.local-postgresql.toml

# Results are exported to ./results/ as JSON
```

**TigerBeetle Full Test:**
```bash
# Start infrastructure
docker compose -f docker/docker-compose.tigerbeetle.yml up -d

# Run the full test suite
cargo run --release --bin coordinator -- -c config.local-tigerbeetle.toml

# Results are exported to ./results/ as JSON
```

### Cleanup

```bash
# Stop PostgreSQL stack
docker compose -f docker/docker-compose.postgresql.yml down -v

# Stop TigerBeetle stack
docker compose -f docker/docker-compose.tigerbeetle.yml down -v
```

### Keep Grafana Running After Test

Add `--keep-running` flag to keep the infrastructure running after the test:

```bash
cargo run --release --bin coordinator -- -c config.local-postgresql.toml --keep-running
```

Or set `keep_grafana_running = true` in the configuration file.

## Configuration

The system uses a single TOML configuration file read by both coordinator and clients.

### Available Configurations

| File | Database | Mode | Duration | Runs | Purpose |
|------|----------|------|----------|------|---------|
| `config.sanity-postgresql.toml` | PostgreSQL | max_throughput | 10s + 5s warmup | 1 | Quick verification |
| `config.sanity-tigerbeetle.toml` | TigerBeetle | max_throughput | 10s + 5s warmup | 1 | Quick verification |
| `config.local-postgresql.toml` | PostgreSQL | max_throughput | 5min + 2min warmup | 3 | Proper local test |
| `config.local-tigerbeetle.toml` | TigerBeetle | max_throughput | 5min + 2min warmup | 3 | Proper local test |
| `config.cloud-tigerbeetle-fixedrate.toml` | TigerBeetle | fixed_rate | 5min + 2min warmup | 3 | Cloud example |

### Configuration Sections

```toml
[workload]
test_mode = "max_throughput"  # or "fixed_rate"
concurrency = 10              # Workers for max_throughput mode
# target_rate = 5000          # Requests/sec for fixed_rate mode
# max_concurrency = 1000      # Max in-flight for fixed_rate mode
num_accounts = 100000
zipfian_exponent = 1.0        # Account access distribution (1.0 = moderate skew)
initial_balance = 1000000
min_transfer_amount = 1
max_transfer_amount = 1000
warmup_duration_secs = 120    # Warmup before measurement
test_duration_secs = 300      # Measurement duration

[database]
type = "postgresql"  # or "tigerbeetle"

[postgresql]
isolation_level = "read_committed"  # or "repeatable_read", "serializable"
connection_pool_size = 20
connection_pool_min_idle = 20
pool_recycling_method = "verified"
auto_vacuum = false

[tigerbeetle]
cluster_addresses = ["3000"]        # Host:port for each replica
measure_batch_sizes = true

[deployment]
type = "local"      # or "cloud"
num_db_nodes = 1
measure_network_latency = false

[coordinator]
test_runs = 3                       # Number of test iterations
max_variance_threshold = 0.10       # Max allowed variance between runs
max_error_rate = 0.05               # Max allowed error rate
metrics_export_path = "./results"
keep_grafana_running = false

[monitoring]
grafana_port = 3000   # 3001 for TigerBeetle (3000 is used by TB)
prometheus_port = 9090
otel_collector_port = 4317
```

## Test Modes

### max_throughput

Closed-loop testing for finding maximum sustainable TPS.

- Each worker continuously sends requests as fast as possible
- Total throughput = sum of all worker throughput
- Best for: Capacity planning, finding bottlenecks

```toml
[workload]
test_mode = "max_throughput"
concurrency = 10  # Number of concurrent workers
```

### fixed_rate

Open-loop testing for accurate latency measurement.

- Requests issued at a constant rate regardless of response time
- Uses coordinated omission correction for accurate tail latencies
- Best for: SLA validation, latency analysis under known load

```toml
[workload]
test_mode = "fixed_rate"
target_rate = 5000        # Requests per second
max_concurrency = 1000    # Max in-flight requests (drops if exceeded)
```

## Observability

### Grafana Dashboards

Access Grafana during or after tests:
- PostgreSQL tests: http://localhost:3000
- TigerBeetle tests: http://localhost:3001

The dashboard shows:
- Test phase (warmup/measurement)
- Throughput (transfers/second)
- Success vs rejection rates
- Latency percentiles (p50, p95, p99, p99.9)
- Error rate

### Prometheus Metrics

Raw metrics are available at http://localhost:9090

Key metrics:
- `workload_completed_total` - Successful transfers
- `workload_rejected_total` - Rejected transfers (insufficient balance)
- `workload_failed_total` - Failed transfers (errors)
- `workload_latency_us` - Latency histogram in microseconds

## Architecture

### Test Coordinator (single instance)

- Orchestrates test execution
- Starts/stops Docker infrastructure
- Initializes database with accounts
- Spawns client binary as subprocess
- Collects metrics from Prometheus after test
- Exports results to JSON

### Test Client (one or more instances)

- Executes the workload (double-entry transfers)
- Generates Zipfian-distributed account selection
- Records latency with coordinated omission correction (fixed_rate mode)
- Exports metrics via OpenTelemetry to collector

### Workload

The workload simulates a financial ledger:

1. Select two random accounts using Zipfian distribution
2. Generate a random transfer amount
3. Execute the transfer (debit one account, credit another)
4. Record the result (success, rejected due to insufficient balance, or error)

For PostgreSQL, this uses a `transfer()` function that locks accounts with `SELECT ... FOR UPDATE` (ordered by account ID to prevent deadlocks) to ensure consistency.

For TigerBeetle, this uses the native transfer API with `DEBITS_MUST_NOT_EXCEED_CREDITS` flags.

## Development Status

Phase 1 (Foundation) - **Complete**
- Project structure and configuration parsing
- Docker Compose infrastructure
- Grafana dashboards

Phase 2 (Local Implementation) - **Complete**
- PostgreSQL workload implementation
- TigerBeetle workload implementation
- OpenTelemetry metrics collection
- Coordinator test orchestration
- JSON results export

Phase 3 (Cloud Infrastructure) - **TODO**
- Terraform modules for AWS deployment
- Multi-client coordination
- Result aggregation across clients

Phase 4 (Testing Scenarios) - **TODO**
- Configuration matrix testing
- Endurance testing
- Automated result analysis

## Troubleshooting

### "Client binary not found"

Build the project first:
```bash
cargo build --release
```

### "Failed to connect to TigerBeetle/PostgreSQL"

Ensure Docker containers are running:
```bash
docker compose -f docker/docker-compose.{postgresql,tigerbeetle}.yml ps
```

### Metrics not showing in Grafana

Wait 15-20 seconds after the test starts. OTel Collector flushes every 5 seconds, and Prometheus scrapes every 5 seconds.

### Balance verification failed

This indicates a correctness issue - the total balance across all accounts changed during the test. This should never happen with properly implemented double-entry accounting.

## License

Apache2
