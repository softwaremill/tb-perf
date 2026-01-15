use crate::docker::DockerManager;
use anyhow::{Context, Result};
use tracing::info;

/// Initialize PostgreSQL schema by running the init script.
/// This creates tables and stored procedures.
/// Safe to run multiple times (uses CREATE TABLE IF NOT EXISTS and CREATE OR REPLACE).
pub async fn init_schema(docker: &DockerManager) -> Result<()> {
    info!("Initializing PostgreSQL schema from init-postgresql.sql...");

    docker
        .exec_postgres_file("/docker-entrypoint-initdb.d/init.sql")
        .await
        .context("Failed to run init-postgresql.sql")?;

    info!("PostgreSQL schema initialized");
    Ok(())
}

/// Reset database to initial state with consistent conditions.
/// Used for both initial setup and between-run resets.
/// Ensures each run starts with identical conditions (checkpoint + vacuum analyze).
pub async fn reset_database(
    docker: &DockerManager,
    num_accounts: u64,
    initial_balance: u64,
) -> Result<()> {
    info!(
        "Resetting database: {} accounts with balance {}",
        num_accounts, initial_balance
    );

    // Truncate and re-initialize accounts (1-based IDs to match AccountSelector)
    let sql = format!(
        "TRUNCATE transfers, accounts CASCADE; \
         INSERT INTO accounts (id, balance) \
         SELECT generate_series(1, {}), {}",
        num_accounts, initial_balance
    );

    docker
        .exec_postgres(&sql)
        .await
        .context("Failed to reset accounts")?;

    // Flush WAL and update statistics for consistent conditions
    checkpoint(docker).await?;
    vacuum_analyze(docker).await?;

    info!("Database reset complete");
    Ok(())
}

/// Verify total balance for correctness checking
pub async fn verify_total_balance(docker: &DockerManager, expected_total: u64) -> Result<bool> {
    info!("Verifying total balance (expected: {})", expected_total);

    let output = docker
        .exec_postgres("SELECT SUM(balance) FROM accounts")
        .await
        .context("Failed to verify total balance")?;

    // Parse the output (psql -t returns tuples-only, just the value with whitespace)
    let actual_total: u64 = output
        .trim()
        .parse()
        .context("Failed to parse total balance")?;

    let is_correct = actual_total == expected_total;
    if is_correct {
        info!("Balance verification passed: {}", actual_total);
    } else {
        tracing::error!(
            "Balance verification FAILED: expected {}, got {}",
            expected_total,
            actual_total
        );
    }

    Ok(is_correct)
}

/// Run VACUUM ANALYZE for optimal performance
async fn vacuum_analyze(docker: &DockerManager) -> Result<()> {
    info!("Running VACUUM ANALYZE...");

    docker
        .exec_postgres("VACUUM ANALYZE")
        .await
        .context("Failed to run VACUUM ANALYZE")?;

    info!("VACUUM ANALYZE completed");
    Ok(())
}

/// Run CHECKPOINT to flush WAL
async fn checkpoint(docker: &DockerManager) -> Result<()> {
    info!("Running CHECKPOINT...");

    docker
        .exec_postgres("CHECKPOINT")
        .await
        .context("Failed to run CHECKPOINT")?;

    info!("CHECKPOINT completed");
    Ok(())
}
