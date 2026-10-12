import json,collections,re,os,sys
langs=set(sys.argv[1].split(','))
drops=collections.defaultdict(list)
add=set()
for l in open('added.jsonl'):
    r=json.loads(l)
    if r['lang'] in langs: add.add((r['repo'],r['site_path'],r['site_line'],r['src']))
for l in open('dropped.jsonl'):
    r=json.loads(l)
    if r['lang'] in langs and (r['repo'],r['site_path'],r['site_line'],r['src']) not in add:
        drops[r['repo']].append(r)
c=collections.Counter(); ex=collections.defaultdict(list)
ext=collections.Counter()
for repo,rs in drops.items():
    g=json.load(open(f'new/{repo}.json'))
    files={k:v['path'] for k,v in g['files'].items()}
    idx=collections.defaultdict(list)
    for u in g['unresolved']:
        idx[(files.get(u['file']),u['site_line'])].append(u)
    for r in rs:
        name=re.split(r'[.:#]+',r['dst'])[-1]
        us=[u for u in idx.get((r['site_path'],r['site_line']),[]) if u['name']==name or u['name'].endswith('.'+name) or u['name'].endswith(name)]
        if us:
            st=us[0]['reason'].get('stage')
            if st=='fanout-cap-exceeded' or 'detail' not in us[0]['reason']: key=st
            else: key=st
        else: key='NOT-IN-UNRESOLVED'
        k=(r['lang'],key)
        c[k]+=1
        if len(ex[k])<6: ex[k].append((r,us[0]['reason'] if us else None))
for k,v in sorted(c.items(),key=lambda x:(x[0][0],-x[1])): print(k,v)
json.dump({'|'.join(k):v for k,v in ex.items()},open(f'/storage/cgg/target/remediation/logs/WS4-review/reasons_{"_".join(sorted(langs))}.json','w'))
