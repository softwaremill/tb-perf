use crate::metrics::WorkloadMetrics;
use crate::workload::{
    AccountSelector, PhaseController, TransferGenerator, TransferResult, get_current_phase,
    record_transfer_result,
};
use anyhow::{Context, Result};
use rand::SeedableRng;
use rand::rngs::SmallRng;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tb::error::{CreateTransferErrorKind, CreateTransfersError};
use tigerbeetle_unofficial as tb;
use tracing::{debug, error, info, warn};

/// TigerBeetle workload executor
pub struct TigerBeetleWorkload {
    client: Arc<tb::Client>,
    account_selector: AccountSelector,
    transfer_generator: TransferGenerator,
    metrics: WorkloadMetrics,
    warmup_duration: Duration,
    test_duration: Duration,
}

impl TigerBeetleWorkload {
    /// Create a new TigerBeetle workload executor
    ///
    /// Note: `measure_batch_sizes` parameter is accepted for API compatibility
    /// but not currently used. TigerBeetle batching is handled internally.
    pub async fn new(
        cluster_addresses: &[String],
        num_accounts: u64,
        zipfian_exponent: f64,
        min_transfer_amount: u64,
        max_transfer_amount: u64,
        warmup_duration_secs: u64,
        test_duration_secs: u64,
        _measure_batch_sizes: bool,
        metrics: WorkloadMetrics,
    ) -> Result<Self> {
        // TigerBeetle cluster ID 0 for local development
        let cluster_id = 0;

        // Join addresses with comma for TigerBeetle client
        let addresses = cluster_addresses.join(",");
        info!("Connecting to TigerBeetle cluster: {}", addresses);

        let client = tb::Client::new(cluster_id, &addresses)
            .context("Failed to create TigerBeetle client")?;

        info!("Connected to TigerBeetle cluster");

        Ok(Self {
            client: Arc::new(client),
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
            let client = self.client.clone();
            let account_selector = self.account_selector.clone();
            let transfer_generator = self.transfer_generator.clone();
            let metrics = self.metrics.clone();
            let stop = phase_ctrl.stop_flag();
            let phase = phase_ctrl.phase_flag();
            let count = phase_ctrl.completed_count();

            handles.push(tokio::spawn(async move {
                max_throughput_worker(
                    worker_id,
                    client,
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
        let client = self.client.clone();
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

                let client = client.clone();
                let metrics = metrics.clone();
                let count = count.clone();
                let in_flight_task = in_flight_clone.clone();
                let phase_flag = phase.clone();

                in_flight_clone.fetch_add(1, Ordering::Relaxed);

                tokio::spawn(async move {
                    let result = execute_transfer(&client, source, dest, amount).await;

                    // Measure from scheduled time for coordinated omission correction
                    let latency = scheduled_time.elapsed();
                    let latency_us = latency.as_micros() as u64;

                    // Capture phase at recording time, not submission time
                    // This ensures accurate phase attribution for requests that span phase boundaries
                    let current_phase = get_current_phase(&phase_flag);

                    record_transfer_result(&result, latency_us, &current_phase, &metrics, &count);

                    in_flight_task.fetch_sub(1, Ordering::Relaxed);
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

/// Worker task for max_throughput mode - runs transfers as fast as possible
async fn max_throughput_worker(
    worker_id: usize,
    client: Arc<tb::Client>,
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
        let result = execute_transfer(&client, source, dest, amount).await;
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

/// Execute a single transfer
async fn execute_transfer(
    client: &tb::Client,
    source: u64,
    dest: u64,
    amount: u64,
) -> Result<TransferResult> {
    // Create the transfer using builder pattern
    let transfer = tb::Transfer::new(tb::id())
        .with_debit_account_id(source as u128)
        .with_credit_account_id(dest as u128)
        .with_amount(amount as u128)
        .with_ledger(1)
        .with_code(1);

    match client.create_transfers(vec![transfer]).await {
        Ok(()) => Ok(TransferResult::Success),
        Err(CreateTransfersError::Api(api_err)) => {
            // Check the first error (single transfer = single error)
            if let Some(err) = api_err.as_slice().first() {
                match err.kind() {
                    CreateTransferErrorKind::ExceedsCredits
                    | CreateTransferErrorKind::ExceedsDebits => {
                        return Ok(TransferResult::InsufficientBalance);
                    }
                    CreateTransferErrorKind::DebitAccountNotFound
                    | CreateTransferErrorKind::CreditAccountNotFound => {
                        return Ok(TransferResult::AccountNotFound);
                    }
                    _ => {
                        warn!("Transfer error: {:?}", err.kind());
                        return Ok(TransferResult::Failed);
                    }
                }
            }
            Ok(TransferResult::Failed)
        }
        Err(CreateTransfersError::Send(e)) => {
            error!("Send error: {:?}", e);
            Err(anyhow::anyhow!("Send error: {:?}", e))
        }
        Err(e) => {
            error!("Unexpected error: {:?}", e);
            Err(anyhow::anyhow!("Unexpected error: {:?}", e))
        }
    }
}
