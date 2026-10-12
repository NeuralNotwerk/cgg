"""refine.py SAMPLES.jsonl: sub-classify implicit-public / protected rows."""
import json, sys, re, collections
rows = [json.loads(l) for l in open(sys.argv[1])]
cache = {}
def src(p):
    if p not in cache:
        try: cache[p] = open(p, errors='replace').read()
        except Exception: cache[p] = ''
    return cache[p]
def owner_is_interface(r):
    parts = r['qn'].split('.')
    if len(parts) < 2: return False
    o = parts[-2]
    return re.search(r'\b(interface|@interface)\s+' + re.escape(o) + r'\b', src(r['path'])) is not None
c = collections.Counter(); ex = collections.defaultdict(list)
for r in rows:
    cl = r.get('class', '')
    if r['lang'] not in ('java', 'csharp'):
        continue
    if cl.startswith('implicit-public') or cl.startswith('protected-cap-from') or cl.startswith('token-only') or cl == 'other-private->public' or cl.startswith('other'):
        sub = 'accessor' if r.get('kind') == 'property' else ('owner-is-interface' if owner_is_interface(r) else 'owner-NOT-interface')
        k = (r['lang'], cl, r['trans'], sub)
        c[k] += 1
        if len(ex[k]) < 8: ex[k].append((r['repo'], r['path'].split('/storage/cgg-test_repos/')[-1], r['line'], r['sig'][:110]))
for k, n in sorted(c.items(), key=lambda x: -x[1]):
    print(n, k)
print()
for k in ex:
    if 'NOT' in k[3] or k[1].startswith('other') or k[1].startswith('token'):
        print('==', k)
        for e in ex[k]: print('   ', e)
