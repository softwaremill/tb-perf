use crate::gcp::GcpRemote;
use anyhow::{Context, Result};
use std::time::Duration;
use tracing::{info, warn};

const POSTGRESQL_SCRIPT: &str = "scripts/gcp-setup-postgresql.sh";
const TIGERBEETLE_SCRIPT: &str = "scripts/gcp-setup-tigerbeetle.sh";
const REMOTE_SCRIPT_PATH: &str = "/tmp/tb-perf-setup-db.sh";

/// One DB node's identity, as needed to drive cluster setup over SSH.
#[derive(Debug, Clone)]
pub struct DbNode {
    pub name: String,
    pub zone: String,
    pub internal_ip: String,
    /// External IP - used for direct connections from the coordinator
    /// (e.g. account initialization), which runs outside the VPC.
    pub external_ip: String,
}

/// Discover the DB nodes for `database_type` (provisioned by
/// terraform/database-cluster), ordered by replica index (see
/// `GcpInstance::index_label` / the `replica-id` label set by Terraform).
pub async fn discover_db_nodes(remote: &GcpRemote, database_type: &str) -> Result<Vec<DbNode>> {
    let label_filter = format!("app=tb-perf,role=db,db-type={}", database_type);
    let instances = remote.list_instances(&label_filter).await?;

    if instances.is_empty() {
        anyhow::bail!(
            "No DB instances found for database_type={} - has `terraform apply` been run in terraform/database-cluster?",
            database_type
        );
    }

    instances
        .into_iter()
        .map(|i| {
            let internal_ip = i
                .internal_ip()
                .with_context(|| format!("DB instance {} has no internal IP", i.name))?
                .to_string();
            let external_ip = i
                .external_ip()
                .with_context(|| format!("DB instance {} has no external IP", i.name))?
                .to_string();
            Ok(DbNode {
                name: i.name.clone(),
                zone: i.zone_name().to_string(),
                internal_ip,
                external_ip,
            })
        })
        .collect()
}

/// Bring up a fresh TigerBeetle cluster across the given nodes (one-time
/// setup per test session - see PLAN.md §3.2/§3.4). Wipes any existing data.
pub async fn setup_tigerbeetle_cluster(remote: &GcpRemote, nodes: &[DbNode]) -> Result<()> {
    let replica_count = nodes.len();
    let addresses: Vec<String> = nodes
        .iter()
        .map(|n| format!("{}:3000", n.internal_ip))
        .collect();
    let addresses_joined = addresses.join(" ");

    info!(
        "Setting up TigerBeetle {}-node cluster: {:?}",
        replica_count, addresses
    );

    for (index, node) in nodes.iter().enumerate() {
        info!("Configuring TigerBeetle replica {} on {}", index, node.name);

        remote
            .copy_to(
                &node.name,
                &node.zone,
                TIGERBEETLE_SCRIPT,
                REMOTE_SCRIPT_PATH,
                false,
            )
            .await
            .with_context(|| format!("Failed to copy setup script to {}", node.name))?;

        let command = format!(
            "chmod +x {path} && sudo {path} {index} {count} {addrs}",
            path = REMOTE_SCRIPT_PATH,
            index = index,
            count = replica_count,
            addrs = addresses_joined
        );

        remote
            .run_command(&node.name, &node.zone, &command)
            .await
            .with_context(|| format!("Failed to set up TigerBeetle on {}", node.name))?;
    }

    info!("TigerBeetle cluster setup complete");
    Ok(())
}

