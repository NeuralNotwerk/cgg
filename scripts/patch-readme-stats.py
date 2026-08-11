#!/usr/bin/env python3
"""Patch README.md with the self-analysis mermaid graph and benchmark table.

When this runs:
  - Manual only. Invoked by `scripts/update-readme-stats.sh` after
    `scripts/benchmark.sh` has refreshed clones under $CGG_BENCH_DIR.
  - NEVER invoked by the pre-commit hook — the inputs are too slow to
    regenerate on every commit (full benchmark sweep + a self-analysis
    cgg run).

What this owns:
  - The mermaid graph under README's "## Self-analysis" heading.
  - The benchmark table at the end of README.

What this does NOT own (don't add it back):
  - The self-stats line `(N callables, …, Nms)` — patched separately by
    `scripts/update-readme-stats.py` via
    `<!-- cgg:begin:self-stats -->` markers. Reintroducing a regex
    substitution here would strip those markers.

Usage:
  patch-readme-stats.py <readme> <self-graph.mmd> <bench-table.md>
"""
import sys
from pathlib import Path


def die(msg: str) -> None:
    """Refuse to touch README rather than write a measurement that failed."""
    sys.exit(f"patch-readme-stats: refusing to write: {msg}")


readme_path = sys.argv[1]
graph_path = sys.argv[2]
bench_path = sys.argv[3]

with open(readme_path) as f:
    content = f.read()

# 1. Replace the self-analysis mermaid graph
with open(graph_path) as f:
    new_graph = f.read().rstrip()

# Both inputs are *measured* output. When the measurement fails, cgg
# leaves behind an empty (or header-only) file, and the naive splice
# below happily replaces a live README section with nothing — deleting
# the benchmark table and the self-analysis graph while printing
# "Patched" and exiting 0. Validate the inputs before touching README.
if not new_graph:
    die(f"{graph_path} is empty - the cgg self-analysis run produced no graph")
if "flowchart" not in new_graph:
    die(f"{graph_path} has no 'flowchart' line - not a mermaid graph")

new_table_raw = Path(bench_path).read_text().rstrip()
table_rows = [ln for ln in new_table_raw.splitlines() if ln.startswith("|")]
# header + separator + at least one measured project
if len(table_rows) < 3:
    die(
        f"{bench_path} has {len(table_rows)} table rows (need header + "
        f"separator + >=1 project). The benchmark sweep did not produce "
        f"data - check $CGG_BENCH_DIR is populated."
    )

lines = content.split('\n')
start_idx = None
end_idx = None
for i, line in enumerate(lines):
    if 'cgg ./crates -t mermaid --filter' in line and 'cgg::analyze_in_pool' in line:
        for j in range(i, min(i + 5, len(lines))):
            if lines[j] == '```mermaid':
                start_idx = j + 1
                break
    if start_idx and end_idx is None and i > start_idx and line == '```':
        end_idx = i
        break

if not (start_idx and end_idx):
    die(
        "could not locate the self-analysis mermaid block in "
        f"{readme_path} (anchor: the `cgg ./crates -t mermaid --filter "
        "... cgg::analyze_in_pool` command followed by a ```mermaid fence)"
    )
lines = lines[:start_idx] + new_graph.split('\n') + lines[end_idx:]
content = '\n'.join(lines)

# 2. Replace the benchmark table
new_table = new_table_raw

lines = content.split('\n')
start_idx = None
end_idx = None
for i, line in enumerate(lines):
    if '| Project | Language | Callables' in line:
        start_idx = i
    if start_idx and i > start_idx + 1 and not line.startswith('|'):
        end_idx = i
        break

if not (start_idx and end_idx):
    die(
        f"could not locate the benchmark table in {readme_path} "
        "(anchor: a '| Project | Language | Callables' header row)"
    )
lines = lines[:start_idx] + new_table.split('\n') + lines[end_idx:]
content = '\n'.join(lines)

with open(readme_path, 'w') as f:
    f.write(content)

print("Patched: graph updated, table updated")
