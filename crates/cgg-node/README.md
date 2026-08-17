# cgg — Node.js bindings

Offline, deterministic call graphs for 44 languages, in-process.

> **Not on npm yet.** `npm install cgg-callgraphgenerator` does **not**
> work today — the name is reserved for this package but nothing has been
> published under it. Until it is, build the module from the repository:
>
> ```bash
> git clone https://github.com/NeuralNotwerk/cgg && cd cgg/crates/cgg-node
> npm install && npm run build      # writes index.js, index.d.ts, cgg.<platform>.node
> ```
>
> Then `require("./index.js")` — or `require("<path>/cgg-node")` — wherever
> the samples below say `require("cgg-callgraphgenerator")`. Everything
> else on this page is what that build gives you.
>
> The Rust CLI (`cargo install cgg`) and the Python module
> (`pip install cgg-callgraphgenerator`) are published and usable now.

```js
const cgg = require("cgg-callgraphgenerator");

const g = await cgg.analyze("./src");
console.log(g.toMermaid());
```

No network calls, no language servers, no build artifacts required. The
analysis is the same Rust pipeline the `cgg` command-line tool runs, in the
same order, so the two cannot disagree — the test suite compares this
module's output against the binary's on the same tree, across the option
surface, and fails if they diverge.

**The package name is `cgg-callgraphgenerator`; `cgg` on npm is an
unrelated project** — a wrapper for the ChampionGG API. Same name as on
PyPI, so the two bindings are findable by the same string.

## Usage

```js
const cgg = require("cgg-callgraphgenerator");

// Whole tree.
const g = await cgg.analyze("./src");

// A neighbourhood around what you care about.
const near = await cgg.analyze("./src", { filter: ["handleRequest$"], hops: 2 });

// Several roots, one graph.
const both = await cgg.analyze(["./api", "./worker"], { lang: ["typescript", "go"] });

g.toMermaid();   // -> string, byte-identical to `cgg -t mermaid`
g.toJson();      // -> string, `cgg.graph.v1`
g.toDot();
g.toGraphml();

g.callableCount;         // without materializing anything
g.callables;             // [{ id, qualifiedName, kind, language, file, startLine, … }]
g.edges;                 // [{ src, dst, siteLine, siteByte, confidence, via }]
g.files;                 // [{ id, path }] — match `callable.file` by id, not array offset
g.metrics;               // whole-run counters
g.notices;               // what the CLI would print to stderr
g.jobs;                  // worker threads actually used
```

`analyze` returns a promise and runs the pipeline on a background thread
(`tokio::task::spawn_blocking`, via napi's `tokio_rt`), so the event loop
stays responsive for the ~130 ms a tree the size of cgg's own `crates/`
costs. `analyzeSync` is there for scripts, where blocking is the point —
it blocks the loop for the whole analysis.

TypeScript definitions ship with the package and are generated from the
Rust, so they cannot drift from what the module actually exposes.

## Options

Every option that changes the graph is a key on the second argument, with
one rename from the CLI — `entryNodes: true` rather than
`--no-entry-nodes`. Same default; a keyword has no reason to be a double
negative.

```js
{
  filter, hops, maxPaths,
  excludePartial, excludeGlob, excludeRegex,
  lang, jobs, ignoreFile, since, roots,
  includeExternal, includeStdlib, dynamicDispatch, referenceEdges,
  entryNodes, includeTests,
  deadCode, deadCodeConfidence, ignoreNames, ignoreAttributes,
}
```

An out-of-range *value* is an error naming what was expected —
`deadCodeConfidence: "nope"` fails with `must be "high", "medium" or
"low"` rather than being quietly coerced.

**An unrecognised *key* is silently ignored**, though, because that is how
napi deserializes an options object: `{ hopz: 2 }` analyzes with defaults
and reports nothing. Check spelling against the list above, or use the
TypeScript definitions, which do catch it. (The C ABI rejects unknown keys
outright; this binding cannot yet.)

## Concurrency

`analyze` takes no locks and keeps no process-global state, so concurrent
calls do not interfere — each gets its own worker pool sized by `jobs`.

## Findings are hypotheses

`deadCode: true` reports callables nothing appears to call. It is **best
effort**: reflection, dynamic dispatch and framework magic are exactly what
a static tool cannot see, so every finding is something to check, not a
fact. `g.notices` carries the coverage disclosure, including which
frameworks cgg recognised and which it saw but could not enumerate.

## Not in this release

`--why-live` proofs, the `--write-roots` baseline and the audit event
stream are reachable from the Rust API but have no option key and no
`Graph` getter here. Use the CLI for those.

The framework-coverage table is *not* missing — as the section above says,
it arrives rendered as one of the strings in `g.notices`. What is missing
is a structured object; parse the notice or use `cgg --framework-coverage`
if you need fields.

Full documentation: <https://github.com/NeuralNotwerk/cgg>

Licensed Apache-2.0 OR MIT.
