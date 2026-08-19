#!/usr/bin/env python3
"""Hunt for nondeterminism in cgg's output across repos, formats and flags.

**Timing fields are excluded, and that exclusion is the whole reason this
script exists.** cgg embeds per-run wall/parse timings in its JSON graph
(`metrics.wall_ms`, `metrics.phases.*`) and in every file record
(`files[].parse_ms`), and the audit sidecar carries the same. Two
identical runs therefore never hash the same, and a naive byte comparison
reports nondeterminism that is not there. That false positive has been
hit three separate times during 0.5.0 development — twice by me — so the
stripping is done here, once, correctly, rather than reinvented per
check.

What IS compared, and must match exactly:

  callables   id, qualified name, language, start line, and their ORDER
  edges       src, dst, site_line, site_byte, via, confidence, resolver
  unresolved  file, name, reason
  files       everything except `parse_ms`
  dead-code   the report sidecar, verbatim
  text output mermaid / dot / graphml verbatim (they carry no timings)

Usage:
  determinism-sweep.py [--runs N] [--repos N] [--quick] [--json OUT]
"""

from __future__ import annotations

import argparse
import collections
import concurrent.futures as cf
import json
import os
import random
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CGG = Path(os.environ.get("CGG_BIN", ROOT / "target/release/cgg"))
CORPUS = Path(os.environ.get("CGG_BENCH_DIR", "/storage/cgg-test_repos"))

# Every flag combination worth exercising. Each has been a source of
# nondeterminism at some point, or reaches a code path the others do not.
CONFIGS = [
    ("default", []),
    ("dead-code", ["--dead-code"]),
    ("dead-code+tests", ["--dead-code", "--include-tests"]),
    ("dynamic-dispatch", ["--dynamic-dispatch"]),
    ("reference-edges", ["--reference-edges"]),
    ("external+stdlib", ["--include-external", "--include-stdlib"]),
    ("paths-truncated", ["-n", "0", "--max-paths", "7", "--filter", "e"]),
    ("neighborhood", ["-n", "2", "--filter", "e"]),
    ("no-entry-nodes", ["--no-entry-nodes"]),
]

FORMATS = ["json", "mermaid", "dot", "graphml"]


def strip_timings(doc: dict) -> dict:
    """Remove every field whose value legitimately varies between runs.

    Two distinct categories, and conflating them is how a determinism
    check produces a false positive:

      * TIMINGS — wall/parse/rss. Vary because the machine varies.
      * INVOCATION — `argv` in the audit's `run_started` record, which
        includes the `--jobs` value and the output path. Those differ
        because THIS SCRIPT deliberately varies them. Comparing them
        would flag every run as nondeterministic, which is exactly the
        first thing this harness did.
    """
    out = dict(doc)
    if out.get("event") == "run_started":
        out.pop("argv", None)
    # Audit events carry `parse_ms` at the TOP level (one
    # `file_analyzed` event per file), not nested the way the graph
    # document does. Strip it wherever it appears.
    out.pop("parse_ms", None)
    m = dict(out.get("metrics") or {})
    for k in ("wall_ms", "phases", "peak_rss_bytes"):
        m.pop(k, None)
    out["metrics"] = m
    files = out.get("files")
    if isinstance(files, dict):
        out["files"] = {
            k: {kk: vv for kk, vv in v.items() if kk != "parse_ms"}
            for k, v in files.items()
        }
    # file_audits carry the same per-file timing.
    fa = out.get("file_audits")
    if isinstance(fa, list):
        out["file_audits"] = [
            {kk: vv for kk, vv in e.items() if kk != "parse_ms"}
            if isinstance(e, dict)
            else e
            for e in fa
        ]
    return out


def canonical(path: Path, fmt: str) -> str:
    """A comparable representation of one output file."""
    raw = path.read_text(errors="replace")
    if fmt != "json":
        return raw
    return json.dumps(strip_timings(json.loads(raw)), sort_keys=False, indent=1)


