# cgg — Call Graph Generator

`cgg` generates call graphs from source code. Point it at a directory,
get a mermaid diagram. No language server, no build step, no
configuration — one binary, instant results.

The primary output is **mermaid flowcharts** — a format that coding
agents (Copilot, Claude, Cursor, Aider, etc.) can read directly in
context to understand how functions call each other across a codebase.
When an agent needs to know "what calls this function?" or "what does
this function depend on?", `cgg` answers in a format the agent already
understands.

## Why mermaid?

Mermaid diagrams are:
- **Readable by agents** — plain text, no binary formats, fits in a prompt
- **Renderable by humans** — GitHub, GitLab, VS Code, and every major
  markdown viewer renders them inline
- **Filterable** — `--filter` + `-n` lets you extract exactly the
  subgraph an agent needs for a specific task
- **Diffable** — text output means call graph changes show up in PRs

Other formats (JSON, DOT, GraphML) are available for toolchain
integration, but mermaid is the default because it works everywhere
with zero setup.

## Quick start

```bash
cargo install --path crates/cgg
cgg ./src -o graph.mmd
```

That's it. `graph.mmd` is a mermaid flowchart you can paste into any
markdown file, feed to an agent, or render in a viewer.

### Give an agent context about a module

```bash
# "Here's how the auth module works"
cgg ./src --filter 'auth::' -n 1 -o auth-graph.mmd
```

### Trace all paths through a function

```bash
# "Show me every call chain that passes through process_order"
cgg ./src --filter 'process_order' -n 0 -o paths.mmd
```

### Full project graph as structured JSON

```bash
cgg ./src -t json -o graph.json
```

## CLI

```
cgg <paths>... [-o FILE] [-t mermaid|json|dot|graphml]
              [--filter PATTERN]... [-n N] [--max-paths N]
              [--exclude-partial SUBSTRING]...
              [--exclude-glob PATTERN]...
              [--exclude-regex PATTERN]...
              [--stack-graphs auto|on|off]
              [--jobs N] [--lang rust,python,...]
              [--audit-format json|jsonl] [--metrics FILE]
              [-v|-vv|-q]
```

| Flag | Default | Description |
|------|---------|-------------|
| `-t` | mermaid | Output format: `mermaid`, `json`, `dot`, `graphml` |
| `-o` | stdout | Output file (use `-` for stdout) |
| `--filter` | (none) | Regex on qualified names; prefix `glob:` for glob |
| `-n` | -1 (full) | Hop depth around filter matches; `0` = full paths |
| `--exclude-partial` | (none) | Exclude nodes containing substring |
| `--exclude-glob` | (none) | Exclude nodes matching glob |
| `--exclude-regex` | (none) | Exclude nodes matching regex |
| `--stack-graphs` | auto | `auto`: 60s timeout + light fallback; `on`/`off` |
| `--jobs` | 0 (auto) | Rayon thread count for parallel extraction |
| `--lang` | (all) | Comma-separated language filter |
| `--metrics` | sidecar | Force audit output to a specific file |
| `--audit-format` | json | `json` (batched) or `jsonl` (streaming) |

## How it works

```
source files
    │
    ▼
┌─────────────────────────────────────────────────────────┐
│  cgg-walk      file discovery (.gitignore, deny-list)   │
├─────────────────────────────────────────────────────────┤
│  cgg-lang      tree-sitter parse → extract callables    │
│                21 language plugins                       │
├─────────────────────────────────────────────────────────┤
│  cgg-resolve   link calls to definitions                │
│                ├── type propagation (params, locals,     │
│                │   constructors, return types)           │
│                ├── intra-file (scope + containment)      │
│                ├── cross-file (imports, pub-use, #include)│
│                └── FFI (PyO3, wasm-bindgen, napi, JNI,  │
│                    P/Invoke, C ABI)                      │
├─────────────────────────────────────────────────────────┤
│  query engine  --filter + -n (BFS neighborhood / paths) │
├─────────────────────────────────────────────────────────┤
│  cgg-format    mermaid │ json │ dot │ graphml           │
└─────────────────────────────────────────────────────────┘
    │
    ▼
mermaid flowchart (or json/dot/graphml)
```

Every phase is offline and deterministic. No network calls, no
language servers, no build artifacts required.

## Agent integration patterns

### Inject call context into a prompt

```bash
# Generate the subgraph around the function the agent is about to modify
cgg ./src --filter 'OrderService::submit' -n 2 -o /tmp/context.mmd
# Then include /tmp/context.mmd in the agent's context window
```

### Pre-commit: detect unintended coupling

```bash
# In CI or a git hook — fail if a module gains unexpected cross-boundary calls
cgg ./src --filter 'internal::' -n 1 -t json | jq '.edges | length'
```

### Continuous documentation

```bash
# Regenerate architecture diagrams on every push
cgg ./src --filter 'main$|run$|handle' -n 1 -o docs/entry-points.mmd
```

### Scope a refactoring

```bash
# "What would break if I change this function's signature?"
cgg ./src --filter 'parse_config' -n 0 -t mermaid
# Shows every entry-to-exit path through parse_config
```

