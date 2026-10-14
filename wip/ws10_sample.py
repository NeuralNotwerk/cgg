import json,random,sys,collections
ab=sys.argv[1]; langs=sys.argv[2].split(','); na=int(sys.argv[3]); nd=int(sys.argv[4])
random.seed(10)
R='/storage/cgg-test_repos/'
def load(k):
    d=collections.defaultdict(list)
    for l in open(f'{ab}/{k}.jsonl'):
        r=json.loads(l)
        if r['lang'] in langs and 'min.js' not in r['site_path']: d[r['lang']].append(r)
    return d
A=load('added'); D=load('dropped')
for lang in langs:
    for kind,src,n in (('ADD',A,na),('DROP',D,nd)):
        rows=src[lang]; random.shuffle(rows)
        for r in rows[:n]:
            print(f"[{lang} {kind} {r['confidence']}] {r['site_path'].replace(R,'')}:{r['site_line']}\n    {r['line'].strip()[:150]}\n    -> {r['dst']}  @ {r['dst_path'].replace(R,'')}")
