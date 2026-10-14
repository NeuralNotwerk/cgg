#!/usr/bin/env bash
# Snapshot the uncommitted working tree onto wip/local-state without touching
# the real index or working tree. origin (public) gets code only; backup
# (private) also gets the untracked planning docs.
set -euo pipefail
cd /storage/cgg
export GIT_INDEX_FILE="$(mktemp -u /tmp/claude-1000/cgg-wip-idx.XXXX)"
trap 'rm -f "$GIT_INDEX_FILE"' EXIT
git read-tree HEAD
git add -A -- crates CHANGELOG.md
A=$(git commit-tree "$(git write-tree)" -p HEAD -m "wip: local working state snapshot $(date -u +%FT%TZ) (in progress, untested)

Backup of the uncommitted working tree on top of $(git rev-parse --short HEAD); not a reviewed commit.")
git add -f EDGE_CASE_TAXONOMY.md FIX_PLAN.md
B=$(git commit-tree "$(git write-tree)" -p "$A" -m "wip: untracked planning docs (private backup only)")
git push -q -f origin "$A:refs/heads/wip/local-state"
git push -q -f backup "$B:refs/heads/wip/local-state"
echo "wip/local-state: origin=$(git rev-parse --short "$A") backup=$(git rev-parse --short "$B")"
