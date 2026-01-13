use crate::docker::DockerManager;
use crate::postgres_setup;
use crate::prometheus::PrometheusClient;
use crate::results::{RunResult, TestResults};
use crate::run_context::RunContext;
use crate::tigerbeetle_setup;
use anyhow::{Context, Result};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tb_perf_common::Config;
use tb_perf_common::config::DatabaseType;
use tokio::process::Command;
use tracing::{debug, error, info, warn};

/// Runs the test orchestration for local tests
pub struct TestRunner<'a> {
    config: Config,
    config_path: String,
    docker: DockerManager,
    prometheus: PrometheusClient,
    run_ctx: &'a RunContext,
}

impl<'a> TestRunner<'a> {
    pub fn new(
        config: Config,
        config_path: String,
        docker: DockerManager,
        run_ctx: &'a RunContext,
    ) -> Self {
        let prometheus_url = format!("http://localhost:{}", config.monitoring.prometheus_port);
        Self {
            config,
            config_path,
            docker,
            prometheus: PrometheusClient::new(&prometheus_url),
            run_ctx,
        }
    }

    /// Run all test iterations
    pub async fn run(&self) -> Result<TestResults> {
        let num_runs = self.config.coordinator.test_runs;
        let num_accounts = self.config.workload.num_accounts;
        let initial_balance = self.config.workload.initial_balance;
        let expected_total = num_accounts * initial_balance;

        info!("Starting test execution with {} runs", num_runs);

        // Initialize database based on type
        match self.config.database.kind {
            DatabaseType::PostgreSQL => {
                postgres_setup::reset_database(&self.docker, num_accounts, initial_balance).await?;
            }
            DatabaseType::TigerBeetle => {
                let tb_config = self
                    .config
                    .tigerbeetle
                    .as_ref()
                    .context("TigerBeetle config missing")?;
                tigerbeetle_setup::init_accounts(
                    &tb_config.cluster_addresses,
                    num_accounts,
                    initial_balance,
                )
                .await?;
            }
        }

        let mut results = TestResults::new(self.config.clone(), num_runs);

        for run_id in 1..=num_runs {
            info!("=== Starting run {}/{} ===", run_id, num_runs);

            let run_result = self.run_single_test(run_id).await?;
            results.add_run(run_result);

            // Verify balance correctness
            let balance_ok = match self.config.database.kind {
                DatabaseType::PostgreSQL => {
                    postgres_setup::verify_total_balance(&self.docker, expected_total).await?
                }
                DatabaseType::TigerBeetle => {
                    let tb_config = self
                        .config
                        .tigerbeetle
                        .as_ref()
                        .context("TigerBeetle config missing")?;
                    tigerbeetle_setup::verify_total_balance(
                        &tb_config.cluster_addresses,
                        num_accounts,
                        expected_total,
                    )
                    .await?
                }
            };

            if !balance_ok {
                error!("Balance verification failed for run {}", run_id);
                results.set_balance_error(run_id);
            }

            // Reset between runs (except last)
            if run_id < num_runs {
                info!("Resetting database for next run...");

                match self.config.database.kind {
                    DatabaseType::PostgreSQL => {
                        postgres_setup::reset_database(&self.docker, num_accounts, initial_balance)
                            .await?;
                    }
                    DatabaseType::TigerBeetle => {
                        // TigerBeetle requires container restart for clean state
                        self.docker.restart_service("tigerbeetle").await?;
                        self.docker
                            .wait_for_tigerbeetle_services(Duration::from_secs(60))
                            .await?;

                        // Wait for TigerBeetle API to be ready (more reliable than port check)
                        let tb_config = self
                            .config
                            .tigerbeetle
                            .as_ref()
                            .context("TigerBeetle config missing")?;
                        tigerbeetle_setup::wait_for_ready(&tb_config.cluster_addresses, 60).await?;

                        // Re-initialize accounts
                        tigerbeetle_setup::init_accounts(
                            &tb_config.cluster_addresses,
                            num_accounts,
                            initial_balance,
                        )
                        .await?;
                    }
                }

                // Wait for system to stabilize
                info!("Waiting 30s for system stabilization...");
                tokio::time::sleep(Duration::from_secs(30)).await;
            }

            info!("=== Completed run {}/{} ===", run_id, num_runs);
        }

        // Calculate aggregate statistics
        results.calculate_aggregates();

        Ok(results)
    }

