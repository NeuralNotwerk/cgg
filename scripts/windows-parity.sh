#!/usr/bin/env bash
# shellcheck disable=SC2016  # single-quoted strings are PowerShell, expanded in the VM
#
# scripts/windows-parity.sh — run one cgg commit on Linux and on the
# Windows VM over the same trees, and require identical output.
#
#   scripts/windows-parity.sh [--ref REF] [--keep-running]
#
# CI builds Windows artifacts but never runs cgg on Windows, so platform
# bugs ship unseen. The ones that matter are paths and ids: before 0.13.3
# the walker reported native `\` paths, and because stable ids hash the
# path, every Windows id differed from Linux (issue #14). Such bugs are
# per-language — include resolution, package roots, module names — so the
# trees come from the benchmark corpus and cover every language
# (scripts/windows-parity-repos.txt; the gate fails if one is missing).
#
# For each repository both hosts write the graph (-t json) with its audit
# sidecar, the dead-code report and the externals report. Every document
# must match after dropping timings (`*_ms`, `peak_rss_bytes`) and the
# recorded command line (`argv`), and no
# Windows path may contain `\`. Timing is never compared.
#
# What is tested:
#   default        the working tree as it is now, tracked and untracked
#                  (snapshotted through a temporary index; the real index,
#                  stash and refs are untouched) — what release.sh releases
#   --ref REF      a commit (tag, branch, sha)
# Either way, both hosts build that one commit; Linux builds it in a
# temporary worktree, never the live tree.
#
# The VM (default /storage/windows_vm, see the push skill §4b) is started
# if it is stopped, and stopped again afterwards unless --keep-running.
# Builds and the corpus are cached in the VM, so later runs are
# incremental; the first build there takes ~15 minutes.
#
# Environment:
#   CGG_WINDOWS_VM_DIR   VM compose dir (default /storage/windows_vm)
#   CGG_BENCH_DIR        corpus (default /storage/cgg-test_repos)
#
# Exit: 0 parity, 1 a difference or failure, 2 VM or corpus unavailable.
#
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT" || exit 1

VM_DIR="${CGG_WINDOWS_VM_DIR:-/storage/windows_vm}"
CORPUS="${CGG_BENCH_DIR:-/storage/cgg-test_repos}"
REPOS="$ROOT/scripts/windows-parity-repos.txt"
WORK="$ROOT/target/windows-parity"
# The Linux copy of the trees must sit outside any git repository: cgg
# honours parent .gitignore files, and this repo ignores target/, so a
# copy under it would analyze nothing. The VM copy has no parent repo.
TREES="${XDG_CACHE_HOME:-$HOME/.cache}/cgg-windows-parity"
REF=""
KEEP=0

while [ $# -gt 0 ]; do
    case "$1" in
        --ref)          REF="${2:-}"; shift 2 ;;
        --keep-running) KEEP=1; shift ;;
        -h|--help)      sed -n '3,/^set -uo/p' "$0" | sed 's/^# \{0,1\}//;$d'; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 1 ;;
    esac
done

say()  { printf '[windows-parity] %s\n' "$*"; }
die()  { printf '[windows-parity] FAIL %s\n' "$*" >&2; exit 1; }
skip() { printf '[windows-parity] UNAVAILABLE %s\n' "$*" >&2; exit 2; }

[ -f "$VM_DIR/docker-compose.yml" ] || skip "no VM at $VM_DIR"
[ -f "$VM_DIR/ssh/id_ed25519" ]     || skip "no VM key at $VM_DIR/ssh/id_ed25519"
[ -d "$CORPUS" ]                    || skip "no corpus at $CORPUS"
command -v docker >/dev/null        || skip "docker not available"

SSH=(ssh -i "$VM_DIR/ssh/id_ed25519" -p 2222 -o BatchMode=yes -o ConnectTimeout=10
     -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile="$VM_DIR/ssh/known_hosts"
     garrett@127.0.0.1)
SCP=(scp -q -i "$VM_DIR/ssh/id_ed25519" -P 2222 -o BatchMode=yes
     -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile="$VM_DIR/ssh/known_hosts")
# Every remote command starts with a PATH that includes what winget
# installed: sshd sessions inherit the PATH from when sshd started.
PS_PRELUDE='$env:Path = [Environment]::GetEnvironmentVariable("Path","Machine") + ";" + [Environment]::GetEnvironmentVariable("Path","User"); $R = "C:\cgg-parity";'
win() { "${SSH[@]}" "$PS_PRELUDE $1" | tr -d '\r'; return "${PIPESTATUS[0]}"; }

