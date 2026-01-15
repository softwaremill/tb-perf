use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub workload: WorkloadConfig,
    pub database: DatabaseConfig,
    #[serde(default)]
    pub postgresql: Option<PostgresqlConfig>,
    #[serde(default)]
    pub tigerbeetle: Option<TigerBeetleConfig>,
    pub deployment: DeploymentConfig,
    pub coordinator: CoordinatorConfig,
    pub monitoring: MonitoringConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WorkloadConfig {
    pub test_mode: String,
    #[serde(default)]
    pub concurrency: Option<usize>,
    #[serde(default)]
    pub target_rate: Option<u64>,
    #[serde(default)]
    pub max_concurrency: Option<usize>,
    pub num_accounts: u64,
    pub zipfian_exponent: f64,
    pub initial_balance: u64,
    pub min_transfer_amount: u64,
    pub max_transfer_amount: u64,
    pub warmup_duration_secs: u64,
    pub test_duration_secs: u64,
}

impl WorkloadConfig {
    pub fn test_mode(&self) -> Result<TestMode, anyhow::Error> {
        match self.test_mode.as_str() {
            "max_throughput" => {
                let concurrency = self.concurrency.ok_or_else(|| {
                    anyhow::anyhow!("max_throughput mode requires 'concurrency' field")
                })?;
                Ok(TestMode::MaxThroughput { concurrency })
            }
            "fixed_rate" => {
                let target_rate = self.target_rate.ok_or_else(|| {
                    anyhow::anyhow!("fixed_rate mode requires 'target_rate' field")
                })?;
                let max_concurrency = self.max_concurrency.ok_or_else(|| {
                    anyhow::anyhow!("fixed_rate mode requires 'max_concurrency' field")
                })?;
                Ok(TestMode::FixedRate {
                    target_rate,
                    max_concurrency,
                })
            }
            _ => Err(anyhow::anyhow!("Invalid test_mode: {}", self.test_mode)),
        }
    }
}

#[derive(Debug, Clone)]
pub enum TestMode {
    MaxThroughput {
        concurrency: usize,
    },
    FixedRate {
        target_rate: u64,
        max_concurrency: usize,
    },
}

