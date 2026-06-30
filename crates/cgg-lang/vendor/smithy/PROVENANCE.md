# Vendored Smithy tree-sitter grammar

`parser.c` and `tree_sitter/parser.h` are vendored verbatim from the
[`tree-sitter-smithy`](https://github.com/tree-sitter/tree-sitter-smithy)
crate, version `0.0.1` (grammar ABI 14), which is licensed MIT.

We vendor the generated parser rather than depend on the crate because
`tree-sitter-smithy 0.0.1` pins an old `tree-sitter 0.20` and exposes the
deprecated `language() -> tree_sitter::Language` API, which is incompatible
with the `tree-sitter 0.26` used across this workspace. Compiling `parser.c`
directly (via `build.rs`) and binding the raw `tree_sitter_smithy()` C symbol
through `tree_sitter_language::LanguageFn` sidesteps the stale dependency while
keeping cgg a single self-contained binary.

`grammar.js` is kept for reference/regeneration only; it is not compiled.

To refresh: re-generate with `tree-sitter generate` against an updated grammar
and copy the resulting `src/parser.c` + `src/tree_sitter/parser.h` here.
