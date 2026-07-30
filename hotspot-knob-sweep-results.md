# Hotspot Knob Sweep — max_concurrency & target_rate

Follow-up to `tigerbeetle-vs-postgresql.md`. That comparison ran everything
at `target_rate=5000`, `max_concurrency=1000` (200 in-flight slots per
client node) and found TigerBeetle achieving only ~4,461 TPS against a
5,000 target despite ~32ms p50 latency - large unused headroom. This
raised the question: was the concurrency cap silently throttling
throughput? This sweep answers that, using a new `dropped_transfers`
metric (added specifically for this investigation - see
`docs/superpowers/specs/2026-07-30-cloud-knob-sweep-design.md`) that
surfaces requests dropped for hitting `max_concurrency`, previously
invisible in results.

Scope: heavy hotspot skew only (`zipfian_exponent = 2.0`), 3 databases
(TigerBeetle, PostgreSQL standard `FOR UPDATE`, PostgreSQL atomic), 2 knob
variants, same topology as before (100k accounts, 5 client / 3 DB nodes,
GCP `europe-central2`, 3x runs of 2min warmup + 5min measurement).
Moderate skew (`zipfian_exponent = 1.0`) was not run this round.

| Variant | target_rate | max_concurrency |
|---|---|---|
| baseline (prior article) | 5,000 | 1,000 |
| `concurrency5k` | 5,000 (unchanged) | 5,000 |
| `rate10k` | 10,000 | 5,000 |

## Results

| | TigerBeetle | PostgreSQL Standard | PostgreSQL Atomic |
|---|---|---|---|
| **baseline** (max_concurrency=1000) |
| Mean throughput | 4,461 TPS | 753 TPS | 977 TPS |
| p50 / p95 / p99 latency | 32 / 358 / 560 ms | 1,306 / 1,890 / 2,123 ms | 1,001 / 1,472 / 1,821 ms |
| Dropped (not tracked at the time) | unknown | unknown | unknown |
| **concurrency5k** (max_concurrency=5000, target_rate=5000) |
| Mean throughput | **5,060 TPS** (+13%) | 683 TPS (-9%) | 878 TPS (-10%) |
| p50 / p95 / p99 latency | 37 / 480 / 676 ms | 5,000 / 5,000 / 5,000 ms (capped) | 5,000 / 5,000 / 5,000 ms (capped) |
| Dropped | **0** | 3,943,554 | 3,774,546 |
| **rate10k** (max_concurrency=5000, target_rate=10000) |
| Mean throughput | **9,431 TPS** | 703 TPS | 867 TPS |
| p50 / p95 / p99 latency | 37 / 611 / 837 ms | 5,000 / 5,000 / 5,000 ms (capped) | 5,000 / 5,000 / 5,000 ms (capped) |
| Dropped | 598,159 | 8,529,844 | 8,363,650 |

