# Remaining PostgreSQL Executor Mode Tests

Covers the 4 tests requested to round out the PostgreSQL comparison beyond
the `standard` (`FOR UPDATE`) executor already tested in
`postgres-standard-hotspot.md`:

- PostgreSQL **atomic** executor, fixed_rate (`config.cloud-postgresql-atomic-fixedrate.toml`)
- PostgreSQL **batched** executor, fixedrate slot (`config.cloud-postgresql-batched-fixedrate.toml`)
- PostgreSQL **atomic** executor, hotspot (`config.cloud-postgresql-atomic-hotspot.toml`)
- PostgreSQL **batched** executor, hotspot (`config.cloud-postgresql-batched-hotspot.toml`)

Same cluster/workload shape as prior tests otherwise: 100k accounts, 5 client
nodes / 3 DB nodes, 3x (2min warmup + 5min measurement) runs.
`zipfian_exponent = 1.0` ("fixedrate" configs) vs `2.0` ("hotspot" configs).

## Important methodology note: batched executor mode could not run under fixed_rate

The `batched` executor uses **one shared database connection per client
instance** (it collects pending transfers and submits them as a single SQL
array-batch call - designed to mirror TigerBeetle's single-writer batching
model). Running it under `fixed_rate` mode - which continuously dispatches
new requests up to `max_concurrency` regardless of whether the backend can
keep up - was tried first and failed consistently: the single connection
couldn't drain the in-flight backlog before the client's graceful-shutdown
wait (drain all in-flight requests, no timeout) exceeded the coordinator's
480s SSH command timeout. Confirmed live via Prometheus: throughput was
~2 completed transfers/sec per node against a configured target of 1000/s
per node.

This is a workload-mode mismatch, not a bug: batched mode's value proposition
is one connection saturated with large batches, which `fixed_rate`'s pacing
model defeats. The repo's own local config
(`config.local-postgresql-batched.toml`) already tests batched mode with
`max_throughput`, not `fixed_rate`, for this exact reason. Both batched
configs here were switched to `max_throughput` (`concurrency = 10`,
mirroring the local config) before running - see the updated `.toml` files
for the full rationale in comments.

## Results overview

### zipfian_exponent = 1.0 ("fixedrate" slot)

| Metric | Standard (`FOR UPDATE`)* | Atomic | Batched (max_throughput, not fixed_rate) |
|---|---|---|---|
| Test mode | fixed_rate | fixed_rate | max_throughput (concurrency=10) |
| Mean throughput | 2,890 TPS | **3,068 TPS** | 30.3 TPS |
| p50 latency | 324.7 ms | 303.7 ms | 1,293 ms |
| p95 latency | 581 ms | 556 ms | 5,000 ms (capped) |
| p99 latency | 960 ms | 927 ms | 5,000 ms (capped) |
| p999 latency | 1,492 ms | 1,473 ms | 5,000 ms (capped) |
| Error rate | 0% | 0% | **4.48%** |
| Balance verified | 3/3 | 3/3 | 3/3 |

\* Standard executor numbers are from `postgres-standard-hotspot.md` (same
config shape, run previously), included here for reference.

Atomic is a modest, consistent improvement over standard (~6% more
throughput, ~7% lower p50) - avoiding explicit `SELECT ... FOR UPDATE` locks
in favor of a single atomic `UPDATE ... WHERE balance >= amount` reduces lock
overhead somewhat, but both executors still pay a per-transfer commit/
replication round trip, so the gain is incremental rather than
architectural. Batched mode, forced into a single-connection role under
concurrency=10, is over 100x slower than the other two and has a real
(if below-threshold-in-this-slot) error rate - it is simply not built to be
driven this way; see the raw JSON below for what "failed" means here
(batch-level failures where one bad item fails the whole batch).

### zipfian_exponent = 2.0 ("hotspot" slot)

