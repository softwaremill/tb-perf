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
| `rate20k` (TigerBeetle only) | 20,000 | 30,000 |
| `rate40k` (TigerBeetle only) | 40,000 | 100,000 |
| `rate80k` (TigerBeetle only) | 80,000 | 200,000 |
| `rate160k` (TigerBeetle only) | 160,000 | 400,000 |

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
| **rate20k** (max_concurrency=30000, target_rate=20000, TigerBeetle only) |
| Mean throughput | **20,257 TPS** | not tested | not tested |
| p50 / p95 / p99 latency | 43 / 846 / 1,046 ms | - | - |
| Dropped | **0** | - | - |
| **rate40k** (max_concurrency=100000, target_rate=40000, TigerBeetle only) |
| Mean throughput | **40,388 TPS** | not tested | not tested |
| p50 / p95 / p99 latency | 73 / 948 / 1,312 ms | - | - |
| Dropped | **0** | - | - |
| **rate80k** (max_concurrency=200000, target_rate=80000, TigerBeetle only) |
| Mean throughput | **81,171 TPS** | not tested | not tested |
| p50 / p95 / p99* latency | **664** / **1,288** / 1,458* ms | - | - |
| Dropped | **0** | - | - |
| **rate160k** (max_concurrency=400000, target_rate=160000, TigerBeetle only) |
| Mean throughput | **107,858 TPS** (67% of offered - real shortfall) | not tested | not tested |
| p50 / p95 / p99 latency | **3,611** / **5,469** / **7,294** ms (real values, no longer bucket-capped) | - | - |
| Dropped | **~34%** of offered load (16.6-16.7M/run) | - | - |

\* p99 and p999 at `rate80k` (and p999 at `rate40k`) sat right against the
histogram's old 1.5s bucket boundary - confirmed a measurement artifact,
not a real value, once `rate160k` proved latency grows freely well past
1.5s when the histogram has room. See below for the full story and an
important caveat on the `rate160k` numbers themselves (2 of 3 runs, due to
an infrastructure issue - not a data-quality problem, but worth reading
before citing these numbers).

Balance verified 3/3 runs in every test through `rate80k`, and 2/2 runs
that completed for `rate160k` (see caveat below) - no correctness issues
anywhere (see "Infrastructure issues hit along the way" for problems that
occurred *before* getting to these clean runs).

## TigerBeetle: the concurrency cap was real, and we found the real ceiling at rate160k

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
workload is somewhere *above* 9,431 TPS, not yet found.

