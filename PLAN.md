# TigerBeetle vs PostgreSQL Performance Comparison - Implementation Plan

## Project Overview
Build a comprehensive performance comparison framework for TigerBeetle vs PostgreSQL on double-entry bookkeeping workloads. Support both local single-node testing and cloud-based 3-node replicated cluster testing with full observability.

## Quick Summary

- Rust-based benchmarking client for TigerBeetle vs PostgreSQL
- Double-entry bookkeeping workload with Zipfian distribution
- Local (Docker) and cloud (AWS 3-node clusters) deployment modes
- Automated test execution: warmup → ramp-up → measurement phases
- Multiple runs with automatic statistical aggregation (mean, stddev, CV, confidence intervals)
- Full observability: OpenTelemetry + Grafana + JSON/CSV exports
- Zero manual intervention during test execution

## Technology Stack

### Core Technologies
- **Client Implementation**: Rust (async/tokio)
- **TigerBeetle Client**: Official TigerBeetle Rust client
- **PostgreSQL Client**: `tokio-postgres` (async PostgreSQL client)
- **Containerization**: Docker + Docker Compose
- **Cloud Infrastructure**: AWS (EC2, EBS)
- **Infrastructure as Code**: Terraform
- **Metrics Collection**: OpenTelemetry (OTLP exporter)
- **Metrics Storage**: Prometheus (as OpenTelemetry backend)
- **System Metrics**: Node Exporter
- **Visualization**: Grafana
- **Load Distribution**: Zipfian distribution via `rand` + `rand_distr` crates

### Additional Rust Dependencies
- `tokio` - async runtime
- `clap` - CLI argument parsing
- `serde` / `serde_json` - configuration management
- `anyhow` / `thiserror` - error handling
- `tracing` / `tracing-subscriber` - structured logging
- `opentelemetry` / `opentelemetry-otlp` - metrics and observability
- `rand` / `rand_distr` - random number generation with Zipfian distribution
- `hdrhistogram` - latency recording with coordinated omission correction

### PostgreSQL Strategy
- **Isolation Levels**: Test at multiple levels (READ COMMITTED, REPEATABLE READ, SERIALIZABLE)
  - Note: REPEATABLE READ in PostgreSQL is implemented as Snapshot Isolation
- **Implementation**: PL/pgSQL stored procedure for transfer logic
- **Locking Strategy**: Pessimistic locking (SELECT FOR UPDATE)
- **Replication**: Synchronous 3-node cluster with quorum-based replication
  - Configure `synchronous_standby_names = 'ANY 2 (*)'` for 2-of-3 quorum
- **Durability Levels**: Configure `synchronous_commit` (on, off, local, remote_write, remote_apply)

## Phase 1: Project Structure & Foundation

### 1.1 Project Initialization

Initialize a Rust workspace with the following structure:
- Workspace root with main Cargo.toml
- Client binary crate for benchmarking workload
- Docker directory for local testing setup (docker-compose, Dockerfiles)
- Grafana provisioning directory for dashboards and datasources
- Terraform directory with modules for AWS infrastructure (network, database-cluster, client-cluster)
- Scripts directory for test orchestration and database setup
- Documentation directory for testing guides and concurrency analysis

### 1.2 Configuration Options

**Client Configuration** (workload behavior):
- `num_accounts` - Total number of accounts (default: 100,000)
- `concurrency` - Concurrent workers per client node (default: 10, based on 2x vCPU count for c5.large)
- `zipfian_exponent` - Account selection skew (0 = uniform, ~1.5 = high skew)
- `initial_balance` - Starting balance per account (default: 1,000,000)
- `min_transfer_amount` - Minimum transfer (default: 1)
- `max_transfer_amount` - Maximum transfer (default: 1,000)
- `think_time_ms` - Delay between requests per worker (default: 0, for max throughput testing)
- `warmup_duration_secs` - Warmup period before metrics collection (default: 120)
- `rampup_duration_secs` - Linear ramp-up to target concurrency (default: 60)
- `test_duration_secs` - Test duration excluding warmup/rampup (default: 300)
- `database` - TigerBeetle or PostgreSQL
- `test_runs` - Number of test runs per configuration (default: 3)
- `max_variance_threshold` - Maximum acceptable variance between runs (default: 0.10 = 10%)
- `max_error_rate` - Maximum error rate for valid test (default: 0.05 = 5%)

