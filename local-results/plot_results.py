#!/usr/bin/env python3
import csv
import sys
import matplotlib.pyplot as plt

# Usage: ./plot_results.py <csv_file> <title>
if len(sys.argv) < 2:
    print("Usage: ./plot_results.py <csv_file> [title]")
    sys.exit(1)

csv_file = sys.argv[1]
title = sys.argv[2] if len(sys.argv) > 2 else csv_file.replace('.csv', '')
output_file = csv_file.replace('.csv', '.png')

# Read data
concurrency = []
tps = []

with open(csv_file, 'r') as f:
    reader = csv.DictReader(f)
    for row in reader:
        concurrency.append(int(row['concurrency']))
        tps.append(float(row['tps']))

# Sort by concurrency for proper line plot
data = sorted(zip(concurrency, tps))
concurrency, tps = zip(*data)

# Find best
best_idx = tps.index(max(tps))
best_concurrency = concurrency[best_idx]
best_tps = tps[best_idx]

# Create figure
fig, ax = plt.subplots(figsize=(12, 7))

# Plot
ax.plot(concurrency, tps, 'b-o', markersize=4, linewidth=1.5)
ax.scatter([best_concurrency], [best_tps], color='red', s=150, zorder=5, label=f'Best: {best_concurrency:,} ({best_tps:,.1f} TPS)')

# Formatting
ax.set_xlabel('Concurrency', fontsize=12)
ax.set_ylabel('Throughput (TPS)', fontsize=12)
ax.set_title(f'{title}: Concurrency vs Throughput', fontsize=14)
ax.set_xscale('log')
ax.grid(True, alpha=0.3)
ax.legend(loc='upper left', fontsize=11)

# Add annotations
ax.annotate(f'{best_tps:,.0f} TPS\n@ {best_concurrency:,}',
            xy=(best_concurrency, best_tps),
            xytext=(best_concurrency * 2, best_tps * 0.85),
            fontsize=10,
            arrowprops=dict(arrowstyle='->', color='red'))

plt.tight_layout()
plt.savefig(output_file, dpi=150)
print(f"Graph saved to {output_file}")
print(f"Best concurrency: {best_concurrency:,} with {best_tps:,.1f} TPS")
