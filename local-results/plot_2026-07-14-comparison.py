#!/usr/bin/env python3
"""Compare PostgreSQL vs TigerBeetle performance (2026-07-14)"""
import pandas as pd
import matplotlib.pyplot as plt
import numpy as np

# Read the detailed CSV data
pg_df = pd.read_csv('2026-07-14-postgresql-detail.csv')
tb_df = pd.read_csv('2026-07-14-tigerbeetle-detail.csv')

# Create figure with subplots
fig, axes = plt.subplots(2, 2, figsize=(14, 10))
fig.suptitle('PostgreSQL vs TigerBeetle Performance Comparison (2026-07-14)', fontsize=16, fontweight='bold')

# Plot 1: TPS Comparison
ax = axes[0, 0]
x = np.arange(len(pg_df))
width = 0.35
bars1 = ax.bar(x - width/2, pg_df['tps'], width, label='PostgreSQL', color='steelblue', alpha=0.7, edgecolor='black')
bars2 = ax.bar(x + width/2, tb_df['tps'], width, label='TigerBeetle', color='coral', alpha=0.7, edgecolor='black')
ax.set_xlabel('Run Number', fontsize=11)
ax.set_ylabel('TPS (Transactions Per Second)', fontsize=11)
ax.set_title('Throughput Comparison', fontweight='bold')
ax.set_xticks(x)
ax.set_xticklabels(pg_df['run_number'])
ax.legend(fontsize=10)
ax.grid(axis='y', alpha=0.3)

# Add value labels on bars
for bars in [bars1, bars2]:
    for bar in bars:
        height = bar.get_height()
        ax.text(bar.get_x() + bar.get_width()/2., height,
                f'{height:.0f}', ha='center', va='bottom', fontsize=9)

# Plot 2: Latency p99 Comparison
ax = axes[0, 1]
x = np.arange(len(pg_df))
bars1 = ax.bar(x - width/2, pg_df['latency_p99_us']/1000, width, label='PostgreSQL', color='steelblue', alpha=0.7, edgecolor='black')
bars2 = ax.bar(x + width/2, tb_df['latency_p99_us']/1000, width, label='TigerBeetle', color='coral', alpha=0.7, edgecolor='black')
ax.set_xlabel('Run Number', fontsize=11)
ax.set_ylabel('Latency p99 (milliseconds)', fontsize=11)
ax.set_title('p99 Latency Comparison', fontweight='bold')
ax.set_xticks(x)
ax.set_xticklabels(pg_df['run_number'])
ax.legend(fontsize=10)
ax.grid(axis='y', alpha=0.3)

# Add value labels on bars
for bars in [bars1, bars2]:
    for bar in bars:
        height = bar.get_height()
        ax.text(bar.get_x() + bar.get_width()/2., height,
                f'{height:.1f}ms', ha='center', va='bottom', fontsize=9)

# Plot 3: All latency percentiles comparison (aggregated)
ax = axes[1, 0]
x = np.arange(4)
width = 0.35
pg_latencies = [pg_df['latency_p50_us'].mean()/1000, pg_df['latency_p95_us'].mean()/1000, 
                pg_df['latency_p99_us'].mean()/1000, pg_df['latency_p999_us'].mean()/1000]
tb_latencies = [tb_df['latency_p50_us'].mean()/1000, tb_df['latency_p95_us'].mean()/1000,
                tb_df['latency_p99_us'].mean()/1000, tb_df['latency_p999_us'].mean()/1000]
bars1 = ax.bar(x - width/2, pg_latencies, width, label='PostgreSQL', color='steelblue', alpha=0.7, edgecolor='black')
bars2 = ax.bar(x + width/2, tb_latencies, width, label='TigerBeetle', color='coral', alpha=0.7, edgecolor='black')
ax.set_ylabel('Latency (milliseconds)', fontsize=11)
ax.set_title('Mean Latency Percentiles', fontweight='bold')
ax.set_xticks(x)
ax.set_xticklabels(['p50', 'p95', 'p99', 'p999'])
ax.legend(fontsize=10)
ax.grid(axis='y', alpha=0.3)

# Plot 4: Summary statistics table
ax = axes[1, 1]
ax.axis('off')

summary_text = f"""
PostgreSQL Performance:
  Mean TPS: {pg_df['tps'].mean():.2f}
  Mean p50 Latency: {pg_df['latency_p50_us'].mean():.0f} µs
  Mean p99 Latency: {pg_df['latency_p99_us'].mean():.0f} µs
  Total Completed: {pg_df['completed_transfers'].sum():,}
  Total Rejected: {pg_df['rejected_transfers'].sum():,}

TigerBeetle Performance:
  Mean TPS: {tb_df['tps'].mean():.2f}
  Mean p50 Latency: {tb_df['latency_p50_us'].mean():.0f} µs
  Mean p99 Latency: {tb_df['latency_p99_us'].mean():.0f} µs
  Total Completed: {tb_df['completed_transfers'].sum():,}
  Total Rejected: {tb_df['rejected_transfers'].sum():,}

Performance Ratio:
  TPS Improvement: {(tb_df['tps'].mean() / pg_df['tps'].mean() - 1) * 100:.1f}%
  p99 Latency Ratio: {tb_df['latency_p99_us'].mean() / pg_df['latency_p99_us'].mean():.2f}x
"""

ax.text(0.05, 0.5, summary_text, fontsize=10, family='monospace', verticalalignment='center')

plt.tight_layout()
plt.savefig('2026-07-14-comparison.png', dpi=300, bbox_inches='tight')
print("Graph saved to 2026-07-14-comparison.png")
print(f"\nPerformance Comparison:")
print(f"  PostgreSQL Mean TPS: {pg_df['tps'].mean():.2f}")
print(f"  TigerBeetle Mean TPS: {tb_df['tps'].mean():.2f}")
print(f"  TigerBeetle is {(tb_df['tps'].mean() / pg_df['tps'].mean()):.1f}x faster")
print(f"\n  PostgreSQL Mean p99 Latency: {pg_df['latency_p99_us'].mean():.0f} µs")
print(f"  TigerBeetle Mean p99 Latency: {tb_df['latency_p99_us'].mean():.0f} µs")
