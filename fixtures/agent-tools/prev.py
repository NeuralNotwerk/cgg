#!/usr/bin/env python3
"""prev.py LANG REPO_PATH OUTDIR [--include-tests]: run cgg --dead-code, classify each HIGH finding by textual mentions."""
import json, subprocess, sys, os, re, collections
CGG = "/storage/cgg/target/release/cgg"
lang, repo, out = sys.argv[1], sys.argv[2], sys.argv[3]
extra = sys.argv[4:]
os.makedirs(out, exist_ok=True)
EXT = {"python": (".py",), "java": (".java",), "csharp": (".cs",)}[lang]
dc = f"{out}/dc.json"
r = subprocess.run(["timeout", "60", CGG, repo, "--dead-code", "--no-graph", "--dead-code-format", "json",
                    "--dead-code-report", dc, "--dead-code-confidence", "low"] + extra, capture_output=True, text=True)
if r.returncode not in (0, 3):
    print("RC", r.returncode, r.stderr[-500:]); sys.exit(1)
D = json.load(open(dc))
highs = [f for f in D["findings"] if f["confidence"] == "high" and f["language"] == lang]
allh = [f for f in D["findings"] if f["confidence"] == "high"]
# load sources
src = {}
for root, _, files in os.walk(repo):
    if "/.git" in root: continue
    for fn in files:
        if fn.endswith(EXT):
            p = os.path.join(root, fn)
            try: src[p] = open(p, encoding="utf-8", errors="replace").read().splitlines()
            except Exception: pass
def classify(line, nm):
    n = re.escape(nm)
    if lang == "java":
        if re.search(r"::\s*" + n + r"\b", line): return "method-ref(::)"
        if re.search(r'"' + n + r'"', line): return "string-literal"
    if lang == "csharp":
        if re.search(r"nameof\(\s*(\w+\.)*" + n + r"\s*\)", line): return "nameof"
        if re.search(r"[+-]=\s*(new\s+[\w.<>]+\(\s*)?(this\.)?" + n + r"\b(?!\s*\()", line): return "event+=/-="
        if re.search(r'"' + n + r'"', line): return "string-literal"
        if re.search(r"[(,]\s*(this\.|\w+\.)?" + n + r"\s*[,)]", line) and not re.search(n + r"\s*\(", line): return "method-group-arg"
        if re.search(r"=\s*(this\.)?" + n + r"\s*;", line): return "method-group-assign"
    if lang == "python":
        if re.search(r"^\s*@\s*(\w+\.)*" + n + r"\b", line): return "decorator"
        if re.search(r"(getattr|setattr|hasattr)\s*\([^)]*['\"]" + n + r"['\"]", line): return "getattr-string"
        if re.search(r"^\s*(return|yield)\b.*\b" + n + r"\b(?!\s*\()", line): return "return/yield-value"
        if re.search(r"['\"]" + n + r"['\"]", line): return "string-literal"
        if re.search(r"\bdef\s+\w+\(.*=\s*" + n + r"\b", line): return "default-arg"
        if re.search(r"\b" + n + r"\b(?!\s*\()", line): return "other-value"
    if re.search(r"\b" + n + r"\s*\(", line): return "call-text"
    return "other"
cats = collections.Counter(); per = collections.defaultdict(list); unmentioned = 0
for f in highs:
    nm = f["simple_name"]; path = f["path"]
    hits = []
    pat = re.compile(r"\b" + re.escape(nm) + r"\b")
    for p, lines in src.items():
        for i, l in enumerate(lines, 1):
            if pat.search(l):
                ap = os.path.normpath(p); fp = os.path.normpath(os.path.join(repo, path)) if not os.path.isabs(path) else os.path.normpath(path)
                if (ap.endswith(os.path.normpath(path).lstrip("./")) and f["start_line"] <= i <= f["start_line"] + 1):
                    continue  # definition line
                hits.append((p, i, l.strip()))
    if not hits:
        unmentioned += 1; continue
    cs = sorted({classify(h[2], nm) for h in hits})
    key = cs[0] if len(cs) == 1 else "+".join(cs)
    # pick the most specific category
    pri = ["method-ref(::)", "nameof", "event+=/-=", "method-group-arg", "method-group-assign", "decorator", "getattr-string", "return/yield-value", "default-arg", "string-literal", "other-value", "call-text", "other"]
    best = min(cs, key=lambda c: pri.index(c))
    cats[best] += 1
    per[best].append((f["qualified_name"], f"{hits[0][0].replace(repo,'')}:{hits[0][1]}", hits[0][2][:110]))
vis_bug = 0
if lang == "csharp":
    for f in highs:
        sig = f.get("signature_hint", "")
        head = sig.split("(")[0]
        if re.search(r"\bpublic\b", sig) and not re.search(r"\bpublic\b", head):
            vis_bug += 1
            per["VIS-BUG(public-with-attr-args)"].append((f["qualified_name"], f["path"], sig[:110]))
print(f"{repo}: high({lang})={len(highs)} high(all)={len(allh)} no-textual-mention={unmentioned} mentioned={sum(cats.values())} csharp_vis_bug={vis_bug}")
for c, n in cats.most_common(): print(f"  {c:22} {n}")
json.dump(per, open(f"{out}/per.json", "w"), indent=1)
