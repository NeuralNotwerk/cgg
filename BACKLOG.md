# Backlog

Work that is known and deliberately deferred. Nothing here is scheduled.
Each item says why it is waiting and roughly what it would take. When an
item ships, delete it here and describe it in `CHANGELOG.md`.

Versioning: a new grammar, parse mechanism or language is a minor release
(`0.x.0`); a fix to an existing parser or resolver is a patch.

## Patch-level follow-ups

Small gaps found during the 0.11.2 remediation (WS32–WS38). Each is a fix
to an existing plugin or resolver.

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

## Found in the 0.13.4 documentation audit (2026-11-25)

Reported by the doc-audit agents from fixtures run against 0.13.3; each
needs a fixture test and a corpus A/B before it ships. Fixes to existing
resolvers, so patch-level.

Wrong `high` edges (fix first):

- Go: `n := NewServer()` types `n` by the `New*` name over the declared
  result type, so `n.Zap()` binds the wrong owner.
- TS/JS: a parameter does not shadow an imported name of the same name.
- Solidity: bare calls bind across unrelated contracts in one file.
- Nix: a bare name binds an imported file's attribute.
- R: a call binds a function nested inside another function.
- Clojure: `(comment …)` bodies produce call edges.
- Erlang: `Mod:f()` through a variable binds the local `f`.
- Erlang/Elixir: a wrong-arity call binds the single candidate.
- CMake: the include arm always tries the literal `CMakeLists.txt`, so
  any `include()` reaches every `CMakeLists.txt`'s functions at `high`.

Other resolver and extraction gaps:

- Lua: a `local function` is reachable from a requiring file (medium).
- Starlark: a `load`ed name loses to a same-named nested function.
- Fortran: an interface body plus a same-named function is
  `ambiguous-in-file`.
- F#: `module U = A.B.C` (dotted target) does not resolve; bare calls
  after `open` reach modules never opened.
- Groovy: `this.<T>id()` breaks the parse; import aliases ignored and
  wildcard imports read as class imports; no external-module refusal;
  `new X()` with no constructor leaves an unresolved record.
- Python: `f = self.loader.load; f()` stays untyped.
- JS/TS: nested one-line `it` callbacks get duplicate `handler_at_N`
  names; `process.on('exit', function onExit(){})` mints two callables.
- Kotlin: `get()` on the next line missed; an enclosing-class member
  call is ambiguous when an unrelated class declares the name; a
  one-line named companion loses its class.
- Scala: `Square(3.0)` with a companion `apply` binds the constructors.
- Java: generic `new Box<>()` never binds.
- C#: `global using static` ignored; `aspnet-mvc` mints an entry for a
  private method.
- C: a file-scope initializer naming another file's function is not
  credited, so that function is reported dead.
- Duplicate bucket records for alias calls: Zig, Dart, PHP, Clojure
  (with `--include-external`).
- Rust dead code: a function used as `value_parser = f` inside a clap
  attribute is reported dead at `high` (cgg's own `parse_externals`).
- `--externals`: `use rayon::prelude::*` absorbs std `Vec` methods;
  PowerShell `Start-Process` reported as stdlib, not a process site;
  `--include-external` leaf names embed receiver expressions
  (`File::open(path)::with_context`).
- Kivy: a `<A,B>:` rule after another rule links nothing.
- Stdlib tables: HCL (`strcontains`, …), PHP (`get_class_methods`).
- Verilog tasks falsely rooted through the `task` attribute; Lean
  visibility never captured; Swift has no `signals()` override.
- Stale gap text in `rules.rs` (gradle-task, grails, flutter-test,
  shelf, expecto, fsharp-nunit, scotty, xctest, go-testing), the
  `go.rs` header and the Rust `BLIND_SPOTS` entry in `caps.rs`.
- Labels: withheld `only-used-by-tests` rows say "(signal not
  extracted)"; the stderr "N unresolved call(s)" line counts external
  calls too.
- `scripts/framework-coverage.py` has no `CGG_TOTAL_BUDGET`.
- Haskell: `import C hiding (h)` still binds `h` at `high`; qualified
  calls (`U.helper`) make no edge to in-tree modules.
- Python: `import os, flask` hides flask from detection; comprehension
  and `for` variables do not shadow (binds the module function, `high`).
- Swift: `let m = Motor(); m.start()` leaves `m` untyped.
- Starlark: `load(":a.bzl", "f")` / `load("//:a.bzl", "f")` make no edge.
- PHP: `call_user_func` missing from `PHP_CALLABLE_ARG_FUNCS`.
- ObjC: `#define MKFN(n)` then `MKFN(one)` breaks the parse.
- CMake: a top-level call to an `include()`d function is unresolved, so
  the function is reported dead.
- Kotlin dead code: capability output says `max_confidence: high` while
  every finding is capped at medium.
- PowerShell: native commands get an empty `--externals` symbol
  (`git::`); dot-source paths written with `\` do not resolve.
- Perl: `POSIX::floor` records an extra `POSIX` external site.

## Process

- Re-score the parity matrix (`dev_docs/matrix/` sections 02, 05, 06, 07,
  11, 12 and 14) against the post-0.11.2 code.
- Tidy the stale rule `gap` prose for go-testing, eunit, common-test,
  xctest, fsharp-nunit and cpp-test-macros (suppressed by
  `has_matchers`, so never printed).
