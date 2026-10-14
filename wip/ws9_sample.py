#!/usr/bin/env python3
"""usage: ws9_sample.py ABDIR LANG CONF N [seed]  -- random added edges"""
import json, sys, random
ab, lang, conf, n = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
random.seed(int(sys.argv[5]) if len(sys.argv) > 5 else 7)
rows = [r for r in map(json.loads, open(f'{ab}/added.jsonl')) if r['lang'] == lang and r['confidence'] == conf]
R = '/storage/cgg-test_repos/'
for r in random.sample(rows, min(n, len(rows))):
    print(f"{r['site_path'].replace(R,'')}:{r['site_line']}  [{r['src']}]  {r['line'].strip()[:110]}\n      -> {r['dst']}  @ {r['dst_path'].replace(R,'')}  ({r['resolver']})")
