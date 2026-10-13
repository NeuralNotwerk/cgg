"""Share of edges at `high` per language over one A/B side (WS14).
Usage: profile.py <side dir> <out.json>"""
import json,os,sys,collections
side,out=sys.argv[1],sys.argv[2]
agg=collections.defaultdict(collections.Counter)
for fn in sorted(os.listdir(side)):
    if not fn.endswith('.json') or fn.endswith('.audit.json') or fn.endswith('.dead.json'): continue
    try: g=json.load(open(os.path.join(side,fn)))
    except Exception: continue
    lang={k:v['language'] for k,v in g['callables'].items()}
    for e in g['edges']:
        l=lang.get(e['src'])
        if not l or e.get('resolver')=='framework-entry': continue
        agg[l]['e_'+e['confidence']]+=1
json.dump(agg,open(out,'w'),indent=1)