**PostgreSQL-Specific Configuration**:
- `isolation_level` - READ_COMMITTED | REPEATABLE_READ | SERIALIZABLE
- `synchronous_commit` - off | local | remote_write | remote_apply | on
- `connection_pool_size` - Connection pool size (default: 20, based on 2x vCPU + spindles formula)
- `connection_pool_min_idle` - Minimum idle connections (default: same as max, for pre-warming)
- `pool_recycling_method` - Fast | Verified (default: Verified for benchmark consistency)
- `auto_vacuum` - Enable/disable auto-vacuum during tests (default: disabled for consistent results)

**TigerBeetle-Specific Configuration**:
- `batch_size` - Explicit batch size for transfers (default: auto, max: 8189)
- `use_single_batcher` - Use single-batcher architecture to maximize batch efficiency (default: false)
- `replication_quorum` - Replication quorum for 3-node cluster (default: 2)
- `measure_batch_sizes` - Record actual batch sizes during test (default: true)

**Cloud Infrastructure Configuration**:
- `num_db_nodes` - Database cluster size (1 for local, 3 for cloud)
- `num_client_nodes` - Number of client instances (cloud only)
- `aws_region` - AWS region for deployment
- `db_instance_type` - EC2 instance type for database (default: i4i.xlarge with NVMe)
- `client_instance_type` - EC2 instance type for clients (default: c5.large)
- `measure_network_latency` - Measure inter-node network latency (default: true)

**Test Coordinator Configuration**:
- `deployment` - Local or Cloud
- `metrics_export_path` - Path for JSON metrics export
- `grafana_port` - Grafana dashboard port (default: 3000)
- `keep_grafana_running` - Keep Grafana running after test completion

## Phase 2: Single-Node Local Implementation

### 2.1 PostgreSQL Setup

- Schema with accounts and transfers tables
- Stored procedure for transfer logic with pessimistic locking (SELECT FOR UPDATE)
- Accounts locked in consistent order to prevent deadlocks
- Balance check before transfer

### 2.2 TigerBeetle Setup

- Use official TigerBeetle Docker image
- Single replica for local testing
- Configure data volume for persistence
- Request batching handled by Rust TigerBeetle client library
- Batching Strategy: Default multi-worker approach fragments batches across event loops
  - Each worker's batcher only sees a fraction of concurrent load
  - Record actual batch sizes during tests to understand batching efficiency
  - Batch size directly impacts throughput and must be documented in results

### 2.3 Rust Client Implementation

**Core workload logic**:
- Zipfian distribution for account selection (hot accounts get more traffic)
- Random transfer amounts within configured range (default: 1-1,000)
- Initial account balance: 1,000,000 (minimizes insufficient balance scenarios)
- **Think time**: Zero by default (continuous load for max throughput testing)
- **Test Phases**:
  1. **Warmup Phase** (default: 120s): Run workload to fill buffer pools, caches, complete JIT compilation
  2. **Ramp-up Phase** (default: 60s): Linear increase from 0 to target concurrency
  3. **Measurement Phase** (default: 300s): Stable load at target concurrency, metrics collected
- No retry logic for insufficient balance (count as successful rejection, not an error)
- Retry logic for serialization failures (with exponential backoff)
- **Latency Recording**: Use HdrHistogram
  - For max-throughput workload (zero think time), coordinated omission correction not applicable
  - Record actual observed latencies with `record(value)`
  - If adding configurable think time, use `record_correct(value, think_time_ms)` for correction
- OpenTelemetry metrics collection:
  - Successful transfers (completed)
  - Rejected transfers (insufficient balance)
  - Failed transfers (errors, serialization failures after max retries)
  - Latency percentiles (p50, p95, p99, p999) for successful+rejected
  - Throughput (total requests/sec)
  - Error rate (%) - test invalid if > 5%
  - TigerBeetle-specific: actual batch sizes used
  - Client resource utilization (CPU, memory) to detect client saturation

**TigerBeetle implementation**:
- Use official Rust client (batching handled by the client library)
- Handle TigerBeetle-specific errors

