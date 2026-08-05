---
name: cgg
description: find unused/dead code, is this function still called, what references this — Use the `cgg` call-graph CLI to map function-level call relationships across a codebase and pull the result (as mermaid) into the working context. Trigger when the user asks "what calls X?", "what does X depend on?", "what would break if I change this?", needs to understand an unfamiliar module before editing it, scopes a refactor or rename, traces a bug across files, generates architecture diagrams, or works on a polyglot codebase with FFI boundaries (PyO3, wasm-bindgen, napi, JNI, P/Invoke, C ABI). Also trigger proactively before editing any non-trivial function in a large codebase to confirm caller/callee impact — running cgg first is faster and more reliable than grepping for usages. Supports 44 languages (plus Jupyter `.ipynb`) including Rust, Python, TS/JS, Go, Java, Kotlin, C/C++, C#, Swift, Ruby, PHP, Bash, PowerShell, Solidity, F#, Verilog/SV, VHDL, Assembly, CMake, Starlark/Bazel, Nix, and the Smithy/Protobuf/GraphQL/OpenAPI/AsyncAPI interface-definition languages (API model topology rendered as service→operation→message/type and operation→schema→schema edges; OpenAPI/AsyncAPI YAML or JSON content-detected). Outputs mermaid (default, agent-readable), plus json/dot/graphml.
---

# cgg — call graph for agents

`cgg` is an offline, language-agnostic CLI that turns source code into
a mermaid call graph in milliseconds. Most projects finish in under a
second; the largest under a minute. Output is plain text designed for
direct injection into a prompt, so use it to reason about call
relationships *before* you edit.

## When to reach for cgg

- **Before editing a function in unfamiliar code.** Confirm the
  caller/callee set so a "small change" doesn't break invisible
  coupling. This is the highest-value use — grep finds string matches;
  cgg finds resolved calls.
- **Scoping a refactor or rename.** One filter answers "what would
  break if I change `parse_config`'s signature?" — including
  transitive paths.
- **Tracing a bug across files.** A stack trace shows what *did*
  happen on one run; a 1-hop neighborhood shows what *can* happen.
- **Onboarding to a module.** A filtered subgraph beats reading every
  file to understand structure.
- **Polyglot / FFI codebases.** cgg resolves across language
  boundaries (PyO3, wasm-bindgen, napi, JNI, P/Invoke, C ABI) where
  IDEs typically can't.

Skip cgg for trivial one-liner edits, single-file fixes where you've
already read the file, or when grep would obviously be enough (e.g.,
deleting a constant with three referenced sites).

## Check it's installed

Run `which cgg` once. If missing, surface the install command to the
user and stop — `cargo install` takes minutes and shouldn't run
without confirmation:

```bash
cargo install --git https://github.com/NeuralNotwerk/cgg
# or, from inside a clone of the cgg repo:
cargo install --path crates/cgg
```

If the user declines or doesn't have Rust, fall back to grep/Read.
Don't try to half-emulate cgg by hand.

## The mental model: two knobs

```
cgg <paths>   --filter <pattern>   -n <hops>   -t <format>   -o <file>
              ^^^^^^^^^^^^^^^^^^   ^^^^^^^^
              what to zoom in on   how far around it
```

Almost every useful invocation is a combination of these two:

- **`--filter`** is a **regex on fully-qualified names** like
  `auth::login`, `MyClass.foo`, or `pkg.module.fn`. Prefix with
  `glob:` for glob syntax. Repeatable — multiple filters OR together.
  This is what makes cgg precise rather than overwhelming.
- **`-n N`** is the **hop depth** around each filter match. `-n 1` is
  the right default. `-n 0` is special: it enumerates entry-to-exit
  paths *passing through* the matches — use it when the question is
  "what are all the ways this gets called?" rather than "what's
  near it?".

Default output is mermaid to stdout. For anything bigger than ~50
nodes, write to a file and Read it back rather than letting it flood
the shell:

```bash
cgg . --filter 'OrderService::submit' -n 2 -o /tmp/cgg.mmd
# then Read /tmp/cgg.mmd
```

## Recipes

### "What calls X?" / "What does X call?"

