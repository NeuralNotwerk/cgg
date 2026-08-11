# Publishing cgg

What exists, what it takes to ship it, and what you personally have to do
because it needs an account only you can own.

Ordered by effort-to-value. The first two are close to free; the last two
are real projects.

| Target | Artifact | Status | Blocked on you |
| --- | --- | --- | --- |
| GitHub Releases | source + binaries | shipping | nothing |
| crates.io | 6 Rust crates | **ready, blocked** | verified email |
| PyPI | `cgg` wheel | buildable, not packaged for CI | account + token |
| GitHub Releases (bin) | CLI + `libcgg` | not wired | nothing |
| npm | Node module | **wrapper not written** | account + token |
| NuGet | .NET package | **wrapper not written** | account + key |
| Maven Central | Java artifact | **wrapper not written** | Sonatype + GPG |

---

## 1. crates.io — ready now, one thing blocking

Everything on the code side is done: names verified free, metadata
complete, per-crate READMEs, `cargo package` clean.

**The blocker:** the account has no verified email, so every upload is
rejected:

```
400 Bad Request: A verified email address is required to publish crates
to crates.io. Visit https://crates.io/settings/profile
```

Nothing was consumed by the failed attempt — all six names are still free.

### What you do

1. <https://crates.io/settings/profile> — set an email, then click the
   link in the confirmation message. This is the whole blocker.
2. <https://crates.io/settings/tokens> — a token scoped `publish-new` and
   `publish-update`. (You already have one at `~/.crates.io.token`; a
   fresh one is only needed if that one lacks `publish-new`.)
3. `cargo login < ~/.crates.io.token`

### Then

```bash
scripts/publish-crates.sh --dry-run   # packages everything, uploads nothing
scripts/publish-crates.sh             # asks you to type the version
```

The script exists because a workspace cannot be published in one command.
Each crate's dependencies must already **be on crates.io** before it can
even be packaged, so the order is fixed —

```
cgg-core → cgg-walk → cgg-format → cgg-lang → cgg-resolve → cgg
```

— and the index is a CDN, so the script waits for each crate to become
visible before publishing whatever depends on it.

`cgg-py` and `cgg-ffi` are `publish = false`: the artifact anyone wants
from those is a wheel and a shared library, not a crate.

> **Publishing is forever.** A version can be yanked but never deleted,
> and the name is claimed permanently. Dry run first.

---

## 2. PyPI — the wheel already builds

`scripts/build-python.sh --wheel` produces a working wheel today. What is
missing is only the multi-platform build and the upload.

`abi3-py39` means **one wheel per platform covers every CPython ≥ 3.9**,
so the matrix is platforms, not platform × Python version — about six
wheels, not thirty.

### The name

**Distribution `cgg-callgraphgenerator`, import `cgg`.** PyPI's `cgg` is
an unrelated GGUF tool — 49 releases, actively maintained — so the short
name was never available.

That package also ships a top-level `cgg` module, so the import names
collide. We took the collision knowingly: `import cgg` matches the CLI,
the crate and every example, and Python separates distribution from
import name routinely. The cost is that **the two packages must not share
an environment** — both write to `site-packages/cgg/` and pip will not
stop you. Documented in the README and the module README.

`cgg-callgraphgen` is also free if the shorter form is ever preferred;
only `[project].name` changes.

### Building a wheel PyPI will accept

**PyPI rejects plain `linux_x86_64` wheels.** Only `manylinux`-tagged
ones are accepted, and a wheel built on a modern box links a glibc newer
than most users have (this machine: 2.39). Build in maturin's container
instead — CentOS 7, glibc 2.17, so the wheel runs essentially everywhere:

```bash
docker run --rm -v "$PWD:/io" -w /io \
  -e CARGO_HOME=/io/target/manylinux-cargo \
  -e CARGO_TARGET_DIR=/io/target/manylinux \
  ghcr.io/pyo3/maturin build --release \
  -m crates/cgg-py/Cargo.toml --out /io/dist
```

