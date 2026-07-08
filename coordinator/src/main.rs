use anyhow::{Context, Result};
use clap::Parser;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tb_perf_common::Config;
use tb_perf_common::config::{DatabaseType, DeploymentType};
use tracing::{error, info, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod client_deploy;
mod docker;
mod gcp;
mod gcp_monitoring;
mod gcp_postgres_setup;
mod gcp_setup;
mod gcp_workload;
mod postgres_setup;
mod prometheus;
mod results;
mod run_context;
mod test_runner;
mod tigerbeetle_setup;

use docker::{DockerManager, find_compose_file};
use run_context::RunContext;
use test_runner::TestRunner;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to configuration file
    #[arg(short, long, default_value = "config.toml")]
    config: String,

    /// Skip starting Docker Compose (assume already running)
    #[arg(long)]
    no_docker: bool,

    /// Keep infrastructure running after test
    #[arg(long)]
    keep_running: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Load configuration first (before logging setup, so we know the output path)
    let config = Config::from_file(&args.config)?;

    // Create run context with dedicated directory for this run's logs
    let run_ctx = RunContext::new(&config.coordinator.metrics_export_path)?;

    // Set up dual logging: file + stdout
    let file = std::fs::File::create(run_ctx.coordinator_log_path())?;
    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(file);

    let stdout_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stdout);

    let filter = tracing_subscriber::EnvFilter::from_default_env()
        .add_directive(tracing::Level::INFO.into());

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(stdout_layer)
        .init();

    info!("Run directory: {}", run_ctx.run_dir.display());
    info!("Loading configuration from: {}", args.config);

    // Copy config to run directory
    run_ctx.copy_config(&args.config)?;

    info!("Configuration loaded successfully");
    info!("Deployment type: {:?}", config.deployment.kind);
    info!("Database type: {:?}", config.database.kind);
    info!("Test mode: {:?}", config.workload.test_mode()?);

    // Coordinator orchestrates the test execution
    info!("Starting test coordinator...");

    match config.deployment.kind {
        DeploymentType::Local => {
            run_local_tests(&config, &args, &run_ctx).await?;
        }
        DeploymentType::Cloud => {
            run_cloud_tests(&config, &args.config, &run_ctx).await?;
        }
    }

    info!("Test coordinator finished");
    info!("Results saved to: {}", run_ctx.run_dir.display());
    Ok(())
}

async fn run_local_tests(config: &Config, args: &Args, run_ctx: &RunContext) -> Result<()> {
    info!("Running local tests");
    info!("  Test runs: {}", config.coordinator.test_runs);
    info!(
        "  Warmup duration: {}s",
        config.workload.warmup_duration_secs
    );
    info!("  Test duration: {}s", config.workload.test_duration_secs);

    // Find docker compose file
    let db_type = format!("{:?}", config.database.kind).to_lowercase();
    let compose_file = find_compose_file(&args.config, &db_type)?;
    info!("Using docker compose file: {}", compose_file);

    let docker = DockerManager::new(&compose_file, "tbperf");

    // Run the test with cleanup guard to always save logs
    let test_result = run_local_tests_inner(config, args, run_ctx, &docker).await;

    // Always save docker logs, regardless of test result
    if !args.no_docker {
        if let Err(e) = docker.save_logs_to_file(&run_ctx.docker_log_path).await {
            warn!("Failed to save docker logs: {:?}", e);
        } else {
            info!(
                "Docker logs saved to: {}",
                run_ctx.docker_log_path.display()
            );
        }
    }

    // Cleanup - stop docker unless keeping running
    let keep_running = args.keep_running || config.coordinator.keep_grafana_running;
    if !keep_running && !args.no_docker {
        if let Err(e) = docker.stop().await {
            warn!("Failed to stop Docker: {:?}", e);
        }
    } else if !args.no_docker {
        info!("Keeping infrastructure running");
        info!(
            "  Grafana: http://localhost:{}",
            config.monitoring.grafana_port
        );
        info!(
            "  Prometheus: http://localhost:{}",
            config.monitoring.prometheus_port
        );
    }

    // Return the actual test result
    test_result
}

async fn run_local_tests_inner(
    config: &Config,
    args: &Args,
    run_ctx: &RunContext,
    docker: &DockerManager,
) -> Result<()> {
    // Start infrastructure
    if !args.no_docker {
        docker.start().await?;

        // Wait for database-specific services
        match config.database.kind {
            DatabaseType::PostgreSQL => {
                docker
                    .wait_for_postgres_services(Duration::from_secs(60))
                    .await?;
            }
            DatabaseType::TigerBeetle => {
                docker
                    .wait_for_tigerbeetle_services(Duration::from_secs(60))
                    .await?;
            }
        }
    } else {
        info!("Skipping Docker start (--no-docker flag)");
    }

    // Run tests
    let runner = TestRunner::new(
        config.clone(),
        args.config.clone(),
        docker.clone(),
        run_ctx,
        args.no_docker,
    );
    let results = runner.run().await?;

    // Print and export results
    results.print_summary();
    results.export_json(run_ctx.results_path().to_str().unwrap())?;

    Ok(())
}

