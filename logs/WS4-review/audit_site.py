import json,sys
repo=sys.argv[1]; side=sys.argv[2]
targets=[(t.rsplit(':',1)[0],int(t.rsplit(':',1)[1])) for t in sys.argv[3:]]
a=json.load(open(f'/storage/cgg/target/remediation/ab/WS4-v5/{side}/{repo}.json.audit.json'))
for e in a:
    if e.get('event','').startswith('file_'): continue
    s=json.dumps(e)
    for frag,line in targets:
        if frag in s and (f'"line": {line},' in s or f'"line": {line}}}' in s):
            print(side,frag,line,s[:500])
