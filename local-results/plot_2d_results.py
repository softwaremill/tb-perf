#!/usr/bin/env python3
import csv
import sys
import matplotlib.pyplot as plt
import numpy as np

# Usage: ./plot_2d_results.py <csv_file> [title]
if len(sys.argv) < 2:
    print("Usage: ./plot_2d_results.py <csv_file> [title]")
    sys.exit(1)

csv_file = sys.argv[1]
title = sys.argv[2] if len(sys.argv) > 2 else csv_file.replace('.csv', '')
output_file = csv_file.replace('.csv', '.png')

# Read data
data = []
with open(csv_file, 'r') as f:
    reader = csv.DictReader(f)
    for row in reader:
        data.append({
            'concurrency': int(row['concurrency']),
            'pool_size': int(row['pool_size']),
            'tps': float(row['tps'])
        })

# Find best
best = max(data, key=lambda x: x['tps'])

# Get unique pool sizes
pool_sizes = sorted(set(d['pool_size'] for d in data))

# Create figure
fig, ax = plt.subplots(figsize=(12, 7))

# Color map for pool sizes
colors = plt.cm.viridis(np.linspace(0, 1, len(pool_sizes)))

# Plot each pool size as a separate line
for i, pool_size in enumerate(pool_sizes):
    subset = [(d['concurrency'], d['tps']) for d in data if d['pool_size'] == pool_size]
    subset.sort()
    if subset:
        conc, tps = zip(*subset)
        ax.scatter(conc, tps, c=[colors[i]], s=50, label=f'pool={pool_size}', alpha=0.7)
        # Connect points with lines if there are multiple
        if len(conc) > 1:
            ax.plot(conc, tps, c=colors[i], alpha=0.3, linewidth=1)

# Highlight best point
ax.scatter([best['concurrency']], [best['tps']], c='red', s=200, zorder=10,
           marker='*', edgecolors='black', linewidths=1,
           label=f"Best: c={best['concurrency']}, p={best['pool_size']} ({best['tps']:,.1f} TPS)")

# Formatting
ax.set_xlabel('Concurrency', fontsize=12)
ax.set_ylabel('Throughput (TPS)', fontsize=12)
ax.set_title(f'{title}: Concurrency vs Throughput by Pool Size', fontsize=14)
ax.set_xscale('log')
ax.grid(True, alpha=0.3)
ax.legend(loc='upper right', fontsize=9, ncol=2)

plt.tight_layout()
plt.savefig(output_file, dpi=150)
print(f"Graph saved to {output_file}")
print(f"Best: concurrency={best['concurrency']}, pool_size={best['pool_size']} with {best['tps']:,.1f} TPS")
