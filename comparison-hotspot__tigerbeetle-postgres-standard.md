# Hotspot Contention Test — TigerBeetle vs PostgreSQL (standard)

`zipfian_exponent = 2.0` (vs the standard test's `1.0`), same cluster/workload
shape otherwise: `fixed_rate` mode, 100k accounts, 5 client nodes / 3 DB
nodes, 3x (2min warmup + 5min measurement) runs.

- TigerBeetle result: `results/run_20260715_062158/results.json`
- PostgreSQL (standard, `FOR UPDATE`) result: `results/run_20260715_065414/results.json`

## Comparison

| Metric | TigerBeetle @1.0 | TigerBeetle @2.0 | PostgreSQL @1.0 | PostgreSQL @2.0 |
|---|---|---|---|---|
| Mean throughput | 4,097 TPS | **4,461 TPS** (+9%) | 2,890 TPS | **753 TPS** (−74%) |
| p50 latency | 32.3 ms | 32.1 ms (flat) | 324.7 ms | **1,306 ms** (4x worse) |
| p95 latency | 508 ms | 358 ms (improved) | 581 ms | **1,890 ms** (3.3x worse) |
| p99 latency | 748 ms | 560 ms (improved) | 960 ms | **2,123 ms** (2.2x worse) |
| p999 latency | 987 ms | 726 ms (improved) | 1,492 ms | **2,916 ms** (2x worse) |
| Error rate | 0% | 0% | 0% | 0% |
| Balance verified | 3/3 | 3/3 | 3/3 | 3/3 |

At moderate skew (1.0), TigerBeetle was ~1.4x faster with ~10x lower median
latency. Under heavy hotspot contention (2.0), TigerBeetle got *faster and
lower-latency* (more rejections resolve quickly without lock waiting), while
PostgreSQL's throughput collapsed by 74% and its median latency quadrupled.
Net result: TigerBeetle ends up ~5.9x faster than PostgreSQL, with ~40x lower
p50 latency, under the same offered load.

## Raw results

### TigerBeetle (zipfian_exponent = 2.0)

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

### PostgreSQL, standard executor mode (zipfian_exponent = 2.0)

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
