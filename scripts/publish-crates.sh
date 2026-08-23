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
ASSUME_YES=0
# How many crates were already up there. A resumed run can legitimately
# publish nothing, and saying "published" when nothing was uploaded is
# the same class of lie the npm check in release.yml exists to prevent.
SKIPPED=0
while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run) DRY=1; shift ;;
        # For CI only. A human gets the typed-version confirmation,
        # because a human can still stop. The workflow cannot type, and
        # its safety comes from somewhere better: it runs only after
        # every other registry has already published successfully.
        --yes|-y)  ASSUME_YES=1; shift ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done

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

if [ "$DRY" = "0" ] && [ "$ASSUME_YES" = "0" ]; then
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

# crates.io rate-limits the creation of NEW crates — a burst allowance,
# then roughly one every ten minutes. Publishing a six-crate workspace for
# the first time hits it, and it hit us on the sixth and most important
# one. The error names the exact time to retry, so parse it and wait
# rather than making the operator rerun the script and re-publish nothing.
#
# Only new-crate creation is limited; publishing a new VERSION of an
# existing crate is not, so this matters on a first release and
# essentially never again.
publish_with_retry() {
    local c="$1" out rc until_str wait_s
    for attempt in 1 2 3; do
        # `|| rc=$?`, not `; rc=$?`. Under `set -e` a bare assignment
        # whose command substitution fails aborts the script *at that
        # line* — `rc` is never read, and neither is anything below.
        # Every failure path in this function was therefore dead code:
        # the 429 rate-limit retry it was written for could not fire, and
        # neither could the already-published skip. The script exited 101
        # on the first non-zero cargo publish instead.
        rc=0
        out="$(cargo publish -p "$c" 2>&1)" || rc=$?
        printf '%s\n' "$out"
        [ "$rc" -eq 0 ] && return 0

        # Already up there is success, not failure. A crates.io version
        # can never be re-uploaded — only yanked — so a run that dies
        # after three of six crates must be *finishable*: re-running has
        # to skip what landed and continue with what did not. Treating
        # this as an error strands the workspace half-published with no
        # way forward, because the version number is spent.
        # Verified against cargo 1.91: "error: crate cgg-core@0.8.1
        # already exists on crates.io index". `uploaded` is kept because
        # older cargo phrased it that way.
        if grep -qE 'already (uploaded|exists)' <<<"$out"; then
            echo "  already on crates.io at $VERSION — skipping"
            SKIPPED=$((SKIPPED + 1))
            return 0
        fi

        grep -q '429 Too Many Requests' <<<"$out" || return "$rc"

        until_str="$(grep -oE 'try again after [^ ]+, [0-9]{2} [A-Za-z]{3} [0-9]{4} [0-9:]{8} GMT' <<<"$out" \
                     | sed 's/try again after //')"
        if [ -z "$until_str" ]; then
            echo "  rate limited, but could not parse the retry time" >&2
            return "$rc"
        fi
        wait_s=$(( $(date -u -d "$until_str" +%s) - $(date -u +%s) + 15 ))
        [ "$wait_s" -lt 0 ] && wait_s=15
        echo "  rate limited (new-crate quota). Waiting ${wait_s}s until $until_str …"
        sleep "$wait_s"
    done
    echo "error: $c still rate limited after 3 attempts" >&2
    return 1
}

for c in "${CRATES[@]}"; do
    echo "=== $c ==="
    if [ "$DRY" = "1" ]; then
        # A dependent cannot be packaged until its siblings are actually
        # ON crates.io — `--no-verify` skips the build check but not
        # dependency resolution, so there is no flag that makes this work
        # offline. On a first release only the root crate is checkable;
        # reporting the rest as failures would be wrong, so they are
        # reported as blocked on a sibling and the run continues.
        if cargo package -p "$c" --no-verify --quiet 2>/tmp/cgg-pkg.err; then
            echo "  packages OK"
        elif grep -q 'no matching package named `cgg' /tmp/cgg-pkg.err; then
            missing="$(grep -o 'no matching package named `[^`]*`' /tmp/cgg-pkg.err \
                       | head -1 | sed 's/.*`\(.*\)`/\1/')"
            echo "  not yet checkable — needs $missing on crates.io first (expected)"
        else
            echo "  FAILED to package" >&2
            sed 's/^/      /' /tmp/cgg-pkg.err >&2
            exit 1
        fi
        continue
    fi
    publish_with_retry "$c"
    # Every crate, including the last. For the others this is an ordering
    # requirement — the next one cannot resolve until this is on the
    # index. For the last it is verification: a publish path that can
    # exit 0 without publishing has to be checked, not trusted, which is
    # the same lesson the npm step in release.yml carries a comment
    # about.
    wait_for_index "$c" "$VERSION"
    echo
done

if [ "$DRY" = "1" ]; then
    echo "dry run OK — nothing was uploaded"
elif [ "$SKIPPED" -eq "${#CRATES[@]}" ]; then
    echo "nothing to do — all ${#CRATES[@]} crates were already on crates.io at $VERSION"
elif [ "$SKIPPED" -gt 0 ]; then
    echo "published $VERSION ($((${#CRATES[@]} - SKIPPED)) uploaded, $SKIPPED already present)."
    echo "Verify: cargo install cgg --version $VERSION"
else
    echo "published $VERSION. Verify: cargo install cgg --version $VERSION"
fi