async fn run_cloud_tests(config: &Config, config_path: &str, run_ctx: &RunContext) -> Result<()> {
    info!("Running cloud tests");
    info!("  Project: {:?}", config.deployment.gcp_project);
    info!("  Region: {:?}", config.deployment.gcp_region);
    info!("  DB nodes: {}", config.deployment.num_db_nodes);
    info!("  Client nodes: {:?}", config.deployment.num_client_nodes);

    let project = config
        .deployment
        .gcp_project
        .as_ref()
        .context("Cloud deployment requires deployment.gcp_project")?;
    let remote = gcp::GcpRemote::new(project);

    // 1. Discover already-provisioned DB nodes (terraform/database-cluster
    //    must have been applied beforehand - this coordinator does not run
    //    `terraform apply` itself, see PLAN.md §3.1/§3.4).
    let db_type_str = format!("{:?}", config.database.kind).to_lowercase();
    let db_nodes = gcp_setup::discover_db_nodes(&remote, &db_type_str).await?;
    info!(
        "Discovered {} DB node(s) for database_type={}",
        db_nodes.len(),
        db_type_str
    );

    if db_nodes.len() != config.deployment.num_db_nodes {
        warn!(
            "Discovered {} DB nodes but config.deployment.num_db_nodes = {} - continuing with what was found",
            db_nodes.len(),
            config.deployment.num_db_nodes
        );
    }

    // 2. Bring up the DB cluster (one-time per test session - wipes any
    //    existing data on those nodes).
    match config.database.kind {
        DatabaseType::PostgreSQL => {
            gcp_setup::setup_postgresql_cluster(&remote, &db_nodes).await?;
            gcp_setup::verify_postgresql_cluster(&remote, &db_nodes).await?;
        }
        DatabaseType::TigerBeetle => {
            gcp_setup::setup_tigerbeetle_cluster(&remote, &db_nodes).await?;
        }
    }

    // 3. Bring up the observability stack (OTel Collector + Prometheus +
    //    Grafana) on the monitoring instance - idempotent, does not wipe
    //    existing Prometheus data.
    let monitoring_node = gcp_monitoring::discover_monitoring_node(&remote).await?;
    gcp_monitoring::deploy_monitoring_stack(&remote, &monitoring_node).await?;

    // 4. Discover already-provisioned client nodes, build and deploy client binary
    let client_nodes = client_deploy::discover_client_nodes(&remote).await?;
    info!("Discovered {} client node(s)", client_nodes.len());

    if let Some(expected) = config.deployment.num_client_nodes
        && client_nodes.len() != expected
    {
        warn!(
            "Discovered {} client nodes but config.deployment.num_client_nodes = {} - continuing with what was found",
            client_nodes.len(),
            expected
        );
    }

    client_deploy::deploy_client_binary(&remote, &client_nodes).await?;

    // 5. Initialize accounts against the remote cluster (over its external
    //    IP - the coordinator runs outside the VPC, see terraform/network's
    //    `allow_operator_to_db` firewall rule).
    let num_accounts = config.workload.num_accounts;
    let initial_balance = config.workload.initial_balance;

    match config.database.kind {
        DatabaseType::PostgreSQL => {
            let primary = db_nodes
                .first()
                .context("No PostgreSQL primary discovered")?;
            let pg_client = gcp_postgres_setup::wait_for_ready(&primary.external_ip, 60).await?;
            gcp_postgres_setup::init_schema(&pg_client).await?;
            gcp_postgres_setup::reset_database(&pg_client, num_accounts, initial_balance).await?;
        }
        DatabaseType::TigerBeetle => {
            let cluster_addresses: Vec<String> = db_nodes
                .iter()
                .map(|n| format!("{}:3000", n.external_ip))
                .collect();
            tigerbeetle_setup::init_accounts(&cluster_addresses, num_accounts, initial_balance)
                .await?;
        }
    }

    // 6. Deploy config to client nodes and run the workload (using
    //    internal IPs throughout - client/DB/monitoring nodes share a VPC).
    let db_args = match config.database.kind {
        DatabaseType::PostgreSQL => {
            let primary = db_nodes
                .first()
                .context("No PostgreSQL primary discovered")?;
            format!("--pg-host {} --pg-port 5432", primary.internal_ip)
        }
        DatabaseType::TigerBeetle => {
            let addresses: Vec<String> = db_nodes
                .iter()
                .map(|n| format!("{}:3000", n.internal_ip))
                .collect();
            format!("--tb-addresses {}", addresses.join(","))
        }
    };
    let otel_endpoint = format!("http://{}:4317", monitoring_node.internal_ip);

    gcp_workload::deploy_config(&remote, &client_nodes, config_path).await?;

    // 7. Run the workload `coordinator.test_runs` times, resetting the
    //    database between runs (except after the last one).
    let num_runs = config.coordinator.test_runs;
    let mut test_results = results::TestResults::new(config.clone(), num_runs);

    for run_id in 1..=num_runs {
        info!("=== Starting run {}/{} ===", run_id, num_runs);

        let run_result = run_single_cloud_test(
            run_id,
            &remote,
            &client_nodes,
            &db_nodes,
            &monitoring_node,
            config,
            &db_args,
            &otel_endpoint,
        )
        .await?;

        let balance_ok = run_result.balance_verified;
        test_results.add_run(run_result);
        if !balance_ok {
            error!("Balance verification failed for run {}", run_id);
            test_results.set_balance_error(run_id);
        }

        if run_id < num_runs {
            info!("Resetting database for next run...");
            match config.database.kind {
                DatabaseType::PostgreSQL => {
                    let primary = db_nodes
                        .first()
                        .context("No PostgreSQL primary discovered")?;
                    let pg_client =
                        gcp_postgres_setup::wait_for_ready(&primary.external_ip, 30).await?;
                    gcp_postgres_setup::reset_database(&pg_client, num_accounts, initial_balance)
                        .await?;
                }
                DatabaseType::TigerBeetle => {
                    gcp_setup::reset_tigerbeetle_cluster(&remote, &db_nodes).await?;
                    let cluster_addresses: Vec<String> = db_nodes
                        .iter()
                        .map(|n| format!("{}:3000", n.external_ip))
                        .collect();
                    tigerbeetle_setup::wait_for_ready(&cluster_addresses, 60).await?;
                    tigerbeetle_setup::init_accounts(
                        &cluster_addresses,
                        num_accounts,
                        initial_balance,
                    )
                    .await?;
                }
            }

            info!("Waiting 30s for system stabilization...");
            tokio::time::sleep(Duration::from_secs(30)).await;
        }

        info!("=== Completed run {}/{} ===", run_id, num_runs);
    }

    test_results.calculate_aggregates();
    test_results.print_summary();
    test_results.export_json(run_ctx.results_path().to_str().unwrap())?;

    info!("Cloud test run complete");

    Ok(())
}

