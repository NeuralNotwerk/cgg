import json,os,sys,collections
side=sys.argv[1]
agg=collections.defaultdict(collections.Counter)
for fn in sorted(os.listdir(side)):
    if not fn.endswith('.json') or fn.endswith('.audit.json') or fn.endswith('.dead.json'): continue
    try: g=json.load(open(os.path.join(side,fn)))
    except Exception: continue
    lang={k:v['language'] for k,v in g['callables'].items()}
    for e in g['edges']:
        l=lang.get(e['src'])
        if not l: continue
        if e.get('resolver')=='framework-entry': continue
        agg[l]['e_'+e['confidence']]+=1
    for l,m in g['metrics'].get('by_language',{}).items():
        if l.startswith('skip:'): continue
        agg[l]['unresolved']+=m.get('unresolved',0); agg[l]['external']+=m.get('external',0); agg[l]['stdlib']+=m.get('stdlib',0)
json.dump(agg,open(f'/storage/cgg/target/remediation/logs/WS4-review/profile_{side}.json','w'))
