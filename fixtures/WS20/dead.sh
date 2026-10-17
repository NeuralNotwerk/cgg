#!/bin/bash
# usage: dead.sh BIN DIR [extra flags]  -> one line per finding: confidence qn path:line evidence-kinds
bin=$1; dir=$2; shift 2
"$bin" "$dir" --dead-code --no-graph --dead-code-format json "$@" 2>/dev/null | python3 -c '
import json,sys
d=json.load(sys.stdin)
for f in sorted(d["findings"], key=lambda f:(f["path"],f["start_line"])):
    print(f["confidence"].ljust(6), f["qualified_name"], "%s:%d"%(f["path"],f["start_line"]), ",".join(e["kind"] for e in f["evidence"]))
'
