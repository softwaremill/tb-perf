use rand::Rng;
use rand_distr::{Distribution, Zipf};

/// Generates account pairs for transfers using Zipfian distribution
#[derive(Clone)]
pub struct AccountSelector {
    num_accounts: u64,
    zipf: Zipf<f64>,
}

impl AccountSelector {
    pub fn new(num_accounts: u64, zipfian_exponent: f64) -> Self {
        // Zipf distribution: lower IDs are more likely to be selected
        // exponent 0 = uniform, exponent ~1.5 = high skew
        let zipf =
            Zipf::new(num_accounts as f64, zipfian_exponent).expect("Invalid Zipfian parameters");
        Self { num_accounts, zipf }
    }

    /// Select a random account using Zipfian distribution
    pub fn select_account<R: Rng>(&self, rng: &mut R) -> u64 {
        // Zipf returns values in [1, n], we want [0, n-1]
        let account = self.zipf.sample(rng) as u64 - 1;
        account.min(self.num_accounts - 1)
    }

    /// Select two different accounts for a transfer
    pub fn select_transfer_accounts<R: Rng>(&self, rng: &mut R) -> (u64, u64) {
        let source = self.select_account(rng);
        let mut dest = self.select_account(rng);

        // Ensure source and destination are different
        while dest == source {
            dest = self.select_account(rng);
        }

        (source, dest)
    }
}

/// Generates random transfer amounts within configured range
#[derive(Clone)]
pub struct TransferGenerator {
    min_amount: u64,
    max_amount: u64,
}

impl TransferGenerator {
    pub fn new(min_amount: u64, max_amount: u64) -> Self {
        Self {
            min_amount,
            max_amount,
        }
    }

    /// Generate a random transfer amount
    pub fn generate_amount<R: Rng>(&self, rng: &mut R) -> u64 {
        rng.random_range(self.min_amount..=self.max_amount)
    }
}

/// Result of a single transfer operation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferResult {
    /// Transfer completed successfully
    Success,
    /// Transfer rejected due to insufficient balance
    InsufficientBalance,
    /// Transfer failed because account doesn't exist
    AccountNotFound,
    /// Transfer failed due to database error (after retries)
    Failed,
}

/// SQL return value constants (must match init-postgresql.sql)
pub mod sql_results {
    pub const SUCCESS: &str = "success";
    pub const INSUFFICIENT_BALANCE: &str = "insufficient_balance";
    pub const ACCOUNT_NOT_FOUND: &str = "account_not_found";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_account_selector_uniform() {
        let selector = AccountSelector::new(1000, 0.0);
        let mut rng = rand::rng();

        // Just verify it doesn't panic and returns valid accounts
        for _ in 0..100 {
            let account = selector.select_account(&mut rng);
            assert!(account < 1000);
        }
    }

    #[test]
    fn test_account_selector_skewed() {
        let selector = AccountSelector::new(1000, 1.5);
        let mut rng = rand::rng();

        // With high skew, lower accounts should be selected more often
        let mut low_count = 0;
        for _ in 0..1000 {
            let account = selector.select_account(&mut rng);
            assert!(account < 1000);
            if account < 100 {
                low_count += 1;
            }
        }

        // With zipf exponent 1.5, we expect significant skew toward low accounts
        assert!(low_count > 500, "Expected skew toward low accounts");
    }

    #[test]
    fn test_transfer_accounts_different() {
        let selector = AccountSelector::new(1000, 1.0);
        let mut rng = rand::rng();

        for _ in 0..100 {
            let (source, dest) = selector.select_transfer_accounts(&mut rng);
            assert_ne!(source, dest, "Source and destination must be different");
        }
    }

    #[test]
    fn test_transfer_generator() {
        let generator = TransferGenerator::new(1, 1000);
        let mut rng = rand::rng();

        for _ in 0..100 {
            let amount = generator.generate_amount(&mut rng);
            assert!(amount >= 1 && amount <= 1000);
        }
    }
}
