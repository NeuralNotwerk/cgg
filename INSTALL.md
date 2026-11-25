# Installing cgg — verified requirements per channel

Every row below was established by starting from a bare `ubuntu:24.04`
container with nothing but `tar`, installing only what the channel refused
to work without, and then analysing a four-language fixture. The
containers are kept for post-publish retesting (`cgg-test-*`).

| Channel | Apt packages needed | Also needed | Compiles? |
| --- | --- | --- | --- |
| **GitHub binaries** | `ca-certificates curl` | — | no |
| **npm** | `nodejs npm` | — | no |
| **PyPI** | `python3 python3-pip python3-venv` | — | no (wheel platforms) |
| **crates.io** | `ca-certificates curl gcc libc6-dev` | rustup ≥ 1.85 | **yes** |
| **GitHub source** | `ca-certificates curl git gcc libc6-dev` | rustup ≥ 1.85 | **yes** |

All five produce a byte-identical graph from the same input.

---

## GitHub release binaries — the cheapest install

```bash
apt-get install -y --no-install-recommends ca-certificates curl
curl -fsSL -o cgg.tar.gz \
  https://github.com/NeuralNotwerk/cgg/releases/download/v0.13.3/cgg-v0.13.3-linux-x86_64.tar.gz
mkdir -p /opt/cgg && tar xzf cgg.tar.gz -C /opt/cgg
install -m755 /opt/cgg/cgg /usr/local/bin/cgg
```

`tar` is already in the base image. The binary links **only** `libc`,
`libgcc_s` and the loader — verified with `ldd` — so there is nothing else
to install. The tarball also carries `cgg.h`, `libcgg.so` and `libcgg.a`
for the C ABI.

Each release publishes `linux-x86_64`, `linux-aarch64`, `macos-arm64` and
`windows-x64` tarballs (`cgg-v<version>-<platform>.tar.gz`). There is no
Intel-macOS binary; use pip, npm or cargo there.

## npm

```bash
apt-get install -y --no-install-recommends nodejs npm
npm install cgg-callgraphgenerator
```

Ubuntu 24.04's Node 18 is new enough (the module targets N-API 8). Installs
**two** packages — the root plus only the host's platform binary — because
the platform builds are `optionalDependencies`. No compiler, no Rust.

## PyPI

```bash
apt-get install -y --no-install-recommends python3 python3-pip python3-venv
python3 -m venv /opt/v && /opt/v/bin/pip install cgg-callgraphgenerator
```

**`python3-venv` is not optional on Ubuntu 24.04.** The system Python is
PEP 668 "externally managed", so a plain `pip3 install` refuses:

```text
error: externally-managed-environment
```

`--break-system-packages` also works but is what the error is warning you
against.

Wheels are published for x86-64 and arm64 Linux, Intel and Apple-silicon
macOS, and x86-64 Windows, so pip takes a prebuilt one on all of those and
needs no toolchain. Off that list — musl Linux, Windows on arm64 — it
falls back to the sdist, which needs Rust ≥ 1.85 and several minutes.

## crates.io

```bash
apt-get install -y --no-install-recommends ca-certificates curl gcc libc6-dev
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
. "$HOME/.cargo/env"
cargo install cgg
```

Two requirements that are easy to get wrong:

* **Ubuntu's Rust is too old.** 24.04 ships 1.75; cgg needs **1.85** for
  edition 2024. `apt install cargo` cannot build it — rustup is required.
  `--profile minimal` is enough.
* **A C compiler is required**, and the failure is opaque if it is
  missing: `error: linker 'cc' not found`. cgg compiles C — every
  tree-sitter grammar, blake3 and mimalloc — so `gcc` and `libc6-dev` are
  needed even though cgg is a Rust program.

## GitHub source

```bash
apt-get install -y --no-install-recommends ca-certificates curl git gcc libc6-dev
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
. "$HOME/.cargo/env"
git clone --depth 1 --branch v0.13.3 https://github.com/NeuralNotwerk/cgg
cd cgg && cargo build --release -p cgg
```

Same as crates.io plus `git`. A shallow clone of v0.13.3 transfers about
6.5 MB (66 MB checked out); most grammars come from crates.io, and the
Smithy, Kivy, Groovy and Lean 4 grammars are vendored.

Use `-p cgg`, not a bare `cargo build`: the workspace root is a virtual
manifest, and building everything also builds the Python, C and Node
bindings, which is a lot of extra work for a CLI.

## What none of them need

No network access at runtime, no language servers, no build artifacts from
the analysed project, and no configuration. cgg reads source files and
nothing else.