| Metric | Standard* | Atomic | Batched (max_throughput) |
|---|---|---|---|
| Test mode | fixed_rate | fixed_rate | max_throughput (concurrency=10) |
| Mean throughput | 753 TPS | **977 TPS** | 7.5 TPS |
| p50 latency | 1,306 ms | 1,001 ms | 386 ms (huge run-to-run spread: 40ms-1,070ms) |
| p95 latency | 1,890 ms | 1,472 ms | 5,000 ms (capped) |
| p99 latency | 2,123 ms | 1,821 ms | 5,000 ms (capped) |
| p999 latency | 2,916 ms | 3,138 ms | 5,000 ms (capped) |
| Error rate | 0% | 0% | **16.59%** (exceeds coordinator's 5% threshold) |
| Balance verified | 3/3 | 3/3 | 3/3 |

\* Standard executor numbers are from `postgres-standard-hotspot.md`.

Under heavy hotspot contention, atomic actually holds up somewhat better
than standard (~30% more throughput, ~23% lower p50) - avoiding the explicit
row lock removes one source of contention overhead, though both are still
far behind TigerBeetle's hotspot numbers (4,461 TPS, 32ms p50 - see
`postgres-standard-hotspot.md`). Batched mode degrades further under
hotspot: throughput drops to single digits and the error rate roughly
quadruples versus its fixedrate-slot run, breaching the coordinator's own
5% error-rate threshold (flagged as an `ERROR` in the run, not just a
`WARNING`). Correctness held in every case (balance verified 3/3 for all six
runs across both slots) - these are real performance/reliability
characteristics, not data corruption.

## Raw results

### Atomic executor, zipfian_exponent = 1.0

```json
{
  "config_summary": {
    "database_type": "PostgreSQL",
    "test_mode": "fixed_rate",
    "num_accounts": 100000,
    "initial_balance": 1000000,
    "warmup_duration_secs": 120,
    "test_duration_secs": 300,
    "num_runs": 3
  },
  "runs": [
    {
      "run_id": 1,
      "duration_secs": 426.102431584,
      "throughput_tps": 3056.19,
      "latency_p50_us": 304053,
      "latency_p95_us": 552877,
      "latency_p99_us": 925463,
      "latency_p999_us": 1470859,
      "completed_transfers": 912736,
      "rejected_transfers": 4121,
      "failed_transfers": 0,
      "balance_verified": true
    },
    {
      "run_id": 2,
      "duration_secs": 423.586495125,
      "throughput_tps": 3075.8733333333334,
      "latency_p50_us": 306434,
      "latency_p95_us": 563108,
      "latency_p99_us": 933899,
      "latency_p999_us": 1476319,
      "completed_transfers": 918441,
      "rejected_transfers": 4321,
      "failed_transfers": 0,
      "balance_verified": true
    },
    {
      "run_id": 3,
      "duration_secs": 423.698539917,
      "throughput_tps": 3072.48,
      "latency_p50_us": 300682,
      "latency_p95_us": 553249,
      "latency_p99_us": 923060,
      "latency_p999_us": 1470976,
      "completed_transfers": 917101,
      "rejected_transfers": 4643,
      "failed_transfers": 0,
      "balance_verified": true
    }
  ],
  "aggregate": {
    "throughput": {
      "mean": 3068.181111111111,
      "stddev": 8.5914196357776,
      "cv": 0.0028001670451149813,
      "min": 3056.19,
      "max": 3075.8733333333334
    },
    "latency_p50": {
      "mean": 303723.0,
      "stddev": 2359.8094555846383,
      "cv": 0.0077696106504434575,
      "min": 300682.0,
      "max": 306434.0
    },
    "latency_p95": {
      "mean": 556411.3333333334,
      "stddev": 4737.693132973285,
      "cv": 0.008514731546877104,
      "min": 552877.0,
      "max": 563108.0
    },
    "latency_p99": {
      "mean": 927474.0,
      "stddev": 4647.8719861889485,
      "cv": 0.005011323213576821,
      "min": 923060.0,
      "max": 933899.0
    },
    "latency_p999": {
      "mean": 1472718.0,
      "stddev": 2546.7394841247506,
      "cv": 0.0017292784389983354,
      "min": 1470859.0,
      "max": 1476319.0
    },
    "total_completed": 2748278,
    "total_rejected": 13085,
    "total_failed": 0,
    "error_rate": 0.0
  },
  "warnings": [],
  "errors": []
}
```

