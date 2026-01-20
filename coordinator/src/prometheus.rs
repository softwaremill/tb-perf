use anyhow::{Context, Result};
use serde::Deserialize;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Client for querying Prometheus metrics
pub struct PrometheusClient {
    base_url: String,
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct PrometheusResponse {
    status: String,
    data: PrometheusData,
}

#[derive(Debug, Deserialize)]
struct PrometheusData {
    result: Vec<PrometheusResult>,
}

#[derive(Debug, Deserialize)]
struct PrometheusResult {
    value: (f64, String), // [timestamp, value]
}

/// Metrics collected from Prometheus
#[derive(Debug, Clone, Default)]
pub struct CollectedMetrics {
    pub completed_transfers: u64,
    pub rejected_transfers: u64,
    pub failed_transfers: u64,
    pub latency_p50_us: u64,
    pub latency_p95_us: u64,
    pub latency_p99_us: u64,
    pub latency_p999_us: u64,
}

impl std::fmt::Display for CollectedMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "completed={}, rejected={}, failed={}, latency_us(p50={}, p95={}, p99={}, p999={})",
            self.completed_transfers,
            self.rejected_transfers,
            self.failed_transfers,
            self.latency_p50_us,
            self.latency_p95_us,
            self.latency_p99_us,
            self.latency_p999_us
        )
    }
}

impl PrometheusClient {
    pub fn new(prometheus_url: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            base_url: prometheus_url.trim_end_matches('/').to_string(),
            client,
        }
    }

    /// Query Prometheus for a single instant value at a specific time
    async fn query_at(&self, query: &str, time_secs: f64) -> Result<Option<f64>> {
        let url = format!("{}/api/v1/query", self.base_url);

        debug!("Prometheus query at {}: {}", time_secs, query);

        let response = self
            .client
            .get(&url)
            .query(&[("query", query), ("time", &time_secs.to_string())])
            .send()
            .await
            .context("Failed to query Prometheus")?;

        if !response.status().is_success() {
            warn!("Prometheus query failed with status: {}", response.status());
            return Ok(None);
        }

        let prom_response: PrometheusResponse = response
            .json()
            .await
            .context("Failed to parse Prometheus response")?;

        if prom_response.status != "success" {
            warn!("Prometheus query returned non-success status");
            return Ok(None);
        }

        if prom_response.data.result.is_empty() {
            debug!("Prometheus query returned no results");
            return Ok(None);
        }

        // Parse the value from the first result
        let value_str = &prom_response.data.result[0].value.1;
        match value_str.parse::<f64>() {
            Ok(v) => Ok(Some(v)),
            Err(_) => {
                warn!("Failed to parse Prometheus value: {}", value_str);
                Ok(None)
            }
        }
    }

    /// Query a counter metric using increase() to get the delta over a time range
    async fn query_counter(
        &self,
        metric: &str,
        range: &str,
        query_time: f64,
    ) -> Result<Option<u64>> {
        let query = format!(
            "sum(increase({}{{phase=\"measurement\"}}[{}]))",
            metric, range
        );
        info!("Prometheus query: {}", query);
        let result = self.query_at(&query, query_time).await?;
        if result.is_none() {
            // Try without phase filter to see if metric exists at all
            let debug_query = format!("sum(increase({}[{}]))", metric, range);
            let debug_result = self.query_at(&debug_query, query_time).await?;
            if debug_result.is_some() {
                warn!(
                    "Metric {} exists but has no phase=\"measurement\" label (got {:?} without filter)",
                    metric, debug_result
                );
            }
        }
        Ok(result.map(|v| v.round() as u64))
    }

    /// Query a histogram metric for a specific quantile
    async fn query_histogram_quantile(
        &self,
        metric: &str,
        quantile: f64,
        range: &str,
        query_time: f64,
    ) -> Result<Option<u64>> {
        let query = format!(
            "histogram_quantile({}, sum(rate({}_bucket{{phase=\"measurement\"}}[{}])) by (le))",
            quantile, metric, range
        );
        Ok(self.query_at(&query, query_time).await?.map(|v| v as u64))
    }

    /// Collect all metrics for a test run
    ///
    /// Queries Prometheus at the current time with a range covering from measurement
    /// start to now. The `phase="measurement"` label filter ensures only measurement
    /// phase data is counted (not warmup or quiet period).
    ///
    /// - `measurement_start`: Unix timestamp when measurement phase began
    pub async fn collect_metrics(&self, measurement_start: f64) -> Result<CollectedMetrics> {
        use std::time::{SystemTime, UNIX_EPOCH};

        let mut metrics = CollectedMetrics::default();

        // Query at current time (after waiting for metrics to be scraped)
        // The phase="measurement" filter ensures we only count measurement data
        let query_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

        // Range covers from measurement start to now, plus 5s buffer for scrape timing.
        // This automatically accounts for the 15s wait period and any other delays.
        let range_secs = query_time - measurement_start + 5.0;
        let range = format!("{}s", range_secs.round() as u64);

        info!(
            "Querying metrics: range={}s, query_time={:.0}",
            range_secs.round(),
            query_time
        );

        // Metric names include tbperf_ prefix from OTel collector namespace config
        const COMPLETED: &str = "tbperf_transfers_completed_total";
        const REJECTED: &str = "tbperf_transfers_rejected_total";
        const FAILED: &str = "tbperf_transfers_failed_total";
        const LATENCY: &str = "tbperf_transfer_latency_us";

        // Query counters
        if let Some(v) = self.query_counter(COMPLETED, &range, query_time).await? {
            metrics.completed_transfers = v;
        }
        if let Some(v) = self.query_counter(REJECTED, &range, query_time).await? {
            metrics.rejected_transfers = v;
        }
        if let Some(v) = self.query_counter(FAILED, &range, query_time).await? {
            metrics.failed_transfers = v;
        }

        // Query latency percentiles
        if let Some(v) = self
            .query_histogram_quantile(LATENCY, 0.50, &range, query_time)
            .await?
        {
            metrics.latency_p50_us = v;
        }
        if let Some(v) = self
            .query_histogram_quantile(LATENCY, 0.95, &range, query_time)
            .await?
        {
            metrics.latency_p95_us = v;
        }
        if let Some(v) = self
            .query_histogram_quantile(LATENCY, 0.99, &range, query_time)
            .await?
        {
            metrics.latency_p99_us = v;
        }
        if let Some(v) = self
            .query_histogram_quantile(LATENCY, 0.999, &range, query_time)
            .await?
        {
            metrics.latency_p999_us = v;
        }

        info!("Collected metrics: {}", metrics);
        Ok(metrics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collected_metrics_default() {
        let metrics = CollectedMetrics::default();
        assert_eq!(metrics.completed_transfers, 0);
        assert_eq!(metrics.rejected_transfers, 0);
        assert_eq!(metrics.failed_transfers, 0);
        assert_eq!(metrics.latency_p50_us, 0);
    }

    #[test]
    fn test_prometheus_client_url_normalization() {
        let client = PrometheusClient::new("http://localhost:9090/");
        assert_eq!(client.base_url, "http://localhost:9090");

        let client2 = PrometheusClient::new("http://localhost:9090");
        assert_eq!(client2.base_url, "http://localhost:9090");
    }
}
