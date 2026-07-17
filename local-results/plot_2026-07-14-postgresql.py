#!/usr/bin/env python3
"""Plot results from PostgreSQL run on 2026-07-14"""
import pandas as pd
import matplotlib.pyplot as plt

# Read the detailed CSV data
df = pd.read_csv('2026-07-14-postgresql-detail.csv')

# Create figure with subplots
fig, axes = plt.subplots(2, 2, figsize=(14, 10))
fig.suptitle('PostgreSQL Performance - Fixed Rate Test (2026-07-14)', fontsize=16, fontweight='bold')

# Plot 1: TPS per run
ax = axes[0, 0]
ax.bar(df['run_number'], df['tps'], color='steelblue', alpha=0.7, edgecolor='black')
ax.set_xlabel('Run Number', fontsize=11)
ax.set_ylabel('TPS (Transactions Per Second)', fontsize=11)
ax.set_title('Throughput per Run', fontweight='bold')
ax.grid(axis='y', alpha=0.3)
for i, v in enumerate(df['tps']):
    ax.text(df['run_number'].iloc[i], v + 20, f'{v:.1f}', ha='center', va='bottom', fontsize=10)

# Plot 2: Latency percentiles
ax = axes[0, 1]
x = df['run_number']
ax.plot(x, df['latency_p50_us'], marker='o', label='p50', linewidth=2, markersize=8)
ax.plot(x, df['latency_p95_us'], marker='s', label='p95', linewidth=2, markersize=8)
ax.plot(x, df['latency_p99_us'], marker='^', label='p99', linewidth=2, markersize=8)
ax.plot(x, df['latency_p999_us'], marker='d', label='p999', linewidth=2, markersize=8)
ax.set_xlabel('Run Number', fontsize=11)
ax.set_ylabel('Latency (microseconds)', fontsize=11)
ax.set_title('Latency Percentiles per Run', fontweight='bold')
ax.legend(fontsize=10)
ax.grid(alpha=0.3)

# Plot 3: Completed vs Rejected transfers
ax = axes[1, 0]
x = df['run_number']
width = 0.35
ax.bar(x - width/2, df['completed_transfers']/1000, width, label='Completed', color='green', alpha=0.7, edgecolor='black')
ax.bar(x + width/2, df['rejected_transfers']/1000, width, label='Rejected', color='red', alpha=0.7, edgecolor='black')
ax.set_xlabel('Run Number', fontsize=11)
ax.set_ylabel('Count (thousands)', fontsize=11)
ax.set_title('Completed vs Rejected Transfers', fontweight='bold')
ax.legend(fontsize=10)
ax.grid(axis='y', alpha=0.3)

# Plot 4: Error metrics summary
ax = axes[1, 1]
ax.axis('off')
summary_text = f"""
Summary Statistics:

Mean TPS: {df['tps'].mean():.2f}
Min TPS: {df['tps'].min():.2f}
Max TPS: {df['tps'].max():.2f}
Std Dev: {df['tps'].std():.2f}

Mean Latency (p50): {df['latency_p50_us'].mean():.0f} µs
Mean Latency (p95): {df['latency_p95_us'].mean():.0f} µs
Mean Latency (p99): {df['latency_p99_us'].mean():.0f} µs
Mean Latency (p999): {df['latency_p999_us'].mean():.0f} µs

Total Completed: {df['completed_transfers'].sum():,}
Total Rejected: {df['rejected_transfers'].sum():,}
Error Rate: {(df['rejected_transfers'].sum() / (df['completed_transfers'].sum() + df['rejected_transfers'].sum()) * 100):.3f}%
"""
ax.text(0.1, 0.5, summary_text, fontsize=11, family='monospace', verticalalignment='center')

plt.tight_layout()
plt.savefig('2026-07-14-postgresql.png', dpi=300, bbox_inches='tight')
print("Graph saved to 2026-07-14-postgresql.png")
print(f"\nPostgreSQL Performance Summary:")
print(f"  Mean TPS: {df['tps'].mean():.2f}")
print(f"  Mean p99 Latency: {df['latency_p99_us'].mean():.0f} µs")