rm -rf "$WORK/linux" "$WORK/windows"
mkdir -p "$WORK/linux" "$WORK/windows"

# ---------------------------------------------------------------- snapshot
if [ -n "$REF" ]; then
    COMMIT="$(git rev-parse --verify "$REF^{commit}")" || die "no such ref: $REF"
    say "testing $REF ($COMMIT)"
else
    # A commit of the working tree, untracked files included, built
    # through a throwaway index so nothing the user has staged moves.
    IDX="$(mktemp)"; rm -f "$IDX"
    GIT_INDEX_FILE="$IDX" git add -A . || die "snapshot (add)"
    TREE="$(GIT_INDEX_FILE="$IDX" git write-tree)" || die "snapshot (write-tree)"
    rm -f "$IDX"
    COMMIT="$(git commit-tree "$TREE" -p HEAD -m "windows-parity snapshot")" || die "snapshot (commit)"
    say "testing the working tree (snapshot $COMMIT on $(git rev-parse --short HEAD))"
fi
# Linux builds the same commit Windows does, from its own checkout: building
# the live tree let edits made during the run reach one side only. The
# target dir persists so dependencies stay cached between runs.
SRC="$WORK/worktree"
git worktree remove --force "$SRC" >/dev/null 2>&1 || true
git worktree add -q --detach "$SRC" "$COMMIT" || die "worktree for $COMMIT"
trap 'git -C "$ROOT" worktree remove --force "$SRC" >/dev/null 2>&1' EXIT
LINUX_TARGET="$ROOT/target/windows-parity-linux"

# ------------------------------------------------------------------ inputs
cargo run -q --release -p cgg --example plugin_ids --manifest-path "$SRC/Cargo.toml" \
    --target-dir "$LINUX_TARGET" > "$WORK/langs.txt" || die "listing languages"
say "$(wc -l < "$WORK/langs.txt") languages; $(grep -cv '^\s*\(#\|$\)' "$REPOS") corpus repositories"

python3 -I "$ROOT/scripts/windows-parity.py" pack --corpus "$CORPUS" --repos "$REPOS" \
    --out "$WORK/corpus.tar" || die "packing the corpus"
CORPUS_SHA="$(sha256sum "$WORK/corpus.tar" | cut -c1-16)"
if [ "$(cat "$TREES/.sha" 2>/dev/null)" != "$CORPUS_SHA" ]; then
    rm -rf "$TREES" && mkdir -p "$TREES"
    tar -xf "$WORK/corpus.tar" -C "$TREES" && echo "$CORPUS_SHA" > "$TREES/.sha"
fi
git -C "$TREES" rev-parse --git-dir >/dev/null 2>&1 \
    && die "$TREES is inside a git repository; set XDG_CACHE_HOME elsewhere"

git update-ref refs/windows-parity/head "$COMMIT"
git bundle create -q "$WORK/src.bundle" refs/windows-parity/head 2>/dev/null \
    || { git update-ref -d refs/windows-parity/head; die "bundling $COMMIT"; }
git update-ref -d refs/windows-parity/head

# ---------------------------------------------------------------------- VM
STARTED=0
if ! docker ps -q -f name=windows-vm -f status=running | grep -q .; then
    say "starting the VM"
    (cd "$VM_DIR" && docker compose start >/dev/null 2>&1) || skip "docker compose start failed in $VM_DIR"
    STARTED=1
fi
stop_vm() {
    if [ "$STARTED" = 1 ] && [ "$KEEP" = 0 ]; then
        say "stopping the VM"
        (cd "$VM_DIR" && docker compose stop >/dev/null 2>&1)
    fi
}
trap 'stop_vm; git -C "$ROOT" worktree remove --force "$SRC" >/dev/null 2>&1' EXIT

for _ in $(seq 1 60); do
    "${SSH[@]}" "exit 0" >/dev/null 2>&1 && break
    sleep 5
done
"${SSH[@]}" "exit 0" >/dev/null 2>&1 || die "VM did not answer SSH within 5 minutes"

