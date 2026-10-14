#!/usr/bin/env python3
# usage: show.py <bin> <dir> [extra cgg args...] -> edges, unresolved, stdlib-booked per site
import json,subprocess,sys,os
b,d=sys.argv[1],sys.argv[2]
out='/tmp/claude-1000/ws11show_%d.json'%os.getpid()
subprocess.run([b,'.','-t','json','-o',out]+sys.argv[3:],cwd=d,stderr=subprocess.DEVNULL,check=True)
g=json.load(open(out))
cs=g['callables']
name=lambda i:(cs[i]['qualified_name'] if i in cs else i)
rows=[]
for e in g['edges']:
    rows.append((e.get('site_line',0),'E',f"{name(e['src'])} -> {name(e['dst'])} [{e['confidence']},{e['resolver']},{e['via']['kind']}]"))
for u in g['unresolved']:
    rows.append((u.get('site_line',0),'U',f"{u.get('name')} recv={u.get('receiver_hint','')!r} reason={u.get('reason',{}).get('stage')}"))
for x in json.load(open(out+'.audit.json')):
    if x.get('event')=='file_analyzed':
        for u in x.get('stdlib_calls',[]):
            rows.append((u.get('site_line',0),'S',f"{u.get('name')} recv={u.get('receiver_hint','')!r} stdlib"))
for r in sorted(rows,key=lambda r:(int(r[0]),r[1],r[2])): print(*r)
