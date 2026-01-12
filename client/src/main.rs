use anyhow::{Context, Result};
use clap::Parser;
use tb_perf_common::Config;
use tb_perf_common::config::{DatabaseType, TestMode};
use tracing::info;

mod metrics;
mod postgres;
mod tigerbeetle;
mod workload;

use metrics::WorkloadMetrics;
use postgres::PostgresWorkload;
use tigerbeetle::TigerBeetleWorkload;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to configuration file
    #[arg(short, long, default_value = "config.toml")]
    config: String,

    /// Client instance ID (for multi-client cloud deployments)
    #[arg(long, default_value = "0")]
    instance_id: usize,

    /// PostgreSQL host (can be overridden from config)
    #[arg(long, default_value = "localhost")]
    pg_host: String,

    /// PostgreSQL port
    #[arg(long, default_value = "5432")]
    pg_port: u16,

    /// TigerBeetle cluster addresses (comma-separated)
    #[arg(long, default_value = "3000")]
    tb_addresses: String,

    /// OpenTelemetry collector endpoint
    #[arg(long, default_value = "http://localhost:4317")]
    otel_endpoint: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let args = Args::parse();
    info!("Client instance {} starting", args.instance_id);
    info!("Loading configuration from: {}", args.config);

    // Load configuration
    let config = Config::from_file(&args.config)?;
    info!("Configuration loaded successfully");
    info!("Database type: {:?}", config.database.kind);

    let test_mode = config.workload.test_mode()?;
    info!("Test mode: {:?}", test_mode);

    // Client executes the workload
    match config.database.kind {
        DatabaseType::PostgreSQL => {
            run_postgresql_workload(&config, &args).await?;
        }
        DatabaseType::TigerBeetle => {
            run_tigerbeetle_workload(&config, &args).await?;
        }
    }

    info!("Client instance {} finished", args.instance_id);
    Ok(())
}

async fn run_postgresql_workload(config: &Config, args: &Args) -> Result<()> {
    info!("Running PostgreSQL workload");

    let pg_config = config
        .postgresql
        .as_ref()
        .context("PostgreSQL config missing (should have been validated)")?;

    info!("  Isolation level: {:?}", pg_config.isolation_level);
    info!("  Connection pool size: {}", pg_config.connection_pool_size);

    let test_mode = config.workload.test_mode()?;
    let test_mode_str = test_mode.as_str();

    // Initialize metrics
    let metrics = WorkloadMetrics::new(&args.otel_endpoint, "postgresql", test_mode_str)
        .context("Failed to initialize metrics")?;
    let metrics_for_shutdown = metrics.clone();

    // Create workload executor
    let workload = PostgresWorkload::new(
        pg_config,
        &args.pg_host,
        args.pg_port,
        "tbperf",
        "postgres",
        "postgres",
        config.workload.num_accounts,
        config.workload.zipfian_exponent,
        config.workload.min_transfer_amount,
        config.workload.max_transfer_amount,
        config.workload.warmup_duration_secs,
        config.workload.test_duration_secs,
        metrics,
    )
    .await
    .context("Failed to create PostgreSQL workload")?;

    // Run workload based on test mode
    let result = match test_mode {
        TestMode::MaxThroughput { concurrency } => workload.run_max_throughput(concurrency).await,
        TestMode::FixedRate {
            target_rate,
            max_concurrency,
        } => workload.run_fixed_rate(target_rate, max_concurrency).await,
    };

    // Shutdown OpenTelemetry provider to flush remaining metrics
    info!("Shutting down metrics provider...");
    metrics_for_shutdown.shutdown();

    result
}

async fn run_tigerbeetle_workload(config: &Config, args: &Args) -> Result<()> {
    info!("Running TigerBeetle workload");

    let tb_config = config
        .tigerbeetle
        .as_ref()
        .context("TigerBeetle config missing (should have been validated)")?;

    // Use addresses from args (allows coordinator to override config)
    let addresses: Vec<String> = args
        .tb_addresses
        .split(',')
        .map(|s| s.to_string())
        .collect();
    info!("  Cluster addresses: {:?}", addresses);
    info!("  Measure batch sizes: {}", tb_config.measure_batch_sizes);

    let test_mode = config.workload.test_mode()?;
    let test_mode_str = test_mode.as_str();

    // Initialize metrics
    let metrics = WorkloadMetrics::new(&args.otel_endpoint, "tigerbeetle", test_mode_str)
        .context("Failed to initialize metrics")?;
    let metrics_for_shutdown = metrics.clone();

    // Create workload executor
    let workload = TigerBeetleWorkload::new(
        &addresses,
        config.workload.num_accounts,
        config.workload.zipfian_exponent,
        config.workload.min_transfer_amount,
        config.workload.max_transfer_amount,
        config.workload.warmup_duration_secs,
        config.workload.test_duration_secs,
        tb_config.measure_batch_sizes,
        metrics,
    )
    .await
    .context("Failed to create TigerBeetle workload")?;

    // Run workload based on test mode
    let result = match test_mode {
        TestMode::MaxThroughput { concurrency } => workload.run_max_throughput(concurrency).await,
        TestMode::FixedRate {
            target_rate,
            max_concurrency,
        } => workload.run_fixed_rate(target_rate, max_concurrency).await,
    };

    // Shutdown OpenTelemetry provider to flush remaining metrics
    info!("Shutting down metrics provider...");
    metrics_for_shutdown.shutdown();

    result
}
