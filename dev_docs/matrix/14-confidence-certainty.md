# 14. Confidence & certainty

Every edge cgg emits carries a confidence band: `high`, `medium` or `low`. `high` is meant to say "structurally certain": exactly one candidate, reached by a path the tree itself proves. That path can be an exact or import-anchored qualified name that only one callable carries, a receiver whose declared, cast or constructed type names exactly one owner, the caller's own class, an inherited member with one declaring ancestor, or an overload or clause set cut to one by arity. `medium` is any guess: by-name matching, duck fan-out, sets with several survivors, receivers whose type was itself guessed, and minified callers. `low` is reserved for opt-in guesses (`--guess-unique-names`) and synthetic edges: dynamic dispatch, framework entries, descriptor-to-host links, FFI name fallbacks and exit nodes. Mermaid adds a leading `?` to the label of any arrow with no `high` site. This category asks three things of each language:

1. Which resolution paths can actually reach `high`.
2. Which guards keep a guess from being labelled `high`.
3. Whether the band is honest, that is, whether false `high` edges are observed.

The machinery is mostly shared:

- The cross-file band is set in one place (`crates/cgg-resolve/src/cross_file.rs:2716-2756`).
- Certainty is judged by `CertainCtx` (`cross_file.rs:4470-4651`) and `OwnerFacts`/`unique_owner_names` (`cross_file.rs:4397-4466`).
- Inherited members go through `one_declaring_ancestor` (`cross_file.rs:4654-4691`).
- The `super` exception is `super_owner_may_be_anonymous` (`cross_file.rs:4697-4699`).
- `qn_carries_module` is at `cross_file.rs:4772-4777`.
- The intra-file linker has its own rule: a single candidate is **always** `high` (`crates/cgg-resolve/src/intra_file.rs:629-640`), and a same-name set is `high` only when arity or clauses cut it to one (`intra_file.rs:660-669`).

Per-language differences come from four sources:

- what the plugin extracts (locals, types, arity);
- which language-specific scope engine sets `module_certain` (`cross_file.rs:5400-5620`: modules.rs for JS/TS/Python/Go/Lua; libscope for Dart, Zig, Perl, Elixir and Scala; PhpIndex for PHP; dedicated arms for Clojure and Erlang);
- the per-language arms of `CertainCtx::anchored` and `unique_owner_names`;
- the generic bare-import arm (`cross_file.rs:2728-2736`), which does not consult `CertainCtx`.

## Legend

| Mark | Meaning |
| --- | --- |
| ✅ | full: works for the language's mainstream forms, with no observed false `high` from this capability |
| 🟡 | partial: works for some forms only, or works but has a verified defect (false `high` or an unreachable `high`) |
| ⬜ | none: the language has the concept and cgg does not support it |
| — | n/a: the concept does not exist in the language (no receivers, no inheritance, no overloads, no imports), or the language class makes it moot |
| ❔ | unverified: the language doc is silent and no fixture was run for this cell |

### Columns

| Column | Capability | Where it lives |
| --- | --- | --- |
| **IF** | Intra-file single candidate → `high` | `intra_file.rs:629-640` |
| **PATH** | Cross-file `high` through an exact, import-anchored or module-anchored path (`module_certain`, `CertainCtx::path`) | `cross_file.rs:2722-2726`, `4488-4498`, the scope engines at `5400-5620`, `5766`, `5960-5967`, `6073-6272` |
| **TYPED** | `high` through a typed receiver: declared parameter, local or field, cast, or construction naming one owner (`OwnerFacts` + `anchored`) | `cross_file.rs:4504-4518`, `4582-4651`, `6133-6141` |
| **INHER** | `high` for an inherited member or `super` (`one_declaring_ancestor`, `bases_anchored`) | `cross_file.rs:4554-4565`, `4654-4691`, `4697-4699`, `6510-6640` |
| **ARITY** | An overload or clause set cut to one by argument count → `high` | `type_hints.rs:1583` (`OVERLOAD_ARITY_LANGS` = java, scala, cpp), `intra_file.rs:835-860`, `870-932`, `cross_file.rs:2611-2643`, `4811-4830` |
| **COPIES** | Same-name copies (vendored, `#[cfg]`/build-tag/preprocessor twins, redefinitions) demote to `medium` or are refused, never `high` | `qn_collisions` `cross_file.rs:1009-1021` → `CertainCtx::path` |
| **SCOPE** | No false `high` from scope blindness: params and locals shadowing a callable, out-of-scope nested defs, unimported modules, other classes' members | locals tables `crates/cgg-lang/src/locals.rs:380-393` (JS, TS, Python, Lua, Scala, Kotlin, Swift, Rust, Dart, OCaml, Zig only) |
| **HEUR** | A receiver type that was only guessed (`weak_receiver_type`: name heuristic, return-type guess, file-wide last write) is never `high` | set in `type_hints.rs:805-811`, `853`, `865`, `943`; honoured cross-file (`cross_file.rs:2724`, `4489`, `5203`); **not** honoured intra-file (`intra_file.rs:638`) |
| **LOWG** | The opt-in `low` unique-name guess (`--guess-unique-names`) | `cross_file.rs:2681-2706`, gated `lang == "rust"` (`2687`) |
| **MARK** | Mermaid `?` on arrows with no `high` site | `crates/cgg-format/src/mermaid.rs:21`, `266-277` (language-agnostic) |

