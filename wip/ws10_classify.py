#!/usr/bin/env python3
"""Classify WS10 A/B edge diffs by receiver shape. usage: ws10_classify.py ABDIR [lang,...]"""
import json, sys, re, collections
ab = sys.argv[1]
langs = set(sys.argv[2].split(',')) if len(sys.argv) > 2 else None
def simple(q): return re.split(r'[.:#/\\]+', q)[-1]
def cls(row):
    line = row.get('line') or ''
    name = re.escape(simple(row['dst']).split('<')[0])
    sp = row.get('site_path', '')
    pre = r'\s*(?:\.|->|\?\.|\?->|::|&\.)\s*' + name + r'\b'
    if re.search(r'\?(\.|->)\s*' + name + r'\b', line): return 'null-conditional'
    if re.search(r'\bas[!?]?\s+[\w.<>\[\]?]+\s*\)' + pre, line): return 'as-cast'
    if re.search(r'\(\s*\(?[\w.:<>*&\s]+\)\s*[\w.]+\s*\)' + pre, line) or re.search(r'_cast\s*<', line) or 'asInstanceOf' in line or re.search(r'\.\(\*?[\w.]+\)' + pre, line): return 'cast'
    if re.search(r'\bawait\b[^;]*\)' + pre, line): return 'await'
    if re.search(r'\bnew\s+[\w.:<>\[\], ]+\s*(\([^;]*\))?\s*(\{[^;]*\})?\s*\)?' + pre, line): return 'new-construction'
    if re.search(r'\.new\b[^.]*' + pre, line): return 'ruby-new'
    if re.search(r'\b[A-Z_]\w*\s*(\([^;]*\)|\{[^;]*\})\s*\)?' + pre, line): return 'type-call/literal construction'
    if re.search(r'\)' + pre, line): return 'parenthesized/call-result'
    if sp.endswith('.ts') or sp.endswith('.mts') or sp.endswith('.cts'): return 'ts-file (grammar)'
    if re.search(r'\b(math|scala|util|io|sys|collection|concurrent|java|javax)\.' + name, line): return 'scala-implicit-pkg'
    return 'other (typed lowercase / import head)'
for kind in ['dropped', 'added']:
    c = collections.Counter(); ex = collections.defaultdict(list)
    for l in open(f'{ab}/{kind}.jsonl'):
        r = json.loads(l)
        if langs and r['lang'] not in langs: continue
        k = (r['lang'], r['confidence'], cls(r))
        c[k] += 1
        if len(ex[k]) < 3: ex[k].append(f"{r['repo']}:{r['site_path'].split(r['repo'])[-1]}:{r['site_line']} {r.get('line','').strip()[:100]} -> {r['dst']}")
    print(f'== {kind}')
    for k, v in sorted(c.items(), key=lambda x: -x[1]):
        print(f'{v:8d} {k}')
        for e in ex[k]: print('           ', e)
