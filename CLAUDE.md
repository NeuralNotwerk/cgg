# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`cgg` is a CLI that generates call graphs (mermaid by default; also json/dot/graphml) from source trees. It is offline, deterministic, single-binary — no language servers, no build artifacts required. The same pipeline is also a Rust library (`cgg::analyze`) and a Python extension module (`import cgg`, `crates/cgg-py`); the CLI remains a single static binary with no Python linkage, and none of the three front ends may fork the resolver ordering. Supports 44 languages via tree-sitter plugins (including the Smithy, Protobuf, GraphQL, OpenAPI/Swagger, and AsyncAPI interface/descriptor languages, whose shape graphs are mapped onto the call-graph model), plus Jupyter notebooks (`.ipynb`) via a JSON cell extractor that feeds the Python plugin. OpenAPI/AsyncAPI documents are YAML or JSON, both parsed with the YAML grammar and content-detected by their root `openapi:`/`swagger:`/`asyncapi:` key (see `cgg-lang::detect`), so ordinary `.yaml`/`.json` files are untouched. Primary consumer of the output is coding agents reading mermaid in their context window.

## Commands

```bash
# Build the CLI (workspace member `cgg`)
cargo build --release -p cgg

# Run the full test suite — this is what the pre-commit hook gates on
cargo test --workspace

# Run a single crate's tests
cargo test -p cgg-resolve

# Run a single test by name
cargo test -p cgg-lang detect_python_by_shebang

# Run cgg against itself (sanity check) — this is also the README showcase
# graph. `analyze_in_pool` is the pipeline body; `cgg::run` is a shim in
# main.rs and matches several test helpers of the same name.
./target/release/cgg ./crates -t mermaid --filter 'cgg::analyze_in_pool$' -n 1

# Build + test the Python extension module (needs `uv`; network on first run).
# NOT required for `cargo build`/`cargo test` — those never need an interpreter.
./scripts/build-python.sh

# Reproduce the README benchmark table (clones repos into $CGG_BENCH_DIR, default /storage/cgg-test_repos)
./scripts/benchmark.sh

# Install the project pre-commit hook (test + release build + README regen)
./scripts/install-hooks.sh

# Bypass the pre-commit hook for a docs-only commit
CGG_SKIP_PRECOMMIT=1 git commit ...    # or: git commit --no-verify
```

There is no separate lint step configured beyond `cargo`'s built-in checks; `deny.toml` exists for `cargo-deny` license/advisory auditing.

## Architecture

The workspace is a strict pipeline. Data flows one direction; later crates depend on earlier ones, never the reverse.

```text
cgg-walk  →  cgg-lang  →  cgg-resolve  →  cgg (query)  →  cgg-format
                                              ↑
                                          cgg-core (Graph, IDs, audit — shared types)

front ends (both call cgg::analyze — one copy of the resolver ordering):
    cgg (bin)  →  cgg::analyze  →  cgg::emit   [the CLI]
    cgg-py     →  cgg::analyze               [the PyO3 module]
```

