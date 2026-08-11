# cgg — Node.js bindings

Offline, deterministic call graphs for 44 languages, in-process.

```bash
npm install cgg-callgraphgenerator
```

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

**The package is `cgg-callgraphgenerator`; `cgg` on npm is an unrelated
project.** Same name as on PyPI, so the two bindings are findable by the
same string.

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
g.files;                 // paths, indexed by `callable.file`
g.metrics;               // whole-run counters
g.notices;               // what the CLI would print to stderr
g.jobs;                  // worker threads actually used
```

`analyze` returns a promise and runs the pipeline on libuv's thread pool,
so an event loop stays responsive for the ~100 ms+ a real tree costs.
`analyzeSync` is there for scripts, where blocking is the point.

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

An unknown value is an error naming what was expected, rather than being
quietly ignored.

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

`--why-live` proofs, the `--write-roots` baseline, the audit event stream
and the framework-coverage table are reachable from the Rust API but are
not exposed here yet. Use the CLI for those.

Full documentation: <https://github.com/NeuralNotwerk/cgg>

Licensed Apache-2.0 OR MIT.
