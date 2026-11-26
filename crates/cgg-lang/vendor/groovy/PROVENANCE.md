# Vendored Groovy tree-sitter grammar

`parser.c`, `scanner.c` and `tree_sitter/*.h` are generated from
[`dekobon/tree-sitter-groovy`](https://github.com/dekobon/tree-sitter-groovy)
(grammar ABI 15, `tree-sitter-cli` 0.27.2), dual licensed Apache-2.0 OR MIT.
See `LICENSE-APACHE` and `LICENSE-MIT`.

The Groovy plugin parses with it (issue #11). It replaced the unmaintained
`tree-sitter-groovy` 0.1 crate and the same-length source rewrites cgg ran
before it (`groovy_rewrite.rs`). `src/plugins/groovy_next.rs` pins the
shapes the old crate rejected.

**Source:** branch `cgg/vendor-0.14.0` of the fork
[`NeuralNotwerk/tree-sitter-groovy`](https://github.com/NeuralNotwerk/tree-sitter-groovy/tree/cgg/vendor-0.14.0),
commit `59095a049a72c6d09f6f40693df442f38efe731e`. It is upstream `main` at
`724a7a6` (after upstream PR #122; the only release, v0.6.0, parses worse)
with the five branches below merged in this order, each the head revised
after review and rebased on `724a7a6`, and `src/parser.c` regenerated
rather than merged:

| Change | Upstream | Branch head |
| --- | --- | --- |
| `?.@`, `??.@`, `*.@` direct field access | issue #112, PR #115 | `12a30c4` |
| String keys in parenthesis-free calls (`attributes 'Main-Class': 'a.B'`); the `:` after a string in a ternary's consequence stays the ternary's | issue #113, PR #116 | `8b98a36` |
| Annotated untyped declarations (`@Field FOO = …`, `@Subject svc = …`) | issue #114, PR #117 | `8963c7c` |
| Annotations and modifiers in any order (`private @Autowired Foo f`, `public @interface X`, `sealed abstract class W`, `final @A x`, `enum F { A, private @Foo F() {} }` a constructor) | issue #92, PR #120 | `92d0214` |
| A slashy string spans lines, and may start with a space, where no division may follow (`x = /a` then `b/`); never during error recovery | issue #119, PR #121 | `9025b28` |

One commit on top (`59095a0`) re-pins the expected tree of the corpus
test "A slash during error recovery opens no slashy string across lines"
(#121), which changes once #117 is merged too: the recovery holds one
`ERROR` instead of two. The previous vendored commit was `7d525a3`
(upstream `eeb38537` plus the pre-review PR heads), still the head of
the fork's `cgg/vendor` branch.

Over 21,667 real Groovy files (the cgg corpus plus apache/groovy,
grails-core and spock), this commit has 147 more error-free files than
upstream `main` at `724a7a6` (21,009 → 21,156; the sum of the five PRs
measured alone) and none newly erroring; 4 files that already had an
`ERROR` have a longer one (PR #120's documented cases). Against the
previous vendored commit `7d525a3` it changes 5 trees, all in files that
already had an `ERROR`, 3 with shorter ones.

**Local patch** (applied to the generated C, not to `grammar.js`): every
exported symbol is renamed from `tree_sitter_groovy*` to
`tree_sitter_groovy_dekobon*`, because the `tree-sitter-groovy` crate
exported `tree_sitter_groovy` too and the two could not otherwise link into
one binary while the port was in progress. Kept so that a crate exporting
the plain name never clashes.

`grammar.js` is kept for reference only; it is not compiled.

To refresh: in the fork, `npx tree-sitter generate` (with
`TREE_SITTER_LIBDIR` set to a private directory for any `tree-sitter`
command that compiles the grammar, so a stale shared
`~/.cache/tree-sitter/lib/groovy.so` is never loaded), then from its root

```bash
V=/path/to/cgg/crates/cgg-lang/vendor/groovy
cp src/tree_sitter/*.h "$V/tree_sitter/"
cp grammar.js LICENSE-APACHE LICENSE-MIT "$V/"
for f in parser.c scanner.c; do
  sed 's/\btree_sitter_groovy\b/tree_sitter_groovy_dekobon/g;
       s/\btree_sitter_groovy_external_scanner_/tree_sitter_groovy_dekobon_external_scanner_/g' \
    "src/$f" > "$V/$f"
done
```

and update the commit above.
