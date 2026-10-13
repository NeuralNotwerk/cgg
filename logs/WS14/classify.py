import json,sys,collections,os,random,re
A=sys.argv[1]
drops=[json.loads(l) for l in open(A+'/dropped.jsonl')]
adds=[json.loads(l) for l in open(A+'/added.jsonl')]
def mini(r):
    p=r['site_path']
    return bool(re.search(r'\.min\.|/dist/|bundle|chunk|vendors?[/.-]|\.yarn/|/static/web/|worker\.js|webroot/js|/public/js|assets?/.*\.js$|swagger-ui|\.cjs$',p)) or len(r.get('line') or '')>=190
repos=set(r['repo'] for r in drops+adds)
unres={}
for repo in repos:
    fn=f"{A}/new/{repo}.json"
    try: g=json.load(open(fn))
    except Exception: continue
    F=g['files']
    for u in g['unresolved']:
        p=F.get(u['file'],{}).get('path')
        st=u['reason']['stage'] if isinstance(u['reason'],dict) else str(u['reason'])
        unres[(p,u['site_line'],u['name'].split('.')[-1].split(':')[-1])]=st
    for fid,f in F.items():
        pass
# added: same-site old edge?
dropsite=collections.defaultdict(list)
for r in drops: dropsite[(r['site_path'],r['site_line'])].append(r)
addsite=collections.defaultdict(list)
for r in adds: addsite[(r['site_path'],r['site_line'])].append(r)
def simple(q): return re.split(r'::|\.|:|#',q)[-1]
C=collections.Counter(); S=collections.defaultdict(list)
for r in drops:
    m='min' if mini(r) else 'src'
    st=unres.get((r['site_path'],r['site_line'],simple(r['dst'])))
    repl=addsite.get((r['site_path'],r['site_line']))
    if repl and any(simple(x['dst'])==simple(r['dst']) for x in repl):
        why='rebound'
    else:
        why=st or 'no-record'
    k=('DROP',r['lang'],m,r['confidence'],why)
    C[k]+=1; S[k].append(r)
for r in adds:
    m='min' if mini(r) else 'src'
    prev=dropsite.get((r['site_path'],r['site_line']))
    why='replaces' if prev and any(simple(x['dst'])==simple(r['dst']) for x in prev) else 'new-site'
    same='samefile' if r['site_path']==r['dst_path'] else 'xfile'
    k=('ADD',r['lang'],m,r['confidence'],why+'/'+same)
    C[k]+=1; S[k].append(r)
for k,v in sorted(C.items(),key=lambda x:(x[0][0],x[0][1],-x[1])):
    print('\t'.join(k),v,sep='\t')
random.seed(int(sys.argv[2]) if len(sys.argv)>2 else 1)
json.dump({'|'.join(k):random.sample(v,min(12,len(v))) for k,v in S.items()},open(A+'/ws14_samples.json','w'))
