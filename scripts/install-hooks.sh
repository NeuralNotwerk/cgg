#!/usr/bin/env bash
#
# Wire this checkout's git config to use the repo-tracked hooks in
# .githooks/. Run once per clone.
#
#   $ scripts/install-hooks.sh
#
# Idempotent. Bypass any hook for a single commit with --no-verify.

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

git config core.hooksPath .githooks
chmod +x .githooks/pre-commit scripts/update-readme-graphs.py scripts/install-hooks.sh 2>/dev/null || true

echo "[cgg] git hooks path set to .githooks/"
echo "[cgg] pre-commit will run tests, release-build, regenerate README graphs."
echo "[cgg] bypass a single commit with: git commit --no-verify"
echo "[cgg] disable entirely for a session: CGG_SKIP_PRECOMMIT=1 git commit ..."