```bash
cgg . --filter 'process_order$' -n 1 -o /tmp/cgg.mmd
```

The `$` anchors the match so `process_order_v2` isn't pulled in. `-n
1` returns the BFS neighborhood — immediate callers *and* callees, in
one shot.

### "Show every path through X" (refactor scoping)

```bash
cgg . --filter 'parse_config' -n 0 --max-paths 100 -o /tmp/cgg.mmd
```

`-n 0` enumerates full entry-to-exit paths — the precise answer to
"what would break if I change this signature?". The `--max-paths` cap
prevents combinatorial blowup; truncation gets recorded in the audit
sidecar so you know if you missed anything.

### Module / namespace overview

```bash
cgg . --filter 'auth::' -n 1 -o /tmp/cgg.mmd        # Rust / C++ style
cgg . --filter 'glob:auth.*' -n 1 -o /tmp/cgg.mmd   # generic
```

Use this to orient before reading source. The graph reveals which
functions are entry points (no internal callers) versus internal
helpers — saving you from reading the whole module top-to-bottom.

### Trim noise with exclusions

```bash
cgg . --filter 'core::' -n 1 \
      --exclude-partial 'tests::' \
      --exclude-glob '*::debug::*' \
      -o /tmp/cgg.mmd
```

Exclusions apply *after* `--filter`. Reach for them when the result
is cluttered with logging, test helpers, trivial getters, or
generated code.

### Subgraph for prompt injection

```bash
cgg . --filter 'CheckoutFlow::run' -n 2 -o /tmp/checkout.mmd
```

Then load `/tmp/checkout.mmd` into context before reasoning about
edits to `CheckoutFlow::run`. Two hops surfaces non-obvious
dependencies without overwhelming the window.

### Programmatic consumption (CI, jq, custom tools)

```bash
cgg . -t json -o /tmp/graph.json
jq '.edges | length' /tmp/graph.json
```

JSON output is the format for CI gates and drift detection — nodes
carry qualified names, files, and kinds; edges carry confidence
levels and resolver provenance. Useful for "fail the build if module
A starts calling into module B's internals."

### Restrict to one language in a polyglot tree

```bash
cgg . --lang rust,python -o /tmp/cgg.mmd
```

Useful in monorepos when you only care about one slice.

### See more than the direct call graph (opt-in)

By default the graph shows resolved internal calls only. Four opt-in
flags add more, each tagged so you can tell them apart in the mermaid
edge labels:

```bash
cgg . --filter 'Service::handle' -n 1 --include-external -o /tmp/cgg.mmd
```

- `--include-external` / `--include-stdlib` — adds leaf "exit nodes" for
  third-party / stdlib calls (edges tagged `ext` / `std`). Answers
  "what does this touch outside the project?"
- `--dynamic-dispatch` — adds interface/trait declaration → impl fan-out
  edges (tagged `dyn`). Answers "what could a `dyn Trait` call reach?"
- `--reference-edges` — adds edges for functions passed by name as
  values (tagged `ref`), so registered handlers aren't invisible.

Leave them off when you want the clean, high-confidence call graph.

## Filter tips that save tokens

- **Anchor specifically.** `--filter 'foo$'` matches only callables
  ending in `foo`. Without the `$` you also pull in `foobar`,
  `do_foo_thing`, etc., and the graph balloons.
- **Use the namespace separator.** `--filter 'auth::login'` is far
  more precise than `--filter 'login'`. For Python/Java/JS use `\.`
  or glob syntax.
- **Combine filters.** `--filter foo --filter bar` includes both
  neighborhoods in a single run — cheaper than two runs.
- **Glob when escaping is annoying.** `--filter 'glob:OrderService::*'`
  beats wrestling with regex special chars.

## Choosing hop depth

| Goal | `-n` |
|------|------|
| Immediate callers + callees of a function | `1` |
| Two-hop context for a refactor | `2` |
| All entry-to-exit paths through a function | `0` |
| Whole module structure (with broad filter) | `1` |
| Whole project graph | omit `-n` (full graph) — only for small projects |

Start at `-n 1`. If the result is too sparse, go to `2`. If too
dense, add `--exclude-*` rather than dropping back to `0` or `1` —
keeping the hop depth and trimming nodes usually gives a more useful
graph than reducing scope.

## Reading the output

A mermaid flowchart from cgg looks like:

```
flowchart LR
  C87["cgg::run"]
  C90["cgg::read_file"]
  C87 --> C90