`rate20k` follows up on exactly that: target_rate doubled again to 20,000,
and this time `max_concurrency` was raised generously (30,000, i.e.
6,000/client against 4,000/client offered - well past 1:1, since
TigerBeetle's concurrency cap is a pure client-side safety valve with no
corresponding server-side resource limit, unlike PostgreSQL's connection
pool, so there's no cost to overprovisioning it). Result: **20,257 TPS
mean, `dropped_transfers = 0`** - TigerBeetle absorbed essentially the
entire 20,000 offered rate (101%) with real headroom in the concurrency
cap this time, not just "not much dropped." Latency did grow somewhat
(p50 37ms -> 43ms, p99 837ms -> 1,046ms), but stayed low and sub-second at
every percentile through p99. **TigerBeetle's ceiling under hotspot skew
still hasn't been found** - it absorbed a 2x jump in offered load with
zero drops and only a modest latency increase, suggesting there's
significant headroom left even above 20,000 TPS.

`rate40k` pushes further still: target_rate doubled again to 40,000, with
`max_concurrency` raised to 100,000 (20,000/client against 8,000/client
offered, 2.5x headroom - generous on purpose, since a real ceiling should
show up as latency growth, not an artificial drop from an undersized cap).
Result: **40,388 TPS mean, `dropped_transfers = 0`** - throughput again
essentially matched the full offered rate (101%).

Latency growth per doubling, by percentile:

| Step | p50 | p95 | p99 | p999 |
|---|---|---|---|---|
| concurrency5k -> rate10k | +0% (37->37ms) | +27% (480->611ms) | +24% (676->837ms) | +9% (908->989ms) |
| rate10k -> rate20k | +16% (37->43ms) | +39% (611->846ms) | +25% (837->1,046ms) | +47% (989->1,454ms) |
| rate20k -> rate40k | +70% (43->73ms) | +12% (846->948ms) | +25% (1,046->1,312ms) | +2%\* (1,454->1,481ms) |
| rate40k -> rate80k | **+810%** (73->664ms) | **+36%** (948->1,288ms) | +11%\* (1,312->1,458ms) | +1%\* (1,481->1,496ms) |

\* **Correction:** the original version of this doc read the flattening
p999 growth at `rate40k` (+2%) as "bumping against a separate, roughly
fixed tail ceiling." That interpretation was wrong. `client/src/metrics.rs`
bounds the exported latency histogram's buckets up to `1,500,000us` (1.5s)
as its second-highest boundary - and both `rate40k`'s p999 (1,481ms) and
`rate80k`'s p999 (1,496ms), plus now `rate80k`'s p99 too (1,458ms), are
sitting within a few milliseconds of that exact boundary. Worse, the
run-to-run coefficient of variation on `rate80k`'s p99/p999 is
essentially zero (0.03% and 0.008% - all three runs landing within a few
hundred *microseconds* of each other), which is the signature of a
value being capped by a bucket edge, not organically converging. **We
don't have a trustworthy number for real tail latency beyond ~1.5s at
either `rate40k` or `rate80k`** - it could be exactly what's shown, or it
could be much worse; the histogram simply can't tell us. This is the same
category of measurement gap as PostgreSQL's 5-second histogram cap found
earlier in this sweep, just less obvious since 1.5s isn't a suspiciously
round number the way 5,000ms is.

The trustworthy signal is p50 and p95, since neither sits anywhere near a
bucket boundary at any step. And on the `rate40k -> rate80k` step, that
signal is dramatic: **p50 exploded 810% (73ms -> 664ms)**, and p95 grew
36% (948ms -> 1,288ms) - both far larger jumps than any previous doubling.
Throughput still tracked the offered rate almost exactly (81,171 TPS
against 80,000 offered, 101%) with `dropped_transfers = 0` - so nothing in
the coordinator's own pass/fail thresholds (error rate, drops) would flag
this run as degraded. At the time, we read this as the strongest signal
yet that we were at or very near TigerBeetle's real ceiling, and stopped
there - reasoning that pushing further without fixing the histogram first
would just repeat the same measurement problem at a new boundary.

**We were wrong about `rate80k` being the ceiling - it wasn't, and here's
the proof.** Before pushing further, we widened `client/src/metrics.rs`'s
histogram buckets from a 1.5s max up to 20s, specifically so the next test
could tell a real architectural cap apart from a bucket-boundary artifact.
Then we ran `rate160k`: target_rate doubled again to 160,000,
`max_concurrency` raised to 400,000 (same 2.5x headroom ratio used
throughout). The result settled the question decisively:

- **Mean throughput: 107,858 TPS - only 67% of the 160,000 offered rate.**
  Every single prior step in this sweep achieved ~100-101% of its offered
  rate. This is the first real throughput shortfall in the entire
  investigation.
- **~34% of offered load dropped** (16.6-16.7M dropped transfers per run,
  out of ~48-49M offered) - substantial, real drops, a world away from the
  0-9% seen at every earlier step.
- **Real tail latency, finally unbounded by the histogram: p50 ~3.6s, p95
  up to 6.2s, p99 up to 9.6s, p999 up to 14.2s.** With buckets now
  available up to 20s, these are genuine measured values, not artifacts.

That last point is the key methodological payoff: **latency at `rate160k`
grew freely to multiple seconds once the histogram had room to show it.**
If `rate40k`/`rate80k`'s ~1.5s plateau had been a real architectural
ceiling, `rate160k`'s numbers would have plateaued at roughly the same
point too, just with a different (higher) bucket boundary in the way. They
didn't - they kept climbing well past 1.5s, confirming the earlier
plateau really was the histogram artifact we suspected, not a genuine cap.
TigerBeetle's actual ceiling under this hotspot workload sits somewhere
between `rate80k` (clean: 0 drops, 664ms p50) and `rate160k` (degraded:
~34% drops, 3.6s+ p50) - `rate80k`'s dramatic p50 growth (+810%) was a
real, correctly-read warning sign of approaching saturation; it just
wasn't the wall itself.

**A caveat on the `rate160k` numbers: they come from 2 of the planned 3
runs, not a full coordinator-produced aggregate.** The coordinator process
was killed by what looks like a hard ~24.5-minute limit on the background
task running it - hit three separate times, always at almost exactly the
same elapsed time, always right as the *third* run's balance verification
was starting (after both prior runs had completed and passed balance
verification cleanly). More frequent check-ins on the process didn't
prevent it, which argues against it being caused by any particular polling
behavior. Rather than keep retrying an ~26-minute test against what
appears to be a hard external ceiling, we extracted the two runs that did
complete (both with clean balance verification, both showing the same
substantial-drop/multi-second-latency picture) and reported their
average directly instead of a full three-run aggregate with
standard-deviation/CV statistics. The two runs agree closely on
throughput (106,866 vs 108,850 TPS) and drop rate (34.2% vs 33.9%), but
latency grew noticeably between them (p999 4.99s -> 14.24s) even after the
30-second stabilization wait and account reset between runs - suggesting
that at this level of overload, the backlog from one run doesn't fully
clear before the next begins. That's a real finding worth flagging on its
own, not just a caveat about our sample size: whatever is queueing under
this load takes longer than 30 seconds to drain.

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

## Follow-up: connection_pool_size=50 (both executors, both knob variants)

Four more configs (`*-pool50.toml`) re-ran the `concurrency5k`/`rate10k`
knob values above with `connection_pool_size` raised from 20 to 50, to
test whether the connection pool is actually the lever. First attempt hit
a second, real bottleneck: **PostgreSQL's server-side `max_connections`
was still 200** (`scripts/gcp-setup-postgresql.sh`), and 5 client nodes x
50 connections each = 250 total client connections exceeds that - so
raising the pool size just moved the ceiling from "client waits for a
free pooled connection" to "server refuses the connection outright." The
first `concurrency5k` (standard executor) run at pool_size=50 completed
with a **68% error rate** (`"Failed to get connection"`), throughput
collapsing to 356 TPS - worse than doing nothing. Retroactively, this also
explains the *original* baseline: 5 x 20 = exactly 100, meaning pool=20
was already sitting exactly at PostgreSQL's default `max_connections`
before anyone even set it to 200 explicitly.

Fix: raised `max_connections` to 300 in both the primary and standby
`docker run` invocations in `scripts/gcp-setup-postgresql.sh` (both must
match - a standby's `max_connections` must be >= the primary's or it
refuses to start). With that in place, all four pool50 configs completed
cleanly - 0 failed transfers, 0% error rate, balance verified 3/3 in every
test:

| | Standard `FOR UPDATE` | Atomic |
|---|---|---|
| baseline (pool=20, cap=1000/client) | 753 TPS | 977 TPS |
| concurrency5k (pool=20, cap=5000 total) | 683 TPS | 878 TPS |
| **concurrency5k, pool=50** | **642 TPS** | **784 TPS** |
| rate10k (pool=20, cap=5000 total) | 703 TPS | 867 TPS |
| **rate10k, pool=50** | **653 TPS** | **796 TPS** |

Raising the pool 2.5x (20 -> 50) did **not** raise throughput for either
executor - if anything, standard came in slightly lower than its pool=20
`concurrency5k`/`rate10k` numbers, and atomic landed a bit lower than its
own pool=20 numbers too, though within a similar range. Both executors
remain far below their own *original* baseline (753/977 TPS), and further
below TigerBeetle's equivalent numbers (5,060-9,431 TPS). Every latency
percentile is still pegged at the 5-second histogram-bucket ceiling in
every pool50 config (see the design-doc caveat on this in
`docs/superpowers/specs/2026-07-30-cloud-knob-sweep-design.md` discussion -
`client/src/metrics.rs:125` caps the exported histogram at 5,000,000us, so
these numbers say "at least 5s," not "exactly 5s").

**Conclusion: `connection_pool_size` was not the real lever either**, once
`max_connections` stopped being the confound. Under heavy hotspot skew,
PostgreSQL's bottleneck for both executors is very likely the row-lock
contention on `FOR UPDATE` (standard) or the implicit lock in the atomic
`UPDATE ... WHERE balance >= amount` (atomic) - exactly the mechanism the
very first local/cloud comparison (`tigerbeetle-vs-postgresql.md`)
identified as PostgreSQL's fundamental weak point under contention. More
available connections just means more transactions competing for the same
few hot rows, not more real parallelism. None of the three knobs tested
across this whole sweep (`max_concurrency`, `target_rate`,
`connection_pool_size`) meaningfully move PostgreSQL's throughput under
hotspot skew - which is itself a meaningful result: the constraint is
architectural, not a tuning gap.

## Infrastructure issues hit along the way

None of these affect the final numbers above (all runs shown passed
balance verification), but they're worth recording since they'll recur if
this sweep is extended.

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
an unusually long client-side graceful-drain phase - hit repeatedly across
this whole investigation (TigerBeetle's first `rate10k` attempt,
PostgreSQL atomic's first `concurrency5k` attempt, and twice more during
the pool50 follow-up), always on the *third* of three runs after the first
two completed cleanly on the same invocation, and once as long as 23
minutes into the run rather than the usual ~12. All five client SSH
connections fail near-simultaneously with `Broken pipe` / exit 255 - later
than the coordinator's own 480s (8 minute) per-run timeout
(`coordinator/src/gcp_workload.rs`), so this doesn't look like that
timeout firing cleanly; more likely an IAP tunnel-level idle/session limit
tripped while the client is stuck draining a large in-flight backlog (the
one 23-minute instance happened on `connection_pool_size=50` *before* the
`max_connections` fix below, when connections were being refused
server-side rather than just queued - a plausibly even larger backlog to
drain). Resolved every time by simply retrying the whole test. Not
investigated further here since it never blocked getting valid results,
but worth a note for anyone extending this sweep further, especially at
higher concurrency where drain time may grow more.

**PostgreSQL's `max_connections` (200) becomes a hard ceiling once
`connection_pool_size` is raised enough that clients x pool exceeds it** -
see the pool50 section above. This is really a config gap rather than a
transient issue: the setup script (`scripts/gcp-setup-postgresql.sh`)
hardcoded `max_connections=200` for both primary and standbys, which
happened to be enough headroom for the pool=20 baseline (5 x 20 = 100
connections) and the pool=20 concurrency5k/rate10k configs (same pool
size, only the client-side cap changed) but not for pool=50 (5 x 50 =
250). Fixed by raising it to 300 in both places (primary and standby
values must match or the standby refuses to start).

**A transient GCP-side authentication error hit `rate40k`'s first
attempt** - not related to load at all. Run 1 completed cleanly (~40,104
TPS, 0 drops - a valid data point on its own, though not used in the final
numbers above since the invocation as a whole didn't finish), but the
cluster-reconfiguration step before run 2 failed with `scp ... ERROR:
(gcloud.compute.scp) Could not fetch resource: - Authentication backend
unavailable` while copying the TigerBeetle setup script to a DB node. This
looks like a one-off GCP IAM/OS Login hiccup rather than anything caused
by the test itself - a full retry of the whole invocation succeeded
cleanly on the first attempt (3/3 runs, no further errors).

**`rate160k` needed seven attempts to get two usable runs, for a mix of
reasons - most of them our own doing, one of them still unexplained.** In
rough order: (1) a hard ~24.5-minute limit on the background task running
the coordinator killed the process during the third run's balance
verification, having already completed runs 1 and 2 cleanly - hit on the
1st, 2nd, and 7th attempts, with no correlation to check-in frequency;
(2) killing the local coordinator process to recover from that left the
*remote* client processes on the VMs still running and holding the
deployed binary open, so the next attempt's `scp` failed with `dest open
"client": Failure` - fixed by recreating the client-cluster; (3) not
tearing down the DB cluster between the killed attempt and the next retry
meant the next invocation's account-funding step ran against
already-funded accounts and doubled every balance - the same non-idempotent
`init_accounts` bug documented above, this time self-inflicted rather than
from a genuinely separate test - fixed by recreating the database-cluster;
(4) a one-off IAP tunnel `ConnectionCreationError` unrelated to any of the
above. The one attempt that finally got two clean, complete runs (the data
reported above) was itself killed by the same ~24.5-minute limit during
run 3 - meaning **every single attempt at this specific knob combination
hit that same background-task ceiling**, regardless of infrastructure
state, check-in cadence, or how many times we'd already worked around
other issues. Given the test's natural runtime (3 runs x ~7min + ~3min
setup = ~24min) sits right at that limit, this is worth flagging
explicitly for anyone else running a similarly long single coordinator
invocation from a background shell.

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

### TigerBeetle, rate20k

Result file: `results/run_20260807_071309/results.json`

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
      "duration_secs": 422.26971175,
      "throughput_tps": 20152.93,
      "latency_p50_us": 43492,
      "latency_p95_us": 861992,
      "latency_p99_us": 1029285,
      "latency_p999_us": 1452928,
      "completed_transfers": 4131887,
      "rejected_transfers": 1913992,
      "failed_transfers": 0,
      "dropped_transfers": 0,
      "balance_verified": true
    },
    {
      "run_id": 2,
      "duration_secs": 423.530890333,
      "throughput_tps": 20308.69,
      "latency_p50_us": 43833,
      "latency_p95_us": 856669,
      "latency_p99_us": 1112028,
      "latency_p999_us": 1461202,
      "completed_transfers": 4170273,
      "rejected_transfers": 1922334,
      "failed_transfers": 0,
      "dropped_transfers": 0,
      "balance_verified": true
    },
    {
      "run_id": 3,
      "duration_secs": 423.501136209,
      "throughput_tps": 20310.763333333332,
      "latency_p50_us": 42619,
      "latency_p95_us": 820425,
      "latency_p99_us": 998019,
      "latency_p999_us": 1447665,
      "completed_transfers": 4170094,
      "rejected_transfers": 1923135,
      "failed_transfers": 0,
      "dropped_transfers": 0,
      "balance_verified": true
    }
  ],
  "aggregate": {
    "throughput": {
      "mean": 20257.46111111111,
      "stddev": 73.91950383297687,
      "cv": 0.0036490013939818157,
      "min": 20152.93,
      "max": 20310.763333333332
    },
    "latency_p50": { "mean": 43314.666666666664, "stddev": 511.2301069207703, "cv": 0.011802702092919342, "min": 42619.0, "max": 43833.0 },
    "latency_p95": { "mean": 846362.0, "stddev": 18468.523835614655, "cv": 0.021821069277229665, "min": 820425.0, "max": 861992.0 },
    "latency_p99": { "mean": 1046444.0, "stddev": 48099.45585970802, "cv": 0.04596467260523068, "min": 998019.0, "max": 1112028.0 },
    "latency_p999": { "mean": 1453931.6666666667, "stddev": 5571.84005114608, "cv": 0.0038322571678490714, "min": 1447665.0, "max": 1461202.0 },
    "total_completed": 12472254,
    "total_rejected": 5759461,
    "total_failed": 0,
    "total_dropped": 0,
    "error_rate": 0.0
  },
  "warnings": [],
  "errors": []
}
```

### TigerBeetle, rate40k

Result file: `results/run_20260807_082727/results.json`

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
      "duration_secs": 422.282383416,
      "throughput_tps": 39722.51,
      "latency_p50_us": 74189,
      "latency_p95_us": 946028,
      "latency_p99_us": 1305695,
      "latency_p999_us": 1480569,
      "completed_transfers": 8150380,
      "rejected_transfers": 3766373,
      "failed_transfers": 0,
      "dropped_transfers": 0,
      "balance_verified": true
    },
    {
      "run_id": 2,
      "duration_secs": 423.131516666,
      "throughput_tps": 40947.43,
      "latency_p50_us": 72542,
      "latency_p95_us": 954033,
      "latency_p99_us": 1327150,
      "latency_p999_us": 1482715,
      "completed_transfers": 8375972,
      "rejected_transfers": 3908257,
      "failed_transfers": 0,
      "dropped_transfers": 0,
      "balance_verified": true
    },
    {
      "run_id": 3,
      "duration_secs": 422.618092708,
      "throughput_tps": 40493.19666666666,
      "latency_p50_us": 73286,
      "latency_p95_us": 943178,
      "latency_p99_us": 1302032,
      "latency_p999_us": 1480203,
      "completed_transfers": 8298347,
      "rejected_transfers": 3849612,
      "failed_transfers": 0,
      "dropped_transfers": 0,
      "balance_verified": true
    }
  ],
  "aggregate": {
    "throughput": {
      "mean": 40387.71222222222,
      "stddev": 505.6035849126687,
      "cv": 0.012518747834260203,
      "min": 39722.51,
      "max": 40947.43
    },
    "latency_p50": { "mean": 73339.0, "stddev": 673.4285411237038, "cv": 0.009182406920243033, "min": 72542.0, "max": 74189.0 },
    "latency_p95": { "mean": 947746.3333333334, "stddev": 4595.0885615936595, "cv": 0.004848437181953743, "min": 943178.0, "max": 954033.0 },
    "latency_p99": { "mean": 1311625.6666666667, "stddev": 11078.751022665967, "cv": 0.008446579923082195, "min": 1302032.0, "max": 1327150.0 },
    "latency_p999": { "mean": 1481162.3333333333, "stddev": 1108.0220615533287, "cv": 0.00074807604583067, "min": 1480203.0, "max": 1482715.0 },
    "total_completed": 24824699,
    "total_rejected": 11524242,
    "total_failed": 0,
    "total_dropped": 0,
    "error_rate": 0.0
  },
  "warnings": [],
  "errors": []
}
```

### TigerBeetle, rate80k

Result file: `results/run_20260807_091056/results.json`

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
      "duration_secs": 422.956180042,
      "throughput_tps": 80669.69666666667,
      "latency_p50_us": 670047,
      "latency_p95_us": 1288070,
      "latency_p99_us": 1457614,
      "latency_p999_us": 1495761,
      "completed_transfers": 16519358,
      "rejected_transfers": 7681551,
      "failed_transfers": 0,
      "dropped_transfers": 0,
      "balance_verified": true
    },
    {
      "run_id": 2,
      "duration_secs": 423.295465584,
      "throughput_tps": 81445.72,
      "latency_p50_us": 654291,
      "latency_p95_us": 1289401,
      "latency_p99_us": 1458046,
      "latency_p999_us": 1495991,
      "completed_transfers": 16643440,
      "rejected_transfers": 7790276,
      "failed_transfers": 0,
      "dropped_transfers": 0,
      "balance_verified": true
    },
    {
      "run_id": 3,
      "duration_secs": 423.429300959,
      "throughput_tps": 81398.06,
      "latency_p50_us": 668093,
      "latency_p95_us": 1285060,
      "latency_p99_us": 1457012,
      "latency_p999_us": 1495701,
      "completed_transfers": 16648025,
      "rejected_transfers": 7771393,
      "failed_transfers": 0,
      "dropped_transfers": 0,
      "balance_verified": true
    }
  ],
  "aggregate": {
    "throughput": {
      "mean": 81171.1588888889,
      "stddev": 355.1207673801829,
      "cv": 0.004374962391091272,
      "min": 80669.69666666667,
      "max": 81445.72
    },
    "latency_p50": { "mean": 664143.6666666666, "stddev": 7012.408494154408, "cv": 0.010558571655662466, "min": 654291.0, "max": 670047.0 },
    "latency_p95": { "mean": 1287510.3333333333, "stddev": 1815.8543137842553, "cv": 0.0014103609631489732, "min": 1285060.0, "max": 1289401.0 },
    "latency_p99": { "mean": 1457557.3333333333, "stddev": 424.02620464096583, "cv": 0.00029091562640026455, "min": 1457012.0, "max": 1458046.0 },
    "latency_p999": { "mean": 1495817.6666666667, "stddev": 124.98888839501782, "cv": 0.00008355890639635745, "min": 1495701.0, "max": 1495991.0 },
    "total_completed": 49810823,
    "total_rejected": 23243220,
    "total_failed": 0,
    "total_dropped": 0,
    "error_rate": 0.0
  },
  "warnings": [],
  "errors": []
}
```

