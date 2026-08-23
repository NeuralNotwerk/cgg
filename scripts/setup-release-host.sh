#!/usr/bin/env bash
#
# scripts/setup-release-host.sh — provision a machine to run cgg's full
# test suite and cut releases from.
#
# Usage:
#   scripts/setup-release-host.sh              # check only, change nothing
#   scripts/setup-release-host.sh --install    # install what needs no root
#   scripts/setup-release-host.sh --corpus     # also clone the benchmark corpus
#   scripts/setup-release-host.sh --install --corpus
#
# THIS SCRIPT CONTAINS NO SECRETS AND NEVER WILL.
#
# It reports which publish credentials are missing and where each one
# goes; it does not carry, fetch, print or transmit any token value. That
# is what makes it safe to keep in a public repository. Provisioning a
# host and authorising it to publish are deliberately two separate acts —
# anyone can run this, and it still cannot release anything.
#
# Why a release HOST at all, rather than "run the gates anywhere":
# `scripts/release.sh` gates on things a laptop does not have. The
# 164-repo benchmark corpus backs both the corpus A/B in
# `scripts/perf-compare.sh` and the framework-coverage table, and the
# `cgg-py` and `cgg-node` suites need interpreters that `cargo test` does
# not. Every one of those has caught a real bug that the Rust tests
# missed. A machine that cannot run them can still develop cgg; it cannot
# release it.

set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DO_INSTALL=0
DO_CORPUS=0
while [ $# -gt 0 ]; do
    case "$1" in
        --install) DO_INSTALL=1; shift ;;
        --corpus)  DO_CORPUS=1;  shift ;;
        -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done

MISSING=0
NOTES=()

have() { command -v "$1" >/dev/null 2>&1; }
ok()   { printf '  \033[32m ok \033[0m %s\n' "$*"; }
no()   { printf '  \033[31mMISS\033[0m %s\n' "$*"; MISSING=$((MISSING + 1)); }
warn() { printf '  \033[33mwarn\033[0m %s\n' "$*"; }
step() { printf '\n\033[1m== %s\033[0m\n' "$*"; }
note() { NOTES+=("$*"); }

# ---------------------------------------------------------------------
step "1. required — the gates cannot run without these"
# ---------------------------------------------------------------------
# `release.sh` fails outright on any of these. Everything below this
# section degrades to a skipped gate instead.

if have cargo; then
    ok "cargo ($(cargo --version 2>/dev/null | cut -d' ' -f2))"
elif [ -x "$HOME/.cargo/bin/cargo" ]; then
    # Installed but not on PATH — a different problem with a different
    # fix, and telling someone to install rustup when rustup is sitting
    # right there sends them down the wrong path entirely. Non-login
    # shells and `ssh host <cmd>` both skip the profile that adds it.
    no "cargo is installed at ~/.cargo/bin but is not on PATH"
    note 'source "$HOME/.cargo/env"   (or add ~/.cargo/bin to PATH in your shell profile)'
    note 'non-login shells and `ssh host <cmd>` skip the profile that sets it'
else
    no "cargo — install from https://rustup.rs"
    note "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
fi

have git     && ok "git"     || no "git"
have python3 && ok "python3 ($(python3 -V 2>&1 | cut -d' ' -f2))" || no "python3"

# tree-sitter grammars and mimalloc are C. Without a compiler the build
# fails deep in a build script rather than here, which is a worse place
# to find out.
if have cc || have gcc || have clang; then
    ok "C compiler"
else
    no "C compiler — tree-sitter grammars and mimalloc need one"
    note "Debian/Ubuntu: sudo apt-get install -y build-essential"
    note "Fedora/RHEL:   sudo dnf groupinstall -y 'Development Tools'"
    note "macOS:         xcode-select --install"
fi

# clippy and rustfmt are rustup COMPONENTS, not separate packages. A
# toolchain can have cargo and still fail `cargo fmt --all --check` with
# "not installed for the toolchain", which reads like a formatting
# failure and is not one.
# `component:subcommand`, because the two differ: the component is
# `rustfmt` but the cargo subcommand is `fmt`, and probing
# `cargo rustfmt --version` reports "no such command" on a toolchain that
# has rustfmt perfectly well installed. Checking the wrong name told this
# host to install something it already had.
for pair in clippy:clippy rustfmt:fmt; do
    comp="${pair%%:*}"; sub="${pair##*:}"
    if ! have cargo; then
        warn "rustup component: $comp — cannot check without cargo on PATH"
        continue
    fi
    if cargo "$sub" --version >/dev/null 2>&1; then
        ok "rustup component: $comp"
    elif [ "$DO_INSTALL" = "1" ] && have rustup; then
        echo "     installing $comp …"
        rustup component add "$comp" >/dev/null 2>&1 \
            && ok "rustup component: $comp (installed)" \
            || no "rustup component: $comp — rustup component add $comp"
    else
        no "rustup component: $comp — rustup component add $comp"
    fi
