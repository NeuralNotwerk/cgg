#!/usr/bin/env python3
"""Framework application coverage — one real app per supported framework.

Every framework rule in `crates/cgg-core/src/frameworks/rules.rs` claims
that cgg can recognise a hand-off shape. This script checks that claim
against a real application that actually uses the framework, rather than
against a hand-written fixture that was written to pass.

It reports two different numbers on purpose:

  registrations  what the coverage table calls "entries" — how many
                 hand-offs the framework resolver matched.
  entry nodes    how many distinct `<framework-entry>` nodes those
                 collapsed into in the emitted graph.

They are not the same number, and the gap is the point. A framework
whose routes live in a config file (Django, Rails) has no route string
to key a node on, so every handler named `get` collapses onto one node.
Reporting only the first number would make that look like full coverage.

Two gates, both fatal:

  * a framework declared by an app in the manifest that does not fire
    on that app — the manifest is stale, or a rule regressed.
  * a rule id in rules.rs that no app in the manifest claims — the
    framework ships with no real-world evidence behind it.

Usage:
    scripts/framework-coverage.py [--clone] [--app NAME] [--json OUT]

Reads the `APPS=( … )` manifest from `scripts/benchmark.sh`, so the repo
corpus has a single source of truth. Clones into $CGG_BENCH_DIR
(default /storage/cgg-test_repos); `--clone` fetches anything missing.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
BENCH_SH = REPO_ROOT / "scripts/benchmark.sh"
RULES_RS = REPO_ROOT / "crates/cgg-core/src/frameworks/rules.rs"
CGG = Path(os.environ.get("CGG_BIN", REPO_ROOT / "target/release/cgg"))
REPOS_DIR = Path(os.environ.get("CGG_BENCH_DIR", "/storage/cgg-test_repos"))

# The sentinel prefix `synthesize_entry_nodes` mints nodes under. Kept in
# sync with cgg_core::frameworks::FRAMEWORK_ENTRY_SENTINEL.
ENTRY_SENTINEL = "<framework-entry>"


def fail(msg: str) -> None:
    print(f"framework-coverage: FAIL: {msg}", file=sys.stderr)
    sys.exit(1)


def rule_ids() -> dict[str, set[str]]:
    """Every (id → languages) pair declared in rules.rs."""
    text = RULES_RS.read_text()
    pairs = re.findall(
        r'id:\s*"([^"]+)",\s*\n\s*language:\s*"([^"]+)"', text
    )
    if not pairs:
        fail(f"could not parse any framework rules out of {RULES_RS}")
    out: dict[str, set[str]] = {}
    for fid, lang in pairs:
        out.setdefault(fid, set()).add(lang)
    return out


def manifest() -> list[dict]:
    """Parse `APPS=( … )` out of benchmark.sh: name|url|frameworks."""
    text = BENCH_SH.read_text()
    m = re.search(r"APPS=\(\s*\n(.*?)\n\)", text, re.DOTALL)
    if not m:
        fail(f"could not locate APPS=( ... ) array in {BENCH_SH}")
    apps = []
    for line in m.group(1).splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        entry = line.strip('"').split("|")
        if len(entry) < 3:
            fail(f"malformed APPS entry: {line}")
        claims = [f for f in entry[2].split(",") if f]
        apps.append(
            {
                "name": entry[0],
                "url": entry[1],
                # Must be enumerated: appear in `recognised` with entries.
                "frameworks": [f for f in claims if not f.startswith("~")],
                # Must be detected: `recognised` or `seen, no rules`. The
                # framework is present in the app and cgg says so, but
                # produces no entry points from it.
                "detect_only": [f[1:] for f in claims if f.startswith("~")],
            }
        )
    return apps


def clone_missing(apps: list[dict]) -> None:
    REPOS_DIR.mkdir(parents=True, exist_ok=True)
    for app in apps:
        dest = REPOS_DIR / app["name"]
        if dest.exists():
            continue
        print(f"  cloning {app['name']}…", file=sys.stderr)
        rc = subprocess.run(
            ["git", "clone", "--depth", "1", "--single-branch", "-q",
             app["url"], str(dest)],
            capture_output=True,
        )
        if rc.returncode != 0:
            print(f"  FAILED {app['name']}: {app['url']}", file=sys.stderr)


def analyze(path: Path) -> dict | None:
    """One cgg run; returns graph + coverage metrics."""
    tmp = Path(tempfile.mkdtemp(prefix="cgg-fw-"))
    try:
        out = tmp / "g.json"
        start = time.monotonic()
        proc = subprocess.run(
            [str(CGG), str(path), "-t", "json", "-o", str(out),
             "--framework-coverage", "--no-update-check"],
            capture_output=True,
            timeout=900,
        )
        elapsed = (time.monotonic() - start) * 1000
        if not out.exists():
            print(
                f"  cgg produced no output for {path.name}: "
                f"{proc.stderr.decode(errors='replace')[:300]}",
                file=sys.stderr,
            )
            return None

        graph = json.loads(out.read_text())
        audit_path = out.with_suffix(".json.audit.json")
        recognised: list[dict] = []
        seen_no_rules: list[dict] = []
        if audit_path.exists():
            for ev in json.loads(audit_path.read_text()):
                if isinstance(ev, dict) and ev.get("event") == "framework_coverage":
                    cov = ev["coverage"]
                    recognised = cov.get("recognised", [])
                    seen_no_rules = cov.get("seen_no_rules", [])

        callables = graph.get("callables", {})
        entry_nodes = [
            c for c in callables.values()
            if c.get("qualified_name", "").startswith(ENTRY_SENTINEL)
        ]
        entry_edges = sum(
            1 for e in graph.get("edges", [])
            if isinstance(e.get("via"), dict)
            and e["via"].get("kind") == "framework_entry"
        )
        kinds: dict[str, int] = {}
        for c in entry_nodes:
            k = c.get("framework_entry") or "?"
            kinds[k] = kinds.get(k, 0) + 1

        return {
            "nodes": len(callables),
            "edges": len(graph.get("edges", [])),
            "entry_nodes": len(entry_nodes),
            "entry_edges": entry_edges,
            "registrations": sum(r.get("entries", 0) for r in recognised),
            "kinds": kinds,
            "recognised": recognised,
            "seen_no_rules": seen_no_rules,
            "ms": elapsed,
        }
    except subprocess.TimeoutExpired:
        print(f"  TIMEOUT analyzing {path.name}", file=sys.stderr)
        return None
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--clone", action="store_true",
                    help="clone any app missing from the corpus first")
    ap.add_argument("--app", help="only run this app")
    ap.add_argument("--json", help="write the metrics table as JSON here")
    args = ap.parse_args()

    if not CGG.exists():
        fail(f"{CGG} not found — run `cargo build --release -p cgg`")

    declared = rule_ids()
    apps = manifest()
    if args.clone:
        clone_missing(apps)
    if args.app:
        apps = [a for a in apps if a["name"] == args.app]
        if not apps:
            fail(f"no app named {args.app} in the manifest")

    header = (
        f"{'Application':<26} │ {'nodes':>7} │ {'edges':>7} │ "
        f"{'entry':>5} │ {'regs':>5} │ {'time':>8} │ frameworks"
    )
    print(header)
    print("─" * 26 + "─┼─" + "─" * 7 + "─┼─" + "─" * 7 + "─┼─"
          + "─" * 5 + "─┼─" + "─" * 5 + "─┼─" + "─" * 8 + "─┼─" + "─" * 30)

    fired_ids: set[str] = set()
    detected_ids: set[str] = set()
    fired_pairs: set[tuple[str, str]] = set()
    rows = []
    missing_apps: list[str] = []
    stale: list[str] = []
    improved: list[str] = []
    gaps: dict[tuple[str, str], str] = {}

    for app in apps:
        path = REPOS_DIR / app["name"]
        if not path.is_dir():
            missing_apps.append(app["name"])
            continue
        res = analyze(path)
        if res is None:
            stale.append(f"{app['name']}: analysis failed")
            continue

        got = {r["id"] for r in res["recognised"]}
        detected = got | {s["id"] for s in res["seen_no_rules"]}
        fired_ids |= got
        detected_ids |= detected
        fired_pairs |= {(r["id"], r["language"]) for r in res["recognised"]}
        fired_pairs |= {(s["id"], s["language"]) for s in res["seen_no_rules"]}

        absent = [f for f in app["frameworks"] if f not in got]
        if absent:
            stale.append(
                f"{app['name']}: declares {','.join(absent)} but they did not fire"
            )
        # A `~` claim still has to show up — the whole point is that the
        # gap is *disclosed*, not that it is absent.
        undisclosed = [f for f in app["detect_only"] if f not in detected]
        if undisclosed:
            stale.append(
                f"{app['name']}: declares ~{',~'.join(undisclosed)} but cgg "
                f"neither enumerated nor disclosed them"
            )
        # A gap marker that has been fixed should be promoted, or the
        # manifest understates what cgg can do.
        promoted = [f for f in app["detect_only"] if f in got]
        if promoted:
            improved.append(
                f"{app['name']}: ~{',~'.join(promoted)} now enumerates entries "
                f"— drop the ~ prefix in scripts/benchmark.sh"
            )
        for s in res["seen_no_rules"]:
            reason = s.get("reason", "")
            gaps.setdefault((s["id"], s["language"]), reason)

        print(
            f"{app['name']:<26} │ {res['nodes']:>7,} │ {res['edges']:>7,} │ "
            f"{res['entry_nodes']:>5} │ {res['registrations']:>5} │ "
            f"{res['ms']:>6.0f}ms │ {','.join(sorted(got)) or '—'}"
        )
        rows.append({"app": app["name"], **{k: v for k, v in res.items()
                                            if k != "recognised"}})

    print()
    # The registrations-vs-nodes gap, stated rather than buried. A
    # framework with no route string to key on collapses every handler
    # of the same method name onto one node.
    collapsed = [r for r in rows
                 if r["registrations"] > 0
                 and r["entry_nodes"] < r["registrations"] / 2]
    if collapsed:
        print("Entry-node collapse (registrations ≫ distinct nodes — handlers")
        print("share a node because the framework carries no route string):")
        for r in collapsed:
            print(f"  {r['app']:<26} {r['registrations']:>4} registrations "
                  f"→ {r['entry_nodes']:>3} nodes")
        print()

    if gaps:
        print("Detected but not enumerated — cgg says so in the coverage")
        print("table rather than reporting zero entries as if complete:")
        for (fid, lang), reason in sorted(gaps.items()):
            print(f"  {fid:<18} {lang:<12} {reason}")
        print()

    claimed_any = {f for a in apps
                   for f in a["frameworks"] + a["detect_only"]}
    unclaimed = sorted(set(declared) - claimed_any)
    never_seen = sorted(set(declared) - detected_ids)
    pair_gaps = sorted(
        {(i, lang) for i, langs in declared.items() for lang in langs}
        - fired_pairs
    )

    total_rules = sum(len(v) for v in declared.values())
    print(f"Frameworks with rules       : {len(declared)} "
          f"({total_rules} id×language rules)")
    print(f"Enumerating entries         : {len(fired_ids)}")
    print(f"Detected, entries not built : {len(detected_ids - fired_ids)}"
          f" — {', '.join(sorted(detected_ids - fired_ids)) or '—'}")
    if pair_gaps:
        print(f"id×language never exercised : {len(pair_gaps)} — "
              + ", ".join(f"{i}/{l}" for i, l in pair_gaps))

    ok = True
    if missing_apps:
        print(f"\nNot cloned ({len(missing_apps)}): {', '.join(missing_apps)}")
        print("  run with --clone to fetch them")
        ok = False
    if stale:
        print(f"\nManifest disagrees with reality ({len(stale)}):")
        for s in stale:
            print(f"  {s}")
        ok = False
    if unclaimed:
        print(f"\nFramework rules with no application ({len(unclaimed)}):")
        print(f"  {', '.join(unclaimed)}")
        print("  every rule in rules.rs needs an APPS entry that exercises it")
        ok = False
    if never_seen and not unclaimed:
        print(f"\nClaimed but never even detected ({len(never_seen)}): "
              f"{', '.join(never_seen)}")
        ok = False
    if improved:
        # Not fatal: cgg got better than the manifest admits.
        print(f"\nGap markers now stale ({len(improved)}):")
        for i in improved:
            print(f"  {i}")

    if args.json:
        Path(args.json).write_text(json.dumps(
            {"apps": rows, "fired": sorted(fired_ids),
             "detected": sorted(detected_ids),
             "detect_only": sorted(detected_ids - fired_ids),
             "gaps": {f"{k[0]}/{k[1]}": v for k, v in sorted(gaps.items())},
             "unclaimed": unclaimed,
             "pair_gaps": [list(p) for p in pair_gaps]}, indent=2))

    if not ok:
        sys.exit(1)
    print(f"\nEvery framework rule has an application behind it: "
          f"{len(fired_ids)} enumerate entry points, "
          f"{len(detected_ids - fired_ids)} are detected with the gap disclosed.")


if __name__ == "__main__":
    main()
