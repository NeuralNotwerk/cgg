#!/usr/bin/env bash
#
# scripts/security-check.sh — run BEFORE scripts/release.sh.
#
# release.sh answers "is this correct?". This answers "is it safe to make
# public?" — which is a different question and has to be asked first,
# because publishing is irreversible and a leaked credential is still
# leaked after you yank the release.
#
#   scripts/security-check.sh          # everything
#   scripts/security-check.sh --quick  # skip the git-history sweep
#
# Everything here is local and offline except trufflehog's own installer
# and its credential *verification* calls. Nothing is uploaded.
#
# Exit 0 = clear. Any non-zero = do not publish until it is understood.

set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

QUICK=0
[ "${1:-}" = "--quick" ] && QUICK=1

FAIL=0
pass() { printf '  \033[32mok\033[0m    %s\n' "$1"; }
fail() { printf '  \033[31mFAIL\033[0m  %s\n' "$1"; FAIL=1; }
warn() { printf '  \033[33mwarn\033[0m  %s\n' "$1"; }
head() { printf '\n\033[1m%s\033[0m\n' "$1"; }

echo "cgg security check — $(git rev-parse --short HEAD 2>/dev/null || echo 'no git')"

# ---------------------------------------------------------------------
head "1. Secrets in the tree and in history"
# ---------------------------------------------------------------------
# `Lob` is excluded, deliberately and narrowly. Its detector matches
# `test_<alnum>` — which is every pytest function name in
# crates/cgg-py/tests — and Lob's test endpoint accepts any such string,
# so trufflehog reports them as VERIFIED. Twelve false positives, zero
# signal. This project has no Lob dependency; if one is ever added, drop
# the exclusion.
TH_EXCLUDE_DETECTORS="Lob"

if command -v trufflehog >/dev/null 2>&1; then
    cat > /tmp/cgg-th-exclude.txt <<'EOF'
target/
node_modules/
.claude/worktrees/
__pycache__/
.pytest_cache/
.ruff_cache/
dist/
EOF
    # `unverified` is NOT optional. trufflehog only marks a finding
    # `verified` if it can authenticate against the live API, so a real
    # credential that has since been rotated — still leaked, still in
    # history — reports as `unverified`. Verified against three randomly
    # generated AWS / GitHub / npm credentials: `verified,unknown` finds
    # ZERO of them; adding `unverified` finds all three.
    TH_RESULTS="verified,unknown,unverified"

    out="$(trufflehog filesystem "$ROOT" \
            --exclude-paths=/tmp/cgg-th-exclude.txt \
            --exclude-detectors="$TH_EXCLUDE_DETECTORS" \
            --results="$TH_RESULTS" --no-update --json 2>/dev/null \
          | grep -c DetectorName)" || out=0
    [ "$out" = "0" ] && pass "trufflehog: working tree clean" \
                     || fail "trufflehog: $out finding(s) in the working tree — inspect before publishing"

    if [ "$QUICK" = "0" ]; then
        # History matters independently: a credential committed and later
        # deleted is still in every clone.
        outh="$(trufflehog git "file://$ROOT" \
                 --exclude-detectors="$TH_EXCLUDE_DETECTORS" \
                 --results="$TH_RESULTS" --no-update --json 2>/dev/null \
               | grep -c DetectorName)" || outh=0
        [ "$outh" = "0" ] && pass "trufflehog: git history clean" \
                          || fail "trufflehog: $outh finding(s) in git history"
    else
        warn "git-history sweep skipped (--quick)"
    fi
else
    fail "trufflehog not installed — curl -sSfL https://raw.githubusercontent.com/trufflesecurity/trufflehog/main/scripts/install.sh | sh -s -- -b ~/.local/bin"
fi

# ---------------------------------------------------------------------
head "2. This machine's real credentials are not in the repo"
# ---------------------------------------------------------------------
# Detectors work on patterns. This works on the actual bytes, which is
# strictly stronger for the specific secrets that pass through a release.
# Never prints a token.
leaked=0
for f in "$HOME/.crates.io.token" "$HOME/.pypi.token" "$HOME/.npm.token" \
         "$HOME/.npmrc" "$HOME/.cargo/credentials.toml"; do
    [ -r "$f" ] || continue
    # Longest credential-looking run in the file; short lines are config.
    tok="$(tr -d '\r' < "$f" | grep -oE '[A-Za-z0-9_./+-]{24,}' | sort -u | head -20)"
    [ -n "$tok" ] || continue
    while IFS= read -r t; do
        [ -n "$t" ] || continue
        if grep -rqF "$t" "$ROOT" --exclude-dir=.git --exclude-dir=target \
              --exclude-dir=node_modules 2>/dev/null; then
            fail "a credential from $(basename "$f") appears in the working tree"
            leaked=1
        fi
        if [ "$QUICK" = "0" ] && git grep -qF "$t" -- $(git rev-list --all 2>/dev/null | head -400) 2>/dev/null; then
            fail "a credential from $(basename "$f") appears in git history"
            leaked=1
        fi
    done <<< "$tok"
done
[ "$leaked" = "0" ] && pass "no local credential appears in the tree or history"

# ---------------------------------------------------------------------
head "3. No credential files inside the repo"
# ---------------------------------------------------------------------
found="$(find "$ROOT" \
    \( -path '*/target' -o -path '*/node_modules' -o -path '*/.git' \) -prune -o \
    -type f \( -name '.npmrc' -o -name '.pypirc' -o -name 'credentials*' \
              -o -name '*.pem' -o -name '*.p12' -o -name '*.pfx' \
              -o -name 'id_rsa*' -o -name 'id_ed25519*' -o -name '.env' \
              -o -name '*.token' \) -print 2>/dev/null)"
