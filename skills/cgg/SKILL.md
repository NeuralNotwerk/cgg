---
name: cgg
description: Use the `cgg` call-graph CLI to map function-level call relationships across a codebase and pull the result (as mermaid) into the working context. Trigger when the user asks "what calls X?", "what does X depend on?", "what would break if I change this?", needs to understand an unfamiliar module before editing it, scopes a refactor or rename, traces a bug across files, generates architecture diagrams, asks "is this still used?" / "what references this?" / "is anything here unused?", or works on a polyglot codebase with FFI boundaries (PyO3, wasm-bindgen, napi, JNI, P/Invoke, C ABI). Also trigger proactively before editing any non-trivial function in a large codebase to confirm caller/callee impact — running cgg first is faster and more reliable than grepping for usages. Supports 44 languages (plus Jupyter `.ipynb`) including Rust, Python, TS/JS, Go, Java, Kotlin, C/C++, C#, Swift, Ruby, PHP, Bash, PowerShell, Solidity, F#, Verilog/SV, VHDL, Assembly, CMake, Starlark/Bazel, Nix, and the Smithy/Protobuf/GraphQL/OpenAPI/AsyncAPI interface-definition languages (API model topology rendered as service→operation→message/type and operation→schema→schema edges; OpenAPI/AsyncAPI YAML or JSON content-detected). Outputs mermaid (default, agent-readable), plus json/dot/graphml.
---

# cgg — call graph for agents

`cgg` is an offline, language-agnostic CLI that turns source code into
a mermaid call graph in milliseconds. A typical library or service
finishes in well under a second (cgg's own `crates/`: 125 files
analyzed, ~140 ms); a large application takes a few seconds (NetBox:
1,273 analyzed files, ~3 s). A really big tree — the Flutter SDK, say
— runs for many minutes, so scope with a path argument rather than
pointing it at a monorepo root. Output is plain text designed for
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
user and stop — `cargo install` compiles 44 tree-sitter grammars and
shouldn't run without confirmation:

```bash
cargo install cgg --locked
# or, from inside a clone of the cgg repo:
cargo install --path crates/cgg --locked
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
- **`-n N`** is the **hop depth** around each filter match. Omitting
  `-n` gives the *whole* filtered graph, not one hop — always pass
  `-n 1` explicitly as your starting point. `-n 0` is special: it
  enumerates entry-to-exit paths *passing through* the matches — use
  it when the question is "what are all the ways this gets called?"
  rather than "what's near it?".

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

### Whole-project shape without blowing your context

```bash
cgg . --rollup 40k                   # fold only if it would exceed 40k tokens
cgg . --rollup-by module             # always cut at module level
cgg . --rollup-by file --rollup 20k  # file level at least, coarser if needed
```

`--filter` answers "what touches X?". `--rollup` answers the other
question — "what is the shape of this codebase?" — which a filter
cannot, because you do not yet know what to filter for. It replaces
each group of callables with one node: `type`, `module`, `file`,
`package`, `dir:N` or `language`.

Reach for it as the **first** command in an unfamiliar repository,
before you know any names to filter on. `cgg . --rollup-by package`
on a large monorepo is a handful of nodes and tells you which
components talk to which.

An arrow between two group nodes means *at least one* call exists
between them; the `Nx` label is how many call sites it stands for. A
count like `⟨14 fns, 24 internal⟩` on a node means 14 callables were
folded in and 24 calls between them were dropped rather than drawn as
a self-loop. **A rolled-up graph is not the call graph** — the diagram
says so in a comment header, and you cannot reason about an individual
function from it. Re-run with `--filter` on a group that looks
interesting.

### Slice one analysis many ways

```bash
cgg ./src -t json -o /tmp/graph.json          # analyze once (the slow part)
cgg --from-graph /tmp/graph.json --rollup-by module
cgg --from-graph /tmp/graph.json --filter 'submit' -n 2
```

Parsing dominates the wall clock and there is no cache, so a saved
`-t json` graph is how you ask five questions for the price of one.
On this repo: ~360 ms to analyze, ~70 ms per replay. `--filter`,
`-n`, `--exclude-*` and `--rollup` all apply to the loaded document
and give byte-identical output to running them against the tree.

Two things it cannot do, both of which cgg tells you rather than
leaving you to find out: a saved graph is the *post-query* graph, so
replaying a filtered document only narrows it further; and options
needing analysis facts the document lacks (`--dead-code`,
`--include-external`, `--dynamic-dispatch`, `--since`, `--lang`) are
refused with a reason, not ignored.

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

JSON output is the format for CI gates and drift detection. Top-level
keys are `callables`, `files`, `edges`, `unresolved`, `file_audits`,
`metrics`. Two shapes to get right before writing a jq expression:

- **`callables` is an object keyed by stringified id, not an array** —
  `.callables[]` iterates values, but `.callables[0]` is an error. Each
  value has `qualified_name`, `simple_name`, `kind`, `language`, `file`
  (the owning file's **id** — look it up as a key of `files`, which is
  also an object keyed by id; it is not an array offset),
  `start_line`/`end_line`, `signature_hint`, `visibility`.
- **Edges use `src`/`dst`, not `from`/`to`**, and carry `site_line`,
  `site_byte`, `confidence`, `via`, `resolver`.

```bash
# every callee of a given caller, by name
jq -r --arg fn 'cgg::analyze_in_pool' '
  (.callables | with_entries({key: .key, value: .value.qualified_name})) as $name
  | [ $name | to_entries[] | select(.value == $fn) | .key ] as $ids
  | .edges[] | select(.src as $s | $ids | index($s)) | $name[.dst]
