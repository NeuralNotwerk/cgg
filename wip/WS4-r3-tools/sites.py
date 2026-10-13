import json, sys
# usage: sites.py ABDIR side repo file_suffix line [line...]
ab, side, repo, suf = sys.argv[1:5]; lines = set(int(x) for x in sys.argv[5:])
g = json.load(open(f"{ab}/{side}/{repo}.json"))
files = {k: v["path"] for k, v in g["files"].items()}; cs = g["callables"]
for e in g["edges"]:
    s = cs[e["src"]]
    if files[s["file"]].endswith(suf) and e["site_line"] in lines:
        d = cs[e["dst"]]
        print(side, e["site_line"], d["qualified_name"], files[d["file"]].split("test_repos/")[-1], e["confidence"])