- **cgg-core** — the substrate. `Graph`, callable/edge IDs, audit records, facts, stdlib lookup tables. Every other crate depends on this; it depends on nothing internal.
- **cgg-walk** — file discovery. Honors `.gitignore` + a built-in deny list, classifies files (binary detection, symlink-chain guards), emits `WalkOutcome`.
- **cgg-lang** — language plugin layer. `LanguageDetector` (extension/shebang/header), `ParserPool` (tree-sitter parser caching), and `PluginRegistry::with_v1_plugins` which wires in all 44 `LanguagePlugin` impls under `crates/cgg-lang/src/plugins/`. `.ipynb` files are pre-processed in `cgg-lang::notebook::extract_python_source` before being handed to the Python plugin. Each plugin implements `extract` to pull callables + raw call sites out of a tree-sitter AST.
- **cgg-resolve** — links call sites to definitions. The order in `cgg::analyze_in_pool` (`crates/cgg/src/lib.rs`) matters:
  1. `type_hints::build_return_type_map` + `propagate_types_with_returns` — infer variable types from params, locals (file-wide, conflict-aware), constructors, return types
  2. `intra_file::link_file` — scope/containment within a single file, with owner-qualified disambiguation of same-name candidates (`names::owner_from_qn`, handling `Self`/qualifier owners)
  3. `cross_file::resolve` — import chains, `#include` transitive closure (depth 8), pub-use chains, and an `(language, owner type, method)` index for typed-receiver method calls (replaces the old O(callables) suffix-scan)
  4. `ffi::link_ffi` — PyO3 / wasm-bindgen / napi / JNI / P/Invoke / `extern "C"` cross-language edges
  5. `descriptor::link` — `$ref`/shape edges for the Smithy/Protobuf/GraphQL/OpenAPI/AsyncAPI interface languages
  6. `frameworks` — entry-point matching (routes, jobs, handlers) → `<framework-entry>` nodes. On by default; `--no-entry-nodes` suppresses it
  7. `dispatch::fanout` (opt-in, `--dynamic-dispatch`) — interface/trait declaration → implementation edges (`Via::Dynamic`), driven by `CallableNode::trait_impl_target`
  `names.rs` holds the shared `owner_from_qn` used by intra_file and cross_file. Every edge carries a confidence level and resolver provenance, so downstream consumers can filter by quality.
  `stack_graphs_resolver.rs` still exists but is a **no-op stub** — the integration was removed in the tree-sitter 0.26 upgrade (upstream pins tree-sitter 0.24 / ABI 14). `--stack-graphs` is parsed and discarded (`options.rs`, destructured as `stack_graphs: _` so it never reaches `RunOptions`). Do not add code paths that assume it resolves anything.
  Steps 1-3 and 6 run under rayon (`par_iter`), as does the per-file parse loop and audit serialisation. Every one of them must produce output in input order — `par_iter().collect()` preserves it — because the graph, and therefore the whole output, is required to be identical at any `--jobs` value (`crates/cgg/tests/determinism.rs`).
- **cgg** (library + binary) — `lib.rs` is the pipeline: `analyze` builds a per-call rayon pool and `analyze_in_pool` orchestrates everything, returning a `RunOutcome` and performing **no I/O beyond reading source** — no writes, no stdout/stderr, no `process::exit`. `main.rs` (98 lines) is a thin shim that owns only what belongs to an application: the global allocator, the tracing subscriber, the startup log line, and the exit code. `options.rs` holds `RunOptions` (only what changes the graph; `From<&Cli>` destructures with no `..` rest, so a new flag fails to compile until it is routed). `outcome.rs` holds `RunOutcome` and `Emission` — the ordered transcript of everything a run writes, diagnostics *and* artifacts in one list because their relative order is observable when the graph goes to stdout and diagnostics to stderr. `emit.rs` is the only place any file descriptor is touched, which is why `crates/cgg-py` needs none of it. `cli.rs` parses flags; `query.rs` applies `--filter` + `-n` (BFS neighborhood / path extraction) and `--exclude-*`. `since.rs` resolves `--since <revspec>` by shelling out to `git diff` and intersecting changed line ranges with callable spans; the resulting qualified names are appended to `--filter` as `^name$` regexes before `apply_query` runs. `synthesize_exit_nodes` mints the deduplicated external/stdlib leaf nodes for `--include-external`/`--include-stdlib` (after the audit-reconciliation prune, before `dedup_edges`). All four of `--include-external`/`--include-stdlib`/`--dynamic-dispatch`/`--reference-edges` are opt-in and never change the default graph.
- **cgg-format** — terminal emitters: `mermaid.rs` (default), `json.rs`, `dot.rs`, `graphml.rs`. New `Via` kinds (`External`/`Stdlib`/`Reference`) tag as `ext`/`std`/`ref` (and `dyn` for `Dynamic`) in the mermaid label slot, with edge styling in dot.
- **cgg-py** — the PyO3 extension module (`import cgg`). A translation layer over `cgg::analyze` with **no analysis logic**: if you find yourself adding graph logic here, it belongs in `cgg`. `publish = false`; the artifact is a wheel built by `scripts/build-python.sh`, not a crate. `abi3-py39` + `extension-module` mean pyo3 links no libpython and needs no interpreter at build time, so `cargo build --workspace` and `cargo test --workspace` work with no Python installed — verified, and worth keeping true. `crates/cgg-py/tests/test_analyze.py` asserts byte-parity between the module's output and the binary's.

