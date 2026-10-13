import json,collections,sys,re,os
ab=sys.argv[1]
langs=set(sys.argv[2].split(',')) if len(sys.argv)>2 else None
simple=lambda q: re.split(r'[.:#/\\]+',q)[-1]
drop=collections.defaultdict(collections.Counter); add=collections.defaultdict(collections.Counter)
chg=collections.defaultdict(collections.Counter)
dsites=collections.defaultdict(list); asites=collections.defaultdict(set)
for l in open(f'{ab}/dropped.jsonl'):
    r=json.loads(l)
    if langs and r['lang'] not in langs: continue
    drop[r['lang']][r['confidence']]+=1
    dsites[r['repo']].append((r['lang'],r['site_path'],r['site_line'],simple(r['dst']),r['confidence']))
for l in open(f'{ab}/added.jsonl'):
    r=json.loads(l)
    if langs and r['lang'] not in langs: continue
    add[r['lang']][r['confidence']]+=1
    asites[r['repo']].add((r['site_path'],r['site_line'],simple(r['dst'])))
for l in open(f'{ab}/changed.jsonl'):
    r=json.loads(l)
    if langs and r['lang'] not in langs: continue
    o=max((c for c,_ in r['old']),key=['low','medium','high'].index,default=None)
    n=max((c for c,_ in r['new']),key=['low','medium','high'].index,default=None)
    if o!=n: chg[r['lang']][f'{o}->{n}']+=1
fate=collections.defaultdict(collections.Counter)
for repo,ds in dsites.items():
    g=json.load(open(f'{ab}/new/{repo}.json'))
    fp={k:(v['path'] if isinstance(v,dict) else v) for k,v in g['files'].items()} if isinstance(g['files'],dict) else {f['id']:f['path'] for f in g['files']}
    root=f'/storage/cgg-test_repos/{repo}'
    un={}
    for u in g['unresolved']:
        p=fp.get(u['file'],'')
        if not p.startswith('/'): p=os.path.join(root,p)
        un[(os.path.normpath(p),u['site_line'],u['name'])]=u['reason'].get('stage') or json.dumps(u['reason'])
    newedge_sites={(p,ln) for p,ln,_ in asites[repo]}
    for lang,p,ln,name,conf in ds:
        if (p,ln,name) in asites[repo]: f='rebound to another target'
        elif (os.path.normpath(p),ln,name) in un: f='now unresolved: '+un[(os.path.normpath(p),ln,name)]
        else: f='external/stdlib or still bound (same name, kept target)'
        fate[lang][f]+=1
out={}
for lang in sorted(set(drop)|set(add)):
    out[lang]=dict(drop=dict(drop[lang]),add=dict(add[lang]),chg=dict(chg[lang]),fate=dict(fate[lang]))
json.dump(out,open(f'{ab}/conf_summary.json','w'),indent=1)
for lang,v in out.items():
    print(lang,'| removed',v['drop'],'| added',v['add'],'| rescored',v['chg'])
    for k,c in sorted(v['fate'].items(),key=lambda x:-x[1])[:6]: print('      ',c,k)
