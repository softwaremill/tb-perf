use crate::metrics::{TestPhase, WorkloadMetrics};
use anyhow::Result;
use async_trait::async_trait;
use rand::Rng;
use rand::SeedableRng;
use rand::rngs::SmallRng;
use rand_distr::{Distribution, Zipf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// Generates account pairs for transfers using Zipfian distribution
#[derive(Clone)]
pub struct AccountSelector {
    num_accounts: u64,
    zipf: Zipf<f64>,
}

impl AccountSelector {
    pub fn new(num_accounts: u64, zipfian_exponent: f64) -> Self {
        // Zipf distribution: lower IDs are more likely to be selected
        // exponent 0 = uniform, exponent ~1.5 = high skew
        let zipf =
            Zipf::new(num_accounts as f64, zipfian_exponent).expect("Invalid Zipfian parameters");
        Self { num_accounts, zipf }
    }

    /// Select a random account using Zipfian distribution
    pub fn select_account<R: Rng>(&self, rng: &mut R) -> u64 {
        // Zipf returns values in [1, n], we want [0, n-1]
        let account = self.zipf.sample(rng) as u64 - 1;
        account.min(self.num_accounts - 1)
    }

    /// Select two different accounts for a transfer
    pub fn select_transfer_accounts<R: Rng>(&self, rng: &mut R) -> (u64, u64) {
        let source = self.select_account(rng);
        let mut dest = self.select_account(rng);

        // Ensure source and destination are different
        while dest == source {
            dest = self.select_account(rng);
        }

        (source, dest)
    }
}

/// Generates random transfer amounts within configured range
#[derive(Clone)]
pub struct TransferGenerator {
    min_amount: u64,
    max_amount: u64,
}

impl TransferGenerator {
    pub fn new(min_amount: u64, max_amount: u64) -> Self {
        Self {
            min_amount,
            max_amount,
        }
    }

    /// Generate a random transfer amount
    pub fn generate_amount<R: Rng>(&self, rng: &mut R) -> u64 {
        rng.random_range(self.min_amount..=self.max_amount)
    }
}

/// Result of a single transfer operation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferResult {
    /// Transfer completed successfully
    Success,
    /// Transfer rejected due to insufficient balance
    InsufficientBalance,
    /// Transfer failed because account doesn't exist
    AccountNotFound,
    /// Transfer failed due to database error (after retries)
    Failed,
}

/// Controls test phase transitions (warmup -> measurement -> stop)
///
/// This controller manages the timing of benchmark phases and provides
/// thread-safe flags for workers to check the current phase.
pub struct PhaseController {
    stop_flag: Arc<AtomicBool>,
    /// Current phase: false = Warmup, true = Measurement
    /// Use get_current_phase() to convert to TestPhase enum
    phase_flag: Arc<AtomicBool>,
    completed_count: Arc<AtomicU64>,
    warmup_duration: Duration,
    test_duration: Duration,
}

impl PhaseController {
    pub fn new(warmup_duration: Duration, test_duration: Duration) -> Self {
        Self {
            stop_flag: Arc::new(AtomicBool::new(false)),
            phase_flag: Arc::new(AtomicBool::new(false)),
            completed_count: Arc::new(AtomicU64::new(0)),
            warmup_duration,
            test_duration,
        }
    }

    pub fn stop_flag(&self) -> Arc<AtomicBool> {
        self.stop_flag.clone()
    }

    pub fn phase_flag(&self) -> Arc<AtomicBool> {
        self.phase_flag.clone()
    }

    pub fn completed_count(&self) -> Arc<AtomicU64> {
        self.completed_count.clone()
    }