done

# ---------------------------------------------------------------------
step "2. front-end test suites — cargo test does not cover these"
# ---------------------------------------------------------------------
# `crates/cgg-py` sets `test = false` and `crates/cgg-node`'s tests are
# JavaScript, so `cargo test --workspace` passes with both front ends
# broken. Both have shipped bugs that only these suites caught.

if have uv; then
    ok "uv — scripts/build-python.sh (cgg-py pytest suite)"
elif [ "$DO_INSTALL" = "1" ]; then
    echo "     installing uv …"
    curl -LsSf https://astral.sh/uv/install.sh 2>/dev/null | sh >/dev/null 2>&1
    have uv && ok "uv (installed)" || no "uv — https://astral.sh/uv"
else
    no "uv — needed by scripts/build-python.sh"
    note "curl -LsSf https://astral.sh/uv/install.sh | sh"
fi

if have node && have npx; then
    ok "node ($(node -v 2>/dev/null)) + npx — cgg-node suite, markdownlint"
else
    no "node + npx — cgg-node 'node --test' suite and the markdownlint gate"
    note "https://nodejs.org or your package manager; node 20+"
fi

# ---------------------------------------------------------------------
step "3. optional gates — release.sh skips these when absent"
# ---------------------------------------------------------------------
# Skipped, not failed. That is a trap worth naming: a release cut on a
# host without these is GREENER than one cut with them, because the gate
# silently does not run.

have cargo-deny && ok "cargo-deny (licence + advisory audit)" || {
    if [ "$DO_INSTALL" = "1" ] && have cargo; then
        echo "     installing cargo-deny (this compiles; a few minutes) …"
        cargo install cargo-deny --locked >/dev/null 2>&1 \
            && ok "cargo-deny (installed)" \
            || warn "cargo-deny install failed — cargo install cargo-deny --locked"
    else
        warn "cargo-deny absent — the audit gate will SKIP, not fail"
        note "cargo install cargo-deny --locked"
    fi
}

# `elsewhere` finds a tool that is installed but invisible to this shell.
# It matters more for the optional gates than the required ones: a
# missing required tool fails loudly, while a missing optional one is
# SKIPPED, so the release comes back greener for having tested less. That
# is not hypothetical — ruff lives in ~/.venv/bin on the host this script
# was written for, `release.sh` skipped both python lint gates, and a
# stale log from an earlier run made it look like they had passed.
elsewhere() {
    local t="$1" p
    for p in "$HOME/.venv/bin/$t" "$HOME/.local/bin/$t" "$HOME/.cargo/bin/$t" \
             "$HOME/.nix-profile/bin/$t" "/usr/local/bin/$t"; do
        [ -x "$p" ] && { echo "$p"; return 0; }
    done
    return 1
}

if have ruff; then
    ok "ruff (python lint/format for scripts/)"
elif found="$(elsewhere ruff)"; then
    warn "ruff is at $found but NOT on PATH — the lint gates will SKIP"
    note "add $(dirname "$found") to PATH, or the ruff gates silently do not run"
elif [ "$DO_INSTALL" = "1" ] && have uv; then
    uv tool install ruff >/dev/null 2>&1 \
        && ok "ruff (installed)" || warn "ruff install failed"
else
    warn "ruff absent — the python lint gates will SKIP, not fail"
    note "uv tool install ruff   (or: pipx install ruff)"
fi

have gh && ok "gh (GitHub CLI — release inspection, secret management)" \
        || warn "gh absent — handy for checking runs and secrets, not required"

if have shellcheck; then
    ok "shellcheck"
elif found="$(elsewhere shellcheck)"; then
    warn "shellcheck is at $found but NOT on PATH — that gate will SKIP"
else
    warn "shellcheck absent — shell lint gate will SKIP"
fi

