import json, sys, os
d = sys.argv[1]
g = json.load(open(os.path.join(d, 'g.json')))
C = g['callables']
F = {}
for fid, f in g['files'].items():
    F[fid] = f['path'] if isinstance(f, dict) else f
def line(fid, n):
    p = F.get(fid)
    if not p: return ''
    if not os.path.isabs(p): p = os.path.join(d, p)
    try:
        return open(p, errors='replace').read().split('\n')[n-1].strip()
    except Exception: return '?'
print("CALLABLES:", ", ".join(sorted(c['qualified_name'] for c in C.values())))
print("EDGES:")
for e in g['edges']:
    s, t = C.get(e['src'], {}), C.get(e['dst'], {})
    print(f"  {s.get('qualified_name')} -> {t.get('qualified_name')}  [{e['via']['kind']} {e['resolver']} {e['confidence']}] L{e['site_line']}: {line(s.get('file'), e['site_line'])[:70]}")
print("UNRESOLVED:")
for u in g.get('unresolved', []):
    print(f"  {u.get('receiver_hint','')}.{u['name']} L{u['site_line']} {u['reason']}")
a = os.path.join(d, 'g.json.audit.json')
if os.path.exists(a) and len(sys.argv) > 2:
    A = json.load(open(a))
    for k in A:
        if isinstance(A[k], list) and A[k] and isinstance(A[k][0], dict) and 'name' in A[k][0] and 'site_line' in A[k][0]:
            print("AUDIT", k, [(x.get('receiver_hint',''), x['name'], x['site_line'], x.get('reason')) for x in A[k]][:30])
