use crate::gcp::GcpRemote;
use anyhow::{Context, Result};
use std::process::Stdio;
use tracing::info;

/// The monitoring instance's identity, as needed to deploy the observability
/// stack over SSH.
#[derive(Debug, Clone)]
pub struct MonitoringNode {
    pub name: String,
    pub zone: String,
    /// Internal IP - client/DB nodes push metrics here (see terraform/network's
    /// `allow_metrics_to_monitoring` firewall rule), staying off the public internet.
    pub internal_ip: String,
}

const LOCAL_TARBALL_PATH: &str = "/tmp/tb-perf-monitoring-stack.tar.gz";
const REMOTE_TARBALL_PATH: &str = "/tmp/tb-perf-monitoring-stack.tar.gz";
const REMOTE_DIR: &str = "tb-perf-monitoring";

/// Discover the monitoring instance (provisioned by terraform/monitoring).
pub async fn discover_monitoring_node(remote: &GcpRemote) -> Result<MonitoringNode> {
    let instances = remote.list_instances("app=tb-perf,role=monitoring").await?;

    let instance = instances.into_iter().next().context(
        "No monitoring instance found - has `terraform apply` been run in terraform/monitoring?",
    )?;

    let internal_ip = instance
        .internal_ip()
        .context("Monitoring instance has no internal IP")?
        .to_string();

    Ok(MonitoringNode {
        name: instance.name.clone(),
        zone: instance.zone_name().to_string(),
        internal_ip,
    })
}

/// Package just the files needed to run the monitoring stack, preserving
/// the docker/ + grafana/ relative layout that docker-compose.monitoring.yml
/// expects (it references ../grafana/... for Grafana provisioning).
fn package_monitoring_config() -> Result<String> {
    let output = std::process::Command::new("tar")
        .args([
            "czf",
            LOCAL_TARBALL_PATH,
            "docker/docker-compose.monitoring.yml",
            "docker/otel-collector-config.yaml",
            "docker/prometheus.yml",
            "grafana/provisioning",
            "grafana/dashboards",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("Failed to run tar")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to package monitoring config: {}", stderr);
    }

    Ok(LOCAL_TARBALL_PATH.to_string())
}

/// Deploy and (idempotently) start the OTel Collector + Prometheus + Grafana
/// stack on the monitoring instance. Safe to call every coordinator run -
/// `docker compose up -d` is a no-op if the stack is already running
/// unchanged, unlike the DB cluster setup this does NOT wipe any existing
/// data (Prometheus's TSDB persists across calls).
pub async fn deploy_monitoring_stack(remote: &GcpRemote, node: &MonitoringNode) -> Result<()> {
    info!("Deploying monitoring stack to {}...", node.name);

    let tarball_path = package_monitoring_config()?;

    remote
        .copy_to(
            &node.name,
            &node.zone,
            &tarball_path,
            REMOTE_TARBALL_PATH,
            false,
        )
        .await
        .with_context(|| format!("Failed to copy monitoring config to {}", node.name))?;

    let command = format!(
        "mkdir -p {dir} && tar xzf {tarball} -C {dir} && \
         cd {dir}/docker && sudo docker compose -f docker-compose.monitoring.yml -p tbperf-monitoring up -d",
        dir = REMOTE_DIR,
        tarball = REMOTE_TARBALL_PATH,
    );

    remote
        .run_command(&node.name, &node.zone, &command)
        .await
        .with_context(|| format!("Failed to start monitoring stack on {}", node.name))?;

    info!("Monitoring stack running on {}", node.name);
    Ok(())
}
