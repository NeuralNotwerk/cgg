#!/bin/bash
# rundc.sh REPO OUTDIR : bounded cgg dead-code json run
mkdir -p "$2"
timeout 60 /storage/cgg/target/release/cgg "$1" --dead-code --no-graph --dead-code-format json --dead-code-report "$2/dc.json" --dead-code-confidence low >/dev/null 2>"$2/err.txt"; echo "rc=$? $1"
