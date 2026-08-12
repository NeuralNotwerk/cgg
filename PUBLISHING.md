# Publishing cgg

What exists, what it takes to ship it, and what you personally have to do
because it needs an account only you can own.

Everything below was checked against the live registries and the workflow
on 2026-08-12. Where a claim is about the world rather than the repo, the
command that proves it is printed next to it.

| Target | Artifact | Status | Blocked on |
| --- | --- | --- | --- |
| crates.io | 6 Rust crates | **published, 0.6.3** | nothing |
| PyPI | `cgg-callgraphgenerator` wheel | **published, 0.6.3** (Linux x86_64 only) | a tag, for the other platforms |
| GitHub Releases (source) | auto tarballs | shipping (7 releases) | nothing |
| GitHub Releases (binaries) | CLI + `libcgg` | **wired in CI, never run** | a tag |
| npm | `cgg-callgraphgenerator` | **wrapper written + green in CI, NOT published** | a tag |
| NuGet | .NET package | wrapper not written | account + key |
| Maven Central | Java artifact | wrapper not written | Sonatype + GPG |

**The single thing blocking three of those rows is a tag.** The newest tag
is `v0.6.1`; the release workflow landed *after* it, so no tag has ever
exercised it. Meanwhile 0.6.2 and 0.6.3 went to crates.io and PyPI from
this machine, so the registries are ahead of the tags.

```bash
git tag --list 'v*'                                    # newest: v0.6.1
git log --oneline --follow -- .github/workflows/release.yml | tail -1
                                                       # dbf2612, after v0.6.1
gh api repos/NeuralNotwerk/cgg/releases \
  -q '.[] | "\(.tag_name) assets=\(.assets | length)"' # every one: assets=0
```

---

## 0. Before any of it: `scripts/release.sh`

```bash
scripts/release.sh --purpose "what this release is for"
```

Runs every gate this project has, in order — build, test, clippy, fmt,
docs-check, determinism — then measures perf against the previous release,
then drafts the CHANGELOG from the measured numbers. **It never commits,
tags or pushes.** It prints the commands and stops.

`--quick` for gates only, `--skip-ai` to measure without generating prose,
`--skip-perf` to skip the corpus comparison. Version numbers come from
`Cargo.toml` unless you pass `--version`.

Published performance numbers are taken at `--jobs 1`. Do not put a number
in the CHANGELOG that this script did not produce.

---

## 1. crates.io — published

All six publishable crates are live at 0.6.3.

```bash
# crates.io rejects curl's default User-Agent; -A is not optional here.
for c in cgg cgg-core cgg-walk cgg-format cgg-lang cgg-resolve; do
  curl -s -A cgg-release-check "https://crates.io/api/v1/crates/$c" \
    | jq -r --arg c "$c" '"\($c) \(.crate.max_version)"'
done
```

```bash
scripts/publish-crates.sh --dry-run   # packages everything, uploads nothing
scripts/publish-crates.sh             # asks you to type the version
```

Prerequisites, for the record:

1. A **verified email** on the account. Without it every upload is
   rejected with `400 A verified email address is required` — nothing is
   consumed, but nothing publishes either.
2. A token from <https://crates.io/settings/tokens>, then
   `cargo login < /path/to/token`.

### Two things that bit on the first release

**New-crate rate limiting.** crates.io allows a burst of new crates and
then roughly one per ten minutes. A six-crate workspace hits it — ours
failed on the sixth and most important one with `429 Too Many Requests`,
after the other five had published. The version is not consumed by a
failed upload, so the fix is to wait and retry that crate alone.
`publish_with_retry` in the script now greps for `429 Too Many Requests`,
parses the retry time out of the message and sleeps until it. Publishing a
new *version* of an existing crate is not limited, so this is a
first-release problem only.

**Order is not optional.** Each crate must be on crates.io before anything
depending on it can even be packaged, and the sparse index is a CDN, so
`wait_for_index` polls `https://index.crates.io/…` for the exact version
before continuing.

The fixed order, from `CRATES=(…)` in the script:

```
cgg-core → cgg-walk → cgg-format → cgg-lang → cgg-resolve → cgg
```

