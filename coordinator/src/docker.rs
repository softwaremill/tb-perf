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

    /// Wait for TigerBeetle container to be running
    ///
    /// This checks if the TigerBeetle process is running in the container.
    /// Actual API readiness should be verified using tigerbeetle_setup::wait_for_ready().
    async fn wait_for_tigerbeetle(&self, timeout: Duration) -> Result<()> {
        info!("Waiting for TigerBeetle container to be ready...");
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > timeout {
                anyhow::bail!("Timeout waiting for TigerBeetle to be ready");
            }

            // Check if the TigerBeetle process is running in the container
            // (TigerBeetle's minimal image doesn't include nc or curl)
            let output = Command::new("docker")
                .args([
                    "compose",
                    "-f",
                    &self.compose_file,
                    "-p",
                    &self.project_name,
                    "exec",
                    "-T",
                    "tigerbeetle",
                    "pgrep",
                    "tigerbeetle",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;

            match output {
                Ok(status) if status.success() => {
                    info!("TigerBeetle is ready");
                    return Ok(());
                }
                _ => {
                    debug!("TigerBeetle not ready yet, retrying...");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }

    /// Wait for PostgreSQL services to be healthy
    pub async fn wait_for_postgres_services(&self, timeout: Duration) -> Result<()> {
        self.wait_for_postgres(timeout).await?;
        Ok(())
    }

    /// Wait for TigerBeetle services to be healthy
    pub async fn wait_for_tigerbeetle_services(&self, timeout: Duration) -> Result<()> {
        self.wait_for_tigerbeetle(timeout).await?;
        Ok(())
    }

    /// Wait for all services to be healthy (generic - PostgreSQL)
    /// Reserved for future use when a generic service check is needed.
    #[allow(dead_code)]
    pub async fn wait_for_services(&self, timeout: Duration) -> Result<()> {
        self.wait_for_postgres(timeout).await?;
        Ok(())
    }

    /// Restart a specific service (useful for TigerBeetle data reset)
    pub async fn restart_service(&self, service: &str) -> Result<()> {
        info!("Restarting {} service...", service);

        // Stop the service
        let output = Command::new("docker")
            .args([
                "compose",
                "-f",
                &self.compose_file,
                "-p",
                &self.project_name,
                "rm",
                "-f",
                "-s",
                "-v",
                service,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to remove service")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!("Failed to remove {}: {}", service, stderr);
        }

        // Start the service again
        let output = Command::new("docker")
            .args([
                "compose",
                "-f",
                &self.compose_file,
                "-p",
                &self.project_name,
                "up",
                "-d",
                service,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to start service")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to start {}: {}", service, stderr);
        }

        info!("{} service restarted", service);
        Ok(())
    }

    /// Get logs from all containers
    pub async fn get_logs(&self) -> Result<String> {
        let output = Command::new("docker")
            .args([
                "compose",
                "-f",
                &self.compose_file,
                "-p",
                &self.project_name,
                "logs",
                "--no-color",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to get docker logs")?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        Ok(format!("{}{}", stdout, stderr))
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
