"""classify.py AB_DIR [OUT_DIR]: compare every dead-code finding old vs new.

Key = (language, qualified_name, path, start_line). For each key whose
confidence or visibility evidence changed, assign a pattern class from
the visibility evidence transition and the signature shape, never from
names. Writes classes.tsv, samples.jsonl (all rows, for spot checks).
"""
import json, glob, os, re, sys, collections

ab = sys.argv[1]
out = sys.argv[2] if len(sys.argv) > 2 else ab

def vis_ev(f):
    for e in f['evidence']:
        if e['kind'] in ('private-visibility', 'public-visibility'):
            return (e['kind'].split('-')[0], e.get('token', ''))
    return ('none', '')

def load(p):
    try:
        d = json.load(open(p))
    except Exception:
        return None
    return {(f['language'], f['qualified_name'], f['path'], f['start_line']): f for f in d['findings']}

MODS = r'\b(public|protected|private|internal)\b'

def head_mods(sig):
    # modifier words appearing outside (...) / [...] groups, before the
    # name's own parameter list: strip balanced annotation/attribute args.
    s = sig
    prev = None
    while prev != s:
        prev = s
        s = re.sub(r'\([^()]*\)', ' ', s, count=1) if re.search(r'[@\[][\w.]+\s*\(', s) else s
        s = re.sub(r'\[[^\[\]]*\]', ' ', s)
    return re.findall(MODS, s.split('(')[0])

def klass(lang, fo, fn):
    vo, vn = vis_ev(fo), vis_ev(fn)
    sig = fn['signature_hint']
    has_args_ann = bool(re.match(r'\s*(@[\w.]+\s*\(|\[)', sig)) or bool(re.search(r'(^|\s)@[\w.]+\s*\(', sig.split(')')[0] if '@' in sig else ''))
    first_paren_cut = re.findall(MODS, sig.split('(')[0])
    if vn == ('public', 'protected'):
        if vo == ('none', ''):
            return 'protected-cap'
        return f'protected-cap-from-{vo[0]}:{vo[1]}'
    if vo[0] == 'private' and vn == ('public', 'public'):
        explicit = 'public' in head_mods(sig)
        cut_saw = 'public' in first_paren_cut
        if explicit and not cut_saw:
            return 'annotation-args-hid-public'
        if not explicit:
            return 'implicit-public-member(no-modifier)'
        return 'other-private->public'
    if vo[0] == 'private' and vn[0] == 'private' and vo[1] != vn[1]:
        return f'token-only {vo[1]}->{vn[1]}'
    if vo == ('none', '') and vn[0] in ('public', 'private') and lang in ('typescript', 'javascript'):
        return f'js-ts-capture:{vn[0]}:{vn[1]}'
    if vo != vn:
        return f'other {vo}->{vn}'
    return 'no-vis-change'

rows = []
cls = collections.Counter()
trans = collections.Counter()
for po in sorted(glob.glob(os.path.join(ab, 'old', '*.dead.json'))):
    repo = os.path.basename(po)[:-10]
    pn = os.path.join(ab, 'new', repo + '.dead.json')
    do, dn = load(po), load(pn)
    if do is None or dn is None:
        continue
    for k in sorted(set(do) | set(dn)):
        fo, fn = do.get(k), dn.get(k)
        if fo is None or fn is None:
            cls[(k[0], 'appeared' if fo is None else 'vanished', '')] += 1
            rows.append({'repo': repo, 'lang': k[0], 'qn': k[1], 'path': k[2], 'line': k[3], 'class': 'appeared' if fo is None else 'vanished'})
            continue
        co, cn = fo['confidence'], fn['confidence']
        if co == cn and vis_ev(fo) == vis_ev(fn):
            continue
        c = klass(k[0], fo, fn)
        t = f'{co}->{cn}'
        cls[(k[0], c, t)] += 1
        rows.append({'repo': repo, 'lang': k[0], 'qn': k[1], 'path': k[2], 'line': k[3],
                     'class': c, 'trans': t, 'sig': fn['signature_hint'][:160],
                     'old_vis': vis_ev(fo), 'new_vis': vis_ev(fn),
                     'kind': fn['kind'], 'variant': fn.get('def_variant', '')})
with open(os.path.join(out, 'classes.tsv'), 'w') as f:
    for (lang, c, t), n in sorted(cls.items(), key=lambda x: (-x[1])):
        f.write(f'{n}\t{lang}\t{c}\t{t}\n')
with open(os.path.join(out, 'samples.jsonl'), 'w') as f:
    for r in rows:
        f.write(json.dumps(r) + '\n')
print(open(os.path.join(out, 'classes.tsv')).read())
