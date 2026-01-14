use crate::metrics::WorkloadMetrics;
use crate::workload::{TransferExecutor, TransferResult, WorkloadRunner};
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::sync::Arc;
use tb::error::{CreateTransferErrorKind, CreateTransfersError};
use tigerbeetle_unofficial as tb;
use tracing::{error, info, warn};

/// TigerBeetle ledger ID for all accounts and transfers
const LEDGER_ID: u32 = 1;
/// TigerBeetle transfer code (application-defined transfer type)
const TRANSFER_CODE: u16 = 1;

/// TigerBeetle transfer executor
///
/// Handles executing transfers against TigerBeetle.
#[derive(Clone)]
pub struct TigerBeetleExecutor {
    client: Arc<tb::Client>,
}

#[async_trait]
impl TransferExecutor for TigerBeetleExecutor {
    async fn execute(&self, source: u64, dest: u64, amount: u64) -> Result<TransferResult> {
        execute_transfer(&self.client, source, dest, amount).await
    }
}

/// Create a TigerBeetle workload runner
pub async fn create_workload(
    cluster_addresses: &[String],
    num_accounts: u64,
    zipfian_exponent: f64,
    min_transfer_amount: u64,
    max_transfer_amount: u64,
    warmup_duration_secs: u64,
    test_duration_secs: u64,
    metrics: WorkloadMetrics,
) -> Result<WorkloadRunner<TigerBeetleExecutor>> {
    // TigerBeetle cluster ID 0 for local development
    let cluster_id = 0;

    // Join addresses with comma for TigerBeetle client
    let addresses = cluster_addresses.join(",");
    info!("Connecting to TigerBeetle cluster: {}", addresses);

    let client =
        tb::Client::new(cluster_id, &addresses).context("Failed to create TigerBeetle client")?;

    info!("Connected to TigerBeetle cluster");

    let executor = TigerBeetleExecutor {
        client: Arc::new(client),
    };

    Ok(WorkloadRunner::new(
        executor,
        num_accounts,
        zipfian_exponent,
        min_transfer_amount,
        max_transfer_amount,
        warmup_duration_secs,
        test_duration_secs,
        metrics,
    ))
}

/// Execute a single transfer
async fn execute_transfer(
    client: &tb::Client,
    source: u64,
    dest: u64,
    amount: u64,
) -> Result<TransferResult> {
    // Create the transfer using builder pattern
    let transfer = tb::Transfer::new(tb::id())
        .with_debit_account_id(source as u128)
        .with_credit_account_id(dest as u128)
        .with_amount(amount as u128)
        .with_ledger(LEDGER_ID)
        .with_code(TRANSFER_CODE);

    match client.create_transfers(vec![transfer]).await {
        Ok(()) => Ok(TransferResult::Success),
        Err(CreateTransfersError::Api(api_err)) => {
            // Check the first error (single transfer = single error)
            if let Some(err) = api_err.as_slice().first() {
                match err.kind() {
                    CreateTransferErrorKind::ExceedsCredits
                    | CreateTransferErrorKind::ExceedsDebits => {
                        return Ok(TransferResult::InsufficientBalance);
                    }
                    CreateTransferErrorKind::DebitAccountNotFound
                    | CreateTransferErrorKind::CreditAccountNotFound => {
                        return Ok(TransferResult::AccountNotFound);
                    }
                    _ => {
                        warn!("Transfer error: {:?}", err.kind());
                        return Ok(TransferResult::Failed);
                    }
                }
            }
            Ok(TransferResult::Failed)
        }
        Err(CreateTransfersError::Send(e)) => {
            error!("Send error: {:?}", e);
            Err(anyhow::anyhow!("Send error: {:?}", e))
        }
        Err(e) => {
            error!("Unexpected error: {:?}", e);
            Err(anyhow::anyhow!("Unexpected error: {:?}", e))
        }
    }
}
