#!/usr/bin/env python3
"""
Convert results.json to CSV format.

Usage:
    python json_to_csv.py <input_json> <output_csv> [--format FORMAT]

Formats:
    - simple: minimal columns (concurrency, tps, results_directory)
    - detailed: includes latency percentiles and error metrics
    - summary: aggregated statistics only
"""

import json
import csv
import sys
import os
import argparse
from pathlib import Path
from typing import List, Dict, Any, Optional


def parse_args():
    parser = argparse.ArgumentParser(
        description='Convert results.json to CSV format',
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog='''
Examples:
  python json_to_csv.py results.json output.csv
  python json_to_csv.py results.json output.csv --format detailed
  python json_to_csv.py results.json output.csv --format summary
        '''
    )
    parser.add_argument('input_json', help='Input JSON file path')
    parser.add_argument('output_csv', help='Output CSV file path')
    parser.add_argument(
        '--format',
        choices=['simple', 'detailed', 'summary'],
        default='simple',
        help='Output format (default: simple)'
    )
    return parser.parse_args()


def load_json(filepath: str) -> Dict[str, Any]:
    """Load and parse JSON file."""
    with open(filepath, 'r') as f:
        return json.load(f)


def extract_results_directory(json_path: str) -> str:
    """Extract the results directory path from JSON file path."""
    return str(Path(json_path).parent)


def create_simple_csv(data: Dict[str, Any], json_path: str) -> List[Dict[str, Any]]:
    """
    Create simple CSV format with core metrics.
    Columns: run_number, tps, results_directory
    """
    results = []
    results_dir = extract_results_directory(json_path)
    
    if 'runs' in data:
        for run in data['runs']:
            results.append({
                'run_number': run.get('run_id', run.get('run_number', '')),
                'tps': round(run.get('throughput_tps', 0), 2),
                'results_directory': results_dir
            })
    
    return results


def create_detailed_csv(data: Dict[str, Any], json_path: str) -> List[Dict[str, Any]]:
    """
    Create detailed CSV format with latency percentiles and error metrics.
    """
    results = []
    results_dir = extract_results_directory(json_path)
    
    if 'runs' in data:
        for run in data['runs']:
            results.append({
                'run_number': run.get('run_id', run.get('run_number', '')),
                'tps': round(run.get('throughput_tps', 0), 2),
                'latency_p50_us': run.get('latency_p50_us', ''),
                'latency_p95_us': run.get('latency_p95_us', ''),
                'latency_p99_us': run.get('latency_p99_us', ''),
                'latency_p999_us': run.get('latency_p999_us', ''),
                'error_rate': run.get('error_rate', 0),
                'completed_transfers': run.get('completed_transfers', ''),
                'rejected_transfers': run.get('rejected_transfers', ''),
                'results_directory': results_dir
            })
    
    return results


def create_summary_csv(data: Dict[str, Any], json_path: str) -> List[Dict[str, Any]]:
    """
    Create summary CSV with aggregated statistics only.
    """
    results = []
    results_dir = extract_results_directory(json_path)
    
    if 'aggregate' in data:
        agg = data['aggregate']
        results.append({
            'metric': 'throughput_tps',
            'mean': round(agg['throughput'].get('mean', 0), 2),
            'stddev': round(agg['throughput'].get('stddev', 0), 2),
            'min': round(agg['throughput'].get('min', 0), 2),
            'max': round(agg['throughput'].get('max', 0), 2),
            'cv': round(agg['throughput'].get('cv', 0), 6),
            'results_directory': results_dir
        })
        
        for metric in ['latency_p50', 'latency_p95', 'latency_p99', 'latency_p999']:
            if metric in agg:
                m = agg[metric]
                results.append({
                    'metric': metric,
                    'mean': round(m.get('mean', 0), 2),
                    'stddev': round(m.get('stddev', 0), 2),
                    'min': round(m.get('min', 0), 2),
                    'max': round(m.get('max', 0), 2),
                    'cv': round(m.get('cv', 0), 6),
                    'results_directory': results_dir
                })
    
    return results


def write_csv(filepath: str, rows: List[Dict[str, Any]]) -> None:
    """Write rows to CSV file."""
    if not rows:
        print('Warning: No data to write', file=sys.stderr)
        return
    
    fieldnames = list(rows[0].keys())
    
    with open(filepath, 'w', newline='') as f:
        writer = csv.DictWriter(f, fieldnames=fieldnames)
        writer.writeheader()
        writer.writerows(rows)
    
    print(f'✓ Wrote {len(rows)} rows to {filepath}')


def main():
    args = parse_args()
    
    # Validate input file
    if not os.path.exists(args.input_json):
        print(f'Error: Input file not found: {args.input_json}', file=sys.stderr)
        sys.exit(1)
    
    # Load JSON
    try:
        data = load_json(args.input_json)
    except json.JSONDecodeError as e:
        print(f'Error: Failed to parse JSON: {e}', file=sys.stderr)
        sys.exit(1)
    
    # Create appropriate CSV based on format
    if args.format == 'detailed':
        rows = create_detailed_csv(data, args.input_json)
    elif args.format == 'summary':
        rows = create_summary_csv(data, args.input_json)
    else:  # simple
        rows = create_simple_csv(data, args.input_json)
    
    # Write CSV
    try:
        write_csv(args.output_csv, rows)
    except IOError as e:
        print(f'Error: Failed to write CSV: {e}', file=sys.stderr)
        sys.exit(1)


if __name__ == '__main__':
    main()
