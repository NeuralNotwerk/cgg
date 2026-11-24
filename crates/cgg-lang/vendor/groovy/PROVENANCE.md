# Vendored Groovy tree-sitter grammar

`parser.c`, `scanner.c` and `tree_sitter/*.h` are generated from
[`dekobon/tree-sitter-groovy`](https://github.com/dekobon/tree-sitter-groovy)
(grammar ABI 15, `tree-sitter-cli` 0.27.2), dual licensed Apache-2.0 OR MIT.
See `LICENSE-APACHE` and `LICENSE-MIT`.

The Groovy plugin parses with it (issue #11). It replaced the unmaintained
`tree-sitter-groovy` 0.1 crate and the same-length source rewrites cgg ran
before it (`groovy_rewrite.rs`). `src/plugins/groovy_next.rs` pins the
shapes the old crate rejected.

**Source:** branch `cgg/vendor` of the fork
[`NeuralNotwerk/tree-sitter-groovy`](https://github.com/NeuralNotwerk/tree-sitter-groovy/tree/cgg/vendor),
commit `7d525a3254e15af002e23d570be3684833bccc56`. It is upstream `main` at
`eeb38537` (the only release, v0.6.0, parses worse than our current
grammar plus `groovy_rewrite.rs`), with these changes:

| Change | Upstream |
| --- | --- |
| `?.@`, `??.@`, `*.@` direct field access | issue #112, PR #115 |
| String keys in parenthesis-free calls (`attributes 'Main-Class': 'a.B'`) | issue #113, PR #116 |
| Annotated untyped declarations (`@Field FOO = …`, `@Subject svc = …`) | issue #114, PR #117 |
| A slashy string spans lines, and may start with a space, where no division may follow (`x = /a` then `b/`) | issue #119, PR #121 |
| Annotations and modifiers in any order (`private @Autowired Foo f`, `public @interface X`, `sealed abstract class W`, `final @A x`) | issue #92, PR #120 |

Over 21,667 real Groovy files (the cgg corpus plus apache/groovy,
grails-core and spock), the fork has 50 more error-free files than
upstream `main` with the three PRs merged, and none newly erroring.

**Local patch** (applied to the generated C, not to `grammar.js`): every
exported symbol is renamed from `tree_sitter_groovy*` to
`tree_sitter_groovy_dekobon*`, because the `tree-sitter-groovy` crate
exported `tree_sitter_groovy` too and the two could not otherwise link into
one binary while the port was in progress. Kept so that a crate exporting
the plain name never clashes.

`grammar.js` is kept for reference only; it is not compiled.

To refresh: in the fork, `npx tree-sitter generate`, then from its root

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
