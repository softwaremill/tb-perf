# TigerBeetle vs PostgreSQL Performance Comparison - Implementation Plan

## Project Overview
Build a comprehensive performance comparison framework for TigerBeetle vs PostgreSQL on double-entry bookkeeping workloads. Support both local single-node testing and cloud-based 3-node replicated cluster testing with full observability.

## Quick Summary

- Performance comparison framework for TigerBeetle vs PostgreSQL
- Double-entry bookkeeping workload
- Local and cloud deployment modes
- Two test modes: maximum throughput and fixed request rate
- Automated test execution with multiple runs
- Equivalent durability and replication for fair comparison
- Full observability and metrics collection

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
- `test_mode` - "max_throughput" or "fixed_rate" (controls workload generation strategy)
- `num_accounts` - Total number of accounts (default: 100,000)
- `zipfian_exponent` - Account selection skew (0 = uniform, ~1.5 = high skew)
- `initial_balance` - Starting balance per account (default: 1,000,000)
- `min_transfer_amount` - Minimum transfer (default: 1)
- `max_transfer_amount` - Maximum transfer (default: 1,000)
- `warmup_duration_secs` - Warmup period before metrics collection (default: 120)
- `test_duration_secs` - Measurement phase duration (default: 300)
- `database` - TigerBeetle or PostgreSQL
- `test_runs` - Number of test runs per configuration (default: 3)
- `max_variance_threshold` - Maximum acceptable variance between runs (default: 0.10 = 10%)
- `max_error_rate` - Maximum error rate for valid test (default: 0.05 = 5%)

**Test Mode: max_throughput**:
- `concurrency` - Concurrent workers per client node (default: 10, based on 2x vCPU count)
- Closed-loop: each worker continuously sends requests (no pauses)
- Measures maximum system throughput and service time under backpressure
- Latencies represent "closed-loop service time" (will underreport tails vs open-loop)

**Test Mode: fixed_rate**:
- `target_rate` - Target requests per second (e.g., 1000, 5000, 10000)
- `max_concurrency` - Maximum concurrent workers (safety limit, default: 1000)
- Open-loop: requests issued at constant rate regardless of completion time
- Enables coordinated omission correction via HdrHistogram `record_correct()`
- Tests system behavior under realistic arrival patterns
- Use this mode to measure latency under various load levels (e.g., 50%, 75%, 90% of max throughput)

**PostgreSQL-Specific Configuration**:
- `isolation_level` - READ_COMMITTED | REPEATABLE_READ | SERIALIZABLE
- `synchronous_commit` - off | local | remote_write | remote_apply | on
- `connection_pool_size` - Connection pool size (default: 20, based on 2x vCPU + spindles formula)
- `connection_pool_min_idle` - Minimum idle connections (default: same as max, for pre-warming)
- `pool_recycling_method` - Fast | Verified (default: Verified for benchmark consistency)
- `auto_vacuum` - Enable/disable auto-vacuum during tests (default: disabled for consistent results)

**TigerBeetle-Specific Configuration**:
- `replication_quorum` - Replication quorum for 3-node cluster (default: 2)
- `measure_batch_sizes` - Record actual batch sizes during test (default: true)
- Note: Uses default TigerBeetle client batching behavior (no custom batch size configuration)

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
- **Test Phases**:
  1. **Warmup Phase** (default: 120s): Run workload at full target load to:
     - Fill database buffer pools and caches
     - Complete JIT compilation (if applicable)
     - Stabilize connection pools
     - Metrics collected but discarded (tagged with `phase="warmup"`)
  2. **Measurement Phase** (default: 300s): Continue at full target load, metrics collected and exported
- No retry logic for insufficient balance (count as successful rejection, not an error)
- Retry logic for serialization failures (with exponential backoff)
- **Latency Recording**: Use HdrHistogram with proper configuration
  - Initialization: `Histogram::<u64>::new_with_bounds(1, 60_000_000, 3)` (1μs to 60s, 3 significant figures)
  - **max_throughput mode**: Record actual observed latencies with `record(value)`
    - Measures closed-loop service time under backpressure
    - Note: This will underreport tail latencies compared to open-loop
  - **fixed_rate mode**: Use `record_correct(value, expected_interval_ns)` for coordinated omission correction
    - `expected_interval_ns = 1_000_000_000 / target_rate`
    - Measures open-loop response time under realistic arrival patterns
  - Multi-client aggregation: Clients export serialized HdrHistogram data (not pre-computed percentiles)
  - Server-side: Use `histogram.add(&other_histogram)` to merge client histograms
