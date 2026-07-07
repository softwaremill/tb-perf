use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::process::Stdio;
use tokio::process::Command;
use tracing::{debug, warn};

/// A GCE instance as reported by `gcloud compute instances list --format=json`.
/// Only the fields tb-perf actually needs are modeled here.
#[derive(Debug, Clone, Deserialize)]
pub struct GcpInstance {
    pub name: String,
    /// Full zone URL, e.g. ".../zones/europe-central2-a" - use `zone_name()`.
    pub zone: String,
    #[serde(rename = "networkInterfaces", default)]
    pub network_interfaces: Vec<NetworkInterface>,
    #[serde(default)]
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NetworkInterface {
    #[serde(rename = "networkIP")]
    pub network_ip: Option<String>,
    #[serde(rename = "accessConfigs", default)]
    pub access_configs: Vec<AccessConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AccessConfig {
    #[serde(rename = "natIP")]
    pub nat_ip: Option<String>,
}

impl GcpInstance {
    /// Short zone name (e.g. "europe-central2-a") parsed out of the full zone URL.
    pub fn zone_name(&self) -> &str {
        self.zone.rsplit('/').next().unwrap_or(&self.zone)
    }

    pub fn internal_ip(&self) -> Option<&str> {
        self.network_interfaces.first()?.network_ip.as_deref()
    }

    pub fn external_ip(&self) -> Option<&str> {
        self.network_interfaces
            .first()?
            .access_configs
            .first()?
            .nat_ip
            .as_deref()
    }

    /// Replica/client index parsed from the "replica-id"/"client-id" label
    /// set by the corresponding Terraform module, used to order nodes
    /// deterministically (e.g. replica 0 = PostgreSQL primary).
    pub fn index_label(&self) -> Option<usize> {
        self.labels
            .get("replica-id")
            .or_else(|| self.labels.get("client-id"))
            .and_then(|v| v.parse().ok())
    }
}

/// Builds a `gcloud` `--filter` expression from a simple `key=value,key=value`
/// label filter string. Split out as a pure function so it's testable
/// without invoking `gcloud`.
fn build_label_filter(label_filter: &str) -> String {
    label_filter
        .split(',')
        .filter(|kv| !kv.is_empty())
        .map(|kv| {
            let mut parts = kv.splitn(2, '=');
            let key = parts.next().unwrap_or("").trim();
            let value = parts.next().unwrap_or("").trim();
            format!("labels.{}={}", key, value)
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

/// Remote execution over GCP: instance discovery via labels, and command
/// execution/file transfer via `gcloud compute ssh/scp --tunnel-through-iap`.
///
/// This is the cloud equivalent of `DockerManager` for local Docker Compose
/// orchestration - see PLAN.md §3.1 "Remote command execution".
#[derive(Clone)]
pub struct GcpRemote {
    project: String,
}

impl GcpRemote {
    pub fn new(project: &str) -> Self {
        Self {
            project: project.to_string(),
        }
    }

    /// List instances matching a label filter, e.g. "app=tb-perf,role=db".
    pub async fn list_instances(&self, label_filter: &str) -> Result<Vec<GcpInstance>> {
        let filter_expr = build_label_filter(label_filter);

        debug!("gcloud compute instances list --filter={}", filter_expr);

        let output = Command::new("gcloud")
            .args([
                "compute",
                "instances",
                "list",
                "--project",
                &self.project,
                "--filter",
                &filter_expr,
                "--format",
                "json",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to execute gcloud compute instances list")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("gcloud compute instances list failed: {}", stderr);
        }

        let mut instances: Vec<GcpInstance> = serde_json::from_slice(&output.stdout)
            .context("Failed to parse gcloud compute instances list JSON")?;

        // Sort by replica/client index label where present, so callers get a
        // deterministic node ordering (e.g. replica 0 = PostgreSQL primary).
        instances.sort_by_key(|i| i.index_label().unwrap_or(usize::MAX));

        Ok(instances)
    }

    /// Run a command on a remote instance via IAP-tunneled SSH, returning stdout.
    /// Fails if the remote command exits non-zero.
    pub async fn run_command(&self, instance: &str, zone: &str, command: &str) -> Result<String> {
        debug!("[{}] running: {}", instance, command);

        let output = Command::new("gcloud")
            .args([
                "compute",
                "ssh",
                instance,
                "--project",
                &self.project,
                "--zone",
                zone,
                "--tunnel-through-iap",
                "--command",
                command,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .with_context(|| format!("Failed to SSH into {}", instance))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Remote command on {} failed: {}", instance, stderr);
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.trim().is_empty() {
            // gcloud/ssh often print warnings (host key, MOTD) to stderr even
            // on success - log but don't fail.
            debug!("[{}] stderr: {}", instance, stderr.trim());
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Run a command on a remote instance, warning (not failing) on error.
    /// Useful for best-effort cleanup steps.
    pub async fn run_command_best_effort(&self, instance: &str, zone: &str, command: &str) {
        if let Err(e) = self.run_command(instance, zone, command).await {
            warn!("[{}] best-effort command failed: {:?}", instance, e);
        }
    }

    /// Copy a local file/directory to a remote instance via IAP-tunneled scp.
    pub async fn copy_to(
        &self,
        instance: &str,
        zone: &str,
        local_path: &str,
        remote_path: &str,
        recurse: bool,
    ) -> Result<()> {
        let mut args = vec!["compute".to_string(), "scp".to_string()];
        if recurse {
            args.push("--recurse".to_string());
        }
        args.extend([
            local_path.to_string(),
            format!("{}:{}", instance, remote_path),
            "--project".to_string(),
            self.project.clone(),
            "--zone".to_string(),
            zone.to_string(),
            "--tunnel-through-iap".to_string(),
        ]);

        debug!("[{}] scp {} -> {}", instance, local_path, remote_path);

        let output = Command::new("gcloud")
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .with_context(|| format!("Failed to scp to {}", instance))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("scp to {} failed: {}", instance, stderr);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_label_filter_single() {
        assert_eq!(build_label_filter("role=db"), "labels.role=db");
    }

    #[test]
    fn test_build_label_filter_multiple() {
        assert_eq!(
            build_label_filter("app=tb-perf,role=db"),
            "labels.app=tb-perf AND labels.role=db"
        );
    }

    #[test]
    fn test_build_label_filter_with_hyphenated_keys() {
        assert_eq!(
            build_label_filter("db-type=tigerbeetle"),
            "labels.db-type=tigerbeetle"
        );
    }

    #[test]
    fn test_build_label_filter_trims_whitespace() {
        assert_eq!(
            build_label_filter("app = tb-perf, role = db"),
            "labels.app=tb-perf AND labels.role=db"
        );
    }

    #[test]
    fn test_zone_name_parses_full_url() {
        let instance = GcpInstance {
            name: "tb-perf-db-tigerbeetle-0".to_string(),
            zone: "https://www.googleapis.com/compute/v1/projects/p/zones/europe-central2-a"
                .to_string(),
            network_interfaces: vec![],
            labels: HashMap::new(),
        };
        assert_eq!(instance.zone_name(), "europe-central2-a");
    }

    #[test]
    fn test_zone_name_passthrough_if_no_slash() {
        let instance = GcpInstance {
            name: "x".to_string(),
            zone: "europe-central2-a".to_string(),
            network_interfaces: vec![],
            labels: HashMap::new(),
        };
        assert_eq!(instance.zone_name(), "europe-central2-a");
    }

    #[test]
    fn test_internal_and_external_ip() {
        let instance = GcpInstance {
            name: "x".to_string(),
            zone: "z".to_string(),
            network_interfaces: vec![NetworkInterface {
                network_ip: Some("10.10.0.5".to_string()),
                access_configs: vec![AccessConfig {
                    nat_ip: Some("34.1.2.3".to_string()),
                }],
            }],
            labels: HashMap::new(),
        };
        assert_eq!(instance.internal_ip(), Some("10.10.0.5"));
        assert_eq!(instance.external_ip(), Some("34.1.2.3"));
    }

    #[test]
    fn test_internal_ip_none_when_no_interfaces() {
        let instance = GcpInstance {
            name: "x".to_string(),
            zone: "z".to_string(),
            network_interfaces: vec![],
            labels: HashMap::new(),
        };
        assert_eq!(instance.internal_ip(), None);
        assert_eq!(instance.external_ip(), None);
    }

    #[test]
    fn test_index_label_from_replica_id() {
        let mut labels = HashMap::new();
        labels.insert("replica-id".to_string(), "2".to_string());
        let instance = GcpInstance {
            name: "x".to_string(),
            zone: "z".to_string(),
            network_interfaces: vec![],
            labels,
        };
        assert_eq!(instance.index_label(), Some(2));
    }

    #[test]
    fn test_index_label_from_client_id() {
        let mut labels = HashMap::new();
        labels.insert("client-id".to_string(), "1".to_string());
        let instance = GcpInstance {
            name: "x".to_string(),
            zone: "z".to_string(),
            network_interfaces: vec![],
            labels,
        };
        assert_eq!(instance.index_label(), Some(1));
    }

    #[test]
    fn test_index_label_missing() {
        let instance = GcpInstance {
            name: "x".to_string(),
            zone: "z".to_string(),
            network_interfaces: vec![],
            labels: HashMap::new(),
        };
        assert_eq!(instance.index_label(), None);
    }
}
