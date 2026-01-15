use anyhow::{Context, Result};
use std::future::Future;
use tb::error::{CreateAccountErrorKind, CreateAccountsError};
use tigerbeetle_unofficial as tb;
use tracing::{error, info, warn};

/// TigerBeetle maximum batch size per API operation.
/// TigerBeetle limits batches to 8190 items; we use 8189 for safety margin.
const BATCH_SIZE: usize = 8189;

/// Process items in batches, calling the provided function for each batch range.
///
/// Returns the sum of values returned by all batch operations.
async fn process_batched<F, Fut>(total: u64, mut process_batch: F) -> Result<u128>
where
    F: FnMut(u64, u64) -> Fut,
    Fut: Future<Output = Result<u128>>,
{
    let mut sum = 0u128;

    for batch_start in (0..total).step_by(BATCH_SIZE) {
        let batch_end = (batch_start + BATCH_SIZE as u64).min(total);
        sum += process_batch(batch_start, batch_end).await?;
    }

    Ok(sum)
}

/// Initialize accounts in TigerBeetle
///
/// Creates accounts with DEBITS_MUST_NOT_EXCEED_CREDITS flag, then funds them
/// from a "bank" account. TigerBeetle doesn't support setting initial balances
/// directly, so we transfer from a constraint-free bank account to each user account.
pub async fn init_accounts(
    cluster_addresses: &[String],
    num_accounts: u64,
    initial_balance: u64,
) -> Result<()> {
    info!(
        "Initializing {} TigerBeetle accounts with balance {}",
        num_accounts, initial_balance
    );

    let cluster_id = 0;
    let addresses = cluster_addresses.join(",");
    let client =
        tb::Client::new(cluster_id, &addresses).context("Failed to create TigerBeetle client")?;

    // Create accounts in batches (1-based IDs, since TigerBeetle reserves ID 0)
    let created = process_batched(num_accounts, |batch_start, batch_end| {
        let client = &client;
        async move {
            let accounts: Vec<tb::Account> = (batch_start..batch_end)
                .map(|id| {
                    tb::Account::new((id + 1) as u128, 1, 1)
                        .with_flags(tb::account::Flags::DEBITS_MUST_NOT_EXCEED_CREDITS)
                })
                .collect();

            let account_count = accounts.len() as u128;
            match client.create_accounts(accounts).await {
                Ok(()) => Ok(account_count),
                Err(CreateAccountsError::Api(api_err)) => {
                    for err in api_err.as_slice() {
                        if !matches!(err.kind(), CreateAccountErrorKind::Exists) {
                            warn!(
                                "Account creation error at index {}: {:?}",
                                err.index(),
                                err.kind()
                            );
                        }
                    }
                    // Return count of newly created (total minus errors)
                    Ok(account_count - api_err.as_slice().len() as u128)
                }
                Err(CreateAccountsError::Send(e)) => {
                    Err(anyhow::anyhow!("Failed to create accounts: {:?}", e))
                }
                Err(e) => Err(anyhow::anyhow!("Failed to create accounts: {:?}", e)),
            }
        }
    })
    .await?;

    info!(
        "TigerBeetle accounts: {} created or already existed",
        created
    );

    // Fund accounts from a "bank" account (no balance constraints)
    info!(
        "Funding accounts with initial balance {}...",
        initial_balance
    );

    let bank_id: u128 = u128::MAX - 1;
    let bank_account = tb::Account::new(bank_id, 1, 1);

    match client.create_accounts(vec![bank_account]).await {
        Ok(()) => info!("Bank account created"),
        Err(CreateAccountsError::Api(api_err)) => {
            for err in api_err.as_slice() {
                if !matches!(err.kind(), CreateAccountErrorKind::Exists) {
                    warn!("Bank account creation error: {:?}", err.kind());
                }
            }
        }
        Err(e) => warn!("Bank account creation failed: {:?}", e),
    }

    // Transfer initial balance from bank to each account
    let funded = process_batched(num_accounts, |batch_start, batch_end| {
        let client = &client;
        async move {
            let transfers: Vec<tb::Transfer> = (batch_start..batch_end)
                .map(|id| {
                    tb::Transfer::new(tb::id())
                        .with_debit_account_id(bank_id)
                        .with_credit_account_id((id + 1) as u128)
                        .with_amount(initial_balance as u128)
                        .with_ledger(1)
                        .with_code(1)
                })
                .collect();

            let batch_count = transfers.len() as u128;
            if let Err(e) = client.create_transfers(transfers).await {
                warn!(
                    "Failed to fund accounts {}-{}: {:?}",
                    batch_start, batch_end, e
                );
                Ok(0)
            } else {
                Ok(batch_count)
            }
        }
    })
    .await?;

    info!("Funded {} accounts", funded);
    Ok(())
}

