# Backlog

Work that is known and deliberately deferred. Nothing here is scheduled.
Each item says why it is waiting and roughly what it would take. When an
item ships, delete it here and describe it in `CHANGELOG.md`.

Versioning: a new grammar, parse mechanism or language is a minor release
(`0.x.0`); a fix to an existing parser or resolver is a patch.

## Minor release (0.12.0) candidates

### Groovy: a fuller grammar

`tree-sitter-groovy` rejects a number of everyday Groovy shapes, and every
method and call inside the resulting ERROR span is lost or misread. 0.11.2
(WS38) recovered what same-length `parse_source` rewrites can reach: ERROR
nodes across the corpus's 9,520 Groovy files fell from 26,212 to 12,411,
and bytes inside ERROR spans from 8.6 MB to 2.2 MB. The rest needs a
grammar that accepts these shapes, either a fuller vendored grammar
(`crates/cgg-lang/vendor/`) or an upstream fix. That is a new parser, so
it belongs in a minor release.

Shapes no rewrite can fix:

- untyped parameters, `def f(x, y)`, read as a type followed by a name
- leading-dot continuation lines, `x` then `.m()` on the next line. A
  rewrite would have to move a newline
- strings used as calls or DSL blocks, `'org:a:1.0' { … }` and
  `'g:a:1'()`
- nested quotes inside a GString interpolation, `"${f("a")}"`
- multiple assignment, `def (a, b) = …`
- untyped modified fields with an uppercase name,
  `private static final ABSOLUTE = …`
- Java-style lambdas as arguments, `x -> expr`
- chained commands, `module('a') tries('1') alignsTo('2')`
- a command as an initializer, `def r = script.sh script: 'x'`
- a closure after a literal argument, `run "x" { … }`
- quoted member access, `obj.'NAME'`
- multi-line slashy and dollar-slashy strings

## Patch-level follow-ups

Small gaps found during the 0.11.2 remediation (WS32–WS38). Each is a fix
to an existing plugin or resolver.

- **Groovy command-expression calls.** The plugin never records
  `juxt_function_call` nodes, so `run "x"`, `succeeds 'x'` and
  `apply plugin: …` produce no call record anywhere.
- **Kotlin `typealias`.** Return types and annotated locals written
  through an alias stay untyped (2 correct edges lost in WS35).
- **Kotlin trailing lambdas in the arity check.** The count is right, but
  a trailing lambda is not placed against the last parameter, so
  overloads whose last parameter is a defaulted lambda stay `medium`
  (12 edges that could be `high`).
- **C entry points `wmain` / `WinMain`.** Not recognised as roots, so
  their callees are reported dead.
- **Clang `__overloadable__` C functions.** Should bind as an overload
  set; today they are `ambiguous-in-file`.
- **C macros redefined after `#undef`.** Should bind the definition in
  effect at the call.
- **Clojure `letfn` body values.** Forms in a `letfn` body are not read as
  value references, so a `letfn` function used only as a map value is
  reported dead (1 corpus finding).
- **F# files whose parse fails.** The `module X.Y` header no longer
  encloses later declarations, so their functions keep bare names and
  module-qualified calls to them fall back to by-name binding.
- **Nested functions in Scala, Kotlin, Swift, Dart and Rust.** WS33
  keeps a function nested in another function out of other files' bare-name
  candidates, but only in `NESTED_SCOPE_LANGS`. These five were left out
  because their member names carry the type, not the enclosing function,
  and a span a parse recovered past made type members look nested. Revisit
  with a nesting test that survives parse recovery.
- **Solidity Yul.** Inline-assembly calls are not recorded.
- **C/C++ type names read as value references** (29 `medium` corpus
  edges).
- **Spock `ProviderSpec`** fixtures not rooted (20 `medium` findings).
- **Fortran arrays brought in with `use`** are read as calls (1 site).
- **JS/TS coincidental roots** hide some findings in vendored code.
- **Dart bases declaring no methods** (393 `medium` findings).
- **Perl `$handler` registrar values.** The registrar capture strips the
  sigil, so `get "/x" => $handler` binds an unrelated `sub handler`.

## Process

- Re-score the parity matrix (`dev_docs/matrix/` sections 02, 05, 06, 07,
  11, 12 and 14) against the post-0.11.2 code.
- Tidy the stale rule `gap` prose for go-testing, eunit, common-test,
  xctest, fsharp-nunit and cpp-test-macros (suppressed by
  `has_matchers`, so never printed).
