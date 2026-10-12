#!/usr/bin/env python3
"""probe.py DIR NAME... : edges (plain graph default / --reference-edges) + dead-code band (default / --reference-edges)."""
import json, subprocess, sys, os, re
CGG = "/storage/cgg/target/release/cgg"
d = sys.argv[1]; names = sys.argv[2:]
extra = os.environ.get("EXTRA", "").split()
def via(e):
    v = e.get("via")
    return v.get("kind") if isinstance(v, dict) else str(v)
def graph(tag, flags):
    g = f"{d}/g_{tag}.json"
    r = subprocess.run(["timeout", "60", CGG, d, "-t", "json", "-o", g] + flags + extra, capture_output=True, text=True)
    if r.returncode not in (0, 3): print("RC", tag, r.returncode, r.stderr[-600:])
    return json.load(open(g))
def dead(tag, flags):
    g = f"{d}/gd_{tag}.json"; dc = f"{d}/dc_{tag}.json"
    r = subprocess.run(["timeout", "60", CGG, d, "-t", "json", "-o", g, "--dead-code", "--dead-code-format", "json",
           "--dead-code-report", dc, "--dead-code-confidence", "low"] + flags + extra, capture_output=True, text=True)
    if r.returncode not in (0, 3): print("RC", tag, r.returncode, r.stderr[-600:])
    subprocess.run(["timeout", "60", CGG, d, "-t", "json", "-o", f"{d}/gd_txt.json", "--dead-code", "--dead-code-format", "text",
           "--dead-code-report", f"{d}/dc_{tag}.txt"] + flags + extra, capture_output=True, text=True)
    return json.load(open(g)), json.load(open(dc))
def edges_into(G, nm):
    ids = {cid for cid, n in G["callables"].items() if n.get("simple_name") == nm}
    if not ids: return None
    ins = [e for e in G["edges"] if e["dst"] in ids]
    return ",".join(sorted({f'{via(e)}/{e.get("resolver")}' for e in ins})) or "-"
def band(D, G, nm):
    ids = {cid for cid, n in G["callables"].items() if n.get("simple_name") == nm}
    if not ids: return "NO-NODE", []
    fs = [f for f in D["findings"] if f["simple_name"] == nm]
    if not fs:
        rooted = [r for r in D["roots"] if r["qualified_name"].split(".")[-1].split("::")[-1] == nm or r["qualified_name"].endswith(nm)]
        return ("live(root:" + rooted[0]["rule"] + ")") if rooted else "live", []
    evs = set()
    for f in fs:
        for e in f["evidence"]:
            k = e["kind"]
            if k == "name-collides-with-screened-site": k += ":" + e.get("screen", "")
            evs.add(k)
    return ";".join(f'{f["confidence"]}' for f in fs), sorted(evs)
G0 = graph("plain", []); G1 = graph("plainref", ["--reference-edges"])
GD0, D0 = dead("def", []); GD1, D1 = dead("ref", ["--reference-edges"])
for nm in names:
    b0, ev = band(D0, GD0, nm); b1, _ = band(D1, GD1, nm)
    print(f"{nm:26} plain={str(edges_into(G0,nm)):28} +ref={str(edges_into(G1,nm)):28} dead-mode-edges={str(edges_into(GD0,nm)):24} band={b0:14} band+ref={b1}")
    if ev: print(f"{'':26}   ev={','.join(e for e in ev if e not in ('no-incoming-edges','language-signals-complete'))}")