## The matrix

| Language | IF | PATH | TYPED | INHER | ARITY | COPIES | SCOPE | HEUR | LOWG | MARK | notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rust | ✅ | ✅ | ✅ | — | — | ✅ | ✅ | 🟡 | ✅ | ✅ | Crate-anchored `rust_owner_certain` (`cross_file.rs:4713-4770`). Guessed file-wide `let` types are weak (`type_hints.rs:807`) but still `high` intra-file. Pattern-bound, `Deref` and glob paths are always `medium` |
| python | ✅ | ✅ | ✅ | ✅ | — | ❔ | ✅ | ⬜ | ⬜ | ✅ | **Name-heuristic types reach `high` intra-file** (fixture `py/`: `def f(k): k.kw()` → `K.kw` high; doc fixtures f4-f6) |
| kivy | 🟡 | — | 🟡 | — | — | 🟡 | 🟡 | — | ⬜ | ✅ | No `CertainCtx`; band comes from the owner score in `link_kivy_python` (`ffi.rs:381-428`). Owner matched by name only, and a name-only fallback binds unrelated classes |
| javascript | ✅ | ✅ | 🟡 | 🟡 | — | ❔ | ✅ | 🟡 | ⬜ | ✅ | `const x = new X(); x.m()` is not a typing source (fixture `js2/` go2 fans out to `Other.run`). `anchored` requires the caller's file (`cross_file.rs:4605`). Cross-file `super` is `medium` |
| typescript | ✅ | ✅ | 🟡 | 🟡 | ⬜ | ❔ | 🟡 | ⬜ | ⬜ | ✅ | **Annotated and `new` locals fan out** (fixtures `ts/` go4 `const t: Svc = new Svc()` and `ts3/` → 3 `medium` edges). Lowercase heuristic gives false `high` (doc f16). Overload sets always `medium` |
| go | ✅ | ✅ | ✅ | ⬜ | — | ✅ | ⬜ | ⬜ | ⬜ | ✅ | No locals table: param and range shadowing give false `high` (doc f10). File-wide `var` last write wins (f9). The `New*` heuristic is firm. Embedded promotion is never `high` |
| java | ✅ | ✅ | ✅ | ✅ | 🟡 | ✅ | ✅ | ✅ | ⬜ | ✅ | **Cross-file arity-to-one stays `medium`** (fixture `jv/`), because overloads share a QN in `qn_collisions` (`cross_file.rs:1013-1021`) and `CertainCtx::path` refuses |
| kotlin | ✅ | ✅ | ✅ | 🟡 | ⬜ | ❔ | ✅ | ❔ | ⬜ | ✅ | `super` is never `high` (blanket `super_owner_may_be_anonymous`). Not in `OVERLOAD_ARITY_LANGS`: fixture `kt/` `over(1)` → both overloads `medium` |
| c | ✅ | ✅ | — | — | — | 🟡 | ⬜ | — | ⬜ | ✅ | No locals: a function-pointer param named like an included function gives a false `high` (doc §07 f7). Preprocessor alternatives bind once at `high` (`cross_file.rs:7316-7360`) |
| cpp | ✅ | 🟡 | ✅ | ✅ | ✅ | ✅ | ⬜ | ❔ | ⬜ | ✅ | No locals table: fn-pointer param false `high` (doc f11). Namespace-alias and static-field-chain calls are `medium`. Cross-file arity-to-one **is** `high` (fixture `cp/`) |
| csharp | ✅ | ✅ | ✅ | ✅ | ⬜ | ✅ | ⬜ | ✅ | ⬜ | ✅ | No arity narrowing (fixture `cs/`: `M(1)` → both overloads `medium`). Owner uniqueness is keyed per directory (`cross_file.rs:4415`), so namespace-narrowed singles stay `medium`. Delegate-shadow false `high` (doc f11b) |
| bash | ✅ | 🟡 | — | — | — | ⬜ | — | — | ⬜ | ✅ | **The sourced-name `high` ignores copies**: fixture `sh2/` binds the *un-sourced* `c.sh:dup` at `high`, and renaming the file flips the target. Cause: bare-import arm `cross_file.rs:2728-2736` |
| ruby | ✅ | 🟡 | 🟡 | ✅ | — | ⬜ | 🟡 | ❔ | ⬜ | ✅ | `require_relative` binds the non-required copy at `high` (doc p5). A reopened class binds a subclass method at `high` (p3). Only construction receivers are typed |
| swift | ✅ | ⬜ | 🟡 | ✅ | ⬜ | ❔ | ✅ | ⬜ | ⬜ | ✅ | **`let engine = Motor(); engine.start()` → `Engine.start` at `high`** (fixture `swift/`). Owner uniqueness per file, so cross-file static and construction calls are `medium` |
| lua | ✅ | ✅ | ⬜ | — | — | ❔ | 🟡 | — | ⬜ | ✅ | No typed-receiver path at all. Use-before-`local` mis-binding is `high` (doc §07) |
| php | ✅ | ✅ | 🟡 | ⬜ | — | ✅ | 🟡 | — | ⬜ | ✅ | Inherited and trait hits are never certain (no `one_declaring_ancestor` arm, `libscope.rs:2059-2293`). Intra-file binds another class's method at `high` (doc p10). No duck fan-out |
| dart | ✅ | ✅ | 🟡 | ✅ | — | ✅ | ✅ | ❔ | ⬜ | ✅ | Local and field receivers stay `medium` (doc f1 L55-57, f10). Extensions are `medium` |
| scala | ✅ | 🟡 | 🟡 | 🟡 | 🟡 | ❔ | 🟡 | ❔ | ⬜ | ✅ | `super` is never `high`. Cross-file arity-to-one is `medium` (fixture `sc2/`), while intra-file is `high` (fixture `sc/`). A bare call to another object's member is a false `high` (doc f5) |
| hcl | 🟡 | ⬜ | — | — | — | — | ⬜ | — | ⬜ | ✅ | The only `high` path is a same-file label match, and it is always wrong (doc fixture c) |
| zig | ✅ | ✅ | ✅ | — | — | ✅ | ✅ | ✅ | ⬜ | ✅ | Type-function results are `medium` (`libscope.rs:1765`). The name heuristics are skipped for Zig (`type_hints.rs:815-820`) |
| objc | ✅ | 🟡 | 🟡 | ⬜ | — | ❔ | ❔ | ⬜ | ⬜ | ✅ | Declared pointer types (`Engine *e`) are ignored (doc §05 line 151). The lowercase heuristic gives `high` intra-file (doc f2). `super` is `medium` |
| r | ✅ | 🟡 | — | — | — | 🟡 | ⬜ | — | ⬜ | ✅ | The `pkg::name` qualifier is never verified, yet binds `high` (doc f2, f3). No locals (f8c, f7) |
| groovy | ✅ | ✅ | ✅ | 🟡 | ⬜ | ❔ | ⬜ | ❔ | ⬜ | ✅ | Bare same-file call across classes is a false `high` (doc f13). `super` is never `high`. Overloads are always `medium` |
| julia | ✅ | 🟡 | — | — | ⬜ | ❔ | ⬜ | — | ⬜ | ✅ | Param shadowing and unimported nested modules give false `high` (doc p6, p4). Multiple-dispatch sets are always `medium` (not in `OVERLOAD_ARITY_LANGS`) |
| perl | ✅ | 🟡 | 🟡 | ✅ | — | ✅ | ❔ | — | ⬜ | ✅ | `use`d-package bare calls are `high` without an Exporter check (doc f7 `not_exported`). Untyped receivers are refused |
| elixir | ✅ | 🟡 | — | — | 🟡 | 🟡 | 🟡 | — | ⬜ | ✅ | An imported bare call can be `high` to the wrong module (doc f5 `E::e`). Multi-arity imports are `medium` and possibly the wrong clause |
| erlang | ✅ | 🟡 | — | — | 🟡 | ✅ | 🟡 | — | ⬜ | ✅ | A variable `Mod:f()` binds the local `f` at `high` (doc f10). Arity fallback gives a false `high` (f2). `-import` is always `medium` and arity-blind |
| fortran | ✅ | 🟡 | ❔ | ❔ | — | 🟡 | ⬜ | — | ⬜ | ✅ | File-wide `use` scope and a module-prefix leak give false `high`. Type-bound procedures (`obj%m`) are not covered by the doc (unverified) |
| clojure | ✅ | 🟡 | — | — | — | 🟡 | ⬜ | — | ⬜ | ✅ | `#_`, `(comment …)`, `let` shadow, no-`:refer` and both-branch `#?` edges are false `high` (doc p1, p3, p4) |
| haskell | ✅ | 🟡 | — | — | 🟡 | ✅ | ⬜ | — | ⬜ | ✅ | Param shadowing, ignored `hiding` and type-class instance selection give false `high` (doc f2, f5). A cross-file multi-equation call is `medium` |
| ocaml | ✅ | ⬜ | — | — | — | ✅ | 🟡 | — | ⬜ | ✅ | Cross-file `high` is unreachable: step 1b needs a qn `B.foo` that OCaml never produces (`cross_file.rs:5942-5968`). Wrong `high` to another function's local `let` |
| powershell | ✅ | 🟡 | 🟡 | ⬜ | — | ❔ | 🟡 | — | ⬜ | ✅ | No receiver types are recorded except type literals (`[T]::M()`). A non-exported module function is a `high` (doc f3). The `([B]$this)` cast gives a self-edge `high` |
| solidity | ✅ | ✅ | 🟡 | ⬜ | ⬜ | ❔ | ⬜ | ❔ | ⬜ | ✅ | Cross-contract intra-file and shadowed `helper()` are false `high` (doc f1). `super` and overloads are always `medium` (no arity) |
| fsharp | ✅ | 🟡 | 🟡 | ⬜ | ⬜ | ❔ | 🟡 | ❔ | ⬜ | ✅ | Only a first-group annotated param is typed. Bare calls after `open` are `medium`. `base.M()` is `medium` |
| starlark | ✅ | 🟡 | — | — | — | ⬜ | ⬜ | — | ⬜ | ✅ | **`load("b.bzl","dup")` binds `c.bzl:dup` at `high`** (fixture `st_84071/`, same arm `cross_file.rs:2728-2736`). Shadowed param and nested def are `high` (doc t2) |
| cmake | ✅ | 🟡 | — | — | — | ⬜ | 🟡 | — | ⬜ | ✅ | `high` to a file that was not included (doc t3) |
| nix | ✅ | 🟡 | — | — | — | 🟡 | ⬜ | — | ⬜ | ✅ | Scoping is not modelled: param shadow, unbound `self.x` and an out-of-scope bare name are all `high` (doc f1-f3). One site gets both a `high` edge and an exit node (f4) |
| verilog | ✅ | ⬜ | — | — | — | ✅ | ✅ | — | — | ✅ | Cross-file instantiation never resolves, so there are no `medium` edges at all |
| vhdl | ✅ | ⬜ | — | — | — | ✅ | ✅ | — | — | ✅ | Cross-file through `use` is `medium` only, even for a single candidate |
| asm | ✅ | ⬜ | — | — | — | ✅ | ✅ | — | — | ✅ | Nothing cross-file. FFI edges are always `medium` (`ffi.rs:117`, `160`) |
| smithy | ✅ | ⬜ | — | — | — | ✅ | ✅ | — | — | ✅ | The descriptor arm never sets `module_certain` (`cross_file.rs:5222-5240`), so a unique cross-file shape is `medium`. It is also namespace-blind |
| proto | ✅ | ⬜ | — | — | — | ✅ | ✅ | — | — | ✅ | The package qualifier is stripped, so an unambiguous `pkg.Msg` is `medium`. Descriptor→host edges are `low` (`descriptor.rs:141`) |
| graphql | ✅ | ⬜ | — | — | — | ✅ | ✅ | — | — | ✅ | A single-candidate cross-file type is `medium` (doc t1) |
| openapi | ✅ | ⬜ | — | — | — | ✅ | 🟡 | — | — | ✅ | A `$ref` inside `example:` gives a `high` edge (doc f). An explicit file `$ref` is still `medium` |
| asyncapi | ✅ | ⬜ | — | — | — | ✅ | 🟡 | — | — | ✅ | The wrong `x.yaml#/…/M` and deep-pointer refs are `high` (doc §03) |
| lean | ✅ | 🟡 | 🟡 | — | — | ❔ | 🟡 | 🟡 | ⬜ | ✅ | File-wide param typing gives `fun p => p.x` a `high`. Type-annotation "calls" are `high`. A qualified `B.foo` is not checked for an import |

