#!/usr/bin/env python3
# usage: show.py <bin> <dir> [extra cgg args...]  -> prints edges + unresolved per site
import json,subprocess,sys,os,tempfile
b,d=sys.argv[1],sys.argv[2]
out=os.path.join(tempfile.gettempdir(),'ws10show_%d.json'%os.getpid())
subprocess.run([b,'.','-t','json','-o',out]+sys.argv[3:],cwd=d,stderr=subprocess.DEVNULL,check=True)
g=json.load(open(out))
cs=g['callables']
name=lambda i:(cs[i]['qualified_name'] if i in cs else i)
rows=[]
for e in g['edges']:
    rows.append((e.get('site_line',0),'E',f"{name(e['src'])} -> {name(e['dst'])} [{e['confidence']},{e['resolver']},{e['via']['kind']}]"))
for u in g['unresolved']:
    rows.append((u.get('site_line',0),'U',f"{u.get('name')} recv={u.get('receiver_hint','')!r} reason={u.get('reason')}"))
for r in sorted(rows,key=lambda r:(str(r[0]),r[1],r[2])): print(*r)