' /tmp/graph.json | sort -u
```

Useful for "fail the build if module A starts calling into module B's
internals."

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
%% cgg: &lt;framework-entry&gt; nodes are SYNTHESIZED. No call to them exists
%% in your source; they represent control entering from a framework.
%% BEST EFFORT — see the coverage table for what cgg did and did not recognise.
flowchart LR
  Cq3rc7yk1ma["svc.list_users"]
  C1e0h9zwbxof["app.list_users"]
  C8tjm5nd42p["&lt;framework-entry&gt;::network::flask::route('/users') ⟨framework entry callback⟩"]
  C1e0h9zwbxof --> Cq3rc7yk1ma
  C8tjm5nd42p -->|entry| C1e0h9zwbxof
```

Note the mermaid escaping: the emitted label is `&lt;framework-entry&gt;`,
not `<framework-entry>`. Match on the *unescaped* form in `--filter`
(patterns run against qualified names, before escaping) but on the
escaped form if you grep cgg's own output.

These are **INFERRED, not observed** — nothing in the source says the
call happens; cgg says so itself in the `%%` banner above the graph.
Say so when relaying them. `--no-entry-nodes` opts out.

The kind is part of the name, which makes the security query expressible:

```bash
# Everything reachable from untrusted input, 3 hops out
cgg ./src --filter '<framework-entry>::network::' -n 3

# Drop framework noise from an ordinary graph
cgg ./src --exclude-partial '<framework-entry>::lifecycle::'
```

Kinds: `network` (the only one cgg treats as attack surface), `queue`,
`schedule`, `cli`, `ffi`, `lifecycle` (the default when no trust
boundary is asserted), `test`, and `public` (a callable the *language*
exposes to anyone with no framework involved — a Solidity
`public`/`external` function).

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

Under `--dead-code` the graph still owns stdout, so the report lands
next to it: a `.deadcode.txt` / `.deadcode.json` sidecar when you pass
`-o`, and stderr for the text report when you don't.
`--dead-code-format json` needs a destination (`-o` or
`--dead-code-report FILE`) — it will tell you so rather than writing
unparseable JSON into the run summary.

`--why-live` is the exception: its proof **replaces** the graph on
stdout, so a run with `--why-live -o out.mmd` writes the proof to
`out.mmd`, not mermaid. Don't pipe it somewhere expecting a diagram.
It looks like this:

```text
cgg::read_file
  LIVE — proof: 2 hop(s) from cgg::analyze [ExportedApi]
   └→ cgg::analyze_in_pool                         ./cgg/src/lib.rs:134  direct / High
   └→ cgg::read_file                               ./cgg/src/lib.rs:1608  direct / High
  weakest hop: High
```

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

Relay findings as *candidates*, never as facts. How much of any band is
genuinely dead is a manual-review question — cgg reports that it found no
caller, which is not the same as there being none. The report prints a
per-language capability table; a "no" column means cgg was guessing for
that language.

