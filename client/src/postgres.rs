use crate::metrics::WorkloadMetrics;
use crate::workload::{TransferExecutor, TransferResult, WorkloadRunner};
use anyhow::{Context, Result};
use async_trait::async_trait;
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use std::time::Duration;
use tb_perf_common::config::{IsolationLevel, PostgresqlConfig};
use tokio_postgres::NoTls;
use tokio_postgres::error::SqlState;
use tracing::{debug, error, info, warn};

/// Maximum number of retries for serialization failures
const MAX_RETRIES: u32 = 5;
/// Base delay for exponential backoff (milliseconds)
const BASE_RETRY_DELAY_MS: u64 = 10;

/// SQL return value constants (must match init-postgresql.sql)
mod sql_results {
    pub const SUCCESS: &str = "success";
    pub const INSUFFICIENT_BALANCE: &str = "insufficient_balance";
    pub const ACCOUNT_NOT_FOUND: &str = "account_not_found";
}

/// PostgreSQL transfer executor
///
/// Handles executing transfers against PostgreSQL with proper transaction
/// isolation and retry logic for serialization failures.
#[derive(Clone)]
pub struct PostgresExecutor {
    pool: Pool,
    isolation_level: IsolationLevel,
}

#[async_trait]
impl TransferExecutor for PostgresExecutor {
    async fn execute(&self, source: u64, dest: u64, amount: u64) -> Result<TransferResult> {
        execute_transfer_with_retry(&self.pool, &self.isolation_level, source, dest, amount).await
    }
}

/// Create a PostgreSQL workload runner
pub async fn create_workload(
    pg_config: &PostgresqlConfig,
    host: &str,
    port: u16,
    database: &str,
    user: &str,
    password: &str,
    num_accounts: u64,
    zipfian_exponent: f64,
    min_transfer_amount: u64,
    max_transfer_amount: u64,
    warmup_duration_secs: u64,
    test_duration_secs: u64,
    metrics: WorkloadMetrics,
) -> Result<WorkloadRunner<PostgresExecutor>> {
    // Build tokio_postgres config
    let mut pg_conn_config = tokio_postgres::Config::new();
    pg_conn_config.host(host);
    pg_conn_config.port(port);
    pg_conn_config.dbname(database);
    pg_conn_config.user(user);
    pg_conn_config.password(password);

    let recycling_method = match pg_config.pool_recycling_method {
        tb_perf_common::config::PoolRecyclingMethod::Fast => RecyclingMethod::Fast,
        tb_perf_common::config::PoolRecyclingMethod::Verified => RecyclingMethod::Verified,
    };

    let mgr_config = ManagerConfig { recycling_method };
    let mgr = Manager::from_config(pg_conn_config, NoTls, mgr_config);

    let pool = Pool::builder(mgr)
        .max_size(pg_config.connection_pool_size)
        .build()
        .context("Failed to create connection pool")?;

    // Pre-warm connection pool
    info!(
        "Pre-warming connection pool with {} connections",
        pg_config.connection_pool_size
    );
    let mut handles = Vec::new();
    for _ in 0..pg_config.connection_pool_size {
        let pool = pool.clone();
        handles.push(tokio::spawn(async move {
            let _ = pool.get().await;
        }));
    }
    for handle in handles {
        let _ = handle.await;
    }
    info!("Connection pool warmed up");

    let executor = PostgresExecutor {
        pool,
        isolation_level: pg_config.isolation_level.clone(),
    };

    Ok(WorkloadRunner::new(
        executor,
        num_accounts,
        zipfian_exponent,
        min_transfer_amount,
        max_transfer_amount,
        warmup_duration_secs,
        test_duration_secs,
        metrics,
    ))
}

/// Check if an error is a serialization failure using SQLSTATE code
fn is_serialization_failure(err: &anyhow::Error) -> bool {
    // Try to extract the underlying tokio_postgres error and check db error code
    if let Some(pg_err) = err.downcast_ref::<tokio_postgres::Error>()
        && let Some(db_err) = pg_err.as_db_error()
    {
        return db_err.code() == &SqlState::T_R_SERIALIZATION_FAILURE;
    }
    // Fallback to string matching for wrapped errors
    let err_str = err.to_string();
    err_str.contains("could not serialize access") || err_str.contains("40001")
}

/// Execute a transfer with retry logic for serialization failures
async fn execute_transfer_with_retry(
    pool: &Pool,
    isolation_level: &IsolationLevel,
    source: u64,
    dest: u64,
    amount: u64,
) -> Result<TransferResult> {
    let mut retries = 0;

    loop {
        match execute_transfer(pool, isolation_level, source, dest, amount).await {
            Ok(result) => return Ok(result),
            Err(e) => {
                if is_serialization_failure(&e) && retries < MAX_RETRIES {
                    retries += 1;
                    let delay = BASE_RETRY_DELAY_MS * 2u64.pow(retries);
                    debug!("Serialization failure, retry {} after {}ms", retries, delay);
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    continue;
                }

                if retries >= MAX_RETRIES {
                    warn!("Transfer failed after {} retries: {:?}", MAX_RETRIES, e);
                    return Ok(TransferResult::Failed);
                }

                error!("Transfer error: {:?}", e);
                return Err(e);
            }
        }
    }
}

/// Execute a single transfer in one network roundtrip
///
/// Uses simple_query to execute BEGIN, SET TRANSACTION, SELECT transfer(), and COMMIT
/// in a single roundtrip. Safe to interpolate u64 values directly (no SQL injection risk).
///
/// Note: synchronous_commit is configured at the PostgreSQL server level (synchronous_commit=on)
/// to match TigerBeetle's durability guarantees.
async fn execute_transfer(
    pool: &Pool,
    isolation_level: &IsolationLevel,
    source: u64,
    dest: u64,
    amount: u64,
) -> Result<TransferResult> {
    use tokio_postgres::SimpleQueryMessage;

    let client = pool.get().await.context("Failed to get connection")?;

    let isolation_level_str = match isolation_level {
        IsolationLevel::ReadCommitted => "READ COMMITTED",
        IsolationLevel::RepeatableRead => "REPEATABLE READ",
        IsolationLevel::Serializable => "SERIALIZABLE",
    };

    // Execute everything in a single roundtrip
    let sql = format!(
        "BEGIN; \
         SET TRANSACTION ISOLATION LEVEL {}; \
         SELECT transfer({}, {}, {}); \
         COMMIT",
        isolation_level_str, source as i64, dest as i64, amount as i64
    );

    let messages = client.simple_query(&sql).await?;

    // Find the row with the transfer result
    for msg in messages {
        if let SimpleQueryMessage::Row(row) = msg {
            let status = row.get(0).context("Missing transfer result")?;
            return match status {
                sql_results::SUCCESS => Ok(TransferResult::Success),
                sql_results::INSUFFICIENT_BALANCE => Ok(TransferResult::InsufficientBalance),
                sql_results::ACCOUNT_NOT_FOUND => Ok(TransferResult::AccountNotFound),
                _ => {
                    warn!("Unexpected transfer result: {}", status);
                    Ok(TransferResult::Failed)
                }
            };
        }
    }

    anyhow::bail!("No result from transfer function")
}
