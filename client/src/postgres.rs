use crate::metrics::{TestPhase, WorkloadMetrics};
use crate::workload::{AccountSelector, TransferGenerator, TransferResult, sql_results};
use anyhow::{Context, Result};
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use rand::SeedableRng;
use rand::rngs::SmallRng;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tb_perf_common::config::{IsolationLevel, PostgresqlConfig};
use tokio_postgres::NoTls;
use tokio_postgres::error::SqlState;
use tracing::{debug, error, info, warn};

/// Maximum number of retries for serialization failures
const MAX_RETRIES: u32 = 5;
/// Base delay for exponential backoff (milliseconds)
const BASE_RETRY_DELAY_MS: u64 = 10;

/// Controls test phase transitions (warmup -> measurement -> stop)
struct PhaseController {
    stop_flag: Arc<AtomicBool>,
    phase_flag: Arc<AtomicBool>, // false = warmup, true = measurement
    completed_count: Arc<AtomicU64>,
    warmup_duration: Duration,
    test_duration: Duration,
}

impl PhaseController {
    fn new(warmup_duration: Duration, test_duration: Duration) -> Self {
        Self {
            stop_flag: Arc::new(AtomicBool::new(false)),
            phase_flag: Arc::new(AtomicBool::new(false)),
            completed_count: Arc::new(AtomicU64::new(0)),
            warmup_duration,
            test_duration,
        }
    }

    fn stop_flag(&self) -> Arc<AtomicBool> {
        self.stop_flag.clone()
    }

    fn phase_flag(&self) -> Arc<AtomicBool> {
        self.phase_flag.clone()
    }

    fn completed_count(&self) -> Arc<AtomicU64> {
        self.completed_count.clone()
    }

    /// Run the phase timing: warmup -> measurement -> stop
    /// Returns (start_time, warmup_count) for stats calculation
    async fn run_phases(&self) -> (Instant, u64) {
        let start = Instant::now();
        info!("Warmup phase started ({:?})", self.warmup_duration);

        tokio::time::sleep(self.warmup_duration).await;

        self.phase_flag.store(true, Ordering::Release);
        let warmup_count = self.completed_count.load(Ordering::Relaxed);
        info!(
            "Measurement phase started ({:?}), warmup completed {} transfers",
            self.test_duration, warmup_count
        );

        tokio::time::sleep(self.test_duration).await;

        self.stop_flag.store(true, Ordering::Release);

        (start, warmup_count)
    }

    /// Log final workload statistics
    fn log_stats(&self, start: Instant, warmup_count: u64) {
        let total_count = self.completed_count.load(Ordering::Relaxed);
        let measurement_count = total_count - warmup_count;
        let elapsed = start.elapsed();
        let tps = measurement_count as f64 / self.test_duration.as_secs_f64();

        info!(
            "Workload completed: {} total transfers, {} in measurement phase ({:.2} TPS), elapsed {:?}",
            total_count, measurement_count, tps, elapsed
        );
    }
}

/// PostgreSQL workload executor
pub struct PostgresWorkload {
    pool: Pool,
    isolation_level: IsolationLevel,
    account_selector: AccountSelector,
    transfer_generator: TransferGenerator,
    metrics: WorkloadMetrics,
    warmup_duration: Duration,
    test_duration: Duration,
}

impl PostgresWorkload {
    /// Create a new PostgreSQL workload executor
    pub async fn new(
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
    ) -> Result<Self> {
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

        Ok(Self {
            pool,
            isolation_level: pg_config.isolation_level.clone(),
            account_selector: AccountSelector::new(num_accounts, zipfian_exponent),
            transfer_generator: TransferGenerator::new(min_transfer_amount, max_transfer_amount),
            metrics,
            warmup_duration: Duration::from_secs(warmup_duration_secs),
            test_duration: Duration::from_secs(test_duration_secs),
        })
    }

    /// Run the workload in max_throughput mode
    pub async fn run_max_throughput(&self, concurrency: usize) -> Result<()> {
        info!(
            "Starting max_throughput workload with {} workers",
            concurrency
        );

        let phase_ctrl = PhaseController::new(self.warmup_duration, self.test_duration);

        // Spawn workers
        let mut handles = Vec::new();
        for worker_id in 0..concurrency {
            let pool = self.pool.clone();
            let isolation_level = self.isolation_level.clone();
            let account_selector = self.account_selector.clone();
            let transfer_generator = self.transfer_generator.clone();
            let metrics = self.metrics.clone();
            let stop = phase_ctrl.stop_flag();
            let phase = phase_ctrl.phase_flag();
            let count = phase_ctrl.completed_count();

            handles.push(tokio::spawn(async move {
                run_worker(
                    worker_id,
                    pool,
                    isolation_level,
                    account_selector,
                    transfer_generator,
                    metrics,
                    stop,
                    phase,
                    count,
                )
                .await
            }));
        }

        // Run phase timing
        let (start, warmup_count) = phase_ctrl.run_phases().await;
        info!("Stopping workers...");

        // Wait for all workers
        for handle in handles {
            if let Err(e) = handle.await {
                error!("Worker error: {:?}", e);
            }
        }

        phase_ctrl.log_stats(start, warmup_count);
        Ok(())
    }

