use anyhow::{Context, Result};
use tb::error::{CreateAccountErrorKind, CreateAccountsError};
use tigerbeetle_unofficial as tb;
use tracing::{error, info, warn};

/// TigerBeetle maximum batch size per API operation.
/// TigerBeetle limits batches to 8190 items; we use 8189 for safety margin.
const BATCH_SIZE: usize = 8189;

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

    // Create accounts in batches
    let mut created = 0u64;
    let mut already_exist = 0u64;

    for batch_start in (0..num_accounts).step_by(BATCH_SIZE) {
        let batch_end = (batch_start + BATCH_SIZE as u64).min(num_accounts);

        let accounts: Vec<tb::Account> = (batch_start..batch_end)
            .map(|id| {
                tb::Account::new(id as u128, 1, 1)
                    .with_flags(tb::account::Flags::DEBITS_MUST_NOT_EXCEED_CREDITS)
            })
            .collect();

        let account_count = accounts.len();
        match client.create_accounts(accounts).await {
            Ok(()) => {
                created += account_count as u64;
            }
            Err(CreateAccountsError::Api(api_err)) => {
                for err in api_err.as_slice() {
                    if matches!(err.kind(), CreateAccountErrorKind::Exists) {
                        already_exist += 1;
                    } else {
                        warn!(
                            "Account creation error at index {}: {:?}",
                            err.index(),
                            err.kind()
                        );
                    }
                }
                // The accounts that didn't error were created
                created += (account_count - api_err.as_slice().len()) as u64;
            }
            Err(CreateAccountsError::Send(e)) => {
                return Err(anyhow::anyhow!("Failed to create accounts: {:?}", e));
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to create accounts: {:?}", e));
            }
        }
    }

    info!(
        "TigerBeetle accounts: {} created, {} already existed",
        created, already_exist
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
    let mut funded = 0u64;
    for batch_start in (0..num_accounts).step_by(BATCH_SIZE) {
        let batch_end = (batch_start + BATCH_SIZE as u64).min(num_accounts);

        let transfers: Vec<tb::Transfer> = (batch_start..batch_end)
            .map(|id| {
                tb::Transfer::new(tb::id())
                    .with_debit_account_id(bank_id)
                    .with_credit_account_id(id as u128)
                    .with_amount(initial_balance as u128)
                    .with_ledger(1)
                    .with_code(1)
            })
            .collect();

        let batch_count = transfers.len() as u64;
        if let Err(e) = client.create_transfers(transfers).await {
            warn!(
                "Failed to fund accounts {}-{}: {:?}",
                batch_start, batch_end, e
            );
        } else {
            funded += batch_count;
        }
    }

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

    let mut total_balance: u128 = 0;
    let mut accounts_found = 0u64;

    for batch_start in (0..num_accounts).step_by(BATCH_SIZE) {
        let batch_end = (batch_start + BATCH_SIZE as u64).min(num_accounts);
        let ids: Vec<u128> = (batch_start..batch_end).map(|id| id as u128).collect();

        let accounts = client
            .lookup_accounts(ids)
            .await
            .context("Failed to lookup accounts")?;

        accounts_found += accounts.len() as u64;

        for account in accounts {
            let balance = account
                .credits_posted()
                .saturating_sub(account.debits_posted());
            total_balance += balance;
        }
    }

    if accounts_found != num_accounts {
        warn!(
            "Expected {} accounts, found {}",
            num_accounts, accounts_found
        );
    }

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
