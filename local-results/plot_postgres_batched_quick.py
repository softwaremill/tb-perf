#!/usr/bin/env python3
import pandas as pd
import matplotlib.pyplot as plt

# Read the CSV data
df = pd.read_csv('2026-01-18-postgres-batched-quick.csv')

# Sort by concurrency for better visualization
df = df.sort_values('concurrency')

# Create the plot
plt.figure(figsize=(12, 7))
plt.plot(df['concurrency'], df['tps'], marker='o', linewidth=2, markersize=8)
plt.xscale('log')
plt.xlabel('Concurrency', fontsize=12)
plt.ylabel('TPS (Transactions Per Second)', fontsize=12)
plt.title('PostgreSQL Batched Performance: Concurrency vs TPS (Quick Tests)', fontsize=14, fontweight='bold')
plt.grid(True, alpha=0.3)

# Mark the optimal point
max_idx = df['tps'].idxmax()
max_concurrency = df.loc[max_idx, 'concurrency']
max_tps = df.loc[max_idx, 'tps']
plt.plot(max_concurrency, max_tps, 'r*', markersize=20, label=f'Optimal: {int(max_concurrency)} concurrency @ {max_tps:.1f} TPS')

plt.legend(fontsize=11)
plt.tight_layout()
plt.savefig('2026-01-18-postgres-batched-quick.png', dpi=300, bbox_inches='tight')
print(f"Graph saved to 2026-01-18-postgres-batched-quick.png")
print(f"\nOptimal configuration:")
print(f"  Concurrency: {int(max_concurrency)}")
print(f"  TPS: {max_tps:.1f}")
