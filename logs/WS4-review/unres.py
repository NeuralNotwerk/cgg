import json,sys
repo=sys.argv[1]
targets=[(t.rsplit(':',1)[0],int(t.rsplit(':',1)[1])) for t in sys.argv[2:]]
for side in ('old','new'):
    g=json.load(open(f'/storage/cgg/target/remediation/ab/WS4-v5/{side}/{repo}.json'))
    files={}
    fl=g['files']
    if isinstance(fl,dict): files={k:(v.get('path') if isinstance(v,dict) else v) for k,v in fl.items()}
    else: files={f.get('id'):f.get('path') for f in fl}
    for u in g['unresolved']:
        p=files.get(u['file'],'')
        for frag,line in targets:
            if frag in p and u['site_line']==line:
                print(side,frag,line,u['name'],json.dumps(u['reason']))
