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
    // Initialize tracing (no ANSI codes since output is captured to file)
    tracing_subscriber::fmt()
        .with_ansi(false)
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
    info!("  Batched mode: {}", pg_config.batched_mode);
    if !pg_config.batched_mode {
        info!("  Connection pool size: {}", pg_config.connection_pool_size);
    }

    let test_mode = config.workload.test_mode()?;
    let test_mode_str = test_mode.as_str();

    // Initialize metrics
    let db_type_label = if pg_config.batched_mode {
        "postgresql_batched"
    } else {
        "postgresql"
    };
    let metrics = WorkloadMetrics::new(&args.otel_endpoint, db_type_label, test_mode_str)
        .context("Failed to initialize metrics")?;
    let metrics_for_shutdown = metrics.clone();

    // Run workload based on batched mode
    let result = if pg_config.batched_mode {
        run_batched_postgresql_workload(config, args, pg_config, &test_mode, metrics).await
    } else {
        run_standard_postgresql_workload(config, args, pg_config, &test_mode, metrics).await
    };

    // Shutdown OpenTelemetry provider to flush remaining metrics
    info!("Shutting down metrics provider...");
    metrics_for_shutdown.shutdown();

    result
}

async fn run_standard_postgresql_workload(
    config: &Config,
    args: &Args,
    pg_config: &tb_perf_common::config::PostgresqlConfig,
    test_mode: &TestMode,
    metrics: WorkloadMetrics,
) -> Result<()> {
    // Create standard workload runner
    let workload = postgres::create_workload(
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
    match test_mode {
        TestMode::MaxThroughput { concurrency } => workload.run_max_throughput(*concurrency).await,
        TestMode::FixedRate {
            target_rate,
            max_concurrency,
        } => {
            workload
                .run_fixed_rate(*target_rate, *max_concurrency)
                .await
        }
    }
}

async fn run_batched_postgresql_workload(
    config: &Config,
    args: &Args,
    pg_config: &tb_perf_common::config::PostgresqlConfig,
    test_mode: &TestMode,
    metrics: WorkloadMetrics,
) -> Result<()> {
    // Create batched workload runner
    let workload = postgres::create_batched_workload(
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
    .context("Failed to create batched PostgreSQL workload")?;

    // Run workload based on test mode
    match test_mode {
        TestMode::MaxThroughput { concurrency } => workload.run_max_throughput(*concurrency).await,
        TestMode::FixedRate {
            target_rate,
            max_concurrency,
        } => {
            workload
                .run_fixed_rate(*target_rate, *max_concurrency)
                .await
        }
    }
}

async fn run_tigerbeetle_workload(config: &Config, args: &Args) -> Result<()> {
    info!("Running TigerBeetle workload");

    // Validate TigerBeetle config exists (addresses come from args)
    let _ = config
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

    let test_mode = config.workload.test_mode()?;
    let test_mode_str = test_mode.as_str();

    // Initialize metrics
    let metrics = WorkloadMetrics::new(&args.otel_endpoint, "tigerbeetle", test_mode_str)
        .context("Failed to initialize metrics")?;
    let metrics_for_shutdown = metrics.clone();

    // Create workload runner
    let workload = tigerbeetle::create_workload(
        &addresses,
        config.workload.num_accounts,
        config.workload.zipfian_exponent,
        config.workload.min_transfer_amount,
        config.workload.max_transfer_amount,
        config.workload.warmup_duration_secs,
        config.workload.test_duration_secs,
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