### Row-by-row evidence (what each mark rests on)

- **IF ✅ everywhere except HCL and Kivy.**
  - `intra_file.rs:629-640` emits `Confidence::High` for any single same-file candidate, with no other check.
  - HCL is 🟡 because its only match is a block label (hcl.md §14).
  - Kivy is 🟡 because KV→Python binding is scored, not linked (kivy.md §14).
  - Same-file value-reference edges are always `medium` (`intra_file.rs:242`).
- **PATH.**
  - ✅ where the language's scope engine or `CertainCtx::path` gives `module_certain` on the mainstream import form:
    - Rust `rust.md` §14 (`rust_owner_certain`).
    - Python `modules.rs:1914-1959`.
    - JS/TS: fixture `js2/` go3 (`new Svc().run()` with `Svc` imported is `high`).
    - Go `modules.rs` `go_same_package`.
    - JVM: `cross_file.rs:5766` with `qn_carries_module`.
    - C: include closure (c.md f1).
    - C#: `using static` (csharp.md f1).
    - Lua `modules.rs:1702-1705`.
    - PHP `libscope.rs:2059-2293`.
    - Dart `libscope.rs:589-689`.
    - Zig `libscope.rs:1479-1486`.
    - Solidity: solidity.md f3/f6.
    - Groovy: groovy.md f1/f5.
  - 🟡 where the path exists but is either unguarded, so false `high` is observed, or covers only some forms (see notes).
  - ⬜ where cross-file `high` is unreachable:
    - Swift (`unique_owner_names` `_` arm is per file, `cross_file.rs:4416`).
    - OCaml.
    - HCL, Verilog, VHDL, asm.
    - All five descriptor languages (`cross_file.rs:5222-5240` returns candidates without `module_certain`).
