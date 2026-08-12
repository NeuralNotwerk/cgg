# Publishing cgg

What exists, what it takes to ship it, and what you personally have to do
because it needs an account only you can own.

Everything below was checked against the live registries and the workflow.
Where a claim is about the world rather than the repo, the command that
proves it is printed next to it.

Only four channels exist: crates.io, PyPI, npm and GitHub Releases. See
[What this project does not publish](#what-this-project-does-not-publish).

| Target | Artifact | Status | Blocked on |
| --- | --- | --- | --- |
| crates.io | 6 Rust crates | **published** | nothing |
| PyPI | `cgg-callgraphgenerator` wheel + sdist | **published** | nothing |
| GitHub Releases (source) | auto tarballs | shipping (7 releases) | nothing |
| GitHub Releases (binaries) | CLI + `libcgg` + `cgg.h` | **published** | nothing |
| npm | `cgg-callgraphgenerator` + 5 platform packages | **published** | nothing |

All four ship from one `v*` tag, except crates.io, which stays manual on
purpose — it is the one registry where a bad upload cannot be corrected by
re-running.

```bash
git tag --list 'v*'                                    # newest: v0.6.1
git log --oneline --follow -- .github/workflows/release.yml | tail -1
                                                       # dbf2612, after v0.6.1
gh api repos/NeuralNotwerk/cgg/releases \
  -q '.[] | "\(.tag_name) assets=\(.assets | length)"' # every one: assets=0
```

---

## 0a. Before any of it: `scripts/security-check.sh`

```bash
scripts/security-check.sh          # everything, including git history
scripts/security-check.sh --quick  # skip the history sweep
```

`release.sh` answers *is this correct?*. This answers *is it safe to make
public?* — a different question, and it has to come first, because
publishing is irreversible and a leaked credential stays leaked after you
yank the release.

Eight checks: trufflehog over the working tree **and** git history; a
direct byte-search for this machine's actual tokens in both; no
credential-shaped files in the repo; `.env` is gitignored; `cargo-deny`
advisories/licences/bans/sources and `npm audit`; no workflow step that
could print a secret; what `cargo package` and `npm pack` would actually
ship; and the permissions on your local token files.

Two things it took a wrong answer to learn, both now baked in:

* **`--results=verified,unknown` is not enough.** trufflehog only marks a
  finding `verified` when it can authenticate against the live API, so a
  real credential that has since been rotated reports as `unverified`.
  Tested against three randomly generated AWS / GitHub / npm
  credentials: that filter finds **zero** of them. The script uses
  `verified,unknown,unverified`.
* **The `Lob` detector is excluded**, narrowly and on purpose. It matches
  `test_<alnum>`, which is every pytest function name in
  `crates/cgg-py/tests`, and Lob's test endpoint accepts any such string
  — so twelve function names get reported as *verified secrets*. If a Lob
  dependency is ever added, drop the exclusion.

---

## 0. Then: `scripts/release.sh`

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

```text
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

## 3. Prebuilt binaries on GitHub Releases — published

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

## 4. npm — published

`cgg-callgraphgenerator` plus five platform packages
(`-linux-x64-gnu`, `-linux-arm64-gnu`, `-darwin-x64`, `-darwin-arm64`,
`-win32-x64-msvc`), listed as `optionalDependencies` so npm installs only
the one matching the host. Verified from a bare container: `npm install`
pulls **two** packages and nothing else.

```bash
cd crates/cgg-node
npx --yes --package=@napi-rs/cli@3.8.5 -- napi build --platform --release
node --test tests/*.test.js     # includes parity against target/release/cgg
```

### The token has to be able to CREATE packages

Two different 403s stand between a fresh token and a first publish, and
the second one is easy to misread.

**A classic read/publish token** on a 2FA-protected account:

```text
403 … Two-factor authentication or granular access token with bypass
2fa enabled is required to publish packages.
```

**A granular token scoped to "Only select packages"**:

```text
403 Forbidden - PUT … - You may not perform that action with these
credentials.
```

The second is the trap: publishing a name that does not exist yet is a
package *creation*, and you cannot pre-select a package that does not
exist. Use a classic **Automation** token, or a granular token with
*Packages and scopes: Read and write* over **all packages**. Once the six
names exist, a narrower granular token can take over.

### Two things that cost a tagged release to find

* **`napi artifacts` is not optional.** `prepublish` validates that every
  `npm/<triple>/` package already contains its `.node` and aborts with
  "Release package … is incomplete" otherwise. Copying the artifacts to
  the crate root is not enough.
* **There is no `--skip-gh-release`.** `--gh-release` is opt-in in
  `@napi-rs/cli` v3; passing the skip form aborts the job.

---

## What this project does not publish

The C ABI (`crates/cgg-ffi`) is deliberately shaped so that a .NET, Java
or Go binding is a source-only wrapper over one shared library.
