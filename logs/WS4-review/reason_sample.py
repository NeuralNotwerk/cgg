import json,collections,re,os,sys,random
lang, want, n = sys.argv[1], sys.argv[2], int(sys.argv[3])
random.seed(int(sys.argv[4]) if len(sys.argv)>4 else 1)
add=set()
for l in open('added.jsonl'):
    r=json.loads(l)
    if r['lang']==lang: add.add((r['repo'],r['site_path'],r['site_line'],r['src']))
drops=collections.defaultdict(list)
for l in open('dropped.jsonl'):
    r=json.loads(l)
    if r['lang']==lang and (r['repo'],r['site_path'],r['site_line'],r['src']) not in add: drops[r['repo']].append(r)
hits=[]; repos=collections.Counter()
for repo,rs in drops.items():
    g=json.load(open(f'new/{repo}.json'))
    files={k:v['path'] for k,v in g['files'].items()}
    idx=collections.defaultdict(list)
    for u in g['unresolved']:
        if u['reason'].get('stage')==want: idx[(files.get(u['file']),u['site_line'])].append(u)
    seen=set()
    for r in rs:
        k=(r['site_path'],r['site_line'])
        if k in seen: continue
        us=idx.get(k)
        if us:
            seen.add(k); hits.append((r,us[0])); repos[repo]+=1
print(f'{lang} {want}: {len(hits)} sites; top repos {repos.most_common(6)}')
for r,u in random.sample(hits,min(n,len(hits))):
    rp=r['repo']+'/'
    L=open(r['site_path'],errors='replace').read().splitlines()
    print(f"\n  {r['repo']} {r['site_path'].split(rp)[-1]}:{r['site_line']} in {r['src']}\n    > {L[r['site_line']-1].strip()[:140]}\n    was -> {r['dst']} @ {r['dst_path'].split(rp)[-1]}\n    now: {u['name']} recv={u.get('receiver_hint','')!r} {json.dumps(u['reason'].get('detail'))[:120]}")