## Supported languages (21)

| Language | Cross-file resolution | Type inference | Notes |
|----------|----------------------|----------------|-------|
| Rust | pub-use chains, Cargo.toml crate names | params, `Foo::new()` | Module paths from src/ |
| Python | from-import, import-as | params, `Foo()` | `__init__.py` package walk |
| JavaScript | ESM import, CJS require() | params | exports.fn, defineGetter |
| TypeScript | ESM import | params | Delegates to JS walker |
| Go | package imports | params, `var T`, `New*()` | Interface methods, func literals |
| Java | import, import static | params, `Type var`, `new Foo()` | Local variable types |
| Kotlin | import, as alias | params, `val x: T`, `Foo()` | Class-as-constructor |
| C | `#include` transitive (depth 8) | — | Macros as callables |
| C++ | `#include` transitive | — | Templates, operators |
| C# | using, using static, alias | params, `Type var`, `new Foo()` | Accessors |
| Bash | `source ./file.sh` | — | Builtin filter |
| Ruby | require/require_relative | — | initialize → Constructor |
| PHP | require_once/include | — | — |
| Objective-C | #import | — | Message expressions |
| R | library(), source() | — | `<-` and `=` assignment |
| Swift | import Module | — | init → Constructor |
| Lua | require('mod') | — | Colon method syntax |
| Dart | import 'file.dart' | — | — |
| Scala | import pkg.Class | — | Object declarations |
| HCL | — | — | Block labels as definitions |
| Zig | @import("std") | — | — |

## Self-analysis

`cgg` run on its own source (685 callables, 978 edges, 165 cross-file, 100ms). This is the 1-hop neighborhood of `cgg::run` — every edge is a
real cross-crate function call:

```bash
cgg ./crates -t mermaid --filter 'cgg::run$' -n 1
```

```mermaid
flowchart LR
  C2["cgg_walk::walk"]
  C72["cgg::query::apply_query"]
  C73["cgg::query::apply_exclusions"]
  C85["cgg::main"]
  C87["cgg::run"]
  C88["cgg::langs_enabled"]
  C89["cgg::count_lines"]
  C90["cgg::read_file"]
  C91["cgg::variant_to_kind"]
  C92["cgg::dedup_edges"]
  C94["cgg::emit_graph"]
  C96["cgg::emit_audit"]
  C541["cgg_lang::PluginRegistry::with_v1_plugins"]
  C574["cgg_resolve::type_hints::build_return_type_map"]
  C575["cgg_resolve::type_hints::propagate_types_with_returns"]
  C591["cgg_resolve::ffi::link_ffi"]
  C612["cgg_resolve::stack_graphs_resolver::resolve"]
  C613["cgg_resolve::stack_graphs_resolver::resolve_light"]
  C614["cgg_resolve::stack_graphs_resolver::is_sg_language"]
  C615["cgg_resolve::cross_file::resolve"]
  C627["cgg_resolve::intra_file::link_file"]
  C676["cgg_core::graph::Graph::new"]
  C677["cgg_core::graph::Graph::add_callable"]
  C678["cgg_core::graph::Graph::add_file"]
  C85 --> C87
  C87 --> C88
  C87 --> C90
  C87 --> C89
  C87 --> C91
  C87 --> C92
  C87 --> C94
  C87 --> C96
  C72 --> C676
  C87 --> C2
  C87 --> C541
  C87 --> C676
  C87 --> C678
  C87 --> C677
  C87 --> C574
  C87 --> C575
  C87 --> C627
  C87 --> C612
  C87 --> C612
  C87 --> C614
  C87 --> C613
  C87 --> C615
  C87 --> C591
  C87 --> C72
  C87 --> C73
```

Focus on subsystems with `--filter`:

```bash
cgg ./crates/cgg-walk -t mermaid          # walker internals
cgg ./crates --filter 'cgg_resolve::' -n 1 -t mermaid  # resolution pipeline
```

<!-- cgg:begin:walk -->
```mermaid
flowchart LR
  C0["cgg_walk::WalkOutcome::is_empty"]
  C1["cgg_walk::<WalkConfig as Default>::default"]
  C2["cgg_walk::walk"]
  C3["cgg_walk::walk_one"]
  C4["cgg_walk::push_candidate"]
  C5["cgg_walk::is_symlink_chain"]
  C6["cgg_walk::classify_file"]
  C7["cgg_walk::is_binary"]
  C8["cgg_walk::builtin_reason"]
  C9["cgg_walk::extract_err_path"]
  C2 --> C3
  C3 --> C4
  C3 --> C5
  C3 --> C6
  C3 --> C8
  C3 --> C9
  C6 --> C7
  C9 --> C9
```
<!-- cgg:end:walk -->