### TigerBeetle, rate160k

**No `results.json` was produced** - the coordinator process was killed
during run 3's balance verification before it could write results (see
"Infrastructure issues hit along the way" above). The two runs below were
extracted manually from the coordinator's log output
(`coordinator::prometheus: Collected metrics: ...` lines) rather than the
standard export format. `throughput_tps` here is computed the same way the
coordinator computes it: `(completed_transfers + rejected_transfers) /
300`.

```
Run 1 (from log, run_20260807_120613, ~12:14:58 UTC):
  completed_transfers: 21845848
  rejected_transfers:  10213879
  failed_transfers:    0
  dropped_transfers:   16629772
  latency_p50_us:      3588627
  latency_p95_us:      4732599
  latency_p99_us:      4946519
  latency_p999_us:     4994651
  throughput_tps:      106865.76  (= (21845848 + 10213879) / 300)
  balance_verified:    true (100000000000)

Run 2 (from log, ~12:23:34 UTC):
  completed_transfers: 22250816
  rejected_transfers:  10404259
  failed_transfers:    0
  dropped_transfers:   16748593
  latency_p50_us:      3633726
  latency_p95_us:      6204574
  latency_p99_us:      9641193
  latency_p999_us:     14241935
  throughput_tps:      108850.25  (= (22250816 + 10404259) / 300)
  balance_verified:    true (100000000000)

Manual 2-run average (not a coordinator-computed aggregate - no stddev/CV,
since n=2 rather than the usual n=3):
  throughput_tps:  107858.00
  latency_p50_us:  3611176   (~3,611ms)
  latency_p95_us:  5468586   (~5,469ms)
  latency_p99_us:  7293856   (~7,294ms)
  latency_p999_us: 9618293   (~9,618ms) - note the wide run-to-run spread
                    here (4.99s vs 14.24s), unlike every other test in this
                    sweep where percentiles were tightly clustered across
                    runs (CV well under 5%). Treat this average as
                    directional, not precise.
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

### PostgreSQL standard, concurrency5k, pool_size=50

Result file: `results/run_20260730_123025/results.json`

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
      "duration_secs": 431.151656209,
      "throughput_tps": 660.1433333333333,
      "latency_p50_us": 5000000,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 140724,
      "rejected_transfers": 57319,
      "failed_transfers": 0,
      "dropped_transfers": 1334751,
      "balance_verified": true
    },
    {
      "run_id": 2,
      "duration_secs": 433.275300666,
      "throughput_tps": 635.4433333333334,
      "latency_p50_us": 5000000,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 135566,
      "rejected_transfers": 55067,
      "failed_transfers": 0,
      "dropped_transfers": 1328058,
      "balance_verified": true
    },
    {
      "run_id": 3,
      "duration_secs": 432.666202541,
      "throughput_tps": 631.4033333333333,
      "latency_p50_us": 5000000,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 134838,
      "rejected_transfers": 54583,
      "failed_transfers": 0,
      "dropped_transfers": 1352634,
      "balance_verified": true
    }
  ],
  "aggregate": {
    "throughput": {
      "mean": 642.3299999999999,
      "stddev": 12.703451849355302,
      "cv": 0.019777142355728836,
      "min": 631.4033333333333,
      "max": 660.1433333333333
    },
    "latency_p50": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "latency_p95": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "latency_p99": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "latency_p999": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "total_completed": 411128,
    "total_rejected": 166969,
    "total_failed": 0,
    "total_dropped": 4015443,
    "error_rate": 0.0
  },
  "warnings": [],
  "errors": []
}
```