[ -z "$found" ] && pass "no credential-shaped files in the repo" \
                || { fail "credential-shaped files present:"; echo "$found" | sed 's/^/          /'; }

# ---------------------------------------------------------------------
head "4. .gitignore would catch them anyway"
# ---------------------------------------------------------------------
missing=""
for pat in .env .npmrc; do
    printf 'x' > "$ROOT/$pat.__sec_probe" 2>/dev/null || continue
    git check-ignore -q "$ROOT/$pat.__sec_probe" 2>/dev/null || missing="$missing $pat"
    rm -f "$ROOT/$pat.__sec_probe"
done
# `.env` is the one that matters most; `.npmrc` is npm's own auth store.
git check-ignore -q "$ROOT/.env" 2>/dev/null && pass ".env is gitignored" \
    || fail ".env is NOT gitignored — an accidental one would be committable"

# ---------------------------------------------------------------------
head "5. Dependency advisories and licences"
# ---------------------------------------------------------------------
if command -v cargo-deny >/dev/null 2>&1 || cargo deny --version >/dev/null 2>&1; then
    if cargo deny check advisories 2>&1 | grep -q '^advisories ok'; then
        pass "cargo-deny: no known advisories"
    else
        fail "cargo-deny: advisory check failed — cargo deny check advisories"
    fi
    if cargo deny check licenses bans sources 2>&1 | grep -qE 'licenses ok'; then
        pass "cargo-deny: licences, bans and sources ok"
    else
        fail "cargo-deny: licences/bans/sources failed"
    fi
else
    warn "cargo-deny not installed — cargo install cargo-deny --locked"
fi

if command -v npm >/dev/null 2>&1 && [ -f crates/cgg-node/package.json ]; then
    vulns="$(cd crates/cgg-node && npm audit --omit=dev --json 2>/dev/null \
             | python3 -c 'import json,sys
try: print(json.load(sys.stdin)["metadata"]["vulnerabilities"]["total"])
except Exception: print(0)' 2>/dev/null)" || vulns=0
    [ "${vulns:-0}" = "0" ] && pass "npm audit: no runtime vulnerabilities" \
                            || fail "npm audit: $vulns vulnerability(ies)"
fi

# ---------------------------------------------------------------------
head "6. CI cannot print a secret"
# ---------------------------------------------------------------------
# A secret referenced in a `run:` body can end up in the public log.
# Referencing it through `env:` is what keeps GitHub's masking reliable.
bad="$(grep -nE '^\s*(run:|\s+)(.*)(echo|printf|cat).*secrets\.' .github/workflows/*.yml 2>/dev/null || true)"
[ -z "$bad" ] && pass "no workflow step echoes a secret" \
              || { fail "a workflow step may print a secret:"; echo "$bad" | sed 's/^/          /'; }

# ---------------------------------------------------------------------
head "7. What the published artifacts would actually contain"
# ---------------------------------------------------------------------
# Packaging globs are how private files escape. cgg has already shipped
# cargo intermediates (libcgg.d, libcgg.rlib) into a release tarball this
# way, so the payload is worth reading rather than assuming.
if command -v cargo >/dev/null 2>&1; then
    sus="$(cargo package --list -p cgg --allow-dirty 2>/dev/null \
           | grep -iE '\.(env|pem|key|token|npmrc|p12)$|credential|secret' || true)"
    [ -z "$sus" ] && pass "cargo package payload: nothing credential-shaped" \
                  || { fail "cargo package would ship:"; echo "$sus" | sed 's/^/          /'; }
fi
if [ -f crates/cgg-node/package.json ] && command -v npm >/dev/null 2>&1; then
    sus="$(cd crates/cgg-node && npm pack --dry-run --json 2>/dev/null \
           | python3 -c 'import json,sys
try:
    d=json.load(sys.stdin)
    for f in d[0].get("files",[]): print(f["path"])
except Exception: pass' 2>/dev/null \
           | grep -iE '\.(env|pem|key|token|npmrc)$|credential|secret' || true)"
    [ -z "$sus" ] && pass "npm pack payload: nothing credential-shaped" \
                  || { fail "npm pack would ship:"; echo "$sus" | sed 's/^/          /'; }
fi

# ---------------------------------------------------------------------
head "8. Local credential files are not world-readable"
# ---------------------------------------------------------------------
loose=""
for f in "$HOME/.crates.io.token" "$HOME/.pypi.token" "$HOME/.npm.token" \
         "$HOME/.npmrc" "$HOME/.cargo/credentials.toml"; do
    [ -e "$f" ] || continue
    mode="$(stat -c '%a' "$f")"
    case "$mode" in 600|400) ;; *) loose="$loose $f($mode)" ;; esac
done
[ -z "$loose" ] && pass "credential files are owner-only" \
                || { warn "loosely-permissioned:$loose"; warn "chmod 600 them"; }

# ---------------------------------------------------------------------
echo
if [ "$FAIL" = "0" ]; then
    printf '\033[32mSECURITY CHECK PASSED\033[0m — safe to run scripts/release.sh\n'
else
    printf '\033[31mSECURITY CHECK FAILED\033[0m — do not publish until each FAIL is understood\n'
fi
exit "$FAIL"