### Adding a new language

1. Add the `tree-sitter-<lang>` crate to `[workspace.dependencies]` in `Cargo.toml`. If no crate is compatible with the workspace `tree-sitter` version (e.g. it pins an ancient `tree-sitter` and the deprecated `language()` API, as `tree-sitter-smithy` does), vendor the generated `parser.c` under `crates/cgg-lang/vendor/<lang>/`, compile it in `crates/cgg-lang/build.rs`, and bind the raw `tree_sitter_<lang>()` C symbol via `tree_sitter_language::LanguageFn` (see `plugins/smithy.rs`).
2. Add a plugin module under `crates/cgg-lang/src/plugins/` implementing `LanguagePlugin` (id, extensions, shebangs, resolver_kind, ts_language, extract). `extract` takes `&ExtractCtx` as its first argument — the per-run extraction switches (dead-code signals, user-supplied registrar verbs). These were process-globals through 0.5.0; they are threaded now so two analyses in one process cannot write each other's state. Do not reintroduce a global for them.
3. Register it in `PluginRegistry::with_v1_plugins` (`crates/cgg-lang/src/plugins.rs`).
4. If the language has cross-file semantics, extend `cgg-resolve::cross_file` to handle its import form.
5. Add a benchmark entry in `scripts/benchmark.sh` and rerun to update README stats.

### Audit / metrics

Every run emits a sidecar `<output>.audit.json` (override with `--metrics FILE`, format `json`|`jsonl`) recording files discovered/analyzed/skipped with reasons, every extracted callable, every unresolved call site with failure reason, and per-phase timing. This is the primary debugging surface — when a call edge is missing, the audit log is where you find out why.

## Pre-commit hook behavior

`.githooks/pre-commit` (installed via `scripts/install-hooks.sh`) runs, in order:

1. `cargo test --workspace`
2. `cargo build --release -p cgg`
3. Installs the freshly-built binary to `$CGG_INSTALL_DIR` (default `~/.local/bin/cgg`) so other tools on the system — agents, sibling repos — pick up the same code about to be committed. Set `CGG_INSTALL_DIR=""` to opt out.
4. Regenerates the two mermaid blocks in `README.md` (between `<!-- cgg:begin:walk -->` / `cgg:begin:lang` markers) by running the freshly built `cgg` against `crates/cgg-walk` and a subset of `crates/cgg-lang`, via `scripts/update-readme-graphs.py`.
5. Re-runs `cgg ./crates --filter 'cgg::analyze_in_pool$' -n 1` and patches the self-analysis stat line (between `<!-- cgg:begin:self-stats -->` markers) via `scripts/update-readme-stats.py`. Sub-millisecond timing variation is rounded to keep commits stable.
6. Runs `scripts/docs-check.py`, which fails the commit if (a) the plugin count, README "Supported languages (N)" heading, README language-table row count, and `scripts/benchmark.sh` `REPOS=()` count disagree, or (b) the README `## CLI` flag table names a flag that no longer exists in `cgg --help`. The reverse direction (every help flag must appear in README) is intentionally not enforced — the README's flag table is curated.
7. Stages `README.md` if any of the patches changed it.

If you intentionally edit the mermaid blocks or the self-stats line by hand, the hook will overwrite them — edit the generators (`scripts/update-readme-graphs.py`, `scripts/update-readme-stats.py`) or the underlying code instead.
