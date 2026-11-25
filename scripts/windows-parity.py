#!/usr/bin/env python3
"""Helpers for scripts/windows-parity.sh: pick, pack and compare.

The gate runs the same cgg commit on Linux and on the Windows VM over
the same byte-identical trees and requires identical output. Paths and
ids are where platforms drift (issue #14: native `\\` paths changed every
Windows id), and a path bug is per-language — include resolution,
package roots, module names — so the trees must cover every language.

  select   scan the corpus and write the smallest repo list that covers
           every language (rarely: when a language is added)
  pack     tar the listed repos deterministically, regular files only,
           minus names Windows cannot represent
  compare  diff Linux vs Windows outputs, ignoring timings; fail on any
           other difference, a `\\` in a path, or an uncovered language
"""

from __future__ import annotations

import argparse
import collections
import concurrent.futures
import json
import os
import re
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

# A language counts as covered by a repo only with this many files of it,
# at least one below the repo root, so paths with separators are exercised.
MIN_FILES = 3

# Names Windows cannot create, so a tree containing them cannot be
# byte-identical on both hosts. Leaving them out is a property of the
# test input, not of cgg.
WIN_BAD_CHARS = re.compile(r'[<>:"|?*\\\x00-\x1f]')
WIN_RESERVED = re.compile(r"^(con|prn|aux|nul|com\d|lpt\d)(\..*)?$", re.IGNORECASE)
MAX_REL_PATH = 180  # under MAX_PATH once rooted at C:\cgg-parity\corpus

# Fields that legitimately differ between hosts: timings, memory, and the
# audit's record of the command line (executable and output paths).
VOLATILE_SUFFIXES = ("_ms",)
VOLATILE_KEYS = {"peak_rss_bytes", "argv"}


def covered_languages(graph: dict) -> set[str]:
    count: collections.Counter[str] = collections.Counter()
    deep: collections.Counter[str] = collections.Counter()
    for f in graph.get("files", {}).values():
        path = f["path"]
        if path.startswith("<"):
            continue
        lang = f["language"]
        count[lang] += 1
        rel = path.split("/", 1)[1] if "/" in path else path
        if "/" in rel:
            deep[lang] += 1
    return {lang for lang in count if count[lang] >= MIN_FILES and deep[lang]}


def read_list(path: Path) -> list[str]:
    out = []
    for line in path.read_text().splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            out.append(line)
    return out


def tree_size_kb(repo: Path) -> int:
    total = 0
    for root, dirs, files in os.walk(repo):
        dirs[:] = [d for d in dirs if d != ".git"]
        for name in files:
            p = Path(root, name)
            if p.is_file() and not p.is_symlink():
                total += p.stat().st_size
    return total // 1024


# --------------------------------------------------------------------- select
def cmd_select(a: argparse.Namespace) -> int:
    corpus = Path(a.corpus)
    langs = read_list(Path(a.langs))
    repos = sorted(p.name for p in corpus.iterdir() if p.is_dir())
    tmp = Path(tempfile.mkdtemp(prefix="windows-parity-select-"))

    def scan(repo: str) -> tuple[str, set[str], int]:
        out = tmp / f"{repo}.json"
        subprocess.run(
            [a.cgg, "-q", "--jobs", "4", "-t", "json", repo, "-o", str(out)],
            cwd=corpus,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=600,
            check=False,
        )
        cov = covered_languages(json.loads(out.read_text())) if out.exists() else set()
        return repo, cov, tree_size_kb(corpus / repo)

    with concurrent.futures.ThreadPoolExecutor(max_workers=a.jobs) as pool:
        scanned = list(pool.map(scan, repos))
    cov = {r: c for r, c, _ in scanned}
    size = {r: s for r, _, s in scanned}

    # Greedy set cover, cheapest kilobyte per newly covered language.
    left, chosen = set(langs), []
    while left:
        cands = [r for r in cov if cov[r] & left]
        if not cands:
            break
        best = min(cands, key=lambda r: (size[r] / len(cov[r] & left), r))
        chosen.append(best)
        left -= cov[best]
    if left:
        print(
            f"no corpus repository covers: {', '.join(sorted(left))}", file=sys.stderr
        )
        return 1

    lines = [
        "# Corpus repositories for scripts/windows-parity.sh, chosen by",
        "# `scripts/windows-parity.py select`: the smallest set in which every",
        f"# cgg language has >= {MIN_FILES} files, at least one below the repo root.",
        "# Regenerate when a language is added; the gate fails until then.",
        "",
    ]
    width = max(len(r) for r in chosen)
    for r in chosen:
        lines.append(
            f"{r:<{width}}  # {size[r] // 1024 or '<1'} MB: {' '.join(sorted(cov[r] & set(langs)))}"
        )
    Path(a.out).write_text("\n".join(lines) + "\n")
    total = sum(size[r] for r in chosen) // 1024
    print(
        f"{len(chosen)} repositories, {total} MB, cover all {len(langs)} languages -> {a.out}"
    )
    return 0


# ----------------------------------------------------------------------- pack
def windows_safe(rel: str, seen_lower: set[str]) -> bool:
    if len(rel) > MAX_REL_PATH:
        return False
    for part in rel.split("/"):
        if (
            WIN_BAD_CHARS.search(part)
            or WIN_RESERVED.match(part)
            or part.endswith((".", " "))
        ):
            return False
    low = rel.lower()
    if low in seen_lower:  # case-insensitive collision on NTFS
        return False
    seen_lower.add(low)
    return True


