# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`cgg` is a CLI that generates call graphs (mermaid by default; also json/dot/graphml) from source trees. It is offline, deterministic, single-binary — no language servers, no build artifacts required. Supports 39 languages via tree-sitter plugins, plus Jupyter notebooks (`.ipynb`) via a JSON cell extractor that feeds the Python plugin. Primary consumer of the output is coding agents reading mermaid in their context window.

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

# Run cgg against itself (sanity check)
./target/release/cgg ./crates -t mermaid --filter 'cgg::run$' -n 1

# Reproduce the README benchmark table (clones repos into $CGG_BENCH_DIR, default /storage/tmp)
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
```

- **cgg-core** — the substrate. `Graph`, callable/edge IDs, audit records, facts, stdlib lookup tables. Every other crate depends on this; it depends on nothing internal.
- **cgg-walk** — file discovery. Honors `.gitignore` + a built-in deny list, classifies files (binary detection, symlink-chain guards), emits `WalkOutcome`.
- **cgg-lang** — language plugin layer. `LanguageDetector` (extension/shebang/header), `ParserPool` (tree-sitter parser caching), and `PluginRegistry::with_v1_plugins` which wires in all 39 `LanguagePlugin` impls under `crates/cgg-lang/src/plugins/`. `.ipynb` files are pre-processed in `cgg-lang::notebook::extract_python_source` before being handed to the Python plugin. Each plugin implements `extract` to pull callables + raw call sites out of a tree-sitter AST.
- **cgg-resolve** — links call sites to definitions. The order in `cgg::run` matters:
  1. `type_hints::build_return_type_map` + `propagate_types_with_returns` — infer variable types from params, locals, constructors, return types
  2. `intra_file::link_file` — scope/containment within a single file
  3. `stack_graphs_resolver::resolve` (or `resolve_light` fallback on timeout, controlled by `--stack-graphs auto|on|off`) — precise name resolution where available
  4. `cross_file::resolve` — import chains, `#include` transitive closure (depth 8), pub-use chains
  5. `ffi::link_ffi` — PyO3 / wasm-bindgen / napi / JNI / P/Invoke / `extern "C"` cross-language edges
  Every edge carries a confidence level and resolver provenance, so downstream consumers can filter by quality.
- **cgg** (the binary) — `main.rs` → `run` orchestrates the pipeline; `cli.rs` parses flags; `query.rs` applies `--filter` + `-n` (BFS neighborhood / path extraction) and `--exclude-*`.
- **cgg-format** — terminal emitters: `mermaid.rs` (default), `json.rs`, `dot.rs`, `graphml.rs`.

### Adding a new language

1. Add the `tree-sitter-<lang>` crate to `[workspace.dependencies]` in `Cargo.toml`.
2. Add a plugin module under `crates/cgg-lang/src/plugins/` implementing `LanguagePlugin` (id, extensions, shebangs, resolver_kind, ts_language, extract).
3. Register it in `PluginRegistry::with_v1_plugins` (`crates/cgg-lang/src/plugins.rs`).
4. If the language has cross-file semantics, extend `cgg-resolve::cross_file` to handle its import form.
5. Add a benchmark entry in `scripts/benchmark.sh` and rerun to update README stats.

### Audit / metrics

Every run emits a sidecar `<output>.audit.json` (override with `--metrics FILE`, format `json`|`jsonl`) recording files discovered/analyzed/skipped with reasons, every extracted callable, every unresolved call site with failure reason, and per-phase timing. This is the primary debugging surface — when a call edge is missing, the audit log is where you find out why.

## Pre-commit hook behavior

`.githooks/pre-commit` (installed via `scripts/install-hooks.sh`) runs, in order:

1. `cargo test --workspace`
2. `cargo build --release -p cgg`
3. Regenerates the two mermaid blocks in `README.md` (between `<!-- cgg:begin:walk -->` / `cgg:begin:lang` markers) by running the freshly built `cgg` against `crates/cgg-walk` and a subset of `crates/cgg-lang`, via `scripts/update-readme-graphs.py`.
4. Re-runs `cgg ./crates --filter 'cgg::run$' -n 1` and patches the self-analysis stat line (between `<!-- cgg:begin:self-stats -->` markers) via `scripts/update-readme-stats.py`. Sub-millisecond timing variation is rounded to keep commits stable.
5. Runs `scripts/docs-check.py`, which fails the commit if (a) the plugin count, README "Supported languages (N)" heading, README language-table row count, and `scripts/benchmark.sh` `REPOS=()` count disagree, or (b) the README `## CLI` flag table names a flag that no longer exists in `cgg --help`. The reverse direction (every help flag must appear in README) is intentionally not enforced — the README's flag table is curated.
6. Stages `README.md` if any of the patches changed it.

If you intentionally edit the mermaid blocks or the self-stats line by hand, the hook will overwrite them — edit the generators (`scripts/update-readme-graphs.py`, `scripts/update-readme-stats.py`) or the underlying code instead.
