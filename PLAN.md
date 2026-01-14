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

### 1.1 Architecture Overview

The system consists of two main components:

**Test Coordinator** (single instance):
- Orchestrates test execution across all phases
- Manages test runs and database resets
- Aggregates metrics from all clients
- Exports results (JSON, CSV)
- Runs observability stack (Grafana, Prometheus, OpenTelemetry Collector)

**Test Clients** (one or more instances):
- Each client runs multiple concurrent workers
- Workers execute the workload (double-entry transfers)
- Collect local metrics (latency via HdrHistogram, throughput, errors)
- Export metrics to coordinator via OpenTelemetry
- In local mode: 1 client instance
- In cloud mode: N client instances (configurable)

**Configuration**:
- Single TOML configuration file shared by coordinator and all clients
- File contains sections for: workload, database, deployment, monitoring
- Coordinator uses: deployment, monitoring, test orchestration settings
- Clients use: workload, database connection, test mode settings

### 1.2 Project Initialization

Initialize a Rust workspace with the following structure:
- Workspace root with main Cargo.toml
- `coordinator/` - Binary crate for test coordinator
- `client/` - Binary crate for test client (workload execution)
- `common/` - Library crate for shared configuration parsing and types
- Docker directory for local testing setup (docker-compose, Dockerfiles)
- Grafana provisioning directory for dashboards and datasources
- Terraform directory with modules for AWS infrastructure (network, database-cluster, client-cluster)
- Scripts directory for database setup
- Example configuration files (local.toml, cloud.toml)

### 1.3 Configuration File Format

The system uses a single TOML configuration file read by both the coordinator and all clients.

**`[workload]` section** (used by clients):
- `test_mode` - "max_throughput" or "fixed_rate"
- `num_accounts` - Total number of accounts (default: 100,000)
- `zipfian_exponent` - Account selection skew (0 = uniform, ~1.5 = high skew)
- `initial_balance` - Starting balance per account (default: 1,000,000)
- `min_transfer_amount` - Minimum transfer (default: 1)
- `max_transfer_amount` - Maximum transfer (default: 1,000)
- `warmup_duration_secs` - Warmup period (default: 120)
- `test_duration_secs` - Measurement phase duration (default: 300)

For `test_mode = "max_throughput"`:
- `concurrency` - Concurrent workers per client instance (default: 10)

For `test_mode = "fixed_rate"`:
- `target_rate` - Total requests/sec across all client instances (e.g., 5000)
- `max_concurrency` - Safety limit on concurrent requests (default: 1000)

**`[database]` section** (used by clients):
- `type` - "postgresql" or "tigerbeetle"

**`[postgresql]` section** (used by clients when database.type = "postgresql"):
- `isolation_level` - READ_COMMITTED | REPEATABLE_READ | SERIALIZABLE
- `synchronous_commit` - off | local | remote_write | remote_apply | on
- `connection_pool_size` - Pool size (default: 20)
- `connection_pool_min_idle` - Min idle connections (default: same as max)
- `pool_recycling_method` - "Fast" | "Verified" (default: Verified)
- `auto_vacuum` - Enable/disable (default: false)

**`[tigerbeetle]` section** (used by clients when database.type = "tigerbeetle"):
- `cluster_addresses` - TigerBeetle cluster connection addresses

**`[deployment]` section** (used by coordinator):
- `type` - "local" or "cloud"
- `num_db_nodes` - Database cluster size (1 for local, 3 for cloud)
- `num_client_nodes` - Number of client instances (cloud only)
- `aws_region` - AWS region (cloud only)
- `db_instance_type` - EC2 instance type for database (default: i4i.xlarge)
- `client_instance_type` - EC2 instance type for clients (default: c5.large)
- `measure_network_latency` - Measure inter-node latency (default: true)

**`[coordinator]` section** (used by coordinator):
- `test_runs` - Number of test runs per configuration (default: 3)
- `max_variance_threshold` - Max acceptable variance between runs (default: 0.10)
- `max_error_rate` - Max error rate for valid test (default: 0.05)
- `metrics_export_path` - Path for JSON exports (default: "./results")
- `keep_grafana_running` - Keep Grafana after test (default: false)

