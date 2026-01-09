use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tracing::{debug, error, info};

/// Manages Docker Compose infrastructure
#[derive(Clone)]
pub struct DockerManager {
    compose_file: String,
    project_name: String,
}

impl DockerManager {
    pub fn new(compose_file: &str, project_name: &str) -> Self {
        Self {
            compose_file: compose_file.to_string(),
            project_name: project_name.to_string(),
        }
    }

    /// Start the Docker Compose stack
    pub async fn start(&self) -> Result<()> {
        info!("Starting Docker Compose stack...");

        let output = Command::new("docker")
            .args([
                "compose",
                "-f",
                &self.compose_file,
                "-p",
                &self.project_name,
                "up",
                "-d",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to execute docker compose up")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!("Docker compose up failed: {}", stderr);
            anyhow::bail!("Docker compose up failed: {}", stderr);
        }

        info!("Docker Compose stack started");
        Ok(())
    }

    /// Stop the Docker Compose stack
    pub async fn stop(&self) -> Result<()> {
        info!("Stopping Docker Compose stack...");

        let output = Command::new("docker")
            .args([
                "compose",
                "-f",
                &self.compose_file,
                "-p",
                &self.project_name,
                "down",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to execute docker compose down")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!("Docker compose down failed: {}", stderr);
            // Don't fail on stop errors
        }

        info!("Docker Compose stack stopped");
        Ok(())
    }

    /// Wait for PostgreSQL to be ready
    async fn wait_for_postgres(&self, timeout: Duration) -> Result<()> {
        info!("Waiting for PostgreSQL to be ready...");
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > timeout {
                anyhow::bail!("Timeout waiting for PostgreSQL to be ready");
            }

            let output = Command::new("docker")
                .args([
                    "compose",
                    "-f",
                    &self.compose_file,
                    "-p",
                    &self.project_name,
                    "exec",
                    "-T",
                    "postgres",
                    "pg_isready",
                    "-U",
                    "postgres",
                    "-d",
                    "tbperf",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;

            match output {
                Ok(status) if status.success() => {
                    info!("PostgreSQL is ready");
                    return Ok(());
                }
                _ => {
                    debug!("PostgreSQL not ready yet, retrying...");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }

    /// Wait for all services to be healthy
    pub async fn wait_for_services(&self, timeout: Duration) -> Result<()> {
        self.wait_for_postgres(timeout).await?;
        // Could add checks for other services here
        Ok(())
    }

    /// Execute a command in the postgres container
    /// Uses -t (tuples-only) for machine-readable output without headers
    pub async fn exec_postgres(&self, command: &str) -> Result<String> {
        let output = Command::new("docker")
            .args([
                "compose",
                "-f",
                &self.compose_file,
                "-p",
                &self.project_name,
                "exec",
                "-T",
                "postgres",
                "psql",
                "-U",
                "postgres",
                "-d",
                "tbperf",
                "-t",
                "-c",
                command,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to execute psql command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("psql command failed: {}", stderr);
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

/// Find the docker compose file relative to the config file
pub fn find_compose_file(config_path: &str, database_type: &str) -> Result<String> {
    let config_dir = Path::new(config_path).parent().unwrap_or(Path::new("."));

    let compose_filename = match database_type {
        "postgresql" => "docker-compose.postgresql.yml",
        "tigerbeetle" => "docker-compose.tigerbeetle.yml",
        _ => anyhow::bail!("Unknown database type: {}", database_type),
    };

    let compose_path = config_dir.join("docker").join(compose_filename);

    if compose_path.exists() {
        Ok(compose_path.to_string_lossy().to_string())
    } else {
        // Try current directory
        let fallback = Path::new("docker").join(compose_filename);
        if fallback.exists() {
            Ok(fallback.to_string_lossy().to_string())
        } else {
            anyhow::bail!(
                "Docker compose file not found: {} or {}",
                compose_path.display(),
                fallback.display()
            )
        }
    }
}
