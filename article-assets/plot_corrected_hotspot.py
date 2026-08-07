#!/usr/bin/env python3
"""Regenerate the hotspot-skew charts for article_v2 using the corrected
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
          "rate80k\n(target=80,000)"]
tigerbeetle = [5060, 9431, 20257, 40388, 81171]
pg_standard = [683, 703]  # not tested past rate10k
pg_atomic = [878, 867]  # not tested past rate10k

x = np.arange(len(groups))
width = 0.25

fig, ax = plt.subplots(figsize=(10, 5.5))
b1 = ax.bar(x - width, tigerbeetle, width, label="TigerBeetle", color="#e8743b")
b2 = ax.bar(x[:2], pg_standard, width, label="PostgreSQL Standard (FOR UPDATE)", color="#2f6690")
b3 = ax.bar(x[:2] + width, pg_atomic, width, label="PostgreSQL Atomic", color="#5fa8d3")

ax.set_yscale("log")
ax.set_ylabel("Throughput (transfers / second, log scale)")
ax.set_title("TigerBeetle vs. PostgreSQL — hotspot skew, corrected knobs", pad=45)
ax.set_xticks(x)
ax.set_xticklabels(groups)
ax.legend(loc="upper center", bbox_to_anchor=(0.5, 1.18), ncol=1, frameon=False)
ax.set_ylim(1, 2e5)

for bars in (b1, b2, b3):
    for bar in bars:
        h = bar.get_height()
        ax.annotate(f"{int(h):,}", (bar.get_x() + bar.get_width() / 2, h),
                    ha="center", va="bottom", fontsize=9, fontweight="bold")

for gx in (x[2], x[3], x[4]):
    ax.annotate("PostgreSQL\nnot tested", (gx, 1.0), ha="center", va="bottom",
                fontsize=8, color="#666666", xytext=(gx, 3))

fig.tight_layout()
fig.savefig(f"{OUT_DIR}/throughput_corrected_hotspot.png", dpi=150)
plt.close(fig)

# --- Chart 2: TigerBeetle latency percentiles, concurrency5k vs rate10k -----
# PostgreSQL's percentiles are omitted here deliberately: every PostgreSQL
# percentile at these knob values was pegged at the OTel histogram's fixed
# 5,000,000us export ceiling (client/src/metrics.rs), so the true value is
# "at least 5s" but not known precisely from this data - charting it as an
# exact bar would overstate our own measurement precision.
percentiles = ["p50", "p95", "p99", "p999"]
# CAPPED = value sits within ~50ms of the client's 1,500,000us histogram
# bucket boundary (client/src/metrics.rs) - treat as a lower bound, not a
# precise measurement. Marked with a hatch pattern below.
series = [
    ("concurrency5k (target=5,000)", [37, 480, 676, 908], [False, False, False, False], "#e8743b"),
    ("rate10k (target=10,000)", [37, 611, 837, 989], [False, False, False, False], "#f4a261"),
    ("rate20k (target=20,000)", [43, 846, 1046, 1454], [False, False, False, False], "#f9c784"),
    ("rate40k (target=40,000)", [73, 948, 1312, 1481], [False, False, False, True], "#c9a876"),
    ("rate80k (target=80,000)", [664, 1288, 1458, 1496], [False, False, True, True], "#8c6d46"),
]

x = np.arange(len(percentiles)) * 1.4
width = 0.16
offsets = np.linspace(-2 * width, 2 * width, len(series))

fig, ax = plt.subplots(figsize=(11, 6))
for (label, values, capped, color), offset in zip(series, offsets):
    bars = ax.bar(x + offset, values, width, label=label, color=color)
    for bar, h, is_capped in zip(bars, values, capped):
        if is_capped:
            bar.set_hatch("////")
            bar.set_edgecolor("#333333")
        ax.annotate(f"{h}{'*' if is_capped else ''}", (bar.get_x() + bar.get_width() / 2, h),
                    ha="center", va="bottom", fontsize=7.5, fontweight="bold", rotation=90 if is_capped else 0)

ax.set_ylabel("Latency (milliseconds)")
ax.set_title("TigerBeetle latency under hotspot skew — corrected knobs", pad=15)
ax.set_xticks(x)
ax.set_xticklabels(percentiles)
ax.set_ylim(0, 1750)
ax.legend(fontsize=8, loc="upper left")
fig.text(0.5, 0.01, "* hatched/starred bars sit against a histogram bucket boundary (~1.5s) - lower bound only, not a precise value",
          ha="center", va="bottom", fontsize=8, color="#555555")

fig.tight_layout(rect=(0, 0.04, 1, 1))
fig.savefig(f"{OUT_DIR}/latency_tigerbeetle_corrected_hotspot.png", dpi=150)
plt.close(fig)

print("Wrote throughput_corrected_hotspot.png and latency_tigerbeetle_corrected_hotspot.png")
