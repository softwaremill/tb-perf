#!/usr/bin/env python3
"""
Plot PostgreSQL Atomic quick test results (after bug fix)
"""

import pandas as pd
import matplotlib.pyplot as plt
import numpy as np

# Read the data
df = pd.read_csv('2026-01-19-postgres-atomic-quick-fixed.csv')

# Find the best configuration
best_idx = df['tps'].idxmax()
best_row = df.loc[best_idx]

print("=== PostgreSQL Atomic Quick Test Results (Bug Fixed) ===")
print(f"\nBest configuration:")
print(f"  Pool size: {best_row['pool_size']}")
print(f"  Concurrency: {best_row['concurrency']}")
print(f"  Throughput: {best_row['tps']:.1f} TPS")
print(f"  Error rate: {best_row['error_rate']}%")

# Top 5 configurations
print(f"\nTop 5 configurations:")
top5 = df.nlargest(5, 'tps')
for idx, (_, row) in enumerate(top5.iterrows(), 1):
    print(f"  {idx}. Pool {int(row['pool_size'])}, Conc {int(row['concurrency'])}: {row['tps']:.1f} TPS")

# Create heatmap
fig, ax = plt.subplots(figsize=(12, 8))

# Pivot data for heatmap
pivot = df.pivot(index='pool_size', columns='concurrency', values='tps')

# Create heatmap
im = ax.imshow(pivot, cmap='YlOrRd', aspect='auto')

# Set ticks and labels
ax.set_xticks(np.arange(len(pivot.columns)))
ax.set_yticks(np.arange(len(pivot.index)))
ax.set_xticklabels(pivot.columns)
ax.set_yticklabels(pivot.index)

# Add colorbar
cbar = plt.colorbar(im, ax=ax)
cbar.set_label('Throughput (TPS)', rotation=270, labelpad=20, fontweight='bold')

# Add text annotations
for i in range(len(pivot.index)):
    for j in range(len(pivot.columns)):
        value = pivot.iloc[i, j]
        if not np.isnan(value):
            text = ax.text(j, i, f'{value:.0f}',
                          ha="center", va="center", color="black", fontsize=9, fontweight='bold')

ax.set_xlabel('Concurrency', fontsize=12, fontweight='bold')
ax.set_ylabel('Connection Pool Size', fontsize=12, fontweight='bold')
ax.set_title('PostgreSQL Atomic Executor Performance Heatmap (Bug Fixed)\nQuick Test Results',
             fontsize=14, fontweight='bold')

plt.tight_layout()
plt.savefig('2026-01-19-postgres-atomic-quick-fixed.png', dpi=300, bbox_inches='tight')
print(f"\nHeatmap saved to 2026-01-19-postgres-atomic-quick-fixed.png")

# Create line plot by pool size
fig, ax = plt.subplots(figsize=(10, 6))

for pool_size in sorted(df['pool_size'].unique()):
    pool_data = df[df['pool_size'] == pool_size].sort_values('concurrency')
    ax.plot(pool_data['concurrency'], pool_data['tps'],
            marker='o', label=f'Pool {int(pool_size)}', linewidth=2)

ax.set_xlabel('Concurrency', fontsize=12, fontweight='bold')
ax.set_ylabel('Throughput (TPS)', fontsize=12, fontweight='bold')
ax.set_title('PostgreSQL Atomic: Throughput vs Concurrency (Bug Fixed)',
             fontsize=14, fontweight='bold')
ax.set_xscale('log', base=2)
ax.grid(True, alpha=0.3)
ax.legend(fontsize=10)

plt.tight_layout()
plt.savefig('2026-01-19-postgres-atomic-concurrency-lines.png', dpi=300, bbox_inches='tight')
print(f"Line chart saved to 2026-01-19-postgres-atomic-concurrency-lines.png")
