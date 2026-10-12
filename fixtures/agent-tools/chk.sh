#!/bin/bash
# usage: chk.sh <fixture-dir> [extra cgg args...]
D=$1; shift
rm -f $D/g.json* $D/dc.txt $D/dc.json $D/g0.json*
timeout 60 /storage/cgg/target/release/cgg $D "$@" -t json -o $D/g.json --dead-code --dead-code-format text --dead-code-report $D/dc.txt 2>$D/stderr.txt
timeout 60 /storage/cgg/target/release/cgg $D "$@" -t json -o /dev/null --metrics /dev/null --dead-code --dead-code-format json --dead-code-confidence low --dead-code-report $D/dc.json 2>/dev/null
python3 -I $(dirname $0)/summ.py $D
if [ -n "$CAP0" ]; then
  timeout 60 /storage/cgg/target/release/cgg $D "$@" --fanout-cap 0 -t json -o $D/g0.json 2>/dev/null
  python3 -I -c "
import json,sys
g=json.load(open('$D/g0.json'));C=g['callables']
q=lambda i:C[i]['qualified_name'] if i in C else i
print('== edges with --fanout-cap 0')
for e in sorted(g['edges'],key=lambda e:e.get('site_line') or 0): print(f\"   L{e.get('site_line')}: {q(e['src'])} -> {q(e['dst'])} [{e['confidence']}]\")
"
fi
