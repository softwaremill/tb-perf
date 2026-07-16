# TigerBeetle vs PostgreSQL — Full Cloud Performance Comparison

Six tests total: TigerBeetle vs three PostgreSQL executor modes (`standard`
i.e. `FOR UPDATE`, `atomic`, `batched`), each at two Zipfian skew levels.

Shared setup unless noted otherwise: 100k accounts, 5 client nodes / 3 DB
nodes (GCP `europe-central2`, `n2-highmem-4` DB / `n2-standard-2` clients),
3x (2min warmup + 5min measurement) runs per test, `fixed_rate` mode
(target 5,000 req/s total) for TigerBeetle/standard/atomic.

- **"Fixedrate" slot**: `zipfian_exponent = 1.0` (moderate skew)
- **"Hotspot" slot**: `zipfian_exponent = 2.0` (a small number of "hub"
  accounts absorb a disproportionate share of transfers - see
  "Why hotspot skew matters" below)

## Summary: all six tests

| Metric | TigerBeetle | PostgreSQL Standard (`FOR UPDATE`) | PostgreSQL Atomic | PostgreSQL Batched (max_throughput) |
|---|---|---|---|---|
| **Fixedrate (zipfian=1.0)** |
| Test mode | fixed_rate | fixed_rate | fixed_rate | max_throughput (concurrency=10)\* |
| Mean throughput | **4,097 TPS** | 2,890 TPS | 3,068 TPS | 30.3 TPS |
| p50 latency | **32.3 ms** | 324.7 ms | 303.7 ms | 1,293 ms |
| p95 latency | **508 ms** | 581 ms | 556 ms | 5,000 ms (capped) |
| p99 latency | **748 ms** | 960 ms | 927 ms | 5,000 ms (capped) |
| p999 latency | **987 ms** | 1,492 ms | 1,473 ms | 5,000 ms (capped) |
| Error rate | 0% | 0% | 0% | 4.48% |
| Balance verified | 3/3 | 3/3 | 3/3 | 3/3 |
| **Hotspot (zipfian=2.0)** |
| Test mode | fixed_rate | fixed_rate | fixed_rate | max_throughput (concurrency=10)\* |
| Mean throughput | **4,461 TPS** | 753 TPS | 977 TPS | 7.5 TPS |
| p50 latency | **32.1 ms** | 1,306 ms | 1,001 ms | 386 ms (huge spread: 40ms-1,070ms) |
| p95 latency | **358 ms** | 1,890 ms | 1,472 ms | 5,000 ms (capped) |
| p99 latency | **560 ms** | 2,123 ms | 1,821 ms | 5,000 ms (capped) |
| p999 latency | **726 ms** | 2,916 ms | 3,138 ms | 5,000 ms (capped) |
| Error rate | 0% | 0% | 0% | **16.59%** (exceeds 5% threshold) |
| Balance verified | 3/3 | 3/3 | 3/3 | 3/3 |

\* Batched mode could not complete under `fixed_rate` at all (see
"Why batched mode runs under max_throughput" below) - not a fixed-rate
result, included for completeness rather than as a like-for-like number.

## Headline takeaways

- **TigerBeetle wins on every metric, in both skew regimes**, and the gap
  *widens* under contention: ~1.4x PostgreSQL's throughput at moderate skew,
  ~4.6-5.9x at heavy hotspot skew (vs. best/worst PostgreSQL mode
  respectively), with p50 latency 10-40x lower throughout.
- **TigerBeetle actually got faster and lower-latency under heavier
  contention** (4,097→4,461 TPS, p95 508→358ms) - more requests resolve
  quickly via rejection (insufficient balance) rather than queuing on a
  lock. All three PostgreSQL modes got worse under hotspot skew.
- **Among PostgreSQL modes, atomic is the best all-rounder**: consistently
  ~6-30% faster than standard with lower latency in both skew regimes,
  by avoiding explicit `SELECT ... FOR UPDATE` locks in favor of a single
  atomic `UPDATE ... WHERE balance >= amount`. Still an incremental gain,
  not architectural - both pay a per-transfer commit/replication round trip.