`cgg-py`, `cgg-ffi` and `cgg-node` are `publish = false`: the artifact
anyone wants from those is a wheel, a shared library and an npm package,
not a crate.

> **Publishing is forever.** A version can be yanked but never deleted,
> and the name is claimed permanently. Dry run first.

No workflow publishes crates.io. It is the one registry where a bad upload
cannot be corrected by re-running, so it stays a deliberate local act.

---

## 2. PyPI — published (Linux x86_64)

Live: <https://pypi.org/project/cgg-callgraphgenerator/>

```bash
pip install cgg-callgraphgenerator
```

Three releases are up — 0.6.1, 0.6.2, 0.6.3 — and each carries exactly one
manylinux x86_64 wheel. Only 0.6.1 also has an sdist. macOS and Windows
wheels build green in CI but have never been uploaded, because that
happens on a tag and there has not been one.

```bash
curl -s https://pypi.org/pypi/cgg-callgraphgenerator/json \
  | jq '.releases | map_values(map(.filename))'
```

`abi3-py39` means **one wheel per platform covers every CPython ≥ 3.9**, so
the matrix is platforms, not platform × Python version — five wheels in
`release.yml`, not thirty-five.

### The name

**Distribution `cgg-callgraphgenerator`, import `cgg`.** PyPI's `cgg` is an
unrelated GGUF tool, so the short name was never available. npm's `cgg` is
taken too — a ChampionGG API wrapper, five versions, untouched since 2022 —
which is why the Node package carries the same long name.

```bash
curl -s https://registry.npmjs.org/cgg | jq '{name, description}'
```

The PyPI package also ships a top-level `cgg` module, so the import names
collide. We took the collision knowingly: `import cgg` matches the CLI, the
crate and every example, and Python separates distribution from import name
routinely. The cost is that **the two packages must not share an
environment** — both write to `site-packages/cgg/` and pip will not stop
you. Documented in the README and the module README.

`cgg-callgraphgen` is also free if the shorter form is ever preferred; only
`[project].name` changes.

### Building a wheel PyPI will accept

**PyPI rejects plain `linux_x86_64` wheels.** Only `manylinux`-tagged ones
are accepted, and a wheel built on a modern box links a glibc newer than
most users have. Build in maturin's container instead:

```bash
docker run --rm -v "$PWD:/io" -w /io \
  -e CARGO_HOME=/io/target/manylinux-cargo \
  -e CARGO_TARGET_DIR=/io/target/manylinux \
  ghcr.io/pyo3/maturin build --release \
  -m crates/cgg-py/Cargo.toml --out /io/dist
```

Run it as **root** (the default). Passing `--user` fails with
`Permission denied` starting `cargo metadata`, because the toolchain in the
image is root-owned. Chown the outputs back afterwards:

```bash
docker run --rm -v "$PWD:/io" --entrypoint chown ghcr.io/pyo3/maturin \
  -R "$(id -u):$(id -g)" /io/dist /io/target/manylinux /io/target/manylinux-cargo
```

`scripts/publish-python.sh` wraps both steps, asserts the wheel is
manylinux-tagged, checks the wheel's embedded description still matches
`README.md` on disk — 0.6.1 shipped a stale one telling everyone to
`pip install cgg`, the wrong project, and PyPI metadata cannot be edited
after upload — then installs it into a venv, runs pytest and twine.

### Upload

```bash
scripts/publish-python.sh --check    # build + twine check, no upload
scripts/publish-python.sh            # uploads, asks you to confirm
```

Needs a token at <https://pypi.org/manage/account/token/> in
`~/.pypi.token` (or `$PYPI_TOKEN_FILE`). Scope it to the project after the
first upload; the first one needs an account-wide token because the project
does not exist yet.

**Size.** The published 0.6.3 wheel is **10.1 MB** compressed and unpacks to
**104 MB**, essentially all of it `cgg/_cgg.abi3.so` at 103.9 MB. PyPI's
limit applies to the uploaded file, which is the 10.1 MB one, and the
default cap is 100 MiB — so there is an order of magnitude of headroom, but
44 vendored grammars is what fills the 104 MB. Add more carefully.

