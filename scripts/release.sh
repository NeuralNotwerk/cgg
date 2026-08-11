#!/usr/bin/env bash
#
# scripts/release.sh — one command to take cgg from "work done" to
# "ready to tag", running every gate this project has learned to need.
#
#   scripts/release.sh --purpose "what this release is for" [options]
#
# WHAT IT DOES, in order. The order is the point: nothing writes prose
# until the numbers exist, because every documentation defect this
# project has shipped came from describing a run that was never made.
#
#   0. preflight   tools present, tree state understood, version resolved
#   1. gates       build, test, clippy, fmt, docs-check, determinism
#   2. measure     perf vs the previous release, corpus stats, coverage
#   3. document    headless Claude drafts the CHANGELOG and audits README
#                  claims — from the MEASURED numbers and the real diff
#   4. verify      re-run the gates over whatever step 3 wrote
#   5. report      print what is ready and what is blocked
#
# IT NEVER COMMITS, TAGS OR PUSHES. It prints the commands and stops.
# A release is a decision; this script only removes the excuses for
# making it badly.
#
# Options:
#   --purpose TEXT     one line on what this release is for. Required
#                      unless --skip-ai: it is what stops the generated
#                      CHANGELOG being a list of diffs with no thesis.
#   --version X.Y.Z    version to release. Default: current Cargo.toml.
#   --baseline REF     git ref or path to a binary to measure against.
#                      Default: the newest v* tag.
#   --skip-ai          run every gate and measurement, write no prose.
#   --skip-perf        skip the corpus comparison (slow; needs a corpus).
#   --quick            gates only. For "is it broken right now".
#   --out DIR          where artifacts land. Default: target/release-prep.
#
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

PURPOSE=""
VERSION=""
BASELINE=""
SKIP_AI=0
SKIP_PERF=0
QUICK=0
OUT="$ROOT/target/release-prep"

while [ $# -gt 0 ]; do
    case "$1" in
        --purpose)   PURPOSE="${2:-}"; shift 2 ;;
        --version)   VERSION="${2:-}"; shift 2 ;;
        --baseline)  BASELINE="${2:-}"; shift 2 ;;
        --out)       OUT="${2:-}"; shift 2 ;;
        --skip-ai)   SKIP_AI=1; shift ;;
        --skip-perf) SKIP_PERF=1; shift ;;
        --quick)     QUICK=1; SKIP_PERF=1; SKIP_AI=1; shift ;;
        -h|--help)   sed -n '2,40p' "$0"; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done

mkdir -p "$OUT"
BLOCKERS="$OUT/blockers.txt"
: > "$BLOCKERS"

# ---------------------------------------------------------------------
# Output helpers. A gate that fails is recorded and the run continues —
# one broken gate should not hide the other nine.
# ---------------------------------------------------------------------
step()  { printf '\n\033[1m== %s\033[0m\n' "$*"; }
ok()    { printf '   \033[32mok\033[0m   %s\n' "$*"; }
warn()  { printf '   \033[33mwarn\033[0m %s\n' "$*"; }
fail()  { printf '   \033[31mFAIL\033[0m %s\n' "$*"; echo "$*" >> "$BLOCKERS"; }
have()  { command -v "$1" >/dev/null 2>&1; }

# Run a gate: name, then the command. Non-zero is a blocker.
gate() {
    local name="$1"; shift
    local log="$OUT/$(echo "$name" | tr ' /' '__').log"
    if "$@" > "$log" 2>&1; then
        ok "$name"
        return 0
    fi
    fail "$name — see $log"
    tail -5 "$log" | sed 's/^/        /'
    return 1
}

# =====================================================================
step "0. preflight"
# =====================================================================
for t in cargo git python3; do
    have "$t" || { fail "missing required tool: $t"; }
done
have cc || have gcc || have clang || \
    warn "no C compiler found — tree-sitter grammars and mimalloc need one"

CUR_VERSION="$(grep -m1 '^version = ' Cargo.toml | cut -d'"' -f2)"
VERSION="${VERSION:-$CUR_VERSION}"
ok "version: $VERSION (Cargo.toml says $CUR_VERSION)"
[ "$VERSION" != "$CUR_VERSION" ] && \
    warn "Cargo.toml is $CUR_VERSION; bump it before tagging $VERSION"

