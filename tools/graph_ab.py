#!/usr/bin/env python3
"""Full-corpus graph + dead-code A/B between two cgg binaries.

Usage: graph_ab.py OLD_BIN NEW_BIN OUTDIR [--repos a,b,c] [--parallel 6]
                   [--no-dead]

For every repo in $CGG_BENCH_DIR (default /storage/cgg-test_repos) plus
/storage/neurosymbolic_kb, runs each binary twice under `timeout`:
  1. default graph:  -t json -o OUT/<side>/<repo>.json
  2. dead-code:      --no-graph --dead-code --dead-code-format json
                     --dead-code-report OUT/<side>/<repo>.dead.json
then diffs them. Edges are compared as a MULTISET keyed by
(src lang, src qn, src path, dst qn, dst path, site_line, via kind);
a key present on both sides whose confidence/resolver changed is a
"changed" edge, not a drop+add.

Writes to OUTDIR:
  summary.tsv          per repo: edges, callables, unresolved, dead high,
                       files analyzed, timeouts — old vs new
  by_language.tsv      net per language (edges, callables, unresolved,
                       dead-high) from metrics.by_language + findings
  dropped.jsonl / added.jsonl / changed.jsonl
                       one row per edge, with the call-site source line
  dead_changed.jsonl   high-band dead findings that appeared/disappeared
  report.txt           human summary

Bounded: CGG_REPO_TIMEOUT (default 60) per run; a repo either side
times out on is NAMED and excluded from the diff, never averaged in.
Graph output is deterministic, so runs execute in parallel; this tool
measures NOTHING about latency (see perf_subset.sh for that).

Null test: run with OLD_BIN == NEW_BIN; every diff must be empty.
"""

from __future__ import annotations

import argparse
import collections
import concurrent.futures as cf
import json
import os
import subprocess
import sys
import time

BENCH = os.environ.get("CGG_BENCH_DIR", "/storage/cgg-test_repos")
EXTRA = ["/storage/neurosymbolic_kb"]
TIMEOUT = int(os.environ.get("CGG_REPO_TIMEOUT", "60"))


def repo_paths(only):
    paths = {}
    for name in sorted(os.listdir(BENCH)):
        p = os.path.join(BENCH, name)
        if os.path.isdir(p):
            paths[name] = p
    for p in EXTRA:
        if os.path.isdir(p):
            paths[os.path.basename(p)] = p
    if only:
        want = set(only)
        missing = want - set(paths)
        if missing:
            sys.exit(f"unknown repos: {sorted(missing)}")
        paths = {k: v for k, v in paths.items() if k in want}
    return paths


def run(bin_, args, log):
    t0 = time.time()
    try:
        r = subprocess.run(
            ["timeout", str(TIMEOUT), bin_, *args],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            text=True,
        )
        rc = r.returncode
        err = r.stderr[-2000:]
    except Exception as e:  # noqa: BLE001
        rc, err = -1, str(e)
    with open(log, "a") as f:
        f.write(f"{bin_} {' '.join(args)} rc={rc} {time.time()-t0:.1f}s\n{err}\n")
    return rc


def run_repo(side, bin_, name, path, out, dead):
    d = os.path.join(out, side)
    g = os.path.join(d, f"{name}.json")
    log = os.path.join(d, f"{name}.log")
    if os.path.exists(log):
        os.remove(log)
    rc = run(bin_, [path, "-t", "json", "-o", g, "--jobs", "4"], log)
    rcd = 0
    if dead:
        rcd = run(
            bin_,
            [path, "--no-graph", "--dead-code", "--dead-code-format", "json",
             "--dead-code-report", os.path.join(d, f"{name}.dead.json"),
             "--jobs", "4"],
            log,
        )
    return side, name, rc, rcd


_line_cache: dict = {}


