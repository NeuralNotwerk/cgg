#!/usr/bin/env bash
#
# scripts/upgrade-local.sh — bring every local cgg instance up to this tree.
#
# When to run: after every release, once the tag is pushed (release.sh
# prints it as the last step). A release that reaches the registries but
# not this machine leaves agents, sibling repos and staged copies running
# the old resolver — 0.10.2 found ~/.local/bin current (the pre-commit
# hook installs it) but a staged copy in another project and the
# in-repo Python/Node builds still on 0.10.1.
#
# What it upgrades:
#   1. $CGG_INSTALL_DIR/cgg (default ~/.local/bin/cgg) and every other
#      `cgg` on PATH that is a regular file you own.
#   2. Every path listed in $CGG_LOCAL_INSTALLS (default
#      ~/.config/cgg/local-installs): one binary path per line, `#`
#      comments allowed. This is where copies staged by other projects
#      live (they copy whatever `cgg` is on PATH at their build time).
#      The list is per machine and deliberately not in the repo.
#   3. The in-repo Python venv (scripts/build-python.sh, runs pytest) and
#      the Node module (napi build, runs node --test).
#
# Binaries are replaced by install + rename, so a process running the old
# one keeps a valid file. Then every instance is asked for its version
# and the script fails if any disagrees with Cargo.toml.
#
# Usage:
#   scripts/upgrade-local.sh            # all of the above
#   scripts/upgrade-local.sh --bin-only # skip the Python/Node rebuilds
#
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT" || exit 1

BIN_ONLY=0
[ "${1:-}" = "--bin-only" ] && BIN_ONLY=1

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
echo "[upgrade-local] target version $VERSION"

cargo build --release -p cgg --quiet || { echo "build failed" >&2; exit 1; }
NEW="$ROOT/target/release/cgg"
got="$("$NEW" --version | awk '{print $2}')"
[ "$got" = "$VERSION" ] || { echo "built binary says $got, Cargo.toml says $VERSION" >&2; exit 1; }

targets=("${CGG_INSTALL_DIR:-$HOME/.local/bin}/cgg")
while IFS= read -r p; do targets+=("$p"); done < <(which -a cgg 2>/dev/null)
LIST="${CGG_LOCAL_INSTALLS:-$HOME/.config/cgg/local-installs}"
if [ -f "$LIST" ]; then
    while IFS= read -r p; do
        p="${p%%#*}"; p="$(echo "$p" | xargs)"
        [ -n "$p" ] && targets+=("${p/#\~/$HOME}")
    done < "$LIST"
else
    echo "[upgrade-local] no $LIST — only PATH copies will be upgraded"
fi

fail=0
declare -A seen
for t in "${targets[@]}"; do
    real="$(realpath -m "$t")"
    [ -n "${seen[$real]:-}" ] && continue
    seen[$real]=1
    [ "$real" = "$(realpath "$NEW")" ] && continue
    if [ -e "$real" ] && [ ! -O "$real" ]; then
        echo "  SKIP $t (not owned by you)"; continue
    fi
    if [ ! -d "$(dirname "$real")" ]; then
        echo "  MISSING $t (directory gone — remove it from $LIST?)"; fail=1; continue
    fi
    install -m 755 "$NEW" "$real.new.$$" && mv -f "$real.new.$$" "$real"
    v="$("$real" --version 2>&1 | awk '{print $2}')"
    if [ "$v" = "$VERSION" ]; then echo "  ok   $t -> $v"; else echo "  FAIL $t -> $v"; fail=1; fi
done

if [ "$BIN_ONLY" = 0 ]; then
    ./scripts/build-python.sh >/dev/null 2>&1 || { echo "  FAIL python build/tests"; fail=1; }
    v="$(target/python-venv/bin/python -c 'import cgg; print(cgg.__version__)' 2>&1)"
    if [ "$v" = "$VERSION" ]; then echo "  ok   python venv -> $v"; else echo "  FAIL python venv -> $v"; fail=1; fi
    if command -v npx >/dev/null; then
        ( cd crates/cgg-node \
          && npx --yes --package=@napi-rs/cli@3.8.5 -- napi build --platform --release >/dev/null 2>&1 \
          && node --test tests/*.test.js >/dev/null 2>&1 ) || { echo "  FAIL node build/tests"; fail=1; }
        v="$(cd crates/cgg-node && node -e "console.log(require('./index.js').version())" 2>&1)"
        if [ "$v" = "$VERSION" ]; then echo "  ok   node module -> $v"; else echo "  FAIL node module -> $v"; fail=1; fi
    else
        echo "  SKIP node module (no npx)"
    fi
fi

if [ "$fail" = 0 ]; then
    echo "[upgrade-local] every instance is at $VERSION"
    echo "Projects that stage a copy may also cache output from the old one"
    echo "(e.g. a generated graph); rebuild those with the project's own script."
else
    echo "[upgrade-local] some instances are NOT at $VERSION" >&2
    exit 1
fi
