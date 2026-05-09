# Task 7 — C and C++ preprocessor-aware resolver

## What shipped

- **C extractor** (`crates/cgg-lang/src/plugins/c.rs`, 302 lines)
  * `function_definition` → `FreeFunction` callable.
  * `declaration` with `function_declarator` → prototype (also
    recorded as `FreeFunction`; dedup is Task 9).
  * `preproc_function_def` → macro callable with `attributes:
    ["macro"]`. This lets the intra-file linker resolve macro
    invocations (tree-sitter parses `SQUARE(x)` as a
    `call_expression`).
  * `preproc_include` with quoted path → `ImportRecord` kind
    `"include"`. System includes (`<stdio.h>`) are ignored.
  * `call_expression` → `RefRecord` (bare `identifier` or
    `field_expression` for `ptr->fn()`).

- **C++ extractor** (`crates/cgg-lang/src/plugins/cpp.rs`, 379 lines)
  * Namespace scope stack (`namespace_definition`).
  * Type scope stack (`class_specifier`, `struct_specifier`).
  * Methods inside class bodies: constructors (name == class name),
    destructors (`destructor_name` → `~ClassName`), regular methods
    (`field_identifier`).
  * Out-of-line definitions (`qualified_identifier` in declarator,
    e.g. `Calc::compute`).
  * `qualified_identifier` in call expressions splits on last `::`
    to produce `receiver_hint` + `name`.
  * `field_expression` for `obj.method()` / `ptr->method()`.
  * Same `preproc_function_def` and `preproc_include` handling as C.

- **Cross-file resolver: `#include` support**
  (`crates/cgg-resolve/src/cross_file.rs`)
  * New `"include"` import kind handler: resolves the quoted path
    relative to the includer's directory, finds the matching
    `FileFacts` in the index, and imports all its definitions as
    direct imports (available by simple name).
  * Transitive `#include` chasing up to depth 8 — if `a.c` includes
    `b.h` which includes `c.h`, definitions from `c.h` are visible
    in `a.c`.
  * Combined with the existing qualified-path lookup (tries both
    `::` and `.` joiners), this resolves `math::Calc::zero()` when
    the caller includes the header that declares it.

- **Placeholder `.tsg` files**: `src/tsg/c.tsg` and `src/tsg/cpp.tsg`
  wired through the stack-graphs resolver as `resolver="tsg:c"` and
  `resolver="tsg:cpp"`.

## Demo

Fixture `/tmp/cgg-c-cpp/` (mixed C + C++ project):

- `lib/math.h` + `lib/math.c`: C library with `add` and `multiply`.
- `main.c`: includes `lib/math.h`, calls `add`, `multiply`, and
  macro `SQUARE`.
- `lib/calc.hpp` + `lib/calc.cpp`: C++ namespace `math` with class
  `Calc` (constructor, destructor, `compute`, static `zero`) and
  free function `helper`.
- `app.cpp`: includes `lib/calc.hpp`, calls `math::Calc::zero()`,
  `math::helper()`.

Result: 16 callables, 8 edges (3 intra-file + 5 cross-file via
`#include`), 6 unresolved (instance method dispatch on local
variables — expected v1 limitation).

Cross-file edges observed:
- `start → math::Calc::zero` (qualified call through header)
- `start → math::helper` (qualified call through header)
- `run → add` (bare call, resolved via transitive include)
- `run → multiply` (bare call, resolved via transitive include)
- `multiply → add` (cross-file: math.c includes math.h which
  declares add; the definition in the same file also matches)

## Test counts

- `cgg-lang` C plugin: 5 passed
  (`function_definitions_extracted`, `prototype_recorded`,
  `include_directive_captured`, `call_expressions_captured`,
  `macro_call_looks_like_call`).
- `cgg-lang` C++ plugin: 5 passed
  (`namespace_qualified_names`, `class_method_and_constructor`,
  `qualified_call_expression`, `field_expression_call`,
  `include_directive_captured`).
- `cgg` integration (`tests/resolve.rs`): 2 new
  (`c_include_header_resolves`,
  `cpp_namespace_cross_file_resolves`).
- Workspace total after Task 7: **113 tests passed** (+12 over
  Task 6b).
- `cargo-deny check licenses bans sources`: **ok**.

## Known limitations

- Prototype + definition both emit a callable node (dedup in Task 9).
- Instance method dispatch (`obj.method()`) requires type inference
  on local variables — not modeled in v1. These appear as unresolved
  with `no-candidate-in-scope`.
- Macro expansion is not modeled — `#define` function-like macros
  are recorded as callables so their invocations resolve intra-file,
  but the macro body's calls are not analyzed.
- `#include` resolution is path-based only; no `-I` include-path
  search. System headers are ignored.
- Out-of-line method definitions (`Calc::compute`) use the
  `qualified_identifier` text as the simple name, which may produce
  slightly different qualified names than the in-class declaration.
  Task 9's dedup will merge these.
