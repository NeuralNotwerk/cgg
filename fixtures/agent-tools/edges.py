import json, sys, os, collections
D = sys.argv[1]
def load(p):
    g = json.load(open(p)); C = g['callables']
    q = lambda i: C[i]['qualified_name'] if i in C else i
    out = collections.defaultdict(list)
    for e in g['edges']:
        out[q(e['src'])].append(f"{q(e['dst']).split('.')[-1] if False else q(e['dst'])}({e['confidence'][0]})")
    return g, C, out
g, C, out = load(os.path.join(D, 'g.json'))
g0, _, out0 = load(os.path.join(D, 'g0.json'))
unres = collections.defaultdict(list)
q = lambda i: C[i]['qualified_name'] if i in C else i
for u in g['unresolved']:
    unres[q(u['src'])].append(f"{u['name']}:{u['reason'].get('stage')}{u['reason'].get('detail',{}).get('candidates','')}[{u.get('receiver_hint','')!r}]")
ext = collections.defaultdict(list)
for ev in json.load(open(os.path.join(D, 'g.json.audit.json'))):
    if ev.get('event') == 'file_analyzed':
        for k in ('external_calls', 'stdlib_calls'):
            for u in ev.get(k, []):
                ext[q(u['src'])].append(f"{k[:3]}:{u['name']}[{u.get('receiver_hint','')!r}]")
for c in sorted(C.values(), key=lambda c: c['start_line']):
    n = c['qualified_name']
    if any(x in n for x in ('zorkle', 'quux', 'Zorkle', 'Quux')) or n in ('Foo','p.Foo','p.Box','Box') : continue
    print(f"{n:28s} E={out.get(n, [])} cap0={out0.get(n, [])} U={unres.get(n, [])} X={ext.get(n, [])}")