# Internal crate pins must track the workspace version or `cargo build`
# fails in a way that is confusing at exactly the wrong moment.
BADPIN="$(grep -E '^cgg(-[a-z]+)? *= *\{ *path' Cargo.toml | grep -v "version = \"$CUR_VERSION\"" || true)"
[ -n "$BADPIN" ] && fail "internal crate pins disagree with $CUR_VERSION:
$BADPIN"


if [ -n "$(git status --porcelain 2>/dev/null)" ]; then
    warn "working tree is dirty — that is normal mid-release, but the diff
        below (and any generated prose) describes the tree, not a commit"
fi

if [ -z "$BASELINE" ]; then
    BASELINE="$(git tag --list 'v*' --sort=-v:refname | head -1)"
fi
[ -n "$BASELINE" ] && ok "baseline: $BASELINE" || warn "no baseline tag found"

# =====================================================================
step "1. gates"
# =====================================================================
gate "build (release)"        cargo build --release -p cgg
gate "test (workspace)"       cargo test --workspace
gate "clippy"                 cargo clippy --workspace --all-targets -- -D warnings
gate "fmt"                    cargo fmt --all --check
gate "docs-check"             python3 scripts/docs-check.py
gate "detect-prefixes"        cargo test -p cgg --test detect_prefixes
gate "determinism"            cargo test -p cgg --test determinism

if have cargo-deny; then
    gate "cargo-deny" cargo deny check
else
    warn "cargo-deny not installed — licence and advisory audit skipped
        (cargo install cargo-deny --locked)"
fi

if have npx; then
    gate "markdownlint" npx --yes markdownlint-cli2 README.md CHANGELOG.md
else
    warn "npx not available — markdown lint skipped"
fi

if have ruff; then
    gate "ruff (scripts)" ruff check scripts/
else
    warn "ruff not installed — python lint skipped"
fi

[ "$QUICK" = 1 ] && step "quick mode: skipping measurement and prose"

# =====================================================================
step "2. measure"
# =====================================================================
FACTS="$OUT/facts.md"
: > "$FACTS"

{
    echo "# Measured facts for $VERSION"
    echo
    echo "Baseline: ${BASELINE:-none}"
    echo "Generated: by scripts/release.sh. Every number below was measured"
    echo "on this machine, on this tree. Nothing here is estimated."
    echo
} >> "$FACTS"

if [ "$QUICK" != 1 ]; then
    # Framework coverage is cheap enough to always run and is the gate
    # most likely to have silently rotted.
    if gate "framework-coverage" python3 scripts/framework-coverage.py \
        --json "$OUT/coverage.json"; then
        {
            echo "## Framework coverage"
            echo '```text'
            tail -20 "$OUT/framework-coverage.log"
            echo '```'
            echo
        } >> "$FACTS"
    fi

    # Self-analysis: the smallest honest performance datapoint, and it
    # needs no corpus.
    SELF="$($ROOT/target/release/cgg ./crates -t mermaid -o /dev/null 2>&1 | tail -1 || true)"
    [ -n "$SELF" ] && { echo "## Self-analysis"; echo '```text'; echo "$SELF"; echo '```'; echo; } >> "$FACTS"
fi

if [ "$SKIP_PERF" != 1 ] && [ -n "$BASELINE" ]; then
    # Build the baseline in a worktree so the working tree is untouched.
    BASE_BIN="$OUT/cgg-baseline"
    if [ -x "$BASELINE" ]; then
        cp "$BASELINE" "$BASE_BIN"
        ok "baseline binary: $BASELINE"
    else
        WT="$(mktemp -d -t cgg-baseline-XXXXXX)"
        if git worktree add -q "$WT" "$BASELINE" 2>/dev/null; then
            ( cd "$WT" && cargo build --release -p cgg ) > "$OUT/baseline-build.log" 2>&1 \
                && cp "$WT/target/release/cgg" "$BASE_BIN" \
                || warn "baseline build failed — see $OUT/baseline-build.log"
            git worktree remove --force "$WT" >/dev/null 2>&1 || true
        else
            warn "could not check out baseline $BASELINE"
        fi
    fi

    if [ -x "$BASE_BIN" ]; then
        step "2b. corpus comparison (this is the slow part)"
        if python3 scripts/compare-release.py "$BASE_BIN" "$ROOT/target/release/cgg" \
             --jobs 1 --out "$OUT/compare.json" > "$OUT/compare.log" 2>&1; then
            ok "corpus comparison"
            {
                echo "## Latency and graph output vs $BASELINE"
                echo
                echo "Measured with --jobs 1, the setting for published numbers."
                echo '```text'
                tail -25 "$OUT/compare.log"
                echo '```'
                echo
            } >> "$FACTS"
        else
            fail "corpus comparison — see $OUT/compare.log"
        fi
    fi
