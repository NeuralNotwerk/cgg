import json, collections, sys
ab = sys.argv[1]; nex = int(sys.argv[2]) if len(sys.argv) > 2 else 4
rows = [json.loads(l) for l in open(f"{ab}/dead_changed.jsonl")]
print("all dead changes:", collections.Counter((r["lang"], r.get("old"), r.get("new")) for r in rows).most_common(12))
rows = [r for r in rows if r.get("new") == "high" and r["lang"] in ("java","csharp")]
cache = {}
def load(side, repo):
    k=(side,repo)
    if k not in cache:
        try: d=json.load(open(f"{ab}/{side}/{repo}.dead.json"))
        except Exception: d={"findings":[]}
        cache[k]={(f["qualified_name"],f["start_line"]):f for f in d["findings"]}
    return cache[k]
cls = collections.Counter(); ex = collections.defaultdict(list)
for r in rows:
    o = load("old", r["repo"]).get((r["qn"], r["line"])); n = load("new", r["repo"]).get((r["qn"], r["line"]))
    ok = {e["kind"] for e in (o or {}).get("evidence", [])}; nk = {e["kind"] for e in (n or {}).get("evidence", [])}
    lost = tuple(sorted(ok - nk)) if o else ("absent-old",)
    sig = (n or {}).get("signature_hint","")
    key = (lost, "override" if ("@Override" in sig or "override" in sig) else "-")
    cls[key]+=1
    if len(ex[key])<nex: ex[key].append((r["repo"], r["qn"], r["line"]))
print("new-high java/csharp:", len(rows))
for k,v in cls.most_common():
    print(v, k)
    for e in ex[k]: print("     ", e)
