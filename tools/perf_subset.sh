#!/usr/bin/env bash
#
# perf_subset.sh — paired before/after latency on a chosen repo subset.
#
# Usage: perf_subset.sh OLD_BIN NEW_BIN RUNS repo [repo ...]
#
# Same method as scripts/perf-compare.sh, minus the worktree build: one
# discarded warm-up per binary per repo, then RUNS samples each with the
# binaries ALTERNATING (which goes first flips per repo), median taken.
# `-t json -o /dev/null` at the default --jobs, exactly as perf-compare.
#
# Bounded: CGG_REPO_TIMEOUT (60) per run, CGG_TOTAL_BUDGET (1800) overall.
# A repo either binary cannot finish under the cap is NAMED and excluded.
# Deltas under 1000 ms are reported but marked "noise" — the remediation
# rules say never chase a sub-second deviation.
#
# Run this ALONE on the machine: it is a timing measurement, and any
# concurrent cargo build / graph_ab run invalidates it. Load is printed.

set -uo pipefail
OLD="$1"; NEW="$2"; RUNS="$3"; shift 3
BENCH="${CGG_BENCH_DIR:-/storage/cgg-test_repos}"
T="${CGG_REPO_TIMEOUT:-60}"
BUDGET="${CGG_TOTAL_BUDGET:-1800}"
START=$(date +%s)

median() { sort -n | awk '{a[NR]=$1} END{print a[int((NR+1)/2)]}'; }
one() { local st en; st=$(date +%s%N)
  timeout "$T" "$1" "$2" -t json -o /dev/null >/dev/null 2>&1; local rc=$?
  en=$(date +%s%N); [ $rc -eq 124 ] && { echo TIMEOUT; return; }
  echo $(( (en - st) / 1000000 )); }

echo "old : $OLD ($("$OLD" --version))"
echo "new : $NEW ($("$NEW" --version))"
echo "runs: median of $RUNS, alternating; load: $(uptime | sed 's/.*load average: //')"
echo
printf '| repo | old ms | new ms | delta ms | delta %% | note |\n|---|---|---|---|---|---|\n'
tot_o=0; tot_n=0; flip=0; excluded=()
for r in "$@"; do
  p="$BENCH/$r"; [ -d "$p" ] || p="/storage/$r"
  [ -d "$p" ] || { excluded+=("$r (missing)"); continue; }
  if [ $(( $(date +%s) - START )) -ge "$BUDGET" ]; then excluded+=("$r (budget)"); continue; fi
  wo=$(one "$OLD" "$p"); wn=$(one "$NEW" "$p")
  if [ "$wo" = TIMEOUT ] || [ "$wn" = TIMEOUT ]; then excluded+=("$r (timeout old=$wo new=$wn)"); continue; fi
  so=(); sn=()
  for i in $(seq 1 "$RUNS"); do
    if [ $(( (i + flip) % 2 )) -eq 0 ]; then so+=("$(one "$OLD" "$p")"); sn+=("$(one "$NEW" "$p")")
    else sn+=("$(one "$NEW" "$p")"); so+=("$(one "$OLD" "$p")"); fi
  done
  flip=$((flip + 1))
  mo=$(printf '%s\n' "${so[@]}" | median); mn=$(printf '%s\n' "${sn[@]}" | median)
  d=$((mn - mo)); pct=$(awk -v o="$mo" -v n="$mn" 'BEGIN{ if (o>0) printf "%+.1f", (n-o)*100/o; else print "n/a"}')
  note=""; [ ${d#-} -lt 1000 ] && note="noise(<1s)"
  printf '| %s | %s | %s | %+d | %s | %s |\n' "$r" "$mo" "$mn" "$d" "$pct" "$note"
  tot_o=$((tot_o + mo)); tot_n=$((tot_n + mn))
done
echo
awk -v o="$tot_o" -v n="$tot_n" 'BEGIN{ printf "total: %d ms -> %d ms (%+d ms, %+.1f%%)\n", o, n, n-o, (o>0)?(n-o)*100/o:0 }'
for x in "${excluded[@]}"; do echo "EXCLUDED: $x"; done
echo "elapsed: $(( $(date +%s) - START ))s"