    /// Run the phase timing: warmup -> measurement -> stop
    /// Returns (start_time, warmup_count) for stats calculation
    pub async fn run_phases(&self) -> (Instant, u64) {
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
    pub fn log_stats(&self, start: Instant, warmup_count: u64) {
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

/// Get the current test phase from the phase flag
pub fn get_current_phase(phase_flag: &AtomicBool) -> TestPhase {
    if phase_flag.load(Ordering::Relaxed) {
        TestPhase::Measurement
    } else {
        TestPhase::Warmup
    }
}

/// Record the result of a transfer operation to metrics
///
/// This helper centralizes the logic for recording transfer results,
/// ensuring consistent behavior across PostgreSQL and TigerBeetle workloads.
pub fn record_transfer_result(
    result: &Result<TransferResult>,
    latency_us: u64,
    phase: &TestPhase,
    metrics: &WorkloadMetrics,
    completed_count: &Arc<AtomicU64>,
) {
    match result {
        Ok(TransferResult::Success) => {
            metrics.record_completed(latency_us, phase.as_str());
            completed_count.fetch_add(1, Ordering::Relaxed);
        }
        Ok(TransferResult::InsufficientBalance) => {
            metrics.record_rejected(latency_us, phase.as_str());
            completed_count.fetch_add(1, Ordering::Relaxed);
        }
        Ok(TransferResult::AccountNotFound) | Ok(TransferResult::Failed) | Err(_) => {
            metrics.record_failed(phase.as_str());
        }
    }
}

/// Trait for database-specific transfer execution
///
/// Implementations handle the details of executing a transfer against
/// a specific database backend (PostgreSQL, TigerBeetle, etc.)
#[async_trait]
pub trait TransferExecutor: Clone + Send + Sync + 'static {
    /// Execute a transfer from source to destination account
    async fn execute(&self, source: u64, dest: u64, amount: u64) -> Result<TransferResult>;
}

/// Generic workload runner that executes transfers using a database-specific executor
///
/// This struct encapsulates the common workload execution logic (max_throughput and
/// fixed_rate modes), parameterized by a TransferExecutor implementation.
pub struct WorkloadRunner<E: TransferExecutor> {
    executor: E,
    account_selector: AccountSelector,
    transfer_generator: TransferGenerator,
    metrics: WorkloadMetrics,
    warmup_duration: Duration,
    test_duration: Duration,
}

impl<E: TransferExecutor> WorkloadRunner<E> {
    /// Create a new workload runner
    pub fn new(
        executor: E,
        num_accounts: u64,
        zipfian_exponent: f64,
        min_transfer_amount: u64,
        max_transfer_amount: u64,
        warmup_duration_secs: u64,
        test_duration_secs: u64,
        metrics: WorkloadMetrics,
    ) -> Self {
        Self {
            executor,
            account_selector: AccountSelector::new(num_accounts, zipfian_exponent),
            transfer_generator: TransferGenerator::new(min_transfer_amount, max_transfer_amount),
            metrics,
            warmup_duration: Duration::from_secs(warmup_duration_secs),
            test_duration: Duration::from_secs(test_duration_secs),
        }
    }

    /// Run the workload in max_throughput mode
    ///
    /// Spawns multiple workers that execute transfers as fast as possible.
    pub async fn run_max_throughput(&self, concurrency: usize) -> Result<()> {
        info!(
            "Starting max_throughput workload with {} workers",
            concurrency
        );

        let phase_ctrl = PhaseController::new(self.warmup_duration, self.test_duration);

        // Spawn workers
        let mut handles = Vec::new();
        for worker_id in 0..concurrency {
            let executor = self.executor.clone();
            let account_selector = self.account_selector.clone();
            let transfer_generator = self.transfer_generator.clone();
            let metrics = self.metrics.clone();
            let stop = phase_ctrl.stop_flag();
            let phase = phase_ctrl.phase_flag();
            let count = phase_ctrl.completed_count();

            handles.push(tokio::spawn(async move {
                max_throughput_worker(
                    worker_id,
                    executor,
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
                tracing::error!("Worker error: {:?}", e);
            }
        }

        phase_ctrl.log_stats(start, warmup_count);
        Ok(())
    }

    /// Run the workload in fixed_rate mode
    ///
    /// Submits transfers at a fixed rate, dropping requests if max concurrency is exceeded.
    /// Uses coordinated omission correction by measuring latency from scheduled time.
    pub async fn run_fixed_rate(&self, target_rate: u64, max_concurrency: usize) -> Result<()> {
        // Guard against division by zero
        if target_rate == 0 {
            anyhow::bail!("target_rate must be greater than 0");
        }

        info!(
            "Starting fixed_rate workload at {} req/s (max concurrency: {})",
            target_rate, max_concurrency
        );

        let phase_ctrl = PhaseController::new(self.warmup_duration, self.test_duration);
        let in_flight = Arc::new(AtomicU64::new(0));

        let interval_ns = 1_000_000_000 / target_rate;
        let expected_interval = Duration::from_nanos(interval_ns);

        // Spawn request submitter
        let executor = self.executor.clone();
        let metrics = self.metrics.clone();
        let stop = phase_ctrl.stop_flag();
        let phase = phase_ctrl.phase_flag();
        let count = phase_ctrl.completed_count();
        let in_flight_clone = in_flight.clone();
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
                let scheduled_time = next_submit;

                // Advance schedule by fixed interval
                next_submit += expected_interval;

                // Check current phase for dropped request tracking
                let current_phase = get_current_phase(&phase);

                // Check concurrency limit
                if in_flight_clone.load(Ordering::Relaxed) >= max_concurrency as u64 {
                    warn!("Max concurrency reached, dropping request");
                    metrics.record_dropped(current_phase.as_str());
                    continue;
                }

                // Submit request
                let (source, dest) = account_selector.select_transfer_accounts(&mut rng);
                let amount = transfer_generator.generate_amount(&mut rng);

                in_flight_clone.fetch_add(1, Ordering::Relaxed);

                tokio::spawn(fixed_rate_transfer_task(
                    executor.clone(),
                    source,
                    dest,
                    amount,
                    scheduled_time,
                    phase.clone(),
                    metrics.clone(),
                    count.clone(),
                    in_flight_clone.clone(),
                ));
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

/// Execute a single transfer and record metrics for fixed_rate mode
///
/// This is spawned as a separate task for each transfer in fixed_rate mode.
/// Latency is measured from `scheduled_time` to account for coordinated omission.
async fn fixed_rate_transfer_task<E: TransferExecutor>(
    executor: E,
    source: u64,
    dest: u64,
    amount: u64,
    scheduled_time: Instant,
    phase_flag: Arc<AtomicBool>,
    metrics: WorkloadMetrics,
    completed_count: Arc<AtomicU64>,
    in_flight: Arc<AtomicU64>,
) {
    let result = executor.execute(source, dest, amount).await;

    // Measure from scheduled time for coordinated omission correction
    let latency = scheduled_time.elapsed();
    let latency_us = latency.as_micros() as u64;

    // Capture phase at recording time, not submission time
    let current_phase = get_current_phase(&phase_flag);

    record_transfer_result(
        &result,
        latency_us,
        &current_phase,
        &metrics,
        &completed_count,
    );

    in_flight.fetch_sub(1, Ordering::Relaxed);
}

/// Worker task for max_throughput mode - runs transfers as fast as possible
async fn max_throughput_worker<E: TransferExecutor>(
    worker_id: usize,
    executor: E,
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

        let start = Instant::now();
        let result = executor.execute(source, dest, amount).await;
        let latency = start.elapsed();
        let latency_us = latency.as_micros() as u64;

        // Capture phase at recording time for accurate attribution
        let current_phase = get_current_phase(&phase);
        record_transfer_result(
            &result,
            latency_us,
            &current_phase,
            &metrics,
            &completed_count,
        );
    }

    debug!("Worker {} stopped", worker_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_account_selector_uniform() {
        let selector = AccountSelector::new(1000, 0.0);
        let mut rng = rand::rng();

        // Just verify it doesn't panic and returns valid accounts
        for _ in 0..100 {
            let account = selector.select_account(&mut rng);
            assert!(account < 1000);
        }
    }

    #[test]
    fn test_account_selector_skewed() {
        let selector = AccountSelector::new(1000, 1.5);
        let mut rng = rand::rng();

        // With high skew, lower accounts should be selected more often
        let mut low_count = 0;
        for _ in 0..1000 {
            let account = selector.select_account(&mut rng);
            assert!(account < 1000);
            if account < 100 {
                low_count += 1;
            }
        }

        // With zipf exponent 1.5, we expect significant skew toward low accounts
        assert!(low_count > 500, "Expected skew toward low accounts");
    }

    #[test]
    fn test_transfer_accounts_different() {
        let selector = AccountSelector::new(1000, 1.0);
        let mut rng = rand::rng();

        for _ in 0..100 {
            let (source, dest) = selector.select_transfer_accounts(&mut rng);
            assert_ne!(source, dest, "Source and destination must be different");
        }
    }

    #[test]
    fn test_transfer_generator() {
        let generator = TransferGenerator::new(1, 1000);
        let mut rng = rand::rng();

        for _ in 0..100 {
            let amount = generator.generate_amount(&mut rng);
            assert!(amount >= 1 && amount <= 1000);
        }
    }
}
