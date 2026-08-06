#!/usr/bin/env bash
#
# scripts/perf-compare.sh — measure this release's latency against the
# previous one, for the CHANGELOG's `### Performance` section.
#
# When to run (manual, release-time):
#   - Before every version bump. The `push` skill requires a
#     `### Performance` block in each CHANGELOG entry, and this is what
#     produces the numbers. Unmeasured perf claims are not shipped.
#
# What it does:
#   1. Checks out the baseline ref into a **separate git worktree**, so
#      your working tree — including uncommitted changes — is never
#      touched. This is not optional: the whole point is to measure a
#      dirty tree against a clean baseline.
#   2. Builds both binaries from source.
#   3. Runs each over the same repo set, taking a median of N runs —
#      after a discard warmup pass per binary, and alternating which
#      binary goes first. Both matter: measuring one binary's full set
#      and then the other's lets the second run against a hot page
#      cache, which fabricated a 4% improvement the first time this was
#      done by hand.
#   4. Prints a markdown table ready to paste into CHANGELOG.md.
#
# Usage:
#   scripts/perf-compare.sh [BASELINE_REF] [RUNS]
#     BASELINE_REF  git ref of the previous release (default: latest
#                   `chore: release` commit reachable from HEAD)
#     RUNS          samples per repo, median taken (default 7)
#
#   CGG_BENCH_DIR   corpus location (default /storage/cgg-test_repos)
#
# Caveats it prints, because they change how the numbers read:
#   - A loaded machine invalidates the comparison. Load is reported.
#   - Small repos (<150ms) have a few ms of noise; treat sub-3% moves on
#     those as nothing.
#   - Features that are ON BY DEFAULT in the new version but absent in
#     the baseline are not like-for-like. Note them in the CHANGELOG
#     rather than presenting the delta as pure overhead.

set -uo pipefail

REPOS_DIR="${CGG_BENCH_DIR:-/storage/cgg-test_repos}"
RUNS="${2:-7}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORKTREE="$(mktemp -d -t cgg-perf-baseline-XXXXXX)"

# Language-diverse and mid-sized: big enough to dominate process
# startup, small enough that 7 runs each stays under a few minutes.
REPOS=(rust-ripgrep python-flask js-express go-fzf c-jq cpp-spdlog
       csharp-serilog swift-alamofire cpp-nlohmann-json)

BASELINE="${1:-}"
if [ -z "$BASELINE" ]; then
  BASELINE=$(git -C "$ROOT" log --oneline --grep='^chore: release' -n 1 --format=%H)
  [ -z "$BASELINE" ] && { echo "error: no 'chore: release' commit found; pass a ref explicitly" >&2; exit 1; }
fi

cleanup() { git -C "$ROOT" worktree remove --force "$WORKTREE" >/dev/null 2>&1 || true; }
trap cleanup EXIT

echo "# Performance comparison"
echo
echo "baseline : $(git -C "$ROOT" log --oneline -1 "$BASELINE")"
echo "current  : $(git -C "$ROOT" log --oneline -1 HEAD)$(git -C "$ROOT" diff --quiet || echo ' + uncommitted changes')"
echo "samples  : median of $RUNS per repo"
echo "load     : $(uptime | sed 's/.*load average: //')"
echo

# --- build baseline in an isolated worktree -----------------------------
echo "building baseline in $WORKTREE ..." >&2
git -C "$ROOT" worktree add --detach "$WORKTREE" "$BASELINE" >/dev/null 2>&1 || {
  echo "error: could not create worktree at $WORKTREE" >&2; exit 1; }
( cd "$WORKTREE" && cargo build --release -p cgg >/dev/null 2>&1 ) || {
  echo "error: baseline build failed" >&2; exit 1; }
OLD_BIN="$WORKTREE/target/release/cgg"

echo "building current ..." >&2
( cd "$ROOT" && cargo build --release -p cgg >/dev/null 2>&1 ) || {
  echo "error: current build failed" >&2; exit 1; }
NEW_BIN="$ROOT/target/release/cgg"

echo "  baseline: $("$OLD_BIN" --version)" >&2
echo "  current : $("$NEW_BIN" --version)" >&2

median() { sort -n | awk '{a[NR]=$1} END{print a[int((NR+1)/2)]}'; }

warmup() { # $1=bin $2=path — discard run, page cache only
  "$1" "$2" -t json -o /dev/null --no-update-check >/dev/null 2>&1
}

bench_one() { # $1=bin $2=path
  local i st en
  for i in $(seq 1 "$RUNS"); do
    st=$(date +%s%N)
    "$1" "$2" -t json -o /dev/null --no-update-check >/dev/null 2>&1
    en=$(date +%s%N)
    echo $(( (en - st) / 1000000 ))
  done | median
}

printf '| repo | %s | %s | delta |\n' "$("$OLD_BIN" --version | awk '{print $2}')" "$("$NEW_BIN" --version | awk '{print $2}')"
printf '|---|---|---|---|\n'

tot_old=0; tot_new=0; flip=0
for r in "${REPOS[@]}"; do
  [ -d "$REPOS_DIR/$r" ] || continue
  # Warm both, then alternate which is timed first.
  warmup "$OLD_BIN" "$REPOS_DIR/$r"; warmup "$NEW_BIN" "$REPOS_DIR/$r"
  if [ $((flip % 2)) -eq 0 ]; then
    o=$(bench_one "$OLD_BIN" "$REPOS_DIR/$r"); n=$(bench_one "$NEW_BIN" "$REPOS_DIR/$r")
  else
    n=$(bench_one "$NEW_BIN" "$REPOS_DIR/$r"); o=$(bench_one "$OLD_BIN" "$REPOS_DIR/$r")
  fi
  flip=$((flip + 1))
  pct=$(awk -v a="$o" -v b="$n" 'BEGIN{if(a>0) printf "%+.1f", (b-a)*100/a; else print "n/a"}')
  # Flag anything under ~150ms: a few ms of noise is a large percentage.
  noise=""
  awk -v a="$o" 'BEGIN{exit !(a<150)}' && noise=" ⚠noise"
  printf '| %s | %s ms | %s ms | %s%%%s |\n' "$r" "$o" "$n" "$pct" "$noise"
  tot_old=$((tot_old + o)); tot_new=$((tot_new + n))
done

pct=$(awk -v a="$tot_old" -v b="$tot_new" 'BEGIN{printf "%+.1f", (b-a)*100/a}')
printf '| **TOTAL** | **%s ms** | **%s ms** | **%s%%** |\n' "$tot_old" "$tot_new" "$pct"
echo
echo "⚠noise = baseline under 150ms; a few ms of jitter reads as several percent."
echo
echo "MEASUREMENT NOISE FLOOR IS ROUGHLY 1-1.5% ON THIS TOTAL. Two identical"
echo "runs of the same commits differ by that much. Do not report a total"
echo "delta under ~2% as an improvement or a regression — report it as flat."
echo "To claim a real change, run this twice and check the spread first."
echo
echo "If the new version enables work by default that the baseline lacked,"
echo "say so in the CHANGELOG — the delta is not pure overhead."
