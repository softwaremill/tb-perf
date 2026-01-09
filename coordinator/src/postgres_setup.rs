use crate::docker::DockerManager;
use anyhow::{Context, Result};
use tracing::info;

/// Initialize accounts in PostgreSQL
pub async fn init_accounts(
    docker: &DockerManager,
    num_accounts: u64,
    initial_balance: u64,
) -> Result<()> {
    info!(
        "Initializing {} accounts with balance {}",
        num_accounts, initial_balance
    );

    // Clear existing data and insert accounts
    let sql = format!(
        "TRUNCATE transfers, accounts CASCADE; \
         INSERT INTO accounts (id, balance) \
         SELECT generate_series(0, {} - 1), {}",
        num_accounts, initial_balance
    );

    docker
        .exec_postgres(&sql)
        .await
        .context("Failed to initialize accounts")?;

    info!("Accounts initialized successfully");
    Ok(())
}

/// Reset database between test runs (truncate and re-initialize)
pub async fn reset_database(
    docker: &DockerManager,
    num_accounts: u64,
    initial_balance: u64,
) -> Result<()> {
    info!("Resetting database for next run");
    init_accounts(docker, num_accounts, initial_balance).await
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
pub async fn vacuum_analyze(docker: &DockerManager) -> Result<()> {
    info!("Running VACUUM ANALYZE...");

    docker
        .exec_postgres("VACUUM ANALYZE")
        .await
        .context("Failed to run VACUUM ANALYZE")?;

    info!("VACUUM ANALYZE completed");
    Ok(())
}

/// Run CHECKPOINT to flush WAL
pub async fn checkpoint(docker: &DockerManager) -> Result<()> {
    info!("Running CHECKPOINT...");

    docker
        .exec_postgres("CHECKPOINT")
        .await
        .context("Failed to run CHECKPOINT")?;

    info!("CHECKPOINT completed");
    Ok(())
}