- **TYPED.**
  - ✅ where declared params, locals, casts and constructions all reach `high`.
  - 🟡 where only some shapes reach it:
    - JS: `new X().m()` only.
    - TS: params, casts, `new`-expressions; not locals, fields or closures (typescript.md §05).
    - Ruby: construction only.
    - Swift: params and casts; cross-file construction is `medium`.
    - Dart and Scala: locals and fields are `medium`.
    - PHP: PhpIndex params and construction.
    - PowerShell: type literals only.
    - F#: first param group only.
    - ObjC: class receivers only.
    - Perl: invocant only.
    - Solidity: typed params.
    - Lean: typed param, plus a false `high` through file-wide typing.
    - Kivy: owner score.
  - JS/TS anchoring requires the candidates in the caller's file (`cross_file.rs:4605`). The scala arm requires the directory to mirror the package (`cross_file.rs:4627-4642`). Every other language takes `_ => true` (`cross_file.rs:4643`).
- **INHER.**
  - ✅ where `one_declaring_ancestor` and `bases_anchored` apply and `super` can be `high`: Python, Java, C++ (typed), C#, Ruby, Swift, Dart, Perl.
  - 🟡 for Kotlin, Scala and Groovy: inherited bare and typed calls can be `high`, but `super` never is (`cross_file.rs:4697-4699`, `5287`).
  - 🟡 for JS and TS: same-file only.
  - ⬜ for Go: embedded promotion is `certain:false` (go.md §14).
  - ⬜ for PHP: always uncertain.
  - ⬜ for ObjC, Solidity, F# and PowerShell: `super`/`base` are `medium`, or no class model exists.
