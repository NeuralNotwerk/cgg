#!/usr/bin/env bash
# scripts/unmerged-inventory.sh — list every piece of work that is NOT on
# main: open pull requests, unmerged local and origin branches, and every
# contributor fork's branches that are ahead of main.
#
# Why: 0.9.0 shipped without a finished Lean 4 plugin that sat on a
# contributor's fork as a branch, never opened as a PR. Checking only the
# PR list missed it. A release should be a decision about all outstanding
# work, so the release script runs this and treats any finding as a
# blocker unless the release acknowledges it (--allow-unmerged, or
# CGG_ALLOW_UNMERGED=1).
#
# Exit status: 0 when nothing is outstanding, 1 when something is (each
# item printed), 2 when GitHub could not be queried (never a silent pass).
set -uo pipefail
cd "$(dirname "$0")/.."
REPO="${CGG_GITHUB_REPO:-$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null)}"
[ -n "$REPO" ] || { echo "cannot determine the GitHub repository (gh not authenticated?)"; exit 2; }
git fetch -q origin 2>/dev/null
found=0
ACK="scripts/unmerged-acknowledged.txt"
acked() { [ -f "$ACK" ] && grep -v '^#' "$ACK" | grep -qF -- "$1 — "; }
say() {
    local key="$1"; shift
    if acked "$key"; then echo "  $key $* (acknowledged: $(grep -F -- "$key — " "$ACK" | head -1 | sed 's/.* — //'))"
    else echo "  $key $*"; found=1; fi
}

echo "open pull requests:"
prs=$(gh pr list --repo "$REPO" --state open --json number,title,author,headRefName \
      --jq '.[] | "#\(.number) \(.author.login):\(.headRefName) — \(.title)"' 2>/dev/null) || { echo "  (gh pr list failed)"; exit 2; }
[ -n "$prs" ] && while IFS= read -r l; do say "${l%% *}" "${l#* }"; done <<< "$prs"

echo "fork branches ahead of main:"
forks=$(gh api "repos/$REPO/forks" --paginate --jq '.[].full_name' 2>/dev/null) || { echo "  (fork list failed)"; exit 2; }
for f in $forks; do
    for b in $(gh api "repos/$f/branches" --paginate --jq '.[].name' 2>/dev/null); do
        ahead=$(gh api "repos/$REPO/compare/main...${f%%/*}:${b}" --jq .ahead_by 2>/dev/null || echo "?")
        # Content already on main (rebased or squashed) is not outstanding.
        [ "$ahead" = "0" ] && continue
        say "$f $b" "(ahead $ahead)"
    done
done

echo "origin branches not merged into main:"
for b in $(git branch -r --no-merged origin/main 2>/dev/null | grep '^  origin/' | sed 's#^  origin/##'); do
    [ "$(git cherry origin/main "origin/$b" | grep -c '^+')" = 0 ] && continue
    say "origin/$b"
done

if [ "$found" = 0 ]; then echo "nothing outstanding"; exit 0; fi
echo
echo "Outstanding work above is not in this release. Merge it, or acknowledge"
echo "it with --allow-unmerged (CGG_ALLOW_UNMERGED=1) and say why in the notes."
exit 1
