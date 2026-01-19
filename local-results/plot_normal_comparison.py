#!/usr/bin/env python3
"""
Plot bar chart comparing normal test results for winning configurations
"""

import pandas as pd
import matplotlib.pyplot as plt
import numpy as np

# Read the data
df = pd.read_csv('2026-01-19-normal-comparison.csv')

# Filter out failed tests (error_rate > 0)
df_success = df[df['error_rate'] == 0].copy()

# Create labels with configuration details
df_success['label'] = df_success.apply(
    lambda row: f"{row['executor']}\n(conc: {row['concurrency']})",
    axis=1
)

# Create the bar chart
fig, ax = plt.subplots(figsize=(10, 6))

x = np.arange(len(df_success))
bars = ax.bar(x, df_success['tps'], color=['#2ecc71', '#3498db', '#e74c3c'])

# Add value labels on top of bars
for i, (idx, row) in enumerate(df_success.iterrows()):
    ax.text(i, row['tps'] + 1000, f"{row['tps']:.1f}",
            ha='center', va='bottom', fontweight='bold')

# Customize the chart
ax.set_xlabel('Executor Configuration', fontsize=12, fontweight='bold')
ax.set_ylabel('Throughput (TPS)', fontsize=12, fontweight='bold')
ax.set_title('Database Performance Comparison - Full Test Results\n(300s measurement, 3 iterations)',
             fontsize=14, fontweight='bold')
ax.set_xticks(x)
ax.set_xticklabels(df_success['label'])
ax.grid(axis='y', alpha=0.3)

# Add a note about the failed test
if len(df[df['error_rate'] > 0]) > 0:
    fig.text(0.5, 0.02,
             'Note: PostgreSQL Atomic (pool: 16, conc: 4) failed with 100% error rate due to transaction abort bug',
             ha='center', fontsize=9, style='italic', color='red')

plt.tight_layout()
plt.savefig('2026-01-19-normal-comparison.png', dpi=300, bbox_inches='tight')
print(f"Chart saved to 2026-01-19-normal-comparison.png")

# Print summary statistics
print("\n=== Summary Statistics ===")
print(f"Winner: {df_success.iloc[0]['executor']} with {df_success.iloc[0]['tps']:.1f} TPS")
print(f"TigerBeetle is {df_success.iloc[0]['tps'] / df_success.iloc[1]['tps']:.2f}x faster than PostgreSQL Batched")
print(f"TigerBeetle is {df_success.iloc[0]['tps'] / df_success.iloc[2]['tps']:.2f}x faster than PostgreSQL Standard")
print(f"PostgreSQL Batched is {df_success.iloc[1]['tps'] / df_success.iloc[2]['tps']:.2f}x faster than PostgreSQL Standard")
