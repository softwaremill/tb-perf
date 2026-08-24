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
          "rate80k\n(target=80,000)", "rate90k\n(target=90,000)",
          "rate100k\n(target=100,000)", "rate120k\n(target=120,000)",
          "rate160k\n(target=160,000)"]
tigerbeetle = [5060, 9431, 20257, 40388, 81171, 91165, 101497, 107112, 107858]
pg_standard = [683, 703]  # not tested past rate10k
pg_atomic = [878, 867]  # not tested past rate10k

x = np.arange(len(groups))
width = 0.25

fig, ax = plt.subplots(figsize=(13, 5.5))
b1 = ax.bar(x - width, tigerbeetle, width, label="TigerBeetle", color="#e8743b")
b2 = ax.bar(x[:2], pg_standard, width, label="PostgreSQL Standard (FOR UPDATE)", color="#2f6690")
b3 = ax.bar(x[:2] + width, pg_atomic, width, label="PostgreSQL Atomic", color="#5fa8d3")

# rate120k and rate160k's bars are real but fell short of their offered
# rate - hatch them differently (not "capped measurement", but "genuine
# shortfall") to make that visually distinct from a clean result.
b1[7].set_hatch("...")
b1[7].set_edgecolor("#7a3d1a")
b1[8].set_hatch("...")
b1[8].set_edgecolor("#7a3d1a")

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

for gx in (x[2], x[3], x[4], x[5], x[6], x[7], x[8]):
    ax.annotate("PostgreSQL\nnot tested", (gx, 1.0), ha="center", va="bottom",
                fontsize=8, color="#666666", xytext=(gx, 3))

ax.annotate("only 89% of\noffered rate", (x[7] - width, 107112), ha="center", va="bottom",
            fontsize=7.5, color="#7a3d1a", xytext=(x[7] - width, 160000))
ax.annotate("only 67% of\noffered rate", (x[8] - width, 107858), ha="center", va="bottom",
            fontsize=7.5, color="#7a3d1a", xytext=(x[8] - width, 160000))

fig.tight_layout()
fig.savefig(f"{OUT_DIR}/throughput_corrected_hotspot.png", dpi=150)
plt.close(fig)

# --- Chart 2: TigerBeetle latency percentiles, all nine knob variants -----
# Log-scale y-axis this time: rate160k's real latency (seconds) is 2-3
# orders of magnitude above concurrency5k's (tens of ms), so a linear axis
# would flatten everything below rate80k to invisibility.
#
# QUANTISED (hatched) = value sits within ~50ms *below* one of the client's
# histogram bucket boundaries (client/src/metrics.rs): 1,500,000us for
# rate40k/rate80k/rate90k, and 4,000,000us for rate120k (the boundary
# that mattered shifted once offered load pushed latency past the first
# one).
#
# IMPORTANT - these are UPPER bounds, not lower bounds. Prometheus
# histogram_quantile interpolates *inside* a finite bucket, so a value just
# below 1.5s means the quantile fell in the (1.0s, 1.5s] bucket - and the
# 2s/3s/5s buckets above it already existed and stayed empty, so the real
# tail cannot be above 1.5s. Contrast PostgreSQL, whose percentiles come
# back as exactly 5,000,000us, which is what histogram_quantile returns for
# the +Inf overflow bucket - those are genuine LOWER bounds ("at least 5s").
# The rate160k run (widened ladder up to 20s) confirmed the distinction:
# latency grew freely well past both boundaries once the resolution was
# there. So treat hatched bars as "no more than this", not precise values.
#
# rate100k's p999 is deliberately left un-hatched despite sitting close to
# the 1.5s boundary too: unlike every hatched bar, its 3 runs disagree by
# ~230ms (run-to-run CV 5.8%, an order of magnitude higher than the
# near-zero variance that marks quantisation) - two of its three runs landed
# in the newly-added 1.75s bucket, so the real tail is already escaping the
# boundary here, just not cleanly enough to report as a single precise
# number.
percentiles = ["p50", "p95", "p99", "p999"]
series = [
    ("concurrency5k (target=5,000)", [37, 480, 676, 908], [False, False, False, False], "#e8743b"),
    ("rate10k (target=10,000)", [37, 611, 837, 989], [False, False, False, False], "#f4a261"),
    ("rate20k (target=20,000)", [43, 846, 1046, 1454], [False, False, False, False], "#f9c784"),
    ("rate40k (target=40,000)", [73, 948, 1312, 1481], [False, False, False, True], "#c9a876"),
    ("rate80k (target=80,000)", [664, 1288, 1458, 1496], [False, False, True, True], "#8c6d46"),
    ("rate90k (target=90,000)", [705, 1316, 1464, 1497], [False, False, True, True], "#7a5c3a"),
    ("rate100k (target=100,000)", [797, 1387, 1483, 1622], [False, False, False, False], "#6b4a2e"),
    ("rate120k (target=120,000)", [2500, 3204, 3841, 3984], [False, False, False, True], "#4a3220"),
    ("rate160k (target=160,000, 2-run avg)", [3611, 5469, 7294, 9618], [False, False, False, False], "#5a2d0f"),
]

x = np.arange(len(percentiles)) * 2.2
width = 0.16
offsets = np.linspace(-4 * width, 4 * width, len(series))

fig, ax = plt.subplots(figsize=(16, 7))
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
#fig.text(0.5, 0.01, "* hatched bars landed just below a histogram bucket boundary (1.5s for rate40k/80k/90k, 4s for rate120k) - read them as upper bounds (the true value is inside the bucket below), not precise readings",
#          ha="center", va="bottom", fontsize=8, color="#555555")

fig.tight_layout(rect=(0, 0.04, 1, 1))
fig.savefig(f"{OUT_DIR}/latency_tigerbeetle_corrected_hotspot.png", dpi=150)
plt.close(fig)

print("Wrote throughput_corrected_hotspot.png and latency_tigerbeetle_corrected_hotspot.png")