---

## 2b. Releasing from a tag (CI)

`.github/workflows/release.yml` triggers on `push` of a `v*` tag and on
`workflow_dispatch`. Five jobs:

| Job | Matrix | Produces |
| --- | --- | --- |
| `wheels` | linux x86_64/aarch64, macOS x86_64/arm64, windows x64 | 5 abi3 wheels, smoke-tested by importing and analysing |
| `sdist` | — | source distribution |
| `node` | linux x64/arm64-gnu, darwin x64/arm64, win32 x64-msvc | 5 `.node` binaries, smoke-tested by `require()` + analyse |
| `binaries` | linux x86_64, macOS arm64, windows x64 | `cgg` + `libcgg`/`cgg.dll` + `cgg.h` + licences, one tar.gz per platform |
| `publish` | needs all four | GitHub release assets, PyPI upload, npm publish |

`NPM_TOKEN` and `PYPI_API_TOKEN` exist as repository secrets and the
`release` environment exists, so a tag publishes without anyone holding a
token locally. The environment currently has **no protection rules** — add
an approval gate in repo settings if you want one; the publish job already
targets it.

```bash
gh api repos/NeuralNotwerk/cgg/actions/secrets -q '.secrets[].name'
gh api repos/NeuralNotwerk/cgg/environments \
  -q '.environments[] | "\(.name) rules=\(.protection_rules|length)"'
```

```bash
git tag -a v0.6.4 -m "…" && git push origin v0.6.4
```

`workflow_dispatch` runs the identical matrix and publishes **nothing** —
the publish job is `if: startsWith(github.ref, 'refs/tags/v')`. Dispatch
first; a tag is the expensive way to discover a typo. The most recent
dispatch (run 31554017653) was green: all 14 build jobs succeeded in about
five minutes and `publish` was skipped, exactly as designed.

```bash
gh run list --workflow=release.yml -L 1
gh run view 31554017653 --json jobs -q '.jobs[] | "\(.name) \(.conclusion)"'
```

Two things the tag path does **not** cover:

* **crates.io.** No workflow publishes it; run `scripts/publish-crates.sh`
  by hand. See §1.
* **Re-running a tag.** PyPI's step sets `skip-existing: true`, so an
  already-uploaded file is skipped rather than failing. npm has no
  equivalent: a version already on the registry makes the npm step fail.
  Do not publish a version locally and then tag it.

The PyPI step still passes `password: ${{ secrets.PYPI_API_TOKEN }}`.
Trusted publishing would remove the token entirely — configure the
publisher at
<https://pypi.org/manage/project/cgg-callgraphgenerator/settings/publishing/>
(owner `NeuralNotwerk`, repo `cgg`, workflow `release.yml`, environment
`release`) and delete that line.

---

## 3. Prebuilt binaries on GitHub Releases — wired, waiting on a tag

The `binaries` job builds the CLI and the C ABI for linux-x86_64,
macos-arm64 and windows-x64, packs each with `cgg.h`, `README.md` and both
licences into `cgg-<tag>-<platform>.tar.gz`, and `publish` attaches them
with `softprops/action-gh-release`. None of the seven existing releases has
a single asset, because all seven predate that workflow.

Note the current archive naming is one tar.gz per platform containing both
the CLI and `libcgg` — not the separate `cgg-…`/`libcgg-…` archives an
earlier draft of this document proposed. If you want them split, change the
`Package` step; nothing downstream depends on the current shape yet.

Attached binaries are also what a Homebrew tap or a Scoop manifest would
point at, so the first tag unblocks both.

---

## 4. npm — blocked on the token type, not on the code

The five platform packages build green in CI and `napi prepublish
--dry-run` passes locally against the real artifacts. Publishing fails:

```
403 … Two-factor authentication or granular access token with bypass
2fa enabled is required to publish packages.
```

The account requires 2FA on publish; a classic read/publish token cannot
satisfy that. Generate one of:

* **Automation token** — npmjs.com → Access Tokens → Generate New Token →
  *Classic* → **Automation**. Bypasses 2FA; this is the CI-intended type.
