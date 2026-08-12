#!/usr/bin/env python3
"""Rebuild the APPS claim lists in benchmark.sh from measured reality.

The manifest asserts which frameworks each corpus application exercises.
Hand-maintaining that across hundreds of rules is how a manifest drifts
from the code it is supposed to police, so this derives it: run cgg over
every app, record what actually fired, and write the claims back.

Three states come out of a run, and the distinction is the whole point:

  framework    the rule enumerated entries on this app.
  ~framework   the rule was detected on this app but enumerated nothing —
               cgg said so in the coverage table's "seen, no rules".
  (unverified) no app in the corpus exercised the rule at all. These go
               to APPS_UNVERIFIED with a reason, because "no real-world
               evidence" is a fact worth stating rather than hiding.

Detect-only rules — no matchers, a `gap` string — are exempt from
needing an app at all, and are not written here.

This rewrites a hand-reviewed, load-bearing manifest in place, so it
refuses to write whenever any app failed to measure — a failed app is
indistinguishable from an app that exercises nothing, and writing one as
the other silently destroys that app's claims. `--dry-run` inspects
without writing; `--allow-partial` accepts the loss explicitly.

Usage: scripts/sync-app-manifest.py [--dry-run] [--allow-partial]
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
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BENCH = ROOT / "scripts/benchmark.sh"
RULES = ROOT / "crates/cgg-core/src/frameworks/rules.rs"
CGG = Path(os.environ.get("CGG_BIN", ROOT / "target/release/cgg"))
CORPUS = Path(os.environ.get("CGG_BENCH_DIR", "/storage/cgg-test_repos"))


def enumerating_ids() -> set[str]:
    text = RULES.read_text()
    out = set()
    for block in re.split(r"(?=RuleSpec\s*\{)", text):
        m = re.search(r'id:\s*"([^"]+)"', block)
        if not m:
            continue
        if any(
            re.search(rf"{f}:\s*(&\[|HTTP_VERBS)", block)
            for f in ("attributes", "registrars", "base_types", "methods")
        ):
            out.add(m.group(1))
    return out


def apps() -> list[tuple[str, str]]:
    text = BENCH.read_text()
    m = re.search(r"APPS=\(\s*\n(.*?)\n\)", text, re.DOTALL)
    if not m:
        sys.exit("no APPS array")
    out = []
    for line in m.group(1).splitlines():
        line = line.strip().strip('"')
        if not line or line.startswith("#"):
            continue
        p = line.split("|")
        if len(p) >= 3:
            out.append((p[0], p[1]))
    return out


def measure(name: str) -> tuple[set[str], set[str], str | None]:
    """(enumerated, detected-only, failure reason or None) for one app.

    The third element is what keeps this script from destroying the
    manifest. Every failure path below — missing clone, cgg error,
    missing audit, timeout — produces the *same* empty result as an app
    that genuinely matches no framework. Without a distinct failure
    signal the caller cannot tell "this app exercises nothing" from
    "we failed to look", and would write the former when it measured
    the latter.
    """
    path = CORPUS / name
    if not path.is_dir():
        return set(), set(), f"not cloned under {CORPUS}"
    tmp = Path(tempfile.mkdtemp(prefix="cgg-sync-"))
    try:
        out = tmp / "g.json"
        proc = subprocess.run(
            [
                str(CGG),
                str(path),
                "-t",
                "json",
                "-o",
                str(out),
                "--framework-coverage",
                "--no-update-check",
            ],
            capture_output=True,
            timeout=1200,
            check=False,
        )
        if proc.returncode != 0:
            tail = proc.stderr.decode("utf-8", "replace").strip().splitlines()
            why = tail[-1] if tail else ""
            return set(), set(), f"cgg exited {proc.returncode}: {why}"
        audit = out.with_suffix(".json.audit.json")
        if not audit.exists():
            return set(), set(), "cgg wrote no audit sidecar"
        for ev in json.loads(audit.read_text()):
            if isinstance(ev, dict) and ev.get("event") == "framework_coverage":
                cov = ev["coverage"]
                enum = {
                    r["id"]
                    for r in cov.get("recognised", [])
                    if r.get("entries", 0) > 0
                }
                seen = {s["id"] for s in cov.get("seen_no_rules", [])}
                return enum, seen - enum, None
        return set(), set(), "audit has no framework_coverage event"
    except subprocess.TimeoutExpired:
        return set(), set(), "timed out after 1200s"
    except (OSError, ValueError, KeyError) as e:
        return set(), set(), f"{type(e).__name__}: {e}"
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument(
        "--allow-partial",
        action="store_true",
        help="write even though some apps failed to measure. Their claims "
        "WILL be emptied and their rules moved to APPS_UNVERIFIED. "
        "Only pass this if you have read the failure list and accept it.",
    )
    args = ap.parse_args()

    if not CGG.exists():
        sys.exit(f"{CGG} not found - run `cargo build --release -p cgg` first")

    need = enumerating_ids()
    rows = apps()
    claims: dict[str, list[str]] = {}
    covered: set[str] = set()

    failures: dict[str, str] = {}
    for name, _url in rows:
        enum, seen, err = measure(name)
        if err:
            failures[name] = err
            print(f"  {name:<30} FAILED: {err}", file=sys.stderr)
            continue
        mine = sorted(enum & need) + sorted(f"~{s}" for s in (seen & need))
        claims[name] = mine
        covered |= (enum | seen) & need
        print(
            f"  {name:<30} {len(enum & need):>3} enumerated, "
            f"{len(seen & need):>3} detected-only",
            file=sys.stderr,
        )

    orphans = sorted(need - covered)
    print(
        f"\n{len(covered)} of {len(need)} enumerating rules exercised; "
        f"{len(orphans)} with no application",
        file=sys.stderr,
    )

    # --dry-run is inspection only and never writes, so it must run
    # BEFORE the refusal guards below — otherwise the very failure the
    # guards are complaining about is the thing you cannot inspect, and
    # the guard's own "pass --dry-run" advice is a dead end.
    if args.dry_run:
        print(
            json.dumps(
                {"claims": claims, "orphans": orphans, "failures": failures},
                indent=1,
            )
        )
        return

    # Refuse to write a manifest built from a failed measurement.
    # A failed app measures identically to an app that exercises nothing,
    # so writing one as the other silently empties a hand-reviewed,
    # load-bearing claim list and moves its rules to APPS_UNVERIFIED —
    # leaving the gates passing on a lie. A PARTIAL failure destroys
    # exactly the apps that failed, which is the common case (one repo
    # not cloned, one timeout under load) and the easiest to miss.
    if failures:
        detail = "\n".join(f"    {n}: {r}" for n, r in sorted(failures.items()))
        if not args.allow_partial:
            sys.exit(
                f"refusing to write: {len(failures)} of {len(rows)} apps "
                f"failed to measure.\n{detail}\n"
                f"  Writing now would empty those apps' claims and move "
                f"their rules to APPS_UNVERIFIED.\n"
                f"  Fix the corpus/binary and re-run, use --dry-run to "
                f"inspect, or --allow-partial to accept the loss."
            )
        print(
            f"\n--allow-partial: accepting {len(failures)} failed apps; "
            f"their claims will be emptied:\n{detail}",
            file=sys.stderr,
        )

    # Backstop for the case where every app measured "successfully" but
    # matched nothing — a plausible symptom of a rule table that failed
    # to load rather than a corpus that genuinely exercises no framework.
    measured = sum(1 for v in claims.values() if v)
    if measured == 0:
        sys.exit(
            "refusing to write: no app produced any framework match. "
            "That means the measurement failed, not that the manifest is "
            "wrong. Check the binary and $CGG_BENCH_DIR."
        )

    text = BENCH.read_text()
    lines = []
    for name, url in rows:
        c = ",".join(claims.get(name) or [])
        lines.append(f'    "{name}|{url}|{c}"')
    text, n_apps = re.subn(
        r"(APPS=\(\s*\n).*?(\n\))",
        lambda m: m.group(1) + "\n".join(lines) + m.group(2),
        text,
        count=1,
        flags=re.DOTALL,
    )
    if n_apps != 1:
        sys.exit("refusing to write: APPS=( ... ) block not found in benchmark.sh")

    # The reason has to survive a reader checking it, and the old one
    # did not. It claimed a fixture: there is no per-rule fixture for
    # these — crates/cgg/tests/frameworks.rs tests the six hand-off
    # shapes, and tests/detect_prefixes.rs tests *detection*, that a
    # rule's first `detect` prefix can fire, explicitly not enumeration.
    # It also claimed the corpus: this loop only measures APPS, so a
    # rule that enumerates on a REPOS language repo lands here anyway.
    # Eight currently do. Scope the claim to what was measured and let
    # the hand-written note above APPS_UNVERIFIED carry the rest.
    un = [
        f'    "{o}|no APPS application exercises this rule; '
        f"tests/detect_prefixes.rs proves its detect prefix can fire, "
        f"nothing in APPS proves it enumerates "
        f'— see the language-corpus note above"'
        for o in orphans
    ]
    text, n_un = re.subn(
        r"(APPS_UNVERIFIED=\(\s*\n).*?(\n\))",
        lambda m: m.group(1) + "\n".join(un) + m.group(2),
        text,
        count=1,
        flags=re.DOTALL,
    )
    if n_un != 1:
        sys.exit(
            "refusing to write: APPS_UNVERIFIED=( ... ) block not found in "
            "benchmark.sh (APPS would have been rewritten without it)"
        )
    BENCH.write_text(text)
    print("benchmark.sh updated", file=sys.stderr)


if __name__ == "__main__":
    main()
