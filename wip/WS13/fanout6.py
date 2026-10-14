import json,collections,sys,glob,os,re
d=sys.argv[1]
tot=collections.Counter(); ex=collections.defaultdict(list)
wrap=re.compile(r'<(.+?) as ([A-Za-z_][\w:]*)(<.*>)?>::(\w+)$')
for fn in sorted(glob.glob(d+'/*.json')):
  if fn.endswith('.audit.json') or fn.endswith('.dead.json'): continue
  repo=os.path.basename(fn)[:-5]
  try: g=json.load(open(fn))
  except Exception: continue
  cs=g['callables']
  sets=collections.defaultdict(list)
  for e in g['edges']:
    s=cs.get(e['src']); t=cs.get(e['dst'])
    if not s or not t or s['language']!='rust': continue
    sets[(e['src'],e['site_byte'])].append((t['qualified_name'],e['confidence'],e['resolver'],e['via']['kind']))
  for k,v in sets.items():
    if len(v)<2: continue
    ms=[]
    for qn,_,_,_ in v:
      tail=qn.split('::<',1)[-1] if '::<' in qn else qn
      m=re.search(r'<([^<>]*?(?:<[^<>]*>)?) as ([\w:]+)(<.*?>)?>::(\w+)$',qn)
      ms.append(m)
    if not all(ms): continue
    owners={m.group(1) for m in ms}; traits={m.group(2) for m in ms}; args={m.group(3) for m in ms}
    if len(owners)==1 and len(traits)==1 and len(args)>1:
      key=('over' if len(v)>5 else 'under', v[0][1], v[0][2])
      tot[(repo,)+key]+=len(v)
      ex[key].append((repo,cs[k[0]]['qualified_name'],v[0][0],len(v)))
for k,v in sorted(tot.items()): print(v,k)
for k,v in ex.items():
  print(k, len(v)); 
  for x in v[:5]: print('   ',x)
