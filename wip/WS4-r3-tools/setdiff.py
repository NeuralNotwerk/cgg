import json, sys, collections
a, b, kind, langs = sys.argv[1], sys.argv[2], sys.argv[3], set(sys.argv[4].split(","))
def key(d): return (d["repo"], d["src"], d["dst"], d["dst_path"], d["site_path"], d["site_line"])
def load(p):
    out = {}
    for l in open(p):
        d = json.loads(l)
        if d["lang"] in langs: out[key(d)] = d
    return out
A, B = load(f"{a}/{kind}.jsonl"), load(f"{b}/{kind}.jsonl")
only_a = [A[k] for k in A if k not in B]; only_b = [B[k] for k in B if k not in A]
json.dump({"only_a": only_a, "only_b": only_b}, open(sys.argv[5], "w"))
for nm, rows in (("only_" + a.split("/")[-1], only_a), ("only_" + b.split("/")[-1], only_b)):
    c = collections.Counter((r["lang"], r["confidence"], r["resolver"]) for r in rows)
    print(nm, len(rows)); [print("   ", k, v) for k, v in c.most_common(8)]
