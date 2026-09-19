# Vendored Kivy (KV) tree-sitter grammar

`parser.c`, `scanner.c` and `tree_sitter/*.h` are vendored from
[`tree-sitter-kivy`](https://github.com/gibrilhamideh/tree-sitter-kivy)
v0.1.0 (grammar ABI 15, `tree-sitter-cli` 0.26.13), MIT licensed.
See `LICENSE`.

There is no crates.io package. Compiling the generated C here and binding
`tree_sitter_kivy()` through `tree_sitter_language::LanguageFn` matches the
Smithy vendor path, with an extra `scanner.c` for KV's indent/dedent tokens.

`grammar.js` is kept for reference/regeneration only; it is not compiled.

To refresh: clone the grammar, run `tree-sitter generate`, and copy
`src/parser.c`, `src/scanner.c` and `src/tree_sitter/` here.
