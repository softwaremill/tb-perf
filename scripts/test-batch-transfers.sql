-- Test script for batch_transfers function
-- Run with: psql -U postgres -d tbperf -f scripts/test-batch-transfers.sql

\echo '=== Setting up test data ==='

-- Clean up any existing test data
TRUNCATE transfers CASCADE;
TRUNCATE accounts CASCADE;

-- Create test accounts with known balances
INSERT INTO accounts (id, balance) VALUES
    (1, 1000),
    (2, 500),
    (3, 100),
    (4, 0);

\echo 'Initial balances:'
SELECT id, balance FROM accounts ORDER BY id;

\echo ''
\echo '=== Test 1: Single successful transfer ==='
SELECT batch_transfers(
    ARRAY[1]::BIGINT[],
    ARRAY[2]::BIGINT[],
    ARRAY[100]::BIGINT[]
) AS result;
-- Expected: {0} (success)

\echo 'Balances after test 1:'
SELECT id, balance FROM accounts ORDER BY id;
-- Expected: 1=900, 2=600, 3=100, 4=0

\echo ''
\echo '=== Test 2: Single insufficient balance ==='
SELECT batch_transfers(
    ARRAY[4]::BIGINT[],
    ARRAY[1]::BIGINT[],
    ARRAY[100]::BIGINT[]
) AS result;
-- Expected: {1} (insufficient_balance)

\echo 'Balances after test 2 (should be unchanged):'
SELECT id, balance FROM accounts ORDER BY id;
-- Expected: 1=900, 2=600, 3=100, 4=0

\echo ''
\echo '=== Test 3: Account not found ==='
SELECT batch_transfers(
    ARRAY[999]::BIGINT[],
    ARRAY[1]::BIGINT[],
    ARRAY[100]::BIGINT[]
) AS result;
-- Expected: {2} (account_not_found)

\echo 'Balances after test 3 (should be unchanged):'
SELECT id, balance FROM accounts ORDER BY id;

\echo ''
\echo '=== Test 4: Multiple transfers in one batch (all succeed) ==='
SELECT batch_transfers(
    ARRAY[1, 2]::BIGINT[],
    ARRAY[3, 4]::BIGINT[],
    ARRAY[50, 50]::BIGINT[]
) AS result;
-- Expected: {0,0} (both succeed)

\echo 'Balances after test 4:'
SELECT id, balance FROM accounts ORDER BY id;
-- Expected: 1=850, 2=550, 3=150, 4=50

\echo ''
\echo '=== Test 5: Mixed results in batch ==='
-- First transfer: 1->2 for 100 (should succeed, 1 has 850)
-- Second transfer: 4->3 for 100 (should fail, 4 only has 50)
-- Third transfer: 3->1 for 50 (should succeed, 3 has 150)
SELECT batch_transfers(
    ARRAY[1, 4, 3]::BIGINT[],
    ARRAY[2, 3, 1]::BIGINT[],
    ARRAY[100, 100, 50]::BIGINT[]
) AS result;
-- Expected: {0,1,0} (success, insufficient_balance, success)

\echo 'Balances after test 5:'
SELECT id, balance FROM accounts ORDER BY id;
-- Expected: 1=800, 2=650, 3=100, 4=50

\echo ''
\echo '=== Test 6: Empty batch ==='
SELECT batch_transfers(
    ARRAY[]::BIGINT[],
    ARRAY[]::BIGINT[],
    ARRAY[]::BIGINT[]
) AS result;
-- Expected: {} (empty array)

\echo ''
\echo '=== Test 7: Transfer to self (should fail due to constraint) ==='
SELECT batch_transfers(
    ARRAY[1]::BIGINT[],
    ARRAY[1]::BIGINT[],
    ARRAY[100]::BIGINT[]
) AS result;
-- Expected: {3} (failed - different_accounts constraint violation)

\echo 'Balances after test 7 (should be unchanged):'
SELECT id, balance FROM accounts ORDER BY id;

\echo ''
\echo '=== Test 8: Verify transfer audit log ==='
SELECT COUNT(*) AS transfer_count FROM transfers;
-- Expected: 5 transfers (test1: 1, test4: 2, test5: 2)

\echo ''
\echo '=== Test 9: Verify total balance unchanged ==='
SELECT SUM(balance) AS total_balance FROM accounts;
-- Expected: 1600 (same as initial: 1000+500+100+0)

\echo ''
\echo '=== Test 10: Array length mismatch (should raise exception) ==='
DO $$
BEGIN
    PERFORM batch_transfers(
        ARRAY[1, 2]::BIGINT[],
        ARRAY[3]::BIGINT[],
        ARRAY[100, 200]::BIGINT[]
    );
    RAISE NOTICE 'ERROR: Should have raised exception!';
EXCEPTION
    WHEN OTHERS THEN
        RAISE NOTICE 'OK: Caught expected exception: %', SQLERRM;
END $$;

\echo ''
\echo '=== All tests completed ==='