- **ARITY.**
  - `OVERLOAD_ARITY_LANGS` is java, scala and cpp (`type_hints.rs:1583`).
  - The clause languages (Erlang, Elixir, Haskell) use `clause_targets` (`intra_file.rs:906-932`) and Elixir `resolve_elixir` (`cross_file.rs:5547-5552`).
  - ⬜ for overloading languages outside the list that `binds_same_name_set` still groups (`intra_file.rs:835-856`): Kotlin, C#, Swift, Groovy, F#, Solidity, TS and Julia. Fixtures `kt/` and `cs/` show both overloads at `medium`.
  - Java and Scala are 🟡 because the cross-file narrowed-to-one case is `medium`, confirmed by fixtures `jv/` and `sc2/`. C++ is ✅: fixture `cp/` gives cross-file `high` via the include closure.
- **COPIES.**
  - ✅ where a doc fixture shows copies demoted or refused:
    - Rust `#[cfg]` twins.
    - Go build tags (f12).
    - Java same-name class (f2).
    - C++ f7c.
    - C# two dirs.
    - PHP copies in one file.
    - Dart several namespaces.
    - Zig (`cross_file.rs:5461-5463`).
    - Perl and Erlang `narrow_copies`.
    - Haskell.
    - Verilog, VHDL and asm (refused).
    - The descriptor languages (always `medium`).
  - ⬜ where a fixture shows a copy bound `high`: Bash (fixture `sh2/`), Starlark (fixture `st_84071/`), Ruby (doc p5) and CMake (doc t3).
- **SCOPE.**
  - ✅ requires a locals table (`locals.rs:380-393`) and no observed scope false `high`.
  - Java and C# have no bare call through a local, so Java is ✅ by language shape.
  - C# is ⬜ because of the delegate case (csharp.md §07 f11b).
- **HEUR.**
  - `weak` is computed for every language that reaches pass 3 of `propagate_types_with_returns` (`type_hints.rs:557-943`). Only Zig skips the name heuristics (`815-820`).
  - Cross-file honours it (`cross_file.rs:2724`, `CertainCtx::path` `4489`). Intra-file does not (`intra_file.rs:638`). Every language with a heuristic-typed receiver is therefore at best 🟡.
  - ⬜ where a doc or fixture shows the false `high`: Python (fixture `py/`), Swift (fixture `swift/`), TS (doc f16), Go (`New*`), ObjC (doc f2).
  - ✅ for Java (test `java_declared_receiver_through_import_is_high_file_wide_guess_is_medium`), C# and Zig.
  - ❔ where no doc or fixture exercised the heuristic.
- **LOWG.** Rust only (`cross_file.rs:2687`). ⬜ for every other language with name lookup; — for HDLs, asm and descriptors.
- **MARK.** Generic. The rendering is `mermaid.rs:266-277`; every language doc §14 cites a fixture arrow, or says the arrow is generic.

## Parity gaps

Ranked by how many languages a gap affects, and by how wrong the resulting `high` is.

