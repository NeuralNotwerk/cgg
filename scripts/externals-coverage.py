#!/usr/bin/env python3
"""Per-language `--externals` coverage over the whole benchmark corpus.

For every directory under $CGG_BENCH_DIR (default /storage/cgg-test_repos)
this runs

    cgg <repo> --externals=all --externals-format json --no-graph -q

and aggregates the report's per-language coverage table: how many call
sites leave the tree, how many cgg attributed to a library and by which
evidence, the top libraries, and the top unattributed names. It is the
correctness counterpart of `scripts/flag-cost.py`.

Bounded like every corpus script: CGG_REPO_TIMEOUT (default 60 s) per
repo, CGG_TOTAL_BUDGET (default 1800 s) for the whole run. A repo that
times out or fails is NAMED and excluded, never averaged in.

Usage:
    python3 scripts/externals-coverage.py [--bin target/release/cgg]
        [--repos a,b] [--out DIR]

Writes a markdown summary to stdout and the full JSON to
target/externals-coverage/<binary-sha>.json (or --out).
"""

from __future__ import annotations

import argparse
import collections
import hashlib
import json
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def sha(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()[:12]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--bin", default=str(ROOT / "target/release/cgg"))
    ap.add_argument("--repos", default="", help="comma list of corpus dir names")
    ap.add_argument("--out", default="")
    args = ap.parse_args()

    bench = Path(os.environ.get("CGG_BENCH_DIR", "/storage/cgg-test_repos"))
    per_repo = int(os.environ.get("CGG_REPO_TIMEOUT", "60"))
    budget = int(os.environ.get("CGG_TOTAL_BUDGET", "1800"))
    binary = Path(args.bin)
    repos = sorted(p for p in bench.iterdir() if p.is_dir())
    if args.repos:
        want = set(args.repos.split(","))
        repos = [r for r in repos if r.name in want]

    started = time.monotonic()
    lang = collections.defaultdict(lambda: collections.Counter())
    lang_libs = collections.defaultdict(collections.Counter)
    lang_unattr = collections.defaultdict(collections.Counter)
    lang_evidence = collections.defaultdict(collections.Counter)
    lang_repos = collections.defaultdict(set)
    timeouts, failures, not_run = [], [], []
    rows = []
    for i, repo in enumerate(repos):
        if time.monotonic() - started > budget:
            not_run = [r.name for r in repos[i:]]
            break
        t0 = time.monotonic()
        try:
            p = subprocess.run(
                [str(binary), str(repo), "--externals=all", "--externals-format", "json", "--no-graph", "-q"],
                capture_output=True,
                timeout=per_repo,
            )
        except subprocess.TimeoutExpired:
            timeouts.append(repo.name)
            print(f"  TIMEOUT {repo.name} (> {per_repo}s, excluded)", file=sys.stderr)
            continue
        ms = (time.monotonic() - t0) * 1000
        if p.returncode != 0:
            failures.append((repo.name, p.returncode, p.stderr.decode(errors="replace")[-400:]))
            print(f"  FAILED {repo.name} rc={p.returncode}", file=sys.stderr)
            continue
        doc = json.loads(p.stdout)
        for c in doc["coverage"]:
            L = c["language"]
            lang[L]["files"] += c["files"]
            lang[L]["imports"] += c["imports"]
            lang[L]["sites"] += c["sites"]
            lang[L]["attributed"] += c["attributed"]
            lang[L]["unattributed"] += c["unattributed"]
            lang_repos[L].add(repo.name)
        for lib in doc["libraries"]:
            lang_libs[lib["language"]][(lib["kind"], lib["name"])] += len(lib["sites"]) or 0
            for s in lib["sites"]:
                lang_evidence[lib["language"]][s["evidence"]] += 1
        for g in doc["unattributed"]:
            lang_unattr[g["language"]][g["name"]] += g["sites"]
        rows.append({"repo": repo.name, "ms": round(ms, 1), "libraries": len(doc["libraries"]), "sites": doc["totals"]["sites"]})
        print(f"  {repo.name:40s} {ms:8.0f} ms  {len(doc['libraries']):5d} libs  {doc['totals']['sites']:8d} sites", file=sys.stderr)

    out = {
        "binary": str(binary),
        "binary_sha": sha(binary),
        "repos_run": len(rows),
        "timeouts": timeouts,
        "failures": failures,
        "not_run": not_run,
        "languages": {
            L: {
                **dict(v),
                "repos": len(lang_repos[L]),
                "attributed_pct": round(100 * v["attributed"] / v["sites"], 1) if v["sites"] else None,
                "evidence": dict(lang_evidence[L]),
                "top_libraries": [f"{k}:{n} ({c})" for (k, n), c in lang_libs[L].most_common(12)],
                "top_unattributed": [f"{n} ({c})" for n, c in lang_unattr[L].most_common(12)],
            }
            for L, v in sorted(lang.items())
        },
        "repos": rows,
    }
    dest = Path(args.out) if args.out else ROOT / "target/externals-coverage"
    dest.mkdir(parents=True, exist_ok=True)
    path = dest / f"{out['binary_sha']}.json"
    path.write_text(json.dumps(out, indent=2))

    print(f"## --externals coverage ({len(rows)} repos, binary {out['binary_sha']})\n")
    print("| language | repos | files | sites | attributed | top libraries |")
    print("| --- | ---: | ---: | ---: | ---: | --- |")
    for L, v in out["languages"].items():
        pct = "—" if v["attributed_pct"] is None else f"{v['attributed_pct']}%"
        libs = ", ".join(x.split(" (")[0].split(":", 1)[1] for x in v["top_libraries"][:5])
        print(f"| {L} | {v['repos']} | {v['files']} | {v['sites']} | {pct} | {libs} |")
    for label, items in (("Timed out (excluded)", timeouts), ("Not run (budget)", not_run)):
        if items:
            print(f"\n**{label}:** {', '.join(items)}")
    if failures:
        print("\n**Failed:** " + ", ".join(f"{n} (rc {rc})" for n, rc, _ in failures))
    print(f"\nFull data: {path}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
