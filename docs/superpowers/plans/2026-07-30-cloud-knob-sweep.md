# Cloud Knob Sweep (max_concurrency & target_rate, hotspot skew) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Surface the previously-untracked `dropped_transfers` metric in coordinator results, then add 6 new cloud config files (TigerBeetle + PostgreSQL standard + PostgreSQL atomic, each at two new `max_concurrency`/`target_rate` combinations) so the hotspot-skew knob sweep described in `docs/superpowers/specs/2026-07-30-cloud-knob-sweep-design.md` can be run.

**Architecture:** No new components. Extends the existing `CollectedMetrics` → `RunResult` → `AggregateResults` pipeline in the `coordinator` crate with one more counter (mirroring how `completed_transfers`/`rejected_transfers`/`failed_transfers` already flow through), and adds new `config.cloud-*.toml` files that are structurally identical to the existing hotspot configs with two fields changed.

**Tech Stack:** Rust (edition 2024), `serde`/`toml` for config, Prometheus HTTP API queried via `reqwest` in `coordinator/src/prometheus.rs`.

## Global Constraints

- Rust 1.85+, edition 2024 (existing workspace requirement — no new toolchain needs).
- Metric names read from Prometheus carry a `tbperf_` prefix added by the OTel collector (see existing `COMPLETED`/`REJECTED`/`FAILED` consts in `coordinator/src/prometheus.rs:187-189`) — the client-side counter is registered as `requests_dropped` (`client/src/metrics.rs:82-87`), so the Prometheus metric name is `tbperf_requests_dropped_total`.
- Follow the existing per-run/aggregate field-naming convention exactly: `completed_transfers` / `rejected_transfers` / `failed_transfers` → add `dropped_transfers` alongside them (not `dropped_requests`), since `RunResult` and `AggregateResults` already use the `_transfers` suffix consistently.
- New config files must exactly mirror the existing `config.cloud-*-hotspot.toml` files (100k accounts, `zipfian_exponent = 2.0`, 5 client / 3 DB nodes, 3 test runs, 2min warmup + 5min measurement) with only `target_rate` and/or `max_concurrency` changed — do not alter any other field.

---

## Task 1: Query and surface `dropped_transfers` from Prometheus

**Files:**
- Modify: `coordinator/src/prometheus.rs`

**Interfaces:**
- Produces: `CollectedMetrics.dropped_transfers: u64` — consumed by Task 2.

- [ ] **Step 1: Update the existing default-metrics test to expect the new field**

In `coordinator/src/prometheus.rs`, in the `tests` module, update `test_collected_metrics_default`:

```rust
    #[test]
    fn test_collected_metrics_default() {
        let metrics = CollectedMetrics::default();
        assert_eq!(metrics.completed_transfers, 0);
        assert_eq!(metrics.rejected_transfers, 0);
        assert_eq!(metrics.failed_transfers, 0);
        assert_eq!(metrics.dropped_transfers, 0);
        assert_eq!(metrics.latency_p50_us, 0);
    }
```

