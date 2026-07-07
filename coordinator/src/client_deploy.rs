use crate::gcp::GcpRemote;
use anyhow::{Context, Result};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command as TokioCommand;
use tracing::info;

/// One client node's identity, as needed to deploy/run the client binary.
#[derive(Debug, Clone)]
pub struct ClientNode {
    pub name: String,
    pub zone: String,
}

const REMOTE_TARBALL_PATH: &str = "/tmp/tb-perf-src.tar.gz";
const REMOTE_SRC_DIR: &str = "tb-perf";
/// Building the whole workspace (including tigerbeetle-unofficial-sys's
/// native build script) from scratch on a modest 2-vCPU instance can
/// legitimately take several minutes.
const BUILD_TIMEOUT: Duration = Duration::from_secs(20 * 60);

/// Discover client instances (provisioned by terraform/client-cluster),
/// ordered by client index (see `GcpInstance::index_label`).
pub async fn discover_client_nodes(remote: &GcpRemote) -> Result<Vec<ClientNode>> {
    let instances = remote.list_instances("app=tb-perf,role=client").await?;

    if instances.is_empty() {
        anyhow::bail!(
            "No client instances found - has `terraform apply` been run in terraform/client-cluster?"
        );
    }

    Ok(instances
        .into_iter()
        .map(|i| ClientNode {
            name: i.name.clone(),
            zone: i.zone_name().to_string(),
        })
        .collect())
}

/// Package the local project source into a tarball for shipping to client
/// nodes. Cross-compiling from a laptop (e.g. macOS/arm64) to the client
/// nodes' Linux/x86_64 target is avoided entirely - instead, the whole
/// workspace source is shipped and built natively on each client node
/// (which already has Rust installed via its Terraform startup-script).
/// This is simpler and more robust than cross-compilation, at the cost of a
/// slower first build (subsequent builds mostly hit each node's local
/// cargo registry/target cache).
fn package_source() -> Result<String> {
    let tarball_path = "/tmp/tb-perf-src-local.tar.gz";

    info!("Packaging project source into {}...", tarball_path);

    let output = std::process::Command::new("tar")
        .args([
            "czf",
            tarball_path,
            "--exclude=target",
            "--exclude=.git",
            "--exclude=results",
            "--exclude=local-results",
            "-C",
            ".",
            ".",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("Failed to run tar")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to package source: {}", stderr);
    }

    Ok(tarball_path.to_string())
}

/// Deploy and build the client binary on every given client node: ship the
/// packaged source via scp, extract, and `cargo build --release --bin
/// client`. Idempotent - safe to re-run (overwrites the previous source
/// tree and rebuilds; cargo's own incremental caching keeps subsequent
/// builds fast).
pub async fn build_client_binary(remote: &GcpRemote, nodes: &[ClientNode]) -> Result<()> {
    let tarball_path = package_source()?;

    for node in nodes {
        info!("Deploying client source to {}...", node.name);

        remote
            .copy_to(
                &node.name,
                &node.zone,
                &tarball_path,
                REMOTE_TARBALL_PATH,
                false,
            )
            .await
            .with_context(|| format!("Failed to copy source tarball to {}", node.name))?;

        let extract_command = format!(
            "mkdir -p {dir} && tar xzf {tarball} -C {dir}",
            dir = REMOTE_SRC_DIR,
            tarball = REMOTE_TARBALL_PATH,
        );
        remote
            .run_command(&node.name, &node.zone, &extract_command)
            .await
            .with_context(|| format!("Failed to extract source on {}", node.name))?;

        info!(
            "Building client binary on {} (this can take several minutes on first build)...",
            node.name
        );
        // Reference the toolchain by full path rather than relying on
        // $HOME/.cargo or shell profile sourcing: it's installed to a
        // shared /opt/rust location (see terraform/client-cluster's
        // startup-script), and non-interactive SSH --command invocations
        // don't source /etc/profile.d anyway.
        let build_command = format!(
            "cd {dir} && /opt/rust/cargo/bin/cargo build --release --bin client",
            dir = REMOTE_SRC_DIR,
        );
        remote
            .run_command_with_timeout(&node.name, &node.zone, &build_command, BUILD_TIMEOUT)
            .await
            .with_context(|| format!("Failed to build client binary on {}", node.name))?;

        info!("Client binary built on {}", node.name);
    }

    // Clean up the local tarball - it's only needed transiently for upload.
    let _ = TokioCommand::new("rm").arg(&tarball_path).output().await;

    Ok(())
}
