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
groups = ["concurrency5k\n(target=5,000)", "rate10k\n(target=10,000)", "rate20k\n(target=20,000)"]
tigerbeetle = [5060, 9431, 20257]
pg_standard = [683, 703, None]  # not tested at rate20k
pg_atomic = [878, 867, None]  # not tested at rate20k

x = np.arange(len(groups))
width = 0.25

fig, ax = plt.subplots(figsize=(9, 5.5))
b1 = ax.bar(x - width, tigerbeetle, width, label="TigerBeetle", color="#e8743b")
b2 = ax.bar(x[:2], pg_standard[:2], width, label="PostgreSQL Standard (FOR UPDATE)", color="#2f6690")
b3 = ax.bar(x[:2] + width, pg_atomic[:2], width, label="PostgreSQL Atomic", color="#5fa8d3")

ax.set_yscale("log")
ax.set_ylabel("Throughput (transfers / second, log scale)")
ax.set_title("TigerBeetle vs. PostgreSQL — hotspot skew, corrected knobs", pad=45)
ax.set_xticks(x)
ax.set_xticklabels(groups)
ax.legend(loc="upper center", bbox_to_anchor=(0.5, 1.18), ncol=1, frameon=False)
ax.set_ylim(1, 5e4)

for bars in (b1, b2, b3):
    for bar in bars:
        h = bar.get_height()
        ax.annotate(f"{int(h):,}", (bar.get_x() + bar.get_width() / 2, h),
                    ha="center", va="bottom", fontsize=9, fontweight="bold")

ax.annotate("not tested\nat rate20k", (x[2], 1.0), ha="center", va="bottom",
            fontsize=8, color="#666666", xytext=(x[2] + width / 2, 3))

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
c5k = [37, 480, 676, 908]
r10k = [37, 611, 837, 989]
r20k = [43, 846, 1046, 1454]

x = np.arange(len(percentiles))
width = 0.25

fig, ax = plt.subplots(figsize=(8, 5))
b1 = ax.bar(x - width, c5k, width, label="concurrency5k (target=5,000)", color="#e8743b")
b2 = ax.bar(x, r10k, width, label="rate10k (target=10,000)", color="#f4a261")
b3 = ax.bar(x + width, r20k, width, label="rate20k (target=20,000)", color="#f9c784")

ax.set_ylabel("Latency (milliseconds)")
ax.set_title("TigerBeetle latency under hotspot skew — corrected knobs")
ax.set_xticks(x)
ax.set_xticklabels(percentiles)
ax.legend()

for bars in (b1, b2, b3):
    for bar in bars:
        h = bar.get_height()
        ax.annotate(f"{int(h)}", (bar.get_x() + bar.get_width() / 2, h),
                    ha="center", va="bottom", fontsize=9, fontweight="bold")

fig.tight_layout()
fig.savefig(f"{OUT_DIR}/latency_tigerbeetle_corrected_hotspot.png", dpi=150)
plt.close(fig)

print("Wrote throughput_corrected_hotspot.png and latency_tigerbeetle_corrected_hotspot.png")