- OpenTelemetry metrics collection:
  - Successful transfers (completed)
  - Rejected transfers (insufficient balance - business rejection, NOT an error)
  - Failed transfers (database errors after max retries - serialization failures, connection errors, etc.)
  - Latency percentiles (p50, p95, p99, p999) for successful+rejected requests
  - Throughput (total requests/sec)
  - Database error rate (%) - test invalid if > 5%
  - TigerBeetle-specific: actual batch sizes used
  - Client resource utilization (CPU, memory) to detect client saturation

**TigerBeetle implementation**:
- Use official Rust client (batching handled by the client library)
- Handle TigerBeetle-specific errors

**PostgreSQL implementation**:
- Connection pooling (using `deadpool-postgres` with `RecyclingMethod::Verified`)
  - Pool min_idle = max_size for pre-warming (avoids connection overhead during measurement)
- Call stored procedure with pessimistic locking
- **Error handling**:
  - Insufficient balance returned by stored procedure: Count as business rejection (NOT an error)
  - Serialization failures: Retry with exponential backoff, count as error only after max retries
  - Connection errors: Count as database error immediately
- Configurable isolation level per connection
- Run explicit CHECKPOINT + VACUUM ANALYZE before each test (with auto-vacuum disabled during tests)
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
  - **Test Phase Indicator**: State timeline panel using `phase` label to show Warmup / Measurement
  - **Test Mode Indicator**: Show whether running max_throughput or fixed_rate mode
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
  - For fixed_rate mode: Target rate vs actual achieved rate comparison

**Metrics Export**:
- JSON export at test completion with summary metrics (measurement phase only):
  - Test configuration (all parameters including test_mode)
  - Test run number (1-N) and timestamp
  - Test duration (warmup, measurement phases)
  - Throughput (requests/sec, successful transfers/sec, rejected/sec)
  - Latency percentiles (p50, p95, p99, p999) from HdrHistogram
    - For max_throughput: closed-loop service time
    - For fixed_rate: open-loop response time (coordinated omission corrected)
  - Error count and rate
  - Resource usage averages (CPU, memory, disk I/O, network I/O)
  - Network latency between nodes (mean, p95, p99)
  - TigerBeetle-specific: mean/median batch size
  - Client saturation indicator (max CPU utilization)
  - For fixed_rate mode: target rate, achieved rate, completion percentage
  - Statistical metadata: standard deviation across runs, coefficient of variation
  - Software versions: PostgreSQL version, TigerBeetle version, Rust toolchain version

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
      - Workload executes at full target load (max_throughput: at target concurrency, fixed_rate: at target rate)
      - Fills buffer pools, completes JIT compilation, stabilizes connection pools
      - Metrics sent to OTel Collector, stored in Prometheus with `phase="warmup"` label
      - Filtered out during result aggregation

   b. **Phase 2: Measurement** (default 300s)
      - Client emits phase marker: `test.phase="measurement"` via OpenTelemetry
      - Workload continues at full target load
      - **Metrics tagged as `phase="measurement"`** (ONLY these are used for results)
      - HdrHistogram records latencies:
        - max_throughput mode: `record(value)` for closed-loop service time
        - fixed_rate mode: `record_correct(value, expected_interval)` for coordinated omission correction
      - All metrics sent via OTLP, timestamped and associated with run_id

   c. **Per-Run Export**:
      - Query Prometheus for metrics with `phase="measurement"` AND `run_id=N`
      - Export to `results/{config_name}/run_{N}_{timestamp}.json` with all metrics from measurement phase

   d. **Database Reset**:
      - For PostgreSQL:
        - DROP DATABASE + recreate schema + repopulate accounts (ensures clean buffer pool state)
        - Run explicit CHECKPOINT before VACUUM ANALYZE
        - Run VACUUM ANALYZE after repopulation
      - For TigerBeetle: Truncate transfers, reset account balances
      - Wait 30s for system to stabilize (checkpoint completion, WAL flush, buffer cache, network draining)

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

