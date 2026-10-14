import json,sys,random,collections,re
ab,kind,lang=sys.argv[1],sys.argv[2],sys.argv[3]
n=int(sys.argv[4]) if len(sys.argv)>4 else 15
conf=sys.argv[5] if len(sys.argv)>5 else None
seed=int(sys.argv[6]) if len(sys.argv)>6 else 1
rows=[json.loads(l) for l in open(f'{ab}/{kind}.jsonl')]
rows=[r for r in rows if r['lang']==lang and (conf is None or r.get('confidence')==conf)]
c=collections.Counter((r['repo']) for r in rows)
print(len(rows),'rows; top repos',c.most_common(8))
c2=collections.Counter((r.get('resolver'),r.get('confidence')) for r in rows)
print(c2.most_common(10))
random.seed(seed)
for r in random.sample(rows,min(n,len(rows))):
    print(f"--{r['repo']} {r['src']} -> {r['dst']} [{r.get('confidence')} {r.get('resolver')}] {r['site_path'].split('/')[-1]}:{r['site_line']}\n    {r['line'][:150]}")