def run_once(repo: Path, fmt: str, flags: list[str], jobs: int, tmp: Path) -> dict:
    out = tmp / f"g.{fmt}"
    cmd = [str(CGG), str(repo), "-t", fmt, "-o", str(out), "--no-update-check"]
    if jobs:
        cmd += ["--jobs", str(jobs)]
    proc = subprocess.run(
        [*cmd, *flags],
        capture_output=True,
        text=True,
        timeout=2400,
        # A nonzero exit is data, not an error: the caller records it as
        # a `run-failed` finding.
        check=False,
    )
    if not out.exists():
        return {"error": proc.stderr[-400:] or f"exit {proc.returncode}"}
    res = {"main": canonical(out, fmt)}
    audit = Path(str(out) + ".audit.json")
    if audit.exists():
        try:
            res["audit"] = json.dumps(
                [
                    strip_timings(e) if isinstance(e, dict) else e
                    for e in json.loads(audit.read_text())
                ],
                sort_keys=False,
                indent=1,
            )
        except json.JSONDecodeError as e:
            res["audit"] = f"UNPARSEABLE: {e}"
    for suffix in (".deadcode.json", ".deadcode.txt"):
        rep = Path(str(out) + suffix)
        if rep.exists():
            res["report"] = rep.read_text(errors="replace")
    return res


def first_diff(a: str, b: str) -> str:
    al, bl = a.split("\n"), b.split("\n")
    for i, (x, y) in enumerate(zip(al, bl)):
        if x != y:
            return f"line {i}:\n      A: {x[:150]}\n      B: {y[:150]}"
    if len(al) != len(bl):
        return f"length {len(al)} vs {len(bl)}"
    return "(identical?)"


