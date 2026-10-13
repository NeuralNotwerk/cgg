# Stratified spot-check sample from an A/B dir: dropped (with fate), added, rescored.
import json,collections,sys,re,os,random,math
ab,n_lists,per,outdir=sys.argv[1],int(sys.argv[2]),int(sys.argv[3]),sys.argv[4]
random.seed(int(sys.argv[5]) if len(sys.argv)>5 else 1)
R='/storage/cgg-test_repos/'
simple=lambda q: re.split(r'[.:#/\\]+',q)[-1]
rel=lambda p:p.replace(R,'')
added=collections.defaultdict(list)
rows=collections.defaultdict(list)
for l in open(f'{ab}/added.jsonl'):
    r=json.loads(l); added[(r['repo'],r['site_path'],r['site_line'],simple(r['dst']))].append(r)
    rows[('added-'+r['confidence'],r['lang'])].append(('added',r))
for l in open(f'{ab}/changed.jsonl'):
    r=json.loads(l)
    o=max((c for c,_ in r['old']),key=['low','medium','high'].index,default=None)
    n=max((c for c,_ in r['new']),key=['low','medium','high'].index,default=None)
    if o!=n: rows[(f'rescored {o}->{n}',r['lang'])].append(('rescored',r))
drops=[json.loads(l) for l in open(f'{ab}/dropped.jsonl')]
unres={}
def un_for(repo):
    if repo in unres: return unres[repo]
    g=json.load(open(f'{ab}/new/{repo}.json'))
    fp={k:(v['path'] if isinstance(v,dict) else v) for k,v in g['files'].items()} if isinstance(g['files'],dict) else {f['id']:f['path'] for f in g['files']}
    d={}
    for u in g['unresolved']:
        p=fp.get(u['file'],'')
        if not p.startswith('/'): p=os.path.join(R+repo,p)
        d[(os.path.normpath(p),u['site_line'],u['name'])]=u['reason'].get('stage') or json.dumps(u['reason'])
    unres[repo]=d; return d
for r in drops:
    k=(r['repo'],r['site_path'],r['site_line'],simple(r['dst']))
    if k in added: f='rebound'; r['_new']=[(rel(a['dst_path']),a['dst'],a['confidence']) for a in added[k]]
    else:
        u=un_for(r['repo']).get((os.path.normpath(r['site_path']),r['site_line'],simple(r['dst'])))
        f='now-unresolved:'+u if u else 'gone (external/stdlib or same-name target kept)'
    r['_fate']=f
    rows[('dropped-'+r['confidence']+' / '+f.split(':')[0],r['lang'])].append(('dropped',r))
strata=[(k,v) for k,v in rows.items() if len(v)>=20]
tot=n_lists*per
w={k:math.sqrt(len(v)) for k,v in strata}; W=sum(w.values())
alloc={k:max(1,round(tot*w[k]/W)) for k,_ in strata}
items=[]
for k,v in strata:
    for kind,r in random.sample(v,min(alloc[k],len(v))):
        e={'stratum':f'{k[0]} [{k[1]}]','repo':r['repo'],'lang':r['lang'],'caller':r['src'],
           'call_site':f"{rel(r['site_path'])}:{r['site_line']}",'call_text':r['line'][:200],
           'target':r['dst'],'target_file':rel(r['dst_path'])}
        if kind=='dropped':
            e['change']=f"edge REMOVED (was {r['confidence']}); fate now: {r['_fate']}"
            if '_new' in r: e['new_targets']=r['_new']
        elif kind=='added': e['change']=f"edge ADDED at {r['confidence']}"
        else: e['change']=f"edge confidence {k[0].split(' ')[1]} (old {r['old']}, new {r['new']})"
        items.append(e)
random.shuffle(items); items=items[:tot]
os.makedirs(outdir,exist_ok=True)
for i in range(n_lists):
    json.dump(items[i::n_lists],open(f'{outdir}/list{i+1}.json','w'),indent=1)
print(len(strata),'strata,',len(items),'items'); print(collections.Counter(e['stratum'].split(' [')[0] for e in items).most_common())