impl TestMode {
    /// Get the test mode as a string label for metrics
    pub fn as_str(&self) -> &'static str {
        match self {
            TestMode::MaxThroughput { .. } => "max_throughput",
            TestMode::FixedRate { .. } => "fixed_rate",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DatabaseConfig {
    #[serde(rename = "type")]
    pub kind: DatabaseType,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseType {
    PostgreSQL,
    TigerBeetle,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PostgresqlConfig {
    pub isolation_level: IsolationLevel,
    pub connection_pool_size: usize,
    pub connection_pool_min_idle: Option<usize>,
    /// Enable batched mode (single connection, batch transfers like TigerBeetle)
    #[serde(default)]
    pub batched_mode: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationLevel {
    ReadCommitted,
    RepeatableRead,
    Serializable,
}

impl IsolationLevel {
    /// Convert to SQL syntax string for SET TRANSACTION ISOLATION LEVEL
    pub fn as_sql_str(&self) -> &'static str {
        match self {
            IsolationLevel::ReadCommitted => "READ COMMITTED",
            IsolationLevel::RepeatableRead => "REPEATABLE READ",
            IsolationLevel::Serializable => "SERIALIZABLE",
        }
    }
}


#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TigerBeetleConfig {
    pub cluster_addresses: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeploymentConfig {
    #[serde(rename = "type")]
    pub kind: DeploymentType,
    pub num_db_nodes: usize,
    #[serde(default)]
    pub num_client_nodes: Option<usize>,
    #[serde(default)]
    pub aws_region: Option<String>,
    #[serde(default)]
    pub db_instance_type: Option<String>,
    #[serde(default)]
    pub client_instance_type: Option<String>,
    pub measure_network_latency: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DeploymentType {
    Local,
    Cloud,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CoordinatorConfig {
    pub test_runs: usize,
    pub max_variance_threshold: f64,
    pub max_error_rate: f64,
    pub metrics_export_path: String,
    pub keep_grafana_running: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MonitoringConfig {
    pub grafana_port: u16,
    pub prometheus_port: u16,
    #[serde(default = "default_otel_port")]
    pub otel_collector_port: u16,
}

fn default_otel_port() -> u16 {
    4317
}

impl Config {
    /// Load configuration from a TOML file
    pub fn from_file<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&contents)?;
        config.validate()?;
        Ok(config)
    }

    /// Validate configuration based on test mode and database type
    pub fn validate(&self) -> anyhow::Result<()> {
        // Validate test mode configuration (ensures required fields are present)
        self.workload.test_mode()?;

        // Validate database-specific configuration
        match self.database.kind {
            DatabaseType::PostgreSQL => {
                if self.postgresql.is_none() {
                    anyhow::bail!(
                        "PostgreSQL database type requires [postgresql] configuration section"
                    );
                }
                if let Some(ref pg) = self.postgresql
                    && pg.connection_pool_size == 0
                {
                    anyhow::bail!("connection_pool_size must be >= 1");
                }
            }
            DatabaseType::TigerBeetle => {
                if self.tigerbeetle.is_none() {
                    anyhow::bail!(
                        "TigerBeetle database type requires [tigerbeetle] configuration section"
                    );
                }
                if let Some(ref tb) = self.tigerbeetle
                    && tb.cluster_addresses.is_empty()
                {
                    anyhow::bail!("cluster_addresses must not be empty");
                }
            }
        }

        // Validate deployment-specific configuration
        if self.deployment.kind == DeploymentType::Cloud {
            if self.deployment.num_client_nodes.is_none() {
                anyhow::bail!("Cloud deployment requires num_client_nodes to be specified");
            }
            if self.deployment.aws_region.is_none() {
                anyhow::bail!("Cloud deployment requires aws_region to be specified");
            }
        }

        // Validate workload configuration
        // Transfers require at least 2 accounts (source and destination must differ)
        if self.workload.num_accounts < 2 {
            anyhow::bail!(
                "num_accounts must be >= 2 (transfers require different source and destination)"
            );
        }

        if self.workload.test_duration_secs == 0 {
            anyhow::bail!("test_duration_secs must be >= 1");
        }

        if self.workload.min_transfer_amount > self.workload.max_transfer_amount {
            anyhow::bail!(
                "min_transfer_amount ({}) must be <= max_transfer_amount ({})",
                self.workload.min_transfer_amount,
                self.workload.max_transfer_amount
            );
        }

        if self.workload.zipfian_exponent < 0.0 {
            anyhow::bail!(
                "zipfian_exponent must be >= 0.0, got {}",
                self.workload.zipfian_exponent
            );
        }

        if self.workload.zipfian_exponent.is_nan() || self.workload.zipfian_exponent.is_infinite() {
            anyhow::bail!("zipfian_exponent must be a finite number");
        }

        if self.coordinator.max_variance_threshold.is_nan()
            || self.coordinator.max_variance_threshold.is_infinite()
        {
            anyhow::bail!("max_variance_threshold must be a finite number");
        }

        if self.coordinator.test_runs == 0 {
            anyhow::bail!("test_runs must be >= 1");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Creates a valid test configuration that can be modified for specific test cases
    fn test_config() -> Config {
        Config {
            workload: WorkloadConfig {
                test_mode: "max_throughput".to_string(),
                concurrency: Some(10),
                target_rate: None,
                max_concurrency: None,
                num_accounts: 100000,
                zipfian_exponent: 1.0,
                initial_balance: 1000000,
                min_transfer_amount: 1,
                max_transfer_amount: 1000,
                warmup_duration_secs: 120,
                test_duration_secs: 300,
            },
            database: DatabaseConfig {
                kind: DatabaseType::PostgreSQL,
            },
            postgresql: Some(PostgresqlConfig {
                isolation_level: IsolationLevel::ReadCommitted,
                connection_pool_size: 20,
                connection_pool_min_idle: Some(20),
                batched_mode: false,
            }),
            tigerbeetle: None,
            deployment: DeploymentConfig {
                kind: DeploymentType::Local,
                num_db_nodes: 1,
                num_client_nodes: None,
                aws_region: None,
                db_instance_type: None,
                client_instance_type: None,
                measure_network_latency: false,
            },
            coordinator: CoordinatorConfig {
                test_runs: 3,
                max_variance_threshold: 0.1,
                max_error_rate: 0.05,
                metrics_export_path: "./results".to_string(),
                keep_grafana_running: false,
            },
            monitoring: MonitoringConfig {
                grafana_port: 3000,
                prometheus_port: 9090,
                otel_collector_port: 4317,
            },
        }
    }

    #[test]
    fn test_valid_config() {
        let config = test_config();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_missing_postgresql_config() {
        let mut config = test_config();
        config.postgresql = None;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_invalid_transfer_amounts() {
        let mut config = test_config();
        config.workload.min_transfer_amount = 1000;
        config.workload.max_transfer_amount = 1;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_missing_concurrency_for_max_throughput() {
        let mut config = test_config();
        config.workload.concurrency = None;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_fixed_rate_mode() {
        let mut config = test_config();
        config.workload.test_mode = "fixed_rate".to_string();
        config.workload.concurrency = None;
        config.workload.target_rate = Some(1000);
        config.workload.max_concurrency = Some(50);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_fixed_rate_missing_target_rate() {
        let mut config = test_config();
        config.workload.test_mode = "fixed_rate".to_string();
        config.workload.concurrency = None;
        config.workload.target_rate = None;
        config.workload.max_concurrency = Some(50);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_invalid_test_mode() {
        let mut config = test_config();
        config.workload.test_mode = "invalid_mode".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_zero_num_accounts() {
        let mut config = test_config();
        config.workload.num_accounts = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_one_num_accounts() {
        // Transfers require at least 2 accounts (source != destination)
        let mut config = test_config();
        config.workload.num_accounts = 1;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_zero_test_duration() {
        let mut config = test_config();
        config.workload.test_duration_secs = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_zero_connection_pool_size() {
        let mut config = test_config();
        if let Some(ref mut pg) = config.postgresql {
            pg.connection_pool_size = 0;
        }
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_tigerbeetle_config() {
        let mut config = test_config();
        config.database.kind = DatabaseType::TigerBeetle;
        config.postgresql = None;
        config.tigerbeetle = Some(TigerBeetleConfig {
            cluster_addresses: vec!["3000".to_string()],
        });
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_tigerbeetle_empty_addresses() {
        let mut config = test_config();
        config.database.kind = DatabaseType::TigerBeetle;
        config.postgresql = None;
        config.tigerbeetle = Some(TigerBeetleConfig {
            cluster_addresses: vec![],
        });
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_cloud_deployment_requires_region() {
        let mut config = test_config();
        config.deployment.kind = DeploymentType::Cloud;
        config.deployment.num_client_nodes = Some(2);
        config.deployment.aws_region = None;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_cloud_deployment_requires_client_nodes() {
        let mut config = test_config();
        config.deployment.kind = DeploymentType::Cloud;
        config.deployment.num_client_nodes = None;
        config.deployment.aws_region = Some("us-east-1".to_string());
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_negative_zipfian_exponent() {
        let mut config = test_config();
        config.workload.zipfian_exponent = -1.0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_nan_zipfian_exponent() {
        let mut config = test_config();
        config.workload.zipfian_exponent = f64::NAN;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_zero_test_runs() {
        let mut config = test_config();
        config.coordinator.test_runs = 0;
        assert!(config.validate().is_err());
    }
}