**`[monitoring]` section** (used by coordinator):
- `grafana_port` - Grafana dashboard port (default: 3000)
- `prometheus_port` - Prometheus port (default: 9090)
- `otel_collector_port` - OpenTelemetry Collector port (default: 4317)

### 1.4 Configuration Parsing Implementation

Implement configuration parsing in the `common` crate:

**Configuration structure**:
- Define structs for each configuration section using serde
- Support conditional sections based on test mode and database type
- Use enums for test modes (MaxThroughput, FixedRate) with associated parameters

**Loading and validation**:
- Load TOML file and deserialize into strongly-typed structs
- Validate configuration based on test mode and database type
- Ensure required sections are present (e.g., postgresql section when database.type = "postgresql")
- Provide clear error messages for missing or invalid configuration

**Usage**:
- Coordinator reads full config, uses deployment, coordinator, and monitoring sections
- Clients read full config, use workload, database, and database-specific sections
- Both components ignore sections they don't need

## Phase 2: Single-Node Local Implementation

### 2.1 PostgreSQL Setup

**Schema**:
- Accounts table with balance column
- Transfers table for audit trail
- Stored procedure for transfer logic with pessimistic locking (SELECT FOR UPDATE)
- Accounts locked in consistent order to prevent deadlocks
- Balance check before transfer

**Isolation Levels and Correctness Analysis**:

Create documentation analyzing isolation levels specifically for the double-entry bookkeeping workload:

**The Double-Entry Transfer Transaction**:
- Read balances of two accounts (source and destination)
- Check if source has sufficient balance
- Debit source account
- Credit destination account
- Record transfer in audit log

**Analysis Required** (to be documented during implementation):

For each isolation level (READ COMMITTED, REPEATABLE READ, SERIALIZABLE), analyze:

1. **Which concurrency phenomena can occur in this workload?**
   - Can dirty reads happen? What would be the impact on balance correctness?
   - Can non-repeatable reads happen? Would this violate double-entry invariants?
   - Can lost updates occur? Could two transactions overwrite each other's balance changes?
   - Can write skew occur? Could concurrent transfers violate balance constraints?
   - Can phantom reads happen in this workload? (Note: workload doesn't use range queries)

2. **Does the isolation level impact correctness of double-entry bookkeeping?**
   - At READ COMMITTED: Can the system produce incorrect balances or violate double-entry invariants?
   - At REPEATABLE READ: Are there scenarios where correctness is compromised?
   - At SERIALIZABLE: Is this the only level that guarantees correctness?

3. **Role of pessimistic locking (SELECT FOR UPDATE)**:
   - How does explicit locking interact with each isolation level?
   - Does SELECT FOR UPDATE on both accounts prevent correctness issues at lower isolation levels?
   - What phenomena does locking prevent vs. what isolation level prevents?

4. **Performance vs. correctness tradeoff**:
   - If lower isolation levels are safe (with proper locking), document why
   - If lower isolation levels are unsafe, document specific failure scenarios
   - Document serialization failure rates expected at each level

**Testing approach**:
- Test all three isolation levels
- Verify balances after test completion (sum of all balances should equal initial total)
- Track serialization failure rates at each level
- Compare throughput degradation vs. isolation level

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

   c. Validate statistical consistency and correctness:
      - Flag runs where throughput CV > 10%
      - Flag runs where p99 latency CV > 15%
      - Flag runs where error rate > 5% (invalid test)
      - Warn if variance exceeds thresholds
      - **Correctness validation**: Verify sum of all account balances equals initial total
        - Query all account balances after test completion
        - Compare to expected total (num_accounts × initial_balance)
        - Flag any discrepancy (indicates data corruption or incorrect transaction logic)

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

   c. Statistical validation and correctness:
      - Flag throughput CV > 10%
      - Flag p99 latency CV > 15%
      - Flag error rate > 5% (invalid test)
      - Check for client saturation (CPU > 80%)
      - Check for network issues (high inter-node latency variance)
      - **Correctness validation**: Verify sum of all account balances equals initial total
        - Query all account balances after test completion
        - Flag any discrepancy (indicates data corruption or incorrect transaction logic)

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

### 4.2 Additional Testing Scenarios

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

### 4.3 Cloud Testing Documentation

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

