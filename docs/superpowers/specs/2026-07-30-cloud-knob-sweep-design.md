# Cloud knob sweep: max_concurrency & target_rate (hotspot skew, first batch)

## Context

Prior cloud results (`tigerbeetle-vs-postgresql.md`, `remaining-postgres-tests.md`) ran
TigerBeetle and PostgreSQL (standard/atomic executors) at `target_rate=5000`,
`max_concurrency=1000`, across 5 client nodes / 3 DB nodes, at two Zipfian skew
levels (1.0 "moderate", 2.0 "hotspot"). TigerBeetle achieved only ~4,100-4,461
TPS against the 5,000 target despite large latency headroom (p50 ~32ms).

Investigation into the client/coordinator code found why this is plausible:
`max_concurrency` and `target_rate` are both divided evenly across client
nodes (`client/src/main.rs:55-59`). At baseline, that's 200 in-flight slots
per client. By Little's Law, per-client in-flight load at the observed p99
latency (~748ms at 1000 req/s/client) would be ~748 - well above the 200 cap.
Requests hitting the cap are dropped (`client/src/workload.rs:372-375`,
`metrics.record_dropped`), but **the coordinator never queries or exports
this metric** (`coordinator/src/prometheus.rs`, `coordinator/src/results.rs`)
- so existing results can't confirm whether the concurrency cap, the client,
or the database was the binding constraint.

The user wants to test two knobs to find out: raising `max_concurrency` (does
the cap explain the gap?) and raising `target_rate` (where's the real
ceiling once the cap isn't confounding?). Client instance count (5) is
explicitly out of scope for now.

## Goal

Run a first batch of cloud tests, restricted to hotspot skew
(`zipfian_exponent = 2.0`, since this is where PostgreSQL is furthest from
its target rate already and TigerBeetle's advantage is most pronounced),
across TigerBeetle + PostgreSQL standard + PostgreSQL atomic (batched
excluded - already proven structurally incompatible with `fixed_rate` mode),
at two new knob variants. Moderate skew (1.0) and any further knob values are
explicitly deferred to a later batch based on what these results show.

## Part 1: Close the dropped-requests observability gap

Small, additive change so results can distinguish "concurrency cap was the
constraint" from "something else was":

- `coordinator/src/prometheus.rs`: add a `dropped_transfers` field to
  `CollectedMetrics`, queried via `tbperf_requests_dropped_total` (same
  `query_counter` pattern as completed/rejected/failed).
- `coordinator/src/results.rs`: add `dropped_transfers` to `RunResult`,
  include it in the per-run and aggregate JSON export (mirroring how
  `rejected_transfers`/`failed_transfers` are already surfaced).
- No changes to client code - the metric already exists and is exported.

## Part 2: New config files (6 total)

Knob variants (both keep everything else identical to the existing
`config.cloud-*-hotspot.toml` / `config.cloud-*-fixedrate.toml` files -
100k accounts, `zipfian_exponent = 2.0`, 5 client / 3 DB nodes, 3 runs of
2min warmup + 5min measurement):

| Variant | target_rate | max_concurrency | Question it answers |
|---|---|---|---|
| `concurrency5k` | 5,000 (unchanged) | 5,000 (was 1,000) | Was the concurrency cap binding at baseline? |
| `rate10k` | 10,000 (was 5,000) | 5,000 | Where's the real ceiling once the cap isn't confounding? |

Files to create (mirroring existing naming conventions):

- `config.cloud-tigerbeetle-hotspot-concurrency5k.toml`
- `config.cloud-tigerbeetle-hotspot-rate10k.toml`
- `config.cloud-postgresql-hotspot-concurrency5k.toml` (executor_mode = standard)
- `config.cloud-postgresql-hotspot-rate10k.toml` (executor_mode = standard)
- `config.cloud-postgresql-atomic-hotspot-concurrency5k.toml`
- `config.cloud-postgresql-atomic-hotspot-rate10k.toml`

## Part 3: Execution

Each config: 3 runs x ~7 min + overhead ~= 25 min. 6 configs ~= ~2.5 hours of
cloud runtime. Before running, verify GCP infra (`terraform/database-cluster`,
`terraform/client-cluster`) is currently provisioned and matching the
expected topology (3 DB nodes, 5 client nodes) rather than assuming - cloud
resources bill continuously whether or not a test is running.

## Expected read-out

- `concurrency5k` vs baseline hotspot results: if baseline
  `dropped_transfers` was non-trivial and throughput/drops improve
  meaningfully at 5k concurrency, the cap was a real constraint. If
  throughput barely moves, something else (client CPU, DB) was already the
  limit.
- `rate10k` vs `concurrency5k`: shows the true ceiling at 2x offered load
  with the concurrency cap held constant across the comparison. Expect
  TigerBeetle to climb further with modest latency growth; PostgreSQL
  (both modes) to show steeper latency growth and/or rising
  drops/errors, especially since existing hotspot throughput (753-977 TPS)
  is already well under even the 5,000 target.

## Out of scope (this batch)

- Moderate skew (`zipfian_exponent = 1.0`) knob variants - defer until
  hotspot results are in.
- Batched executor mode - already proven incompatible with `fixed_rate`.
- Changing client instance count (5) - explicitly deferred by the user.
- Any further knob values beyond the two variants above.
