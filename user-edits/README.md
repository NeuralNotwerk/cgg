User's own uncommitted edits (2026-10-12), backed up off-box because the NVMe is failing.
Apply on main: `git apply CLAUDE.md-release.sh.patch && cp upgrade-local.sh scripts/`.
Pending: commit to main WITH the pre-commit hook between WS1 and WS2 (worktree target/main-wt has them applied).
