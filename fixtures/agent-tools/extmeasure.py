"""Count edges whose call site is written through an EXTERNAL module/package
qualifier (per the caller file's own imports) but whose dst is a workspace callable.
Usage: extmeasure.py <g.json> <repo_root> [samples]"""
import json, sys, os, re, collections, glob
gpath, root = sys.argv[1], sys.argv[2]
nsample = int(sys.argv[3]) if len(sys.argv) > 3 else 0
g = json.load(open(gpath))
C, F = g['callables'], g['files']
src_cache = {}
def lines(fid):
    p = F[fid]['path']
    if p not in src_cache:
        try:
            src_cache[p] = [] if os.path.getsize(p) > 400000 else open(p, errors='replace').read().split('\n')
        except Exception: src_cache[p] = []
    return src_cache[p]

# ---- workspace vocab
all_paths = [f['path'] for f in F.values()]
rel = [os.path.relpath(p, root) for p in all_paths]
py_top = set()
for r in rel:
    parts = r.split(os.sep)
    for p in parts[:-1]: py_top.add(p)
    py_top.add(os.path.splitext(parts[-1])[0])
go_mods = []
for gm in glob.glob(os.path.join(root, '**/go.mod'), recursive=True):
    for l in open(gm, errors='replace'):
        if l.startswith('module '): go_mods.append(l.split()[1].strip())
js_pkgs = set()
for pj in glob.glob(os.path.join(root, '**/package.json'), recursive=True):
    if 'node_modules' in pj: continue
    try: n = json.load(open(pj)).get('name')
    except Exception: n = None
    if n: js_pkgs.add(n)
js_alias = set()
for tc in glob.glob(os.path.join(root, '**/tsconfig*.json'), recursive=True):
    if 'node_modules' in tc: continue
    txt = open(tc, errors='replace').read()
    for m in re.finditer(r'"(@?[\w./-]+?)/?\*?"\s*:\s*\[', txt):
        js_alias.add(m.group(1).rstrip('/*'))
java_pkgs = set()
cpp_ns = set()
for fid, f in F.items():
    lang = f['language']
    if lang in ('java', 'kotlin'):
        for l in lines(fid)[:60]:
            m = re.match(r'\s*package\s+([\w.]+)', l)
            if m: java_pkgs.add(m.group(1))
    if lang in ('cpp', 'c'):
        for l in lines(fid):
            for m in re.finditer(r'\bnamespace\s+([\w:]+)', l):
                for s in m.group(1).split('::'): cpp_ns.add(s)
owners = set()
for c in C.values():
    for s in re.split(r'::|\.', c['qualified_name'])[:-1]: owners.add(s)