/// Run a single measured iteration: run the client workload, verify the
/// double-entry balance invariant, and collect aggregated metrics from
/// Prometheus. Mirrors `test_runner.rs`'s local `run_single_test`, adapted
/// for remote nodes and Prometheus reachable over its external IP.
#[allow(clippy::too_many_arguments)]
async fn run_single_cloud_test(
    run_id: usize,
    remote: &gcp::GcpRemote,
    client_nodes: &[client_deploy::ClientNode],
    db_nodes: &[gcp_setup::DbNode],
    monitoring_node: &gcp_monitoring::MonitoringNode,
    config: &Config,
    db_args: &str,
    otel_endpoint: &str,
) -> Result<results::RunResult> {
    let warmup_duration = config.workload.warmup_duration_secs;
    let test_duration = config.workload.test_duration_secs;
    let num_accounts = config.workload.num_accounts;
    let initial_balance = config.workload.initial_balance;

    let spawn_unix_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let start_time = Instant::now();

    gcp_workload::run_workload(
        remote,
        client_nodes,
        db_args,
        otel_endpoint,
        warmup_duration,
        test_duration,
    )
    .await?;

    let elapsed = start_time.elapsed();

    // Verify balance correctness (double-entry invariant: total across all
    // accounts must be unchanged).
    let expected_total = num_accounts * initial_balance;
    let balance_ok = match config.database.kind {
        DatabaseType::PostgreSQL => {
            let primary = db_nodes
                .first()
                .context("No PostgreSQL primary discovered")?;
            let pg_client = gcp_postgres_setup::wait_for_ready(&primary.external_ip, 30).await?;
            gcp_postgres_setup::verify_total_balance(&pg_client, expected_total).await?
        }
        DatabaseType::TigerBeetle => {
            let cluster_addresses: Vec<String> = db_nodes
                .iter()
                .map(|n| format!("{}:3000", n.external_ip))
                .collect();
            tigerbeetle_setup::verify_total_balance(
                &cluster_addresses,
                num_accounts,
                expected_total,
            )
            .await?
        }
    };

    // Collect aggregated metrics from Prometheus (over its external IP - see
    // terraform/network's `allow_operator_to_monitoring` rule). Wait first
    // for the OTel collector to flush and Prometheus to scrape.
    info!("Waiting for metrics to be available...");
    tokio::time::sleep(Duration::from_secs(15)).await;

    let measurement_start = spawn_unix_time + warmup_duration as f64;
    let prometheus_url = format!(
        "http://{}:{}",
        monitoring_node.external_ip, config.monitoring.prometheus_port
    );
    let prometheus_client = prometheus::PrometheusClient::new(&prometheus_url);
    let metrics = match prometheus_client.collect_metrics(measurement_start).await {
        Ok(m) => m,
        Err(e) => {
            warn!("Failed to collect metrics: {:?}", e);
            prometheus::CollectedMetrics::default()
        }
    };

    let total_transfers = metrics.completed_transfers + metrics.rejected_transfers;
    let throughput_tps = if test_duration > 0 {
        total_transfers as f64 / test_duration as f64
    } else {
        0.0
    };

    Ok(results::RunResult {
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
        balance_verified: balance_ok,
    })
}
