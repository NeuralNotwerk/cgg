#!/bin/bash
# usage: entries.sh BIN DIR [flags] -> framework-entry edges "entry -> target  (route/site)"
bin=$1; dir=$2; shift 2
"$bin" "$dir" -t json "$@" 2>/dev/null | python3 -c '
import json,sys
d=json.load(sys.stdin); c=d["callables"]
for e in sorted(d["edges"], key=lambda e:(c[e["src"]]["qualified_name"], c[e["dst"]]["qualified_name"])):
    s=c[e["src"]]["qualified_name"]
    if s.startswith("<framework"):
        print(s, "->", c[e["dst"]]["qualified_name"], e["confidence"])
'