def imports_for(fid):
    lang = F[fid]['language']; L = lines(fid); out = {}
    txt = '\n'.join(L)
    if lang == 'python':
        for m in re.finditer(r'^\s*import\s+([\w.]+)(?:\s+as\s+(\w+))?', txt, re.M):
            mod, al = m.group(1), m.group(2)
            out[al or mod.split('.')[0]] = mod if al else mod.split('.')[0]
        for m in re.finditer(r'^\s*from\s+([\w.]+)\s+import\s+\(?([^)\n]+)', txt, re.M) if False else []:
            mod = m.group(1)
            for part in m.group(2).split(','):
                part = part.strip().split('#')[0].strip()
                if not part: continue
                ps = part.split(' as ')
                out[ps[-1].strip()] = mod + '.' + ps[0].strip()
        def ext(spec): return not spec.startswith('.') and spec.split('.')[0] not in py_top
    elif lang in ('javascript', 'typescript'):
        for m in re.finditer(r'import\s+(?:\*\s+as\s+(\w+)|(\w+))(?:\s*,\s*\{[^}]*\})?\s+from\s+[\'"]([^\'"]+)', txt):
            out[m.group(1) or m.group(2)] = m.group(3)
        for m in []:
            for part in m.group(1).split(','):
                ps = [x.strip() for x in part.split(' as ')]
                if ps[-1]: out[ps[-1]] = m.group(2)
        for m in re.finditer(r'(?:const|let|var)\s+(\w+)\s*=\s*require\(\s*[\'"]([^\'"]+)', txt):
            out[m.group(1)] = m.group(2)
        def ext(spec):
            if spec.startswith(('.', '/', '~/', '#', '@/', 'src/', '$')): return False
            if any(r.endswith(sfx) for sfx in (spec, spec + '.js', spec + '.ts', spec + '/index.js', spec + '/index.ts') for r in rel): return False
            if any(spec == p or spec.startswith(p + '/') for p in js_pkgs): return False
            if any(spec == a or spec.startswith(a + '/') for a in js_alias if a): return False
            return True
    elif lang == 'go':
        for m in re.finditer(r'^\s*(?:import\s+)?(\w+\s+)?"([\w./-]+)"', txt, re.M):
            al = (m.group(1) or '').strip(); path = m.group(2)
            name = al or path.rstrip('/').split('/')[-1]
            if re.match(r'v\d+$', name): name = path.split('/')[-2]
            name = name.replace('-', '_').replace('go_', '').split('.')[0]
            out[name] = path
        def ext(spec): return not any(spec == mp or spec.startswith(mp + '/') for mp in go_mods)
    elif lang in ('java', 'kotlin'):
        for m in re.finditer(r'^\s*import\s+([\w.]+)', txt, re.M):
            out[m.group(1).split('.')[-1]] = m.group(1)
        def ext(spec):
            pkg = spec.rsplit('.', 1)[0]
            return not any(pkg == p or pkg.startswith(p + '.') for p in java_pkgs)
    else:
        def ext(spec): return False
    inner = ext
    def ext(spec, _l=lang):
        k = (_l, spec)
        if k not in EXT_CACHE: EXT_CACHE[k] = inner(spec)
        return EXT_CACHE[k]
    return out, ext
EXT_CACHE = {}

imp_cache = {}
counts = collections.Counter(); samples = []
total_xfile = 0
for e in g['edges']:
    if e['via']['kind'] != 'direct' or not e['resolver'].startswith('cross-file'): continue
    s, t = C.get(e['src']), C.get(e['dst'])
    if not s or not t: continue
    total_xfile += 1
    lang = F[s['file']]['language']
    L = lines(s['file'])
    if e['site_line'] - 1 >= len(L): continue
    text = L[e['site_line'] - 1]
    if len(text) > 400: continue
    name = t['simple_name']
    m = re.search(r'([A-Za-z_$][\w$]*(?:(?:\.|::)[A-Za-z_$][\w$]*)*)\s*(?:\.|::|->)\s*' + re.escape(name) + r'\s*[(<]', text)
    if not m: continue
    chain = m.group(1); head = re.split(r'\.|::', chain)[0]
    if s['file'] not in imp_cache: imp_cache[s['file']] = imports_for(s['file'])
    imps, ext = imp_cache[s['file']]
    kind = None
    if lang == 'cpp' or lang == 'c':
        if '::' in text[m.start():m.end()] and head not in cpp_ns and head not in owners:
            kind = 'cpp-external-namespace'
    elif lang in ('java', 'kotlin'):
        if head in imps and ext(imps[head]): kind = 'imported-external-class'
        elif head[:1].islower() and '.' in chain and head not in imps and not any(p.split('.')[0] == head for p in java_pkgs) and head not in ('this', 'super'):
            kind = 'fq-external-package' if head in ('java', 'javax', 'kotlin', 'kotlinx', 'org', 'com', 'io', 'android', 'androidx') else None
    elif head in imps and ext(imps[head]):
        kind = 'imported-external-module'
    if kind:
        counts[(kind, e['confidence'])] += 1
        samples.append((kind, e['confidence'], F[s['file']]['path'].replace(root, ''), e['site_line'], text.strip()[:90], t['qualified_name'], F[t['file']]['path'].replace(root, ''), (imps.get(head) if isinstance(imps, dict) else '')))
print(os.path.basename(root.rstrip('/')), 'cross-file direct edges:', total_xfile, 'external-head->workspace:', sum(counts.values()), dict(counts))
import random; random.seed(1)
for x in random.sample(samples, min(nsample, len(samples))): print('   ', x)