**Key Design Principle**: Client emits phase labels (`warmup`, `measurement`) with all metrics. Only `phase="measurement"` metrics exported to final results. Zero manual intervention required.

### 2.6 Baseline Testing Strategy and Durability Equivalence

**Baseline Approach**:
This benchmark focuses on comparing TigerBeetle and PostgreSQL under **equivalent durability and replication guarantees**. We are NOT interested in:
- Async PostgreSQL (synchronous_commit=off) as a baseline
- Single-node deployments without replication

**Target Configuration for Fair Comparison**:
- **TigerBeetle**: 3-node cluster with replication_quorum=2 (synchronous replication to majority)
- **PostgreSQL**: 3-node cluster with `synchronous_standby_names = 'ANY 2 (*)'` and `synchronous_commit=on/remote_apply` (synchronous replication to majority)

Both configurations provide:
- Durability: Data persisted to disk on multiple nodes
- Consistency: Quorum-based replication (2 of 3 nodes must acknowledge)
- Availability: System tolerates single node failure

**Testing Multiple Durability Levels**:
While the fair comparison uses equivalent durability, we will test PostgreSQL at multiple `synchronous_commit` levels to understand the performance/durability tradeoff:
- `off` - Fully async (NOT equivalent to TigerBeetle, documented for reference only)
- `local` - Local fsync only (NOT equivalent to TigerBeetle)
- `remote_write` - Written to standby OS cache
- `remote_apply` - Applied to standby database (closest to TigerBeetle's guarantees)
- `on` - Same as `remote_apply` for synchronous standbys

**Documentation Note**: Results will clearly label which PostgreSQL configuration is comparable to TigerBeetle (3-node, remote_apply, quorum=2). Other configurations shown for educational purposes only.

### 2.7 Local Testing Documentation

Create detailed local testing documentation with:
- Prerequisites (Docker, Rust toolchain)
- How to run tests step-by-step
- Configuration options explained
- Understanding test modes (max_throughput vs fixed_rate)
- Understanding test phases (warmup, measurement)
- How to access Grafana dashboards
- Interpreting metrics and identifying client saturation
- Statistical validity requirements (multiple runs, variance thresholds)
- Software version pinning (Docker images, database versions)
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
  - **Verify NVMe mounting**: Setup script must verify NVMe device is mounted before starting database
- For PostgreSQL: Set up synchronous 3-node cluster replication (matching TigerBeetle's consistency model)
  - Primary + 2 synchronous standbys
  - Configure `synchronous_standby_names = 'ANY 2 (*)'` for 2-of-3 quorum
  - Maximize performance while maintaining replication/durability guarantees
  - Tune: shared_buffers, work_mem, fsync settings, WAL configuration, checkpoint settings
- For TigerBeetle: 3-node cluster with synchronous replication
- Install node-exporter for metrics
- Automated setup scripts via user-data
- **Version pinning**: Pin PostgreSQL version, TigerBeetle version, and all Docker image tags

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

**Clock Synchronization Note**:
- All latency measurements are computed **locally on each client node**
- Each client records start time and end time for its own requests
- Clients export serialized HdrHistogram data (not timestamps)
- Server-side coordinator aggregates histograms using `histogram.add()`
- **No cross-node clock synchronization required** for latency measurements
- NTP drift only matters for correlating logs/events, not for performance metrics

### 3.4 Automated Cloud Test Orchestration

Cloud tests run fully automatically from laptop with zero manual intervention during execution.

**Orchestration Flow**:

1. **Infrastructure Provisioning** (from laptop):
   - Run `terraform apply` to provision all AWS resources
   - Wait for all instances to be ready
   - Automated health checks for all services

2. **Pre-Test Setup**:
   - **NVMe verification**: Verify i4i instance storage is mounted on all DB nodes
   - **Version verification**: Verify PostgreSQL/TigerBeetle versions match pinned versions
   - **Network latency measurement**: Automatically measure RTT between all DB node pairs
   - Export network metrics to `results/{config_name}/network_latency.json`
   - Deploy client binary to all client nodes (parallel deployment)
   - Initialize database cluster (create accounts with initial balance 1,000,000)
   - For PostgreSQL: Run explicit CHECKPOINT + VACUUM ANALYZE, disable auto-vacuum
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
      - Full workload at target load (max_throughput: concurrency, fixed_rate: rate) on all client nodes
      - Fills buffer pools, completes JIT compilation, stabilizes system
      - Metrics sent to OTel Collector, tagged for exclusion from results

   c. **Phase 2: Measurement** (default 300s):
      - All clients emit: `test.phase="measurement"`, `run_id=N` via OpenTelemetry
      - Continue stable workload at target load
      - **All metrics from this phase used for final results**
      - Each client records local metrics in HdrHistogram

   d. **Per-Run Data Collection**:
      - Each client exports local metrics to coordinator
      - Coordinator aggregates across all client nodes:
        - Sum throughputs from all clients
        - Merge HdrHistograms for global latency percentiles
        - Max CPU saturation across all clients
      - Query Prometheus for DB-side metrics (CPU, disk I/O, network)
      - Filter by: `phase="measurement"` AND `run_id=N`
      - Export to `results/{config_name}/run_{N}_{timestamp}.json`

   f. **Database Reset Between Runs**:
      - For PostgreSQL:
        - DROP DATABASE on primary + recreate schema + repopulate accounts (propagates to standbys, ensures clean buffer pool)
        - Run explicit CHECKPOINT to ensure WAL flush completion
        - Run VACUUM ANALYZE after repopulation
      - For TigerBeetle: Truncate transfers on all nodes, reset account balances
      - Wait 60s for cluster stabilization (checkpoint completion, WAL flush, buffer cache, network draining)

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

**Test Modes**:
1. **max_throughput**: Find maximum sustainable throughput for each configuration
   - Run with varying concurrency levels to find saturation point
   - Primary metric: maximum TPS (transactions per second)

2. **fixed_rate**: Measure latency distribution under various load levels
   - Run at 50%, 75%, 90%, 95% of max throughput (from max_throughput tests)
   - Primary metric: latency percentiles under realistic load
   - Enables coordinated omission correction for accurate tail latencies

**PostgreSQL configurations to test**:
- Isolation levels: READ COMMITTED, REPEATABLE READ, SERIALIZABLE
- Synchronous commit levels: off, local, remote_write, remote_apply, on
- Fair comparison to TigerBeetle: 3-node, synchronous_commit=remote_apply, quorum=2

**Workload scenarios**:
- Low contention: 1M accounts, Zipfian with low skew (exponent ~0)
- High contention: 10K accounts, Zipfian with high skew (exponent ~1.5)
- Medium contention: 100K accounts, Zipfian with medium skew (exponent ~1.0)

### 4.2 Configuration File Format

**Example 1: max_throughput mode**
```toml
[workload]
test_mode = "max_throughput"
num_accounts = 100000
concurrency = 10  # concurrent workers per client node
zipfian_exponent = 1.0
initial_balance = 1000000
min_transfer_amount = 1
max_transfer_amount = 1000
warmup_duration_secs = 120
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

**Example 2: fixed_rate mode**
```toml
[workload]
test_mode = "fixed_rate"
target_rate = 5000  # requests per second (total across all clients)
max_concurrency = 1000  # safety limit
num_accounts = 100000
zipfian_exponent = 1.0
initial_balance = 1000000
min_transfer_amount = 1
max_transfer_amount = 1000
warmup_duration_secs = 120
test_duration_secs = 300
test_runs = 3

[database]
type = "tigerbeetle"

[tigerbeetle]
replication_quorum = 2
measure_batch_sizes = true

[deployment]
type = "cloud"
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

### 4.4 Cloud Testing Documentation

Create detailed cloud testing documentation with:
- Prerequisites (Terraform, AWS credentials, SSH keys)
- How to provision infrastructure
- Software version pinning strategy (PostgreSQL, TigerBeetle, Docker images)
- NVMe instance storage verification on i4i instances
- Network latency measurement procedures
- How to run tests from laptop remotely
- Understanding test modes (max_throughput vs fixed_rate)
- Understanding test phases (warmup, measurement)
- How to access Grafana dashboards (port forwarding or public access)
- Identifying client saturation vs database saturation
- How to run multiple tests on same infrastructure (using data cleanup script)
- Statistical validity: interpreting variance and confidence intervals
- Error classification: business rejections vs database errors
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
- **Multiple runs**: 3 runs per configuration (default)
- **Variance thresholds**: Flag results with >10% variance for investigation
- **Confidence intervals**: Report mean, standard deviation, and coefficient of variation
- **Outlier handling**: Flag and investigate high-variance runs before accepting results
- **Software version pinning**: Pin PostgreSQL, TigerBeetle, and Docker image versions for reproducibility

### Test Execution Strategy
- **Two test modes**:
  1. **max_throughput**: Closed-loop testing for maximum TPS measurement
  2. **fixed_rate**: Open-loop testing for accurate latency under realistic load
- **Warmup phase**: 120s at full target load to fill buffer pools, complete JIT compilation, stabilize connections
- **Measurement phase**: 300s at full target load with metrics collection
- **No ramp-up phase**: Warmup already runs at full load; ramp-up only useful for capacity testing
- **Baseline establishment**: Compare equivalent durability/replication configurations (3-node, quorum=2)
- **Database reset between runs**: DROP DATABASE (PostgreSQL) or truncate+reset (TigerBeetle) with explicit CHECKPOINT

### Latency Measurement
- **HdrHistogram configuration**: `Histogram::<u64>::new_with_bounds(1, 60_000_000, 3)` (1μs to 60s, 3 sig figs)
- **max_throughput mode**: `record(value)` for closed-loop service time (underreports tails)
- **fixed_rate mode**: `record_correct(value, expected_interval)` for coordinated omission correction
- **Multi-client aggregation**: Export serialized histograms, merge with `histogram.add()`
- **Phase separation**: Only measurement phase metrics exported to final results
- **Client saturation detection**: Monitor client CPU to ensure database limits are measured
- **Error rate validation**: Tests with >5% error rate flagged as invalid
- **Local latency computation**: All measurements computed on client nodes; no cross-node clock sync required

### TigerBeetle-Specific
- **Default batching**: Use TigerBeetle client's default batching behavior (no custom configuration)
- **Batch size measurement**: Record actual batch sizes to understand batching efficiency
- **Cluster configuration**: 3-node cluster with replication_quorum=2 for fair comparison to PostgreSQL

### Error Classification
- **Insufficient balance**: Business rejection, count as successful request (not an error)
- **Serialization failure**: Database error, retry with exponential backoff
  - Only count as error after max retries exhausted
- **Clear separation**: Business rejections tracked separately from database errors
- **Validation**: Tests with >5% database error rate are invalid

### PostgreSQL-Specific
- **Connection pool sizing**: Start with 2x vCPU count (not arbitrary large pools)
- **Pool pre-warming**: Set min_idle = max_size to avoid connection overhead during measurement
- **Pool recycling method**: Use `RecyclingMethod::Verified` for consistency
- **Auto-vacuum control**: Disable during tests, run VACUUM ANALYZE before each test
- **Buffer pool consistency**: DROP DATABASE between runs for clean buffer pool state
- **Explicit CHECKPOINT**: Run CHECKPOINT before VACUUM ANALYZE to ensure WAL flush completion
- **Isolation level testing**: Test multiple levels with documented trade-offs

### Cloud Testing
- **Network latency measurement**: Record inter-node latency before tests using ping/iperf3
- **NVMe verification**: Verify i4i instance storage is properly mounted before tests
- **Clock synchronization**: Not required for latency measurements (computed locally on clients)
  - NTP drift only matters for log correlation, not performance metrics
- **Multiple runs with cleanup**: Reset database state between runs with 60s stabilization and explicit CHECKPOINT
- **Client coordination**: Barrier synchronization for simultaneous start across client nodes
- **Resource monitoring**: Track CPU, memory, disk I/O, network I/O on all nodes

### Additional Considerations
- **Endurance testing**: 2-4 hour tests to detect long-term issues (memory leaks, checkpoint patterns)
  - PostgreSQL: Need 24+ checkpoint cycles (default 5min = 2 hours minimum)
- **Failure testing**: Optional failover and recovery performance measurement
- **Test mode flexibility**: max_throughput for finding capacity, fixed_rate for realistic latency measurement