**PostgreSQL implementation**:
- Connection pooling (using `deadpool-postgres` with `RecyclingMethod::Verified`)
  - Pool min_idle = max_size for pre-warming (avoids connection overhead during measurement)
- Call stored procedure with pessimistic locking
- Handle serialization failures and retry
- Configurable isolation level per connection
- Run VACUUM before each test (with auto-vacuum disabled during tests)
- Connection pool size: 2x vCPU count as starting point (tune based on workload)

### 2.4 Observability Stack

**OpenTelemetry**:
- Client exports metrics via OTLP protocol
- OpenTelemetry Collector receives and processes metrics
- Collector exports to Prometheus for storage
- Client workload metrics: throughput, latency, success/rejection/failure counts, phase labels
- System resource metrics via node-exporter on DB nodes

**Prometheus**:
- Acts as time-series storage backend for OpenTelemetry metrics
- Scrapes node-exporter from DB servers (CPU, memory, disk I/O, network I/O)
- Provides query interface for Grafana and result aggregation

**Grafana**:
- Pre-configured dashboard with panels:
  - **Test Phase Indicator**: State timeline panel using `phase` label to show Warmup / Ramp-up / Measurement
  - Throughput (total requests/sec) - time series
  - Successful transfers/sec vs Rejected transfers/sec - stacked time series
  - Error rate (%) - time series with alert annotation at 5% threshold
  - Latency percentiles (p50, p95, p99, p999) - time series
  - CPU usage (all nodes) - time series with separate series per node
  - Memory usage (all nodes) - time series
  - Network I/O (all nodes) - time series (sent/received)
  - Network latency between DB nodes (cloud only) - time series
  - Disk I/O (DB nodes) - time series (read/write)
  - Active connections/clients - gauge
  - TigerBeetle batch sizes (histogram) - time series
  - Client CPU saturation indicator - gauge

**Metrics Export**:
- JSON export at test completion with summary metrics (measurement phase only):
  - Test configuration (all parameters)
  - Test run number (1-N) and timestamp
  - Total duration (warmup, ramp-up, measurement phases)
  - Throughput (requests/sec, successful transfers/sec, rejected/sec)
  - Latency percentiles (p50, p95, p99, p999) from HdrHistogram
  - Error count and rate
  - Resource usage averages (CPU, memory, disk I/O, network I/O)
  - Network latency between nodes (mean, p95, p99)
  - TigerBeetle-specific: mean/median batch size
  - Client saturation indicator (max CPU utilization)
  - Statistical metadata: standard deviation across runs, coefficient of variation

### 2.5 Automated Test Orchestration

**Test Coordinator Responsibilities**:
- Fully automated test execution from start to finish
- Phase-aware metrics collection with automatic separation
- Automatic result aggregation and statistical analysis
- No manual intervention required between test phases or runs

**Local Test Script Flow**:

1. **Infrastructure Setup**:
   - Start docker-compose (PostgreSQL/TigerBeetle + OTel Collector + Prometheus + Grafana)
   - Wait for services to be ready
   - Initialize database (create accounts with initial balance of 1,000,000)
   - For PostgreSQL: Run VACUUM, disable auto-vacuum

2. **Automated Test Loop** (N runs, default: 3):

   For each run (run_id: 1..N):

   a. **Phase 1: Warmup** (default 120s)
      - Client emits phase marker: `test.phase="warmup"` label via OpenTelemetry
      - Workload executes at target concurrency
      - Metrics sent to OTel Collector, stored in Prometheus with `phase="warmup"` label
      - Filtered out during result aggregation

   b. **Phase 2: Ramp-up** (default 60s)
      - Client emits phase marker: `test.phase="rampup"` via OpenTelemetry
      - Concurrency increases linearly from 0 to target (e.g., 0→10 over 60s)
      - Metrics tagged with `phase="rampup"`, filtered out during result aggregation

   c. **Phase 3: Measurement** (default 300s)
      - Client emits phase marker: `test.phase="measurement"` via OpenTelemetry
      - Workload runs at stable target concurrency
      - **Metrics tagged as `phase="measurement"`** (ONLY these are used for results)
      - HdrHistogram records latencies with coordinated omission correction
      - All metrics sent via OTLP, timestamped and associated with run_id

   d. **Per-Run Export**:
      - Query Prometheus for metrics with `phase="measurement"` AND `run_id=N`
      - Export to `results/{config_name}/run_{N}_{timestamp}.json` with all metrics from measurement phase

   e. **Database Reset**:
      - For PostgreSQL: DROP DATABASE + recreate schema + repopulate accounts (ensures clean buffer pool state)
      - For TigerBeetle: Truncate transfers, reset account balances
      - For PostgreSQL: Run VACUUM ANALYZE after repopulation
      - Wait 30s for system to stabilize (WAL flush, buffer cache, network draining)

