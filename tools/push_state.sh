#!/usr/bin/env bash
#
# push_state.sh — snapshot the remediation state (tracker, findings,
# protocol, tools, fixtures, A/B + perf summaries) onto the orphan branch
# `remediation-state` and push it to origin and backup.
#
# The box's NVMe is failing; this keeps everything needed to resume the
# remediation off-box. It never touches the main working tree or index:
# it stages into a throwaway directory with its own GIT_INDEX_FILE and
# writes the commit with plumbing (write-tree / commit-tree / update-ref).
#
# Excluded: binaries (bins/), full graph outputs (ab/*/old, ab/*/new),
# anything over 20 MB.
#
# Usage: push_state.sh "message"

set -euo pipefail
REPO=/storage/cgg
SRC=$REPO/target/remediation
MSG="${1:-remediation state}"
BR=remediation-state
STAGE=$(mktemp -d -p "$REPO/target" rstate-XXXXXX)
IDX=$(mktemp -p "$REPO/target" rstate-index-XXXXXX)
trap 'rm -rf "$STAGE" "$IDX"' EXIT
rm -f "$IDX"

rsync -a --prune-empty-dirs \
  --exclude='bins/' --exclude='target/' --exclude='*.rlib' --exclude='*.o' --exclude='*.a' --exclude='ab/*/old/' --exclude='ab/*/new/' \
  --max-size=20m "$SRC/" "$STAGE/"
mkdir -p "$STAGE/docs"
for f in FIX_PLAN.md EDGE_CASE_TAXONOMY.md; do
  [ -f "$REPO/$f" ] && cp "$REPO/$f" "$STAGE/docs/$f"
done

export GIT_DIR=$REPO/.git GIT_INDEX_FILE=$IDX GIT_WORK_TREE=$STAGE
git add -A .
TREE=$(git write-tree)
PARENT=$(git rev-parse -q --verify "refs/heads/$BR" || true)
if [ -n "$PARENT" ] && [ "$(git rev-parse "$PARENT^{tree}")" = "$TREE" ]; then
  echo "state unchanged ($PARENT)"
else
  if [ -n "$PARENT" ]; then
    C=$(git commit-tree "$TREE" -p "$PARENT" -m "$MSG")
  else
    C=$(git commit-tree "$TREE" -m "$MSG")
  fi
  git update-ref "refs/heads/$BR" "$C"
  echo "state committed $C"
fi
unset GIT_INDEX_FILE GIT_WORK_TREE
for r in origin backup; do
  git -C "$REPO" push -q ${FORCE:+--force} "$r" "refs/heads/$BR:refs/heads/$BR" && echo "pushed $BR -> $r"
done