fi

ok "facts written to $FACTS ($(wc -l < "$FACTS") lines)"

# =====================================================================
step "3. document"
# =====================================================================
#
# Headless Claude drafts prose FROM the measured facts and the real
# diff. Three rules are baked into every prompt, because each maps to a
# defect this project actually shipped:
#
#   * Never invent a number. If it is not in facts.md, do not state it.
#   * Disclose defects prominently, especially ones already released.
#   * Say what could not be verified, rather than implying completeness.
#
DIFFSTAT="$OUT/diffstat.txt"
DIFFBODY="$OUT/diff.txt"
# Both halves, always. Mid-release the tree is normally dirty and the
# whole release may still be uncommitted — capturing only `baseline..HEAD`
# then hands the drafting step an EMPTY diff and it writes a release note
# about nothing. Untracked files are included for the same reason: a new
# module is exactly the kind of thing the notes must mention.
: > "$DIFFSTAT"; : > "$DIFFBODY"
if [ -n "$BASELINE" ] && git rev-parse "$BASELINE" >/dev/null 2>&1; then
    {
        echo "### committed since $BASELINE"
        git diff --stat "$BASELINE"..HEAD
    } >> "$DIFFSTAT" 2>/dev/null
    git diff "$BASELINE"..HEAD -- ':!*.lock' >> "$DIFFBODY" 2>/dev/null
fi
{
    echo "### uncommitted in the working tree"
    git diff --stat HEAD
} >> "$DIFFSTAT" 2>/dev/null
git diff HEAD -- ':!*.lock' >> "$DIFFBODY" 2>/dev/null
UNTRACKED="$(git ls-files --others --exclude-standard | grep -vE '\.lock$' || true)"
if [ -n "$UNTRACKED" ]; then
    { echo "### new files (untracked)"; echo "$UNTRACKED"; } >> "$DIFFSTAT"
    echo "$UNTRACKED" | while read -r f; do
        [ -f "$f" ] && { echo "=== NEW FILE: $f ==="; head -120 "$f"; } >> "$DIFFBODY"
    done
fi
ok "diff captured ($(wc -l < "$DIFFBODY") lines)"

ask_claude() {
    # $1 = task name, $2 = output file, $3 = prompt (on stdin)
    local name="$1" outfile="$2"
    if ! have claude; then
        warn "claude CLI not found — skipping: $name"
        return 1
    fi
    if timeout 1800 claude -p --permission-mode acceptEdits \
        > "$outfile" 2>"$OUT/$name.err"; then
        if [ -s "$outfile" ]; then
            ok "$name -> $outfile"
            return 0
        fi
        warn "$name produced nothing"
        return 1
    fi
    warn "$name failed — see $OUT/$name.err"
    return 1
}

if [ "$SKIP_AI" = 1 ]; then
    warn "--skip-ai: no prose generated"
elif [ -z "$PURPOSE" ]; then
    fail "--purpose is required unless --skip-ai.
        Without it the generated CHANGELOG is a list of diffs with no thesis;
        the one thing a human must supply is what the release is FOR."
else
    RULES='RULES, all three non-negotiable:
1. NEVER invent a number. Every figure you state must appear in the FACTS
   section below. If a number is not there, do not state it — say it was
   not measured. Fabricated benchmark numbers are the worst possible
   output of this script.
2. Disclose defects prominently, and put anything that was already
   RELEASED in a callout at the top of its section. A user who hit the
   bug needs to find it without reading the whole entry.
3. State what was NOT verified. Partial coverage described as complete is
   the failure mode this project cares most about.