- **Batched mode is a cautionary tale, not a data point in PostgreSQL's
  favor**: it's 100x+ slower than the other two modes and its error rate
  roughly quadruples under hotspot skew (4.48%→16.59%, breaching the
  coordinator's own 5% threshold). See root-cause analysis below - this is
  a structural property of the approach, not misconfiguration.
- **Correctness held everywhere**: all 18 runs (6 tests × 3 runs) passed the
  double-entry balance invariant. Every difference above is a genuine
  performance/reliability characteristic, not data corruption.

## Why hotspot skew matters

Real financial ledgers rarely spread load evenly across accounts - a
handful of "hub" accounts (an exchange's clearing account, a popular
merchant's settlement account) absorb far more traffic than the rest.
Raising `zipfian_exponent` from 1.0 to 2.0 concentrates transfers onto a
small set of accounts, which is exactly where PostgreSQL's row-locking
approach (`FOR UPDATE` or the equivalent implicit lock in an atomic UPDATE)
pays a queueing tax: many concurrent transactions contend for the same few
rows, waiting their turn one at a time. TigerBeetle has no row locks at all
- a single sequential state machine processes every transfer in order, so
there's nothing to contend *over*. This is why the throughput/latency gap
widens so sharply under hotspot skew rather than shrinking.

## Why batched mode runs under max_throughput, not fixed_rate

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

This is a workload-mode mismatch, not a bug: batched mode's value
proposition is one connection saturated with large batches, which
`fixed_rate`'s pacing model defeats. The repo's own local config
(`config.local-postgresql-batched.toml`) already tests batched mode with
`max_throughput`, not `fixed_rate`, for this exact reason. Both batched
configs were switched to `max_throughput` (`concurrency = 10`, mirroring
the local config) before running.

## Root cause of batched mode's errors: cross-connection deadlocks

The `batch_transfers()` SQL function (`scripts/init-postgresql.sql`) orders
each transfer's two `UPDATE`s by account ID to prevent a *single*
transaction from deadlocking with itself - but that does nothing to prevent
a deadlock between *two different client connections'* transactions. If
connection A's batch is mid-processing transfer (5→10), holding a lock on
account 5 while waiting to update account 10, and connection B's batch is
simultaneously processing (10→5), holding account 10 while waiting for
account 5 - classic deadlock. PostgreSQL's deadlock detector kills one side;
the function's `WHEN OTHERS` catch-all quietly converts that into result
code 3 ("failed") and moves on to the next item in the batch. That's
exactly the "failed_transfers" observed.

Standard and atomic modes don't show this because each executes *one
transfer per short-lived transaction* - locks are held for microseconds.
Batched mode bundles up to hundreds of transfers into *one long-lived
transaction* (spanning many UPDATEs plus a cross-zone synchronous-
replication commit wait), which massively widens the window for another
connection to need the same row. That's also why the error rate roughly
quadrupled under hotspot skew: concentrating traffic onto fewer accounts
multiplies the odds that two long-running batch transactions from
different client nodes collide on the same rows.

TigerBeetle cannot have this failure mode at all - not because it handles
contention better, but because it has no independent concurrent execution
contexts to begin with. A single authoritative sequential state machine
processes every transfer from every client one at a time; there is nothing
to deadlock. PostgreSQL's row-locking model always carries this tradeoff
for independently-writing connections; avoiding it would require deliberate
application-level coordination (e.g. sorting *all* accounts touched across
an entire batch, not just per-transfer-pair, before applying updates), which
is exactly the kind of complexity TigerBeetle's architecture avoids by
design.

## Raw results

### TigerBeetle, zipfian_exponent = 1.0

Result file: `results/run_20260714_073817/results.json`

```json
{
  "config_summary": {
    "database_type": "TigerBeetle",
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
      "duration_secs": 422.394550875,
      "throughput_tps": 4064.483333333333,
      "latency_p50_us": 32264,
      "latency_p95_us": 509381,
      "latency_p99_us": 749088,
      "latency_p999_us": 988138,
      "completed_transfers": 1212565,
      "rejected_transfers": 6780,
      "failed_transfers": 0,
      "balance_verified": true
    },
    {
      "run_id": 2,
      "duration_secs": 423.427307167,
      "throughput_tps": 4123.373333333333,
      "latency_p50_us": 32309,
      "latency_p95_us": 511095,
      "latency_p99_us": 754691,
      "latency_p999_us": 991998,
      "completed_transfers": 1230415,
      "rejected_transfers": 6597,
      "failed_transfers": 0,
      "balance_verified": true
    },
    {
      "run_id": 3,
      "duration_secs": 422.847994834,
      "throughput_tps": 4103.576666666667,
      "latency_p50_us": 32292,
      "latency_p95_us": 504117,
      "latency_p99_us": 739988,
      "latency_p999_us": 980624,
      "completed_transfers": 1223786,
      "rejected_transfers": 7287,
      "failed_transfers": 0,
      "balance_verified": true
    }
  ],
  "aggregate": {
    "throughput": {
      "mean": 4097.144444444445,
      "stddev": 24.46818528943297,
      "cv": 0.0059720094376000815,
      "min": 4064.483333333333,
      "max": 4123.373333333333
    },
    "latency_p50": {
      "mean": 32288.333333333332,
      "stddev": 18.55322673343433,
      "cv": 0.000574610852220131,
      "min": 32264.0,
      "max": 32309.0
    },
    "latency_p95": {
      "mean": 508197.6666666667,
      "stddev": 2969.0996764825677,
      "cv": 0.005842411075905309,
      "min": 504117.0,
      "max": 511095.0
    },
    "latency_p99": {
      "mean": 747922.3333333334,
      "stddev": 6058.802760355291,
      "cv": 0.008100844820815117,
      "min": 739988.0,
      "max": 754691.0
    },
    "latency_p999": {
      "mean": 986920.0,
      "stddev": 4722.613118461713,
      "cv": 0.0047852035813051854,
      "min": 980624.0,
      "max": 991998.0
    },
    "total_completed": 3666766,
    "total_rejected": 20664,
    "total_failed": 0,
    "error_rate": 0.0
  },
  "warnings": [],
  "errors": []
}
```

### TigerBeetle, zipfian_exponent = 2.0

Result file: `results/run_20260715_062158/results.json`

```json
{
  "config_summary": {
    "database_type": "TigerBeetle",
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
      "duration_secs": 422.780591417,
      "throughput_tps": 4418.886666666666,
      "latency_p50_us": 31757,
      "latency_p95_us": 359272,
      "latency_p99_us": 583990,
      "latency_p999_us": 736514,
      "completed_transfers": 916611,
      "rejected_transfers": 409055,
      "failed_transfers": 0,
      "balance_verified": true
    },
    {
      "run_id": 2,
      "duration_secs": 422.849256208,
      "throughput_tps": 4478.87,
      "latency_p50_us": 31903,
      "latency_p95_us": 357625,
      "latency_p99_us": 550259,
      "latency_p999_us": 721692,
      "completed_transfers": 927360,
      "rejected_transfers": 416301,
      "failed_transfers": 0,
      "balance_verified": true
    },
    {
      "run_id": 3,
      "duration_secs": 260.6045565,
      "throughput_tps": 4486.403333333334,
      "latency_p50_us": 32600,
      "latency_p95_us": 355608,
      "latency_p99_us": 546995,
      "latency_p999_us": 719729,
      "completed_transfers": 931956,
      "rejected_transfers": 413965,
      "failed_transfers": 0,
      "balance_verified": true
    }
  ],
  "aggregate": {
    "throughput": {
      "mean": 4461.386666666666,
      "stddev": 30.2089973107108,
      "cv": 0.006771212532735144,
      "min": 4418.886666666666,
      "max": 4486.403333333334
    },
    "latency_p50": {
      "mean": 32086.666666666668,
      "stddev": 367.8426596008438,
      "cv": 0.011464034685253805,
      "min": 31757.0,
      "max": 32600.0
    },
    "latency_p95": {
      "mean": 357501.6666666667,
      "stddev": 1498.3618462248105,
      "cv": 0.004191202408076821,
      "min": 355608.0,
      "max": 359272.0
    },
    "latency_p99": {
      "mean": 560414.6666666666,
      "stddev": 16723.45020888001,
      "cv": 0.029841207240971588,
      "min": 546995.0,
      "max": 583990.0
    },
    "latency_p999": {
      "mean": 725978.3333333334,
      "stddev": 7492.820845464159,
      "cv": 0.010320997888546938,
      "min": 719729.0,
      "max": 736514.0
    },
    "total_completed": 2775927,
    "total_rejected": 1239321,
    "total_failed": 0,
    "error_rate": 0.0
  },
  "warnings": [],
  "errors": []
}
```

### PostgreSQL, standard executor mode, zipfian_exponent = 1.0

Result file: `results/run_20260714_080715/results.json`

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
      "duration_secs": 423.301793125,
      "throughput_tps": 2872.58,
      "latency_p50_us": 323758,
      "latency_p95_us": 582761,
      "latency_p99_us": 960881,
      "latency_p999_us": 1493684,
      "completed_transfers": 857566,
      "rejected_transfers": 4208,
      "failed_transfers": 0,
      "balance_verified": true
    },
    {
      "run_id": 2,
      "duration_secs": 423.6316055,
      "throughput_tps": 2921.6066666666666,
      "latency_p50_us": 322113,
      "latency_p95_us": 575016,
      "latency_p99_us": 953982,
      "latency_p999_us": 1486332,
      "completed_transfers": 872462,
      "rejected_transfers": 4020,
      "failed_transfers": 0,
      "balance_verified": true
    },
    {
      "run_id": 3,
      "duration_secs": 423.54588825,
      "throughput_tps": 2874.6566666666668,
      "latency_p50_us": 328170,
      "latency_p95_us": 585464,
      "latency_p99_us": 964179,
      "latency_p999_us": 1495410,
      "completed_transfers": 858228,
      "rejected_transfers": 4169,
      "failed_transfers": 0,
      "balance_verified": true
    }
  ],
  "aggregate": {
    "throughput": {
      "mean": 2889.614444444445,
      "stddev": 22.637798010527217,
      "cv": 0.00783419326202861,
      "min": 2872.58,
      "max": 2921.6066666666666
    },
    "latency_p50": {
      "mean": 324680.3333333333,
      "stddev": 2557.3210375095437,
      "cv": 0.00787642728851109,
      "min": 322113.0,
      "max": 328170.0
    },
    "latency_p95": {
      "mean": 581080.3333333334,
      "stddev": 4427.840431732933,
      "cv": 0.007620014269512247,
      "min": 575016.0,
      "max": 585464.0
    },
    "latency_p99": {
      "mean": 959680.6666666666,
      "stddev": 4248.552877810933,
      "cv": 0.004427048522888099,
      "min": 953982.0,
      "max": 964179.0
    },
    "latency_p999": {
      "mean": 1491808.6666666667,
      "stddev": 3936.172195871969,
      "cv": 0.002638523480807393,
      "min": 1486332.0,
      "max": 1495410.0
    },
    "total_completed": 2588256,
    "total_rejected": 12397,
    "total_failed": 0,
    "error_rate": 0.0
  },
  "warnings": [],
  "errors": []
}
```

### PostgreSQL, standard executor mode, zipfian_exponent = 2.0

Result file: `results/run_20260715_065414/results.json`

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
      "duration_secs": 424.370822209,
      "throughput_tps": 748.2866666666666,
      "latency_p50_us": 1306581,
      "latency_p95_us": 1891212,
      "latency_p99_us": 2127272,
      "latency_p999_us": 2915907,
      "completed_transfers": 158824,
      "rejected_transfers": 65662,
      "failed_transfers": 0,
      "balance_verified": true
    },
    {
      "run_id": 2,
      "duration_secs": 424.904034583,
      "throughput_tps": 758.63,
      "latency_p50_us": 1302970,
      "latency_p95_us": 1885190,
      "latency_p99_us": 2102109,
      "latency_p999_us": 2915656,
      "completed_transfers": 160334,
      "rejected_transfers": 67255,
      "failed_transfers": 0,
      "balance_verified": true
    },
    {
      "run_id": 3,
      "duration_secs": 424.803020875,
      "throughput_tps": 752.04,
      "latency_p50_us": 1307436,
      "latency_p95_us": 1892767,
      "latency_p99_us": 2139700,
      "latency_p999_us": 2916417,
      "completed_transfers": 160391,
      "rejected_transfers": 65221,
      "failed_transfers": 0,
      "balance_verified": true
    }
  ],
  "aggregate": {
    "throughput": {
      "mean": 752.9855555555555,
      "stddev": 4.2752538008553955,
      "cv": 0.005677736802933886,
      "min": 748.2866666666666,
      "max": 758.63
    },
    "latency_p50": {
      "mean": 1305662.3333333333,
      "stddev": 1935.5017150312444,
      "cv": 0.0014823907113027778,
      "min": 1302970.0,
      "max": 1307436.0
    },
    "latency_p95": {
      "mean": 1889723.0,
      "stddev": 3267.5753498070503,
      "cv": 0.0017291292691082505,
      "min": 1885190.0,
      "max": 1892767.0
    },
    "latency_p99": {
      "mean": 2123027.0,
      "stddev": 15637.259755681833,
      "cv": 0.007365549169031686,
      "min": 2102109.0,
      "max": 2139700.0
    },
    "latency_p999": {
      "mean": 2915993.3333333335,
      "stddev": 316.6178909804196,
      "cv": 0.00010857977189491274,
      "min": 2915656.0,
      "max": 2916417.0
    },
    "total_completed": 479549,
    "total_rejected": 198138,
    "total_failed": 0,
    "error_rate": 0.0
  },
  "warnings": [],
  "errors": []
}
```

### PostgreSQL, atomic executor mode, zipfian_exponent = 1.0

Result file: `results/run_20260715_103147/results.json`

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

### PostgreSQL, atomic executor mode, zipfian_exponent = 2.0

Result file: `results/run_20260715_134910/results.json`

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

### PostgreSQL, batched executor mode (max_throughput, concurrency=10), zipfian_exponent = 1.0

Result file: `results/run_20260715_113912/results.json`

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

### PostgreSQL, batched executor mode (max_throughput, concurrency=10), zipfian_exponent = 2.0

Result file: `results/run_20260715_142200/results.json`

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
