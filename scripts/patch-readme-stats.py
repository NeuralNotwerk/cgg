#!/usr/bin/env python3
"""Patch README.md with generated stats, graph, and benchmark table."""
import re
import sys

readme_path = sys.argv[1]
stats_path = sys.argv[2]
graph_path = sys.argv[3]
bench_path = sys.argv[4]

with open(readme_path) as f:
    content = f.read()

# Parse self-analysis stats
with open(stats_path) as f:
    stats_line = f.read().strip()
calls = re.search(r'(\d+) callables', stats_line).group(1)
edges = re.search(r'(\d+) edges', stats_line).group(1)
cf = re.search(r'(\d+) cross-file', stats_line).group(1)
ms = re.search(r'([\d.]+) ms', stats_line).group(1)
ms_int = str(int(float(ms)))

# 1. Update the self-analysis description line
content = re.sub(
    r'`cgg` run on its own source \([^)]*\)',
    f'`cgg` run on its own source ({calls} callables, {edges} edges, {cf} cross-file, {ms_int}ms)',
    content
)

# 2. Replace the self-analysis mermaid graph
with open(graph_path) as f:
    new_graph = f.read().rstrip()

lines = content.split('\n')
start_idx = None
end_idx = None
for i, line in enumerate(lines):
    if 'cgg ./crates -t mermaid --filter' in line and 'cgg::run' in line:
        for j in range(i, min(i + 5, len(lines))):
            if lines[j] == '```mermaid':
                start_idx = j + 1
                break
    if start_idx and end_idx is None and i > start_idx and line == '```':
        end_idx = i
        break

if start_idx and end_idx:
    lines = lines[:start_idx] + new_graph.split('\n') + lines[end_idx:]
    content = '\n'.join(lines)

# 3. Replace the benchmark table
with open(bench_path) as f:
    new_table = f.read().rstrip()

lines = content.split('\n')
start_idx = None
end_idx = None
for i, line in enumerate(lines):
    if '| Project | Language | Callables' in line:
        start_idx = i
    if start_idx and i > start_idx + 1 and not line.startswith('|'):
        end_idx = i
        break

if start_idx and end_idx:
    lines = lines[:start_idx] + new_table.split('\n') + lines[end_idx:]
    content = '\n'.join(lines)

with open(readme_path, 'w') as f:
    f.write(content)

print(f"Patched: stats={calls}/{edges}/{cf}/{ms_int}ms, graph updated, table updated")
