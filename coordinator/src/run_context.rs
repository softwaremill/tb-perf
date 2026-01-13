use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;

/// Manages the run directory and associated log files
pub struct RunContext {
    /// Path to the run directory
    pub run_dir: PathBuf,
    /// Path to client log file
    pub client_log_path: PathBuf,
    /// Path to coordinator log file
    pub coordinator_log_path: PathBuf,
    /// Path to docker log file
    pub docker_log_path: PathBuf,
    /// Path to results file
    pub results_path: PathBuf,
    /// Path to config copy
    pub config_path: PathBuf,
}

impl RunContext {
    /// Create a new run context with a timestamped directory
    pub fn new(base_path: &str) -> Result<Self> {
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let run_dir = PathBuf::from(base_path).join(format!("run_{}", timestamp));

        fs::create_dir_all(&run_dir)
            .with_context(|| format!("Failed to create run directory: {}", run_dir.display()))?;

        info!("Created run directory: {}", run_dir.display());

        Ok(Self {
            client_log_path: run_dir.join("client.log"),
            coordinator_log_path: run_dir.join("coordinator.log"),
            docker_log_path: run_dir.join("docker.log"),
            results_path: run_dir.join("results.json"),
            config_path: run_dir.join("config.toml"),
            run_dir,
        })
    }

    /// Copy the config file to the run directory
    pub fn copy_config(&self, source_path: &str) -> Result<()> {
        fs::copy(source_path, &self.config_path)
            .with_context(|| format!("Failed to copy config to {}", self.config_path.display()))?;
        info!("Config copied to: {}", self.config_path.display());
        Ok(())
    }

    /// Get the path for the coordinator log (for tracing setup)
    pub fn coordinator_log_path(&self) -> &Path {
        &self.coordinator_log_path
    }

    /// Get the results file path
    pub fn results_path(&self) -> &Path {
        &self.results_path
    }
}
