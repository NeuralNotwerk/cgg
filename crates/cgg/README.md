# cgg — call graph generator

Point it at a directory, get a mermaid diagram. No language server, no build
step, no configuration — one binary, instant results.

```bash
cargo install cgg
cgg ./src -o graph.mmd
```

Offline, deterministic, single-binary. **44 languages** via tree-sitter —
the Smithy, Protobuf, GraphQL, OpenAPI and AsyncAPI descriptor languages
among them — plus Jupyter notebooks, which route through the Python plugin.
It makes no network calls, ever.

The primary output is a **mermaid flowchart**, because the primary consumer is
a coding agent reading it in a context window. JSON, DOT and GraphML are there
for toolchain integration.

```bash
# "Here's how the auth module works"
cgg ./src --filter 'auth::' -n 1 -o auth.mmd

# Every call chain that passes through one function
cgg ./src --filter 'process_order' -n 0 -o paths.mmd

# The call surface of a PR
cgg ./src --since main..HEAD -n 0 -o pr-surface.mmd
```

## As a library

```rust
use cgg::{RunOptions, analyze};

let outcome = analyze(&RunOptions {
    paths: vec!["./src".into()],
    ..Default::default()
})?;

println!("{} callables", outcome.graph.callables.len());
println!("{}", cgg::emit::graph_to_string(&outcome.graph, cgg::OutputFormat::Mermaid));
```

`analyze` performs **no I/O beyond reading the source tree** — no writes, no
stdout, no stderr, no `process::exit`. (`RunOptions::since` is the single
exception: resolving a revspec shells out to `git diff`.) Everything a run
produces comes back on `RunOutcome`, including an ordered transcript of every
diagnostic and artifact.
`cgg::emit` is the CLI's own front end over that value.

It takes no locks and keeps no process-global state, so it is safe to call
concurrently; each call gets its own worker pool sized by `RunOptions::jobs`.

> The library API is **pre-1.0**: `RunOptions` gains a field whenever a
> graph-affecting flag is added. Pin an exact minor if you depend on it.

## Other languages

The same pipeline is also a Python module (`import cgg`) and a C ABI
(`libcgg.so` / `libcgg.a`) that serves .NET, Java, Go, Ruby and anything else
with an FFI. Both live in the [repository](https://github.com/NeuralNotwerk/cgg).

## Findings are hypotheses

`--dead-code` reports callables nothing appears to call. It is **best effort**:
reflection, dynamic dispatch and framework magic are exactly the things a
static tool cannot see, so every finding is a hypothesis to check, not a fact.
cgg discloses which frameworks it recognised and which it saw but could not
enumerate, because a partial list that reads as complete is worse than no list.

Full documentation, the language table, benchmarks and the audit format:
**<https://github.com/NeuralNotwerk/cgg>**

Licensed Apache-2.0 OR MIT.
