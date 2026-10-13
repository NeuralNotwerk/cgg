import csv, sys, collections
def load(d):
    rows = list(csv.DictReader(open(f"{d}/by_language.tsv"), delimiter="\t"))
    return {r[list(r.keys())[0]]: r for r in rows}
a, b = load(sys.argv[1]), load(sys.argv[2])
keys = list(next(iter(a.values())).keys())
print("\t".join(["lang"] + [f"{k}:v8r/v9" for k in keys[1:]]))
for lang in sorted(set(a) | set(b)):
    ra, rb = a.get(lang, {}), b.get(lang, {})
    vals = [f"{ra.get(k,'-')}/{rb.get(k,'-')}" for k in keys[1:]]
    if any(x.split('/')[0] != x.split('/')[1] for x in vals):
        print("\t".join([lang] + vals))