# --------------------------------------------------------- Windows: build
"${SCP[@]}" "$WORK/src.bundle" garrett@127.0.0.1:cgg-parity.bundle || die "copying the source"
say "building in the VM"
win '
New-Item -ItemType Directory -Force $R | Out-Null
if (-not (Test-Path $R\src\.git)) { git init -q $R\src }
git -C $R\src fetch -q $HOME\cgg-parity.bundle refs/windows-parity/head
if ($LASTEXITCODE) { exit 1 }
git -C $R\src checkout -q -f --detach FETCH_HEAD; git -C $R\src clean -q -fdx
cargo build -q --release -p cgg --manifest-path $R\src\Cargo.toml --target-dir $R\target
exit $LASTEXITCODE' > "$WORK/windows-build.log" 2>&1 \
    || { tail -20 "$WORK/windows-build.log"; die "Windows build (log: $WORK/windows-build.log)"; }

# -------------------------------------------------------- Windows: corpus
if [ "$(win 'Get-Content $R\corpus\.sha -ErrorAction SilentlyContinue')" != "$CORPUS_SHA" ]; then
    say "sending the corpus ($(du -m "$WORK/corpus.tar" | cut -f1) MB)"
    "${SCP[@]}" "$WORK/corpus.tar" garrett@127.0.0.1:cgg-parity-corpus.tar || die "copying the corpus"
    win "
Remove-Item -Recurse -Force \$R\corpus -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force \$R\corpus | Out-Null
tar -xf \$HOME\cgg-parity-corpus.tar -C \$R\corpus
if (\$LASTEXITCODE) { exit 1 }
Set-Content -Path \$R\corpus\.sha -Value '$CORPUS_SHA' -NoNewline
exit 0" || die "extracting the corpus in the VM"
fi

# ------------------------------------------------------------- both: run
# Same flags, same relative paths, same --jobs on both hosts. (PowerShell
# variables are case-insensitive: a loop variable named $r would clobber $R.)
mapfile -t LIST < <(grep -v '^\s*\(#\|$\)' "$REPOS" | sed 's/\s*#.*//')

say "running on Linux"
LINUX_CGG="$LINUX_TARGET/release/cgg"
cargo build -q --release -p cgg --manifest-path "$SRC/Cargo.toml" --target-dir "$LINUX_TARGET" \
    || die "Linux build"
for r in "${LIST[@]}"; do
    (cd "$TREES" &&
     timeout 600 "$LINUX_CGG" -q --jobs 4 -t json "$r" -o "$WORK/linux/$r.graph.json" \
        --dead-code --dead-code-format json --dead-code-confidence low \
        --dead-code-report "$WORK/linux/$r.deadcode.json" &&
     timeout 600 "$LINUX_CGG" -q --jobs 4 "$r" --no-graph --externals=all \
        --externals-format json --externals-report "$WORK/linux/$r.externals.json") \
        || die "Linux run failed on $r"
done

say "running on Windows"
REPO_ARRAY="$(printf "'%s'," "${LIST[@]}")"
win "
Set-Location \$R\corpus
Remove-Item -Recurse -Force \$R\out -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force \$R\out | Out-Null
\$cgg = \"\$R\target\release\cgg.exe\"
foreach (\$repo in @(${REPO_ARRAY%,})) {
  & \$cgg -q --jobs 4 -t json \$repo -o \"\$R\out\\\$repo.graph.json\" --dead-code --dead-code-format json --dead-code-confidence low --dead-code-report \"\$R\out\\\$repo.deadcode.json\"
  if (\$LASTEXITCODE) { \"run failed: \$repo\"; exit 1 }
  & \$cgg -q --jobs 4 \$repo --no-graph --externals=all --externals-format json --externals-report \"\$R\out\\\$repo.externals.json\"
  if (\$LASTEXITCODE) { \"externals failed: \$repo\"; exit 1 }
}
tar -cf \$HOME\cgg-parity-out.tar -C \$R\out .
exit \$LASTEXITCODE" > "$WORK/windows-run.log" 2>&1 \
    || { tail -5 "$WORK/windows-run.log"; die "Windows run (log: $WORK/windows-run.log)"; }
"${SCP[@]}" garrett@127.0.0.1:cgg-parity-out.tar "$WORK/windows-out.tar" || die "fetching Windows output"
tar -xf "$WORK/windows-out.tar" -C "$WORK/windows"

# ----------------------------------------------------------------- compare
python3 -I "$ROOT/scripts/windows-parity.py" compare --linux "$WORK/linux" \
    --windows "$WORK/windows" --repos "$REPOS" --langs "$WORK/langs.txt"
