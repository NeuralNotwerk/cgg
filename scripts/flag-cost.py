#!/usr/bin/env python3
"""What one flag costs: the same binary, with and without it, paired.

`scripts/perf-compare.sh` times two *binaries* on fixed flags. This times
one binary on two *flag sets* — the question an opt-in feature has to
answer ("what does `--externals` cost, per language?") and the default
gates never ask, because the default never turns it on.

    python3 scripts/flag-cost.py --flags="--externals=all" [--runs 5]
        [--corpus repos|full|both] [--bin target/release/cgg] [--repos a,b]

Two tables:

* **per language** — the `REPOS` rows of scripts/benchmark.sh, each run
  with `--lang <lang>` on its source subdirectory, so each row is exactly
  one language;
* **full corpus** — every directory under $CGG_BENCH_DIR, grouped by the
  run's dominant language (most files, from the run's own metrics).

Each repo: one probe with the flags under CGG_REPO_TIMEOUT (default 60 s;
a timeout is NAMED and excluded), then RUNS alternating A/B pairs at
`--jobs 1`, median of each. CGG_TOTAL_BUDGET (default 1800 s) is checked
before every repo; repos not reached are named. Peak RSS comes from
`/usr/bin/time -f %M` when available. Output sizes are the report and
graph bytes of the probe, with the `--rollup` token estimate.

Markdown goes to stdout; the full JSON to target/flag-cost/.
"""

from __future__ import annotations

import argparse
import collections
import hashlib
import json
import os
import re
import shlex
import statistics
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TIME = "/usr/bin/time" if Path("/usr/bin/time").exists() else None


