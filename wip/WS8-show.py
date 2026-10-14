import json,sys,subprocess,tempfile,os
bin_,d=sys.argv[1],sys.argv[2]
extra=sys.argv[3:]
out=tempfile.mktemp(suffix='.json',dir='/tmp/claude-1000')
subprocess.run([bin_,d,'-t','json','-o',out,*extra],stderr=subprocess.DEVNULL)
g=json.load(open(out))
c=g['callables']; f={k:os.path.basename(v['path'])+':'+v['language'] for k,v in g['files'].items()}
print('CALLABLES')
for v in sorted(c.values(),key=lambda v:(v['file'],v['start_line'])):
    print(f"  {f[v['file']]} {v['qualified_name']} [{v['kind']}] L{v['start_line']}-{v['end_line']} vis={v.get('visibility')} tags={v.get('tags','')}")
print('EDGES')
for e in g['edges']:
    s=c.get(e['src'],{}).get('qualified_name',e['src']); t=c.get(e['dst'],{}).get('qualified_name',e['dst'])
    print(f"  {s} -> {t} L{e.get('site_line')} {e.get('confidence')} {e.get('resolver')} {e.get('via','')}")
print('UNRESOLVED')
for u in g['unresolved']:
    s=c.get(u['src'],{}).get('qualified_name',u['src']) if u['src'] else None
    print(f"  {s} L{u['site_line']} {u['name']} {u['reason']}")
