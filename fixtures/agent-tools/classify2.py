import json, sys, re, os, collections
for o in sys.argv[1:]:
    r = os.path.basename(o)
    g = json.load(open(f"{o}/g.json")); dc = json.load(open(f"{o}/dc.json")); C = g["callables"]
    unres = collections.Counter((C.get(u["src"], {}).get("language") or "", u["name"]) for u in g["unresolved"])
    unres_any = collections.Counter(u["name"] for u in g["unresolved"])
    called = collections.Counter()
    for e in g["edges"]:
        d = C.get(e["dst"])
        if d: called[(d["language"], d["simple_name"])] += 1
    highs = [f for f in dc["findings"] if f["confidence"] == "high"]
    cnt = collections.Counter()
    fc = {}
    for f in highs:
        c = C.get(f["id"], {}); sig = c.get("signature_hint", "").strip(); attrs = c.get("attributes") or []
        q = f["qualified_name"]; own = q.rsplit(".", 1)[0].split(".")[-1]
        p = f["path"]; p = p if p.startswith("/") else "/" + p
        if p not in fc:
            try: fc[p] = open(p, errors="replace").read()
            except Exception: fc[p] = ""
        iface = re.search(r"\binterface\s+" + re.escape(own) + r"\b", fc[p]) is not None
        bodiless = sig.endswith(";")
        lang = f["language"]; sn = f["simple_name"]
        if lang == "java":
            ov = "@Override" in attrs
            expl = False
        else:
            ov = re.search(r"\boverride\b", sig) is not None
            expl = re.search(r"\bI[A-Z]\w*(<[^>]*>)?\.\w+\s*[(<]", sig) is not None
        if iface and bodiless: k = "iface-decl"
        elif bodiless and "abstract" in sig: k = "abstract-decl"
        elif ov: k = "override-impl"
        elif expl: k = "explicit-iface-impl"
        else: k = "other"
        cnt[k] += 1
        if k != "other":
            if unres_any[sn]: cnt[k + " +same-name-unresolved-site"] += 1
            if called[(lang, sn)]: cnt[k + " +same-name-callable-has-incoming-edge"] += 1
            if c.get("vis") in ("internal", "private"): cnt[k + " vis=internal/private"] += 1
            if c.get("vis") == "protected": cnt[k + " vis=protected"] += 1
    print(f"## {r}: high={len(highs)}  " + "; ".join(f"{k}={v}" for k, v in sorted(cnt.items())))