def sha(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()[:12]


def tokens(nbytes: int, text: str) -> int:
    # The `--rollup` estimator: max(words * 2.5, bytes / 1.8).
    return int(max(len(text.split()) * 2.5, nbytes / 1.8))


def repos_rows() -> list[tuple[str, Path, str]]:
    src = (ROOT / "scripts/benchmark.sh").read_text()
    i = src.index("REPOS=(")
    j = src.index("\n)\n", i)
    bench = Path(os.environ.get("CGG_BENCH_DIR", "/storage/cgg-test_repos"))
    out = []
    for name, lang, sub in re.findall(
        r'^\s*"([^"|]+)\|[^|]*\|([^|]*)\|([^|]*)\|', src[i:j], re.MULTILINE
    ):
        d = "asm-xv6" if name == "asm-xv6-mixed" else name
        out.append((name, bench / d / sub, lang))
    return out


def run_once(cmd: list[str], timeout: int) -> tuple[float, int | None, int]:
    """Wall ms, peak RSS KiB (or None), return code."""
    with tempfile.NamedTemporaryFile() as tf:
        full = ([TIME, "-f", "%M", "-o", tf.name] if TIME else []) + cmd
        # Not `subprocess.run(timeout=…)`: with a timeout, Python polls the
        # child with sleeps of up to 50 ms, which quantizes every timing.
        # A blocking wait plus a kill timer measures to the microsecond.
        t0 = time.monotonic()
        p = subprocess.Popen(full, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        killer = threading.Timer(timeout, p.kill)
        killer.start()
        try:
            p.wait()
        finally:
            killer.cancel()
        ms = (time.monotonic() - t0) * 1000
        if p.returncode == -9:
            raise subprocess.TimeoutExpired(cmd, timeout)
        rss = None
        if TIME:
            try:
                rss = int(Path(tf.name).read_text().strip().splitlines()[-1])
            except (ValueError, IndexError):
                rss = None
        return ms, rss, p.returncode


def measure(
    binary: Path,
    path: Path,
    lang: str | None,
    flags: list[str],
    runs: int,
    timeout: int,
):
    base = [str(binary), str(path), "-t", "json", "--jobs", "1", "-q"]
    if lang:
        base += ["--lang", lang]
    with tempfile.TemporaryDirectory() as td:
        report = Path(td) / "report"
        graph = Path(td) / "graph"
        a = base
        b = base + flags + ["--externals-report", str(report)]
        # Probe: bounded, and it captures the sizes and the languages.
        probe = subprocess.run(
            b + ["--externals-format", "json"],
            capture_output=True,
            timeout=timeout,
            check=False,
        )
        if probe.returncode != 0:
            return {
                "error": probe.stderr.decode(errors="replace")[-300:],
                "rc": probe.returncode,
            }
        graph.write_bytes(probe.stdout)
        try:
            doc = json.loads(report.read_text())
        except (OSError, ValueError):
            doc = {}
        try:
            g = json.loads(probe.stdout)
            by_lang = g.get("metrics", {}).get("by_language", {})
        except ValueError:
            by_lang = {}
        ta, tb, ra, rb = [], [], [], []
        for k in range(runs):
            order = (
                [(a, ta, ra), (b, tb, rb)] if k % 2 == 0 else [(b, tb, rb), (a, ta, ra)]
            )
            for cmd, ts, rs in order:
                ms, rss, rc = run_once(cmd, timeout)
                if rc != 0:
                    return {"error": f"rc {rc} during timing", "rc": rc}
                ts.append(ms)
                if rss is not None:
                    rs.append(rss)
        rep_text = report.read_text() if report.exists() else ""
        dominant = max(
            (
                (l, v.get("files", 0))
                for l, v in by_lang.items()
                if not l.startswith("skip:")
            ),
            key=lambda x: x[1],
            default=(lang or "?", 0),
        )[0]
        return {
            "base_ms": statistics.median(ta),
            "flag_ms": statistics.median(tb),
            "base_rss_kib": max(ra) if ra else None,
            "flag_rss_kib": max(rb) if rb else None,
            "report_bytes": len(rep_text.encode()),
            "report_tokens": tokens(len(rep_text.encode()), rep_text),
            "libraries": len(doc.get("libraries", [])),
            "sites": doc.get("totals", {}).get("sites", 0),
            "dominant_language": lang or dominant,
        }


def pct(a: float, b: float) -> float:
    return 100.0 * (b - a) / a if a else 0.0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--flags", default="--externals=all")
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--corpus", choices=["repos", "full", "both"], default="both")
    ap.add_argument("--bin", default=str(ROOT / "target/release/cgg"))
    ap.add_argument("--repos", default="")
    args = ap.parse_args()

    binary = Path(args.bin)
    flags = shlex.split(args.flags)
    per_repo = int(os.environ.get("CGG_REPO_TIMEOUT", "60"))
    budget = int(os.environ.get("CGG_TOTAL_BUDGET", "1800"))
    bench = Path(os.environ.get("CGG_BENCH_DIR", "/storage/cgg-test_repos"))
    want = set(args.repos.split(",")) if args.repos else None

    jobs: list[tuple[str, str, Path, str | None]] = []
    if args.corpus in ("repos", "both"):
        jobs += [("repos", n, p, l) for n, p, l in repos_rows()]
    if args.corpus in ("full", "both"):
        jobs += [
            ("full", p.name, p, None) for p in sorted(bench.iterdir()) if p.is_dir()
        ]
    if want:
        jobs = [j for j in jobs if j[1] in want]

    started = time.monotonic()
    results, timeouts, failures, not_run = [], [], [], []
    for i, (table, name, path, lang) in enumerate(jobs):
        if time.monotonic() - started > budget:
            not_run = [f"{t}:{n}" for t, n, _, _ in jobs[i:]]
            break
        if not path.exists():
            failures.append((f"{table}:{name}", "path missing"))
            continue
        try:
            r = measure(binary, path, lang, flags, args.runs, per_repo)
        except subprocess.TimeoutExpired:
            timeouts.append(f"{table}:{name}")
            print(f"  TIMEOUT {table}:{name} (excluded)", file=sys.stderr)
            continue
        if "error" in r:
            failures.append((f"{table}:{name}", r["error"]))
            continue
        r.update({"table": table, "repo": name})
        results.append(r)
        print(
            f"  {table}:{name:36s} {r['base_ms']:8.0f} → {r['flag_ms']:8.0f} ms ({pct(r['base_ms'], r['flag_ms']):+.1f}%)",
            file=sys.stderr,
        )

    def runway(r):
        d = r["flag_ms"] - r["base_ms"]
        return d <= max(0.20 * r["base_ms"], 5000)

    out = {
        "binary_sha": sha(binary),
        "flags": flags,
        "runs": args.runs,
        "timeouts": timeouts,
        "failures": failures,
        "not_run": not_run,
        "results": results,
    }
    dest = ROOT / "target/flag-cost"
    dest.mkdir(parents=True, exist_ok=True)
    tag = hashlib.sha256(" ".join(flags).encode()).hexdigest()[:8]
    path = dest / f"{out['binary_sha']}-{tag}.json"
    path.write_text(json.dumps(out, indent=2))

    print(
        f"## Cost of `{' '.join(flags)}` (binary {out['binary_sha']}, --jobs 1, median of {args.runs})\n"
    )
    per = [r for r in results if r["table"] == "repos"]
    if per:
        print("### Per language (one benchmark repo each, `--lang`)\n")
        print(
            "| language | repo | base ms | flag ms | Δ ms | Δ% | RSS Δ MiB | report KiB | ~tokens | libraries |"
        )
        print("| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |")
        for r in sorted(per, key=lambda r: r["dominant_language"]):
            rss = "—"
            if r["base_rss_kib"] and r["flag_rss_kib"]:
                rss = f"{(r['flag_rss_kib'] - r['base_rss_kib']) / 1024:+.1f}"
            mark = "" if runway(r) else " ⚠"
            print(
                f"| {r['dominant_language']} | {r['repo']} | {r['base_ms']:.0f} | {r['flag_ms']:.0f} | "
                f"{r['flag_ms'] - r['base_ms']:+.0f} | {pct(r['base_ms'], r['flag_ms']):+.1f}%{mark} | {rss} | "
                f"{r['report_bytes'] / 1024:.1f} | {r['report_tokens']} | {r['libraries']} |"
            )
    full = [r for r in results if r["table"] == "full"]
    if full:
        print("\n### Full corpus, by dominant language\n")
        print(
            "| language | repos | median Δ% | p90 Δ% | max Δ% | worst repo | total Δ s |"
        )
        print("| --- | ---: | ---: | ---: | ---: | --- | ---: |")
        groups = collections.defaultdict(list)
        for r in full:
            groups[r["dominant_language"]].append(r)
        for lang, rs in sorted(groups.items()):
            ds = sorted(pct(r["base_ms"], r["flag_ms"]) for r in rs)
            p90 = ds[min(len(ds) - 1, int(0.9 * len(ds)))]
            worst = max(rs, key=lambda r: pct(r["base_ms"], r["flag_ms"]))
            tot = sum(r["flag_ms"] - r["base_ms"] for r in rs) / 1000
            print(
                f"| {lang} | {len(rs)} | {statistics.median(ds):+.1f}% | {p90:+.1f}% | {ds[-1]:+.1f}% | {worst['repo']} | {tot:+.1f} |"
            )
        tb = sum(r["base_ms"] for r in full) / 1000
        tf = sum(r["flag_ms"] for r in full) / 1000
        print(
            f"\nTotal: {tb:.1f} s → {tf:.1f} s ({pct(tb, tf):+.1f}%) over {len(full)} repos."
        )
    over = [r for r in results if not runway(r)]
    print(
        f"\nOutside the opt-in runway max(20%, 5 s): {', '.join(r['table'] + ':' + r['repo'] for r in over) or 'none'}"
    )
    for label, items in (
        ("Timed out (excluded)", timeouts),
        ("Not run (budget)", not_run),
    ):
        if items:
            print(f"\n**{label}:** {', '.join(items)}")
    if failures:
        print("\n**Failed:** " + "; ".join(f"{n}: {e[:120]}" for n, e in failures))
    print(f"\nFull data: {path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
