#!/usr/bin/env python3
"""Stratified random sample of an A/B's dropped/added edges with source
context, for manual precision judgement.

usage: audit_sample.py ABDIR KIND(dropped|added) LANG N [SEED]
Prints, per sampled edge: the call site (±1 line), the file's import lines
that mention the receiver head / callee name, the destination definition
line, and what (if anything) replaced/accompanied it at the same site.
"""
import json, os, random, re, sys, collections

ab, kind, lang, n = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
seed = int(sys.argv[5]) if len(sys.argv) > 5 else 7
random.seed(seed)

def load(fn):
    with open(os.path.join(ab, fn)) as f:
        for l in f:
            r = json.loads(l)
            if r['lang'] == lang and r['repo'] not in os.environ.get('EXCLUDE', '').split(','):
                yield r

other = 'added.jsonl' if kind == 'dropped' else 'dropped.jsonl'
peers = collections.defaultdict(list)
for r in load(other):
    peers[(r['repo'], r['site_path'], r['site_line'], r['src'])].append(r)
rows = list(load(kind + '.jsonl'))
# One sample per call site, so a fanned-out site is not over-weighted.
by_site = collections.defaultdict(list)
for r in rows:
    by_site[(r['repo'], r['site_path'], r['site_line'], r['src'])].append(r)
sites = list(by_site)
pick = random.sample(sites, min(n, len(sites)))

graphs = {}
def now_at(repo, path, line, name):
    """What the NEW graph has at this site: edges, or the unresolved reason."""
    if repo not in graphs:
        try:
            g = json.load(open(os.path.join(ab, 'new', repo + '.json')))
        except Exception:
            graphs[repo] = None
        else:
            files = {k: v['path'] for k, v in g['files'].items()}
            cal = {}
            for c in (g['callables'].values() if isinstance(g['callables'], dict) else g['callables']):
                cal[c['id']] = (c.get('qualified_name') or c.get('qn') or c.get('name'), files.get(c.get('file'), c.get('file')))
            site = collections.defaultdict(list)
            for e in g['edges']:
                s = cal.get(e['src'])
                if s:
                    site[(s[1], e['site_line'])].append((cal.get(e['dst'], ('?', '?')), e['confidence'], e['resolver']))
            unres = collections.defaultdict(list)
            for u in g['unresolved']:
                unres[(files.get(u['file']), u['site_line'])].append((u['name'], u['reason']))
            graphs[repo] = (site, unres)
    G = graphs[repo]
    if not G:
        return []
    site, unres = G
    out = [f"NOW -> {d[0]} [{c}/{rv}] @ {os.path.relpath(d[1], '/storage/cgg-test_repos/' + repo) if d[1] else '?'}" for d, c, rv in site.get((path, line), []) if d[0] and re.split(r'[.:#]+', d[0])[-1] == name]
    out += [f"NOW unresolved {n}: {json.dumps(rs)[:140]}" for n, rs in unres.get((path, line), []) if n.split('.')[-1] == name or n.endswith(name)]
    return out or ['NOW: nothing at this site for that name (external bucket or other)']

lines_cache = {}
def lines(p):
    if p not in lines_cache:
        try:
            lines_cache[p] = open(p, errors='replace').read().splitlines()
        except Exception:
            lines_cache[p] = []
    return lines_cache[p]

IMPORT = re.compile(r'^\s*(import|from|use|require|using|#include|-import|open|package|local\s+\w+\s*=\s*require|const\s+\w+\s*=\s*require)\b')
print(f'# {kind} {lang}: {len(rows)} edges at {len(sites)} sites; sampled {len(pick)}')
for i, k in enumerate(pick, 1):
    es = by_site[k]
    r = es[0]
    rp = r['repo'] + '/'
    L = lines(r['site_path'])
    ln = r['site_line']
    name = re.split(r'[.:#]+', r['dst'])[-1]
    print(f"\n[{i}] {r['repo']} {r['site_path'].split(rp)[-1]}:{ln}  in {r['src']}")
    for j in range(max(1, ln - 1), min(len(L), ln + 1) + 1):
        print(f"   {'>' if j == ln else ' '}{j}: {L[j-1].rstrip()[:150]}")
    m = re.search(r'([A-Za-z_$][\w$]*)\s*(?:\.|->|::|:)\s*(?:[\w$]+\s*(?:\(.*?\))?\s*(?:\.|->|::)\s*)*' + re.escape(name), L[ln-1] if 0 < ln <= len(L) else '')
    head = m.group(1) if m else None
    imps = [ (j+1, l.strip()) for j, l in enumerate(L[:400]) if IMPORT.search(l) and ((head and re.search(r'\b'+re.escape(head)+r'\b', l)) or re.search(r'\b'+re.escape(name)+r'\b', l))]
    for j, l in imps[:3]:
        print(f"    import {j}: {l[:150]}")
    for e in es[:4]:
        D = lines(e['dst_path'])
        defline = ''
        for j, l in enumerate(D):
            if re.search(r'\b' + re.escape(name) + r'\b', l) and re.search(r'\b(def|func|function|fn|sub|let|fun|class|static|public|private|protected|void|async|export|module)\b|^\s*' + re.escape(name) + r'\s*\(', l):
                defline = f"{j+1}: {l.strip()[:110]}"
                break
        print(f"    {kind[:4].upper()} -> {e['dst']} [{e['confidence']}/{e['resolver']}] @ {e['dst_path'].split(rp)[-1]}  {defline}")
    if len(es) > 4:
        print(f"    … +{len(es)-4} more {kind} at this site")
    for x in now_at(r['repo'], r['site_path'], ln, name)[:4]:
        print('    ' + x)
    for e in peers.get(k, [])[:3]:
        print(f"    {'ADD ' if kind=='dropped' else 'DROP'}(same site) -> {e['dst']} [{e['confidence']}] @ {e['dst_path'].split(rp)[-1]}")
