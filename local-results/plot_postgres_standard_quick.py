#!/usr/bin/env python3
import pandas as pd
import matplotlib.pyplot as plt
import numpy as np

# Read the CSV data
df = pd.read_csv('2026-01-19-postgres-standard-quick.csv')

# All configs have 0 error rate
df_clean = df[df['error_rate'] == 0]

# Create figure with subplots
fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(16, 6))

# Plot 1: 2D scatter plot (pool_size vs concurrency, size = TPS)
scatter = ax1.scatter(df['concurrency'], df['pool_size'],
                      s=df['tps']/5, alpha=0.6, edgecolors='black', c='steelblue')
ax1.set_xlabel('Concurrency', fontsize=12)
ax1.set_ylabel('Pool Size', fontsize=12)
ax1.set_title('PostgreSQL Standard: All Test Results\n(bubble size = TPS)', fontsize=14, fontweight='bold')
ax1.set_xscale('log')
ax1.set_yscale('log')
ax1.grid(True, alpha=0.3)

# Mark optimal point on first plot
max_idx = df_clean['tps'].idxmax()
max_pool = df_clean.loc[max_idx, 'pool_size']
max_concurrency = df_clean.loc[max_idx, 'concurrency']
max_tps = df_clean.loc[max_idx, 'tps']
ax1.scatter([max_concurrency], [max_pool], marker='*', s=500,
           color='red', edgecolors='black', linewidths=2, zorder=5,
           label=f'Optimal: pool={int(max_pool)}, conc={int(max_concurrency)}\n@ {max_tps:.1f} TPS')
ax1.legend(fontsize=10)

# Plot 2: TPS by concurrency for different pool sizes
for pool in sorted(df_clean['pool_size'].unique()):
    df_pool = df_clean[df_clean['pool_size'] == pool].sort_values('concurrency')
    if len(df_pool) >= 2:  # Only plot if we have at least 2 points
        ax2.plot(df_pool['concurrency'], df_pool['tps'],
                marker='o', linewidth=2, markersize=8, label=f'Pool size {int(pool)}')

ax2.set_xlabel('Concurrency', fontsize=12)
ax2.set_ylabel('TPS (Transactions Per Second)', fontsize=12)
ax2.set_title('PostgreSQL Standard: TPS vs Concurrency by Pool Size', fontsize=14, fontweight='bold')
ax2.set_xscale('log')
ax2.grid(True, alpha=0.3)
ax2.legend(fontsize=10)

# Mark optimal point on second plot
ax2.scatter([max_concurrency], [max_tps], marker='*', s=500,
           color='red', edgecolors='black', linewidths=2, zorder=5)

plt.tight_layout()
plt.savefig('2026-01-19-postgres-standard-quick.png', dpi=300, bbox_inches='tight')
print(f"Graph saved to 2026-01-19-postgres-standard-quick.png")
print(f"\nOptimal configuration:")
print(f"  Pool size: {int(max_pool)}")
print(f"  Concurrency: {int(max_concurrency)}")
print(f"  TPS: {max_tps:.1f}")
print(f"\nTotal configurations tested: {len(df)}")
print(f"All configurations had 0% error rate")
