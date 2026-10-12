import json,collections,sys
add=collections.defaultdict(list)
for l in open('added.jsonl'):
    r=json.loads(l); add[(r['repo'],r['site_path'],r['site_line'],r['src'])].append(r)
def show(r):
    rp=r['repo']+'/'
    return f"{r['repo']} {r['site_path'].split(rp)[-1]}:{r['site_line']} src={r['src']}\n     -> {r['dst']} @ {r['dst_path'].split(rp)[-1]} [{r['confidence']}]\n     | {r['line'].strip()[:160]}"
if __name__=='__main__':
    targets=sys.argv[1:]
    for l in open('dropped.jsonl'):
        r=json.loads(l)
        k=(r['repo'],r['site_path'],r['site_line'],r['src'])
        for t in targets:
            if t in r['site_path']+':'+str(r['site_line']):
                print('DROP',show(r))
                for a in add.get(k,[]): print('  ADD',show(a))
