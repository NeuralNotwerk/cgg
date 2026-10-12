#!/usr/bin/env python3
"""precise.py LANG OUTDIR REPO: from OUTDIR/dc.json, count HIGH findings matched by precise value-position patterns."""
import json, sys, os, re, collections
lang, out, repo = sys.argv[1:4]
D = json.load(open(f"{out}/dc.json"))
highs = [f for f in D["findings"] if f["confidence"] == "high" and f["language"] == lang]
def read(p):
    try: return open(p, encoding="utf-8", errors="replace").read()
    except Exception: return ""
EXT = {"java": ".java", "csharp": ".cs", "python": ".py"}[lang]
allsrc = {}
for root, _, files in os.walk(repo):
    if "/.git" in root: continue
    for fn in files:
        if fn.endswith(EXT): allsrc[os.path.join(root, fn)] = read(os.path.join(root, fn))
JOINED = "\n".join(allsrc.values())
cnt = collections.Counter(); ex = collections.defaultdict(list)
for f in highs:
    nm = re.escape(f["simple_name"]); qn = f["qualified_name"]
    owner = qn.split(".")[-2] if "." in qn else ""
    fpath = os.path.normpath(os.path.join(repo, f["path"])) if not os.path.isabs(f["path"]) else f["path"]
    same = read(fpath)
    hit = None
    if lang == "java":
        if re.search(r"\b(this|super)\s*::\s*" + nm + r"\b", same) or (owner and re.search(r"\b" + re.escape(owner) + r"\s*::\s*" + nm + r"\b", same)):
            hit = "method-ref same file"
        elif owner and re.search(r"\b" + re.escape(owner) + r"\s*::\s*" + nm + r"\b", JOINED):
            hit = "method-ref Owner::m other file"
        elif re.search(r'@MethodSource\(\s*(value\s*=\s*)?\{?\s*"[^"]*\b' + nm + r'"', same) or re.search(r'dataProvider\s*=\s*"' + nm + '"', same):
            hit = "MethodSource/dataProvider string"
        elif re.search(r'getDeclaredMethod\(\s*"' + nm + r'"|getMethod\(\s*"' + nm + r'"', JOINED):
            hit = "reflection getMethod string"
        elif f["simple_name"] in ("readObject", "writeObject", "readResolve", "writeReplace", "readObjectNoData"):
            hit = "serialization hook"
    elif lang == "csharp":
        sig = f.get("signature_hint", "")
        if re.search(r"\bpublic\b", sig) and not re.search(r"\bpublic\b", sig.split("(")[0]):
            hit = "VIS-BUG public w/ attr args"
        elif re.search(r"nameof\(\s*(\w+\.)*" + nm + r"\s*\)", same):
            hit = "nameof (same file)"
        elif re.search(r"[+-]=\s*(new\s+[\w.<>]+\(\s*)?(this\.)?" + nm + r"\s*\)?\s*;", same):
            hit = "event += method group"
        elif re.search(r"[(,]\s*(this\.)?" + nm + r"\s*[,)]", same):
            hit = "method group argument"
        elif re.search(r"=\s*(this\.)?" + nm + r"\s*;", same):
            hit = "method group assignment"
        elif re.search(r'TestCaseSource\(\s*"' + nm + '"|MemberData\(\s*"' + nm + '"|ValueSource\(\s*"' + nm + '"', same):
            hit = "test source string"
        elif re.search(r'GetMethod\(\s*"' + nm + '"', JOINED):
            hit = "reflection GetMethod string"
    elif lang == "python":
        if re.search(r"^\s*@\s*(\w+\.)*" + nm + r"\b", same, re.M) or re.search(r"^\s*@\s*(\w+\.)*" + nm + r"\b", JOINED, re.M):
            hit = "decorator usage"
        elif re.search(r"^\s*(return|yield)\b[^\n#]*\b" + nm + r"\b(?!\s*\()", same, re.M):
            hit = "return/yield value"
        elif re.search(r"(getattr|hasattr|setattr)\([^)\n]*['\"]" + nm + r"['\"]", JOINED):
            hit = "getattr string"
        elif re.search(r"=\s*\w+\s*=\s*[\w\s=]*\b" + nm + r"\s*$", same, re.M):
            hit = "chained assignment"
        elif re.search(r"def\s+\w+\([^)]*=\s*" + nm + r"\b", same):
            hit = "default arg"
        elif re.search(r"\bfor\b[^\n]*:[^\n]*\b" + nm + r"\b(?!\s*\()|\bin\s*[\[\(][^\n]*\b" + nm + r"\b(?!\s*\()", same):
            hit = "comprehension/for-iterable"
    if hit:
        cnt[hit] += 1; ex[hit].append(f'{qn} @ {f["path"]}:{f["start_line"]}')
print(f"{repo}: HIGH({lang})={len(highs)}  matched={sum(cnt.values())}")
for k, v in cnt.most_common(): print(f"   {k:34} {v}   e.g. {ex[k][0]}")
json.dump(ex, open(f"{out}/precise.json", "w"), indent=1)