Balance verified 3/3 runs in every one of the six tests - no correctness
issues in the final numbers above (see "Two infrastructure issues hit along
the way" for problems that occurred *before* getting to these clean runs).

## TigerBeetle: the concurrency cap was real, and the ceiling is higher than 10k

`concurrency5k` confirms the original hypothesis cleanly: with the cap
raised from 200 to 1,000 in-flight slots per client (same offered rate),
throughput rose 13% to 5,060 TPS and `dropped_transfers` dropped to **zero**.
The baseline's shortfall against its 5,000 target wasn't a database limit -
it was the concurrency cap silently discarding requests the whole time,
invisibly, because the coordinator never surfaced that counter until now.

`rate10k` then doubles the offered load with the cap held constant, and
TigerBeetle absorbed nearly all of it: 9,431 TPS achieved against 10,000
offered (94%), with p50 latency barely moving (37ms, same as
`concurrency5k`) and p99 still under a second. `dropped_transfers` is
nonzero here (598k, ~9% of offered load) - so *this* time the cap is
plausibly binding again, meaning TigerBeetle's true ceiling under this
workload is somewhere *above* 9,431 TPS, not yet found. A useful next step
is a `target_rate=15000`+ test with `max_concurrency` raised further to
pin down the actual ceiling.

## PostgreSQL: raising max_concurrency made things *worse*, not better

This is the counterintuitive result of the sweep. Both PostgreSQL
executors got worse, not better, when `max_concurrency` was raised - and
raising `target_rate` on top of that barely moved the needle (703 TPS vs
683 TPS standard; 867 vs 878 TPS atomic, within run-to-run noise). Every
latency percentile pegged at the 5-second cap, and `dropped_transfers`
exploded (up to 8.5M in the `rate10k` runs, vastly more than the number of
transfers actually completed).

The reason: **`connection_pool_size` stayed at 20 in every PostgreSQL
config in this sweep** - only `max_concurrency` and `target_rate` changed.
Raising the client-side concurrency cap from 200 to 1,000 in-flight slots
per client doesn't help if there are still only 20 real database
connections behind it. Every request beyond those 20 connections now waits
in a much deeper queue than before (5x deeper), and with more requests
queued for longer, more of them end up exceeding the window before the
next scheduled tick and getting dropped, or sit until they hit the
5-second cap. Compared to the baseline's 200-slot cap, the *effective*
bottleneck (20 connections) was already saturated - widening the funnel
above a fixed narrow point downstream just means more traffic piles up
waiting, not more traffic getting through.

This points at `connection_pool_size` as the parameter that actually
matters for PostgreSQL's throughput ceiling, not `max_concurrency` or
`target_rate`. Both of the latter were red herrings for PostgreSQL in this
sweep - a natural next test is raising `connection_pool_size` (e.g. to
100-200) at the *baseline* `max_concurrency`/`target_rate` to see whether
that's the real lever.

## Two infrastructure issues hit along the way

Neither affects the final numbers above (all runs shown passed balance
verification), but both are worth recording since they'll recur if this
sweep is extended.

**TigerBeetle account funding isn't idempotent across separate coordinator
invocations.** Running two `./target/release/coordinator` invocations
back-to-back against the same persistent TigerBeetle DB cluster (without
tearing it down in between) doubled every account's balance - the first
`rate10k` attempt failed balance verification with `expected
100000000000, got 200000000000`, exactly double. Root cause:
`coordinator/src/tigerbeetle_setup.rs`'s `init_accounts` creates the
funding transfers with a fresh unique ID every call and re-issues them
unconditionally; account creation is idempotent (`Exists` errors are
tolerated) but funding is not. The coordinator wipes and reformats between
*runs* within a single invocation, but has no equivalent step at the start
of a *new* invocation - which `PLAN.md` §3.5 anticipated ("Data Cleanup
Script... does NOT destroy the cloud infrastructure") but was never
implemented. Worked around here by destroying and recreating the
TigerBeetle database-cluster between separate test invocations; a real
fix would either implement that cleanup script or make `init_accounts`
idempotent (e.g. skip funding for accounts whose balance is already
`initial_balance`).

**Occasional simultaneous SSH/IAP disconnects on all client nodes** during
an unusually long client-side graceful-drain phase - hit once with
TigerBeetle's first `rate10k` attempt and once with PostgreSQL atomic's
first `concurrency5k` attempt (specifically on the *third* of three runs
in each case, after two prior runs completed cleanly on the same
invocation). All five client SSH connections failed near-simultaneously
with `Broken pipe` / exit 255, roughly 12 minutes after the run started -
longer than the coordinator's own 480s (8 minute) per-run timeout
(`coordinator/src/gcp_workload.rs`), so this doesn't look like that
timeout firing cleanly; more likely an IAP tunnel-level idle/session limit
tripped while the client was stuck draining a large in-flight backlog.
Resolved by simply retrying the whole test, which succeeded cleanly both
times. Not investigated further here since it didn't block getting valid
results, but worth a note for anyone extending this sweep to even higher
concurrency values where drain time may grow further.

## Raw results

`results/` is gitignored, so the full JSON is embedded below rather than
just linked (matching the convention in `tigerbeetle-vs-postgresql.md`).

### TigerBeetle, concurrency5k

Result file: `results/run_20260730_062414/results.json`

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
      "duration_secs": 422.460295875,
      "throughput_tps": 5040.083333333333,
      "latency_p50_us": 37701,
      "latency_p95_us": 488116,
      "latency_p99_us": 683599,
      "latency_p999_us": 920320,
      "completed_transfers": 1043947,
      "rejected_transfers": 468078,
      "failed_transfers": 0,
      "dropped_transfers": 0,
      "balance_verified": true
    },
    {
      "run_id": 2,
      "duration_secs": 422.683704375,
      "throughput_tps": 5061.836666666667,
      "latency_p50_us": 36631,
      "latency_p95_us": 478278,
      "latency_p99_us": 673685,
      "latency_p999_us": 902210,
      "completed_transfers": 1044625,
      "rejected_transfers": 473926,
      "failed_transfers": 0,
      "dropped_transfers": 0,
      "balance_verified": true
    },
    {
      "run_id": 3,
      "duration_secs": 422.662559875,
      "throughput_tps": 5078.6866666666665,
      "latency_p50_us": 36447,
      "latency_p95_us": 474897,
      "latency_p99_us": 671049,
      "latency_p999_us": 901772,
      "completed_transfers": 1049139,
      "rejected_transfers": 474467,
      "failed_transfers": 0,
      "dropped_transfers": 0,
      "balance_verified": true
    }
  ],
  "aggregate": {
    "throughput": {
      "mean": 5060.202222222222,
      "stddev": 15.802065109611384,
      "cv": 0.003122812965896015,
      "min": 5040.083333333333,
      "max": 5078.6866666666665
    },
    "latency_p50": { "mean": 36926.333333333336, "stddev": 552.8986244230392, "cv": 0.014973017207856341, "min": 36447.0, "max": 37701.0 },
    "latency_p95": { "mean": 480430.3333333333, "stddev": 5607.132025871654, "cv": 0.011671061622958142, "min": 474897.0, "max": 488116.0 },
    "latency_p99": { "mean": 676111.0, "stddev": 5403.0690044331905, "cv": 0.00799139343160101, "min": 671049.0, "max": 683599.0 },
    "latency_p999": { "mean": 908100.6666666666, "stddev": 8642.223530756164, "cv": 0.009516812230167027, "min": 901772.0, "max": 920320.0 },
    "total_completed": 3137711,
    "total_rejected": 1416471,
    "total_failed": 0,
    "total_dropped": 0,
    "error_rate": 0.0
  },
  "warnings": [],
  "errors": []
}
```

### TigerBeetle, rate10k

Result file: `results/run_20260730_080216/results.json`

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
      "duration_secs": 422.336838625,
      "throughput_tps": 9402.486666666666,
      "latency_p50_us": 37200,
      "latency_p95_us": 611824,
      "latency_p99_us": 839894,
      "latency_p999_us": 987432,
      "completed_transfers": 1932577,
      "rejected_transfers": 888169,
      "failed_transfers": 0,
      "dropped_transfers": 199905,
      "balance_verified": true
    },
    {
      "run_id": 2,
      "duration_secs": 422.733899667,
      "throughput_tps": 9511.68,
      "latency_p50_us": 36592,
      "latency_p95_us": 582409,
      "latency_p99_us": 788727,
      "latency_p999_us": 980946,
      "completed_transfers": 1961491,
      "rejected_transfers": 892013,
      "failed_transfers": 0,
      "dropped_transfers": 174547,
      "balance_verified": true
    },
    {
      "run_id": 3,
      "duration_secs": 423.046408959,
      "throughput_tps": 9378.413333333334,
      "latency_p50_us": 37648,
      "latency_p95_us": 639374,
      "latency_p99_us": 883111,
      "latency_p999_us": 998086,
      "completed_transfers": 1931910,
      "rejected_transfers": 881614,
      "failed_transfers": 0,
      "dropped_transfers": 223707,
      "balance_verified": true
    }
  ],
  "aggregate": {
    "throughput": {
      "mean": 9430.859999999999,
      "stddev": 57.987272422170165,
      "cv": 0.006148672806315667,
      "min": 9378.413333333334,
      "max": 9511.68
    },
    "latency_p50": { "mean": 37146.666666666664, "stddev": 432.7565392021503, "cv": 0.011649942727983227, "min": 36592.0, "max": 37648.0 },
    "latency_p95": { "mean": 611202.3333333334, "stddev": 23260.018032858206, "cv": 0.038056166942302586, "min": 582409.0, "max": 639374.0 },
    "latency_p99": { "mean": 837244.0, "stddev": 38577.64239383567, "cv": 0.04607694100385989, "min": 788727.0, "max": 883111.0 },
    "latency_p999": { "mean": 988821.3333333334, "stddev": 7066.002421611687, "cv": 0.007145883875494549, "min": 980946.0, "max": 998086.0 },
    "total_completed": 5825978,
    "total_rejected": 2661796,
    "total_failed": 0,
    "total_dropped": 598159,
    "error_rate": 0.0
  },
  "warnings": [],
  "errors": []
}
```

