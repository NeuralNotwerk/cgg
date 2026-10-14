import json,random,sys,collections
ab=sys.argv[1]; random.seed(12)
rows=collections.defaultdict(list)
for l in open(f'{ab}/added.jsonl'):
    d=json.loads(l); rows[(d['lang'],d['confidence'])].append(d)
for (lang,conf),v in sorted(rows.items()):
    if lang not in ('c','cpp','elixir','clojure'): continue
    k=6 if conf=='high' else 5
    print(f'=== {lang} {conf} (n={len(v)})')
    for d in random.sample(v,min(k,len(v))):
        print(f" {d['repo']} {d['site_path'].split(d['repo'])[-1]}:{d['site_line']}  {d['src']} -> {d['dst']} @ {d['dst_path'].split(d['repo'])[-1]}\n     | {d['line'][:110]}")