1. **Intra-file `high` ignores `weak_receiver_type`. This affects every language with a type heuristic (Python, Swift, TS, JS, ObjC, Go, Rust, Lean). Cross-file already honours the flag.**
   - Evidence: `intra_file.rs:629-640` emits `High` unconditionally, while `cross_file.rs:2724` and `CertainCtx::path` (`4489`) refuse `high` when the receiver type is weak.
   - Fixture `py/`: `def f(k): k.kw()` → `K.kw` `high`, with `J.kw` also present.
   - Fixture `swift/`: `let engine = Motor(); engine.start()` → `Engine.start` `high`, which is the wrong class.
   - Python doc §14 lists the same defect, and swift.md §14 flags it too.
   - Effort **S**: gate the intra-file single-candidate arm on `!rref.weak_receiver_type` when a receiver is present. Measure edge movement across the full corpus, because Rust WS13 history shows `high` counts are load-bearing.
2. **The bare-import arm bypasses `CertainCtx` (Bash, Starlark, and likely CMake, PowerShell, R and Ruby), unlike the guarded module path that Python, JS and Lua take.**
   - Evidence: `cross_file.rs:2728-2736` grants `High` when `cids.len()==1 && direct_imports.contains_key(&r.name)` with no `qn_collisions` or `CertainCtx::path` check.
   - Fixture `sh2/`: `a.sh` sources `b.sh`; `c.sh` defines `dup` too; the call binds `c.sh:dup` at `high`, and renaming `c.sh`→`0c.sh` makes it bind `b.sh` instead (the choice is name-order dependent).
   - Fixture `st_84071/`: `load("b.bzl","dup")` → `c.bzl:dup` `high`.
   - Effort **S** (add `cert.path(cids)` to the arm). Separately, the lookup must restrict to the sourced or loaded file: **M**.
3. **No locals table, so there is no shadowing guard for Go, C, C++, C#, R, Groovy, Julia, Haskell, Clojure, Nix, Starlark, Solidity, Fortran and Perl. Rust, Kotlin, Dart, Scala, Swift, Python, JS, TS, Lua, OCaml and Zig have one.**
   - Evidence: `locals.rs:380-393` lists only 11 languages.
   - False `high` from shadowing is observed in go.md (f10), c.md (f7), cpp.md (f11), csharp.md (f11b), r.md (f8c), julia.md (p6), haskell.md (f2), clojure.md (p1), nix.md (f1), starlark.md (t2) and solidity.md (f1).
   - Effort **M** per language (a `Spec` of binder node kinds), **L** in total.
4. **Cross-file arity-narrowed overloads are never `high` in Java or Scala, while C++ gets `high`.**
   - Evidence: fixture `jv/` (`A.over(1)` from `B.java` → `medium`), fixture `sc2/` (`A.over(1)` → `medium`), fixture `cp/` (C++ `over(1)` cross-file → `high`). The intra-file Scala case is `high` (fixture `sc/`).
   - Cause: overloads share a QN, so `qn_collisions` (`cross_file.rs:1013-1021`) contains it and `CertainCtx::path` refuses. CLAUDE.md's "an overload set narrowed to one by arity" is `high` only intra-file or through the C-family path.
   - Effort **S**: exempt collisions that are one file's overload set (`one_method`, `cross_file.rs:4787`) when arity left one.
