use crate::metrics::WorkloadMetrics;
use crate::workload::{TransferExecutor, TransferResult, WorkloadRunner};
use anyhow::{Context, Result};
use async_trait::async_trait;
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use std::time::Duration;
use tb_perf_common::config::{IsolationLevel, PostgresqlConfig};
use tokio::sync::{mpsc, oneshot};
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

/// Create a PostgreSQL connection pool manager
fn create_pool_manager(
    host: &str,
    port: u16,
    database: &str,
    user: &str,
    password: &str,
) -> Manager {
    let mut pg_conn_config = tokio_postgres::Config::new();
    pg_conn_config.host(host);
    pg_conn_config.port(port);
    pg_conn_config.dbname(database);
    pg_conn_config.user(user);
    pg_conn_config.password(password);

    // Use Verified recycling to ensure connections are valid before reuse
    let mgr_config = ManagerConfig {
        recycling_method: RecyclingMethod::Verified,
    };
    Manager::from_config(pg_conn_config, NoTls, mgr_config)
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
    let mgr = create_pool_manager(host, port, database, user, password);

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

// ============================================================================
// Atomic PostgreSQL Executor (no explicit locks)
// ============================================================================

/// PostgreSQL executor using atomic UPDATE (no SELECT FOR UPDATE)
///
/// Uses the `transfer_atomic` SQL function which relies on atomic UPDATE
/// statements with balance checks in the WHERE clause. Safe at READ COMMITTED.
#[derive(Clone)]
pub struct AtomicPostgresExecutor {
    pool: Pool,
    isolation_level: IsolationLevel,
}

#[async_trait]
impl TransferExecutor for AtomicPostgresExecutor {
    async fn execute(&self, source: u64, dest: u64, amount: u64) -> Result<TransferResult> {
        execute_atomic_transfer(&self.pool, &self.isolation_level, source, dest, amount).await
    }
}

/// Create an atomic PostgreSQL workload runner
pub async fn create_atomic_workload(
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
) -> Result<WorkloadRunner<AtomicPostgresExecutor>> {
    let mgr = create_pool_manager(host, port, database, user, password);

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

    let executor = AtomicPostgresExecutor {
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

/// Execute an atomic transfer (no explicit locks)
async fn execute_atomic_transfer(
    pool: &Pool,
    isolation_level: &IsolationLevel,
    source: u64,
    dest: u64,
    amount: u64,
) -> Result<TransferResult> {
    use tokio_postgres::SimpleQueryMessage;

    let client = pool.get().await.context("Failed to get connection")?;

    let isolation_level_str = isolation_level.as_sql_str();

    // Execute the atomic transfer function
    // Uses simple_query to execute multiple statements (BEGIN, SELECT, COMMIT)
    let sql = format!(
        "BEGIN TRANSACTION ISOLATION LEVEL {}; \
         SELECT transfer_atomic({}, {}, {}); \
         COMMIT;",
        isolation_level_str, source, dest, amount
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

    anyhow::bail!("No result from transfer_atomic function")
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

    let isolation_level_str = isolation_level.as_sql_str();

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

// ============================================================================
// Batched PostgreSQL Executor
// ============================================================================

/// Maximum batch size (matches TigerBeetle's batch limit)
const MAX_BATCH_SIZE: usize = 8190;

/// Request sent to the batch processor
struct TransferRequest {
    source: u64,
    dest: u64,
    amount: u64,
    response_tx: oneshot::Sender<TransferResult>,
}

/// Batched PostgreSQL transfer executor
///
/// Mimics TigerBeetle's batching behavior: transfers are queued and processed
/// in batches by a single background task using a single database connection.
/// This minimizes network round-trips and allows fair comparison with TigerBeetle.
#[derive(Clone)]
pub struct BatchedPostgresExecutor {
    request_tx: mpsc::Sender<TransferRequest>,
}

impl BatchedPostgresExecutor {
    /// Create a new batched executor with a background processing task
    pub fn new(pool: Pool, isolation_level: IsolationLevel) -> Self {
        // Bounded channel sized to hold 2 full batches worth of requests.
        // This provides enough buffer for the batch processor to drain one batch
        // while new requests accumulate, without excessive memory usage.
        let (request_tx, request_rx) = mpsc::channel(MAX_BATCH_SIZE * 2);

        // Spawn the background batch processor
        tokio::spawn(batch_processor(pool, isolation_level, request_rx));

        Self { request_tx }
    }
}

#[async_trait]
impl TransferExecutor for BatchedPostgresExecutor {
    async fn execute(&self, source: u64, dest: u64, amount: u64) -> Result<TransferResult> {
        let (response_tx, response_rx) = oneshot::channel();

        let request = TransferRequest {
            source,
            dest,
            amount,
            response_tx,
        };

        // Send request to batch processor
        self.request_tx
            .send(request)
            .await
            .map_err(|_| anyhow::anyhow!("Batch processor shut down"))?;

        // Wait for response
        response_rx
            .await
            .map_err(|_| anyhow::anyhow!("Batch processor dropped response"))
    }
}

/// Background task that processes transfers in batches
async fn batch_processor(
    pool: Pool,
    isolation_level: IsolationLevel,
    mut request_rx: mpsc::Receiver<TransferRequest>,
) {
    let mut batch: Vec<TransferRequest> = Vec::with_capacity(MAX_BATCH_SIZE);

    loop {
        // Wait for at least one request
        match request_rx.recv().await {
            Some(first_request) => {
                batch.push(first_request);

                // Collect any additional pending requests (non-blocking)
                while batch.len() < MAX_BATCH_SIZE {
                    match request_rx.try_recv() {
                        Ok(req) => batch.push(req),
                        Err(mpsc::error::TryRecvError::Empty) => break,
                        Err(mpsc::error::TryRecvError::Disconnected) => {
                            // Channel closed, process remaining and exit
                            if !batch.is_empty() {
                                execute_batch(&pool, &isolation_level, &mut batch).await;
                            }
                            return;
                        }
                    }
                }

                // Execute the batch
                execute_batch(&pool, &isolation_level, &mut batch).await;
            }
            None => {
                // Channel closed
                return;
            }
        }
    }
}

/// Result codes from batch_transfers SQL function
///
/// These match the SMALLINT values returned by the batch_transfers() SQL function.
/// Code 3 (FAILED) is returned for unexpected errors; any unrecognized code is also
/// treated as failed.
mod batch_result_codes {
    pub const SUCCESS: i16 = 0;
    pub const INSUFFICIENT_BALANCE: i16 = 1;
    pub const ACCOUNT_NOT_FOUND: i16 = 2;
    #[allow(dead_code)] // Documented for completeness; matched by wildcard
    pub const FAILED: i16 = 3;
}

/// Execute a batch of transfers and send results back to callers
async fn execute_batch(
    pool: &Pool,
    isolation_level: &IsolationLevel,
    batch: &mut Vec<TransferRequest>,
) {
    debug!("Executing batch of {} transfers", batch.len());

    // Build parallel arrays for efficient transfer
    let source_ids: Vec<i64> = batch.iter().map(|req| req.source as i64).collect();
    let dest_ids: Vec<i64> = batch.iter().map(|req| req.dest as i64).collect();
    let amounts: Vec<i64> = batch.iter().map(|req| req.amount as i64).collect();

    // Execute batch transfer
    let results = execute_batch_transfer(pool, isolation_level, &source_ids, &dest_ids, &amounts).await;

    // Send results back to callers.
    // Note: We ignore send errors because the receiver may have been dropped (e.g., caller timed out).
    // This is expected behavior - the caller simply won't receive the result.
    match results {
        Ok(result_vec) => {
            // Verify result count matches batch size (should always match if SQL is correct)
            if result_vec.len() != batch.len() {
                error!(
                    "Result count mismatch: expected {}, got {}. Marking all as failed.",
                    batch.len(),
                    result_vec.len()
                );
                for req in batch.drain(..) {
                    let _ = req.response_tx.send(TransferResult::Failed);
                }
                return;
            }
            for (req, result) in batch.drain(..).zip(result_vec.into_iter()) {
                let _ = req.response_tx.send(result);
            }
        }
        Err(e) => {
            // On batch failure, mark all transfers as failed
            error!("Batch transfer failed: {:?}", e);
            for req in batch.drain(..) {
                let _ = req.response_tx.send(TransferResult::Failed);
            }
        }
    }
}

/// Execute a batch of transfers using the batch_transfers SQL function with array parameters
async fn execute_batch_transfer(
    pool: &Pool,
    isolation_level: &IsolationLevel,
    source_ids: &[i64],
    dest_ids: &[i64],
    amounts: &[i64],
) -> Result<Vec<TransferResult>> {
    let client = pool.get().await.context("Failed to get connection")?;

    let isolation_level_str = isolation_level.as_sql_str();

    // Start transaction with proper isolation level
    client
        .execute(
            &format!(
                "BEGIN TRANSACTION ISOLATION LEVEL {}",
                isolation_level_str
            ),
            &[],
        )
        .await
        .context("Failed to begin transaction")?;

    // Execute batch transfer with array parameters (binary protocol).
    // On error, rollback to leave connection in clean state for pool reuse.
    let query_result = client
        .query_one(
            "SELECT batch_transfers($1, $2, $3)",
            &[&source_ids, &dest_ids, &amounts],
        )
        .await;

    let row = match query_result {
        Ok(row) => row,
        Err(e) => {
            let _ = client.execute("ROLLBACK", &[]).await;
            return Err(e).context("Failed to execute batch_transfers");
        }
    };

    // Commit transaction. On error, rollback.
    if let Err(e) = client.execute("COMMIT", &[]).await {
        let _ = client.execute("ROLLBACK", &[]).await;
        return Err(e).context("Failed to commit transaction");
    }

    // Parse the SMALLINT[] result
    let result_codes: Vec<i16> = row.get(0);

    Ok(result_codes
        .into_iter()
        .map(|code| match code {
            batch_result_codes::SUCCESS => TransferResult::Success,
            batch_result_codes::INSUFFICIENT_BALANCE => TransferResult::InsufficientBalance,
            batch_result_codes::ACCOUNT_NOT_FOUND => TransferResult::AccountNotFound,
            _ => TransferResult::Failed, // Includes FAILED (3) and any unexpected codes
        })
        .collect())
}

/// Create a batched PostgreSQL workload runner
pub async fn create_batched_workload(
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
) -> Result<WorkloadRunner<BatchedPostgresExecutor>> {
    let mgr = create_pool_manager(host, port, database, user, password);

    // For batched mode, we only need a single connection
    let pool = Pool::builder(mgr)
        .max_size(1)
        .build()
        .context("Failed to create connection pool")?;

    // Warm up the single connection
    info!("Warming up batched connection pool (single connection)");
    let _ = pool.get().await.context("Failed to warm up connection")?;
    info!("Connection pool warmed up");

    let executor = BatchedPostgresExecutor::new(pool, pg_config.isolation_level.clone());

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
