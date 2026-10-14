import re,subprocess,sys
base=sys.argv[1]
files=subprocess.run(['git','diff','--name-only',base,'--','crates'],capture_output=True,text=True).stdout.split()
lit=re.compile(r'"((?:[^"\\]|\\.)*)"')
for f in files:
    if '/tests/' in f or not f.endswith('.rs'): continue
    try: src=open(f).read().split('\n')
    except FileNotFoundError: continue
    # test region: from first '#[cfg(test)]' line to EOF (convention in this repo)
    test_start=next((i for i,l in enumerate(src) if l.strip()=='#[cfg(test)]'),len(src))
    d=subprocess.run(['git','diff','-U0',base,'--',f],capture_output=True,text=True).stdout
    ln=0
    for l in d.split('\n'):
        m=re.match(r'@@ -\S+ \+(\d+)',l)
        if m: ln=int(m.group(1)); continue
        if l.startswith('+') and not l.startswith('+++'):
            idx=ln-1; ln+=1
            if idx>=test_start: continue
            s=l[1:].strip()
            if s.startswith('//'): continue
            code=s.split('//')[0] if '"' not in s.split('//')[0] else s
            for x in lit.findall(code):
                print(f"{f}:{ln-1}\t{x}")
        elif not l.startswith('-'): pass
