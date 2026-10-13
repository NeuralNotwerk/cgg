import json,sys,linecache
S=json.load(open(sys.argv[1]))
keys=sys.argv[2].split(';')
n=int(sys.argv[3]) if len(sys.argv)>3 else 6
for k in keys:
    v=S.get(k,[])
    print('#####',k,len(v))
    for r in v[:n]:
        ln=linecache.getline(r['site_path'],r['site_line']).strip()
        print(f"  {r['site_path'].split('test_repos/')[-1]}:{r['site_line']} | {ln[:120]}")
        print(f"      {r['src']} -> {r['dst']} @{r['dst_path'].split('test_repos/')[-1]}")
