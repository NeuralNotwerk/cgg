import json, sys, os
D = sys.argv[1]
g = json.load(open(os.path.join(D, 'g.json')))
C = g['callables']
F = {k: v['path'] for k, v in g['files'].items()}
def qn(i):
    c = C.get(i)
    return c['qualified_name'] if c else i
print('== callables')
for c in C.values():
    print(f"   {c['qualified_name']:40s} {c['kind']:12s} {os.path.basename(F.get(c['file'],'?'))}:{c['start_line']}")
print('== edges')
for e in sorted(g['edges'], key=lambda e: (e.get('site_line') or 0)):
    v = e['via']['kind']
    print(f"   L{e.get('site_line')}: {qn(e['src'])} -> {qn(e['dst'])}  [{e['confidence']}, {e['resolver']}, {v}]")
seen = set()
print('== unresolved (graph)')
for u in g['unresolved']:
    extra = {k: v for k, v in u.items() if k not in ('src', 'file', 'site_line', 'site_byte', 'name', 'reason')}
    print(f"   L{u['site_line']}: {u['name']!r} reason={json.dumps(u['reason'])} {json.dumps(extra) if extra else ''}")
print('== audit (unresolved/external/stdlib per file)')
for ev in json.load(open(os.path.join(D, 'g.json.audit.json'))):
    if ev.get('event') != 'file_analyzed':
        continue
    for key in ev:
        if key.endswith('_calls') and isinstance(ev[key], list):
            for u in ev[key]:
                extra = {k: v for k, v in u.items() if k not in ('src', 'file', 'site_line', 'site_byte', 'name', 'reason')}
                print(f"   [{key}] {os.path.basename(ev['path'])} L{u['site_line']}: {u['name']!r} reason={json.dumps(u['reason'])} {json.dumps(extra) if extra else ''}")
print('== dead-code (all bands)')
try:
    dc = json.load(open(os.path.join(D, 'dc.json')))
    fs = dc.get('findings', [])
    for f in fs:
        print(f"   {f.get('qualified_name')}: {f.get('confidence')} {[ev.get('kind', ev) if isinstance(ev, dict) else ev for ev in f.get('evidence', [])][:4]}")
    if not fs: print('   (none)', [k for k in dc.keys()])
except Exception as ex:
    print('   dc.json error', ex)
