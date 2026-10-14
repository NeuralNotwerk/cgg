#!/bin/bash
# usage: ab_when_free.sh OLD NEW OUTDIR
while true; do
  a=$(free -g | awk '/^Mem:/{print $7}')
  if [ "$a" -ge 40 ]; then break; fi
  echo "$(date +%T) waiting: available ${a} GB" 
  sleep 60
done
echo "$(date +%T) starting A/B with ${a} GB available"
cd /storage/cgg
python3 target/remediation/tools/graph_ab.py "$1" "$2" "$3" --parallel 6
