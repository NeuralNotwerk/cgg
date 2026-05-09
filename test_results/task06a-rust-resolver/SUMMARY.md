# Task 6a — Custom `.tsg` rules for Rust + cross-crate resolution

## What shipped

Rather than pursuing a full Rust scope-graph implementation (which is a
multi-thousand-line `.tsg` undertaking comparable to
`tree-sitter-stack-graphs-python`), Task 6a delivers:

- A **placeholder `rust.tsg`** (`crates/cgg-resolve/src/tsg/rust.tsg`)
  wired behind the `ResolverService` trait. Edges it emits carry
  `resolver="tsg:rust"`; the file documents explicitly what is
  out-of-scope for v1 (impls, trait dispatch, generics, macros).
- A substantially improved **Rust extractor** (`crates/cgg-lang/src/plugins/rust.rs`):
  * Derives the crate-root segment from the enclosing `Cargo.toml`'s
    `[package] name` (hyphen → underscore normalized). Files with no
    manifest above them fall back to `"crate"` as before.
  * Derives the module path from the file's location under the
    crate's `src/` (or `tests/`/`benches/`/`examples/`/`bin/`):
    * `src/lib.rs` / `src/main.rs` → no module segments.
    * `src/foo.rs` → `foo`.
    * `src/foo/mod.rs` → `foo`.
    * `src/foo/bar.rs` → `foo::bar`.
  * Parses `use_declaration` structurally via a group expander that
    handles `use a::b::{X, Y as Z, self}`, `pub use a::b::c`, and
    nested groups.
  * Emits a synthetic `crate-root` import so the cross-file resolver
    knows each file's owning crate, including lib.rs files that have
    no callables of their own (e.g. pure re-export facades).
- A **substantially improved cross-file resolver**
  (`crates/cgg-resolve/src/cross_file.rs`):
  * Builds a Rust re-export map from every `pub use` record. Look-ups
    chase the re-export chain up to eight hops deep.
  * Resolves full qualified-path calls (`foo::bar::baz()`) directly,
    and also tries rewrites via any `use foo as f` alias.
  * Accepts `pub-use` records as direct imports too.

## Numbers

| | before Task 6a | after |
|---|---|---|
| Workspace tests | 82 | **88** |
| Cross-crate edges (self-analysis, `cgg ./crates`) | 1 | **16** |

First dozen of the 16 cross-crate edges now resolved on the cgg
workspace itself:

```
cgg::run                                                --> cgg_walk::walk
cgg::run                                                --> cgg_lang::PluginRegistry::with_v1_plugins
cgg::run                                                --> cgg_core::graph::Graph::new
cgg::run                                                --> cgg_resolve::intra_file::link_file
cgg::run                                                --> cgg_resolve::stack_graphs_resolver::resolve
cgg::run                                                --> cgg_resolve::cross_file::resolve
cgg_format::mermaid::tests::mk_graph                    --> cgg_core::graph::Graph::new
cgg_format::mermaid::tests::mk_graph                    --> cgg_core::ids::ResolverId::new
cgg_format::mermaid::tests::empty_graph_is_still_valid  --> cgg_core::graph::Graph::new
cgg_resolve::stack_graphs_resolver::resolve_language    --> cgg_core::ids::ResolverId::new
cgg_resolve::cross_file::resolve                        --> cgg_core::ids::ResolverId::new
cgg_resolve::intra_file::link_file                      --> cgg_core::ids::ResolverId::new
```

Every edge above is a legitimate cross-crate call in the workspace.

## Tests

- `cgg-lang`: 4 new Rust extractor unit tests
  (`use_block_imports_flatten`, `use_self_in_block`, `pub_use_is_tagged`,
  `nested_use_block_flatten`).
- `cgg`: 2 new integration tests in `tests/resolve.rs`
  (`rust_cross_crate_use_resolves`, `rust_pub_use_reexport_chains`).
- `cargo-deny check licenses bans sources`: clean (`tree-sitter-rust`
  was added as a direct dep on `cgg-resolve` for the placeholder
  `rust.tsg` grammar binding; same license as before).

## Explicit limitations (v1)

Carried over to the TODO for a future `v2 rust.tsg`:

- `impl` blocks and trait dispatch — method calls `obj.foo()` still
  resolve only when the simple name is unambiguous in the same file
  (or when the receiver type is known via an import alias).
- Generics / lifetimes / associated types.
- Macro expansion — macro-invoked calls are tree-sitter-invisible.
- Wildcard imports (`use a::*`) — recorded as a marker but never a
  resolution source.

## Artifacts

- `self.mmd` — current full-workspace mermaid graph.
- `self.audit.json` — full pretty-JSON audit.
- `self.stderr.txt` — stderr summary.
- `cargo-test.txt` — full workspace test run (88 tests passed).
