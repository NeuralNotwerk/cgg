#!/usr/bin/env python3
"""Classify WS9 A/B edge diffs by call-site pattern. usage: ws9_classify.py ABDIR"""
import json, sys, re, collections
ab = sys.argv[1]
def simple(q): return re.split(r'[.:#/\\]+', q)[-1]
def cls(row):
    line = row.get('line') or ''
    name = simple(row['dst'])
    i = line.find(name)
    after = line[i + len(name):] if i >= 0 else ''
    if re.match(r'\s*<', after): return 'type-args <..>'
    if re.match(r'\s*\[', after): return 'type-args [..]'
    if re.search(r'\?\.\s*' + re.escape(name), line): return 'null-conditional ?.'
    if re.search(r'>\s*\.\s*' + re.escape(name), line): return 'generic-type receiver'
    if re.search(r'\btemplate\s+' + re.escape(name), line): return 'template disambiguator'
    return 'other'
for kind in ['dropped', 'added']:
    c = collections.Counter(); ex = collections.defaultdict(list)
    for l in open(f'{ab}/{kind}.jsonl'):
        r = json.loads(l)
        k = (r['lang'], r['confidence'], cls(r))
        c[k] += 1
        if len(ex[k]) < 3: ex[k].append(f"{r['repo']}:{r['site_path'].split(r['repo'])[-1]}:{r['site_line']} {r.get('line','').strip()[:90]} -> {r['dst']}")
    print(f'== {kind}')
    for k, v in sorted(c.items(), key=lambda x: -x[1]):
        print(f'{v:8d} {k}')
        for e in ex[k]: print('           ', e)
