<!-- markdownlint-disable MD013 -->
<!-- CLAUDE.md is agent-facing prose: long unwrapped paragraphs on
     purpose, because an agent reads the file whole and rewrapping
     would churn it on every edit. Every other rule still applies. -->

# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

`scripts/docs-check.py` reads this file (check 7). If you change the
self-analysis `--filter` below, change it in `.githooks/pre-commit`,
`scripts/update-readme-stats.sh`, `scripts/patch-readme-stats.py` and
`README.md` too, or the next commit fails.

## What this is

`cgg` is a CLI that generates call graphs (mermaid by default; also json/dot/graphml) from source trees. It is offline, deterministic, single-binary — no language servers, no build artifacts required. The same pipeline is also a Rust library (`cgg::analyze`), a Python extension module (`import cgg`, `crates/cgg-py`), a C ABI (`crates/cgg-ffi`) and a Node/N-API module (`crates/cgg-node`); the CLI remains a single static binary with no Python or Node linkage, and none of the four front ends may fork the resolver ordering. Supports 44 languages via tree-sitter plugins (including the Smithy, Protobuf, GraphQL, OpenAPI/Swagger, and AsyncAPI interface/descriptor languages, whose shape graphs are mapped onto the call-graph model), plus Jupyter notebooks (`.ipynb`) via a JSON cell extractor that feeds the Python plugin. OpenAPI/AsyncAPI documents are YAML or JSON, both parsed with the YAML grammar and content-detected by their root `openapi:`/`swagger:`/`asyncapi:` key (see `cgg-lang::detect`), so ordinary `.yaml`/`.json` files are untouched. Primary consumer of the output is coding agents reading mermaid in their context window.

## Commands

```bash
# Build the CLI (workspace member `cgg`)
cargo build --release -p cgg

# Run the full test suite — this is what the pre-commit hook gates on
cargo test --workspace

# Run a single crate's tests
cargo test -p cgg-resolve

# Run a single test by name
cargo test -p cgg-lang python_shebang_beats_extension

# Run cgg against itself (sanity check) — this is also the README showcase
# graph. `analyze_in_pool` is the pipeline body; `cgg::run` is a private
# shim in main.rs and matches several test helpers of the same name.
./target/release/cgg ./crates -t mermaid --filter 'cgg::analyze_in_pool$' -n 1

# Doc/README consistency gate (also the last step of the pre-commit hook).
# Needs target/release/cgg to exist — it shells out to `cgg --help`.
python3 scripts/docs-check.py

# Build + test the Python extension module (needs `uv`; network on first
# run). The pytest suite under crates/cgg-py/tests/ runs from HERE, not
# from `cargo test` — cgg-py sets `test = false`. Neither `cargo build`
# nor `cargo test` needs an interpreter.
./scripts/build-python.sh
./scripts/build-python.sh --wheel      # release wheel into dist/

# Build + test the Node module (needs node + npx; downloads @napi-rs/cli)
cd crates/cgg-node && npx --yes --package=@napi-rs/cli@3.8.5 -- napi build --platform --release
cd crates/cgg-node && node --test tests/*.test.js

# Reproduce the README benchmark table (clones repos into $CGG_BENCH_DIR, default /storage/cgg-test_repos)
./scripts/benchmark.sh

# Every gate this repo has, in order, before tagging a release.
# Never commits, tags or pushes — it prints the commands and stops.
./scripts/release.sh --purpose "what this release is for"

# Install the project pre-commit hook (test + release build + README regen)
./scripts/install-hooks.sh

# Bypass the pre-commit hook for a docs-only commit
CGG_SKIP_PRECOMMIT=1 git commit ...    # or: git commit --no-verify
```

No lint runs on commit, but `scripts/release.sh` gates on `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check`, so code that fails either blocks a release. `deny.toml` exists for `cargo-deny` license/advisory auditing; nothing runs it automatically.

## Architecture

Nine workspace crates. The core is a strict pipeline — data flows one direction, later crates depend on earlier ones, never the reverse — and the four front ends are translation layers over one entry point.

```text
cgg-walk  →  cgg-lang  →  cgg-resolve  →  cgg (query)  →  cgg-format
                                              ↑
                                          cgg-core (Graph, IDs, audit — shared types)

front ends (all four call cgg::analyze — one copy of the resolver ordering):
    cgg (bin)  →  cgg::analyze  →  cgg::emit   [the CLI]
    cgg-py     →  cgg::analyze               [the PyO3 module,  import cgg]
    cgg-ffi    →  cgg::analyze               [the C ABI,        libcgg.so]
    cgg-node   →  cgg::analyze               [the N-API module, require()]
```

