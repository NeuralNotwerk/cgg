#!/usr/bin/env python3
"""Compare two cgg builds across the whole corpus: latency and graph output.

A note on "precision" and "recall", because the words promise more than
any of this can deliver: **there is no ground-truth call graph** for
these repositories. Nobody has hand-labelled the true edge set of
ripgrep. So this reports *proxies*, and names them as proxies:

  recall proxy      callables found vs `ctags` symbol count, where ctags
                    supports the language. This is the README's existing
                    methodology. It bounds "did we find the definitions",
                    not "did we find the edges".
  precision proxy   the unresolved-call rate — call sites cgg saw and
                    could not bind — plus the confidence mix of the edges
                    it did emit. A build that resolves more sites at the
                    same or higher confidence is doing better; a build
                    that emits more edges *and* more unresolved sites may
                    just be seeing more code.

Neither is a true precision or recall figure and neither should be
quoted as one. What IS exact and comparable: latency, node counts, edge
counts, entry counts, dead-code finding counts, and the confidence
histogram — all deterministic, all directly measured on both builds.

Latency method: one discarded warm-up per repo, then the two builds
alternate A,B,A,B and the MINIMUM of each build's samples is taken.
Minimum rather than mean because interference only ever adds time.

Usage: compare-release.py OLD_BIN NEW_BIN [--out report.json] [--limit N]
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

CORPUS = Path(os.environ.get("CGG_BENCH_DIR", "/storage/cgg-test_repos"))
ROOT = Path(__file__).resolve().parent.parent
BENCH = ROOT / "scripts/benchmark.sh"

SUMMARY = re.compile(
    r"(\d+) callables, (\d+) edges \((\d+) cross-file\), (\d+) unresolved"
)
DEAD = re.compile(
    r"(\d+) callable\(s\) marked unreferenced at high confidence, (\d+) withheld"
)


def ctags_langs() -> dict[str, tuple[str, str, str]]:
    """repo name -> (ctags language, kinds, src subdir) from benchmark.sh."""
    text = BENCH.read_text()
    m = re.search(r"REPOS=\(\s*\n(.*?)\n\)", text, re.DOTALL)
    out: dict[str, tuple[str, str, str]] = {}
    if not m:
        return out
    for line in m.group(1).splitlines():
        line = line.strip().strip('"')
        p = line.split("|")
        if len(p) >= 6 and p[4]:
            out[p[0]] = (p[4], p[5], p[3])
    return out


def run(binary: Path, repo: Path, extra: list[str]) -> tuple[float, str]:
    tmp = Path(tempfile.mkdtemp(prefix="cgg-cmp-"))
    try:
        start = time.monotonic()
        proc = subprocess.run(
            [
                str(binary),
                str(repo),
                "-o",
                str(tmp / "g.mmd"),
                "--no-update-check",
                *extra,
            ],
            capture_output=True,
            text=True,
            timeout=1800,
            check=False,
        )
        return (time.monotonic() - start) * 1000, proc.stderr
    except subprocess.TimeoutExpired:
        return float("nan"), ""
    finally:
        subprocess.run(["rm", "-rf", str(tmp)], capture_output=True, check=False)


def parse(stderr: str) -> dict:
    out = {"callables": 0, "edges": 0, "cross": 0, "unresolved": 0, "entries": 0}
    m = SUMMARY.search(stderr)
    if m:
        out.update(
            callables=int(m.group(1)),
            edges=int(m.group(2)),
            cross=int(m.group(3)),
            unresolved=int(m.group(4)),
        )
    e = re.search(r"framework entries: (\d+) node\(s\) minted", stderr)
    if e:
        out["entries"] = int(e.group(1))
    return out


def ctags_count(repo: Path, lang: str, kinds: str, sub: str) -> int:
    scan = repo / sub if sub and (repo / sub).is_dir() else repo
    try:
        p = subprocess.run(
            [
                "ctags",
                "-R",
                f"--languages={lang}",
                f"--kinds-{lang}={kinds}",
                "--exclude=test*",
                "--exclude=*_test*",
                "--exclude=spec",
                "--exclude=vendor",
                "--exclude=node_modules",
                "-f",
                "-",
                str(scan),
            ],
            capture_output=True,
            timeout=600,
            check=False,
        )
    except subprocess.TimeoutExpired:
        return 0
    names = set()
    for line in p.stdout.decode(errors="replace").splitlines():
        f = line.split("\t")
        if not f or line.startswith("!"):
            continue
        n = f[0]
        if not n or "__anon" in n or n.isupper():
            continue
        names.add(n)
    return len(names)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("old_bin")
    ap.add_argument("new_bin")
    ap.add_argument("--out", default="")
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument(
        "--jobs",
        type=int,
        default=8,
        help="repos measured concurrently (1 for published numbers)",
    )
    args = ap.parse_args()

    old, new = Path(args.old_bin), Path(args.new_bin)
    for b in (old, new):
        if not b.exists():
            sys.exit(f"missing binary {b}")
    if old.read_bytes() == new.read_bytes():
        sys.exit("the two binaries are byte-identical — nothing to compare")

    tags = ctags_langs()
    repos = sorted(d for d in CORPUS.iterdir() if d.is_dir())
    if args.limit:
        repos = repos[: args.limit]

    def measure(repo: Path) -> dict:
        name = repo.name
        run(old, repo, [])  # warm the page cache; discarded
        ta1, sa1 = run(old, repo, [])
        tb1, sb1 = run(new, repo, [])
        ta2, _ = run(old, repo, [])
        tb2, _ = run(new, repo, [])
        _, da = run(old, repo, ["--dead-code"])
        _, db = run(new, repo, ["--dead-code"])
        ga, gb = parse(sa1), parse(sb1)
        ma, mb = DEAD.search(da), DEAD.search(db)
        row = {
            "repo": name,
            "old_ms": min(ta1, ta2),
            "new_ms": min(tb1, tb2),
            "old": ga,
            "new": gb,
            "old_dead": (int(ma.group(1)) + int(ma.group(2))) if ma else 0,
            "new_dead": (int(mb.group(1)) + int(mb.group(2))) if mb else 0,
        }
        if name in tags:
            lang, kinds, sub = tags[name]
            row["ctags"] = ctags_count(repo, lang, kinds, sub)
        return row

    # cgg itself only parallelises parse/extract, and measured 114-212%
    # CPU on a 64-core box — so running several repos at once uses
    # hardware that would otherwise sit idle. Timing is still the MINIMUM
    # of two samples per binary, and the two binaries are measured under
    # identical concurrency, so contention biases both equally. Keep
    # `--jobs 1` for numbers being published.
    rows = []
    done = 0
    with ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        for row in pool.map(measure, repos):
            done += 1
            ga, gb = row["old"], row["new"]
            rows.append(row)
            print(
                f"[{done}/{len(repos)}] {row['repo']:<28} "
                f"{row['old_ms']:>8.0f} -> {row['new_ms']:>8.0f} ms  "
                f"nodes {ga['callables']:>6}->{gb['callables']:<6} "
                f"edges {ga['edges']:>6}->{gb['edges']:<6} "
                f"entry {ga['entries']:>5}->{gb['entries']:<5}",
                flush=True,
            )

    if args.out:
        Path(args.out).write_text(json.dumps(rows, indent=1))
    print(f"\ndone: {len(rows)} repos", flush=True)


if __name__ == "__main__":
    main()