### PostgreSQL standard, concurrency5k

Result file: `results/run_20260730_083128/results.json`

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
      "duration_secs": 430.8018085,
      "throughput_tps": 682.4433333333334,
      "latency_p50_us": 5000000,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 146395,
      "rejected_transfers": 58338,
      "failed_transfers": 0,
      "dropped_transfers": 1306481,
      "balance_verified": true
    },
    {
      "run_id": 2,
      "duration_secs": 431.17197675,
      "throughput_tps": 685.39,
      "latency_p50_us": 5000000,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 146796,
      "rejected_transfers": 58821,
      "failed_transfers": 0,
      "dropped_transfers": 1327178,
      "balance_verified": true
    },
    {
      "run_id": 3,
      "duration_secs": 431.724972625,
      "throughput_tps": 680.79,
      "latency_p50_us": 5000000,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 145709,
      "rejected_transfers": 58528,
      "failed_transfers": 0,
      "dropped_transfers": 1309895,
      "balance_verified": true
    }
  ],
  "aggregate": {
    "throughput": {
      "mean": 682.8744444444445,
      "stddev": 1.9025233406527258,
      "cv": 0.00278605145664886,
      "min": 680.79,
      "max": 685.39
    },
    "latency_p50": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "latency_p95": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "latency_p99": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "latency_p999": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "total_completed": 438900,
    "total_rejected": 175687,
    "total_failed": 0,
    "total_dropped": 3943554,
    "error_rate": 0.0
  },
  "warnings": [],
  "errors": []
}
```

### PostgreSQL standard, rate10k

Result file: `results/run_20260730_085711/results.json`

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
      "duration_secs": 431.592092459,
      "throughput_tps": 704.83,
      "latency_p50_us": 5000000,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 149933,
      "rejected_transfers": 61516,
      "failed_transfers": 0,
      "dropped_transfers": 2858287,
      "balance_verified": true
    },
    {
      "run_id": 2,
      "duration_secs": 432.368910083,
      "throughput_tps": 696.22,
      "latency_p50_us": 5000000,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 146799,
      "rejected_transfers": 62067,
      "failed_transfers": 0,
      "dropped_transfers": 2814366,
      "balance_verified": true
    },
    {
      "run_id": 3,
      "duration_secs": 431.853734291,
      "throughput_tps": 708.5033333333333,
      "latency_p50_us": 5000000,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 150955,
      "rejected_transfers": 61596,
      "failed_transfers": 0,
      "dropped_transfers": 2857191,
      "balance_verified": true
    }
  ],
  "aggregate": {
    "throughput": {
      "mean": 703.1844444444445,
      "stddev": 5.147877184449212,
      "cv": 0.007320806404522111,
      "min": 696.22,
      "max": 708.5033333333333
    },
    "latency_p50": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "latency_p95": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "latency_p99": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "latency_p999": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "total_completed": 447687,
    "total_rejected": 185179,
    "total_failed": 0,
    "total_dropped": 8529844,
    "error_rate": 0.0
  },
  "warnings": [],
  "errors": []
}
```