- **cgg-core** — the substrate. `Graph`, callable/edge IDs, audit records, facts, framework rules, stdlib lookup tables, `cpu::default_jobs`. Every other crate depends on this; it depends on nothing internal.
- **cgg-walk** — file discovery. Honors `.gitignore` + a built-in deny list, classifies files (binary detection, symlink-chain guards), emits `WalkOutcome`.
- **cgg-lang** — language plugin layer. `LanguageDetector` (extension/shebang/header), `ParserPool` (tree-sitter parser caching), and `PluginRegistry::with_v1_plugins` (defined in `crates/cgg-lang/src/lib.rs`), which calls `plugins::register_all` — the 44 `reg.register(…)` lines in `crates/cgg-lang/src/plugins.rs` that wire in every `LanguagePlugin` impl under `crates/cgg-lang/src/plugins/`. `.ipynb` files are pre-processed in `cgg-lang::notebook::extract_python_source` before being handed to the Python plugin. Each plugin implements `extract` to pull callables + raw call sites out of a tree-sitter AST.
- **cgg-resolve** — links call sites to definitions. The order in `analyze_in_pool` (`crates/cgg/src/lib.rs`, phase comments `Phase 3` … `Phase 3f`) matters:
  1. `type_hints::build_return_type_map` + `propagate_types_with_returns` — infer variable types from params, locals (file-wide, conflict-aware), constructors, return types
  2. `intra_file::link_file` — scope/containment within a single file, with owner-qualified disambiguation of same-name candidates (`names::owner_from_qn`, handling `Self`/qualifier owners)
  3. `cross_file::resolve` — import chains, `#include` transitive closure (depth 8), pub-use chains, and an `(language, owner type, method)` index for typed-receiver method calls (replaces the old O(callables) suffix-scan). Steps are tried in order and the last two are pure additions, so they can only resolve what nothing earlier did: **step 6** maps a bare `Widget(3)` onto `Widget.__init__` (`constructor_names`), **step 7** maps `agent("x")` onto `type(agent).__call__` (`call_operator_names`) using the file's `local_types`. A missed owner-method lookup retries through the declared base chain (`resolve_via_bases`, depth 8, visited-guarded), which is what makes an *inherited* method resolve. Duck-typed fan-out is bounded by `--fanout-cap` (default 5) and narrowed twice before the cap applies: `Protocol`/`ABC`/`@abstractmethod` declarations are dropped when a concrete candidate survives, and candidates whose `signature_hint` cannot accept the call's keyword arguments are dropped when any candidate can. Both narrowings are one-sided — if they would empty the set, the original set stands. Exceeding the cap emits `UnresolvedReason::FanoutCapExceeded` with the count; a drop is never silent.
  4. `ffi::link_ffi` — PyO3 / wasm-bindgen / napi / JNI / P/Invoke / `extern "C"` cross-language edges
  5. `descriptor::link_descriptors` — `$ref`/shape edges for the Smithy/Protobuf/GraphQL/OpenAPI/AsyncAPI interface languages
  6. `frameworks::detect` — entry-point matching (routes, jobs, handlers) → `<framework-entry>` nodes. On by default; `--no-entry-nodes` suppresses it
  7. `dispatch::fanout` (opt-in, `--dynamic-dispatch`) — interface/trait declaration → implementation edges (`Via::Dynamic`), driven by `CallableNode::trait_impl_target`

  Between 5 and 6 the driver drops `Via::Reference` edges unless `--reference-edges`, reconciles the audit buckets against everything later resolvers bound, and then mints exit nodes (`synthesize_exit_nodes`). `names.rs` holds the shared `owner_from_qn` used by intra_file, cross_file and descriptor. Every edge carries a confidence level and resolver provenance, so downstream consumers can filter by quality.

  `stack_graphs_resolver.rs` still exists but is a **no-op stub** (46 lines) — the integration was removed in the tree-sitter 0.26 upgrade (upstream pins tree-sitter 0.24 / ABI 14). `--stack-graphs` is parsed and discarded (`options.rs:206`, destructured as `stack_graphs: _` so it never reaches `RunOptions`). Do not add code paths that assume it resolves anything.

  Steps 1, 2, 3 and 6 run under rayon — `par_iter_mut` for type propagation and `par_iter` for the intra-file link in `lib.rs`, plus `par_iter` inside `cross_file.rs` and `frameworks/mod.rs` — as do the per-file parse loop (`lib.rs`) and audit serialisation (`cgg-core/src/audit.rs`). Every one of them must produce output in input order — `par_iter().collect()` preserves it — because the graph, and therefore the whole output, is required to be identical at any `--jobs` value (`crates/cgg/tests/determinism.rs`, `crates/cgg/tests/lib_api.rs::output_does_not_depend_on_worker_count`).
