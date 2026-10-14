# Vendored Lean 4 tree-sitter grammar

`parser.c`, `scanner.c` and `tree_sitter/*.h` are vendored from
[`tree-sitter-lean`](https://github.com/wvhulle/tree-sitter-lean) at
revision `79a84684a2ceaad707c8cac8dc60b56799836150` of Nathan Howell's
fork (<https://github.com/NathanHowell/tree-sitter-lean>), which rebinds
the crate through `tree-sitter-language`; the grammar sources are
unchanged from upstream. MIT licensed (see `LICENSE`).

The published crate, `tree-sitter-lean4`, depends on the `tree-sitter`
core crate, which carries `links = "tree-sitter"` and so cannot coexist
with the workspace runtime. Patching it to a git fork builds locally but
does not survive publishing — `[patch]` is dropped from a packaged crate —
so it is vendored like Smithy and Kivy, and `tree_sitter_lean()` is bound
through `tree_sitter_language::LanguageFn` in `plugins/lean.rs`.

`grammar.js` is kept for reference and regeneration only; it is not
compiled. `parser.c` is large (about 44 MB, 2.3 MB compressed); the
published `cgg-lang` crate stays within crates.io's 10 MB limit.

To refresh: check out the fork at the new revision, run
`tree-sitter generate`, and copy `src/parser.c`, `src/scanner.c` and
`src/tree_sitter/*.h` here.