def cmd_pack(a: argparse.Namespace) -> int:
    corpus = Path(a.corpus)
    repos = read_list(Path(a.repos))
    missing = [r for r in repos if not (corpus / r).is_dir()]
    if missing:
        print(f"missing from {corpus}: {', '.join(missing)}", file=sys.stderr)
        return 1
    dropped = 0
    with tarfile.open(a.out, "w", format=tarfile.PAX_FORMAT) as tar:
        for repo in repos:
            seen: set[str] = set()
            files = []
            for root, dirs, names in os.walk(corpus / repo):
                dirs[:] = sorted(d for d in dirs if d != ".git")
                for name in names:
                    p = Path(root, name)
                    if p.is_symlink() or not p.is_file():
                        continue
                    files.append(p.relative_to(corpus).as_posix())
            for rel in sorted(files):
                if not windows_safe(rel, seen):
                    dropped += 1
                    continue
                info = tar.gettarinfo(str(corpus / rel), arcname=rel)
                info.mtime, info.uid, info.gid, info.uname, info.gname = 0, 0, 0, "", ""
                info.mode = 0o644
                with open(corpus / rel, "rb") as fh:
                    tar.addfile(info, fh)
    print(
        f"packed {len(repos)} repositories ({dropped} Windows-unsafe names left out) -> {a.out}"
    )
    return 0


# -------------------------------------------------------------------- compare
def strip_volatile(v):
    if isinstance(v, dict):
        return {
            k: strip_volatile(x)
            for k, x in v.items()
            if k not in VOLATILE_KEYS and not k.endswith(VOLATILE_SUFFIXES)
        }
    if isinstance(v, list):
        return [strip_volatile(x) for x in v]
    return v


def first_differences(a, b, path="", out=None, limit=5):
    out = [] if out is None else out
    if len(out) >= limit:
        return out
    if type(a) is not type(b):
        out.append(f"{path or '/'}: {str(a)[:80]!r} != {str(b)[:80]!r}")
    elif isinstance(a, dict):
        for k in sorted(set(a) | set(b)):
            if k not in a or k not in b:
                out.append(f"{path}/{k}: only on {'windows' if k in b else 'linux'}")
            else:
                first_differences(a[k], b[k], f"{path}/{k}", out, limit)
    elif isinstance(a, list):
        if len(a) != len(b):
            out.append(f"{path}: {len(a)} items on linux, {len(b)} on windows")
        else:
            for i, (x, y) in enumerate(zip(a, b)):
                first_differences(x, y, f"{path}[{i}]", out, limit)
    elif a != b:
        out.append(f"{path}: linux {str(a)[:80]!r} != windows {str(b)[:80]!r}")
    return out


def backslash_paths(doc) -> list[str]:
    hits = []

    def walk(v, key=""):
        if isinstance(v, dict):
            for k, x in v.items():
                walk(x, k)
        elif isinstance(v, list):
            for x in v:
                walk(x, key)
        elif isinstance(v, str) and key in ("path", "file", "file_path") and "\\" in v:
            hits.append(v)

    walk(doc)
    return hits


def cmd_compare(a: argparse.Namespace) -> int:
    linux, windows = Path(a.linux), Path(a.windows)
    repos = read_list(Path(a.repos))
    langs = set(read_list(Path(a.langs)))
    failures: list[str] = []
    covered: set[str] = set()
    compared = 0
    for repo in repos:
        lin_files = sorted(p.name for p in linux.glob(f"{repo}.*json"))
        win_files = sorted(p.name for p in windows.glob(f"{repo}.*json"))
        if not lin_files:
            failures.append(f"{repo}: no Linux output")
            continue
        if lin_files != win_files:
            failures.append(
                f"{repo}: output files differ: linux {lin_files} windows {win_files}"
            )
            continue
        for name in lin_files:
            lj = json.loads((linux / name).read_text(encoding="utf-8"))
            wj = json.loads((windows / name).read_text(encoding="utf-8"))
            if name == f"{repo}.graph.json":
                covered |= covered_languages(lj)
            bad = backslash_paths(wj)
            if bad:
                failures.append(
                    f"{name}: {len(bad)} Windows path(s) with '\\', e.g. {bad[0]!r}"
                )
            ls, ws = strip_volatile(lj), strip_volatile(wj)
            if ls != ws:
                diffs = first_differences(ls, ws)
                failures.append(f"{name}: differs\n      " + "\n      ".join(diffs))
            compared += 1
    uncovered = sorted(langs - covered)
    if uncovered:
        failures.append(
            "languages with no coverage (add a corpus repo, then run "
            f"`scripts/windows-parity.py select`): {', '.join(uncovered)}"
        )
    print(
        f"compared {compared} documents from {len(repos)} repositories; "
        f"{len(covered & langs)}/{len(langs)} languages covered"
    )
    for f in failures:
        print(f"  FAIL {f}")
    return 1 if failures else 0


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("select")
    s.add_argument("--corpus", required=True)
    s.add_argument("--cgg", required=True)
    s.add_argument("--langs", required=True, help="file of plugin ids, one per line")
    s.add_argument("--out", required=True)
    s.add_argument("--jobs", type=int, default=8)
    p = sub.add_parser("pack")
    p.add_argument("--corpus", required=True)
    p.add_argument("--repos", required=True)
    p.add_argument("--out", required=True)
    c = sub.add_parser("compare")
    c.add_argument("--linux", required=True)
    c.add_argument("--windows", required=True)
    c.add_argument("--repos", required=True)
    c.add_argument("--langs", required=True)
    a = ap.parse_args()
    return {"select": cmd_select, "pack": cmd_pack, "compare": cmd_compare}[a.cmd](a)


if __name__ == "__main__":
    sys.exit(main())