- [ ] **Step 2: Run the test to verify it fails to compile (field doesn't exist yet)**

Run: `cargo test -p tb-perf-coordinator test_collected_metrics_default`
Expected: FAIL to compile — `no field \`dropped_transfers\` on type \`CollectedMetrics\``

- [ ] **Step 3: Add the field to `CollectedMetrics` and its `Display` impl**

In `coordinator/src/prometheus.rs`, change the struct (around line 29-38):

```rust
/// Metrics collected from Prometheus
#[derive(Debug, Clone, Default)]
pub struct CollectedMetrics {
    pub completed_transfers: u64,
    pub rejected_transfers: u64,
    pub failed_transfers: u64,
    pub dropped_transfers: u64,
    pub latency_p50_us: u64,
    pub latency_p95_us: u64,
    pub latency_p99_us: u64,
    pub latency_p999_us: u64,
}
```

And update `Display` (around line 40-54):

```rust
impl std::fmt::Display for CollectedMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "completed={}, rejected={}, failed={}, dropped={}, latency_us(p50={}, p95={}, p99={}, p999={})",
            self.completed_transfers,
            self.rejected_transfers,
            self.failed_transfers,
            self.dropped_transfers,
            self.latency_p50_us,
            self.latency_p95_us,
            self.latency_p99_us,
            self.latency_p999_us
        )
    }
}
```

- [ ] **Step 4: Query the new metric in `collect_metrics`**

In `coordinator/src/prometheus.rs`, inside `collect_metrics` (around line 186-201), add the constant and query alongside the existing three:

```rust
        // Metric names include tbperf_ prefix from OTel collector namespace config
        const COMPLETED: &str = "tbperf_transfers_completed_total";
        const REJECTED: &str = "tbperf_transfers_rejected_total";
        const FAILED: &str = "tbperf_transfers_failed_total";
        const DROPPED: &str = "tbperf_requests_dropped_total";
        const LATENCY: &str = "tbperf_transfer_latency_us";

        // Query counters
        if let Some(v) = self.query_counter(COMPLETED, &range, query_time).await? {
            metrics.completed_transfers = v;
        }
        if let Some(v) = self.query_counter(REJECTED, &range, query_time).await? {
            metrics.rejected_transfers = v;
        }
        if let Some(v) = self.query_counter(FAILED, &range, query_time).await? {
            metrics.failed_transfers = v;
        }
        if let Some(v) = self.query_counter(DROPPED, &range, query_time).await? {
            metrics.dropped_transfers = v;
        }
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p tb-perf-coordinator test_collected_metrics_default`
Expected: PASS

- [ ] **Step 6: Run the full prometheus test module to confirm no regressions**

Run: `cargo test -p tb-perf-coordinator prometheus::`
Expected: PASS (all tests in `coordinator/src/prometheus.rs`)

- [ ] **Step 7: Commit**

```bash
git add coordinator/src/prometheus.rs
git commit -m "$(cat <<'EOF'
Query dropped-request count from Prometheus in coordinator

The client already tracks requests dropped due to max_concurrency
(client/src/workload.rs, requests_dropped counter) but the coordinator
never queried it, making it impossible to tell whether a fixed_rate
test fell short of target_rate because of the concurrency cap or some
other bottleneck.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Thread `dropped_transfers` through `RunResult` and `AggregateResults`

**Files:**
- Modify: `coordinator/src/results.rs`
- Modify: `coordinator/src/test_runner.rs`

**Interfaces:**
- Consumes: `CollectedMetrics.dropped_transfers: u64` (Task 1).
- Produces: `RunResult.dropped_transfers: u64`, `AggregateResults.total_dropped: u64` — both now part of the exported JSON schema (`results/*/results.json`), consumed by any future analysis scripts (none currently read this field, so no other code needs updating).

- [ ] **Step 1: Update the `make_run_result` test helper and its call sites to include the new field**

In `coordinator/src/results.rs`, in the `tests` module, change the helper (around line 340-354):

```rust
    fn make_run_result(run_id: usize, tps: f64, completed: u64, failed: u64) -> RunResult {
        RunResult {
            run_id,
            duration_secs: 10.0,
            throughput_tps: tps,
            latency_p50_us: 100,
            latency_p95_us: 200,
            latency_p99_us: 300,
            latency_p999_us: 400,
            completed_transfers: completed,
            rejected_transfers: 0,
            failed_transfers: failed,
            dropped_transfers: 0,
            balance_verified: true,
        }
    }
```

(No call-site changes needed — the helper's parameter list is unchanged, only its body grows a new field set to a fixed default of `0`.)

- [ ] **Step 2: Add a test asserting `total_dropped` aggregates correctly**

In `coordinator/src/results.rs`, in the `tests` module, add:

```rust
    #[test]
    fn test_calculate_aggregates_total_dropped() {
        let config = make_test_config();
        let mut results = TestResults::new(config, 2);
        let mut run1 = make_run_result(1, 1000.0, 10000, 0);
        run1.dropped_transfers = 50;
        let mut run2 = make_run_result(2, 1000.0, 10000, 0);
        run2.dropped_transfers = 75;
        results.add_run(run1);
        results.add_run(run2);
        results.calculate_aggregates();

        let agg = results.aggregate.unwrap();
        assert_eq!(agg.total_dropped, 125);
    }
```

- [ ] **Step 3: Run the new test to verify it fails to compile**

Run: `cargo test -p tb-perf-coordinator test_calculate_aggregates_total_dropped`
Expected: FAIL to compile — `no field \`dropped_transfers\` on type \`RunResult\`` and `no field \`total_dropped\` on type \`AggregateResults\``

- [ ] **Step 4: Add `dropped_transfers` to `RunResult`**

In `coordinator/src/results.rs`, change the struct (around line 8-21):

```rust
/// Result of a single test run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    pub run_id: usize,
    pub duration_secs: f64,
    pub throughput_tps: f64,
    pub latency_p50_us: u64,
    pub latency_p95_us: u64,
    pub latency_p99_us: u64,
    pub latency_p999_us: u64,
    pub completed_transfers: u64,
    pub rejected_transfers: u64,
    pub failed_transfers: u64,
    pub dropped_transfers: u64,
    pub balance_verified: bool,
}
```

- [ ] **Step 5: Add `total_dropped` to `AggregateResults` and compute it in `calculate_aggregates`**

In `coordinator/src/results.rs`, change the struct (around line 84-95):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateResults {
    pub throughput: AggregateStats,
    pub latency_p50: AggregateStats,
    pub latency_p95: AggregateStats,
    pub latency_p99: AggregateStats,
    pub latency_p999: AggregateStats,
    pub total_completed: u64,
    pub total_rejected: u64,
    pub total_failed: u64,
    pub total_dropped: u64,
    pub error_rate: f64,
}
```

In `calculate_aggregates` (around line 141-187), add the sum and thread it into the constructed value:

```rust
        let total_completed: u64 = self.runs.iter().map(|r| r.completed_transfers).sum();
        let total_rejected: u64 = self.runs.iter().map(|r| r.rejected_transfers).sum();
        let total_failed: u64 = self.runs.iter().map(|r| r.failed_transfers).sum();
        let total_dropped: u64 = self.runs.iter().map(|r| r.dropped_transfers).sum();
```

```rust
        self.aggregate = Some(AggregateResults {
            throughput: throughput_stats,
            latency_p50: AggregateStats::from_values(&p50s),
            latency_p95: AggregateStats::from_values(&p95s),
            latency_p99: p99_stats,
            latency_p999: AggregateStats::from_values(&p999s),
            total_completed,
            total_rejected,
            total_failed,
            total_dropped,
            error_rate,
        });
```

- [ ] **Step 6: Surface `total_dropped` in `print_summary`**

In `coordinator/src/results.rs`, in `print_summary` (around line 232-236), add a line:

```rust
            info!("--- Transfers ---");
            info!("  Completed: {}", agg.total_completed);
            info!("  Rejected:  {}", agg.total_rejected);
            info!("  Failed:    {}", agg.total_failed);
            info!("  Dropped:   {} (max_concurrency reached)", agg.total_dropped);
            info!("  Error rate: {:.2}%", agg.error_rate * 100.0);
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p tb-perf-coordinator`
Expected: PASS (this will fail to compile until `test_runner.rs` is updated too — do Step 8 first if the compiler complains about `RunResult` construction there, then re-run)

- [ ] **Step 8: Wire `dropped_transfers` through `test_runner.rs`**

In `coordinator/src/test_runner.rs`, update the log line (around line 289-293):

```rust
            Ok(m) => {
                info!(
                    "Collected metrics: completed={}, rejected={}, failed={}, dropped={}",
                    m.completed_transfers, m.rejected_transfers, m.failed_transfers, m.dropped_transfers
                );
                m
            }
```

And the `RunResult` construction (around line 308-320):

```rust
        Ok(RunResult {
            run_id,
            duration_secs: elapsed.as_secs_f64(),
            throughput_tps,
            latency_p50_us: metrics.latency_p50_us,
            latency_p95_us: metrics.latency_p95_us,
            latency_p99_us: metrics.latency_p99_us,
            latency_p999_us: metrics.latency_p999_us,
            completed_transfers: metrics.completed_transfers,
            rejected_transfers: metrics.rejected_transfers,
            failed_transfers: metrics.failed_transfers,
            dropped_transfers: metrics.dropped_transfers,
            balance_verified: client_success,
```

(Leave everything else in that struct literal and the surrounding function untouched — only the two new lines shown above are added.)

- [ ] **Step 9: Run the full test suite to verify everything passes**

Run: `cargo test -p tb-perf-coordinator`
Expected: PASS, including `test_calculate_aggregates_total_dropped`

- [ ] **Step 10: Commit**

```bash
git add coordinator/src/results.rs coordinator/src/test_runner.rs
git commit -m "$(cat <<'EOF'
Thread dropped_transfers through RunResult and AggregateResults

Surfaces the Prometheus-sourced dropped-request count (added in the
previous commit) in the per-run and aggregate JSON export, so future
test results can show whether max_concurrency was the binding
constraint instead of silently omitting dropped requests.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Add the 6 new hotspot-skew cloud config files

**Files:**
- Create: `config.cloud-tigerbeetle-hotspot-concurrency5k.toml`
- Create: `config.cloud-tigerbeetle-hotspot-rate10k.toml`
- Create: `config.cloud-postgresql-hotspot-concurrency5k.toml`
- Create: `config.cloud-postgresql-hotspot-rate10k.toml`
- Create: `config.cloud-postgresql-atomic-hotspot-concurrency5k.toml`
- Create: `config.cloud-postgresql-atomic-hotspot-rate10k.toml`
- Modify: `common/src/config.rs` (add a regression test)

**Interfaces:**
- None — these are leaf config files consumed directly by `./target/release/coordinator -c <file>`.

- [ ] **Step 1: Write a failing test that the 6 new files parse and validate**

In `common/src/config.rs`, in the `tests` module, add (this will fail until Step 2 creates the files):

```rust
    #[test]
    fn test_new_hotspot_knob_sweep_configs_parse_and_validate() {
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let files = [
            "config.cloud-tigerbeetle-hotspot-concurrency5k.toml",
            "config.cloud-tigerbeetle-hotspot-rate10k.toml",
            "config.cloud-postgresql-hotspot-concurrency5k.toml",
            "config.cloud-postgresql-hotspot-rate10k.toml",
            "config.cloud-postgresql-atomic-hotspot-concurrency5k.toml",
            "config.cloud-postgresql-atomic-hotspot-rate10k.toml",
        ];
        for file in files {
            let path = repo_root.join(file);
            let config = Config::from_file(&path)
                .unwrap_or_else(|e| panic!("failed to parse/validate {}: {:?}", file, e));
            assert_eq!(config.workload.zipfian_exponent, 2.0, "{file}: wrong skew");
            assert_eq!(config.deployment.num_client_nodes, Some(5), "{file}: wrong client count");
        }
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p tb-perf-common test_new_hotspot_knob_sweep_configs_parse_and_validate`
Expected: FAIL — panics on the first missing file (`config.cloud-tigerbeetle-hotspot-concurrency5k.toml`)

- [ ] **Step 3: Create `config.cloud-tigerbeetle-hotspot-concurrency5k.toml`**

```toml
# Cloud TigerBeetle hotspot-contention test configuration - concurrency sweep
# Same as config.cloud-tigerbeetle-hotspot.toml, but max_concurrency raised
# from 1,000 to 5,000 (target_rate unchanged at 5,000) to test whether the
# concurrency cap was the binding constraint on achieved throughput - see
# docs/superpowers/specs/2026-07-30-cloud-knob-sweep-design.md.

[workload]
test_mode = "fixed_rate"
target_rate = 5000  # Total requests/sec across all 5 client instances
max_concurrency = 5000
num_accounts = 100000
zipfian_exponent = 2.0
initial_balance = 1000000
min_transfer_amount = 1
max_transfer_amount = 1000
warmup_duration_secs = 120
test_duration_secs = 300

[database]
type = "tigerbeetle"

[tigerbeetle]
cluster_addresses = ["tb-node-1:3000", "tb-node-2:3000", "tb-node-3:3000"]

[deployment]
type = "cloud"
num_db_nodes = 3
num_client_nodes = 5
gcp_project = "tigerbettle-sandbox"
gcp_region = "europe-central2"
db_machine_type = "n2-highmem-4"
client_machine_type = "n2-standard-2"
measure_network_latency = true

[coordinator]
test_runs = 3
max_variance_threshold = 0.10
max_error_rate = 0.05
metrics_export_path = "./results"
keep_grafana_running = true

[monitoring]
grafana_port = 3000
prometheus_port = 9090
otel_collector_port = 4317
```

- [ ] **Step 4: Create `config.cloud-tigerbeetle-hotspot-rate10k.toml`**

```toml
# Cloud TigerBeetle hotspot-contention test configuration - rate sweep
# Same as config.cloud-tigerbeetle-hotspot-concurrency5k.toml, but
# target_rate doubled from 5,000 to 10,000 (max_concurrency held at 5,000)
# to find the real throughput ceiling once the concurrency cap isn't
# confounding the result - see
# docs/superpowers/specs/2026-07-30-cloud-knob-sweep-design.md.

[workload]
test_mode = "fixed_rate"
target_rate = 10000  # Total requests/sec across all 5 client instances
max_concurrency = 5000
num_accounts = 100000
zipfian_exponent = 2.0
initial_balance = 1000000
min_transfer_amount = 1
max_transfer_amount = 1000
warmup_duration_secs = 120
test_duration_secs = 300

[database]
type = "tigerbeetle"

[tigerbeetle]
cluster_addresses = ["tb-node-1:3000", "tb-node-2:3000", "tb-node-3:3000"]

[deployment]
type = "cloud"
num_db_nodes = 3
num_client_nodes = 5
gcp_project = "tigerbettle-sandbox"
gcp_region = "europe-central2"
db_machine_type = "n2-highmem-4"
client_machine_type = "n2-standard-2"
measure_network_latency = true

[coordinator]
test_runs = 3
max_variance_threshold = 0.10
max_error_rate = 0.05
metrics_export_path = "./results"
keep_grafana_running = true

[monitoring]
grafana_port = 3000
prometheus_port = 9090
otel_collector_port = 4317
```

- [ ] **Step 5: Create `config.cloud-postgresql-hotspot-concurrency5k.toml`**

```toml
# Cloud PostgreSQL (standard executor) hotspot-contention test configuration
# - concurrency sweep. Same as config.cloud-postgresql-hotspot.toml, but
# max_concurrency raised from 1,000 to 5,000 (target_rate unchanged at
# 5,000) - see docs/superpowers/specs/2026-07-30-cloud-knob-sweep-design.md.

[workload]
test_mode = "fixed_rate"
target_rate = 5000 # Total requests/sec across all client instances
max_concurrency = 5000
num_accounts = 100000
zipfian_exponent = 2.0
initial_balance = 1000000
min_transfer_amount = 1
max_transfer_amount = 1000
warmup_duration_secs = 120
test_duration_secs = 300

[database]
type = "postgresql"

[postgresql]
isolation_level = "read_committed"
connection_pool_size = 20
connection_pool_min_idle = 20
executor_mode = "standard"

[deployment]
type = "cloud"
num_db_nodes = 3
num_client_nodes = 5
gcp_project = "tigerbettle-sandbox"
gcp_region = "europe-central2"
db_machine_type = "n2-highmem-4"
client_machine_type = "n2-standard-2"
measure_network_latency = true

[coordinator]
test_runs = 3
max_variance_threshold = 0.10
max_error_rate = 0.05
metrics_export_path = "./results"
keep_grafana_running = true

[monitoring]
grafana_port = 3000
prometheus_port = 9090
otel_collector_port = 4317
```

- [ ] **Step 6: Create `config.cloud-postgresql-hotspot-rate10k.toml`**

```toml
# Cloud PostgreSQL (standard executor) hotspot-contention test configuration
# - rate sweep. Same as config.cloud-postgresql-hotspot-concurrency5k.toml,
# but target_rate doubled from 5,000 to 10,000 (max_concurrency held at
# 5,000) - see docs/superpowers/specs/2026-07-30-cloud-knob-sweep-design.md.

[workload]
test_mode = "fixed_rate"
target_rate = 10000 # Total requests/sec across all client instances
max_concurrency = 5000
num_accounts = 100000
zipfian_exponent = 2.0
initial_balance = 1000000
min_transfer_amount = 1
max_transfer_amount = 1000
warmup_duration_secs = 120
test_duration_secs = 300

[database]
type = "postgresql"

[postgresql]
isolation_level = "read_committed"
connection_pool_size = 20
connection_pool_min_idle = 20
executor_mode = "standard"

[deployment]
type = "cloud"
num_db_nodes = 3
num_client_nodes = 5
gcp_project = "tigerbettle-sandbox"
gcp_region = "europe-central2"
db_machine_type = "n2-highmem-4"
client_machine_type = "n2-standard-2"
measure_network_latency = true

[coordinator]
test_runs = 3
max_variance_threshold = 0.10
max_error_rate = 0.05
metrics_export_path = "./results"
keep_grafana_running = true

[monitoring]
grafana_port = 3000
prometheus_port = 9090
otel_collector_port = 4317
```

- [ ] **Step 7: Create `config.cloud-postgresql-atomic-hotspot-concurrency5k.toml`**

```toml
# Cloud PostgreSQL (atomic executor) hotspot-contention test configuration
# - concurrency sweep. Same as config.cloud-postgresql-atomic-hotspot.toml,
# but max_concurrency raised from 1,000 to 5,000 (target_rate unchanged at
# 5,000) - see docs/superpowers/specs/2026-07-30-cloud-knob-sweep-design.md.

[workload]
test_mode = "fixed_rate"
target_rate = 5000 # Total requests/sec across all client instances
max_concurrency = 5000
num_accounts = 100000
zipfian_exponent = 2.0
initial_balance = 1000000
min_transfer_amount = 1
max_transfer_amount = 1000
warmup_duration_secs = 120
test_duration_secs = 300

[database]
type = "postgresql"

[postgresql]
isolation_level = "read_committed"
connection_pool_size = 20
connection_pool_min_idle = 20
executor_mode = "atomic"

[deployment]
type = "cloud"
num_db_nodes = 3
num_client_nodes = 5
gcp_project = "tigerbettle-sandbox"
gcp_region = "europe-central2"
db_machine_type = "n2-highmem-4"
client_machine_type = "n2-standard-2"
measure_network_latency = true

[coordinator]
test_runs = 3
max_variance_threshold = 0.10
max_error_rate = 0.05
metrics_export_path = "./results"
keep_grafana_running = true

[monitoring]
grafana_port = 3000
prometheus_port = 9090
otel_collector_port = 4317
```

- [ ] **Step 8: Create `config.cloud-postgresql-atomic-hotspot-rate10k.toml`**

```toml
# Cloud PostgreSQL (atomic executor) hotspot-contention test configuration
# - rate sweep. Same as
# config.cloud-postgresql-atomic-hotspot-concurrency5k.toml, but target_rate
# doubled from 5,000 to 10,000 (max_concurrency held at 5,000) - see
# docs/superpowers/specs/2026-07-30-cloud-knob-sweep-design.md.

[workload]
test_mode = "fixed_rate"
target_rate = 10000 # Total requests/sec across all client instances
max_concurrency = 5000
num_accounts = 100000
zipfian_exponent = 2.0
initial_balance = 1000000
min_transfer_amount = 1
max_transfer_amount = 1000
warmup_duration_secs = 120
test_duration_secs = 300

[database]
type = "postgresql"

[postgresql]
isolation_level = "read_committed"
connection_pool_size = 20
connection_pool_min_idle = 20
executor_mode = "atomic"

[deployment]
type = "cloud"
num_db_nodes = 3
num_client_nodes = 5
gcp_project = "tigerbettle-sandbox"
gcp_region = "europe-central2"
db_machine_type = "n2-highmem-4"
client_machine_type = "n2-standard-2"
measure_network_latency = true

[coordinator]
test_runs = 3
max_variance_threshold = 0.10
max_error_rate = 0.05
metrics_export_path = "./results"
keep_grafana_running = true

[monitoring]
grafana_port = 3000
prometheus_port = 9090
otel_collector_port = 4317
```

- [ ] **Step 9: Run the test to verify it passes**

Run: `cargo test -p tb-perf-common test_new_hotspot_knob_sweep_configs_parse_and_validate`
Expected: PASS

- [ ] **Step 10: Run the full workspace test suite to confirm no regressions anywhere**

Run: `cargo test`
Expected: PASS (all crates: `common`, `client`, `coordinator`)

- [ ] **Step 11: Commit**

```bash
git add config.cloud-tigerbeetle-hotspot-concurrency5k.toml \
        config.cloud-tigerbeetle-hotspot-rate10k.toml \
        config.cloud-postgresql-hotspot-concurrency5k.toml \
        config.cloud-postgresql-hotspot-rate10k.toml \
        config.cloud-postgresql-atomic-hotspot-concurrency5k.toml \
        config.cloud-postgresql-atomic-hotspot-rate10k.toml \
        common/src/config.rs
git commit -m "$(cat <<'EOF'
Add hotspot-skew cloud configs for max_concurrency/target_rate sweep

Six new configs (TigerBeetle, PostgreSQL standard, PostgreSQL atomic,
each at max_concurrency=5000/target_rate=5000 and
max_concurrency=5000/target_rate=10000) to test whether the previous
1,000 concurrency cap was constraining achieved throughput, and where
the real ceiling is once it isn't. See
docs/superpowers/specs/2026-07-30-cloud-knob-sweep-design.md for the
full rationale.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Build the release binaries

**Files:** None (verification only — no source changes).

**Interfaces:** None.

- [ ] **Step 1: Build the coordinator and client in release mode**

Run: `source scripts/setup-zig.sh && cargo build --release`
Expected: Build succeeds with no errors (this compiles the code changes from Tasks 1-2 in release mode, which is what the cloud runs actually execute).

Note: do not invoke `./target/release/coordinator` directly against the new config files as a smoke test — `Config::from_file` runs at the very top of `main()` (`coordinator/src/main.rs:49`), before the `--no-docker` flag has any effect, so any invocation with `deployment.type = "cloud"` immediately proceeds into real GCP discovery/SSH logic (`run_cloud_tests`). The `test_new_hotspot_knob_sweep_configs_parse_and_validate` test added in Task 3 is the safe, authoritative check that these configs parse and validate — it exercises the exact same `Config::from_file` path without triggering any cloud calls.

---

## Out of scope (reminder from the spec)

- Do not run the actual cloud tests as part of this plan — that's a separate, explicitly-confirmed step (checking GCP infra state, running `terraform apply` if needed, then invoking the coordinator against each of the 6 new configs). This plan only prepares the code and config files.
- Do not add moderate-skew (`zipfian_exponent = 1.0`) variants, batched-executor configs, or additional knob values.
- Do not change `num_client_nodes` (stays at 5).
