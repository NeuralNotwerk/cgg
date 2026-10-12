import json, sys, re, os, collections
out, repo = sys.argv[1:3]
D = json.load(open(f"{out}/dc.json"))
hs = [f for f in D["findings"] if f["confidence"] == "high" and f["language"] == "java"]
c = collections.Counter(); cache = {}
for f in hs:
    sig = f.get("signature_hint", ""); head = sig.split("(")[0]
    p = f["path"] if os.path.isabs(f["path"]) else os.path.join(repo, f["path"])
    if p not in cache:
        try: cache[p] = open(p, errors="replace").read()
        except Exception: cache[p] = ""
    owner = f["qualified_name"].split(".")[-2] if f["qualified_name"].count(".") else ""
    if re.search(r"\bpublic\b", sig) and not re.search(r"\bpublic\b", head): c["public-after-annotation-args"] += 1
    elif not re.search(r"\b(public|private|protected)\b", sig) and owner and re.search(r"\b(interface|@interface)\s+" + re.escape(owner) + r"\b", cache[p]): c["no-modifier-in-interface(implicitly public)"] += 1
    elif re.search(r"\bprivate\b", sig): c["private"] += 1
    else: c["other"] += 1
print(f"  java HIGH={len(hs)} breakdown: {dict(c)}")