# ---------------------------------------------------------------------
step "4. benchmark corpus"
# ---------------------------------------------------------------------
# The corpus backs the two gates that a laptop silently cannot run:
# `perf-compare.sh` (the paired A/B the CHANGELOG's Performance block
# needs) and the framework-coverage table.
CORPUS="${CGG_BENCH_DIR:-/storage/cgg-test_repos}"
if [ -d "$CORPUS" ] && [ "$(find "$CORPUS" -maxdepth 1 -mindepth 1 -type d 2>/dev/null | wc -l)" -gt 100 ]; then
    ok "corpus at $CORPUS ($(find "$CORPUS" -maxdepth 1 -mindepth 1 -type d | wc -l) repos)"
elif [ "$DO_CORPUS" = "1" ]; then
    echo "     cloning the corpus into $CORPUS — this is many GB and takes a while"
    if ! mkdir -p "$CORPUS" 2>/dev/null; then
        no "cannot create $CORPUS — set CGG_BENCH_DIR to a writable path"
    else
        ./scripts/benchmark.sh --apps >/dev/null 2>&1 || true
        ./scripts/benchmark.sh >/dev/null 2>&1 || true
        ok "corpus populated (re-run this script to confirm the count)"
    fi
else
    warn "no corpus at $CORPUS — perf-compare and framework-coverage cannot run"
    note "re-run with --corpus, or set CGG_BENCH_DIR to an existing copy"
    note "corpus is cloned by scripts/benchmark.sh; it is many GB"
fi

# ---------------------------------------------------------------------
step "5. publish credentials — presence only, never values"
# ---------------------------------------------------------------------
# Checked, never read. This script reports whether a credential exists so
# a release does not fail halfway; it does not print, copy or transmit
# any of them. Authorising a host to publish stays a separate, manual,
# deliberate act.
#
# Local publishing (scripts/publish-*.sh) reads the first column. CI
# publishing reads GitHub Actions secrets, which are set once on the
# repository and are NOT files on this machine.

cred() { # name, path, how-to
    if [ -s "$2" ]; then
        ok "$1 — present at $2"
    else
        warn "$1 — absent ($2)"
        note "$3"
    fi
}

cred "crates.io token" "$HOME/.cargo/credentials.toml" \
     "cargo login   (token: https://crates.io/settings/tokens, scope publish-update)"
cred "npm token"       "$HOME/.npmrc" \
     "npm login     (or an automation token in ~/.npmrc)"
# `scripts/publish-python.sh` reads $PYPI_TOKEN, then $PYPI_TOKEN_FILE,
# then ~/.pypi.token — NOT ~/.pypirc, which is twine's own convention and
# is not what this repo uses. Checking the wrong path reported a token
# that was present as missing.
if [ -n "${PYPI_TOKEN:-}" ]; then
    ok "PyPI token — \$PYPI_TOKEN is set"
elif [ -s "${PYPI_TOKEN_FILE:-$HOME/.pypi.token}" ]; then
    ok "PyPI token — present at ${PYPI_TOKEN_FILE:-$HOME/.pypi.token}"
elif [ -s "$HOME/.pypirc" ]; then
    warn "found ~/.pypirc, but publish-python.sh reads ~/.pypi.token"
    note "put the token in ~/.pypi.token, or set \$PYPI_TOKEN_FILE"
else
    warn "PyPI token — absent (\$PYPI_TOKEN, \$PYPI_TOKEN_FILE or ~/.pypi.token)"
    note "https://pypi.org/manage/account/token/ then write ~/.pypi.token"
fi

if have gh && gh auth status >/dev/null 2>&1; then
    ok "gh authenticated"
else
    warn "gh not authenticated — gh auth login"
fi

echo
echo "  CI publishing uses repository secrets, not these files:"
echo "    CARGO_REGISTRY_TOKEN, PYPI_API_TOKEN, NPM_TOKEN"
echo "    Settings > Secrets and variables > Actions"
echo "  The 'preflight' job fails a release before anything publishes if"
echo "  any of the three is missing."

# ---------------------------------------------------------------------
step "summary"
# ---------------------------------------------------------------------
if [ "${#NOTES[@]}" -gt 0 ]; then
    echo "to fix:"
    printf '  %s\n' "${NOTES[@]}"
    echo
fi

if [ "$MISSING" -gt 0 ]; then
    echo "$MISSING required item(s) missing — the gates will not all run."
    [ "$DO_INSTALL" = "0" ] && echo "Re-run with --install to install what needs no root."
    exit 1
fi

echo "All required tooling present."
echo "Verify with:  ./scripts/release.sh --purpose 'dry run' --skip-perf --skip-ai"
