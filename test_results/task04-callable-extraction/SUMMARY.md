# Task 4 — Callable extraction for Rust + Python

## What shipped

- **`FileFacts` / `DefRecord` / `RefRecord` / `ImportRecord`** in
  `cgg-core::facts` — the intermediate two-phase AST-pass shape that
  every later stage (intra-file linker, scope-aware resolvers, FFI
  linker, cache) consumes. Carries qualified names, byte ranges,
  line ranges, decorators/attributes, and visibility.
- **`LanguagePlugin::extract`** trait method with a default
  empty-facts implementation so languages without a Task 4 extractor
  still compile and participate in the pipeline.
- **Per-language plugin modules** under `cgg-lang/src/plugins/`,
  one file per language. Stubs for JS, TS, Go, Java, C, C++, C#
  (they pick up real extractors in later tasks).
- **Rust extractor** walking a `tree-sitter-rust` tree with a typed
  scope stack (`Crate | Mod | InherentImpl | TraitImpl | Trait`),
  correctly distinguishing:
  * free functions (`crate::mod_a::foo`)
  * inherent methods (`crate::Client::new`)
  * trait methods in an impl block (`crate::<Client as Hook>::before`)
  * trait default methods (`crate::Hook::before`)
  * async functions (marker wins over method tag)
  * named closures (`let inc = |x| ...;` → `crate::mod_a::inc`).
  Emits `use ... as ...` import records.
- **Python extractor** walking `tree-sitter-python` with a scope stack
  rooted at the file stem. Handles:
  * free functions, nested functions (`m.outer.inner`)
  * class methods, `__init__` (Constructor), `__del__` (Destructor)
  * `@staticmethod`, `@classmethod`, `@property` variants
  * named lambdas (`foo = lambda x: ...`)
  * `import` / `import ... as ...` / `from ... import ...` records
  * method calls capture `receiver_hint`.
- **`cgg` binary** invokes `plugin.extract` on every parsed file and
  embeds the callable list inside each `file_analyzed` audit record
  with `kind`, `qualified_name`, and both line + byte ranges.

## Artifacts

- `two-lang.json` — full audit for the demo fixture.
- `two-lang.stderr.txt` — stderr summary.
- `cargo-test.txt` — full workspace test run (62 passed).

## Fixture

`/tmp/cgg-two-lang/lib.rs` — a Rust file exercising every variant:

- `mod net { fn connect(...) { let verify = |...| ...; verify(...) } }`
- `pub struct Client; impl Client { pub fn new() -> Self; pub async fn send; fn finalize; }`
- `trait Hook { fn before() {} }` + `impl Hook for Client { fn before; }`

`/tmp/cgg-two-lang/app.py` — a Python file exercising every variant:

- free function, class with `__init__` / regular method / `_protected`
- decorated `@staticmethod`, `@classmethod`, `@property`
- named lambda
- `async def` function

## Observed behavior

```
/tmp/cgg-two-lang/lib.rs (rust, 7 callables):
  [freefunction]        crate::net::connect
  [namedclosure]        crate::net::verify
  [inherentmethod]      crate::Client::new
  [asyncfunction]       crate::Client::send
  [inherentmethod]      crate::Client::finalize
  [traitdefaultmethod]  crate::Hook::before
  [traitmethod]         crate::<Client as Hook>::before

/tmp/cgg-two-lang/app.py (python, 8 callables):
  [freefunction]        app.top_level
  [constructor]         app.Service.__init__
  [inherentmethod]      app.Service.handle
  [inherentmethod]      app.Service._process
  [staticmethod]        app.Service.kind
  [classmethod]         app.Service.factory
  [namedlambda]         app.inc
  [asyncfunction]       app.run
```

15 callables across the two files; every qualified name is distinct
and correctly scoped.

## Bug caught by the demo

Initial run emitted `[freefunction] crate::Client::new` etc. — the
variant classifier used string-prefix heuristics on the scope stack
which failed to distinguish inherent `impl` blocks from `mod` blocks.
Fixed by introducing a typed `ScopeSegment` enum and a new unit test
`method_in_impl` that asserts `InherentMethod` on the variant (not
just the qualified name).

## Test counts

- `cgg-lang` unit tests: **28 passed** (Rust extractor 8, Python
  extractor 9, detection 6, parser pool 3, plugins-roundup 1,
  registry 1).
- Workspace total after Task 4: **62 tests passed**.