### Batched executor (max_throughput, concurrency=10), zipfian_exponent = 1.0

```json
{
  "config_summary": {
    "database_type": "PostgreSQL",
    "test_mode": "max_throughput",
    "num_accounts": 100000,
    "initial_balance": 1000000,
    "warmup_duration_secs": 120,
    "test_duration_secs": 300,
    "num_runs": 3
  },
  "runs": [
    {
      "run_id": 1,
      "duration_secs": 424.489772333,
      "throughput_tps": 30.176666666666666,
      "latency_p50_us": 1275661,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 9053,
      "rejected_transfers": 0,
      "failed_transfers": 420,
      "balance_verified": true
    },
    {
      "run_id": 2,
      "duration_secs": 425.644886084,
      "throughput_tps": 28.5,
      "latency_p50_us": 1341639,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 8550,
      "rejected_transfers": 0,
      "failed_transfers": 435,
      "balance_verified": true
    },
    {
      "run_id": 3,
      "duration_secs": 422.922437208,
      "throughput_tps": 32.1,
      "latency_p50_us": 1260628,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 9630,
      "rejected_transfers": 0,
      "failed_transfers": 423,
      "balance_verified": true
    }
  ],
  "aggregate": {
    "throughput": {
      "mean": 30.25888888888889,
      "stddev": 1.4708433794641707,
      "cv": 0.048608638105157476,
      "min": 28.5,
      "max": 32.1
    },
    "latency_p50": {
      "mean": 1292642.6666666667,
      "stddev": 35185.01842482899,
      "cv": 0.02721944689908811,
      "min": 1260628.0,
      "max": 1341639.0
    },
    "latency_p95": {
      "mean": 5000000.0,
      "stddev": 0.0,
      "cv": 0.0,
      "min": 5000000.0,
      "max": 5000000.0
    },
    "latency_p99": {
      "mean": 5000000.0,
      "stddev": 0.0,
      "cv": 0.0,
      "min": 5000000.0,
      "max": 5000000.0
    },
    "latency_p999": {
      "mean": 5000000.0,
      "stddev": 0.0,
      "cv": 0.0,
      "min": 5000000.0,
      "max": 5000000.0
    },
    "total_completed": 27233,
    "total_rejected": 0,
    "total_failed": 1278,
    "error_rate": 0.044824804461435934
  },
  "warnings": [],
  "errors": []
}
```

### Atomic executor, zipfian_exponent = 2.0

