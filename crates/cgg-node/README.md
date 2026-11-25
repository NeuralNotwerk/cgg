# cgg — Node.js bindings

Offline, deterministic call graphs for 46 languages, in-process.

```bash
npm install cgg-callgraphgenerator
```

The native addon comes as `optionalDependencies`, one package per platform
— `linux-x64-gnu`, `linux-arm64-gnu`, `darwin-x64`, `darwin-arm64`,
`win32-x64-msvc` — so an install pulls the root plus only the binary the
host needs, with no compiler. To build from a clone instead:

```bash
git clone https://github.com/NeuralNotwerk/cgg && cd cgg/crates/cgg-node
npm install && npm run build      # writes index.js, index.d.ts, cgg.<platform>.node
```

and `require("./index.js")` wherever the samples below say
`require("cgg-callgraphgenerator")`.

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
g.toMermaid("hash", true);   // `--node-ids hash --locations`
g.toJson();      // -> string, `cgg.graph.v2`
g.toDot();       // toDot(true) / toGraphml(true) add call-site locations
g.toGraphml();
g.externalsJson();   // `cgg.externals.v1` string, or null without `externals`
g.externalsText();   // the ranked text report, or null

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
stays responsive for the ~0.5 s a tree the size of cgg's own `crates/`
costs. `analyzeSync` is there for scripts, where blocking is the point —
it blocks the loop for the whole analysis.

TypeScript definitions ship with the package and are generated from the
Rust, so they cannot drift from what the module actually exposes.

## Options

Every option that changes the graph, apart from the two named under *Not
in this release*, is a key on the second argument, with three renames from
the CLI — `entryNodes: true` rather than `--no-entry-nodes`,
`testEntries: true` rather than `--no-test-entries`, and `allowDirs: […]`
for the repeatable `--allow-dir`. Same defaults; a keyword has no reason to
be a double negative.

```js
{
  filter, hops, maxPaths,
  excludePartial, excludeGlob, excludeRegex,
  lang, jobs, ignoreFile, allowDirs, since, roots,
  skipMinified, includeExternal, includeStdlib, dynamicDispatch, referenceEdges,
  guessUniqueNames, moduleScopeCalls, propertyEdges,
  entryNodes, testEntries, includeTests,
  deadCode, deadCodeConfidence, ignoreNames, ignoreAttributes,
  rollup, rollupBy, rollupFormat, nodeIds, locations,
  fromGraph, externals,
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

`analyze` shares no run state between calls, so concurrent calls do not
interfere — each gets its own worker pool sized by `jobs`.

## Findings are hypotheses

`deadCode: true` reports callables nothing appears to call. It is **best
effort**: reflection, dynamic dispatch and framework magic are exactly what
a static tool cannot see, so every finding is something to check, not a
fact. `g.notices` carries the coverage disclosure, including which
frameworks cgg recognised and which it saw but could not enumerate.

## Not in this release

`--why-live` proofs, the `--write-roots` baseline, the ranked dead-code
report (each callable's `unreferenced` band is here; the report with its
evidence is not), `--report-unreferenced`, `--fanout-cap` and the audit
event stream are reachable from the Rust API but have no option key and no
`Graph` getter here. Use the CLI for those.

The framework-coverage table is *not* missing — as the section above says,
it arrives rendered as one of the strings in `g.notices`. What is missing
is a structured object; parse the notice or use `cgg --framework-coverage`
if you need fields.

Full documentation: <https://github.com/NeuralNotwerk/cgg>

Licensed Apache-2.0 OR MIT.
