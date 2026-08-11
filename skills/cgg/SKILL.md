---
name: cgg
description: Use the `cgg` call-graph CLI to map function-level call relationships across a codebase and pull the result (as mermaid) into the working context. Trigger when the user asks "what calls X?", "what does X depend on?", "what would break if I change this?", needs to understand an unfamiliar module before editing it, scopes a refactor or rename, traces a bug across files, generates architecture diagrams, asks "is this still used?" / "what references this?" / "is anything here unused?", or works on a polyglot codebase with FFI boundaries (PyO3, wasm-bindgen, napi, JNI, P/Invoke, C ABI). Also trigger proactively before editing any non-trivial function in a large codebase to confirm caller/callee impact — running cgg first is faster and more reliable than grepping for usages. Supports 44 languages (plus Jupyter `.ipynb`) including Rust, Python, TS/JS, Go, Java, Kotlin, C/C++, C#, Swift, Ruby, PHP, Bash, PowerShell, Solidity, F#, Verilog/SV, VHDL, Assembly, CMake, Starlark/Bazel, Nix, and the Smithy/Protobuf/GraphQL/OpenAPI/AsyncAPI interface-definition languages (API model topology rendered as service→operation→message/type and operation→schema→schema edges; OpenAPI/AsyncAPI YAML or JSON content-detected). Outputs mermaid (default, agent-readable), plus json/dot/graphml.
---

# cgg — call graph for agents

`cgg` is an offline, language-agnostic CLI that turns source code into
a mermaid call graph in milliseconds. A typical library or service
finishes in well under a second; a large application (NetBox, ~1,300
analyzed files) takes a few seconds. Very large trees — hundreds of
thousands of callables — still take minutes, so scope with a path
argument rather than pointing it at a monorepo root. Output is plain
text designed for direct injection into a prompt, so use it to reason
about call relationships *before* you edit.

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

```bash
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
prevents combinatorial blowup. If the cap is reached, cgg says so on
stderr (`stopped at --max-paths N`) and records a `paths_truncated`
event in the audit sidecar — **read that line before reporting the
result as complete**, because a capped path set looks exactly like a
full one.

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

## Framework entry points (on by default)

Frameworks invoke user code by means that are not calls, so a route
handler would otherwise have in-degree zero — which is a claim
("nothing calls this") and a false one. cgg synthesizes a
`<framework-entry>` node for each recognised entry point:

```text
C0["<framework-entry>::network::flask::route('/users') ⟨framework entry callback⟩"]
C0 -->|entry| C1["svc.list_users"]
```

These are **INFERRED, not observed** — nothing in the source says the
call happens. Say so when relaying them. `--no-entry-nodes` opts out.

The kind is part of the name, which makes the security query expressible:

```bash
# Everything reachable from untrusted input, 3 hops out
cgg ./src --filter '<framework-entry>::network::' -n 3

# Drop framework noise from an ordinary graph
cgg ./src --exclude-partial '<framework-entry>::lifecycle::'
```

Kinds: `network` (attack surface), `queue`, `schedule`, `cli`, `ffi`,
`lifecycle`, `test`.

**Two things to relay honestly, every time:**

1. **Reachability is not data flow.** "Reachable from a `network` entry"
   means control can get there. It does *not* mean attacker-controlled
   data does — there is no taint tracking. Use it to bound where to
   look, never to conclude something is exploitable.
2. **Coverage is partial.** Every run prints a coverage table on stderr
   naming which frameworks were recognised and which were *seen and not
   enumerated*. Read it before reporting a count. "3 network entries" on
   an app whose framework is in the gap list is not "3 routes" — it is
   "3 that cgg could see". To cover a missing framework, use the
   `cgg-frameworks` skill.

## Finding unreferenced code

```bash
cgg ./src --dead-code                    # ranked report on stderr
cgg ./src --dead-code -o g.mmd           # ...also g.mmd.deadcode.txt
cgg ./src --why-live 'MyType::method$'   # why is this considered live?
```

The graph owns stdout, so the report lands next to it: a
`.deadcode.txt` / `.deadcode.json` sidecar when you pass `-o`, and
stderr for the text report when you don't. `--dead-code-format json`
needs a destination (`-o` or `--dead-code-report FILE`) — it will tell
you so rather than writing unparseable JSON into the run summary.

**BEST EFFORT — EVERY FINDING IS A HYPOTHESIS, NOT A FACT.** cgg reports
what it could not find a caller for, which is not the same as proving no
caller exists. Reflection, string-keyed dispatch, dynamic imports,
build-time codegen, conditional compilation and FFI consumers outside
the tree are all invisible to it. Every finding must be manually
reviewed against the source before it is acted on.

Recognised framework entry points are now marked live automatically, so
route handlers, jobs and lifecycle methods no longer produce findings —
*for the frameworks in the run's `recognised` list*. For anything under
`seen, no rules`, the old caveat still applies in full.

Relay findings as *candidates*, never as facts. On cgg's own source the
highest-confidence band is roughly 20-45% precise. The report prints a
per-language capability table; a "no" column means cgg was guessing for
that language.

`--why-live` is often a better answer to "what calls X?" than
`--filter X -n 1`, because it prints the shortest proving path from an
entry point rather than a neighbourhood.

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
| --- | --- |
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

```text
flowchart LR
  C213["cgg::analyze_in_pool"]
  C222["cgg::read_file"]
  C213 --> C222
