use crate::metrics::{TestPhase, WorkloadMetrics};
use anyhow::Result;
use rand::Rng;
use rand_distr::{Distribution, Zipf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tracing::info;

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

/// SQL return value constants (must match init-postgresql.sql)
pub mod sql_results {
    pub const SUCCESS: &str = "success";
    pub const INSUFFICIENT_BALANCE: &str = "insufficient_balance";
    pub const ACCOUNT_NOT_FOUND: &str = "account_not_found";
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