/// Bring up a fresh 3-node PostgreSQL synchronous replication cluster
/// (nodes[0] = primary, nodes[1..] = standbys). Requires exactly 3 nodes,
/// matching the symmetric topology from PLAN.md §3.1 and the fixed
/// 2-standby argument list `scripts/gcp-setup-postgresql.sh` expects.
pub async fn setup_postgresql_cluster(remote: &GcpRemote, nodes: &[DbNode]) -> Result<()> {
    if nodes.len() != 3 {
        anyhow::bail!(
            "PostgreSQL cluster setup requires exactly 3 nodes (1 primary + 2 standbys), got {}",
            nodes.len()
        );
    }

    let primary = &nodes[0];
    let standbys = &nodes[1..];
    let standby_names: Vec<String> = (1..=standbys.len())
        .map(|i| format!("standby{}", i))
        .collect();

    info!(
        "Setting up PostgreSQL cluster: primary={}, standbys={:?}",
        primary.name,
        standbys.iter().map(|n| &n.name).collect::<Vec<_>>()
    );

    // Primary must be up and reachable before standbys can pg_basebackup from it.
    remote
        .copy_to(
            &primary.name,
            &primary.zone,
            POSTGRESQL_SCRIPT,
            REMOTE_SCRIPT_PATH,
            false,
        )
        .await
        .with_context(|| format!("Failed to copy setup script to {}", primary.name))?;

    let primary_command = format!(
        "chmod +x {path} && sudo {path} primary {s1} {s2}",
        path = REMOTE_SCRIPT_PATH,
        s1 = standby_names[0],
        s2 = standby_names[1],
    );
    remote
        .run_command(&primary.name, &primary.zone, &primary_command)
        .await
        .with_context(|| format!("Failed to set up PostgreSQL primary on {}", primary.name))?;

    for (standby, standby_name) in standbys.iter().zip(standby_names.iter()) {
        info!(
            "Configuring PostgreSQL standby '{}' on {}",
            standby_name, standby.name
        );

        remote
            .copy_to(
                &standby.name,
                &standby.zone,
                POSTGRESQL_SCRIPT,
                REMOTE_SCRIPT_PATH,
                false,
            )
            .await
            .with_context(|| format!("Failed to copy setup script to {}", standby.name))?;

        let standby_command = format!(
            "chmod +x {path} && sudo {path} standby {primary_ip} {name}",
            path = REMOTE_SCRIPT_PATH,
            primary_ip = primary.internal_ip,
            name = standby_name,
        );
        remote
            .run_command(&standby.name, &standby.zone, &standby_command)
            .await
            .with_context(|| format!("Failed to set up PostgreSQL standby on {}", standby.name))?;
    }

    // Only now that standbys actually exist do we require synchronous
    // acknowledgment from one of them - enabling this any earlier (e.g. at
    // initial primary startup) deadlocks every write, including Docker's
    // own automatic `CREATE DATABASE` during first-time container init,
    // since there's nothing yet to satisfy the synchronous requirement.
    info!("Activating synchronous replication on primary...");
    // Two separate -c flags, NOT one -c with two ;-separated statements:
    // psql sends a single multi-statement -c string as one combined query,
    // which Postgres implicitly wraps in a transaction block - and ALTER
    // SYSTEM is explicitly disallowed inside a transaction block.
    let activate_command = format!(
        "sudo docker exec {container} psql -U postgres -c \"ALTER SYSTEM SET synchronous_standby_names = 'ANY 1 ({s1}, {s2})';\" -c \"SELECT pg_reload_conf();\"",
        container = "tb-perf-postgres",
        s1 = standby_names[0],
        s2 = standby_names[1],
    );
    remote
        .run_command(&primary.name, &primary.zone, &activate_command)
        .await
        .context("Failed to activate synchronous replication on primary")?;

    info!("PostgreSQL cluster setup complete");
    Ok(())
}

/// Sanity-check that synchronous replication actually came up after
/// `setup_postgresql_cluster` - logs a warning (but doesn't fail the run)
/// if fewer standbys are connected than expected, since replication can
/// take a few seconds to establish after the standby container starts.
pub async fn verify_postgresql_cluster(remote: &GcpRemote, nodes: &[DbNode]) -> Result<()> {
    if nodes.is_empty() {
        return Ok(());
    }
    let primary = &nodes[0];
    let expected_standbys = nodes.len() - 1;

    info!("Waiting for standbys to connect to primary...");
    tokio::time::sleep(Duration::from_secs(10)).await;

    let command = r#"sudo docker exec tb-perf-postgres psql -U postgres -t -A -F',' -c "SELECT application_name, state, sync_state FROM pg_stat_replication;""#;

    let output = remote
        .run_command(&primary.name, &primary.zone, command)
        .await
        .context("Failed to query pg_stat_replication on primary")?;

    let connected: Vec<&str> = output
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();

    info!("Primary replication status: {:?}", connected);

    if connected.len() < expected_standbys {
        warn!(
            "Expected {} standby(s) connected, found {} - replication may still be establishing (this does not fail the run)",
            expected_standbys,
            connected.len()
        );
    } else {
        info!(
            "All {} standby(s) connected and replicating",
            connected.len()
        );
    }

    Ok(())
}
