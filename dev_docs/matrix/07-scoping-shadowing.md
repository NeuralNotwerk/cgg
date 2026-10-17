# 07. Scoping & shadowing

**Category definition.** This category covers how cgg decides which *name binding* a bare call (or the head of a receiver) refers to, given where in the source the call sits. It includes:

- the per-file table of parameters and locals with their scope spans (`FileFacts::locals`, built by `cgg_lang::locals::collect` from a per-language `Spec`, `crates/cgg-lang/src/locals.rs:380-393`), or a plugin-side equivalent;
- the `shadowed-by-local` refusal: a bare call through a parameter or local is never a same-named function declared outside the binding's function (`crates/cgg-resolve/src/scope.rs:117-124`, `intra_file.rs:275-306`, `cross_file.rs:2388-2443`);
- receiver heads that are locals, which are therefore not imports or packages (`scope.rs:129-138`, `heads.rs:678`, `cross_file.rs:5389/5429/5456/6064`);
- declaration order: a declaration is in scope only after its initializer (`Scope::Decl`, `locals.rs:23`);
- lexical narrowing of nested definitions (`intra_file.rs:119-137` allowlist, applied at `intra_file.rs:350-399`);
- ML-style positional shadowing, where the last same-scope binding before the site wins (`intra_file.rs:400-416`, OCaml/F#/Lua);
- self-exclusion of a non-recursive binding (`RefRecord::not_def`, OCaml);
- confinement of a bare call to its enclosing package, module, contract or object inside one file;
- whether a local hides an *imported* name;
- conditional or alternative definitions (`DefRecord::conditional`, preprocessor twins).

Class-member lookup (`member_of_calling_class`, bases) is category 06. Import resolution is category 04. Type-binding shadowing (`RustBindings::shadowed_since`) is category 05.

**Sources.** Section `## 07. Scoping & shadowing` and Summary-scorecard row 07 of all 46 `dev_docs/languages/<lang>.md`. I re-read the shared code (`locals.rs`, `scope.rs`, `intra_file.rs`, `cross_file.rs:2380-2445`, `cross_file.rs:4110-4140`). New fixtures were run with `target/release/cgg` 0.11.0 under `scratchpad/parity/matrix-07/<dir>/` and are cited as `fx:<dir>`. Inputs and results:

| fixture | input (abridged) | result |
| --- | --- | --- |
| `fx:py` | `def comp(xs): return [helper() for helper in xs]`; `for run in []: pass` then `run()`; `lambda helper: helper()` | comp→`a.helper` **high** (wrong); loop→`a.run` **high** (wrong); lambda→`shadowed-by-local` |
| `fx:py2` | `outer` nests the only `inner`; `other()` calls `inner()`; `lambda inner: inner()` | other→`a.outer.inner` **high**; lam→`a.outer.inner` **high** (both wrong) |
| `fx:js2` | `function outer(){function inner(){} inner();}`; `(inner) => inner()`; `function param(inner){inner();}` | lam→nested `inner` **high** (wrong); param→`shadowed-by-local` |
| `fx:ts` | `import {helper}`; `function p(helper){helper();}`; lone nested `inner` called from `outer3` | p→imported `helper` **high**; outer3→`inner` (nested in outer1) **high** (both wrong) |
| `fx:rs2` | `fn outer(){fn inner(){} inner();}`; `fn other(){inner();}`; closure `\|inner: fn()\| inner()` | other→`crate::inner` **high**; closure→`crate::inner` **high** (both wrong) |
| `fx:kt`, `fx:kt2` | `fun shad(target: () -> Unit){target()}`; `forEach { target -> target() }`; lone nested `inner` called from `other` | both `shadowed-by-local`; other→nested `inner` **high** (wrong) |
| `fx:sw2` | the same lone-nested shape in Swift | other→nested `inner` **high** (wrong) |
| `fx:hs2` | `one x = go x where go y = y`; `two x = go x` | two→`A.go` (one's `where`) **high** (wrong) |
| `fx:ml` | `let shadow () = let f = 3 in f`; `let app2 = fun f -> f 1`; `let app3 f = f 1` | app2→nested `f` **high** (wrong); app3→`shadowed-by-local` |
| `fx:go2` | `func cb(){}`; `func run(cb func()){cb()}` | run→`t.cb` **high** (wrong) |
| `fx:cs2` | `class B { void Work(){} void F(System.Action Work){ Work(); } }` | F→`B.Work` **high** (wrong) |
| `fx:fs2` | `let g x = x`; `let apply g = g 1` | apply→`g` **high** intra-file (wrong) |

## Shared machinery (applies to every row unless the row says otherwise)

| mechanism | behaviour | evidence |
| --- | --- | --- |
| Locals table | Generic tree walk with a per-language `Spec` of binders, function scopes, blocks and patterns. Only 10 specs exist: `javascript`/`typescript` (shared `JS_SPEC`), `python`, `lua`, `scala`, `kotlin`, `swift`, `rust`, `dart`, `ocaml`, `zig`. Every other language gets `None` and an empty table | `locals.rs:380-393` |
| Minified guard | No locals are collected in a minified file (a line over 2,000 bytes) | CLAUDE.md; `locals.rs:406` comment |
| Import names are never shadowed by a bare-call local | `shadows_bare_call` returns false when the name is bound by any import of the file. This is deliberate ("a component's prop that is the bound action creator it imports"). A parameter named like an import therefore keeps the import reading, at whatever confidence the import gives | `scope.rs:22-25, 83, 122`; `fx:ts` |
| A local that *is* its import | `const m = require('./m')` inside a function is not a shadow | `scope.rs:35-57` |
| Shadow filter (intra-file) | Candidates outside the binding's function are dropped. A candidate declared between the declaration and the binding (`const f = () => …`) survives. For a non-parameter local, any *nested* definition anywhere in the file also survives ("a local may hold a closure a factory returns"). "Parameter" is detected as `fs == ls`: the binding's scope starts where its innermost enclosing *definition* starts | `intra_file.rs:275-306` |
| Shadow refusal (cross-file) | A shadowed bare call never reaches another file. It is recorded `shadowed-by-local` only when some same-named callable exists; otherwise it is dropped. A typed local enters its type's call operator (step 7) | `cross_file.rs:2388-2443` |
| Lexical narrowing | Allowlist: python, javascript, typescript, lua, ocaml, fsharp, scala, kotlin, swift, rust, dart, haskell, julia. JS/TS only when locals were read. Applies to a bare call (Python also to `dot_call`). **Runs only when `candidates.len() > 1`**. A Kotlin constructor span is not a lexical scope | `intra_file.rs:119-137, 360-368, 373-377` |
| Positional shadowing | Among the narrowed candidates, the last one starting at or before the site wins. OCaml, F#, Lua only | `intra_file.rs:400-416` |
| Nested-in-caller always reachable | A definition nested inside the calling function passes `bare_call_reaches` in every language | `cross_file.rs:4120-4128` |
| Package scope (Perl) | A bare call keeps only candidates of the caller's package; `main` equals the empty package | `intra_file.rs:329-347` |
| Conditional definitions | `DefRecord::conditional` is set only by the PHP plugin. `PhpIndex::one_copy` prefers the unconditional copy | `php.rs:159`; `libscope.rs:2023-2029, 2114` |
| `not_def` | A non-recursive binding is not its own callee. Set non-trivially only by `ocaml.rs`; every other plugin writes `None` | `ocaml.rs:248-257`; grep `not_def` in `crates/cgg-lang/src/plugins/` |

## Legend

✅ full · 🟡 partial · ⬜ none (the concept exists in the language but cgg does not model it) · — n/a (the concept does not exist in the language, or the language has no construct it would apply to)

Columns:

- **Locals**: a `FileFacts::locals` spec, or an equivalent plugin-side binder model.
- **Shadow**: a bare call through a parameter or local is not bound to a same-named function (`shadowed-by-local`).
- **RecvHead**: a receiver headed by a local is not read as an import, package or module.
- **DeclOrder**: a declaration is in scope only after its initializer.
- **Lexical**: lexical narrowing of nested definitions.
- **Positional**: the last same-scope binding before the site wins.
- **NonRec**: a non-recursive binding does not see itself.
- **Container**: a bare call is confined to its enclosing package, module, contract or object within one file.
- **ImportShadow**: a local named like an imported name hides the import.
- **CondDefs**: alternative or conditional definitions are recognised (one binds, or the unconditional one is preferred).

## The matrix

| language | Locals | Shadow | RecvHead | DeclOrder | Lexical | Positional | NonRec | Container | ImportShadow | CondDefs | notes (most important gap) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rust | ✅ `RUST_SPEC` | 🟡 closure params miss (`fx:rs2`) | ✅ | ✅ (`let`/`if let`/`for` tests) | 🟡 lone nested `fn` leaks file-wide (`fx:rs2`) | ⬜ | — | — | ⬜ (scope.rs:122) | ⬜ `#[cfg]` twins both bind medium | A lone nested `fn` is bound from any function at **high** |
| python | ✅ `PY_SPEC` | 🟡 comprehension vars not binders; lambda params miss (`fx:py`, `fx:py2`) | ✅ heads.rs:678 | 🟡 `Decl` ok; `for` var does not leak past the loop as Python does (`fx:py`) | 🟡 also `dot_call`; lone nested def leaks (`fx:py2`) | ⬜ (`run(); run = 3` binds) | — | — | ⬜ | ⬜ `if/else` defs both bind medium | Comprehension and loop variables bind the module function at **high** |
| kivy | — | — | — | — | — | — | — | — (rule/widget stacks bound spans only, kivy.rs:157-193) | — | — | Nothing to model; rule nesting only attributes spans |
| javascript | ✅ `JS_SPEC` (function bodies only) | 🟡 arrow params miss (`fx:js2`); imports never shadowed | ✅ | ✅ | 🟡 not in minified files; lone nested def leaks (`fx:js2`) | ⬜ | — | — | ⬜ (deliberate, scope.rs:22-25) | — | Arrow parameter named like a nested function elsewhere binds at **high** |
| typescript | ✅ `JS_SPEC` | 🟡 parameter named like an import binds the import at **high** (`fx:ts`) | ✅ cross_file.rs:5389 | ✅ | 🟡 lone nested def leaks at **high** (`fx:ts`) | ⬜ | — | — | ⬜ | ⬜ `if (c) function f` extracted, untagged | Parameters never shadow imported names |
| go | 🟡 import-name shadow spans only (go.rs:553-575, 647-691) | ⬜ `run(cb func()){cb()}` → `t.cb` **high** (`fx:go2`) | 🟡 import heads only (`go_shadow`, modules.rs:1582, 2011) | 🟡 `:=` shadow to end of block (import names only) | ⬜ local `func` literals share the package QN | ⬜ | — | — | ✅ the only language that has it | ⬜ build-tag twins both bind | No `locals` spec: params and range vars bind package functions at **high** |
| java | ⬜ | — (a local cannot be called bare) | ⬜ unverified | — | — (no nested functions; local classes flattened) | — | — | — | — | — | Receiver head that is a local is not distinguished from a class (unverified) |
| kotlin | ✅ `KOTLIN_SPEC` | ✅ incl. lambda params (`fx:kt`) | ✅ | ✅ (`val now = now()` test) | 🟡 lone nested `fun` leaks (`fx:kt2`) | ⬜ | — | — | ⬜ | — | Lone nested `fun` bound from other functions at **high** |
| c | ⬜ | ⬜ fn-pointer param/local binds at **high** (c doc f7) | — | ⬜ | — | ⬜ | — | ⬜ `static` file scope not modelled | — | 🟡 cross-file `c_include_copies` binds once; same-file refused `ambiguous-in-file` | Function-pointer parameters and locals produce false **high** edges |
| cpp | ⬜ (per-site local *types* only, cpp.rs:555) | ⬜ `handler()` param → global **high** (cpp doc f11) | ⬜ unverified | ⬜ | ⬜ lambdas in locals ambiguous with globals | ⬜ | — | 🟡 anonymous namespace per TU (cpp.rs:1388-1393) | ⬜ | ⬜ `#if/#else` twins both bind medium | No locals: parameter/local lambdas bind same-named functions at **high** |
| csharp | ⬜ (per-function local *types*, type_hints.rs:785) | ⬜ delegate param → `B.Work` **high** (`fx:cs2`) | ⬜ | ⬜ | ⬜ local functions owned by the type, same QN | — | — | — | ⬜ | 🟡 `#if` re-parse keeps a clean/first branch (csharp.rs:683-716) | Delegate parameters shadowing members give false **high** edges |
| bash | — | — | — | — | — (nested functions global once defined) | ⬜ redefinitions both bind | — | — | — | ⬜ both branches bind medium | Last-definition-wins redefinition is not modelled |
| ruby | 🟡 plugin-side (ruby.rs:71-251) | 🟡 a local suppresses the paren-less record; resolver reason never fires | ⬜ | ✅ Ruby's parser rule mirrored | — (nested `def` is a method) | ⬜ | — | — | — | ⬜ both branches bind (ruby doc p8) | No `FileFacts::locals`, so receiver-head-is-local never fires |
| swift | ✅ `SWIFT_SPEC` | ✅ | ✅ (moot per swift doc) | ✅ | 🟡 lone nested `func` leaks (`fx:sw2`) | ⬜ | — | — | — | ⬜ `#if` branches both bind medium | Lone nested `func` bound from other functions at **high** |
| lua | ✅ `LUA_SPEC` | ✅ | ✅ | ✅ | 🟡 lone nested def leaks (shared rule) | 🟡 use-before-`local` binds the later local at **high** (lua doc p3) | — | — | — | ⬜ | Use before a `local function` in the same function mis-binds at **high** |
| php | — (`$` variables) | — | — | — | — (nested functions are global) | — | — | 🟡 namespaces by byte; braced form broken (libscope.rs:1908-1914) | — | ✅ `DefRecord::conditional` (php.rs:159) | `$fn()` variable calls become `<external>::$fn` |
| dart | ✅ `DART_SPEC` | 🟡 local fn / closure var calls go external `dart:core` | ✅ cross_file.rs:5429 | ✅ | 🟡 (local functions are not callables) | ⬜ | — | — | ⬜ | — | Calls to local functions and closures misfiled as external |
| scala | ✅ `SCALA_SPEC` | ✅ | ✅ heads.rs:676-678; member `val`s shadow packages | ✅ | 🟡 lone nested def leaks (shared rule) | ⬜ | — | ⬜ bare call to another object in the same file binds **high** | ⬜ | — | Cross-object bare calls in one file bind at **high** |
| hcl | — (no user functions) | — | — | — | — | — | — | ⬜ a call binds to a block whose last label matches, **high** | — | — | Function call bound to a same-named resource block (always wrong) |
| zig | ✅ `ZIG_SPEC` | ✅ | ✅ cross_file.rs:5456 | ✅ (zig doc p2) | ⬜ not in `lexically_scoped`; container members `ambiguous-in-file` | ⬜ | — | ⬜ containers do not narrow bare calls | — | ⬜ comptime branches both kept | Bare call inside a container not narrowed to that container |
| objc | ⬜ | — (C calls not recorded) | — | — | — | — | — | — | — | ⬜ | No locals; C call sites are not recorded at all |
| r | ⬜ | ⬜ param → sourced `target` **high** (r doc f8c) | — | ⬜ | ⬜ nested `inner` reached from outside **high** | ⬜ | — | — | ⬜ | ⬜ both branches kept | Not lexically scoped, despite R closures being lexical |
| groovy | ⬜ | ⬜ closure vars → `<external>` | — | ⬜ | — (closures are not callables) | ⬜ | — | ⬜ no class scoping of bare calls | — | — | No locals; closure-variable calls misfiled as external |
| julia | ⬜ | ⬜ param → `M.helper` **high** (julia doc p6) | — | ⬜ | 🟡 (shared rule) | ⬜ | — | ⬜ module boundaries ignored | — | ⬜ | Parameters do not shadow, giving false **high** edges |
| perl | ⬜ | — (`$f->()` form) | — | — | ⬜ `my sub` recorded as a package sub | ⬜ | — | 🟡 package scoping ✅; `package X { }` block leaks | — | ⬜ two `sub dup` ambiguous | Block-form packages leak to the rest of the file |
| elixir | ⬜ | — (`f.()` required) | — | — | — | — | — | 🟡 cross-file module scoping; same-file modules ambiguous | — | ⬜ untagged | Two modules in one file defining the same name are ambiguous |
| erlang | ⬜ | — (variables are capitalised) | — | — | — | — | — | — (module = file) | — | 🟡 `-ifdef` branches merged as clauses; binds the first only | `-else` branch and its callees become unreachable |
| fortran | ⬜ | ⬜ dummy procedures unmodelled; arrays read as calls | — | — | ⬜ host-contained procedures not preferred | — | — | ⬜ `use, only:` and renames ignored | ⬜ | ⬜ unverified | Case-sensitive name matching in a case-insensitive language |
| clojure | ⬜ | ⬜ `(let [inner …] (inner))` → top-level **high** | — | ⬜ | ⬜ (nested defs not extracted) | ⬜ later `defn` does not shadow | — | — | — | ⬜ | `let`-bound fns bind same-named vars at **high** |
| haskell | ⬜ | ⬜ `shadow f = f 1` → `A.Ops.f` **high** | — | — | 🟡 `where`/`let` narrowing; lone one leaks (`fx:hs2`) | — (bindings recursive) | — | — | ⬜ | ⬜ CPP `#else` dropped by grammar | Parameters do not shadow, giving false **high** edges |
| ocaml | ✅ `OCAML_SPEC` | 🟡 `fun f ->` params keep nested `f`s elsewhere (`fx:ml`) | — (module heads capitalised) | ✅ via `not_def` | 🟡 (shared rule) | ✅ | ✅ simple names only (ocaml.rs:95-98) | — | — | — | Lambda parameter binds another function's local `let` at **high** |
| powershell | ⬜ | — (`& $sb` form) | — | — | ⬜ nested `Helper` ambiguous | ⬜ redefinition ambiguous | — | — | — | ⬜ ambiguous | Scope modifiers (`script:` etc.) kept in names, so calls miss |
| solidity | ⬜ | ⬜ function-typed param → `S.helper` **high** | — | ⬜ | — | — | — | ⬜ bare call binds another contract in the same file **high** | — | — | Cross-contract same-file bare calls bind at **high** |
| fsharp | ⬜ | ⬜ `let apply g = g 1` → `g` **high** (`fx:fs2`) | — | ⬜ | 🟡 (shared rule) | ✅ | ⬜ no `not_def` (F# `let` is non-recursive) | ⬜ modules are not scopes | — | 🟡 positional rule picks the last `#if` branch | No locals spec although OCaml (same family) has one |
| starlark | ⬜ | ⬜ param → loaded `dup` **high** | — | ⬜ | ⬜ nested def visible file-wide **high** | ⬜ | — | — | ⬜ | ⬜ | Same rules as Python but no `PY_SPEC`-style table |
| cmake | — | — | — | — | — (functions global once defined) | ⬜ redefinitions ambiguous | — | — | — | ⬜ ambiguous | Last-definition-wins not modelled |
| nix | ⬜ | ⬜ `shadow = greet: greet "x"` → **high** | — | ⬜ | ⬜ two `let inner`s bind four edges | ⬜ | — | ⬜ `with` scope unmodelled | — | — | Lambda parameters and `let` scopes not modelled |
| verilog | — | — | — | — | — | — | — | 🟡 module scope for qualified names | — | ⬜ `` `ifdef `` twins ambiguous | `` `ifdef `` alternatives draw no edge |
| vhdl | — | — | — | — | — | — | — | 🟡 only `architecture` pushes a scope (vhdl.rs:124-126) | — | ⬜ `generate` bodies not distinguished | Packages, processes, subprograms are not scopes |
| asm | — | — | — | — | — | — | — | — | — | ⬜ `.if`/`.ifdef` labels all callables | Duplicate same-file labels `ambiguous-in-file` |
| smithy | — | — | — | — | — | — | — | — (same file first, then global) | — | — | Nothing missing for the language |
| proto | — | — | — | — | ⬜ nested message scope unmodelled | — | — | ⬜ | — | — | Nested type references bind same-named types anywhere (wrong edges) |
| graphql | — | — | — | — | — | — | — | — | — | — | Nothing to model |
| openapi | — | — | — | — | — | — | — | — | — | — | Same-file duplicate names fall through to global medium |
| asyncapi | — | — | — | — | — | — | — | — | — | — | `$ref`s outside every def are unattributed |
| lean | 🟡 declaration binders only (lean.rs:396-429) | 🟡 `let`/`fun`/`match` binders → external, no wrong edge | — | — | — (nested defs not extracted) | — | — | 🟡 namespace prefixes; `section variable` not modelled | — | — | `fun`/`let`/`have` binders untracked |

## Parity gaps

Ranked by impact: wrong `high` edges first, then the breadth of languages affected.

1. **Lexical narrowing skips a site with only one candidate.** This affects all 13 languages in `lexically_scoped`. The narrowing block requires `candidates.len() > 1` (`intra_file.rs:367`). A lone nested function is therefore bound from *any* function in the file at **high**, although these languages make it invisible outside its parent. Haskell `two`→`one`'s `where go`, Kotlin, Swift, Rust, Python, JS and TS all reproduce it (`fx:hs2`, `fx:kt2`, `fx:sw2`, `fx:rs2`, `fx:py2`, `fx:js2`, `fx:ts`). The visibility filter at `intra_file.rs:384-388` already computes the right answer. Running it at one candidate, and refusing when nothing is visible, would fix it. Effort **S**.
2. **Lambda and closure parameters are treated as plain locals.** The parameter test `is_param = fs == ls` (`intra_file.rs:296`) is true only when the binding's scope is a *definition*. For a `fun f ->`, `lambda x:`, `(x) =>` or `|x|` parameter the innermost definition is the enclosing named function, so `is_param` is false. Every nested definition elsewhere in the file then stays a candidate (`intra_file.rs:301`). Confirmed for OCaml (`fx:ml`), Python (`fx:py2`), JS (`fx:js2`) and Rust (`fx:rs2`). Kotlin passed only because no nested candidate existed (`fx:kt`). This is the root cause of the OCaml doc's "unverified" bug. Effort **S**: let the `Spec` mark closure-function nodes so their params count as params.
3. **Go has no `locals` spec; Rust, Kotlin and Zig do.** `run(cb func()){cb()}` binds `t.cb` at **high** (`fx:go2`). Range variables do too (go doc f10). Go already has binder knowledge for import shadowing (`go.rs:553-575, 647-691`). Effort **M**: add `GO_SPEC` (parameter_declaration, short_var_declaration, range_clause, func_literal).
4. **C# has no `locals` spec; Kotlin and Scala do.** A delegate parameter shadowing a member binds at **high** (`fx:cs2`). C# local functions also share the type's QN, so lexical narrowing cannot help (csharp doc §07). Effort **M**.
5. **F# has no locals spec and no `not_def`; OCaml has both.** `let apply g = g 1` → `g` at **high** (`fx:fs2`). F# already has lexical and positional rules (`intra_file.rs:126, 405`). Effort **M**: port `OCAML_SPEC` and `ocaml.rs:248-257`.
6. **C and C++ have no locals.** Function-pointer parameters and locals, and lambdas held in locals, bind same-named functions at **high** (c doc f7, cpp doc f11). Effort **M** (one spec for both grammars).
7. **Haskell, Julia, R, Nix, Clojure, Starlark and Solidity have no locals.** Parameter shadowing yields **high** wrong edges in each, verified in the language docs: haskell f2, julia p6, r f8c, nix f1, clojure p1, starlark t2, solidity f9. Haskell and Julia are already lexically scoped, so only the binder table is missing. Starlark is Python-shaped and could reuse most of `PY_SPEC`. Effort **M** each (S for Starlark).
8. **A parameter never shadows an imported name.** This holds in every locals language except Go. `scope.rs:122` excludes import-bound names from `shadows_bare_call` on purpose (the React-prop pattern, `scope.rs:22-25`). In TS `function p(helper){helper();}` with `import {helper}` binds the import at **high** (`fx:ts`). Go's `go_shadow` handles the same case correctly. At minimum the binding should drop to `medium`. Effort **M** (policy per language).
9. **Python comprehension variables and loop-variable leak.** `[helper() for helper in xs]` and a post-loop `run()` bind module functions at **high** (`fx:py`). `PY_SPEC` has no comprehension binder, and the `for` binder uses `Own` scope (`locals.rs:183-204`). Effort **S**.
10. **Same-file container confinement.** Perl confines a bare call to its package (`intra_file.rs:329-347`). Scala bare calls across objects and Solidity bare calls across contracts bind at **high** (scala doc f5, solidity doc f3). Elixir same-file modules are ambiguous (elixir doc f6). Zig container members are ambiguous (zig doc p4), and Zig is not in `lexically_scoped`. Effort **M**: generalise the Perl filter to an owner-path filter.
11. **Positional shadowing is OCaml/F#/Lua only.** The concept also exists in Rust `let` shadowing of closures (rust doc f3), in last-definition-wins redefinitions (Bash, PowerShell, CMake, Clojure `defn`, which bind both or go ambiguous), and in Python `run(); run = 3`. Lua's own rule mis-binds a use that precedes a `local function` (lua doc p3), because narrowing does not check that the nested def starts before the site. Effort **M**.
12. **Conditional definitions are PHP only.** C refuses same-file `#ifdef` twins intra-file but binds the last one cross-file at high, which is inconsistent (c doc f9). Erlang merges `-ifdef` branches as clauses and binds the first, so the `-else` callees go dead (erlang doc f8). C++, Swift, Rust `#[cfg]`, Go build tags, Bash, Python, R and Verilog keep both copies (medium or ambiguous). Effort **M**: tag preprocessor and `cfg` alternatives with `DefRecord::conditional` and reuse `one_copy`.
13. **Dart local-function and closure-variable calls go to external `dart:core`** instead of `shadowed-by-local` (dart doc f1, f11). Effort **S**.
14. **Ruby, Lean and Groovy have binder knowledge but no `FileFacts::locals`.** Ruby tracks locals in the plugin (`ruby.rs:71-251`), and Lean tracks declaration binders (`lean.rs:396-429`). Neither feeds `FileLocals`, so the receiver-head check and the `shadowed-by-local` reason never fire. Groovy closure variables become `<external>::l`. Effort **S** for Ruby and Lean (emit `LocalBinding`s), **M** for Groovy.
15. **HCL function calls bind same-named resource blocks at high** (hcl doc fixture c). A call can never target a block. Effort **S**: exclude block defs from call candidates.
16. **Fortran matches names case-sensitively** (fortran doc h/a.f90), and its `use, only:` renames do not scope (fixture g). Effort **S**.
17. **Proto nested-message scope resolution is absent**, so nested type refs bind any same-named type (proto doc fixture c). Effort **M**.
18. **PowerShell scope modifiers** (`script:Foo`) stay in the function name, so calls miss (powershell doc f14). Effort **S**.
19. **Perl `package X { }` block scope leaks**, and `my sub` is package-global (perl doc f10). Effort **S**.

## Doc corrections

1. **typescript.md §07**, "A parameter does not shadow an imported name … Unverified why". The cause is deliberate. `FileLocals::shadows_bare_call` rejects any name the file imports (`crates/cgg-resolve/src/scope.rs:22-25, 122`). It applies to every language with a locals spec, not only TS. CLAUDE.md's "Locals and parameters shadow import names" holds for module resolution and receiver heads (`cross_file.rs:5389`), not for a bare call.
2. **typescript.md §07**, "outer3 … bound both at medium instead of refusing". This is true only with two nested `inner`s. With one nested `inner`, `outer3 -> inner` binds at **high** (`fx:ts`), because narrowing needs more than one candidate (`intra_file.rs:367`).
3. **javascript.md, kotlin.md, swift.md, ocaml.md** score row 07 as **full**. Gap 1 applies to all four: a lone nested definition is visible file-wide at high (`fx:js2`, `fx:kt2`, `fx:sw2`). Gap 2 also applies to JS and OCaml (`fx:js2`, `fx:ml`). Each is **partial**.
4. **ocaml.md §07**, "Root cause beyond this observation unverified". Root cause: `is_param = fs == ls` (`intra_file.rs:296`) is false for a `fun` parameter, whose scope is not a definition. Nested `let`s in other functions then survive via `intra_file.rs:301`. Reproduced with `fx:ml` (`app2` → nested `f` high; `app3` with a named parameter → `shadowed-by-local`).
5. **rust.md §07**, "Lexical narrowing of nested definitions: `rust` is in the lexical-narrowing allowlist". This is true, but a lone nested `fn` is still bound from other functions (`fx:rs2`: `crate::other -> crate::inner` high).
6. **fsharp.md §07** reports the parameter-shadowing wrong edge as **medium** (cross-file). In the same file it is **high** intra-file (`fx:fs2`).
7. **bash.md, starlark.md, csharp.md, cmake.md, julia.md** (and CLAUDE.md, which must not be edited here) list the lexical-narrowing languages without Python. The code includes `python` (`intra_file.rs:121`), and Python additionally narrows `dot_call` refs (`intra_file.rs:361`).
8. **ruby.md §07** says `locals.rs:380-391` lists "JS/TS, Python, … Zig and others". There are no others. The `spec` match is exactly 10 arms (11 language ids, JS and TS shared) and then `_ => return None` (`locals.rs:380-393`).
9. **python.md §07**: no correction. Comprehension and loop-leak are re-confirmed (`fx:py`). Add that a `lambda` parameter named like a nested def elsewhere binds that def at high (`fx:py2`).
10. **haskell.md §07** says lexical narrowing is "supported" with fixture f3 (two `go`s). With a single `where go`, `two` binds it at **high** (`fx:hs2`).