Write in the voice of the existing CHANGELOG.md: plain, specific, no
marketing. Prefer a table to a list of adjectives. Explain WHY a change
matters, not just what changed.'

    step "3a. CHANGELOG entry"
    {
        echo "You are drafting the CHANGELOG entry for cgg $VERSION."
        echo
        echo "PURPOSE OF THIS RELEASE (supplied by the maintainer):"
        echo "$PURPOSE"
        echo
        echo "$RULES"
        echo
        echo "Read /storage/cgg/CHANGELOG.md for the house style and read the"
        echo "top two existing entries before writing. Match their structure."
        echo
        echo "=== FACTS (the only numbers you may use) ==="
        cat "$FACTS"
        echo
        echo "=== DIFF SUMMARY ==="
        head -200 "$DIFFSTAT"
        echo
        echo "=== DIFF (truncated) ==="
        head -3000 "$DIFFBODY"
        echo
        echo "Output ONLY the markdown for the new '## [$VERSION]' section."
        echo "Do not edit any file; print the section to stdout."
    } | ask_claude "changelog" "$OUT/CHANGELOG-draft.md"

    step "3b. README claim audit"
    {
        echo "You are auditing README.md for claims that cgg $VERSION has"
        echo "made false. Do NOT rewrite the README wholesale."
        echo
        echo "PURPOSE OF THIS RELEASE: $PURPOSE"
        echo
        echo "$RULES"
        echo
        echo "Read /storage/cgg/README.md. For each claim the diff below"
        echo "invalidates, output one entry:"
        echo "  FILE:LINE | CURRENT TEXT | CORRECTED TEXT | WHY"
        echo "Look especially for: counts (languages, frameworks, packages),"
        echo "performance figures, statements about threading or determinism,"
        echo "the flag table and usage synopsis, and dependency claims."
        echo "If a claim is still true, do not mention it."
        echo
        echo "=== FACTS ==="
        cat "$FACTS"
        echo
        echo "=== DIFF SUMMARY ==="
        head -200 "$DIFFSTAT"
        echo
        echo "Output only the list. Edit nothing."
    } | ask_claude "readme-audit" "$OUT/README-audit.md"

    step "3c. release-notes summary"
    {
        echo "Write the GitHub release notes for cgg $VERSION."
        echo "PURPOSE: $PURPOSE"
        echo
        echo "$RULES"
        echo
        echo "Read /storage/cgg/CHANGELOG.md for tone. Existing releases open"
        echo "with a one-paragraph thesis, then a table, then the caveats."
        echo "Keep it shorter than the CHANGELOG entry — this is the summary a"
        echo "reader sees first."
        echo
        echo "=== FACTS ==="
        cat "$FACTS"
        echo
        echo "=== DRAFT CHANGELOG ENTRY (if present) ==="
        [ -f "$OUT/CHANGELOG-draft.md" ] && head -300 "$OUT/CHANGELOG-draft.md"
        echo
        echo "Output only the markdown body. Edit nothing."
    } | ask_claude "release-notes" "$OUT/RELEASE-NOTES.md"
fi

# =====================================================================
step "4. verify"
# =====================================================================
# Anything step 3 wrote is a draft for a human to paste in. Re-run the
# cheap gates so a hand-edit between steps cannot slip through.
gate "docs-check (post)" python3 scripts/docs-check.py
have npx && gate "markdownlint (post)" npx --yes markdownlint-cli2 README.md CHANGELOG.md

# =====================================================================
step "5. report"
# =====================================================================
echo
if [ -s "$BLOCKERS" ]; then
    printf '\033[31m%s blocker(s):\033[0m\n' "$(wc -l < "$BLOCKERS")"
    sed 's/^/  - /' "$BLOCKERS"
    echo
    echo "Artifacts: $OUT"
    exit 1
fi

printf '\033[32mAll gates passed.\033[0m\n\n'
echo "Artifacts in $OUT:"
for f in facts.md CHANGELOG-draft.md README-audit.md RELEASE-NOTES.md compare.json; do
    [ -f "$OUT/$f" ] && printf '  %-22s %s\n' "$f" "$(wc -l < "$OUT/$f") lines"
done
cat <<EOF

Nothing has been committed, tagged or pushed. Review the drafts, fold them
into CHANGELOG.md and README.md yourself, then:

  git add -A && git commit -m "chore: release $VERSION"
  git tag -a "v$VERSION" -m "cgg $VERSION"
  git push origin main && git push origin "v$VERSION"
  gh release create "v$VERSION" --title "..." --notes-file $OUT/RELEASE-NOTES.md

The drafts are drafts. Read them against $OUT/facts.md before shipping —
this script can check that a number was measured, not that a sentence is
true.
EOF