def main() -> None:
    ap = argparse.ArgumentParser()
    # 2, not 4. Determinism is a PAIRWISE property: every run is compared
    # against the first, so run 2 is the one that can detect a difference
    # at all and every run after it is only a second chance at an
    # intermittent one. Depth per cell is the wrong place to buy that
    # confidence — the sweep already runs ~4k cells across the corpus, so
    # breadth samples intermittent nondeterminism far better than a third
    # or fourth repeat of the same cell would, and the cost here is
    # multiplicative across repos x configs x formats.
    ap.add_argument("--runs", type=int, default=2, help="runs per configuration")
    # The whole corpus by default. It used to be 14, which made a green
    # sweep a statement about a random eighth of the corpus while reading
    # like a statement about all of it. The work is
    # `repos x configs x formats x runs` cgg invocations — 110 x 9 x 4 x 3
    # is ~12k — so this is only affordable because the sweep runs them
    # concurrently; see `--workers`.
    ap.add_argument("--repos", type=int, default=10_000)
    # Each cgg run takes half the physical cores (capped), so a handful of
    # concurrent repos already saturates the box. Oversubscription is
    # harmless here and arguably useful: determinism must hold under any
    # scheduling, so varied contention is a stronger test, not a flakier
    # one.
    ap.add_argument(
        "--workers", type=int, default=max(1, (os.cpu_count() or 8) // 4)
    )
    ap.add_argument("--quick", action="store_true", help="default config only")
    ap.add_argument("--json", default="")
    ap.add_argument("--seed", type=int, default=0)
    args = ap.parse_args()

    if not CGG.exists():
        sys.exit(f"{CGG} not found - cargo build --release -p cgg")

    repos = sorted(d for d in CORPUS.iterdir() if d.is_dir())
    # Three repos are excluded deliberately. Each takes tens of seconds
    # per invocation, and the sweep runs `configs x formats x runs` of
    # them, so any one of the three costs more wall time than the other
    # 110 combined — while a timeout is not a determinism signal anyway.
    # They are NAMED in the summary rather than silently dropped, because
    # "the corpus is deterministic" and "110 of 113 repos are
    # deterministic" are different claims and only one of them is true.
    skip = {"dart-flutter", "erlang-otp", "zig-zig"}
    skipped_names = sorted(r.name for r in repos if r.name in skip)
    repos = [r for r in repos if r.name not in skip]
    rng = random.Random(args.seed)
    rng.shuffle(repos)
    repos = repos[: args.repos]

    configs = CONFIGS[:1] if args.quick else CONFIGS
    formats = ["json"] if args.quick else FORMATS

    def one_cell(repo: Path, cname: str, flags: list[str], fmt: str) -> tuple:
        """One (repo, config, format) cell: N runs, compared pairwise.

        Returns (findings, runs_done). Self-contained — its own tempdir,
        no shared state — which is what lets the cells run concurrently.
        """
        local: list[dict] = []
        ran = 0
        tmp = Path(tempfile.mkdtemp(prefix="cgg-det-"))
        try:
            base = None
            for _ in range(args.runs):
                # Every run uses the DEFAULT worker count. Thread counts
                # are no longer swept: the three defects found so far were
                # all reproducible at a fixed count (two of them
                # single-threaded), and forcing `--jobs 1` on a large repo
                # cost over a hundred seconds per run to test a dimension
                # that never produced a finding. Determinism across thread
                # counts is covered by tests/determinism.rs on a small
                # fixture, where it is cheap.
                jobs = 0
                res = run_once(repo, fmt, flags, jobs, tmp)
                ran += 1
                if "error" in res:
                    local.append(
                        {
                            "repo": repo.name,
                            "config": cname,
                            "fmt": fmt,
                            "kind": "run-failed",
                            "detail": res["error"],
                        }
                    )
                    break
                if base is None:
                    base = res
                    continue
                for part in ("main", "audit", "report"):
                    if part in base and part in res and base[part] != res[part]:
                        local.append(
                            {
                                "repo": repo.name,
                                "config": cname,
                                "fmt": fmt,
                                "kind": f"nondeterministic:{part}",
                                "jobs": jobs,
                                "detail": first_diff(base[part], res[part]),
                            }
                        )
                        break
        finally:
            # Best-effort cleanup in a finally block; a failure here must
            # not mask the exception being unwound.
            subprocess.run(["rm", "-rf", str(tmp)], capture_output=True, check=False)
        return local, ran

    # Cells are independent, so the sweep is embarrassingly parallel. It
    # was serial until 0.8.0, which is why a full-corpus run was ~95
    # minutes of a 64-thread machine sitting mostly idle and why the
    # default had been quietly reduced to 14 repos to compensate.
    cells = [
        (repo, cname, flags, fmt)
        for repo in repos
        for cname, flags in configs
        for fmt in formats
    ]
    findings, checked, done = [], 0, 0
    per_repo: dict[str, int] = collections.Counter()
    with cf.ThreadPoolExecutor(max_workers=args.workers) as pool:
        futures = {pool.submit(one_cell, *c): c for c in cells}
        for fut in cf.as_completed(futures):
            repo, cname, _flags, fmt = futures[fut]
            local, ran = fut.result()
            findings.extend(local)
            checked += ran
            per_repo[repo.name] += 1
            done += 1
            if per_repo[repo.name] == len(configs) * len(formats):
                print(
                    f"  {repo.name:<30} done "
                    f"({done}/{len(cells)} cells, {checked} runs)",
                    flush=True,
                )

    nd = [f for f in findings if f["kind"].startswith("nondeterministic")]
    err = [f for f in findings if f["kind"] == "run-failed"]
    print(
        f"\n{checked} runs over {len(repos)} repos x {len(configs)} configs "
        f"x {len(formats)} formats x {args.runs} runs"
    )
    print(f"  nondeterministic: {len(nd)}")
    print(f"  failed to run   : {len(err)}")
    if skipped_names:
        print(
            f"  NOT swept       : {len(skipped_names)} repo(s) — "
            f"{', '.join(skipped_names)} (excluded by cost; this sweep "
            f"covers {len(repos)} of {len(repos) + len(skipped_names)})"
        )
    for f in nd[:20]:
        print(
            f"\n  !! {f['repo']} [{f['config']}/{f['fmt']}] {f['kind']} at --jobs {f.get('jobs')}"
        )
        print(f"     {f['detail']}")
    for f in err[:5]:
        print(f"  ?? {f['repo']} [{f['config']}/{f['fmt']}] {f['detail'][:160]}")

    if args.json:
        Path(args.json).write_text(
            json.dumps(
                {
                    "checked": checked,
                    "repos": [r.name for r in repos],
                    "findings": findings,
                },
                indent=1,
            )
        )

    sys.exit(1 if nd else 0)


if __name__ == "__main__":
    main()