    /// Run a single test iteration
    async fn run_single_test(&self, run_id: usize) -> Result<RunResult> {
        let warmup_duration = self.config.workload.warmup_duration_secs;
        let test_duration = self.config.workload.test_duration_secs;
        let total_duration = warmup_duration + test_duration;

        info!(
            "Run {}: warmup {}s, measurement {}s (total {}s)",
            run_id, warmup_duration, test_duration, total_duration
        );

        let start_time = Instant::now();

        // Find the client binary and validate it
        let client_binary = find_client_binary()?;
        validate_client_binary(&client_binary)?;
        info!("Using client binary: {}", client_binary);

        // Record spawn time as Unix timestamp for Prometheus queries
        let spawn_unix_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

        // Build client arguments based on database type
        let mut client_args = vec!["-c".to_string(), self.config_path.clone()];

        match self.config.database.kind {
            DatabaseType::PostgreSQL => {
                client_args.extend([
                    "--pg-host".to_string(),
                    "localhost".to_string(),
                    "--pg-port".to_string(),
                    "5432".to_string(),
                ]);
            }
            DatabaseType::TigerBeetle => {
                let tb_config = self
                    .config
                    .tigerbeetle
                    .as_ref()
                    .context("TigerBeetle config missing")?;
                client_args.extend([
                    "--tb-addresses".to_string(),
                    tb_config.cluster_addresses.join(","),
                ]);
            }
        }

        client_args.extend([
            "--otel-endpoint".to_string(),
            "http://localhost:4317".to_string(),
        ]);

        // Create log file for client output
        let log_file = std::fs::File::create(&self.run_ctx.client_log_path)
            .context("Failed to create client log file")?;
        let log_file_err = log_file.try_clone().context("Failed to clone log file")?;

        // Spawn the client with output redirected to log file
        let mut child = Command::new(&client_binary)
            .args(&client_args)
            .stdout(Stdio::from(log_file))
            .stderr(Stdio::from(log_file_err))
            .spawn()
            .context("Failed to spawn client binary")?;

        info!("Client started, waiting for completion...");

        // Wait for client to complete (with timeout)
        let timeout = Duration::from_secs(total_duration + 60); // Add buffer
        let result = tokio::time::timeout(timeout, child.wait()).await;

        let elapsed = start_time.elapsed();

        let client_success = match result {
            Ok(Ok(status)) => {
                if status.success() {
                    info!("Client completed successfully in {:?}", elapsed);
                    true
                } else {
                    error!("Client exited with status: {}", status);
                    // Log hint for common exit codes
                    if let Some(code) = status.code() {
                        match code {
                            126 => error!(
                                "Exit code 126: Permission denied or binary not executable. \
                                Check that {} has execute permission.",
                                client_binary
                            ),
                            127 => error!(
                                "Exit code 127: Command not found. \
                                Check that {} exists.",
                                client_binary
                            ),
                            _ => {}
                        }
                    }
                    false
                }
            }
            Ok(Err(e)) => {
                error!("Client error: {:?}", e);
                false
            }
            Err(_) => {
                error!("Client timed out after {:?}", timeout);
                // Kill the process on timeout
                let _ = child.kill().await;
                false
            }
        };

        // Wait for metrics to be available in Prometheus
        // OTel collector flushes every 5s, Prometheus scrapes every 5s
        info!("Waiting for metrics to be available...");
        tokio::time::sleep(Duration::from_secs(15)).await;

        // Calculate measurement window start (after warmup)
        let measurement_start = spawn_unix_time + warmup_duration as f64;

        // Query metrics from Prometheus for the measurement window
        let metrics = match self
            .prometheus
            .collect_metrics(measurement_start)
            .await
        {
            Ok(m) => {
                info!(
                    "Collected metrics: completed={}, rejected={}, failed={}",
                    m.completed_transfers, m.rejected_transfers, m.failed_transfers
                );
                m
            }
            Err(e) => {
                warn!("Failed to collect metrics: {:?}", e);
                crate::prometheus::CollectedMetrics::default()
            }
        };

        let total_transfers = metrics.completed_transfers + metrics.rejected_transfers;
        let throughput_tps = if test_duration > 0 {
            total_transfers as f64 / test_duration as f64
        } else {
            0.0
        };

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
            balance_verified: client_success,
        })
    }
}

/// Find the client binary
fn find_client_binary() -> Result<String> {
    // Check common locations - prefer release builds
    let candidates = [
        "./target/release/client",
        "target/release/client",
        "./target/debug/client",
        "target/debug/client",
    ];

    for candidate in candidates {
        if Path::new(candidate).exists() {
            return Ok(candidate.to_string());
        }
    }

    anyhow::bail!(
        "Client binary not found. Please run 'cargo build --release' first. Checked: {:?}",
        candidates
    )
}

/// Validate that the client binary is executable
fn validate_client_binary(path: &str) -> Result<()> {
    let path = Path::new(path);

    // Check if file exists
    if !path.exists() {
        anyhow::bail!("Client binary does not exist: {}", path.display());
    }

    // Check if it's a file (not a directory)
    if !path.is_file() {
        anyhow::bail!("Client binary path is not a file: {}", path.display());
    }

    // Check execute permission
    let metadata = fs::metadata(path).context("Failed to read binary metadata")?;
    let permissions = metadata.permissions();
    let mode = permissions.mode();

    if mode & 0o111 == 0 {
        anyhow::bail!(
            "Client binary is not executable: {}. Run: chmod +x {}",
            path.display(),
            path.display()
        );
    }

    // Log binary info for debugging
    debug!(
        "Client binary validated: {} (size: {} bytes, mode: {:o})",
        path.display(),
        metadata.len(),
        mode
    );

    Ok(())
}
