use anyhow::Result;
use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Histogram, MeterProvider};
use opentelemetry_otlp::{MetricExporter, WithExportConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use std::sync::Arc;
use std::time::Duration;

/// Metrics collected during workload execution
#[derive(Clone)]
pub struct WorkloadMetrics {
    /// Counter for completed transfers
    pub completed: Counter<u64>,
    /// Counter for rejected transfers (insufficient balance)
    pub rejected: Counter<u64>,
    /// Counter for failed transfers (database errors)
    pub failed: Counter<u64>,
    /// Counter for dropped requests (in fixed_rate mode when max concurrency reached)
    pub dropped: Counter<u64>,
    /// Histogram for transfer latency in microseconds
    pub latency_us: Histogram<u64>,
    /// The meter provider (kept alive for shutdown). None for test/noop metrics.
    _provider: Option<Arc<SdkMeterProvider>>,
}

impl WorkloadMetrics {
    /// Initialize OpenTelemetry metrics with OTLP exporter
    pub fn new(otel_endpoint: &str, database_type: &str, test_mode: &str) -> Result<Self> {
        let resource = Resource::new(vec![
            KeyValue::new("service.name", "tb-perf-client"),
            KeyValue::new("database.type", database_type.to_string()),
            KeyValue::new("test.mode", test_mode.to_string()),
        ]);

        let exporter = MetricExporter::builder()
            .with_tonic()
            .with_endpoint(otel_endpoint)
            .with_timeout(Duration::from_secs(10))
            .build()?;

        let reader = PeriodicReader::builder(exporter, opentelemetry_sdk::runtime::Tokio)
            .with_interval(Duration::from_secs(5))
            .build();

        let provider = SdkMeterProvider::builder()
            .with_resource(resource)
            .with_reader(reader)
            .build();

        let meter = provider.meter("tb-perf");

        let completed = meter
            .u64_counter("transfers_completed")
            .with_description("Number of completed transfers")
            .build();

        let rejected = meter
            .u64_counter("transfers_rejected")
            .with_description("Number of rejected transfers (insufficient balance)")
            .build();

        let failed = meter
            .u64_counter("transfers_failed")
            .with_description("Number of failed transfers (database errors)")
            .build();

        let dropped = meter
            .u64_counter("requests_dropped")
            .with_description(
                "Number of dropped requests due to max concurrency in fixed_rate mode",
            )
            .build();

        let latency_us = meter
            .u64_histogram("transfer_latency_us")
            .with_description("Transfer latency in microseconds")
            .build();

        Ok(Self {
            completed,
            rejected,
            failed,
            dropped,
            latency_us,
            _provider: Some(Arc::new(provider)),
        })
    }

    /// Record a completed transfer
    pub fn record_completed(&self, latency_us: u64, phase: &str) {
        let attrs = &[KeyValue::new("phase", phase.to_string())];
        self.completed.add(1, attrs);
        self.latency_us.record(latency_us, attrs);
    }

    /// Record a rejected transfer (insufficient balance)
    pub fn record_rejected(&self, latency_us: u64, phase: &str) {
        let attrs = &[KeyValue::new("phase", phase.to_string())];
        self.rejected.add(1, attrs);
        self.latency_us.record(latency_us, attrs);
    }

    /// Record a failed transfer (database error)
    pub fn record_failed(&self, phase: &str) {
        let attrs = &[KeyValue::new("phase", phase.to_string())];
        self.failed.add(1, attrs);
    }

    /// Record a dropped request (in fixed_rate mode when max concurrency reached)
    pub fn record_dropped(&self, phase: &str) {
        let attrs = &[KeyValue::new("phase", phase.to_string())];
        self.dropped.add(1, attrs);
    }

    /// Shutdown the OpenTelemetry provider and flush remaining metrics
    pub fn shutdown(&self) {
        if let Some(ref provider) = self._provider
            && let Err(e) = provider.shutdown()
        {
            tracing::warn!("Failed to shutdown OpenTelemetry provider: {:?}", e);
        }
    }

    /// Create a no-op metrics instance for testing (no OTel export)
    ///
    /// Uses SDK provider without readers - metrics are collected but not exported.
    /// This avoids network connections and background tasks.
    #[cfg(test)]
    pub fn new_noop() -> Self {
        // Create SDK provider with no readers - no network, no background export
        let provider = SdkMeterProvider::builder().build();
        let meter = provider.meter("test");

        Self {
            completed: meter.u64_counter("test_completed").build(),
            rejected: meter.u64_counter("test_rejected").build(),
            failed: meter.u64_counter("test_failed").build(),
            dropped: meter.u64_counter("test_dropped").build(),
            latency_us: meter.u64_histogram("test_latency").build(),
            _provider: None, // Don't keep provider - avoid shutdown delays
        }
    }
}

/// Test phase tracking
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestPhase {
    Warmup,
    Measurement,
}

impl TestPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            TestPhase::Warmup => "warmup",
            TestPhase::Measurement => "measurement",
        }
    }
}