```

Each node is a callable, labeled with its fully-qualified name. Each
edge is a *resolved* call site — cgg doesn't emit edges it can't
prove, so unresolved sites go to the audit sidecar instead of cluttering
the graph with guesses.

When the same caller calls the same callee at multiple distinct call
sites in the source, the mermaid and dot renderers collapse those
into a single arrow with a multiplicity label — e.g. `C213 -->|3x| C222`
in mermaid, or `n213 -> n222 [label="3x"];` in dot. The bare arrow form
is used when the count is 1. JSON and GraphML still emit one edge per
call site (with `site_line`/`site_byte`) so programmatic consumers
don't lose call-frequency information.

If an edge you expected is missing, check the audit sidecar. It is a
**JSON array of events**, not an object, so select the event first:

```bash
jq '.[] | select(.event=="file_analyzed") | .unresolved_calls[]?
    | select(.name | contains("foo"))' /tmp/cgg.mmd.audit.json
```

Each entry carries `name`, `site_line`, a structured `reason.stage`
(`no-candidate-in-file`, `ambiguous-in-file`, `no-candidate-cross-file`,
…) and the candidate counts behind it. Slice by stage to see whether the
resolver never found a candidate or found too many:

```bash
jq -r '.[] | select(.event=="file_analyzed") | .unresolved_calls[]?
       | .reason.stage' /tmp/cgg.mmd.audit.json | sort | uniq -c
```

Common reasons calls don't resolve:

- Dynamic dispatch through generics or trait objects
- Reflection / eval / dynamic require
- Languages that yield no cross-file edges at all — HCL, Verilog/SV and
  Assembly. (Verilog parses `` `include ``, but task/function calls are
  never captured, so nothing crosses a file either way.) Most other
  languages *do* have it (Bash `source`, Clojure `:require`, Elixir
  `alias`, Erlang `-include`, Fortran `use`, Julia `using`, …); see the
  project README's language table for the per-language matrix and the
  benchmark table for how much of it lands in practice.

This is a feature: cgg is honest about what it doesn't know rather
than emitting low-confidence edges.

## Output formats

| Format | When to use |
| -------- | ------------- |
| `mermaid` (default) | Agent context, markdown docs, PRs, anything human + AI need to read |
| `json` | CI gates, jq pipelines, custom tooling, drift detection |
| `dot` | Graphviz rendering for very large graphs |
| `graphml` | yEd, Gephi, or other graph-analysis tooling |

## Performance and limits

- Most projects finish in under a second; a large application takes a
  few seconds (NetBox: 1,273 files, ~6.8s). Don't pre-optimize; just
  run it. On a very large tree, narrow the *path* you point it at —
  that is the only lever that matters, and `--filter` is not it
  (filtering happens after the whole tree is parsed and resolved).
- cgg uses every core by default. `--jobs N` caps it; the graph is the
  same at any thread count.
- There is **no cache**, and no flag to control one. Every run
  re-parses from source, which is why a run is reproducible from the
  tree alone — and why a re-run costs the same as the first. If you
  need the same graph twice in a session, write it to a file with `-o`
  and read the file back rather than re-running.
- C/C++ macros are listed as callables but not expanded — no
  preprocessor simulation.
- Type inference is partial: it handles parameters, locals,
  constructors, and return types, but not full generic resolution.
- No watch mode — re-run when the code changes.
- For huge graphs, prefer `-t dot` + Graphviz over a single
  thousand-node mermaid diagram. Better still: narrow the filter.

## Three anti-patterns to avoid

1. **Running cgg without `--filter` on a large project and pasting
   the whole graph into context.** This wastes tokens and obscures
   the answer you wanted. Always start with a filter — even a broad
   one like `glob:Module::*` is better than nothing.
2. **Skipping cgg and grepping for function names instead.** Grep
   matches strings; cgg matches resolved calls. Grep will miss
   method dispatch and over-match on common names. If you find
   yourself piping grep through `wc -l` to "estimate impact", that's
   the moment to reach for cgg.
3. **Relaying a `--dead-code` finding as established fact.** cgg reports
   what it could not find a caller for, which is not the same as proving
   none exists. Open the file first, and say "cgg could not find a
   caller", not "this is dead".
