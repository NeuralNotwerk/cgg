# cgg — Call Graph Generator

`cgg` is a single-binary, offline, multi-language call graph generator
written in Rust. It parses source code with tree-sitter and resolves
callable-to-callable edges to near-LSP quality without running any
language server.

Supported v1 languages: **Rust, Python, JavaScript, TypeScript, Go,
Java, C, C++, C#**.

## Quick start

```
cgg ./some/folder ./other/folder -o me-app.mmd -t mermaid
```

Optional filtering to N-hop neighborhoods or full entry-to-exit call
paths:

```
cgg ./src --filter 'process_order' -n 2
cgg ./src --filter 'glob:handle_*' -n 0 -t json -o paths.json
```

## Output formats

`-t mermaid | json | dot | graphml`. The internal graph is shared; each
formatter is a thin transform over the same IR.

## Audit / metrics

Every file considered, every file skipped, every callable extracted,
and every unresolved call is tracked in a structured audit log. For
`-t json`, the audit is embedded in the output. For any other format,
an audit sidecar (`<output>.audit.json`) is written. `--audit-format
jsonl` streams per-file audit events for SIEM ingestion.

## Scope

- Nodes: callables only (functions, methods, named lambdas, callable
  properties, default methods on objects).
- Edges: "A calls B" within a language or across a declared FFI boundary
  (both sides must be inside the scanned roots).
- Entry / exit detection: pure topology (in-degree 0 / out-degree 0).
  Emergent HTTP handlers and registered callbacks surface naturally.
- Cycles (recursion, mutual recursion) are preserved — never silently
  removed.

## Out of scope (v1)

- Additional top-20 languages (Ruby, PHP, Kotlin, Swift, Scala, Dart,
  Lua, Shell, HCL). Plugin trait makes these straightforward to add.
- Real-LSP implementation behind the `ResolverService` trait — seam
  exists; implementation deferred.
- Daemon / watch mode.
- Macro expansion for C / C++ (macro call sites are emitted as
  unresolved with an audit note).

## License

Apache-2.0 OR MIT (dual). All transitive dependencies use MIT,
Apache-2.0, BSD, ISC, Unlicense, CC0, or BlueOak — enforced by
`cargo-deny`.
