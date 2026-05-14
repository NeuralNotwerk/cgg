#!/usr/bin/env python3
"""Patch the README's self-analysis stat line from a fresh `cgg` run.

Reads `cgg`'s stderr summary line on stdin, extracts the four numbers
that show up in the README ("N callables, N edges, N cross-file, Nms"),
and rewrites the body between
`<!-- cgg:begin:self-stats -->` / `<!-- cgg:end:self-stats -->`.

When this runs:
  - Automatically on every commit, by `.githooks/pre-commit` (after the
    release build, before docs-check).
  - Also from `scripts/update-readme-stats.sh` so the manual benchmark-
    refresh workflow stays consistent with the hook.

Sub-millisecond timing variation is rounded to keep commits stable.
Idempotent: if the numbers already match, README is left untouched.

Usage:
  cgg ... 2>summary.txt && update-readme-stats.py < summary.txt
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

# cgg's summary line looks like:
#   cgg: 108 files, 72 analyzed, 36 skipped; 992 callables, 1459 edges
#        (1451 cross-file), 560 unresolved, 7337 external (126.9 ms)
SUMMARY_RE = re.compile(
    r"(\d+)\s+callables,\s+(\d+)\s+edges\s+\((\d+)\s+cross-file\),"
    r".*?\(([\d.]+)\s*ms\)"
)

BEGIN = "<!-- cgg:begin:self-stats -->"
END = "<!-- cgg:end:self-stats -->"


def main() -> None:
    stderr_text = sys.stdin.read()
    m = SUMMARY_RE.search(stderr_text)
    if not m:
        print(
            "error: could not find cgg summary line in stdin",
            file=sys.stderr,
        )
        sys.exit(1)

    callables, edges, cross_file, ms = m.groups()
    # Round ms to nearest int — sub-ms variation is noise that would
    # churn commits.
    ms_int = round(float(ms))
    new_body = (
        f"({callables} callables, {edges} edges, "
        f"{cross_file} cross-file, {ms_int}ms)"
    )

    readme_path = Path("README.md")
    readme = readme_path.read_text()

    pattern = re.compile(
        re.escape(BEGIN) + r".*?" + re.escape(END), re.DOTALL
    )
    replacement = f"{BEGIN}{new_body}{END}"
    patched, n = pattern.subn(replacement, readme)
    if n == 0:
        print(
            f"error: markers {BEGIN!r}/{END!r} not found in README.md",
            file=sys.stderr,
        )
        sys.exit(1)

    if patched != readme:
        readme_path.write_text(patched)


if __name__ == "__main__":
    main()
