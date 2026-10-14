import json,collections,sys
a=json.load(open(sys.argv[1]))
want=sys.argv[2] if len(sys.argv)>2 else None
c=collections.Counter(); rows=[]
for fa in a:
  if fa.get('event')!='file_analyzed': continue
  for bucket in ('unresolved_calls','external_calls','stdlib_calls'):
    for u in fa.get(bucket,[]):
      r=u['reason']; key=r.get('stage') if isinstance(r,dict) else r
      c[(bucket,key)]+=1
      if want and key==want: rows.append((fa['path'],u['site_line'],u['name'],u.get('receiver_hint'),bucket,json.dumps(r)))
for k,v in c.most_common(60): print(v,k)
for r in rows[:80]: print(*r)