5. **No arity narrowing for Kotlin, C#, Swift, Groovy, F#, Solidity, TypeScript or Julia, compared with Java, Scala and C++.**
   - Evidence: `OVERLOAD_ARITY_LANGS` (`type_hints.rs:1583`). Fixtures `kt/` and `cs/` show `over(1)`/`M(1)` → both overloads `medium`. kotlin.md, groovy.md, solidity.md and fsharp.md §14 show the same.
   - Effort **M**: the plugins must set `RefRecord::arity`, and `accepts_arity` must handle named/default args (Kotlin, C#, Swift labels).
6. **JS and TS locals are not a typing source (`const t = new X()`, or annotated `const t: X = …`), unlike Dart params, Kotlin/Java locals and Go `x := NewT()`.**
   - Evidence: fixture `ts3/` (same-file `const t = new Svc(); t.run()` → 3 `medium` edges, including `Third.run` in another file) and fixture `ts/` go4 (`const t: Svc = new Svc()` → 2 `medium`); javascript.md §05 lines 193-196 and typescript.md §05 lines 209-210 agree.
   - Effort **M**: a file-wide or scoped `local_types` capture for `lexical_declaration`/`variable_declarator`.
7. **Kotlin, Scala and Groovy `super` is never `high` because of a blanket language rule, while Java, C#, Python and Dart judge it per call.**
   - Evidence: `super_owner_may_be_anonymous` (`cross_file.rs:4697-4699`, applied at `5287`) excludes the whole language because anonymous-object `super` is recorded against the enclosing class.
   - Effort **M**: record whether a site is inside an object expression or anonymous class (plugin), then refuse only those sites.
8. **Descriptor languages (Smithy, Proto, GraphQL, OpenAPI, AsyncAPI) can never reach cross-file `high`, even with one candidate or a fully qualified name.**
   - Evidence: `cross_file.rs:5222-5240` returns `by_simple` candidates (up to 4) without setting `module_certain`; graphql.md t1 (one `Person` → `medium`); proto.md (package qualifier stripped).
   - Effort **S** for a single candidate whose namespace or package matches (Smithy `ns#Shape`, proto `pkg.Msg`, an OpenAPI file `$ref`). **M** for namespace-aware narrowing.
9. **PHP inherited and trait members are never `high`, while Java, C#, Dart and Ruby use `one_declaring_ancestor`.**
   - Evidence: php.md §14 (`libscope.rs:2059-2063`, `2187-2189`, `2286-2293`: `certain` only for a hit on the class itself).
   - Effort **M**: count declaring ancestors in `PhpIndex`, including traits.
10. **OCaml cross-file `high` is unreachable, while F# (stem-matched module) and Haskell (single opened module) reach it.**
    - Evidence: ocaml.md §14 (step 1b at `cross_file.rs:5942-5968` needs qn `B.foo`, and OCaml qns carry no module, ocaml.md §02).
    - Effort **M**: give file-module-qualified qns or a module-to-file map (`module_files` already exists for `open`).
11. **Swift owner uniqueness is per file, so a cross-file static or construction call is `medium`; Kotlin and Java judge owners by QN path.**
    - Evidence: `unique_owner_names` `_ => (path, file_path())` (`cross_file.rs:4416`); swift.md §14 (b.swift L5, L7).
    - Effort **S/M**: treat a Swift module (target directory) as the site, as Go and C# use directories.
12. **C# owner uniqueness is keyed per directory even after namespace narrowing has left one candidate, while Java uses the QN path.**
    - Evidence: `cross_file.rs:4415`; csharp.md §14 f3/f4 (`Util.Go` `medium`).
    - Effort **S**: key C# by namespace QN when the plugin recorded one (`FileFacts::packages`).
13. **ObjC ignores declared receiver types (`Engine *e`), while C++ and Swift use them.**
    - Evidence: objc.md §05 line 151 (`Engine *eng` fans out to `Engine::start` + `Motor::start`).
    - Effort **M**: record declaration types into `local_types` in the objc plugin.
14. **Erlang `-import` bare calls are always `medium` and arity-blind, while Elixir `import` bare calls reach `high` (and Elixir module-qualified calls are arity-narrowed).**
    - Evidence: erlang.md §14 (f2, f3); elixir.md §14 (`cross_file.rs:5945-5975`).
    - Effort **S**.
15. **Dart and Scala local and field receivers stay `medium`, while Kotlin and Java locals and fields reach `high`.**
    - Evidence: dart.md §14 (f1 L55-57, f10); scala.md §14 (local `val`, constructor param, class `val`).
    - Effort **M**.
16. **JS/TS typed anchoring requires the candidate in the caller's file. Python accepts "imported from the candidate's own package".**
    - Evidence: `CertainCtx::anchored` `"javascript" | "typescript" => in_caller_file` (`cross_file.rs:4605`).
    - Mitigated for imported classes by the ModuleIndex path: fixture `ts/` go (param `s: Svc` imported → `high`), fixture `js2/` go3. It still bites for types reached through re-exports or by name only.
    - Effort **M**.
17. **Bare calls into imported modules (Perl `use`, PowerShell `Import-Module`, Lean qualified names) bind `high` without an export or import check, while JS/TS/Python follow module exports (`modules.rs`).**
    - Evidence: perl.md §14 (`not_exported` `high`), powershell.md §14 (`Get-Private` `high`), lean.md §14 (`B.foo` without `import`).
    - Effort **M** each.
18. **The `--guess-unique-names` low band exists only for Rust.**
    - Evidence: `cross_file.rs:2687`. This is deliberate (CLAUDE.md), but no other language has an opt-in `low` recall path.
    - Effort **S** to generalise the gate. Generalising would need per-language guards like `unique_name_guess`'s.
19. **Kivy, HCL, Verilog, VHDL and asm have no structural `high` beyond same-file names.**
    - HCL's only `high` is wrong (hcl.md fixture c).
    - Verilog and asm never resolve cross-file.
    - Kivy owner matching is by bare class name (`ffi.rs:424-428`).
    - Effort **M** each.

## Doc corrections

1. **rust.md §14** says "Rust does not set `weak_receiver_type` (Python only)". This is wrong: `type_hints.rs:805-811` marks a Rust receiver weak whenever its type came from the file-wide map rather than a visible `let` (`rust || …`). Strategies 4 and 2 (`type_hints.rs:853`, `865`) mark weak for every language, and `type_hints.rs:943` writes the flag on every reference.
2. **typescript.md §14** says "Not applicable: `weak_receiver_type`". This is wrong for the same reason. TS receivers typed by the lowercase heuristic (`find_constructor_assignments`) are marked weak; the false `high` in fixture f16 arises because intra-file ignores the flag (`intra_file.rs:638`), not because the flag is never set.
3. **zig.md §14** says "`weak_receiver_type` is never set for Zig … unverified". This is now resolved: Zig skips the name heuristics (`type_hints.rs:815-820`), and its locals are a scoped reading (`scoped_reading = cpp || zig`, `type_hints.rs:741`), so the file-wide weak branch (`806`) does not fire.
4. **starlark.md §14** says "Why the collision check did not lower these to medium is unverified". The cause is the bare-import confidence arm `cross_file.rs:2728-2736`. It grants `High` for one candidate whose bare name is in `direct_imports`, without `CertainCtx::path`/`qn_collisions`. Reproduced with fixture `st_84071/` (`load("b.bzl", "dup")` → `c.bzl:dup` `high`). The same fixture with label `":b.bzl"` or `"//:b.bzl"` gave no edge (audit: `candidates-in-other-files`, 2 candidates), not the `medium` that doc fixture t4 reports. That behaviour depends on the shape (unverified which).
5. **bash.md §14** says "CLAUDE.md's `CertainCtx` rule … is evidently not enforced on this path". This is confirmed, and the mechanism is the same arm (`cross_file.rs:2728-2736`). The wrong choice also depends on file name order: fixture `sh2/` binds `c.sh:dup`, and after renaming `c.sh`→`0c.sh` it binds `b.sh:dup`, both at `high`.
6. **scala.md §14** says "every overload binding, even when narrowed to one (fixture f4)" is `medium`. This is wrong for intra-file calls: fixture `sc/` (`def over(x: Int)`, `def over(x: Int, y: Int)`, `over(1)` in the same object) gives one `high` edge, because Scala is in `OVERLOAD_ARITY_LANGS` and `intra_file.rs:660-669` gives `High` for one survivor. Only the cross-file case is `medium` (fixture `sc2/`).
7. **java.md §14** says cross-file arity-to-one is "medium, likely from `qn_collisions`". This is confirmed: overloads share one QN, so `qn_collisions` (`cross_file.rs:1013-1021`) contains it and `CertainCtx::path` (`4488-4498`) refuses. Fixture `jv/` shows both the static (`A.over(1)`) and typed-receiver (`a.over(1, 2)`) cases at `medium`.
8. **objc.md §14** says "No `OwnerFacts`-style typed-receiver certainty exists, since there are no declared types". This is wrong on the language. Objective-C has static pointer types, and the same doc's §05 (objc.md:151) shows `Engine *eng` being ignored. The gap is that cgg does not record them, not that they do not exist.
9. **javascript.md §14** says "A bare-type-name lookup across files therefore stays medium". This is too broad. A construction receiver of an imported class is bound `high` cross-file through the ModuleIndex path before `anchored` is consulted: fixture `js2/` go3 `new Svc().run()` → `Svc.run` `high` (`cross-file:imports`). Only lookups that reach the `OwnerFacts` arm are held to `in_caller_file`.
10. **CLAUDE.md** (not edited; recorded here) lists "an overload set narrowed to one by arity" among the `high` paths without qualification. In practice that path is `high` only intra-file and through the C-family include path (fixture `cp/`). JVM cross-file narrowing to one stays `medium` (item 7).

### Fixtures run for this matrix

All are under `<scratch>/parity/matrix-14/`, run with `target/release/cgg <dir> -t json` (0.11.0) and printed by `show.py` in that directory.

| Fixture | Result |
| --- | --- |
| `sc/` | Scala intra-file `over(1)` → `A::over@2` **high** |
| `sc2/` | Scala cross-file `A.over(1)` → `A::over@3` **medium** (one edge; arity narrowed) |
| `kt/` | Kotlin intra-file `over(1)` → both overloads **medium** |
| `cs/` | C# intra-file `M(1)` → both overloads **medium** |
| `jv/` | Java cross-file `A.over(1)` and `a.over(1, 2)` → one overload each, **medium** |
| `cp/` | C++ cross-file `over(1)` → `over@a.cpp:2` **high** |
| `sh2/` | Bash: sourced `b.sh` and unsourced `c.sh` both define `dup` → **high** to `c.sh` (to `b.sh` after renaming `c.sh` to `0c.sh`) |
| `st_84071/` | Starlark: `load("b.bzl","dup")` → **high** to `c.bzl:dup` |
| `py/` | Python `def f(k): k.kw()` with `K.kw` and `J.kw` → `K.kw` **high** intra-file |
| `swift/` | Swift `let engine = Motor(); engine.start()` → `Engine.start` **high** |
| `ts/` | TS param `s: Svc` (imported) → **high**. `const t = new Svc()` and `const t: Svc = new Svc()` → fan-out to `Svc.run` + `Other.run`, **medium**. `(new Svc()).run()` → **high** |
| `ts3/` | TS same-file `const t = new Svc(); t.run()` → three **medium** edges, one of them into another file |
| `js2/` | JS `const t = new Svc(); t.run()` → two **medium** edges. `new Svc().run()` with `Svc` imported → **high** |
