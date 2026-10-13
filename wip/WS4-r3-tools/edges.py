import json, sys, subprocess
binp, fx = sys.argv[1], sys.argv[2]
out = subprocess.run([binp, fx, "-t", "json"] + sys.argv[3:], capture_output=True, text=True).stdout
g = json.loads(out)
cs = g["callables"]; files = {k: f["path"] for k, f in g["files"].items()}
def nm(i):
    c = cs.get(i)
    if not c: return i
    return f'{files.get(c["file"], c["file"])}:{c["qualified_name"]}@{c["start_line"]}'
rows = [f'  L{e["site_line"]} {nm(e["src"])} -> {nm(e["dst"])} [{e.get("confidence")} {e.get("resolver")}]' for e in g["edges"]]
print("\n".join(sorted(rows)))
for u in g.get("unresolved", []) if isinstance(g.get("unresolved"), list) else []:
    pass
