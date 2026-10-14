#!/usr/bin/env bash
#
# Build and test the cgg Python extension module.
#
#   scripts/build-python.sh              build into a venv and run the tests
#   scripts/build-python.sh --wheel      build a release wheel into dist/
#
# `cargo build` compiles the cdylib; only maturin makes it importable.
#
# Requires a Rust toolchain (>= 1.85) and `uv`. uv rather than the system
# python because abi3-py39 rules out anything older than 3.9, and it
# fetches a self-contained CPython — which also sidesteps distro builds
# whose libpython is missing from the loader path.

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

WHEEL=0
for arg in "$@"; do
    case "$arg" in
        --wheel)   WHEEL=1 ;;
        -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
        *) echo "unknown option: $arg" >&2; exit 2 ;;
    esac
done

for cmd in cargo uv git; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
        echo "error: $cmd not found on PATH" >&2
        exit 1
    fi
done

VENV="${CGG_PY_VENV:-target/python-venv}"
PY_VERSION="${CGG_PY_VERSION:-3.12}"

if [[ ! -x "$VENV/bin/python" ]]; then
    # Pin to uv's own managed build. `uv venv --python 3.12` may otherwise
    # select a system interpreter whose shared library is not loadable —
    # which is the case on the machine this was written for.
    MANAGED="$(uv python find "cpython-$PY_VERSION" --managed-python 2>/dev/null || true)"
    if [[ -z "$MANAGED" ]]; then
        echo "[build-python] provisioning CPython $PY_VERSION"
        # `--no-bin`: the shim in ~/.local/bin is not wanted and warns
        # loudly when something else already owns that name.
        uv python install --no-bin "$PY_VERSION" >/dev/null
        MANAGED="$(uv python find "cpython-$PY_VERSION" --managed-python 2>/dev/null || true)"
    fi
    if [[ -z "$MANAGED" ]]; then
        echo "error: could not locate a managed CPython $PY_VERSION" >&2
        exit 1
    fi
    echo "[build-python] creating venv at $VENV from $MANAGED"
    uv venv --python "$MANAGED" "$VENV"
fi

# maturin locates the target interpreter through VIRTUAL_ENV, not through
# PATH, so exporting it is what makes `maturin develop` install into our
# venv rather than refusing to run.
VIRTUAL_ENV="$PWD/$VENV"
export VIRTUAL_ENV
PATH="$VIRTUAL_ENV/bin:$PATH"
export PATH

echo "[build-python] installing maturin + pytest"
uv pip install --quiet --python "$VENV/bin/python" maturin pytest

# The parity test compares the module's output against the binary's, so the
# binary has to exist and has to come from this same tree.
echo "[build-python] cargo build --release -p cgg  (parity test needs it)"
cargo build --release -p cgg

if [[ "$WHEEL" == "1" ]]; then
    echo "[build-python] maturin build --release"
    ( cd crates/cgg-py && "../../$VENV/bin/maturin" build --release --out ../../dist )
    echo "[build-python] wheel(s) in dist/:"
    ls -1 dist/*.whl
    # Install it so the tests below exercise the wheel, not the source tree.
    uv pip install --quiet --python "$VENV/bin/python" --force-reinstall dist/*.whl
else
    echo "[build-python] maturin develop --release"
    ( cd crates/cgg-py && "../../$VENV/bin/maturin" develop --release )
fi

echo "[build-python] pytest"
CGG_BIN="$PWD/target/release/cgg" "$VENV/bin/python" -m pytest crates/cgg-py/tests -q

echo "[build-python] ok"
