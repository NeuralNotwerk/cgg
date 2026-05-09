# Task 6 — ResolverService trait + stack-graphs integration

## What shipped

- **Ecosystem upgrade to tree-sitter 0.24.7**. All 9 grammar crates
  bumped to compatible ABI-14 versions so `tree-sitter-stack-graphs`
  0.10 and the language packs can co-exist:
  `tree-sitter-rust 0.23`, `tree-sitter-python =0.23.5`,
  `tree-sitter-javascript =0.23.1`, `tree-sitter-typescript =0.23.2`,
  `tree-sitter-java =0.23.4`, `tree-sitter-go 0.23`, `tree-sitter-c 0.23`,
  `tree-sitter-cpp 0.23`, `tree-sitter-c-sharp =0.23.1`.
  All plugins switched from the old `language()` call to the new
  `LANGUAGE.into()` pattern.
- **`ResolverService` trait** previously shipped in Task 5 is now
  backed by two concrete resolvers:
  1. **`cgg_resolve::stack_graphs_resolver`** — wraps
     `tree-sitter-stack-graphs-{python,javascript,typescript,java}`.
     Builds a single `StackGraph` per language, merges the language's
     built-in `.tsg` rules plus the per-language `builtins.py` /
     `builtins.js` scope graph, runs
     `find_minimal_partial_path_set_in_file` per file, then
     `ForwardPartialPathStitcher::find_all_complete_partial_paths`
     per call-site reference. Resolved paths are mapped back to
     `CallableId`s by byte-range containment. Edges emit with
     `resolver="stack-graphs:<lang>"` and `confidence=High` for
     single-candidate paths, `Low` for ambiguous.
  2. **`cgg_resolve::cross_file`** — a companion pass that walks
     each file's import table and emits cross-file edges for
     call sites that match an imported symbol (or an aliased module
     member). Edges emit with `resolver="cross-file:imports"` and
     `confidence=Medium`.
  The two resolvers are complementary: stack-graphs handles
  intra-package scope resolution; cross-file handles the
  import-binding indirection that stack-graphs returns for
  `from x import y` patterns.
- **Python plugin** now derives full dotted module names by walking
  up through `__init__.py` chains (`pkg/sub/file.py` → `pkg.sub.file`).
  This aligns qualified names with the forms that appear in
  `from pkg.sub.file import foo` imports.
- `cgg` binary runs the full pipeline:
  `walk → detect → parse → extract → build Graph → intra-file link →
   stack-graphs → cross-file → emit`.

## Artifacts

- `crossfile.mmd` — mermaid graph for the demo fixture.
- `crossfile.audit.json` — full pretty-JSON audit.
- `crossfile.stderr.txt` — stderr summary.
- `cargo-test.txt` — full workspace test run (82 tests passed).

## Demo

Fixture `/tmp/cgg-crossfile/` (three Python files + `__init__.py`):

- `pkg/math.py` — `add`, `multiply` (calls `add`), `scale` (calls
  `multiply`).
- `pkg/stats.py` — `from pkg.math import add; def total: acc = add(acc, x)`;
  `def mean: total(xs)`.
- `app.py` — `from pkg.math import scale, multiply`,
  `from pkg.stats import mean`, `import pkg.math as m`;
  `def process: scale(...); m.multiply(...); mean(...)`;
  `def entry: process(...)`.

Observed mermaid output:

```
flowchart LR
  C0["pkg.math.add"]
  C1["pkg.math.multiply"]
  C2["pkg.math.scale"]
  C3["pkg.stats.total"]
  C4["pkg.stats.mean"]
  C5["app.process"]
  C6["app.entry"]
  C1 --> C0      # math.multiply → math.add  (intra-file)
  C2 --> C1      # math.scale → math.multiply (intra-file)
  C4 --> C3      # stats.mean → stats.total (intra-file)
  C6 --> C5      # app.entry → app.process (intra-file)
  ... [intra-file edges duplicated from stack-graphs resolver]
  C3 --> C0      # stats.total → math.add   (cross-file via `from pkg.math import add`)
  C5 --> C2      # app.process → math.scale (cross-file)
  C5 --> C1      # app.process → math.multiply (cross-file via alias `m.multiply`)
  C5 --> C4      # app.process → stats.mean (cross-file)
```

Metrics:

- 7 callables across 3 files (+ 1 empty `__init__.py`).
- 12 edges total: **8 high-confidence** (intra-file), **4 medium-confidence** (cross-file).
- 14 unresolved calls (mostly Python builtins: `sum`, `range`, `len`).
- `by_language.python.edges = 4` counts intra-file only;
  stack-graphs and cross-file edges add on top in the graph totals.

## Known limitations

- **Stack-graphs resolves to local import bindings, not through to
  the actual definition in another file.** For `from helpers import greet`,
  the stack-graphs resolver returns the local binding in `main.py` at the
  import site; it does not chase through to `helpers.greet`'s body. The
  `cross-file:imports` companion resolver compensates by walking the
  import table directly. This pragmatic split keeps the stack-graphs
  scaffolding in place (so Task 6a/6b's custom `.tsg` rules can plug
  into the same pipeline) while delivering the cross-file edges the
  demo requires.
- **Intra-file edges are currently emitted twice** — once by the
  intra-file linker and once by stack-graphs (when it hits a local
  definition). Task 9 adds deduplication with a confidence-preferring
  policy (keep the highest-confidence duplicate).
- **Stack-graphs resolver walltime is ~1 second per language per run**
  even on a tiny fixture — dominated by `.tsg` rule compilation and
  per-file graph building. Task 11's cache will amortize.

## Test counts

- `cgg-resolve` unit tests: 6 intra-file + 3 cross-file = **9 passed**.
- `cgg` integration tests: `cli.rs` 5 + `walker.rs` 3 + `detect.rs` 5 +
  `link.rs` 5 + `resolve.rs` 3 = **21 passed**.
- Workspace total after Task 6: **82 tests passed**.
- `cargo-deny check licenses bans sources`: **ok**.
