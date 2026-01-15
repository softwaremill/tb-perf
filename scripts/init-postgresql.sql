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

-- Atomic transfer function (no explicit locks)
-- Uses atomic UPDATE with balance check in WHERE clause
-- Updates accounts in ID order to prevent deadlocks
-- Safe at READ COMMITTED due to row-level locking during UPDATE
--
-- Returns 'success' on success.
-- Raises exception with SQLSTATE 'TB001' for insufficient_balance
-- Raises exception with SQLSTATE 'TB002' for account_not_found
CREATE OR REPLACE FUNCTION transfer_atomic(
    p_source_id BIGINT,
    p_dest_id BIGINT,
    p_amount BIGINT
) RETURNS TEXT AS $$
DECLARE
    v_updated INT;
BEGIN
    IF p_source_id < p_dest_id THEN
        -- Source has lower ID: debit first, then credit
        UPDATE accounts SET balance = balance - p_amount
        WHERE id = p_source_id AND balance >= p_amount;
        GET DIAGNOSTICS v_updated = ROW_COUNT;
        IF v_updated = 0 THEN
            PERFORM 1 FROM accounts WHERE id = p_source_id;
            IF NOT FOUND THEN
                RAISE EXCEPTION 'account_not_found' USING ERRCODE = 'TB002';
            ELSE
                RAISE EXCEPTION 'insufficient_balance' USING ERRCODE = 'TB001';
            END IF;
        END IF;

        UPDATE accounts SET balance = balance + p_amount WHERE id = p_dest_id;
        GET DIAGNOSTICS v_updated = ROW_COUNT;
        IF v_updated = 0 THEN
            RAISE EXCEPTION 'account_not_found' USING ERRCODE = 'TB002';
        END IF;
    ELSE
        -- Dest has lower ID: credit first, then debit
        -- If debit fails, exception rolls back the credit
        UPDATE accounts SET balance = balance + p_amount WHERE id = p_dest_id;
        GET DIAGNOSTICS v_updated = ROW_COUNT;
        IF v_updated = 0 THEN
            RAISE EXCEPTION 'account_not_found' USING ERRCODE = 'TB002';
        END IF;

        UPDATE accounts SET balance = balance - p_amount
        WHERE id = p_source_id AND balance >= p_amount;
        GET DIAGNOSTICS v_updated = ROW_COUNT;
        IF v_updated = 0 THEN
            PERFORM 1 FROM accounts WHERE id = p_source_id;
            IF NOT FOUND THEN
                RAISE EXCEPTION 'account_not_found' USING ERRCODE = 'TB002';
            ELSE
                RAISE EXCEPTION 'insufficient_balance' USING ERRCODE = 'TB001';
            END IF;
        END IF;
    END IF;

    -- Record the transfer
    INSERT INTO transfers (source_id, dest_id, amount) VALUES (p_source_id, p_dest_id, p_amount);

    RETURN 'success';
END;
$$ LANGUAGE plpgsql;

-- Batch transfer function for batched executor
-- Processes multiple transfers in a single call, each in its own subtransaction
-- Input: Three parallel arrays (source_ids, dest_ids, amounts)
-- Output: Array of result codes (0=success, 1=insufficient_balance, 2=account_not_found, 3=failed)
--
-- Using integer codes instead of text for efficiency (smaller return payload)
-- Uses PL/pgSQL BEGIN...EXCEPTION blocks which create implicit savepoints
--
-- Example:
--   SELECT batch_transfers(ARRAY[1,3], ARRAY[2,4], ARRAY[100,50]);
--   Returns: ARRAY[0, 1]  (first succeeded, second had insufficient balance)
CREATE OR REPLACE FUNCTION batch_transfers(
    p_source_ids BIGINT[],
    p_dest_ids BIGINT[],
    p_amounts BIGINT[]
) RETURNS SMALLINT[] AS $$
DECLARE
    v_results SMALLINT[] := '{}';
    v_result SMALLINT;
    v_source_id BIGINT;
    v_dest_id BIGINT;
    v_amount BIGINT;
    v_source_balance BIGINT;
    v_first_id BIGINT;
    v_second_id BIGINT;
    v_first_balance BIGINT;
    v_second_balance BIGINT;
    v_len INT;
    v_idx INT;
BEGIN
    v_len := array_length(p_source_ids, 1);
    IF v_len IS NULL THEN
        RETURN v_results;
    END IF;

    -- Validate all arrays have the same length
    IF array_length(p_dest_ids, 1) IS DISTINCT FROM v_len
       OR array_length(p_amounts, 1) IS DISTINCT FROM v_len THEN
        RAISE EXCEPTION 'Array length mismatch: source_ids=%, dest_ids=%, amounts=%',
            v_len,
            array_length(p_dest_ids, 1),
            array_length(p_amounts, 1);
    END IF;

    -- Process each transfer in the batch
    FOR v_idx IN 1..v_len
    LOOP
        v_source_id := p_source_ids[v_idx];
        v_dest_id := p_dest_ids[v_idx];
        v_amount := p_amounts[v_idx];

        -- Each transfer in its own BEGIN...EXCEPTION block (implicit savepoint)
        BEGIN
            -- Determine lock order (always lock lower ID first to prevent deadlocks)
            IF v_source_id < v_dest_id THEN
                v_first_id := v_source_id;
                v_second_id := v_dest_id;
            ELSE
                v_first_id := v_dest_id;
                v_second_id := v_source_id;
            END IF;

            -- Lock both accounts in consistent order and verify they exist
            SELECT balance INTO STRICT v_first_balance FROM accounts WHERE id = v_first_id FOR UPDATE;
            SELECT balance INTO STRICT v_second_balance FROM accounts WHERE id = v_second_id FOR UPDATE;

            -- Get source balance (may be first or second depending on order)
            IF v_source_id = v_first_id THEN
                v_source_balance := v_first_balance;
            ELSE
                v_source_balance := v_second_balance;
            END IF;

            IF v_source_balance < v_amount THEN
                RAISE EXCEPTION 'insufficient_balance' USING ERRCODE = 'P0001';
            END IF;

            -- Perform the transfer
            UPDATE accounts SET balance = balance - v_amount WHERE id = v_source_id;
            UPDATE accounts SET balance = balance + v_amount WHERE id = v_dest_id;

            -- Record the transfer
            INSERT INTO transfers (source_id, dest_id, amount) VALUES (v_source_id, v_dest_id, v_amount);

            v_result := 0;  -- success
            v_results := array_append(v_results, v_result);

        EXCEPTION
            WHEN NO_DATA_FOUND THEN
                v_result := 2;  -- account_not_found
                v_results := array_append(v_results, v_result);
            WHEN SQLSTATE 'P0001' THEN
                v_result := 1;  -- insufficient_balance
                v_results := array_append(v_results, v_result);
            WHEN OTHERS THEN
                v_result := 3;  -- failed
                v_results := array_append(v_results, v_result);
        END;
    END LOOP;

    RETURN v_results;
END;
$$ LANGUAGE plpgsql;

