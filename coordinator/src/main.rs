use anyhow::Result;
use clap::Parser;
use std::time::Duration;
use tb_perf_common::Config;
use tb_perf_common::config::{DatabaseType, DeploymentType};
use tracing::{info, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod docker;
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
            run_cloud_tests(&config).await?;
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
    let runner = TestRunner::new(config.clone(), args.config.clone(), docker.clone(), run_ctx);
    let results = runner.run().await?;

    // Print and export results
    results.print_summary();
    results.export_json(run_ctx.results_path().to_str().unwrap())?;

    // Save docker logs directly to file
    if !args.no_docker
        && let Err(e) = docker.save_logs_to_file(&run_ctx.docker_log_path).await
    {
        warn!("Failed to save docker logs: {:?}", e);
    }

    // Cleanup
    let keep_running = args.keep_running || config.coordinator.keep_grafana_running;
    if !keep_running && !args.no_docker {
        docker.stop().await?;
    } else {
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

    Ok(())
}

async fn run_cloud_tests(config: &Config) -> Result<()> {
    info!("Running cloud tests");
    info!("  Region: {:?}", config.deployment.aws_region);
    info!("  DB nodes: {}", config.deployment.num_db_nodes);
    info!("  Client nodes: {:?}", config.deployment.num_client_nodes);

    // TODO: Implement cloud test orchestration
    // 1. Verify infrastructure is provisioned
    // 2. Deploy client binaries
    // 3. Initialize database cluster
    // 4. Coordinate multi-client test execution
    // 5. Aggregate results from all clients
    // 6. Download results to laptop

    warn!("Cloud test orchestration not yet implemented");

    Ok(())
}
