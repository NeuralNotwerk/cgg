#!/usr/bin/env python3
"""Fail the commit when README.md disagrees with the code.

When this runs:
  - Automatically on every commit, by `.githooks/pre-commit` (last step,
    after all README patchers have run, so what it checks is the
    post-patch state).
  - Safe to run by hand from the repo root any time:
    `python3 scripts/docs-check.py`

Two checks:

1. Language-count consistency. The number of `register(` calls in
   `crates/cgg-lang/src/plugins.rs` must equal:
     - the `(N)` in README's "Supported languages (N)" heading,
     - the row count of the language table,
     - the entry count of `REPOS=( … )` in `scripts/benchmark.sh`
       (allowing one extra row for the combined `xv6 (c+asm)` entry,
       which is two languages running through one repo).

2. CLI flag freshness. Every flag named in the README's `## CLI` flag
   table must exist in `cgg --help` (catches renames/removals). The
   reverse direction is intentionally not checked — the README is
   curated and omits niche flags by design.

Run from the repo root. Exits non-zero on any mismatch with a
human-readable message naming the offending file.
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
PLUGINS_RS = REPO_ROOT / "crates/cgg-lang/src/plugins.rs"
BENCH_SH = REPO_ROOT / "scripts/benchmark.sh"
README = REPO_ROOT / "README.md"
CGG_BIN = REPO_ROOT / "target/release/cgg"


def fail(msg: str) -> None:
    print(f"[docs-check] {msg}", file=sys.stderr)
    sys.exit(1)


# ---------------------------------------------------------------------
# Check 1: language counts
# ---------------------------------------------------------------------

def count_plugins() -> int:
    text = PLUGINS_RS.read_text()
    return len(re.findall(r"\bregister\(", text))


def readme_intro_count(readme: str) -> int:
    m = re.search(r"## Supported languages \((\d+)\)", readme)
    if not m:
        fail("README is missing '## Supported languages (N)' heading")
    return int(m.group(1))


def readme_lang_table_rows(readme: str) -> int:
    # The language table starts with this header.
    m = re.search(
        r"\| Language \| Cross-file resolution \|.*?\n\|[-| ]+\|\n((?:\|.*\n)+)",
        readme,
    )
    if not m:
        fail("README language table not found")
    body = m.group(1)
    return sum(1 for line in body.splitlines() if line.startswith("|"))


def benchmark_repo_count() -> int:
    text = BENCH_SH.read_text()
    # benchmark.sh has lines like:  "name|lang|url|subdir"  inside REPOS=( ... )
    m = re.search(r"REPOS=\(\s*\n(.*?)\n\)", text, re.DOTALL)
    if not m:
        fail("could not locate REPOS=( ... ) array in scripts/benchmark.sh")
    body = m.group(1)
    return sum(1 for line in body.splitlines() if line.strip().startswith('"'))


def check_language_counts() -> None:
    plugins = count_plugins()
    readme = README.read_text()
    intro = readme_intro_count(readme)
    rows = readme_lang_table_rows(readme)
    bench = benchmark_repo_count()
    # benchmark.sh allows one extra entry: the combined "xv6 (c+asm)"
    # row exercises the multi-language path with no new plugin.
    bench_expected_min, bench_expected_max = plugins, plugins + 1

    if intro != plugins:
        fail(
            f"plugin count {plugins} != README intro '({intro})' — update "
            f"'## Supported languages (N)' in README.md"
        )
    if rows != plugins:
        fail(
            f"plugin count {plugins} != README language table rows {rows} — "
            f"add/remove a row in the language table"
        )
    if not bench_expected_min <= bench <= bench_expected_max:
        fail(
            f"plugin count {plugins} but scripts/benchmark.sh has {bench} "
            f"REPOS entries (expected {plugins} or {plugins + 1})"
        )


# ---------------------------------------------------------------------
# Check 2: CLI flag freshness
# ---------------------------------------------------------------------

def cgg_help_flags() -> set[str]:
    if not CGG_BIN.exists():
        fail(f"{CGG_BIN} not found — run `cargo build --release -p cgg` first")
    out = subprocess.run(
        [str(CGG_BIN), "--help"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    flags: set[str] = set()
    for m in re.finditer(r"(?<![A-Za-z0-9])(--[a-z][a-z0-9-]*|-[a-zA-Z])(?=[ ,\n])", out):
        flags.add(m.group(1))
    return flags


def readme_flag_table_entries(readme: str) -> list[str]:
    # Slurp lines under "## CLI" between the table header and the next
    # blank line / next heading.
    m = re.search(
        r"## CLI\b.*?\n\| Flag \| Default \| Description \|\n\|[-| ]+\|\n((?:\|.*\n)+)",
        readme,
        re.DOTALL,
    )
    if not m:
        fail("README '## CLI' flag table not found")
    body = m.group(1)
    entries: list[str] = []
    for line in body.splitlines():
        if not line.startswith("|"):
            continue
        first_col = line.split("|", 2)[1].strip()
        # First column entries look like `` `--filter` `` or `` `-t` ``.
        for tok in re.findall(r"`([^`]+)`", first_col):
            entries.append(tok)
    return entries


def check_cli_flags() -> None:
    readme = README.read_text()
    documented = readme_flag_table_entries(readme)
    real = cgg_help_flags()
    stale = [f for f in documented if f not in real]
    if stale:
        fail(
            f"README documents flag(s) that no longer exist in `cgg --help`: "
            f"{', '.join(stale)} — update the '## CLI' table in README.md"
        )


# ---------------------------------------------------------------------

def main() -> None:
    check_language_counts()
    check_cli_flags()
    print("[docs-check] ok")


if __name__ == "__main__":
    main()
