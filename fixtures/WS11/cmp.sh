#!/bin/bash
# usage: cmp.sh <fixture-dir>... : diff before vs after binary
B=/storage/cgg/target/remediation/bins/WS11-before
A=/storage/cgg/target/release/cgg
H=/storage/cgg/target/remediation/fixtures/WS11
for d in "$@"; do
  echo "== $d"
  python3 $H/show.py $B $H/$d > /tmp/claude-1000/ws11_b.txt
  python3 $H/show.py $A $H/$d > /tmp/claude-1000/ws11_a.txt
  diff /tmp/claude-1000/ws11_b.txt /tmp/claude-1000/ws11_a.txt
done