### PostgreSQL standard, rate10k, pool_size=50

Result file: `results/run_20260731_070224/results.json`

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
      "duration_secs": 432.290006708,
      "throughput_tps": 646.2266666666667,
      "latency_p50_us": 5000000,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 137484,
      "rejected_transfers": 56384,
      "failed_transfers": 0,
      "dropped_transfers": 2816688,
      "balance_verified": true
    },
    {
      "run_id": 2,
      "duration_secs": 432.811347375,
      "throughput_tps": 664.2533333333333,
      "latency_p50_us": 5000000,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 141526,
      "rejected_transfers": 57750,
      "failed_transfers": 0,
      "dropped_transfers": 2879711,
      "balance_verified": true
    },
    {
      "run_id": 3,
      "duration_secs": 432.911271542,
      "throughput_tps": 647.3766666666667,
      "latency_p50_us": 5000000,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 138487,
      "rejected_transfers": 55726,
      "failed_transfers": 0,
      "dropped_transfers": 2838146,
      "balance_verified": true
    }
  ],
  "aggregate": {
    "throughput": {
      "mean": 652.6188888888888,
      "stddev": 8.240179939303427,
      "cv": 0.012626327676988756,
      "min": 646.2266666666667,
      "max": 664.2533333333333
    },
    "latency_p50": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "latency_p95": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "latency_p99": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "latency_p999": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "total_completed": 417497,
    "total_rejected": 169860,
    "total_failed": 0,
    "total_dropped": 8534545,
    "error_rate": 0.0
  },
  "warnings": [],
  "errors": []
}
```

### PostgreSQL atomic, concurrency5k, pool_size=50

Result file: `results/run_20260731_072822/results.json`

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
      "duration_secs": 429.809767917,
      "throughput_tps": 774.94,
      "latency_p50_us": 5000000,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 164760,
      "rejected_transfers": 67722,
      "failed_transfers": 0,
      "dropped_transfers": 1279277,
      "balance_verified": true
    },
    {
      "run_id": 2,
      "duration_secs": 430.263030458,
      "throughput_tps": 785.55,
      "latency_p50_us": 5000000,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 166090,
      "rejected_transfers": 69575,
      "failed_transfers": 0,
      "dropped_transfers": 1292488,
      "balance_verified": true
    },
    {
      "run_id": 3,
      "duration_secs": 430.433753958,
      "throughput_tps": 790.8066666666666,
      "latency_p50_us": 5000000,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 167818,
      "rejected_transfers": 69424,
      "failed_transfers": 0,
      "dropped_transfers": 1295552,
      "balance_verified": true
    }
  ],
  "aggregate": {
    "throughput": {
      "mean": 783.7655555555556,
      "stddev": 6.59929083357994,
      "cv": 0.00841998067764304,
      "min": 774.94,
      "max": 790.8066666666666
    },
    "latency_p50": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "latency_p95": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "latency_p99": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "latency_p999": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "total_completed": 498668,
    "total_rejected": 206721,
    "total_failed": 0,
    "total_dropped": 3867317,
    "error_rate": 0.0
  },
  "warnings": [],
  "errors": []
}
```

