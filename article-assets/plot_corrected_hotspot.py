#!/usr/bin/env python3
"""Regenerate the hotspot-skew charts for article_v3 using the corrected
max_concurrency/target_rate knob-sweep results (see
hotspot-knob-sweep-results.md). Run from repo root:
    python3 article-assets/plot_corrected_hotspot.py
"""
import matplotlib.pyplot as plt
import numpy as np

OUT_DIR = "article-assets"

# --- Chart 1: throughput, corrected hotspot numbers -------------------------
groups = ["concurrency5k\n(target=5,000)", "rate10k\n(target=10,000)",
          "rate20k\n(target=20,000)", "rate40k\n(target=40,000)",
          "rate80k\n(target=80,000)", "rate160k\n(target=160,000)"]
tigerbeetle = [5060, 9431, 20257, 40388, 81171, 107858]
pg_standard = [683, 703]  # not tested past rate10k
pg_atomic = [878, 867]  # not tested past rate10k

x = np.arange(len(groups))
width = 0.25

fig, ax = plt.subplots(figsize=(11, 5.5))
b1 = ax.bar(x - width, tigerbeetle, width, label="TigerBeetle", color="#e8743b")
b2 = ax.bar(x[:2], pg_standard, width, label="PostgreSQL Standard (FOR UPDATE)", color="#2f6690")
b3 = ax.bar(x[:2] + width, pg_atomic, width, label="PostgreSQL Atomic", color="#5fa8d3")

# rate160k's bar is real but fell short of its offered rate - hatch it
# differently (not "capped measurement", but "genuine shortfall") to make
# that visually distinct from a clean result.
b1[5].set_hatch("...")
b1[5].set_edgecolor("#7a3d1a")

ax.set_yscale("log")
ax.set_ylabel("Throughput (transfers / second, log scale)")
ax.set_title("TigerBeetle vs. PostgreSQL — hotspot skew, corrected knobs", pad=45)
ax.set_xticks(x)
ax.set_xticklabels(groups)
ax.legend(loc="upper center", bbox_to_anchor=(0.5, 1.18), ncol=1, frameon=False)
ax.set_ylim(1, 6e5)

for bars in (b1, b2, b3):
    for bar in bars:
        h = bar.get_height()
        ax.annotate(f"{int(h):,}", (bar.get_x() + bar.get_width() / 2, h),
                    ha="center", va="bottom", fontsize=9, fontweight="bold")

for gx in (x[2], x[3], x[4], x[5]):
    ax.annotate("PostgreSQL\nnot tested", (gx, 1.0), ha="center", va="bottom",
                fontsize=8, color="#666666", xytext=(gx, 3))

ax.annotate("only 67% of\noffered rate", (x[5] - width, 107858), ha="center", va="bottom",
            fontsize=7.5, color="#7a3d1a", xytext=(x[5] - width, 160000))

fig.tight_layout()
fig.savefig(f"{OUT_DIR}/throughput_corrected_hotspot.png", dpi=150)
plt.close(fig)

# --- Chart 2: TigerBeetle latency percentiles, all six knob variants -------
# Log-scale y-axis this time: rate160k's real latency (seconds) is 2-3
# orders of magnitude above concurrency5k's (tens of ms), so a linear axis
# would flatten everything below rate80k to invisibility.
#
# CAPPED (hatched) = value sits within ~50ms of the client's old 1,500,000us
# histogram bucket boundary (client/src/metrics.rs) - confirmed a
# measurement artifact by rate160k (run with widened buckets up to 20s,
# which showed real latency growing freely past 1.5s). Treat hatched bars
# as lower bounds, not precise values. rate160k's own bars are NOT capped -
# they're real values from the widened-bucket run, just averaged across
# only 2 of 3 runs (see hotspot-knob-sweep-results.md for why).
percentiles = ["p50", "p95", "p99", "p999"]
series = [
    ("concurrency5k (target=5,000)", [37, 480, 676, 908], [False, False, False, False], "#e8743b"),
    ("rate10k (target=10,000)", [37, 611, 837, 989], [False, False, False, False], "#f4a261"),
    ("rate20k (target=20,000)", [43, 846, 1046, 1454], [False, False, False, False], "#f9c784"),
    ("rate40k (target=40,000)", [73, 948, 1312, 1481], [False, False, False, True], "#c9a876"),
    ("rate80k (target=80,000)", [664, 1288, 1458, 1496], [False, False, True, True], "#8c6d46"),
    ("rate160k (target=160,000, 2-run avg)", [3611, 5469, 7294, 9618], [False, False, False, False], "#5a2d0f"),
]

x = np.arange(len(percentiles)) * 1.6
width = 0.2
offsets = np.linspace(-2.5 * width, 2.5 * width, len(series))

fig, ax = plt.subplots(figsize=(12, 6.5))
for (label, values, capped, color), offset in zip(series, offsets):
    bars = ax.bar(x + offset, values, width, label=label, color=color)
    for bar, h, is_capped in zip(bars, values, capped):
        if is_capped:
            bar.set_hatch("////")
            bar.set_edgecolor("#333333")
        ax.annotate(f"{h}{'*' if is_capped else ''}", (bar.get_x() + bar.get_width() / 2, h),
                    ha="center", va="bottom", fontsize=7, fontweight="bold", rotation=90)

ax.set_yscale("log")
ax.set_ylabel("Latency (milliseconds, log scale)")
ax.set_title("TigerBeetle latency under hotspot skew — corrected knobs", pad=15)
ax.set_xticks(x)
ax.set_xticklabels(percentiles)
ax.set_ylim(10, 30000)
ax.legend(fontsize=8, loc="upper left")
fig.text(0.5, 0.01, "* hatched bars sat against the old 1.5s histogram bucket boundary - confirmed a measurement artifact by rate160k's widened-bucket run",
          ha="center", va="bottom", fontsize=8, color="#555555")

fig.tight_layout(rect=(0, 0.04, 1, 1))
fig.savefig(f"{OUT_DIR}/latency_tigerbeetle_corrected_hotspot.png", dpi=150)
plt.close(fig)

print("Wrote throughput_corrected_hotspot.png and latency_tigerbeetle_corrected_hotspot.png")
