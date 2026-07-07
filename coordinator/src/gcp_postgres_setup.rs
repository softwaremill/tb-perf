use anyhow::{Context, Result};
use std::fs;
use std::time::{Duration, Instant};
use tokio_postgres::{Client, NoTls};
use tracing::{debug, info};

/// Connect directly to a remote PostgreSQL primary over its external IP
/// (see terraform/network's `allow_operator_to_db` firewall rule - this is
/// the cloud equivalent of the local `docker exec`-based connection in
/// `postgres_setup.rs`).
async fn connect(host: &str) -> Result<Client> {
    let conn_str = format!(
        "host={host} port=5432 user=postgres password=postgres dbname=tbperf \
         sslmode=disable connect_timeout=10"
    );

    let (client, connection) = tokio_postgres::connect(&conn_str, NoTls)
        .await
        .with_context(|| format!("Failed to connect to PostgreSQL at {}", host))?;

    // The connection object performs the actual I/O; it must be driven by a
    // background task or the client will never make progress.
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            tracing::warn!("PostgreSQL connection error: {:?}", e);
        }
    });

    Ok(client)
}

/// Retry connecting until the primary is reachable, or `timeout_secs` elapses.
/// Useful right after cluster setup, since the external IP / container may
/// take a moment to become reachable.
pub async fn wait_for_ready(host: &str, timeout_secs: u64) -> Result<Client> {
    info!("Waiting for PostgreSQL to be ready at {}...", host);

    let start = Instant::now();
    let timeout = Duration::from_secs(timeout_secs);

    loop {
        match connect(host).await {
            Ok(client) => {
                info!("PostgreSQL is ready at {}", host);
                return Ok(client);
            }
            Err(e) => {
                if start.elapsed() > timeout {
                    return Err(e).context("Timed out waiting for PostgreSQL to become reachable");
                }
                debug!("PostgreSQL not ready yet at {}: {:?}", host, e);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

/// Initialize PostgreSQL schema (tables + stored procedures). Unlike the
/// local Docker Compose setup (which mounts scripts/init-postgresql.sql via
/// docker-entrypoint-initdb.d), the cloud primary isn't bootstrapped with
/// this automatically, so it's executed explicitly here. Safe to run
/// multiple times (uses CREATE TABLE IF NOT EXISTS / CREATE OR REPLACE).
pub async fn init_schema(client: &Client) -> Result<()> {
    info!("Initializing PostgreSQL schema from init-postgresql.sql...");

    let sql = fs::read_to_string("scripts/init-postgresql.sql")
        .context("Failed to read scripts/init-postgresql.sql")?;

    client
        .batch_execute(&sql)
        .await
        .context("Failed to execute init-postgresql.sql")?;

    info!("PostgreSQL schema initialized");
    Ok(())
}

/// Reset database to initial state with consistent conditions. Mirrors
/// `postgres_setup::reset_database`, just issued over a direct network
/// connection instead of `docker exec`.
pub async fn reset_database(
    client: &Client,
    num_accounts: u64,
    initial_balance: u64,
) -> Result<()> {
    info!(
        "Resetting database: {} accounts with balance {}",
        num_accounts, initial_balance
    );

    let sql = format!(
        "TRUNCATE transfers, accounts CASCADE; \
         INSERT INTO accounts (id, balance) \
         SELECT generate_series(1, {}), {}",
        num_accounts, initial_balance
    );

    client
        .batch_execute(&sql)
        .await
        .context("Failed to reset accounts")?;

    client
        .batch_execute("CHECKPOINT")
        .await
        .context("Failed to run CHECKPOINT")?;

    client
        .batch_execute("VACUUM ANALYZE")
        .await
        .context("Failed to run VACUUM ANALYZE")?;

    info!("Database reset complete");
    Ok(())
}

/// Verify total balance for correctness checking.
pub async fn verify_total_balance(client: &Client, expected_total: u64) -> Result<bool> {
    info!("Verifying total balance (expected: {})", expected_total);

    // Cast to BIGINT explicitly: SUM(bigint) returns NUMERIC in PostgreSQL,
    // which doesn't map directly to i64.
    let row = client
        .query_one("SELECT SUM(balance)::BIGINT FROM accounts", &[])
        .await
        .context("Failed to verify total balance")?;

    let actual_total: i64 = row.get(0);
    let is_correct = actual_total as u64 == expected_total;

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