```json
{
  "config_summary": {
    "database_type": "PostgreSQL",
    "test_mode": "fixed_rate",
    "num_accounts": 100000,
    "initial_balance": 1000000,
    "warmup_duration_secs": 120,
    "test_duration_secs": 300,
    "num_runs": 3
  },
  "runs": [
    {
      "run_id": 1,
      "duration_secs": 423.909499708,
      "throughput_tps": 974.1466666666666,
      "latency_p50_us": 998815,
      "latency_p95_us": 1471680,
      "latency_p99_us": 1798279,
      "latency_p999_us": 2114429,
      "completed_transfers": 205591,
      "rejected_transfers": 86653,
      "failed_transfers": 0,
      "balance_verified": true
    },
    {
      "run_id": 2,
      "duration_secs": 377.569677042,
      "throughput_tps": 992.6,
      "latency_p50_us": 940715,
      "latency_p95_us": 1462410,
      "latency_p99_us": 1816226,
      "latency_p999_us": 5000000,
      "completed_transfers": 209841,
      "rejected_transfers": 87939,
      "failed_transfers": 0,
      "balance_verified": true
    },
    {
      "run_id": 3,
      "duration_secs": 424.745046791,
      "throughput_tps": 963.33,
      "latency_p50_us": 1064922,
      "latency_p95_us": 1481104,
      "latency_p99_us": 1847633,
      "latency_p999_us": 2299481,
      "completed_transfers": 202842,
      "rejected_transfers": 86157,
      "failed_transfers": 0,
      "balance_verified": true
    }
  ],
  "aggregate": {
    "throughput": {
      "mean": 976.6922222222223,
      "stddev": 12.084235317548085,
      "cv": 0.01237261344218897,
      "min": 963.33,
      "max": 992.6
    },
    "latency_p50": {
      "mean": 1001484.0,
      "stddev": 50742.40424602156,
      "cv": 0.05066721410029672,
      "min": 940715.0,
      "max": 1064922.0
    },
    "latency_p95": {
      "mean": 1471731.3333333333,
      "stddev": 7631.879861621396,
      "cv": 0.005185647467555036,
      "min": 1462410.0,
      "max": 1481104.0
    },
    "latency_p99": {
      "mean": 1820712.6666666667,
      "stddev": 20396.92725769143,
      "cv": 0.011202716184225716,
      "min": 1798279.0,
      "max": 1847633.0
    },
    "latency_p999": {
      "mean": 3137970.0,
      "stddev": 1318819.6367335452,
      "cv": 0.4202779621008312,
      "min": 2114429.0,
      "max": 5000000.0
    },
    "total_completed": 618274,
    "total_rejected": 260749,
    "total_failed": 0,
    "error_rate": 0.0
  },
  "warnings": [],
  "errors": []
}
```

### Batched executor (max_throughput, concurrency=10), zipfian_exponent = 2.0

```json
{
  "config_summary": {
    "database_type": "PostgreSQL",
    "test_mode": "max_throughput",
    "num_accounts": 100000,
    "initial_balance": 1000000,
    "warmup_duration_secs": 120,
    "test_duration_secs": 300,
    "num_runs": 3
  },
  "runs": [
    {
      "run_id": 1,
      "duration_secs": 430.612378792,
      "throughput_tps": 6.526666666666666,
      "latency_p50_us": 1070199,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 1958,
      "rejected_transfers": 0,
      "failed_transfers": 407,
      "balance_verified": true
    },
    {
      "run_id": 2,
      "duration_secs": 431.868806375,
      "throughput_tps": 7.283333333333333,
      "latency_p50_us": 47901,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 2185,
      "rejected_transfers": 0,
      "failed_transfers": 461,
      "balance_verified": true
    },
    {
      "run_id": 3,
      "duration_secs": 437.025414833,
      "throughput_tps": 8.73,
      "latency_p50_us": 40159,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 2619,
      "rejected_transfers": 0,
      "failed_transfers": 477,
      "balance_verified": true
    }
  ],
  "aggregate": {
    "throughput": {
      "mean": 7.513333333333333,
      "stddev": 0.9140913318498122,
      "cv": 0.12166255525951361,
      "min": 6.526666666666666,
      "max": 8.73
    },
    "latency_p50": {
      "mean": 386086.3333333333,
      "stddev": 483751.0311178216,
      "cv": 1.2529607741908027,
      "min": 40159.0,
      "max": 1070199.0
    },
    "latency_p95": {
      "mean": 5000000.0,
      "stddev": 0.0,
      "cv": 0.0,
      "min": 5000000.0,
      "max": 5000000.0
    },
    "latency_p99": {
      "mean": 5000000.0,
      "stddev": 0.0,
      "cv": 0.0,
      "min": 5000000.0,
      "max": 5000000.0
    },
    "latency_p999": {
      "mean": 5000000.0,
      "stddev": 0.0,
      "cv": 0.0,
      "min": 5000000.0,
      "max": 5000000.0
    },
    "total_completed": 6762,
    "total_rejected": 0,
    "total_failed": 1345,
    "error_rate": 0.16590600715431109
  },
  "warnings": [
    "High throughput variance: CV = 12.17% (threshold: 10%)"
  ],
  "errors": [
    "High error rate: 16.59% (threshold: 5%)"
  ]
}
```
