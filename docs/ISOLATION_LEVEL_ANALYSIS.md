# PostgreSQL Isolation Levels and Correctness Analysis

## Overview

This document analyzes how PostgreSQL isolation levels affect correctness for our double-entry bookkeeping workload. The analysis focuses on whether our implementation maintains the invariant that **total balance across all accounts remains constant** (money is neither created nor destroyed).

## The Double-Entry Transfer Transaction

Our transfer function performs the following operations within a single transaction:

```sql
-- 1. Lock both accounts in ID order (prevents deadlocks)
SELECT balance INTO v_first_balance FROM accounts WHERE id = v_first_id FOR UPDATE;
SELECT balance INTO v_second_balance FROM accounts WHERE id = v_second_id FOR UPDATE;

-- 2. Check sufficient balance
IF v_source_balance < p_amount THEN RETURN 'insufficient_balance';

-- 3. Perform transfer
UPDATE accounts SET balance = balance - p_amount WHERE id = p_source_id;
UPDATE accounts SET balance = balance + p_amount WHERE id = p_dest_id;

-- 4. Record audit log
INSERT INTO transfers (source_id, dest_id, amount) VALUES (...);
```

## Concurrency Phenomena Analysis

### Dirty Reads

**Definition**: Reading uncommitted data from another transaction.

**Can it occur?** No. PostgreSQL does not permit dirty reads at any isolation level.

**Impact**: Not applicable.

### Non-Repeatable Reads

**Definition**: Reading the same row twice within a transaction and getting different values because another transaction modified and committed it.

**Can it occur in our workload?** No, due to `SELECT ... FOR UPDATE`.

The `FOR UPDATE` clause acquires a row-level exclusive lock. Once we lock an account:
- No other transaction can modify that row until we commit/rollback
- Our subsequent reads and writes see consistent data
- The lock is held until transaction end

**Impact**: Not a concern because pessimistic locking prevents this.

### Lost Updates

**Definition**: Two transactions read the same value, compute new values based on it, and both write back—one update overwrites the other.

Classic example without locking:
```
T1: reads balance = 100
T2: reads balance = 100
T1: writes balance = 100 - 50 = 50
T2: writes balance = 100 - 30 = 70  <- T1's update is lost!
```

**Can it occur in our workload?** No, due to `SELECT ... FOR UPDATE`.

The lock acquired by T1 blocks T2's `SELECT ... FOR UPDATE` until T1 commits. T2 then reads the committed value (50) and computes correctly (50 - 30 = 20).

**Impact**: Not a concern because pessimistic locking serializes access to each account.

### Write Skew

**Definition**: Two transactions read overlapping data, make disjoint updates based on what they read, and both commit—violating a constraint that spans the data.

Classic example: A constraint requires at least one doctor on-call. Two doctors each see the other is on-call and both go off-call simultaneously.

**Can it occur in our workload?** No.

Our workload does not have constraints spanning multiple accounts. Each transfer:
- Locks both participating accounts exclusively
- Only checks constraint on source account (balance >= amount)
- Updates only those two accounts

The `FOR UPDATE` lock on the source account ensures:
- No other transaction can read-then-update that account concurrently
- The balance check and debit are atomic with respect to other transactions

**Impact**: Not a concern for this workload pattern.

### Phantom Reads

**Definition**: A transaction re-executes a range query and gets different rows because another transaction inserted/deleted rows.

**Can it occur in our workload?** Not applicable.

Our workload does not use range queries. Each transfer accesses exactly two accounts by primary key.

**Impact**: Not a concern because we don't use range queries.

## Analysis by Isolation Level

### READ COMMITTED

**PostgreSQL behavior**: Each statement sees data committed before that statement began. Different statements within the same transaction may see different committed states.

**Is it safe for our workload?** Yes, with our locking strategy.

**Why it's safe**:
1. `SELECT ... FOR UPDATE` acquires exclusive locks on both accounts
2. Once locked, no other transaction can modify those rows
3. The balance check and updates execute atomically (no interleaving)
4. Locks held until commit ensures consistent state