3. **Automated Result Aggregation**:

   After all runs complete, automatically:

   a. Load all per-run JSON files

   b. Calculate aggregate statistics:
      - Mean across runs for each metric
      - Standard deviation
      - Coefficient of variation (CV = stddev/mean)
      - Confidence intervals (95% CI)

   c. Validate statistical consistency:
      - Flag runs where throughput CV > 10%
      - Flag runs where p99 latency CV > 15%
      - Flag runs where error rate > 5% (invalid test)
      - Warn if variance exceeds thresholds

   d. Export aggregated results to `results/{config_name}/aggregate_{timestamp}.json` with mean, stddev, CV, confidence intervals, and validation warnings

   e. Generate comparison CSV: `results/comparison_{timestamp}.csv`

4. **Post-Test Actions**:
   - Optionally keep Grafana running for manual analysis
   - Cleanup containers OR preserve infrastructure for next test

**Key Design Principle**: Client emits phase labels (`warmup`, `rampup`, `measurement`) with all metrics. Only `phase="measurement"` metrics exported to final results. Zero manual intervention required.

### 2.6 Baseline Testing Strategy

Before testing replicated clusters, establish single-node baselines:
1. **Single-node PostgreSQL** (no replication, synchronous_commit=local)
2. **Single-node TigerBeetle** (no replication)
3. Document baseline throughput and latency
4. Express replicated cluster results as degradation percentage from baseline

### 2.7 Local Testing Documentation

Create detailed local testing documentation with:
- Prerequisites (Docker, Rust toolchain)
- How to run tests step-by-step
- Configuration options explained
- Understanding test phases (warmup, ramp-up, measurement)
- How to access Grafana dashboards
- Interpreting metrics and identifying client saturation
- Statistical validity requirements (multiple runs, variance thresholds)
- Troubleshooting common issues

## Phase 3: Cloud Infrastructure (AWS)

### 3.1 Terraform Infrastructure

**Network Module**:
- VPC with public/private subnets across 3 AZs
- Internet Gateway
- NAT Gateways (for private subnets)
- Security Groups:
  - Database cluster: internal communication + client access
  - Client cluster: outbound only
  - Monitoring: Prometheus/Grafana access

**Database Cluster Module**:
- 3 EC2 instances (i4i.xlarge: 4 vCPU, 32 GB RAM, 1x 468 GB NVMe SSD)
  - NVMe instance storage provides ~250k IOPS, ~4 GB/s throughput (vs gp3: 3k-16k IOPS)
  - Critical for database workloads with high write throughput
  - Data ephemeral (acceptable for benchmarking, not production)