* **Granular Access Token** with *Bypass 2FA* enabled, scoped to the
  `cgg-callgraphgenerator*` packages.

Then `gh secret set NPM_TOKEN` and re-run the release workflow, or publish
by hand:

```bash
cd crates/cgg-node
gh run download <run-id> -D /tmp/art -p 'node-*'
npx --yes --package=@napi-rs/cli@3.8.5 -- napi artifacts -d /tmp/art --npm-dir npm
npx --yes --package=@napi-rs/cli@3.8.5 -- napi prepublish -t npm --dry-run
npx --yes --package=@napi-rs/cli@3.8.5 -- napi prepublish -t npm
```

**`napi artifacts` is not optional.** `prepublish` validates that every
`npm/<triple>/` package already contains its `.node` and aborts with
"Release package … is incomplete" otherwise; copying the files to the
crate root is not enough. And there is no `--skip-gh-release` —
`--gh-release` is opt-in. Both cost a tagged release to discover.


**The wrapper exists.** `crates/cgg-node` is a napi-rs N-API module — not
`ffi-napi` over the C ABI, which would have been slower and dragged in a
fragile dependency. npm needs a per-platform artifact either way, so the C
ABI bought nothing here.

```bash
curl -s -o /dev/null -w '%{http_code}\n' \
  https://registry.npmjs.org/cgg-callgraphgenerator     # 404 — not published
```

Package layout, from `crates/cgg-node/package.json` and `npm/*/`:

* main package `cgg-callgraphgenerator` (`index.js` + `index.d.ts`)
* five platform packages `cgg-callgraphgenerator-{linux-x64-gnu,
  linux-arm64-gnu, darwin-x64, darwin-arm64, win32-x64-msvc}`

Unscoped, so no npm organisation is needed. `napi prepublish` moves each
`.node` into its platform package, publishes those, then publishes the main
package that depends on them — order matters, since the main package's
`optionalDependencies` must already exist. That is what the `Publish to
npm` step runs, using `NPM_TOKEN`, which is already set.

So: **an npm account owning the name is the only remaining requirement, and
the first `v*` tag ships it.**

Build and test it locally:

```bash
cd crates/cgg-node
npx --yes --package=@napi-rs/cli@3.8.5 -- napi build --platform --release
node --test tests/*.test.js     # includes parity against target/release/cgg
```

---

## 5. NuGet and Maven Central — wrappers first

Neither has a wrapper. The C ABI (`crates/cgg-ffi`, seven exported
functions, header at `crates/cgg-ffi/include/cgg.h`) is the foundation both
ride on, and it is done and tested — but the language-side binding is still
to write.

### NuGet (.NET)

The easier of the two. P/Invoke over `cgg.h` is pure C# — no native build
on the .NET side — and native libraries ship in `runtimes/<rid>/native/`
inside one package. The `binaries` job already produces the `libcgg` half
for three platforms.

You need: a nuget.org account and an API key.

### Maven Central (Java)

The most painful, entirely for reasons outside the code. The **Foreign
Function & Memory API** (JDK 22+) avoids a JNI shim and `jextract` can
generate bindings from `cgg.h` directly; JNA is the fallback if older JDKs
must be supported.

You need: a Sonatype Central account, a **verified namespace** (proving you
control `io.github.neuralnotwerk` or a domain), and a **published GPG
key** — every artifact must be signed. Budget the account setup separately
from the code.

---

## Recommended order

1. **Tag.** `scripts/release.sh --purpose …`, then `git tag -a v0.6.4`.
   One action ships the npm package, the macOS/Windows wheels and the first
   prebuilt binaries, all of which already build green.
2. **Run `scripts/publish-crates.sh`** for the same version. It is the one
   registry CI deliberately does not touch.
3. **Switch PyPI to trusted publishing** and delete `PYPI_API_TOKEN`.
4. **NuGet**, then **Maven** — each needs its wrapper written first, and
   each is a genuinely separate piece of work.

A note on doing all of them: every registry is a permanent commitment to
users who install from it. Four half-maintained bindings serve people worse
than two that work. The C ABI is deliberately shaped so that adding a
language is a source-only wrapper — no new native artifact — so there is no
rush to claim them all at once.