Run it as **root** (the default). Passing `--user` fails with
`Permission denied` starting `cargo metadata`, because the toolchain in
the image is root-owned. Chown the outputs back afterwards:

```bash
docker run --rm -v "$PWD:/io" --entrypoint chown ghcr.io/pyo3/maturin \
  -R "$(id -u):$(id -g)" /io/dist /io/target/manylinux /io/target/manylinux-cargo
```

`scripts/publish-python.sh` wraps both steps plus the upload.

### Upload

```bash
scripts/publish-python.sh --check    # build + twine check, no upload
scripts/publish-python.sh            # uploads, asks you to confirm
```

Needs a token at <https://pypi.org/manage/account/token/> in
`~/.pypi.token`. Scope it to the project after the first upload; the
first one needs an account-wide token because the project does not exist
yet.

### Other platforms

The command above produces a Linux x86_64 wheel only. macOS and Windows
need a CI matrix — `PyO3/maturin-action` across `ubuntu-latest`,
`macos-latest` (x86_64 + aarch64) and `windows-latest`. `abi3-py39` means
one wheel per *platform* covers every CPython ≥ 3.9, so that is about six
wheels, not thirty.

**Size note:** ~99 MB on disk, ~10 MB compressed — comfortably under
PyPI's 100 MB per-file limit, but do not add grammars carelessly.

---

## 3. Prebuilt binaries on GitHub Releases — free, do it next

The cheapest reach for the most users, and it needs no account at all.
Every release should attach:

- `cgg-<version>-<target>.tar.gz` — the CLI
- `libcgg-<version>-<target>.tar.gz` — `libcgg.so`/`.a` + `cgg.h`, so C,
  .NET, Java and Go users have something to link without a Rust toolchain

`taiki-e/upload-rust-binary-action` does the CLI half in a few lines.
This also unblocks a Homebrew tap and a Scoop manifest later, both of
which just point at these URLs.

---

## 4. npm, NuGet, Maven Central — wrappers first

**None of these have a wrapper yet.** The C ABI (`crates/cgg-ffi`) is the
foundation all three ride on, and it is done and tested — but the
language-side binding is still to write.

Rough order of effort:

### npm (Node)

Two routes:

- **napi-rs** (recommended) — a native module, best DX, `@cgg/core` plus
  per-platform `@cgg/core-linux-x64-gnu` packages. Mirrors what
  `crates/cgg-py` already does; `napi-rs/action` handles the matrix.
- **ffi-napi over the C ABI** — no native build, but slower and drags in
  a fragile dependency. Not worth it here.

You need: an npm account, `NPM_TOKEN` as a secret, and an `@yourscope`
organisation if you want scoped packages.

### NuGet (.NET)

The easiest of the three. P/Invoke over `cgg.h` is pure C# — no native
build on the .NET side — and native libraries ship in
`runtimes/<rid>/native/` inside one package.

You need: a nuget.org account and an API key.

### Maven Central (Java)

The most painful, entirely for reasons outside the code. Use the
**Foreign Function & Memory API** (JDK 22+) rather than JNI — no C shim
to compile, and `jextract` generates bindings from `cgg.h` directly. Fall
back to JNA only if you must support older JDKs.

You need: a Sonatype Central account, a **verified namespace** (proving
you control `io.github.neuralnotwerk` or a domain), and a **published GPG
key** — every artifact must be signed. Budget a day for the account
setup alone, separate from the code.

---

## Recommended order

1. **Verify the crates.io email** and run `scripts/publish-crates.sh`.
   Everything is ready; this is minutes of your time.
2. **Attach binaries to GitHub Releases.** No account, wide reach,
   unblocks Homebrew/Scoop later.
3. **PyPI.** The wheel already works; this is a CI workflow.
4. **npm**, then **NuGet**, then **Maven** — each needs its wrapper
   written first, and each is a genuinely separate piece of work.

A note on doing all of them: every registry is a permanent commitment to
users who install from it. Four half-maintained bindings serve people
worse than two that work. The C ABI is deliberately shaped so that adding
a language is a source-only wrapper — no new native artifact — so there
is no rush to claim them all at once.