    /// Run the workload in fixed_rate mode
    pub async fn run_fixed_rate(&self, target_rate: u64, max_concurrency: usize) -> Result<()> {
        info!(
            "Starting fixed_rate workload at {} req/s (max concurrency: {})",
            target_rate, max_concurrency
        );

        let phase_ctrl = PhaseController::new(self.warmup_duration, self.test_duration);
        let in_flight = Arc::new(AtomicU64::new(0));

        let interval_ns = 1_000_000_000 / target_rate;
        let expected_interval = Duration::from_nanos(interval_ns);

        // Spawn request submitter
        let pool = self.pool.clone();
        let isolation_level = self.isolation_level.clone();
        let metrics = self.metrics.clone();
        let stop = phase_ctrl.stop_flag();
        let phase = phase_ctrl.phase_flag();
        let count = phase_ctrl.completed_count();
        let flight = in_flight.clone();
        let account_selector = self.account_selector.clone();
        let transfer_generator = self.transfer_generator.clone();

        let submitter = tokio::spawn(async move {
            let mut rng = SmallRng::from_rng(&mut rand::rng());
            let mut next_submit = Instant::now();

            while !stop.load(Ordering::Relaxed) {
                // Rate limit - sleep until scheduled time
                let now = Instant::now();
                if now < next_submit {
                    tokio::time::sleep(next_submit - now).await;
                }

                // Record scheduled time for coordinated omission correction
                // Latency includes any queue wait time if we're running behind
                let scheduled_time = next_submit;

                // Advance schedule by fixed interval (not from current time)
                // This maintains intended rate even when running behind
                next_submit += expected_interval;

                // Get current phase for metrics
                let current_phase = if phase.load(Ordering::Relaxed) {
                    TestPhase::Measurement
                } else {
                    TestPhase::Warmup
                };

                // Check concurrency limit
                if flight.load(Ordering::Relaxed) >= max_concurrency as u64 {
                    warn!("Max concurrency reached, dropping request");
                    metrics.record_dropped(current_phase.as_str());
                    continue;
                }

                // Submit request
                let (source, dest) = account_selector.select_transfer_accounts(&mut rng);
                let amount = transfer_generator.generate_amount(&mut rng);

                let pool = pool.clone();
                let isolation = isolation_level.clone();
                let metrics = metrics.clone();
                let count = count.clone();
                let flight = flight.clone();

                flight.fetch_add(1, Ordering::Relaxed);

                tokio::spawn(async move {
                    let result =
                        execute_transfer_with_retry(&pool, &isolation, source, dest, amount).await;

                    // Measure from scheduled time, not actual submit time
                    // This properly accounts for coordinated omission
                    let latency = scheduled_time.elapsed();
                    let latency_us = latency.as_micros() as u64;

                    match result {
                        Ok(TransferResult::Success) => {
                            metrics.record_completed(latency_us, current_phase.as_str());
                            count.fetch_add(1, Ordering::Relaxed);
                        }
                        Ok(TransferResult::InsufficientBalance) => {
                            metrics.record_rejected(latency_us, current_phase.as_str());
                            count.fetch_add(1, Ordering::Relaxed);
                        }
                        Ok(TransferResult::AccountNotFound)
                        | Ok(TransferResult::Failed)
                        | Err(_) => {
                            metrics.record_failed(current_phase.as_str());
                        }
                    }

                    flight.fetch_sub(1, Ordering::Relaxed);
                });
            }
        });

        // Run phase timing
        let (start, warmup_count) = phase_ctrl.run_phases().await;
        info!("Stopping...");

        let _ = submitter.await;

        // Wait for in-flight requests
        while in_flight.load(Ordering::Relaxed) > 0 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        phase_ctrl.log_stats(start, warmup_count);
        Ok(())
    }
}

/// Worker task for max_throughput mode
async fn run_worker(
    worker_id: usize,
    pool: Pool,
    isolation_level: IsolationLevel,
    account_selector: AccountSelector,
    transfer_generator: TransferGenerator,
    metrics: WorkloadMetrics,
    stop: Arc<AtomicBool>,
    phase: Arc<AtomicBool>,
    completed_count: Arc<AtomicU64>,
) {
    let mut rng = SmallRng::from_rng(&mut rand::rng());
    debug!("Worker {} started", worker_id);

    while !stop.load(Ordering::Relaxed) {
        let (source, dest) = account_selector.select_transfer_accounts(&mut rng);
        let amount = transfer_generator.generate_amount(&mut rng);
        let current_phase = if phase.load(Ordering::Relaxed) {
            TestPhase::Measurement
        } else {
            TestPhase::Warmup
        };

        let start = Instant::now();
        let result =
            execute_transfer_with_retry(&pool, &isolation_level, source, dest, amount).await;
        let latency = start.elapsed();
        let latency_us = latency.as_micros() as u64;

        match result {
            Ok(TransferResult::Success) => {
                metrics.record_completed(latency_us, current_phase.as_str());
                completed_count.fetch_add(1, Ordering::Relaxed);
            }
            Ok(TransferResult::InsufficientBalance) => {
                metrics.record_rejected(latency_us, current_phase.as_str());
                completed_count.fetch_add(1, Ordering::Relaxed);
            }
            Ok(TransferResult::AccountNotFound) | Ok(TransferResult::Failed) | Err(_) => {
                metrics.record_failed(current_phase.as_str());
            }
        }
    }

    debug!("Worker {} stopped", worker_id);
}

/// Check if an error is a serialization failure using SQLSTATE code
fn is_serialization_failure(err: &anyhow::Error) -> bool {
    // Try to extract the underlying tokio_postgres error
    if let Some(pg_err) = err.downcast_ref::<tokio_postgres::Error>() {
        if let Some(db_err) = pg_err.as_db_error() {
            return db_err.code() == &SqlState::T_R_SERIALIZATION_FAILURE;
        }
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