### PostgreSQL atomic, rate10k, pool_size=50

Result file: `results/run_20260731_075326/results.json`

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
      "duration_secs": 430.836118291,
      "throughput_tps": 791.71,
      "latency_p50_us": 5000000,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 169180,
      "rejected_transfers": 68333,
      "failed_transfers": 0,
      "dropped_transfers": 2823005,
      "balance_verified": true
    },
    {
      "run_id": 2,
      "duration_secs": 431.468291375,
      "throughput_tps": 800.9633333333334,
      "latency_p50_us": 5000000,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 169680,
      "rejected_transfers": 70609,
      "failed_transfers": 0,
      "dropped_transfers": 2829450,
      "balance_verified": true
    },
    {
      "run_id": 3,
      "duration_secs": 432.339816541,
      "throughput_tps": 795.08,
      "latency_p50_us": 5000000,
      "latency_p95_us": 5000000,
      "latency_p99_us": 5000000,
      "latency_p999_us": 5000000,
      "completed_transfers": 168605,
      "rejected_transfers": 69919,
      "failed_transfers": 0,
      "dropped_transfers": 2831221,
      "balance_verified": true
    }
  ],
  "aggregate": {
    "throughput": {
      "mean": 795.9177777777778,
      "stddev": 3.823824276658829,
      "cv": 0.004804295598642163,
      "min": 791.71,
      "max": 800.9633333333334
    },
    "latency_p50": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "latency_p95": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "latency_p99": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "latency_p999": { "mean": 5000000.0, "stddev": 0.0, "cv": 0.0, "min": 5000000.0, "max": 5000000.0 },
    "total_completed": 507465,
    "total_rejected": 208861,
    "total_failed": 0,
    "total_dropped": 8483676,
    "error_rate": 0.0
  },
  "warnings": [],
  "errors": []
}
```
