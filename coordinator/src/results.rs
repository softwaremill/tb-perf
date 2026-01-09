use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use tb_perf_common::Config;
use tracing::{info, warn};

/// Result of a single test run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    pub run_id: usize,
    pub duration_secs: f64,
    pub throughput_tps: f64,
    pub latency_p50_us: u64,
    pub latency_p95_us: u64,
    pub latency_p99_us: u64,
    pub latency_p999_us: u64,
    pub completed_transfers: u64,
    pub rejected_transfers: u64,
    pub failed_transfers: u64,
    pub balance_verified: bool,
}

/// Aggregate statistics across runs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateStats {
    pub mean: f64,
    pub stddev: f64,
    pub cv: f64, // Coefficient of variation
    pub min: f64,
    pub max: f64,
}

impl AggregateStats {
    fn from_values(values: &[f64]) -> Self {
        if values.is_empty() {
            return Self {
                mean: 0.0,
                stddev: 0.0,
                cv: 0.0,
                min: 0.0,
                max: 0.0,
            };
        }

        let n = values.len() as f64;
        let mean = values.iter().sum::<f64>() / n;
        let variance = values.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
        let stddev = variance.sqrt();
        let cv = if mean > 0.0 { stddev / mean } else { 0.0 };
        let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

        Self {
            mean,
            stddev,
            cv,
            min,
            max,
        }
    }
}

/// Aggregated test results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResults {
    pub config_summary: ConfigSummary,
    pub runs: Vec<RunResult>,
    pub aggregate: Option<AggregateResults>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigSummary {
    pub database_type: String,
    pub test_mode: String,
    pub num_accounts: u64,
    pub initial_balance: u64,
    pub warmup_duration_secs: u64,
    pub test_duration_secs: u64,
    pub num_runs: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateResults {
    pub throughput: AggregateStats,
    pub latency_p50: AggregateStats,
    pub latency_p95: AggregateStats,
    pub latency_p99: AggregateStats,
    pub latency_p999: AggregateStats,
    pub total_completed: u64,
    pub total_rejected: u64,
    pub total_failed: u64,
    pub error_rate: f64,
}

impl TestResults {
    pub fn new(config: Config, num_runs: usize) -> Self {
        let config_summary = ConfigSummary {
            database_type: format!("{:?}", config.database.kind),
            test_mode: config.workload.test_mode.clone(),
            num_accounts: config.workload.num_accounts,
            initial_balance: config.workload.initial_balance,
            warmup_duration_secs: config.workload.warmup_duration_secs,
            test_duration_secs: config.workload.test_duration_secs,
            num_runs,
        };

        Self {
            config_summary,
            runs: Vec::new(),
            aggregate: None,
            warnings: Vec::new(),
            errors: Vec::new(),
        }
    }

    pub fn add_run(&mut self, run: RunResult) {
        self.runs.push(run);
    }

    pub fn set_balance_error(&mut self, run_id: usize) {
        self.errors
            .push(format!("Balance verification failed for run {}", run_id));
        if let Some(run) = self.runs.iter_mut().find(|r| r.run_id == run_id) {
            run.balance_verified = false;
        }
    }

    pub fn calculate_aggregates(&mut self) {
        if self.runs.is_empty() {
            return;
        }

        let throughputs: Vec<f64> = self.runs.iter().map(|r| r.throughput_tps).collect();
        let p50s: Vec<f64> = self.runs.iter().map(|r| r.latency_p50_us as f64).collect();
        let p95s: Vec<f64> = self.runs.iter().map(|r| r.latency_p95_us as f64).collect();
        let p99s: Vec<f64> = self.runs.iter().map(|r| r.latency_p99_us as f64).collect();
        let p999s: Vec<f64> = self.runs.iter().map(|r| r.latency_p999_us as f64).collect();

        let total_completed: u64 = self.runs.iter().map(|r| r.completed_transfers).sum();
        let total_rejected: u64 = self.runs.iter().map(|r| r.rejected_transfers).sum();
        let total_failed: u64 = self.runs.iter().map(|r| r.failed_transfers).sum();

        let total_requests = total_completed + total_rejected + total_failed;
        let error_rate = if total_requests > 0 {
            total_failed as f64 / total_requests as f64
        } else {
            0.0
        };

        let throughput_stats = AggregateStats::from_values(&throughputs);
        let p99_stats = AggregateStats::from_values(&p99s);

        // Check for high variance warnings
        if throughput_stats.cv > 0.10 {
            self.warnings.push(format!(
                "High throughput variance: CV = {:.2}% (threshold: 10%)",
                throughput_stats.cv * 100.0
            ));
        }

        if p99_stats.cv > 0.15 {
            self.warnings.push(format!(
                "High p99 latency variance: CV = {:.2}% (threshold: 15%)",
                p99_stats.cv * 100.0
            ));
        }

        if error_rate > 0.05 {
            self.errors.push(format!(
                "High error rate: {:.2}% (threshold: 5%)",
                error_rate * 100.0
            ));
        }

        self.aggregate = Some(AggregateResults {
            throughput: throughput_stats,
            latency_p50: AggregateStats::from_values(&p50s),
            latency_p95: AggregateStats::from_values(&p95s),
            latency_p99: p99_stats,
            latency_p999: AggregateStats::from_values(&p999s),
            total_completed,
            total_rejected,
            total_failed,
            error_rate,
        });
    }

    /// Export results to JSON file
    pub fn export_json(&self, output_path: &str) -> anyhow::Result<()> {
        // Create directory if needed
        if let Some(parent) = Path::new(output_path).parent() {
            fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string_pretty(self)?;
        fs::write(output_path, json)?;

        info!("Results exported to: {}", output_path);
        Ok(())
    }

    /// Print summary to console
    pub fn print_summary(&self) {
        info!("=== Test Results Summary ===");
        info!(
            "Database: {}, Mode: {}",
            self.config_summary.database_type, self.config_summary.test_mode
        );
        info!(
            "Accounts: {}, Runs: {}",
            self.config_summary.num_accounts,
            self.runs.len()
        );

        if let Some(agg) = &self.aggregate {
            info!("--- Throughput ---");
            info!(
                "  Mean: {:.2} TPS, StdDev: {:.2}, CV: {:.2}%",
                agg.throughput.mean,
                agg.throughput.stddev,
                agg.throughput.cv * 100.0
            );

            info!("--- Latency (microseconds) ---");
            info!("  p50:  {:.0}", agg.latency_p50.mean);
            info!("  p95:  {:.0}", agg.latency_p95.mean);
            info!("  p99:  {:.0}", agg.latency_p99.mean);
            info!("  p999: {:.0}", agg.latency_p999.mean);

            info!("--- Transfers ---");
            info!("  Completed: {}", agg.total_completed);
            info!("  Rejected:  {}", agg.total_rejected);
            info!("  Failed:    {}", agg.total_failed);
            info!("  Error rate: {:.2}%", agg.error_rate * 100.0);
        }

        if !self.warnings.is_empty() {
            warn!("--- Warnings ---");
            for warning in &self.warnings {
                warn!("  {}", warning);
            }
        }

        if !self.errors.is_empty() {
            tracing::error!("--- Errors ---");
            for error in &self.errors {
                tracing::error!("  {}", error);
            }
        }
    }
}
