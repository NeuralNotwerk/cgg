import json, sys, collections
p = sys.argv[1]; langs = set(sys.argv[2].split(","))
sites = collections.defaultdict(list)
for l in open(p):
    d = json.loads(l)
    if d["lang"] not in langs: continue
    sites[(d["repo"], d["lang"], d["site_path"], d["site_line"], d["src"], d["dst"].rsplit(".",1)[-1])].append(d)
own = lambda q: q.rsplit(".",1)[0] if "." in q else ""
cls = collections.Counter(); ex = collections.defaultdict(list)
for k, v in sites.items():
    owners = {own(x["dst"]) for x in v}; files = {x["dst_path"] for x in v}
    if len(owners) <= 1 and len(files) <= 1: continue
    callown = own(k[4]); bare = callown.rsplit(".",1)[-1]
    exact = [x for x in v if own(x["dst"]) == callown]
    samebare = [x for x in v if own(x["dst"]).rsplit(".",1)[-1] == bare]
    if len(owners) == 1: c = "copies(same owner, >1 file)"
    elif exact and len(samebare) == len(v): c = "own-bare-owner(has exact)"
    elif len(samebare) == len(v): c = "own-bare-owner(no exact)"
    elif samebare: c = "mixed"
    else: c = "other-owner"
    cls[(k[1], c, x["resolver"])] += len(v)
    if len(ex[(k[1], c, x["resolver"])]) < int(sys.argv[3] if len(sys.argv)>3 else 3): ex[(k[1], c, x["resolver"])].append((k[0], k[2].split("test_repos/")[-1], k[3], k[4], v[0]["line"][:70], len(v), sorted(owners)[:3]))
for k, n in cls.most_common():
    print(n, k)
    for e in ex[k]: print("    ", e)
