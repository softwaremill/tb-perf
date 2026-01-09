-- TigerBeetle Performance Benchmark - PostgreSQL Schema
-- Double-entry bookkeeping with pessimistic locking
--
-- Data type choice:
--   TigerBeetle uses u128 (16 bytes fixed) for id, amount, and balance fields.
--   PostgreSQL has no native 128-bit integer. Options:
--     - NUMERIC: variable-length, slower arithmetic (unfair comparison)
--     - BIGINT: 8 bytes fixed, fast native ops, max ~9.2 quintillion
--   We use BIGINT for fair performance comparison. This is sufficient for
--   realistic financial workloads and matches TigerBeetle's fixed-size semantics.

-- Accounts table
CREATE TABLE IF NOT EXISTS accounts (
    id BIGINT PRIMARY KEY,
    balance BIGINT NOT NULL DEFAULT 0,
    CONSTRAINT balance_non_negative CHECK (balance >= 0)
);

-- Transfers audit log
CREATE TABLE IF NOT EXISTS transfers (
    id BIGSERIAL PRIMARY KEY,
    source_id BIGINT NOT NULL REFERENCES accounts(id),
    dest_id BIGINT NOT NULL REFERENCES accounts(id),
    amount BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT amount_positive CHECK (amount > 0),
    CONSTRAINT different_accounts CHECK (source_id != dest_id)
);

-- Index for transfer queries
CREATE INDEX IF NOT EXISTS idx_transfers_source ON transfers(source_id);
CREATE INDEX IF NOT EXISTS idx_transfers_dest ON transfers(dest_id);
CREATE INDEX IF NOT EXISTS idx_transfers_created_at ON transfers(created_at);

-- Transfer stored procedure with pessimistic locking
-- Returns: 'success', 'insufficient_balance', or 'account_not_found'
-- Accounts are locked in ID order to prevent deadlocks
CREATE OR REPLACE FUNCTION transfer(
    p_source_id BIGINT,
    p_dest_id BIGINT,
    p_amount BIGINT
) RETURNS TEXT AS $$
DECLARE
    v_source_balance BIGINT;
    v_dest_balance BIGINT;
    v_first_id BIGINT;
    v_second_id BIGINT;
    v_first_balance BIGINT;
    v_second_balance BIGINT;
BEGIN
    -- Determine lock order (always lock lower ID first to prevent deadlocks)
    IF p_source_id < p_dest_id THEN
        v_first_id := p_source_id;
        v_second_id := p_dest_id;
    ELSE
        v_first_id := p_dest_id;
        v_second_id := p_source_id;
    END IF;

    -- Lock both accounts in consistent order and verify they exist
    SELECT balance INTO v_first_balance FROM accounts WHERE id = v_first_id FOR UPDATE;
    IF NOT FOUND THEN
        RETURN 'account_not_found';
    END IF;

    SELECT balance INTO v_second_balance FROM accounts WHERE id = v_second_id FOR UPDATE;
    IF NOT FOUND THEN
        RETURN 'account_not_found';
    END IF;

    -- Get source balance (may be first or second depending on order)
    IF p_source_id = v_first_id THEN
        v_source_balance := v_first_balance;
    ELSE
        v_source_balance := v_second_balance;
    END IF;

    IF v_source_balance < p_amount THEN
        RETURN 'insufficient_balance';
    END IF;

    -- Perform the transfer
    UPDATE accounts SET balance = balance - p_amount WHERE id = p_source_id;
    UPDATE accounts SET balance = balance + p_amount WHERE id = p_dest_id;

    -- Record the transfer
    INSERT INTO transfers (source_id, dest_id, amount) VALUES (p_source_id, p_dest_id, p_amount);

    RETURN 'success';
END;
$$ LANGUAGE plpgsql;

