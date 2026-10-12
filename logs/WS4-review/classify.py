import json,collections,os,re,sys,random
random.seed(1)
def load(f):
    for l in open(f):
        yield json.loads(l)
def shape(r):
    name=re.split(r'[.:#]+',r['dst'])[-1]
    if name=='constructor' or name=='__init__' or name=='new': name=re.split(r'[.:#]+',r['dst'])[-2] if len(re.split(r'[.:#]+',r['dst']))>1 else name
    line=r.get('line') or ''
    m=re.search(r'(\.|->|::|:)?\s*\b'+re.escape(name)+r'\b',line)
    if not m: return 'name-not-on-line'
    if m.group(1): return 'qualified'+m.group(1)
    if re.search(r'\bnew\s+'+re.escape(name),line): return 'new'
    return 'bare'
def rel(r):
    a=os.path.dirname(r['site_path']); b=os.path.dirname(r['dst_path'])
    if r['site_path']==r['dst_path']: return 'samefile'
    if a==b: return 'samedir'
    return 'otherdir'
langs=sys.argv[2].split(',') if len(sys.argv)>2 else None
fn=sys.argv[1]
c=collections.Counter(); ex=collections.defaultdict(list)
for r in load(fn):
    if langs and r['lang'] not in langs: continue
    k=(r['lang'],r['confidence'],shape(r),rel(r))
    c[k]+=1
    if len(ex[k])<400: ex[k].append(r)
for k,v in sorted(c.items(),key=lambda x:(x[0][0],-x[1])):
    if v>=50: print(f"{k[0]:11} {k[1]:7} {k[2]:16} {k[3]:9} {v}")
json.dump({ '|'.join(k):random.sample(v,min(8,len(v))) for k,v in ex.items()},open(fn+'.samples.json','w'))
