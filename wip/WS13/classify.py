import json,collections,sys,re,os,random
ab=sys.argv[1]
rows=[json.loads(l) for l in open(f'{ab}/dropped.jsonl')]
adds=[json.loads(l) for l in open(f'{ab}/added.jsonl')]
simple=lambda q: re.split(r'[.:#/\\]+',q)[-1]
# audit fates in new
fates={}
for repo in {r['repo'] for r in rows}:
    a=json.load(open(f'{ab}/new/{repo}.json.audit.json'))
    for fa in a:
        if fa.get('event')!='file_analyzed': continue
        p=fa['path']
        for b in ('unresolved_calls','external_calls','stdlib_calls'):
            for u in fa.get(b,[]):
                rr=u['reason']; st=rr.get('stage') if isinstance(rr,dict) else rr
                fates[(repo,os.path.normpath(p),u['site_line'],u['name'].split('::')[-1])]=(b,st)
addsites=collections.defaultdict(list)
for r in adds: addsites[(r['repo'],r['site_path'],r['site_line'])].append(r)
sites=collections.defaultdict(list)
for r in rows: sites[(r['repo'],r['site_path'],r['site_line'],simple(r['dst']))].append(r)
cls=collections.Counter(); ex=collections.defaultdict(list)
for (repo,sp,line,name),rs in sites.items():
    rel=os.path.normpath(sp)
    f=fates.get((repo,rel,line,name))
    ad=[a for a in addsites.get((repo,sp,line),[]) if simple(a['dst'])==name]
    wrap=all(' as ' in r['dst'] for r in rs) and len({r['dst'].split(' as ')[0] for r in rs})==1
    if ad: fate='rebound'
    elif f: fate=f'{f[0]}:{f[1]}'
    else: fate='bound-elsewhere/merged'
    pat = 'P6-trait-overloads' if (wrap and (len(rs)>1 or (f and f[1]=='fanout-cap-exceeded'))) else ('P4-foreign-type' if f and f[1]=='external-module' else 'other')
    k=(pat,fate,rs[0]['confidence'])
    cls[k]+=len(rs); ex[k].append((repo,rel,line,rs[0]['src'],[r['dst'] for r in rs][:2],[a['dst'] for a in ad][:2],rs[0]['line'][:100]))
for k,v in sorted(cls.items(),key=lambda x:-x[1]): print(v,k)
random.seed(int(sys.argv[2]) if len(sys.argv)>2 else 0)
for k,v in ex.items():
    print('\n==',k,len(v))
    for x in random.sample(v,min(4,len(v))): print('  ',x)