- For PostgreSQL: Set up synchronous 3-node cluster replication (matching TigerBeetle's consistency model)
  - Primary + 2 synchronous standbys
  - Configure `synchronous_standby_names = 'ANY 2 (*)'` for 2-of-3 quorum
  - Maximize performance while maintaining replication/durability guarantees
  - Tune: shared_buffers, work_mem, fsync settings, WAL configuration, checkpoint settings
- For TigerBeetle: 3-node cluster with synchronous replication
- Install node-exporter for metrics
- Automated setup scripts via user-data

**Client Cluster Module**:
- Configurable number of EC2 instances via `num_client_nodes` variable (c5.large: 2 vCPU, 4 GB RAM) - compute-optimized
- Docker pre-installed
- Rust toolchain pre-installed
- Client binary deployment via S3 or built on instance

### 3.2 Database Cluster Setup

**PostgreSQL**:
- Automated setup script to configure:
  - Primary node with replication slots
  - Standby nodes with replication from primary
  - Synchronous replication settings
  - `synchronous_commit` level (configurable: off, local, remote_write, remote_apply, on)

**TigerBeetle**:
- Initialize 3-node cluster with replication factor 2 (quorum of 2/3)
- Configure cluster addresses and node IDs
- All 3 nodes handle client requests (load distributed)
- Start replicas in cluster mode
- Request batching handled by Rust client library
- Measure actual batch sizes to understand batching efficiency with distributed load

### 3.3 Network Latency Measurement

Before running tests, measure network latency between database nodes:
- Use `ping` or `iperf3` to measure RTT between all node pairs
- Record mean, p95, p99 latencies
- Export as part of test metadata
- Critical for understanding synchronous replication overhead

### 3.4 Automated Cloud Test Orchestration

Cloud tests run fully automatically from laptop with zero manual intervention during execution.

**Orchestration Flow**:

1. **Infrastructure Provisioning** (from laptop):
   - Run `terraform apply` to provision all AWS resources
   - Wait for all instances to be ready
   - Automated health checks for all services

2. **Pre-Test Setup**:
   - **Network latency measurement**: Automatically measure RTT between all DB node pairs
   - Export network metrics to `results/{config_name}/network_latency.json`
   - Deploy client binary to all client nodes (parallel deployment)
   - Initialize database cluster (create accounts with initial balance 1,000,000)
   - For PostgreSQL: Run VACUUM, disable auto-vacuum
   - Start OpenTelemetry Collector + Prometheus + Grafana on dedicated monitoring instance

3. **Automated Multi-Run Test Loop** (N runs, default: 3):

   For each run (run_id: 1..N):

   a. **Client Coordination**:
      - Barrier synchronization: HTTP endpoint or file-based coordination
      - All client nodes wait at barrier until all are ready
      - Coordinator signals: "START RUN {N}"
      - All clients begin simultaneously (clock sync via NTP)

   b. **Phase 1: Warmup** (default 120s):
      - All clients emit: `test.phase="warmup"`, `run_id=N` via OpenTelemetry
      - Full workload at target concurrency on all client nodes
      - Metrics sent to OTel Collector, tagged for exclusion from results

   c. **Phase 2: Ramp-up** (default 60s):
      - All clients emit: `test.phase="rampup"`, `run_id=N` via OpenTelemetry
      - Coordinated linear ramp from 0 to target concurrency
      - Each client ramps independently (0→10 over 60s per client)
      - Metrics sent to OTel Collector, excluded from results

   d. **Phase 3: Measurement** (default 300s):
      - All clients emit: `test.phase="measurement"`, `run_id=N` via OpenTelemetry
      - Stable workload at target concurrency
      - **All metrics from this phase used for final results**
      - Each client records local metrics in HdrHistogram

   e. **Per-Run Data Collection**:
      - Each client exports local metrics to coordinator
      - Coordinator aggregates across all client nodes:
        - Sum throughputs from all clients
        - Merge HdrHistograms for global latency percentiles
        - Max CPU saturation across all clients
      - Query Prometheus for DB-side metrics (CPU, disk I/O, network)
      - Filter by: `phase="measurement"` AND `run_id=N`
      - Export to `results/{config_name}/run_{N}_{timestamp}.json`

   f. **Database Reset Between Runs**:
      - For PostgreSQL: DROP DATABASE on primary + recreate schema + repopulate accounts (propagates to standbys, ensures clean buffer pool)
      - For TigerBeetle: Truncate transfers on all nodes, reset account balances
      - For PostgreSQL: Run VACUUM ANALYZE after repopulation
      - Wait 60s for cluster stabilization (WAL flush, buffer cache, network draining)

4. **Automated Result Aggregation** (same as local):

   After all runs complete:

   a. Load all per-run JSON files

   b. Calculate aggregate statistics for each metric:
      - Mean, standard deviation, coefficient of variation
      - 95% confidence intervals
      - Per-run values for debugging

   c. Statistical validation:
      - Flag throughput CV > 10%
      - Flag p99 latency CV > 15%
      - Flag error rate > 5% (invalid test)
      - Check for client saturation (CPU > 80%)
      - Check for network issues (high inter-node latency variance)
      - Verify clock sync: acceptable NTP drift < 1ms

   d. Export aggregated results:
      - `results/{config_name}/aggregate_{timestamp}.json` with statistics and validation
      - `results/comparison_{timestamp}.csv` for multi-config comparison

   e. Generate summary report with throughput, latency, validation status

5. **Download Results to Laptop**:
   - All JSON files automatically downloaded via rsync/scp
   - Stored locally in `./results/{config_name}/`
   - Ready for further analysis or visualization

6. **Post-Test Cleanup Options**:
   - **Option A**: Keep infrastructure, keep Grafana running for analysis
   - **Option B**: Keep infrastructure, cleanup data only (run data cleanup script)
   - **Option C**: Full teardown (`terraform destroy`)

**Multi-Configuration Batches**:

Script automatically runs multiple configs sequentially with data cleanup between each. Single command kicks off entire test suite, returns with all results aggregated and downloaded.

### 3.5 Data Cleanup Script

Create data cleanup script that:
- Connects to database cluster
- For PostgreSQL: DROP DATABASE + recreate schema + repopulate accounts
- For TigerBeetle: Truncate transfers table, reset account balances to initial values
- For PostgreSQL: Run VACUUM ANALYZE after repopulation
- Does NOT destroy the cloud infrastructure
- Allows multiple tests to run on provisioned infrastructure without full reprovisioning

### 3.6 Client Coordination

Simple barrier synchronization mechanism for coordinating multiple client nodes:
- File-based or HTTP endpoint synchronization
- All clients start workload simultaneously
- Each client exports metrics via OpenTelemetry to central collector

## Phase 4: Configuration & Testing Scenarios

### 4.1 Configuration Matrix

**PostgreSQL configurations to test**:
- Isolation levels: READ COMMITTED, REPEATABLE READ, SERIALIZABLE
- Synchronous commit levels: off, local, remote_write, remote_apply, on

**Workload scenarios**:
- Low contention: 1M accounts, low concurrency
- High contention: 10K accounts, high concurrency
- Hot accounts: Zipfian with high skew (exponent ~1.5)
- Uniform: Zipfian with low skew (exponent ~0)

### 4.2 Configuration File Format

Example:
```toml
[workload]
num_accounts = 100000
concurrency = 10  # 2x vCPU count for c5.large (2 vCPU)
zipfian_exponent = 1.0
initial_balance = 1000000
min_transfer_amount = 1
max_transfer_amount = 1000
warmup_duration_secs = 120
rampup_duration_secs = 60
test_duration_secs = 300  # measurement phase only
test_runs = 3  # number of runs per configuration
max_variance_threshold = 0.10  # 10% max variance between runs

[database]
type = "postgresql"  # or "tigerbeetle"

[postgresql]
isolation_level = "read_committed"
synchronous_commit = "remote_apply"
connection_pool_size = 20  # 2x vCPU for c5.xlarge (4 vCPU) + spindles
pool_recycling_method = "verified"
auto_vacuum = false

[tigerbeetle]
batch_size = "auto"  # or explicit size up to 8189
use_single_batcher = false
replication_quorum = 2
measure_batch_sizes = true

[deployment]
type = "cloud"  # or "local"
num_db_nodes = 3
num_client_nodes = 5
measure_network_latency = true

[monitoring]
metrics_port = 9090
grafana_port = 3000
```

### 4.3 Additional Testing Scenarios

Consider adding these test scenarios:

**Endurance Testing**:
- Run at least one 2-4 hour test per configuration (industry standard for stability)
- Detect memory leaks, connection pool degradation
- Observe PostgreSQL checkpoint cycles (default 5min) - need 24+ cycles for pattern analysis
- Monitor long-term WAL flush impacts

**Failure and Recovery Testing** (optional):
- Performance during leader failover
- Recovery time after node failure
- Impact of lagging replicas

**Connection Pool Sizing Tests**:
- Test multiple pool sizes (10, 20, 50, 100, 200)
- Identify optimal pool size for workload
- Measure connection wait time at different pool sizes

### 4.4 Cloud Testing Documentation

Create detailed cloud testing documentation with:
- Prerequisites (Terraform, AWS credentials, SSH keys)
- How to provision infrastructure
- Network latency measurement procedures
- How to run tests from laptop remotely
- Understanding test phases and metrics
- How to access Grafana dashboards (port forwarding or public access)
- Identifying client saturation vs database saturation
- How to run multiple tests on same infrastructure (using data cleanup script)
- Statistical validity: interpreting variance and confidence intervals
- How to fully teardown infrastructure
- Troubleshooting common issues (high variance, client saturation, network issues)
- AWS cost estimation

## Phase 5: Documentation

### 5.1 Concurrency Phenomena Documentation

Create concurrency phenomena documentation:
- **Dirty Reads**: Reading uncommitted data from another transaction
- **Non-Repeatable Reads**: Same query returns different results within transaction
- **Phantom Reads**: New rows appear in range queries within transaction
- **Lost Updates**: Two transactions read same value, both update, one overwrites the other
- **Write Skew**: Two transactions read overlapping data and make disjoint updates that violate constraints
- **Serialization Anomalies**: Results that couldn't occur if transactions executed serially

For each isolation level (READ COMMITTED, REPEATABLE READ, SERIALIZABLE):
- What phenomena are prevented
- What phenomena can still occur
- Performance implications
- Recommended use cases
- Note: REPEATABLE READ in PostgreSQL is Snapshot Isolation

## Summary of Performance Testing Best Practices Applied

This plan incorporates the following critical performance testing best practices:

### Statistical Validity
- **Multiple runs**: 3-5 runs per configuration (default: 3)
- **Variance thresholds**: Flag results with >10% variance for investigation
- **Confidence intervals**: Report mean, standard deviation, and coefficient of variation
- **Outlier handling**: Flag and investigate high-variance runs before accepting results

### Test Execution Strategy
- **Warmup phase**: 120s to fill buffer pools, caches, and complete JIT compilation
- **Ramp-up phase**: 60s linear increase to target concurrency (avoid cold-start artifacts)
- **Measurement phase**: 300s stable load with metrics collection
- **Baseline establishment**: Single-node tests before replicated cluster tests
- **Result normalization**: Express replicated results as degradation % from baseline

### Latency Measurement
- **HdrHistogram**: Use `record()` for max-throughput tests (coordinated omission correction requires known arrival rate)
- **Phase separation**: Only collect metrics during measurement phase
- **Client saturation detection**: Monitor client CPU to ensure database limits are measured, not client limits
- **Error rate validation**: Tests with >5% error rate flagged as invalid
- **Note**: With zero think time, latencies reflect closed-loop system behavior, not open-loop arrival patterns

### TigerBeetle-Specific
- **Batch fragmentation awareness**: Multiple workers fragment batches across event loops
- **Batch size measurement**: Record actual batch sizes to understand batching efficiency
- **Single-batcher option**: Optional architecture to maximize batch sizes
- **Cluster configuration**: Document replication quorum and client routing

### PostgreSQL-Specific
- **Connection pool sizing**: Start with 2x vCPU count (not arbitrary large pools)
- **Pool pre-warming**: Set min_idle = max_size to avoid connection overhead during measurement
- **Pool recycling method**: Use `RecyclingMethod::Verified` for consistency
- **Auto-vacuum control**: Disable during tests, run VACUUM ANALYZE before each test
- **Buffer pool consistency**: DROP DATABASE between runs for clean buffer pool state
- **Isolation level testing**: Test multiple levels with documented trade-offs

### Cloud Testing
- **Network latency measurement**: Record inter-node latency before tests
- **Clock synchronization**: NTP with <1ms drift tolerance for accurate metric correlation
- **Multiple runs with cleanup**: Reset database state between runs with 60s stabilization
- **Client coordination**: Barrier synchronization for simultaneous start
- **Resource monitoring**: Track CPU, memory, disk I/O, network I/O on all nodes

### Additional Considerations
- **Endurance testing**: 2-4 hour tests to detect long-term issues (memory leaks, checkpoint patterns)
- **Failure testing**: Optional failover and recovery performance measurement
- **Connection pool tuning**: Test multiple pool sizes to find optimal configuration
- **Think time**: Zero by default (max throughput focus), configurable for realistic workload modeling


