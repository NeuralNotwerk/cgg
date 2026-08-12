#!/usr/bin/env bash
#
# scripts/publish-python.sh — build a portable wheel and upload it to PyPI.
#
#   scripts/publish-python.sh --check   # build, test, twine check. No upload.
#   scripts/publish-python.sh           # the above, then upload.
#
# Why a container: PyPI REJECTS plain `linux_x86_64` wheels — only
# `manylinux`-tagged ones are accepted — and a wheel built on a modern
# machine links a glibc newer than most users have. maturin's image is
# CentOS 7 (glibc 2.17), so the wheel it produces runs essentially
# everywhere. `abi3-py39` means that one wheel also covers every CPython
# from 3.9 up, so the matrix is platforms, not platforms x versions.
#
# The distribution is `cgg-callgraphgenerator`; the import is `cgg`.
# PyPI's `cgg` is an unrelated GGUF tool.
#
# Token: ~/.pypi.token (or $PYPI_TOKEN). Get one at
# https://pypi.org/manage/account/token/ — the FIRST upload of a new
# project needs an account-scoped token, because a project-scoped one
# cannot be created until the project exists.
#
# UPLOADS ARE FOREVER. A file can be deleted from PyPI but its
# (name, version) can never be reused. Run --check first.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

CHECK_ONLY=0
[ "${1:-}" = "--check" ] && CHECK_ONLY=1

IMAGE="ghcr.io/pyo3/maturin"
VERSION="$(grep -m1 '^version = ' Cargo.toml | cut -d'"' -f2)"
DIST="$ROOT/dist"

command -v docker >/dev/null || { echo "error: docker not found" >&2; exit 1; }

echo "cgg-callgraphgenerator $VERSION -> PyPI"
echo

# --- build ------------------------------------------------------------
rm -rf "$DIST"
mkdir -p "$DIST" target/manylinux target/manylinux-cargo

echo "[1/4] building manylinux wheel"
# Runs as root on purpose. `--user` fails with "Permission denied"
# starting `cargo metadata`, because the toolchain in the image is
# root-owned; ownership is repaired below instead.
docker run --rm -v "$ROOT:/io" -w /io \
    -e CARGO_HOME=/io/target/manylinux-cargo \
    -e CARGO_TARGET_DIR=/io/target/manylinux \
    "$IMAGE" build --release -m crates/cgg-py/Cargo.toml --out /io/dist

# The container wrote as root; give them back.
docker run --rm -v "$ROOT:/io" --entrypoint chown "$IMAGE" \
    -R "$(id -u):$(id -g)" /io/dist /io/target/manylinux /io/target/manylinux-cargo

# An sdist as well as the wheel. Without one, every platform with no
# matching wheel gets "No matching distribution found" instead of falling
# back to a source build — pip has that fallback, but only if an sdist
# exists. 0.6.2 and 0.6.3 shipped wheel-only and were uninstallable on
# macOS, Windows and Linux ARM because this step was missing.
echo "[1a/4] building sdist"
docker run --rm -v "$ROOT:/io" -w /io \
    -e CARGO_HOME=/io/target/manylinux-cargo \
    -e CARGO_TARGET_DIR=/io/target/manylinux \
    "$IMAGE" sdist -m crates/cgg-py/Cargo.toml --out /io/dist
docker run --rm -v "$ROOT:/io" --entrypoint chown "$IMAGE" \
    -R "$(id -u):$(id -g)" /io/dist

WHEEL="$(ls "$DIST"/*.whl)"
SDIST="$(ls "$DIST"/*.tar.gz)"
echo "  $(basename "$WHEEL") ($(du -h "$WHEEL" | cut -f1))"
echo "  $(basename "$SDIST") ($(du -h "$SDIST" | cut -f1))"

case "$(basename "$WHEEL")" in
    *manylinux*) ;;
    *) echo "error: wheel is not manylinux-tagged; PyPI will reject it" >&2; exit 1 ;;
esac

# The wheel's embedded description must match the README on disk.
#
# 0.6.1 shipped with a stale one: the README was edited while the build
# was already running, maturin had read the old text, and the wheel went
# out telling everyone to `pip install cgg` — the wrong project. PyPI
# metadata cannot be edited after upload, so fixing it cost a version.
# This is cheap and catches both that race and a plain forgotten rebuild.
echo "[1b/4] checking the wheel's description matches README.md"
python3 - "$WHEEL" "$ROOT/crates/cgg-py/README.md" <<'PY'
import sys, zipfile
whl, readme = sys.argv[1], sys.argv[2]
z = zipfile.ZipFile(whl)
name = next(n for n in z.namelist() if n.endswith(".dist-info/METADATA"))
# Decoded as UTF-8 explicitly and split by hand rather than through
# `email`, whose default payload decoding mangles every non-ASCII byte —
# this README is full of em-dashes, so that route reports a difference on
# every line that has one.
raw = z.read(name).decode("utf-8")
embedded = raw.split("\n\n", 1)[1].strip() if "\n\n" in raw else ""
on_disk = open(readme, encoding="utf-8").read().strip()
if embedded != on_disk:
    print("error: the wheel's description does not match crates/cgg-py/README.md.", file=sys.stderr)
    print("       The README changed after maturin read it — rebuild the wheel.", file=sys.stderr)
    import difflib
    diff = list(difflib.unified_diff(embedded.splitlines(), on_disk.splitlines(),
                                     "wheel", "README.md", lineterm="", n=1))
    print("\n".join("       " + l for l in diff[:20]), file=sys.stderr)
    sys.exit(1)
print("  description matches")
PY

# --- verify -----------------------------------------------------------
# Against the INSTALLED wheel, not the source tree: the point is that what
# ships works, and the parity test compares it to the binary from this
# same commit.
echo "[2/4] installing into a clean venv and running the test suite"
VENV="$(mktemp -d)"
trap 'rm -rf "$VENV"' EXIT
python3 -m venv "$VENV"
"$VENV/bin/pip" install --quiet "$WHEEL" pytest twine
cargo build --release -p cgg --quiet
CGG_BIN="$ROOT/target/release/cgg" "$VENV/bin/python" -m pytest crates/cgg-py/tests -q

echo "[3/4] twine check"
"$VENV/bin/twine" check "$WHEEL" "$SDIST"

if [ "$CHECK_ONLY" = "1" ]; then
    echo
    echo "check OK — nothing uploaded. Wheel is in dist/."
    exit 0
fi

# --- upload -----------------------------------------------------------
TOKEN_FILE="${PYPI_TOKEN_FILE:-$HOME/.pypi.token}"
if [ -n "${PYPI_TOKEN:-}" ]; then
    TOKEN="$PYPI_TOKEN"
elif [ -r "$TOKEN_FILE" ]; then
    TOKEN="$(tr -d '\r\n' < "$TOKEN_FILE")"
else
    echo "error: no token. Set \$PYPI_TOKEN or put one in $TOKEN_FILE" >&2
    exit 1
fi

echo
echo "This uploads $VERSION permanently. A version can never be reused."
printf 'Type the version to continue: '
read -r CONFIRM
[ "$CONFIRM" = "$VERSION" ] || { echo "aborted"; exit 1; }

echo "[4/4] uploading"
TWINE_USERNAME=__token__ TWINE_PASSWORD="$TOKEN" \
    "$VENV/bin/twine" upload --non-interactive "$WHEEL" "$SDIST"

echo
echo "published. Verify with:"
echo "  pip install cgg-callgraphgenerator==$VERSION"
