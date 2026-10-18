#!/usr/bin/env bash
# Copy target/release/cgg to bins/<NAME> and record its identity in
# bins/MANIFEST.tsv: sha256, `--version` (commit + diff fingerprint for
# interim builds), the source commit, and when. Refuses to overwrite.
# Usage: tools/stash_bin.sh <NAME>    e.g. WS27-after-v1
set -euo pipefail
cd "$(dirname "$0")/../../.."
name="${1:?usage: stash_bin.sh NAME}"
dst="target/remediation/bins/$name"
[[ -e "$dst" ]] && { echo "exists: $dst" >&2; exit 1; }
cp target/release/cgg "$dst"
sha="$(sha256sum "$dst" | cut -d' ' -f1)"
ver="$("$dst" --version)"
printf '%s\t%s\t%s\t%s\t%s\n' "$name" "$sha" "$ver" "$(git rev-parse --short=7 HEAD)" "$(date -Is)" \
    >> target/remediation/bins/MANIFEST.tsv
echo "$name  $ver  sha256=$sha"
