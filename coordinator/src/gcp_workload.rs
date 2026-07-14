use crate::client_deploy::ClientNode;
use crate::gcp::GcpRemote;
use anyhow::{Context, Result};
use std::time::Duration;
use tracing::{error, info};

const REMOTE_CONFIG_PATH: &str = "config.toml";
const REMOTE_BINARY_PATH: &str = "./client";
/// Buffer added on top of the workload's own warmup+measurement duration,
/// to account for connection setup/teardown - mirrors the local
/// `test_runner.rs`'s `total_duration + 60` pattern.
const TIMEOUT_BUFFER: Duration = Duration::from_secs(60);

/// Ship the config file to every client node - the client binary reads it
/// via `-c`, so it must exist on each node's own filesystem.
pub async fn deploy_config(
    remote: &GcpRemote,
    nodes: &[ClientNode],
    local_config_path: &str,
) -> Result<()> {
    for node in nodes {
        remote
            .copy_to(
                &node.name,
                &node.zone,
                local_config_path,
                REMOTE_CONFIG_PATH,
                false,
            )
            .await
            .with_context(|| format!("Failed to copy config to {}", node.name))?;
    }
    Ok(())
}

/// Run the client workload on every given node concurrently, so they start
/// close together and overlap for the full warmup+measurement duration.
/// Waits for all to complete; fails if any node's client process errors.
///
/// `db_args` are the already-formatted database connection flags (e.g.
/// `--tb-addresses=ip1:3000,ip2:3000,ip3:3000` or `--pg-host=ip --pg-port=5432`),
/// identical across all nodes - only `--instance-id` varies per node.
pub async fn run_workload(
    remote: &GcpRemote,
    nodes: &[ClientNode],
    db_args: &str,
    otel_endpoint: &str,
    warmup_duration_secs: u64,
    test_duration_secs: u64,
) -> Result<()> {
    info!("Starting client workload on {} node(s)...", nodes.len());

    let total_timeout =
        Duration::from_secs(warmup_duration_secs + test_duration_secs) + TIMEOUT_BUFFER;

    let mut handles = Vec::new();
    let num_client_nodes = nodes.len();

    for (index, node) in nodes.iter().enumerate() {
        let remote = remote.clone();
        let node = node.clone();
        let command = format!(
            "{binary} -c {config} --instance-id {index} --num-client-nodes {num_client_nodes} {db_args} --otel-endpoint {otel}",
            binary = REMOTE_BINARY_PATH,
            config = REMOTE_CONFIG_PATH,
            index = index,
            num_client_nodes = num_client_nodes,
            db_args = db_args,
            otel = otel_endpoint,
        );

        handles.push(tokio::spawn(async move {
            let result = remote
                .run_command_with_timeout(&node.name, &node.zone, &command, total_timeout)
                .await;
            (node.name, result)
        }));
    }

    let mut any_failed = false;
    for handle in handles {
        let (name, result) = handle.await.context("Client task panicked")?;
        match result {
            Ok(output) => info!("Client on {} completed:\n{}", name, output.trim()),
            Err(e) => {
                error!("Client on {} failed: {:?}", name, e);
                any_failed = true;
            }
        }
    }

    if any_failed {
        anyhow::bail!("One or more client nodes failed during workload execution");
    }

    info!("All client nodes completed successfully");
    Ok(())
}