`--why-live` is often a better answer to "what calls X?" than
`--filter X -n 1`, because it prints the shortest proving path from a
*root* rather than a neighbourhood. A root is whatever cgg treats as
an entry: a framework entry point, an exported API, a declared root in
`cgg-deadcode.toml`. The bracketed tag on the proof line
(`[ExportedApi]` above) names which.

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
- **When you cannot filter, roll up.** `--rollup 40k` caps the output
  instead of narrowing it, which is the right move when you want the
  whole shape rather than one neighbourhood. It is safe to leave on: a
  graph already under budget comes back byte-identical.
- **The budget is an estimate**, `max(words x 2.5, bytes / 1.8)` — no
  tokenizer ships in the binary. The divisor is measured against cgg's
  own output (1.78-2.25 bytes/token) and set at the low end, so the
  estimate runs 10-25% *high*. Treat it as a bound with slack, not an
  exact count.

## Choosing hop depth

| Goal | `-n` |
| --- | --- |
| Immediate callers + callees of a function | `1` |
| Two-hop context for a refactor | `2` |
| All entry-to-exit paths through a function | `0` |
| Whole module structure (with broad filter) | `1` |
| Whole project graph | omit `-n` (full graph) — only for small projects |
| Whole project *shape*, any size | omit `-n`, add `--rollup 40k` or `--rollup-by module` |

Start at `-n 1`. If the result is too sparse, go to `2`. If too
dense, add `--exclude-*` rather than dropping back to `0` or `1` —
keeping the hop depth and trimming nodes usually gives a more useful
graph than reducing scope.

## Reading the output

A mermaid flowchart from cgg looks like this (real output of
`cgg ./crates --filter 'cgg::read_file$' -n 1` on cgg's own tree):

```text
flowchart LR
  Cqf3yb5yflr["cgg::analyze_in_pool"]
  Cykc0uwg6nu["cgg::read_file"]
  Cqf3yb5yflr --> Cykc0uwg6nu
```

Node ids (`Cqf3yb5yflr`) are a type prefix plus lowercase base36 digits,
derived by hashing the callable's identity — **not** a sequential index.
The same callable keeps the same id across runs of the same tree, so ids
are comparable between two runs: editing an unrelated file, or moving
code within a file, leaves an id alone. The path that feeds the hash is
relative to the analysis root, so the id does not change with how cgg
was invoked or where the tree is checked out. Two caveats before you rely on
that. Ids are not comparable across cgg *versions*. And where cgg
genuinely cannot tell two callables apart — same file, same qualified
name, same signature — it separates them by declaration order, so
removing one can hand its id to the other; that is rare, and overloads
with distinct signatures are not affected. Still quote the labels to
the user, not the ids.

Each node is a callable, labeled with its fully-qualified name. Each
edge is a *resolved* call site — cgg doesn't emit edges it can't
prove, so unresolved sites go to the audit sidecar instead of cluttering
the graph with guesses.

When the same caller calls the same callee at multiple distinct call
sites in the source, the mermaid and dot renderers collapse those
into a single arrow with a multiplicity label — e.g.
`Cqf3yb5yflr -->|18x| Cykc0uwg6nu` in mermaid, or
`nk6rdns31fh -> n1a7yv3q0ebt [label="18x"];` in dot. The bare arrow form
is used when the count is 1. When an edge also carries a `Via` tag the
label slot holds both, space-separated: `-->|std 9x|`, `-->|ref 10x|`.
JSON and GraphML still emit one edge per call site (with
`site_line`/`site_byte`) so programmatic consumers don't lose
call-frequency information — on the run above, 60 mermaid arrows
against 91 JSON/GraphML edges.

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

- Most projects finish in under a second (cgg's own `crates/`: 125
  files analyzed, ~140 ms); a large application takes a few seconds
  (NetBox: 2,637 files discovered, 1,273 analyzed, ~3.0 s). Don't
  pre-optimize; just run it. On a very large tree, narrow the *path*
  you point it at — that is the only lever that matters, and
  `--filter` is not it (filtering happens after the whole tree is
  parsed and resolved).
- `--jobs` defaults to `0`, which means **auto: half the machine's
  physical cores, capped at 8** and bounded by any cgroup quota — not
  every core. Raise it explicitly (`--jobs 32`) on a big tree if the
  machine has the cores; it helps on parse-bound trees and does
  roughly nothing on resolve-bound ones, so measure rather than
  assume. The graph is byte-identical at any thread count.
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
