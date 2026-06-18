---
name: cgg-install
description: Install the `cgg` call-graph CLI on the user's machine. Trigger when the user says "install cgg", "set up cgg", "I don't have cgg", "how do I get cgg", or when another skill (like the `cgg` skill) reports `cgg` is not on PATH and the user wants it installed. Handles bootstrapping Rust via rustup if missing, the C toolchain check that tree-sitter grammars need, choosing between `cargo install --git` (end-user) and a clone-based dev install, PATH setup for `~/.cargo/bin`, and a post-install verification. Walks the user through each step rather than running long-running installs without confirmation.
---

# cgg-install — install cgg from source

`cgg` is distributed as a Rust crate (no prebuilt binaries today), so
installing it means compiling it. This skill is the procedure for
doing that on a machine that may not have Rust set up.

Total build time on a clean machine: ~3–8 minutes depending on CPU.
Most of that is one-time tree-sitter grammar compilation.

## Prerequisites — check before installing

Run these checks first. **Do not start the install until every
prerequisite is green or the user has confirmed the bootstrap step.**

```bash
# 1. Rust toolchain (cargo + rustc >= 1.85, for the 2024 edition)
command -v cargo && cargo --version
command -v rustc && rustc --version

# 2. git (needed for cargo install --git)
command -v git && git --version

# 3. C toolchain — tree-sitter grammars compile C code
command -v cc || command -v gcc || command -v clang
```

The Rust check is the one most often missing. The C toolchain check
is the one most often missed *until* the build fails 4 minutes in
with a `linker 'cc' not found` error — check it up front.

## Step 1 — Install Rust if missing

If `cargo` is not on PATH, propose installing via rustup:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
. "$HOME/.cargo/env"
```

**Stop and ask the user before running this** if they didn't already
explicitly opt in. It writes to `~/.cargo` and `~/.rustup`, modifies
shell rc files, and pulls down ~300 MB. Not something to do silently.

After installation:
- The user needs to either restart their shell or `source
  $HOME/.cargo/env` so `cargo` is on PATH for the current session.
- Verify: `rustc --version` should report 1.85 or newer. If older,
  run `rustup update stable`.

Windows: rustup has its own installer (`rustup-init.exe`) — point
the user to https://rustup.rs and pause.

## Step 2 — Install the C toolchain if missing

tree-sitter grammars are shipped as C source and compiled at install
time. If no C compiler is found:

- **Debian/Ubuntu:** `sudo apt install build-essential`
- **Fedora/RHEL:** `sudo dnf groupinstall "Development Tools"`
- **Arch:** `sudo pacman -S base-devel`
- **macOS:** `xcode-select --install` (CLT only; full Xcode not needed)
- **Windows:** install the "C++ build tools" workload of Visual Studio
  Build Tools, or use the `gnu` Rust toolchain plus MSYS2's `gcc`.

Again, **stop and ask** before invoking `sudo`. The user may want to
run the package manager command themselves.

## Step 3 — Install cgg

Two install paths. Pick based on what the user wants:

### 3a. End-user install (recommended)

The user just wants the `cgg` binary on PATH.

```bash
cargo install --git https://github.com/NeuralNotwerk/cgg --locked
```

Notes:
- `--locked` uses the committed `Cargo.lock` for reproducible builds.
- The binary lands in `~/.cargo/bin/cgg`.
- Re-running this command upgrades to the latest commit on the
  default branch.
- First-time build pulls ~40 grammar crates and takes several
  minutes. Don't cancel.

### 3b. Developer install

The user wants a local clone (to read source, file PRs, run the
benchmark script, etc.).

```bash
git clone https://github.com/NeuralNotwerk/cgg.git
cd cgg
cargo install --path crates/cgg --locked
```

Same destination (`~/.cargo/bin/cgg`), but they keep the source tree
for `./scripts/benchmark.sh`, `./scripts/install-skill.sh`, and so on.

## Step 4 — Verify PATH

If `cargo install` finishes but `cgg` is not found:

```bash
ls ~/.cargo/bin/cgg          # should exist after install
echo $PATH | tr ':' '\n' | grep -F "$HOME/.cargo/bin"
```

If `~/.cargo/bin` isn't on PATH, the rustup installer normally adds a
line to the shell rc, but a restart may be needed. Permanent fix:

```bash
# bash
echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.bashrc
# zsh
echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.zshrc
# fish
fish_add_path "$HOME/.cargo/bin"
```

Then `source` the rc file or open a new shell.

## Step 5 — Verify the install

```bash
cgg --help        # should print usage
cgg --version 2>/dev/null || true   # version subcommand may not exist
```

For a real smoke test, run cgg against itself if the user has the
source tree:

```bash
cgg ./crates --filter 'cgg::run$' -n 1
```

A few lines of mermaid output means everything works.

## Step 6 — Offer to install the skills

If the user did the developer install (clone), there's a companion
script:

```bash
./scripts/install-skill.sh
```

It installs the `cgg` and `cgg-install` skills into Claude Code,
Kiro, Cline, Roo Code, and/or OpenCode if they're detected. Ask
before running it.

## Common failures and fixes

| Symptom | Cause | Fix |
|---|---|---|
| `error: linker 'cc' not found` | No C toolchain | Step 2 above |
| `error: rustc 1.x.x is not supported` | Rust too old | `rustup update stable` |
| Build hangs/fails on a `tree-sitter-*` crate | OOM on small machines (each grammar compile is RAM-heavy) | `cargo install ... -j 2` to limit parallelism |
| `cgg: command not found` after install | `~/.cargo/bin` not on PATH | Step 4 above |
| `error: failed to compile … tree-sitter-…` on Windows MSVC | Missing C++ build tools | Install VS Build Tools "C++ build tools" workload |
| `Permission denied` writing to `~/.cargo` | `~/.cargo` owned by root from a previous `sudo cargo` | `sudo chown -R "$USER" ~/.cargo ~/.rustup` |

## Things to NOT do

- **Don't `sudo cargo install`.** It installs into root's home and
  leaves the user's `~/.cargo` perms broken. `cargo install` is meant
  to run as the user.
- **Don't suggest a system package manager (`apt install cgg`, `brew
  install cgg`).** No such package exists today.
- **Don't try to half-install** by downloading individual files or
  building a single crate — the workspace needs to build together.
- **Don't run the install non-interactively** (no `< /dev/null`,
  no piping `yes`). Build errors need to surface; the user may need
  to make platform-specific decisions mid-install.

## Uninstall

```bash
cargo uninstall cgg
# and if they did the developer install:
rm -rf /path/to/cgg
```

`cargo install` does not pollute outside `~/.cargo/bin/cgg` and
`~/.cargo/registry/` cache.