- **cgg** (library + binary) — `lib.rs` (~2120 lines) is the pipeline: `pub fn analyze` builds a **per-call** rayon pool (`ThreadPoolBuilder … pool.install(…)`; nothing sets a global pool, because a global can only be set once and a second call would silently reuse the first's width) and the private `analyze_in_pool` orchestrates everything, returning a `RunOutcome` and performing **no I/O beyond reading source** — no writes, no stdout/stderr, no `process::exit`, enforced by `crates/cgg/tests/lib_api.rs::analyze_writes_nothing`. `main.rs` (104 lines) is a thin shim that owns only what belongs to an application: the global allocator, the tracing subscriber, the startup log line, and the exit code (3 for `--fail-on-dead` findings, 1 for errors). `options.rs` holds `RunOptions` (only what changes the graph; `From<&Cli>` destructures with no `..` rest, so a new flag fails to compile until it is routed). `outcome.rs` holds `RunOutcome` and `Emission` — the ordered transcript of everything a run writes, diagnostics *and* artifacts in one list because their relative order is observable when the graph goes to stdout and diagnostics to stderr. `emit.rs` is the only place any file descriptor is touched, which is why `cgg-py`, `cgg-ffi` and `cgg-node` need none of it. `cli.rs` parses flags; `query.rs` applies `--filter` + `-n`/`--hops` (BFS neighborhood / path extraction) and `--exclude-*`. `since.rs` resolves `--since <revspec>` by shelling out to `git diff` and intersecting changed line ranges with callable spans; the resulting qualified names are appended to `--filter` as `^name$` regexes before `apply_query` runs, anchored on the tree being analyzed rather than the process cwd. `synthesize_exit_nodes` mints the deduplicated external/stdlib leaf nodes for `--include-external`/`--include-stdlib` (after the audit-reconciliation prune, before `dedup_edges`). All four of `--include-external`/`--include-stdlib`/`--dynamic-dispatch`/`--reference-edges` are opt-in and never change the default graph. `--no-graph` and `--report-unreferenced` change what is *emitted*, not the graph, so the first lives on `Cli` only while the second is a `RunOptions` field (it replaces the artifact, like `--why-live`).
- **cgg-format** — terminal emitters: `mermaid.rs` (default), `json.rs`, `dot.rs`, `graphml.rs`. `Via` kinds tag the mermaid label slot — `dyn`/`ref`/`ext`/`std`/`desc`/`entry` — with per-kind edge styling in dot (`via_dot` in `dot.rs`).
- **cgg-ffi** — the C ABI (`libcgg.so` / `libcgg.a`, header hand-written at `crates/cgg-ffi/include/cgg.h`). **Seven** exported functions (`cgg_version`, `cgg_analyze`, `cgg_graph_render`, `cgg_graph_meta`, `cgg_graph_callable_count`, `cgg_graph_free`, `cgg_string_free`); a translation layer over `cgg::analyze` with **no analysis logic**. Options cross as a JSON document and results as strings, deliberately: that is what lets one shared library serve every language with an FFI without gaining an entry point per flag, so **the ABI does not change when cgg gains a feature** — do not add a function for a new option, add a `RunOptions` field and it is reachable for free. `cgg_analyze` returns an opaque handle rather than a rendered string so a caller pays for one analysis, not one per format. Every entry point wraps its body in `catch_unwind` because a Rust panic crossing into C is UB, and every pointer argument is null-checked. `publish = false`.
- **cgg-py** — the PyO3 extension module (`import cgg`). A translation layer over `cgg::analyze` with **no analysis logic**: if you find yourself adding graph logic here, it belongs in `cgg`. `publish = false`; the artifact is a wheel built by `scripts/build-python.sh`, not a crate. `crate-type = ["cdylib"]`, `test = false`. The workspace pins pyo3 with `abi3-py39` + `extension-module`, so pyo3 links no libpython and needs no interpreter at build time — `cargo build --workspace` and `cargo test --workspace` work with no Python installed, and that is worth keeping true. `crates/cgg-py/tests/test_analyze.py` asserts **structural** parity with the binary — identical callable and edge sets including provenance and confidence, identical file-path→blake3 map — under the default run and under an option matrix, with a non-vacuity assertion that at least one option changed the graph. It is a pytest suite; `cargo test` does not run it.
- **cgg-node** — the N-API module (`require("cgg-callgraphgenerator")`), built with `@napi-rs/cli`. A native module rather than a wrapper over the C ABI: npm needs a per-platform artifact either way, so the C ABI would buy nothing while costing an FFI dependency and a slower boundary. Same no-analysis-logic contract as cgg-py and cgg-ffi. `analyze()` is async — the pipeline runs on `spawn_blocking` so a server's event loop does not stall — with `analyzeSync()` for scripts, plus `version()` and `languages()`. Keywords are camelCase and, like Python, un-negate the CLI's double negative: `entryNodes: false` rather than `--no-entry-nodes`. `publish = false` for cargo; the artifact is an npm package. Tests: `crates/cgg-node/tests/analyze.test.js` (`node --test`), which also checks parity against `target/release/cgg`.

### Adding a new language

1. Add the `tree-sitter-<lang>` crate to `[workspace.dependencies]` in `Cargo.toml`. If no crate is compatible with the workspace `tree-sitter` version (e.g. it pins an ancient `tree-sitter` and the deprecated `language()` API, as `tree-sitter-smithy` does), vendor the generated `parser.c` under `crates/cgg-lang/vendor/<lang>/`, compile it in `crates/cgg-lang/build.rs`, and bind the raw `tree_sitter_<lang>()` C symbol via `tree_sitter_language::LanguageFn` (see `plugins/smithy.rs`).
2. Add a plugin module under `crates/cgg-lang/src/plugins/` implementing `LanguagePlugin` (id, extensions, shebangs, resolver_kind, ts_language, extract). After `&self`, `extract` takes `&ExtractCtx<'_>` — the per-run extraction switches (dead-code signals, user-supplied registrar verbs). These were process-globals through 0.5.0; they are threaded now so two analyses in one process cannot write each other's state. Do not reintroduce a global for them.
3. Register it with a `reg.register(Box::new(…))` line in `plugins::register_all` (`crates/cgg-lang/src/plugins.rs`). That function is what `PluginRegistry::with_v1_plugins` calls, and counting `register(` in that file is how `scripts/docs-check.py` derives the language count that README, the skills and `scripts/benchmark.sh` are checked against.
4. If the language has cross-file semantics, extend `cgg-resolve::cross_file` to handle its import form.
5. Add a benchmark entry in `scripts/benchmark.sh` and a matching row in `scripts/update-readme-stats.sh` `ENTRIES` — docs-check fails if the two disagree — then rerun to update README stats.
6. Bump the `assert_eq!(reg.all().len(), 44)` in `crates/cgg-lang/src/lib.rs::v1_registry_has_all_languages`.

### Audit / metrics

A run writes a sidecar audit document **only when it has an output file**: `--metrics FILE` wins, otherwise `-o P` produces `P.audit.json`, and a run whose graph goes to stdout writes no audit at all (`emit.rs::sidecar`). Shape is chosen with `--audit-format json|jsonl` (`json` = one pretty document, `jsonl` = one event per line). It records files discovered/analyzed/skipped with reasons, every extracted callable, every unresolved call site with failure reason, and per-phase timing. This is the primary debugging surface — when a call edge is missing, run with `-o` and the audit log is where you find out why.

### Corpus runs are time-bounded

Every script that iterates the benchmark corpus caps itself, and none of
them may hang:

| Variable | Default | Meaning |
| --- | --- | --- |
| `CGG_REPO_TIMEOUT` | `60` | wall seconds per repo |
| `CGG_TOTAL_BUDGET` | `1800` | wall seconds for the whole run (30 min) |

Applies to `scripts/benchmark.sh`, `scripts/perf-compare.sh` and
`scripts/framework-coverage.py` (whose per-repo cap was 900s, which over
a hundred repos is not a timeout).

A repo that trips the cap is **named in the output and excluded**, never
silently averaged in or waited on. The corpus is ~113 repos and almost
all finish in seconds, so a timeout means a cgg bug — chase it, do not
raise the cap.

The rule exists because it was learned expensively: `erlang-otp`
(177 MB, 3,851 `.erl` files) ran for **3h40m** inside an unguarded sweep
before anyone noticed, and the *released* binary hangs on it too.

### Threads and performance

`--jobs 0` (the default) means **half the physical cores**, clamped (`cgg_core::cpu::default_jobs`) — not one thread per logical CPU. Extraction is allocation-heavy, so SMT siblings contend rather than add; the binary uses mimalloc for the same reason.

Measure, never estimate. `scripts/perf-compare.sh` does paired A/B against a baseline binary and `scripts/release.sh` runs it; published numbers are taken at `--jobs 1`. A cautionary example is in the tree: 0.6.x was reported as a "+4–6.8% corpus-wide regression" against 0.5.0 on the strength of a 9-repo sample. Re-measured across 29 repositories it is **not** corpus-wide — median per-repo delta +0.0%, total +2.7%, 10 of 29 faster — and the effect traces to one repository (`c-jq`) whose file-size skew defeats the work split. See the `### Performance` blocks in CHANGELOG.md for the full numbers.

## Pre-commit hook behavior

`.githooks/pre-commit` (installed via `scripts/install-hooks.sh`, which sets `core.hooksPath`) runs, in order:

1. `cargo test --workspace --quiet`
2. `cargo build --release -p cgg --quiet`
3. Installs the freshly-built binary to `$CGG_INSTALL_DIR` (default `~/.local/bin/cgg`) so other tools on the system — agents, sibling repos — pick up the same code about to be committed. Set `CGG_INSTALL_DIR=""` to opt out.
4. Runs the freshly built `cgg` three times into `target/cgg-readme-graphs/`: over `crates/cgg-walk`, over three files of `crates/cgg-lang`, and over `crates/cgg` with the showcase filter. Then `scripts/update-readme-graphs.py --self-test`, then the same script patching **three** README blocks — `walk`, `lang`, and `raw:self` (verbatim, because the README presents it as the literal output of the command printed above it).
5. Re-runs `cgg ./crates --filter 'cgg::analyze_in_pool$' -n 1` and patches the self-analysis stat line (between `<!-- cgg:begin:self-stats -->` markers) via `scripts/update-readme-stats.py`, fed that run's stderr. Sub-millisecond timing variation is rounded to keep commits stable.
6. Runs `scripts/docs-check.py`.
7. Stages `README.md` if any of the patches changed it.

`CGG_SKIP_PRECOMMIT=1` skips the whole hook, as does `git commit --no-verify`.

### What docs-check.py actually checks

Twelve check functions, called from `main()` in `scripts/docs-check.py`. Its own docstring header still says "Seven checks" and is stale; the numbering in the body runs 0–10, and `check_framework_apps` is unnumbered. Any of them fails the commit:

- **Language counts** — `register(` calls in `plugins.rs` must equal README's `## Supported languages (N)` heading, the language-table row count, and `REPOS=( … )` in `scripts/benchmark.sh` (which may carry one extra row for the combined `xv6 (c+asm)` entry).
- **Benchmark-table coverage** — `REPOS` in `benchmark.sh` and `ENTRIES` in `update-readme-stats.sh` must name the same set of languages.
- **Framework apps** — every enumerating `RuleSpec` id in `cgg-core/src/frameworks/rules.rs` must be named by some `APPS` entry in `benchmark.sh` or listed in `APPS_UNVERIFIED`, and `APPS` must not name a rule that does not exist.
- **CLI flag freshness** — every flag in README's `## CLI` flag table must exist in `cgg --help`. The reverse is deliberately not checked; the table is curated.
- **CLI synopsis coverage** — the ```text usage block under `## CLI` must name every live flag and no dead one. Deprecated no-ops (`--stack-graphs`, `--no-update-check`) are exempt because their help text says "No effect".
- **Skill language count** — a skill saying "Supports N languages" must match the plugin count.
- **Skill inventory** — every `skills/*/SKILL.md` must be linked from README, and README's "N bundled skills" must match how many exist.
- **Attribute-capture count** — prose claiming "N plugins listed in Step 2" must match the plugins declaring `attributes: true`.
- **Self-analysis showcase (check 7)** — `.githooks/pre-commit`, `scripts/update-readme-stats.sh`, `README.md` and **this file** must all name the same `--filter`; `scripts/patch-readme-stats.py` must contain the bare callable name; and the committed `<!-- cgg:begin:self -->` block must span at least three crates.
- **Python keyword parity (check 8)** — every `RunOptions` field must be reachable as a `cgg-py` keyword argument or be listed in `PY_DEFERRED_OPTIONS` with a reason. Note this covers Python only: nothing yet checks `cgg-node` or `cgg-ffi` for the same drift.
- **Deliberate leaks (check 9)** — `Box::leak`, `.leak()` and `mem::forget` in the pipeline crates must be listed in `ALLOWED_LEAKS` with a reason. `analyze` is called in a loop by four front ends now, so "the process is about to exit" is no longer a justification. This check exists because `type_hints.rs` leaked ~161 bytes per call until 0.6.2.

If you intentionally edit the mermaid blocks or the self-stats line by hand, the hook will overwrite them — edit the generators (`scripts/update-readme-graphs.py`, `scripts/update-readme-stats.py`) or the underlying code instead.
