use crate::gcp::GcpRemote;
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use tracing::info;

/// One client node's identity, as needed to deploy the client binary.
#[derive(Debug, Clone)]
pub struct ClientNode {
    pub name: String,
    pub zone: String,
}

const CROSS_TARGET: &str = "x86_64-unknown-linux-gnu";
const LOCAL_BINARY_PATH: &str = "target/x86_64-unknown-linux-gnu/release/client";
const REMOTE_BINARY_PATH: &str = "client";

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

/// Cross-compile the client binary locally for the client nodes' target
/// (Linux/x86_64) via `cargo zigbuild`. Requires, on the machine running the
/// coordinator: `rustup target add x86_64-unknown-linux-gnu`, `cargo install
/// cargo-zigbuild`, and `brew install zig llvm` (the latter for bindgen's
/// libclang requirement - see README.md's Cloud Testing section).
fn cross_compile_client() -> Result<String> {
    info!("Cross-compiling client binary for {}...", CROSS_TARGET);

    let libclang_path = std::process::Command::new("brew")
        .args(["--prefix", "llvm"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| format!("{}/lib", String::from_utf8_lossy(&o.stdout).trim()));

    let mut cmd = std::process::Command::new("cargo");
    cmd.args([
        "zigbuild",
        "--release",
        "--target",
        CROSS_TARGET,
        "--bin",
        "client",
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());

    if let Some(path) = &libclang_path {
        cmd.env("LIBCLANG_PATH", path);
    }

    let output = cmd.output().context(
        "Failed to run `cargo zigbuild` - is it installed? (cargo install cargo-zigbuild, \
         rustup target add x86_64-unknown-linux-gnu, brew install zig llvm)",
    )?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("cargo zigbuild failed: {}", stderr);
    }

    if !Path::new(LOCAL_BINARY_PATH).exists() {
        anyhow::bail!(
            "cargo zigbuild reported success but {} doesn't exist",
            LOCAL_BINARY_PATH
        );
    }

    info!("Cross-compiled binary ready: {}", LOCAL_BINARY_PATH);
    Ok(LOCAL_BINARY_PATH.to_string())
}

/// Deploy the client binary to every given client node: cross-compile once
/// locally, then scp the resulting binary directly. No build step (Rust
/// toolchain, libclang, etc.) is needed on the nodes themselves at all.
pub async fn deploy_client_binary(remote: &GcpRemote, nodes: &[ClientNode]) -> Result<()> {
    let binary_path = cross_compile_client()?;

    for node in nodes {
        info!("Deploying client binary to {}...", node.name);

        remote
            .copy_to(
                &node.name,
                &node.zone,
                &binary_path,
                REMOTE_BINARY_PATH,
                false,
            )
            .await
            .with_context(|| format!("Failed to copy client binary to {}", node.name))?;

        remote
            .run_command(
                &node.name,
                &node.zone,
                &format!("chmod +x {}", REMOTE_BINARY_PATH),
            )
            .await
            .with_context(|| format!("Failed to chmod client binary on {}", node.name))?;

        info!("Client binary deployed to {}", node.name);
    }

    Ok(())
}