/// Verify total balance for correctness checking
///
/// Sums up (credits_posted - debits_posted) across all accounts to verify
/// that no money was created or destroyed during the workload.
pub async fn verify_total_balance(
    cluster_addresses: &[String],
    num_accounts: u64,
    expected_total: u64,
) -> Result<bool> {
    info!(
        "Verifying TigerBeetle total balance (expected: {})",
        expected_total
    );

    let cluster_id = 0;
    let addresses = cluster_addresses.join(",");
    let client =
        tb::Client::new(cluster_id, &addresses).context("Failed to create TigerBeetle client")?;

    let total_balance = process_batched(num_accounts, |batch_start, batch_end| {
        let client = &client;
        async move {
            let ids: Vec<u128> = (batch_start..batch_end).map(|id| (id + 1) as u128).collect();

            let accounts = client
                .lookup_accounts(ids)
                .await
                .context("Failed to lookup accounts")?;

            let batch_balance: u128 = accounts
                .iter()
                .map(|a| a.credits_posted().saturating_sub(a.debits_posted()))
                .sum();

            Ok(batch_balance)
        }
    })
    .await?;

    let is_correct = total_balance == expected_total as u128;
    if is_correct {
        info!("Balance verification passed: {}", total_balance);
    } else {
        error!(
            "Balance verification FAILED: expected {}, got {}",
            expected_total, total_balance
        );
    }

    Ok(is_correct)
}

/// Wait for TigerBeetle to be ready to accept API requests
///
/// Polls the cluster until it can successfully execute a lookup operation.
/// This is more reliable than just checking if the port is open.
pub async fn wait_for_ready(cluster_addresses: &[String], timeout_secs: u64) -> Result<()> {
    info!("Waiting for TigerBeetle to be ready...");

    let cluster_id = 0;
    let addresses = cluster_addresses.join(",");
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(timeout_secs);

    loop {
        if start.elapsed() > timeout {
            anyhow::bail!("Timeout waiting for TigerBeetle to be ready");
        }

        match tb::Client::new(cluster_id, &addresses) {
            Ok(client) => {
                // Try a simple operation to verify connection
                match client.lookup_accounts(vec![0u128]).await {
                    Ok(_) => {
                        info!("TigerBeetle is ready");
                        return Ok(());
                    }
                    Err(e) => {
                        tracing::debug!("TigerBeetle not ready yet: {:?}", e);
                    }
                }
            }
            Err(e) => {
                tracing::debug!("Cannot connect to TigerBeetle: {:?}", e);
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_process_batched_empty() {
        let result = process_batched(0, |_, _| async { Ok(1u128) })
            .await
            .unwrap();
        assert_eq!(result, 0);
    }

    #[tokio::test]
    async fn test_process_batched_single_batch() {
        let result = process_batched(100, |start, end| async move { Ok((end - start) as u128) })
            .await
            .unwrap();
        assert_eq!(result, 100);
    }

    #[tokio::test]
    async fn test_process_batched_multiple_batches() {
        // Use a smaller batch size for testing by processing more than BATCH_SIZE items
        let total = (BATCH_SIZE as u64) * 2 + 500; // 2.5 batches worth

        let mut batch_count = 0u64;
        let result = process_batched(total, |start, end| {
            batch_count += 1;
            async move { Ok((end - start) as u128) }
        })
        .await
        .unwrap();

        assert_eq!(result, total as u128);
        assert_eq!(batch_count, 3); // Should be 3 batches
    }

    #[tokio::test]
    async fn test_process_batched_exact_batch_boundary() {
        let total = BATCH_SIZE as u64; // Exactly one batch

        let mut batch_count = 0u64;
        let result = process_batched(total, |start, end| {
            batch_count += 1;
            async move { Ok((end - start) as u128) }
        })
        .await
        .unwrap();

        assert_eq!(result, total as u128);
        assert_eq!(batch_count, 1);
    }

    #[tokio::test]
    async fn test_process_batched_propagates_error() {
        let result: Result<u128> =
            process_batched(100, |_, _| async { Err(anyhow::anyhow!("test error")) }).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("test error"));
    }
}