<!-- cgg:begin:lang -->
```mermaid
flowchart LR
  C0["cgg_lang::detect::LanguageDetector<'r>::new"]
  C1["cgg_lang::detect::LanguageDetector<'r>::detect"]
  C2["cgg_lang::detect::LanguageDetector<'r>::match_ext"]
  C3["cgg_lang::detect::extension"]
  C4["cgg_lang::detect::read_shebang"]
  C5["cgg_lang::detect::header_verdict"]
  C14["cgg_lang::parser::ParserPool<'r>::new"]
  C15["cgg_lang::parser::ParserPool<'r>::parse"]
  C16["cgg_lang::parser::ParserPool<'r>::plugin"]
  C17["cgg_lang::parser::set_language"]
  C21["cgg_lang::<ResolverKind as fmt::Display>::fmt"]
  C22["cgg_lang::LanguagePlugin::id"]
  C23["cgg_lang::LanguagePlugin::extensions"]
  C24["cgg_lang::LanguagePlugin::shebangs"]
  C25["cgg_lang::LanguagePlugin::resolver_kind"]
  C26["cgg_lang::LanguagePlugin::ts_language"]
  C27["cgg_lang::LanguagePlugin::extract"]
  C28["cgg_lang::PluginRegistry::new"]
  C29["cgg_lang::PluginRegistry::register"]
  C30["cgg_lang::PluginRegistry::all"]
  C31["cgg_lang::PluginRegistry::by_id"]
  C32["cgg_lang::PluginRegistry::with_v1_plugins"]
  C1 --> C2
  C1 --> C22
  C1 --> C24
  C1 --> C3
  C1 --> C4
  C1 --> C5
  C15 --> C15
  C15 --> C17
  C15 --> C26
  C15 --> C31
  C16 --> C31
  C2 --> C22
  C2 --> C23
  C27 --> C22
  C31 --> C22
  C32 --> C28
```
<!-- cgg:end:lang -->

## Output formats

| Format | Use case |
|--------|----------|
| **mermaid** (default) | Agent context, markdown docs, PR descriptions |
| **json** | Programmatic consumption, custom tooling, CI checks |
| **dot** | Graphviz rendering for large graphs |
| **graphml** | Import into yEd, Gephi, or other graph analysis tools |

## Resolution pipeline

`cgg` doesn't just find function definitions — it resolves which
function each call site actually targets:

1. **Type propagation** — infer variable types from parameters, local
   declarations, constructors (`Foo::new()`, `new Foo()`), and return
   types
2. **Intra-file linking** — scope-based, smallest-enclosing-range
   containment with receiver-hint narrowing
3. **Cross-file resolution** — walk import chains, `#include`
   transitive closure (depth 8), pub-use re-export chains
4. **FFI linking** — detect `#[pyfunction]`, `#[wasm_bindgen]`,
   `#[napi]`, `@JNI`, `[DllImport]`, `extern "C"` and link across
   language boundaries

Edges carry confidence levels and resolver provenance so downstream
tools can filter by quality.

## Audit / metrics

Every run produces a structured audit trail:
- Files discovered, analyzed, and skipped (with reasons)
- Every callable extracted
- Every unresolved call site (with failure reason)
- Timing per phase

Written as a sidecar (`<output>.audit.json`) or forced to a path with
`--metrics FILE`. Use `--audit-format jsonl` for streaming/SIEM
integration.

## Benchmark

Run `./scripts/benchmark.sh` to reproduce on real-world projects:

| Project | Language | Callables | Edges | Cross-file | Time |
|---------|----------|-----------|-------|------------|------|
| ripgrep | rust | 2,766 | 4,041 | 54% | 469ms |
| flask | python | 388 | 234 | 30% | 52ms |
| express | javascript | 92 | 59 | 20% | 21ms |
| zod | typescript | 1,675 | 2,410 | 65% | 225ms |
| fzf | go | 1,048 | 4,785 | 47% | 154ms |
| gson | java | 943 | 1,354 | 54% | 61ms |
| okio | kotlin | 3,673 | 5,484 | 72% | 345ms |
| jq | c | 1,073 | 20,819 | 93% | 112ms |
| nlohmann/json | cpp | 1,122 | 2,244 | 58% | 111ms |
| serilog | csharp | 828 | 432 | 68% | 70ms |
| acme.sh | bash | 1,433 | 3,904 | 0% | 156ms |
| jekyll | ruby | 902 | 1,237 | 63% | 72ms |
| laravel | php | 13,464 | 253 | 0% | 1585ms |
| AFNetworking | objc | 299 | 113 | 7% | 53ms |
| ggplot2 | r | 946 | 419 | 3% | 104ms |
| Alamofire | swift | 829 | 998 | 63% | 82ms |
| kong | lua | 2,782 | 0 | — | 203ms |
| flame | dart | 1,647 | 0 | — | 88ms |
| play | scala | 1,989 | 487 | 0% | 217ms |
| terraform-vpc | hcl | 1,779 | 0 | — | 81ms |
| http.zig | zig | 451 | 832 | 52% | 87ms |

## Limitations

- No macro expansion for C/C++ (preprocessor defines are not followed)
- No full type inference (trait dispatch, generics, dynamic typing)
- No daemon / watch mode
- Additional languages not yet supported: Elixir, Haskell, OCaml, Perl, Groovy

## License

Apache-2.0 OR MIT (dual). All transitive dependencies use MIT,
Apache-2.0, BSD, ISC, Unlicense, CC0, or BlueOak — enforced by
`cargo-deny`.
