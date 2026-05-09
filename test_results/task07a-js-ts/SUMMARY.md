# Task 7a — JavaScript and TypeScript callable extraction

## What shipped

- **JavaScript extractor** (`crates/cgg-lang/src/plugins/javascript.rs`,
  453 lines)
  * `function_declaration` / `generator_function_declaration` →
    `FreeFunction`.
  * `arrow_function` and `function_expression` in
    `variable_declarator` → named lambda (`FreeFunction`).
  * `method_definition` in `class_body`: constructor, get/set
    (Property), static (StaticMethod), regular (InherentMethod).
  * ESM imports: `import { x } from '...'` → `from-import`;
    `import * as ns from '...'` → `import` with alias;
    `import x from '...'` → `from-import` with `default`.
  * `call_expression` → `RefRecord` (bare `identifier` or
    `member_expression` with receiver_hint).

- **TypeScript extractor** (`crates/cgg-lang/src/plugins/typescript.rs`,
  118 lines)
  * Delegates to the shared `JsWalker` since tree-sitter-typescript
    produces identical node kinds for callables, imports, and calls
    (type annotations are simply ignored by the walker).
  * Uses `LANGUAGE_TSX` grammar (superset of TS + TSX).

- **Cross-file resolver enrichment**
  (`crates/cgg-resolve/src/cross_file.rs`)
  * `from-import` with relative paths (starting with `.` or `/`)
    now also tries the bare item name as a lookup candidate, since
    JS/TS definitions don't carry a module-path prefix.
  * Module-alias resolution (Step 2) now falls back to bare name
    lookup when the qualified-path form doesn't match — handles
    `import * as ns from './mod'; ns.fn()` where `fn` is defined
    without a module prefix.

## Demo

Fixture `/tmp/cgg-js-ts/` (mixed JS + TS project):

- `utils.js`: exports `helper`, `scale`, `double` (arrow fn).
- `main.js`: imports from utils, defines `run`, class `App` with
  constructor/start/create, and `entry`.
- `service.ts`: imports `helper`, defines class `Service` with
  constructor/run/create, and async `main`.

Result: 12 callables, 13 edges (3 cross-file: `run→helper`,
`run→scale`, `run→double`), 12 unresolved (instance method dispatch
on local variables, `new` expressions, stack-graphs overhead).

## Test counts

- `cgg-lang` JS plugin: 5 passed
  (`function_declarations`, `arrow_function_named`,
  `class_methods`, `esm_imports_captured`,
  `call_expressions_captured`).
- `cgg-lang` TS plugin: 5 passed
  (`typed_function_declaration`, `typed_arrow_function`,
  `class_with_types`, `import_from_captured`,
  `call_expressions`).
- `cgg` integration (`tests/resolve.rs`): 2 new
  (`js_esm_import_resolves`, `ts_namespace_import_resolves`).
- Workspace total after Task 7a: **125 tests passed** (+12 over
  Task 7).
- `cargo-deny check licenses bans sources`: **ok**.

## Known limitations

- Instance method dispatch (`s.run()`) requires type inference on
  local variables — not modeled in v1.
- `require()` (CJS) destructuring patterns are not yet parsed as
  imports (only ESM `import` statements are handled).
- `export default class` anonymous classes don't produce a named
  callable.
- Stack-graphs for JS/TS adds ~2s overhead per file due to the
  upstream `.tsg` compilation; Task 11's cache will amortize this.
