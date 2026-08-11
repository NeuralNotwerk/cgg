#!/usr/bin/env bash
#
# scripts/publish-crates.sh — publish the Rust crates to crates.io, in
# dependency order, waiting for each to appear in the index before the
# next one is attempted.
#
# Why a script: a workspace cannot be published in one command. Each
# crate's dependencies must already EXIST on crates.io before it can even
# be packaged — `cargo package -p cgg` fails with "no matching package
# named `cgg-core` found" until cgg-core is live. And the index is a CDN,
# so "published" and "visible to the next publish" are seconds apart.
# Doing this by hand means six commands, a fixed order, and a wait in
# between; getting the order wrong wastes a version number, because a
# version once published can never be reused.
#
# Usage:
#   scripts/publish-crates.sh --dry-run   # package + verify, upload nothing
#   scripts/publish-crates.sh             # for real, with a confirmation
#
# Prerequisites:
#   1. A crates.io account with a VERIFIED EMAIL. Without it every upload
#      is rejected with 400 "A verified email address is required".
#      https://crates.io/settings/profile
#   2. A token: https://crates.io/settings/tokens (scope: publish-new +
#      publish-update). Then `cargo login < /path/to/token`.
#   3. A clean tree at the commit you intend to publish.
#
# PUBLISHING IS FOREVER. A version can be yanked but never deleted, and
# the name is claimed permanently. Run --dry-run first.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# Dependency order. cgg-core first because everything needs it; cgg last
# because it needs everything. cgg-py and cgg-ffi are absent on purpose —
# both are `publish = false`, since the artifact anyone wants from them is
# a wheel and a shared library, not a crate.
CRATES=(cgg-core cgg-walk cgg-format cgg-lang cgg-resolve cgg)

DRY=0
[ "${1:-}" = "--dry-run" ] && DRY=1

VERSION="$(grep -m1 '^version = ' Cargo.toml | cut -d'"' -f2)"
echo "cgg $VERSION -> crates.io"
echo "order: ${CRATES[*]}"
echo

# Internal pins must match, or a published crate would depend on a version
# that does not exist. Cheap to check, expensive to get wrong.
BAD="$(grep -E '^cgg(-[a-z]+)? *= *\{ *path' Cargo.toml | grep -v "version = \"$VERSION\"" || true)"
if [ -n "$BAD" ]; then
    echo "error: internal crate pins disagree with $VERSION:" >&2
    echo "$BAD" >&2
    exit 1
fi

if [ -n "$(git status --porcelain)" ]; then
    echo "error: working tree is dirty. Publish from the commit you tagged." >&2
    exit 1
fi

if [ "$DRY" = "0" ]; then
    echo "This uploads $VERSION permanently. Versions can be yanked, never deleted."
    printf 'Type the version to continue: '
    read -r CONFIRM
    [ "$CONFIRM" = "$VERSION" ] || { echo "aborted"; exit 1; }
    echo
fi

# Wait for a crate to be resolvable before publishing whatever depends on
# it. The sparse index is a CDN and lags the upload by a few seconds.
wait_for_index() {
    local name="$1" want="$2" path
    if [ "${#name}" -le 3 ]; then
        path="${#name}/${name:0:1}/$name"
    else
        path="${name:0:2}/${name:2:2}/$name"
    fi
    printf '  waiting for %s %s in the index' "$name" "$want"
    for _ in $(seq 1 60); do
        if curl -sf "https://index.crates.io/$path" 2>/dev/null | grep -q "\"vers\":\"$want\""; then
            echo " — visible"
            return 0
        fi
        printf '.'
        sleep 5
    done
    echo
    echo "error: $name $want never appeared. Publish the rest by hand." >&2
    exit 1
}

for c in "${CRATES[@]}"; do
    echo "=== $c ==="
    if [ "$DRY" = "1" ]; then
        # `--no-verify` because a dependent cannot be verified until its
        # dependencies are actually on crates.io — that check is exactly
        # what a first-time dry run cannot satisfy.
        cargo package -p "$c" --no-verify --quiet \
            && echo "  packages OK" \
            || { echo "  FAILED to package" >&2; exit 1; }
        continue
    fi
    cargo publish -p "$c"
    [ "$c" = "${CRATES[-1]}" ] || wait_for_index "$c" "$VERSION"
    echo
done

if [ "$DRY" = "1" ]; then
    echo "dry run OK — nothing was uploaded"
else
    echo "published $VERSION. Verify: cargo install cgg --version $VERSION"
fi