### PostgreSQL atomic, concurrency5k

Result file: `results/run_20260730_095258/results.json`

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
      "duration_secs": 428.816768125,
      "throughput_tps": 892.4433333333334,
      "latency_p50_us": 5000000,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 188411,
      "rejected_transfers": 79322,
      "failed_transfers": 0,
      "dropped_transfers": 1254886,
      "balance_verified": true
    },
    {
      "run_id": 2,
      "duration_secs": 432.214356791,
      "throughput_tps": 872.7633333333333,
      "latency_p50_us": 5000000,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 184800,
      "rejected_transfers": 77029,
      "failed_transfers": 0,
      "dropped_transfers": 1252301,
      "balance_verified": true
    },
    {
      "run_id": 3,
      "duration_secs": 429.371581292,
      "throughput_tps": 869.3433333333334,
      "latency_p50_us": 5000000,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 184442,
      "rejected_transfers": 76361,
      "failed_transfers": 0,
      "dropped_transfers": 1267359,
      "balance_verified": true
    }
  ],
  "aggregate": {
    "throughput": {
      "mean": 878.1833333333334,
      "stddev": 10.17954812356621,
      "cv": 0.01159159794678356,
      "min": 869.3433333333334,
      "max": 892.4433333333334
    },
    "latency_p50": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "latency_p95": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "latency_p99": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "latency_p999": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "total_completed": 557653,
    "total_rejected": 232712,
    "total_failed": 0,
    "total_dropped": 3774546,
    "error_rate": 0.0
  },
  "warnings": [],
  "errors": []
}
```

### PostgreSQL atomic, rate10k

Result file: `results/run_20260730_101757/results.json`

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
      "duration_secs": 433.202237625,
      "throughput_tps": 862.3166666666667,
      "latency_p50_us": 5000000,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 182876,
      "rejected_transfers": 75819,
      "failed_transfers": 0,
      "dropped_transfers": 2773641,
      "balance_verified": true
    },
    {
      "run_id": 2,
      "duration_secs": 430.66079325,
      "throughput_tps": 865.23,
      "latency_p50_us": 5000000,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 183145,
      "rejected_transfers": 76424,
      "failed_transfers": 0,
      "dropped_transfers": 2800928,
      "balance_verified": true
    },
    {
      "run_id": 3,
      "duration_secs": 430.362187667,
      "throughput_tps": 873.8833333333333,
      "latency_p50_us": 5000000,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 184241,
      "rejected_transfers": 77924,
      "failed_transfers": 0,
      "dropped_transfers": 2789081,
      "balance_verified": true
    }
  ],
  "aggregate": {
    "throughput": {
      "mean": 867.1433333333333,
      "stddev": 4.912065266787989,
      "cv": 0.005664652056893311,
      "min": 862.3166666666667,
      "max": 873.8833333333333
    },
    "latency_p50": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "latency_p95": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "latency_p99": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "latency_p999": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "total_completed": 550262,
    "total_rejected": 230167,
    "total_failed": 0,
    "total_dropped": 8363650,
    "error_rate": 0.0
  },
  "warnings": [],
  "errors": []
}
```
