# TigerBeetle vs PostgreSQL: performance benchmark in the cloud

In the [previous article](https://softwaremill.com/tigerbeetle-vs-postgresql-performance-benchmark-setup-local-tests/)
we benchmarked TigerBeetle against PostgreSQL on a single machine - a
MacBook Pro, one client process, one database instance. TigerBeetle came
out roughly 2.8x faster than the best PostgreSQL setup we could put
together. That's a good headline number, but it leaves an obvious question
open: single-node, single-machine tests don't say much about how these
databases behave once you deploy them the way a real financial system
would - as a replicated cluster, driven by multiple independent clients
over a network, with a workload that occasionally hammers the same few
accounts. This article picks up exactly where the local tests left off.

## What changed from the local setup

TigerBeetle isn't really meant to run as a single node - production
deployments are a replicated cluster (TigerBeetle recommends six nodes
across three cloud providers; we used three for this test). Likewise, a
production PostgreSQL setup for anything financial would run synchronous
replication rather than a single instance you could lose along with all
its data. So this time around, both databases ran as a genuine 3-node
cluster: TigerBeetle configured for its usual leader + 2 replicas
consensus, PostgreSQL set up with one primary and two synchronous standbys
(quorum commit, so a transaction isn't acknowledged until it's durable on
at least one standby too - the closest apples-to-apples match to
TigerBeetle's own write quorum).

The whole thing ran on GCP: a 3-node database cluster, five separate client
VMs generating load, and one monitoring node running Prometheus and
Grafana. Driving load from multiple independent machines over the network
also let us test something the local benchmark couldn't: `fixed_rate` mode.
Instead of "how fast can N concurrent workers go" (closed-loop,
`max_throughput`), `fixed_rate` issues requests at a constant, pre-decided
rate regardless of how the database responds, with coordinated-omission
correction for the latency numbers. That's a much closer match to how
production traffic actually behaves and to what an SLA usually promises:
"under this decided rate, latency stays under X" - rather than "how fast
can I possibly go if I let it".

## The workload, again

Same double-entry bookkeeping simulation as before: 100,000 accounts,
random transfers debiting one and crediting another, Zipfian-distributed
account selection so some accounts are hit more often than others. We
tested two skew levels this time:

- **Moderate skew** (`zipfian_exponent = 1.0`) - roughly the same
  distribution shape as the local benchmark.
- **Heavy hotspot skew** (`zipfian_exponent = 2.0`) - a small number of
  accounts absorb a disproportionate share of transfers, closer to what a
  real ledger looks like: an exchange's main clearing account, a popular
  merchant's settlement account, a payroll clearing account. Real
  financial traffic is rarely evenly spread, so it seemed worth testing
  what happens when it really isn't.

Both tests ran a shared target rate of 5,000 transfers/second across five
client instances, three runs of five minutes each (with a two-minute
warmup before every run), and verified after every single run that the
total balance across all 100,000 accounts hadn't drifted by even one unit -
the one thing that must never happen in a ledger, replicated or not.

## Results: moderate skew

| | TigerBeetle | PostgreSQL (`FOR UPDATE`) |
|---|---|---|
| Mean throughput | 4,097 TPS | 2,890 TPS |
| p50 latency | 32 ms | 325 ms |
| p95 latency | 508 ms | 581 ms |
| p99 latency | 748 ms | 960 ms |

TigerBeetle came out about 1.4x faster in raw throughput, but the more
striking number is the median latency - ten times lower. PostgreSQL isn't
struggling to keep up here (both stayed comfortably below the target rate's
ceiling with 0% errors), but every transfer still has to acquire a row
lock, wait its turn, write, and wait for the synchronous replica to
acknowledge before it's done. TigerBeetle's single, lock-free sequential
processing pipeline just has less overhead per transfer to begin with.

## Results: heavy hotspot skew

This is where it gets interesting. Cranking the skew up to concentrate
traffic onto a handful of accounts is exactly the scenario where
PostgreSQL's row-locking approach should hurt the most - and TigerBeetle's
lock-free design shouldn't notice much at all, since there's no lock to
contend over in the first place.

| | TigerBeetle | PostgreSQL (`FOR UPDATE`) |
|---|---|---|
| Mean throughput | 4,461 TPS (+9% vs. moderate skew) | 753 TPS (−74% vs. moderate skew) |
| p50 latency | 32 ms (flat) | 1,306 ms (4x worse) |
| p95 latency | 358 ms (improved) | 1,890 ms (3.3x worse) |
| p99 latency | 560 ms (improved) | 2,123 ms (2.2x worse) |

That's exactly what happened, and more dramatically than expected.
TigerBeetle's numbers didn't just hold steady under heavier contention -
they got slightly *better* (more transfers get rejected quickly for
insufficient balance rather than queuing behind a lock, which pulls the
average down). PostgreSQL's throughput collapsed by three-quarters and its
median latency quadrupled. The gap between the two databases, only ~1.4x
under moderate skew, widened to almost 6x under hotspot contention. Both
databases still passed every balance check on every run - this is a real,
repeatable performance characteristic, not data corruption or a flaky test.

## A closer look at PostgreSQL's other executors

The local benchmark already tested three ways of implementing a transfer
in PostgreSQL: explicit locking (`SELECT ... FOR UPDATE`), an atomic
`UPDATE ... WHERE balance >= amount` with no explicit lock, and a batched
mode that groups many transfers into one round trip through a single
connection (closer to TigerBeetle's own batching model). We repeated all
three in the cloud, at both skew levels.

**Atomic** was a modest, consistent improvement over explicit locking in
both regimes - about 6% faster at moderate skew, 30% faster under hotspot
contention, with correspondingly lower latency. Removing the explicit lock
removes one source of contention overhead, but the fundamental cost -
one network round trip and one synchronous-replication wait per transfer -
is still there, so the improvement is incremental rather than
architectural.

**Batched mode was the most instructive result of this whole test.** On a
single machine, one connection processing many transfers per round trip is
a clear win - that's exactly what made it competitive with TigerBeetle
locally. Distributed across a cluster, driven by five independent client
connections, it fell apart: throughput dropped to single digits (yes,
single-digit transfers per second, not thousands), and its error rate
quadrupled under hotspot skew, from 4.5% to nearly 17%.

The reason is a genuine architectural mismatch, not a bug. Each transfer in
a batch orders its two account updates by ID to stop a single transaction
from deadlocking with itself - a well-known PostgreSQL trick. But it does
nothing to stop two *different* connections' transactions from deadlocking
with each other: if connection A is mid-transfer 5→10 (holding a lock on
account 5, waiting on account 10) while connection B is mid-transfer 10→5
(holding account 10, waiting on account 5), that's a classic deadlock, and
PostgreSQL's deadlock detector has to kill one side. Batched transactions
hold their locks far longer than a single-transfer transaction would (many
updates, plus a cross-node replication wait, all inside one transaction),
which massively widens the window for this kind of collision - and under
hotspot skew, where many different connections are all reaching for the
same few accounts, that window gets hit constantly.

TigerBeetle can't have this failure mode at all, and it's worth being
precise about why: not because it's better at resolving contention, but
because it has no independent concurrent writers to begin with. A single
sequential state machine processes every transfer from every client, one
at a time, in order - there's simply nothing to deadlock. PostgreSQL's
row-locking model always carries this tradeoff the moment you want several
connections writing concurrently; avoiding it takes deliberate
application-level coordination (sorting every account touched across an
*entire* batch, not just per-transfer, before applying anything), which is
exactly the kind of complexity a design like TigerBeetle's sidesteps
structurally.

## Caveats, same as last time

This is still one workload, one region, one specific hardware
configuration, and one specific way of implementing "a ledger" in
PostgreSQL. Different account-contention patterns, batch sizes, connection
pool tuning, or PostgreSQL extensions could all move these numbers. As
before: benchmark your own workload before betting a production decision
on someone else's numbers - what we can say with confidence is that the
gap we found locally didn't shrink once we moved to a realistic, replicated,
networked deployment. If anything, it grew - and it grew specifically in
the contention scenario that looks most like a real financial ledger's hot
path.
