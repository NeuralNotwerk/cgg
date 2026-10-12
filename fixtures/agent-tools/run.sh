#!/bin/bash
# usage: run.sh <fixture-dir>
B=/storage/cgg/target/release/cgg
f=$1; n=$(basename $f); o=$(dirname $f)/_out/$n; mkdir -p $o
timeout 60 $B $f -t json -o $o/g.json --dead-code --dead-code-format json --dead-code-confidence low --dead-code-report $o/dc.json 2>$o/stderr.txt
timeout 60 $B $f -t json -o $o/g.json.txtrun --dead-code --dead-code-format text --dead-code-report $o/dc.txt 2>/dev/null
timeout 60 $B $f -t json -o $o/gdd.json --dynamic-dispatch 2>/dev/null
timeout 60 $B $f -t json -o $o/gplain.json 2>/dev/null