```

Each node is a callable, labeled with its fully-qualified name. Each
edge is a *resolved* call site — cgg doesn't emit edges it can't
prove, so unresolved sites go to the audit sidecar instead of cluttering
the graph with guesses.

When the same caller calls the same callee at multiple distinct call
sites in the source, the mermaid and dot renderers collapse those
into a single arrow with a multiplicity label — e.g. `C87 -->|3x| C90`
in mermaid, or `n87 -> n90 [label="3x"];` in dot. The bare arrow form
is used when the count is 1. JSON and GraphML still emit one edge per
call site (with `site_line`/`site_byte`) so programmatic consumers
don't lose call-frequency information.

If an edge you expected is missing, check the audit:

```bash
cat /tmp/cgg.mmd.audit.json | jq '.unresolved[] | select(.caller | contains("foo"))'
```

Common reasons calls don't resolve:
- Dynamic dispatch through generics or trait objects
- Reflection / eval / dynamic require
- Languages without cross-file resolution in cgg (Bash, HCL, Fortran,
  Julia, Erlang, Clojure, Phoenix/Elixir — see the project README's
  language table for the current matrix)

This is a feature: cgg is honest about what it doesn't know rather
than emitting low-confidence edges.

## Output formats

| Format | When to use |
|--------|-------------|
| `mermaid` (default) | Agent context, markdown docs, PRs, anything human + AI need to read |
| `json` | CI gates, jq pipelines, custom tooling, drift detection |
| `dot` | Graphviz rendering for very large graphs |
| `graphml` | yEd, Gephi, or other graph-analysis tooling |

## Performance and limits

- Most projects finish in under a second. Don't pre-optimize; just
  run it.
- The on-disk cache (`./.cgg-cache`) makes re-runs near-instant.
  Leave it on unless you're debugging cgg itself (`--no-cache`).
- C/C++ macros are listed as callables but not expanded — no
  preprocessor simulation.
- Type inference is partial: it handles parameters, locals,
  constructors, and return types, but not full generic resolution.
- No watch mode — re-run when the code changes.
- For huge graphs, prefer `-t dot` + Graphviz over a single
  thousand-node mermaid diagram. Better still: narrow the filter.

## Two anti-patterns to avoid

1. **Running cgg without `--filter` on a large project and pasting
   the whole graph into context.** This wastes tokens and obscures
   the answer you wanted. Always start with a filter — even a broad
   one like `glob:Module::*` is better than nothing.
2. **Skipping cgg and grepping for function names instead.** Grep
   matches strings; cgg matches resolved calls. Grep will miss
   method dispatch and over-match on common names. If you find
   yourself piping grep through `wc -l` to "estimate impact", that's
   the moment to reach for cgg.

## Finding unreferenced code

```bash
cgg ./src --dead-code                    # ranked report
cgg ./src --why-live 'MyType::method$'   # why is this considered live?
```

**BEST EFFORT — EVERY FINDING IS A HYPOTHESIS, NOT A FACT.** cgg reports
what it could not find a caller for, which is not the same as proving no
caller exists. Reflection, string-keyed dispatch, dynamic imports,
build-time codegen, conditional compilation, framework entry points and
FFI consumers outside the tree are all invisible to it. Every finding
must be manually reviewed against the source before it is acted on.

Relay findings as *candidates*, never as facts. On cgg's own source the
highest-confidence band is roughly 20-45% precise. The report prints a
per-language capability table; a "no" column means cgg was guessing for
that language.

`--why-live` is often a better answer to "what calls X?" than
`--filter X -n 1`, because it prints the shortest proving path from an
entry point rather than a neighbourhood.

### A third anti-pattern

Relaying a cgg dead-code finding as established fact. Open the file
first — and say "cgg could not find a caller", not "this is dead".
