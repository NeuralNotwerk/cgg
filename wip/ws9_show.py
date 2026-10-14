#!/usr/bin/env python3
"""Show edges + unresolved for a cgg -t json run of the cwd. usage: ws9_show.py BIN [extra args]"""
import json, subprocess, sys, os
out = '/tmp/claude-1000/-storage-cgg/c8f78891-c04e-411a-ae7e-8b5d66e02b1f/scratchpad/ws9show.json'
subprocess.run([sys.argv[1], '.', '-t', 'json', '-o', out] + sys.argv[2:], stderr=subprocess.DEVNULL, check=True)
d = json.load(open(out))
c = d['callables']
for e in d['edges']:
    s = c.get(e['src'], {}).get('qualified_name', e['src']); t = c.get(e['dst'], {}).get('qualified_name', e['dst'])
    print(f"EDGE L{e.get('site_line')} {s} -> {t} [{e['confidence']} {e['resolver']} {e['via']['kind']}]")
for ev in json.load(open(out + '.audit.json')):
    if ev.get('event') != 'file_analyzed':
        continue
    for k, v in ev.items():
        if isinstance(v, list) and v and isinstance(v[0], dict) and 'site_line' in v[0]:
            for x in v:
                print(f"  {k} {ev['path']}:L{x['site_line']} {x.get('receiver_hint','')}.{x['name']} {json.dumps(x.get('reason'))} {x.get('name_screen_applied','')}")