def line_text(path, line):
    if not path or not line:
        return ""
    lines = _line_cache.get(path)
    if lines is None:
        try:
            with open(path, "rb") as f:
                lines = f.read().decode("utf-8", "replace").split("\n")
        except OSError:
            lines = []
        if len(_line_cache) > 2000:
            _line_cache.clear()
        _line_cache[path] = lines
    if 0 < line <= len(lines):
        return lines[line - 1].strip()[:240]
    return ""


def load_graph(p):
    with open(p) as f:
        g = json.load(f)
    files = g["files"]
    calls = g["callables"]

    def node(cid):
        c = calls.get(cid)
        if c is None:
            return ("?", cid, "", "")
        fp = files.get(c["file"], {}).get("path", "")
        return (c["language"], c["qualified_name"], fp, c.get("kind", ""))

    edges = collections.Counter()
    meta = {}
    for e in g["edges"]:
        s = node(e["src"])
        t = node(e["dst"])
        k = (s[0], s[1], s[2], t[1], t[2], e.get("site_line"),
             e["via"]["kind"])
        edges[k] += 1
        meta.setdefault(k, []).append((e["confidence"], e["resolver"]))
    m = g.get("metrics", {})
    return edges, meta, m, len(calls), len(g["unresolved"])


def load_dead(p):
    try:
        with open(p) as f:
            d = json.load(f)
    except (OSError, ValueError):
        return None
    out = {}
    for x in d.get("findings", []):
        out[(x["language"], x["qualified_name"], x.get("path", ""),
             x.get("start_line"))] = x["confidence"]
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("old")
    ap.add_argument("new")
    ap.add_argument("out")
    ap.add_argument("--repos", default="")
    ap.add_argument("--parallel", type=int, default=6)
    ap.add_argument("--no-dead", action="store_true")
    a = ap.parse_args()
    dead = not a.no_dead
    repos = repo_paths([r for r in a.repos.split(",") if r])
    for side in ("old", "new"):
        os.makedirs(os.path.join(a.out, side), exist_ok=True)
    t0 = time.time()
    status = collections.defaultdict(dict)
    jobs = []
    with cf.ThreadPoolExecutor(a.parallel) as ex:
        for name, path in repos.items():
            jobs.append(ex.submit(run_repo, "old", a.old, name, path, a.out, dead))
            jobs.append(ex.submit(run_repo, "new", a.new, name, path, a.out, dead))
        for j in cf.as_completed(jobs):
            side, name, rc, rcd = j.result()
            status[name][side] = (rc, rcd)
    elapsed = time.time() - t0

    fo = {k: open(os.path.join(a.out, f"{k}.jsonl"), "w")
          for k in ("dropped", "added", "changed", "dead_changed")}
    summary = [("repo", "edges_old", "edges_new", "dropped", "added",
                "changed", "callables_old", "callables_new", "unres_old",
                "unres_new", "files_old", "files_new", "deadhigh_old",
                "deadhigh_new", "note")]
    by_lang = collections.defaultdict(lambda: collections.Counter())
    excluded = []
    for name in sorted(repos):
        st = status[name]
        (ro, rdo), (rn, rdn) = st.get("old", (-9, -9)), st.get("new", (-9, -9))
        if ro != 0 or rn != 0:
            excluded.append(f"{name}: graph rc old={ro} new={rn}")
            continue
        eo, mo, meto, co, uo = load_graph(os.path.join(a.out, "old", f"{name}.json"))
        en, mn, metn, cn, un = load_graph(os.path.join(a.out, "new", f"{name}.json"))
        dropped = eo - en
        added = en - eo
        changed = 0
        for k in (eo.keys() & en.keys()):
            if sorted(mo[k]) != sorted(mn[k]):
                changed += 1
                fo["changed"].write(json.dumps({
                    "repo": name, "lang": k[0], "src": k[1], "dst": k[3],
                    "dst_path": k[4], "site_path": k[2], "site_line": k[5],
                    "via": k[6], "old": sorted(mo[k]), "new": sorted(mn[k]),
                    "line": line_text(k[2], k[5])}) + "\n")
        for kind, cnt, meta in (("dropped", dropped, mo), ("added", added, mn)):
            for k, n in sorted(cnt.items()):
                for i in range(n):
                    conf = meta[k][min(i, len(meta[k]) - 1)]
                    fo[kind].write(json.dumps({
                        "repo": name, "lang": k[0], "src": k[1], "dst": k[3],
                        "dst_path": k[4], "site_path": k[2], "site_line": k[5],
                        "via": k[6], "confidence": conf[0], "resolver": conf[1],
                        "line": line_text(k[2], k[5])}) + "\n")
                    by_lang[k[0]][f"edges_{kind}"] += 1
        for lang in set(meto.get("by_language", {})) | set(metn.get("by_language", {})):
            lo = meto.get("by_language", {}).get(lang, {})
            ln = metn.get("by_language", {}).get(lang, {})
            for f in ("files", "callables", "edges", "unresolved"):
                by_lang[lang][f"{f}_delta"] += ln.get(f, 0) - lo.get(f, 0)
        dho = dhn = ""
        note = ""
        if dead:
            if rdo != 0 or rdn != 0:
                note = f"dead rc old={rdo} new={rdn}"
            else:
                do = load_dead(os.path.join(a.out, "old", f"{name}.dead.json")) or {}
                dn = load_dead(os.path.join(a.out, "new", f"{name}.dead.json")) or {}
                dho = sum(1 for v in do.values() if v == "high")
                dhn = sum(1 for v in dn.values() if v == "high")
                for k in sorted(set(do) | set(dn)):
                    vo, vn = do.get(k), dn.get(k)
                    if vo != vn and "high" in (vo, vn):
                        fo["dead_changed"].write(json.dumps({
                            "repo": name, "lang": k[0], "qn": k[1],
                            "path": k[2], "line": k[3], "old": vo,
                            "new": vn}) + "\n")
                        by_lang[k[0]]["deadhigh_delta"] += (vn == "high") - (vo == "high")
        summary.append((name, sum(eo.values()), sum(en.values()),
                        sum(dropped.values()), sum(added.values()), changed,
                        co, cn, uo, un, meto.get("files_analyzed"),
                        metn.get("files_analyzed"), dho, dhn, note))
    for f in fo.values():
        f.close()
    with open(os.path.join(a.out, "summary.tsv"), "w") as f:
        for row in summary:
            f.write("\t".join(str(x) for x in row) + "\n")
    with open(os.path.join(a.out, "by_language.tsv"), "w") as f:
        cols = ["edges_dropped", "edges_added", "edges_delta",
                "callables_delta", "unresolved_delta", "files_delta",
                "deadhigh_delta"]
        f.write("lang\t" + "\t".join(cols) + "\n")
        for lang in sorted(by_lang):
            f.write(lang + "\t" + "\t".join(str(by_lang[lang][c]) for c in cols) + "\n")
    changed_repos = [r for r in summary[1:] if r[3] or r[4] or r[5]
                     or r[6] != r[7] or r[8] != r[9] or r[10] != r[11]
                     or r[12] != r[13]]
    with open(os.path.join(a.out, "report.txt"), "w") as f:
        f.write(f"old={a.old}\nnew={a.new}\nrepos={len(repos)} elapsed={elapsed:.0f}s\n")
        f.write(f"excluded (timeout/error, NOT diffed): {len(excluded)}\n")
        for x in excluded:
            f.write(f"  {x}\n")
        f.write(f"repos changed: {len(changed_repos)}\n")
        for r in changed_repos:
            f.write(f"  {r[0]}: edges {r[1]}->{r[2]} (-{r[3]} +{r[4]} ~{r[5]}) "
                    f"callables {r[6]}->{r[7]} unresolved {r[8]}->{r[9]} "
                    f"files {r[10]}->{r[11]} deadhigh {r[12]}->{r[13]} {r[14]}\n")
        f.write("by language: see by_language.tsv\n")
    print(open(os.path.join(a.out, "report.txt")).read())


if __name__ == "__main__":
    main()
