# Task 6b — Go and C# plugins + cross-file resolution

## What shipped

- **Go extractor** (`crates/cgg-lang/src/plugins/go.rs`, 415 lines)
  * Derives package root from the `package foo` clause; emits a
    synthetic `package-root` import record so downstream resolvers
    know the file's package namespace.
  * Extracts `function_declaration` as free functions and
    `method_declaration` as methods, folding the receiver type
    (with pointer-vs-value equivalence) into the qualified name
    (`pkg.T.Do`).
  * Parses `import_declaration` specs including aliased
    (`al "other/lib"`), dot-imports, and blank imports (`_ "..."`),
    stripping the quoted path and preserving any alias.
  * Tracks `call_expression` with bare `identifier` and
    `selector_expression` receivers as `RefRecord`s with
    `receiver_hint`.

- **C# extractor** (`crates/cgg-lang/src/plugins/csharp.rs`, 406 lines)
  * Tracks a scope stack of namespaces and types; qualified names
    join with `.` (e.g. `App.Core.Service.Run`).
  * Handles `namespace_declaration`,
    `file_scoped_namespace_declaration`, `class_declaration`,
    `struct_declaration`, `record_declaration`,
    `interface_declaration`.
  * Emits `method_declaration` / `local_function_statement` as
    inherent methods, `constructor_declaration` as Constructor,
    `destructor_declaration` as Destructor (simple name prefixed with
    `~` so it doesn't collide with the constructor), and
    `accessor_declaration` (get/set/init) as Property.
  * Parses `using X;`, `using Alias = X.Y;`, and
    `using static X.Y;` — the last stored with kind `using-static`.
  * Tracks `invocation_expression` with `identifier`,
    `member_access_expression`, and `generic_name` callees.

- **Cross-file resolver enrichment** (`crates/cgg-resolve/src/cross_file.rs`):
  * Python and Go `import` records now use the right "binding name"
    and target-root semantics per language — Python picks the first
    dotted segment and preserves the full path; Go uses the last
    slash-separated segment (package convention) both as the
    binding and the target.
  * C# `using` and `using-static` directives participate as
    module aliases and static-import sources.
  * Qualified-path call resolution now tries **both** `::` and `.`
    joiners when building direct lookups and alias rewrites, so
    `App.Lib.Helpers.Scale(...)` works the same way as
    `app::lib::helpers::scale(...)`.

- **Placeholder tsg files**: `src/tsg/go.tsg` and `src/tsg/csharp.tsg`
  wired through the stack-graphs resolver as `resolver="tsg:go"` and
  `resolver="tsg:csharp"` respectively. Full scope graphs for these
  languages remain future work; the cross-file resolver carries the
  resolution weight today.

## Demo

Fixture `/tmp/cgg-go-csharp/` (parallel Go and C# mini-packages):

- `golib/math.go` + `main.go`: Go main imports `example.com/golib`
  and calls `golib.Add` / `golib.Multiply` — cross-package.
- `cslib/Helpers.cs` + `main.cs`: C# runner calls
  `App.Lib.Helpers.Scale(3, 4)` — fully-qualified cross-file,
  across namespaces.

Observed mermaid:

```
flowchart LR
  C0["golib.Add"]
  C1["golib.Multiply"]
  C2["App.Runner.Go"]
  C3["App.Runner.Entry"]
  C4["App.Lib.Helpers.Add"]
  C5["App.Lib.Helpers.Scale"]
  C6["main.Run"]
  C7["main.Start"]
  C1 --> C0                  # intra-file: golib.Multiply -> Add
  C3 --> C2                  # intra-file: App.Runner.Entry -> Go
  C5 --> C4                  # intra-file: Scale -> Add
  C7 --> C6                  # intra-file: main.Start -> Run
  C2 --> C5                  # cross-file: App.Runner.Go -> App.Lib.Helpers.Scale
  C6 --> C0                  # cross-package: main.Run -> golib.Add
  C6 --> C1                  # cross-package: main.Run -> golib.Multiply
```

8 callables, 7 edges — 4 intra-file + 3 cross-file/cross-package.
3 unresolved calls (C# `new` / builtin constructors, Go builtin
loops). No false edges observed.

## Test counts

- `cgg-lang` Go plugin: 5 passed
  (`free_function`, `method_qualified_name_has_receiver_type`,
  `imports_captured`, `call_expressions_captured`,
  `package_root_marker_emitted`).
- `cgg-lang` C# plugin: 5 passed
  (`method_qualified_name_with_namespace_and_class`,
  `constructor_and_destructor_variants`,
  `using_directive_captured`,
  `invocation_references_extracted`,
  `nested_namespaces_join_dots`).
- `cgg` integration (`tests/resolve.rs`): 3 new
  (`go_cross_package_call_resolves`,
  `go_aliased_import_resolves`,
  `csharp_cross_file_namespace_call_resolves`).
- Workspace total after Task 6b: **101 tests passed** (+13 over Task 6a).
- `cargo-deny check licenses bans sources`: **ok**.

## Known limitations

Carried over from Task 6a style:

- Go interface dispatch and embedded-struct method promotion.
- C# partial classes merged only by name (not by overload signature).
- No handling of C# extension methods as alternative dispatch.
- `.tsg` rules are architectural placeholders; a serious
  `tree-sitter-stack-graphs-go` / `-csharp` would replace them.