**Potential issue (if we didn't use FOR UPDATE)**:
```
T1: SELECT balance FROM accounts WHERE id=1;  -- sees 100
T2: UPDATE accounts SET balance = 50 WHERE id=1; COMMIT;
T1: UPDATE accounts SET balance = balance - 60 WHERE id=1;  -- 50-60 = -10 (negative!)
```

**Our implementation avoids this** because `FOR UPDATE` blocks T2 until T1 commits.

**Serialization failures**: Rare. Only occur if lock wait timeout exceeded.

### REPEATABLE READ (Snapshot Isolation)

**PostgreSQL behavior**: Transaction sees a consistent snapshot from transaction start. Modifications by other transactions (even if committed) are not visible.

**Is it safe for our workload?** Yes.

**Why it's safe**: Same reasoning as READ COMMITTED—our `FOR UPDATE` locks provide the necessary isolation regardless of the isolation level.

**Key difference from READ COMMITTED**: If a concurrent transaction modified a row we're trying to update, PostgreSQL will abort our transaction with a serialization failure rather than blocking.

**Serialization failures**: Can occur when:
- T1 starts, takes snapshot
- T2 modifies account A and commits
- T1 tries to `SELECT ... FOR UPDATE` on account A
- PostgreSQL detects the conflict and aborts T1

This is slightly more likely than READ COMMITTED because REPEATABLE READ detects "first-updater-wins" conflicts.

### SERIALIZABLE

**PostgreSQL behavior**: Full serializability via Serializable Snapshot Isolation (SSI). Detects all anomalies that could result in non-serializable execution.

**Is it safe for our workload?** Yes.

**Why it's safe**: SERIALIZABLE provides the strongest guarantees. With our locking strategy, it's strictly safer than needed.

**Serialization failures**: Most likely to occur. PostgreSQL's SSI tracks read/write dependencies and aborts transactions that would create cycles. Even read-only transactions can be aborted.

For our workload, the explicit `FOR UPDATE` locks mean we're already serializing access, so SSI's additional tracking is redundant but not harmful.

## Performance vs. Correctness Summary

| Isolation Level | Correctness | Serialization Failure Rate | Notes |
|-----------------|-------------|---------------------------|-------|
| READ COMMITTED | Safe (with FOR UPDATE) | Lowest | Best performance, safe due to explicit locking |
| REPEATABLE READ | Safe (with FOR UPDATE) | Medium | Detects first-updater-wins conflicts |
| SERIALIZABLE | Safe | Highest | SSI overhead unnecessary given our locking |

## The Role of `SELECT ... FOR UPDATE`

Our implementation's correctness does **not** depend on the isolation level. It depends entirely on the pessimistic locking strategy:

1. **Exclusive access**: `FOR UPDATE` ensures only one transaction can modify an account at a time
2. **Deadlock prevention**: Locking accounts in ID order prevents circular wait
3. **Atomicity**: Lock held until commit ensures balance check and updates are atomic

The isolation level affects:
- **Visibility rules** for reads (not relevant when we lock rows)
- **Conflict detection** strategy (blocking vs. aborting)
- **Serialization failure frequency** (lower at READ COMMITTED)

## Recommendation

**Use READ COMMITTED** for this workload.

Rationale:
- **Correctness is guaranteed** by our `FOR UPDATE` locking, not by isolation level
- **Lower serialization failure rate** means fewer retries, better throughput
- **Less overhead** than REPEATABLE READ or SERIALIZABLE
- **TigerBeetle comparison**: TigerBeetle also uses optimistic concurrency with conflict detection, most comparable to READ COMMITTED behavior

Testing at higher isolation levels is valuable for:
- Understanding performance impact of stricter isolation
- Validating that serialization failures are handled correctly
- Comparing behavior under different conflict detection strategies

## Correctness Verification

After each test run, we verify correctness by checking:

```sql
SELECT SUM(balance) FROM accounts;
```

This must equal `num_accounts × initial_balance`. Any discrepancy indicates:
- Bug in transfer logic
- Incorrect isolation/locking implementation
- Data corruption

Our test framework automatically performs this check and fails the test if the invariant is violated.

## References

- [PostgreSQL Transaction Isolation](https://www.postgresql.org/docs/current/transaction-iso.html)
- [PostgreSQL Explicit Locking](https://www.postgresql.org/docs/current/explicit-locking.html)
- [A Critique of ANSI SQL Isolation Levels](https://www.microsoft.com/en-us/research/wp-content/uploads/2016/02/tr-95-51.pdf)
- [Serializable Snapshot Isolation in PostgreSQL](https://drkp.net/papers/ssi-vldb12.pdf)
