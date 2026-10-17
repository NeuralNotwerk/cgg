# haskell

The Haskell plugin is `HaskellPlugin` in `crates/cgg-lang/src/plugins/haskell.rs` (294 lines including tests). It is registered at `crates/cgg-lang/src/plugins.rs:153`. It is one small recursive walker over the `tree-sitter-haskell` 0.23 grammar from crates.io (`Cargo.toml:107`, `crates/cgg-lang/Cargo.toml:50`), so nothing is vendored. The walker reacts to only four node kinds (`haskell.rs:66-90`):

- `header`: the module name.
- `import`: import records.
- `function` and `bind`: definitions.
- `apply`: call sites, recorded only when the head is a plain `variable`.

Other properties of the plugin:

- It ignores its `ExtractCtx` (`_ctx`, `haskell.rs:27`).
- It has no `signals()` override, so every `PluginSignals` field is false (`crates/cgg-lang/src/lib.rs:304-306`, default).
- It records no locals, visibility, exports, attributes, value refs, dynamic uses, class/instance records or arity.

The `LanguagePlugin` trait has no `resolver_kind` method in this tree. Haskell behaviour is driven by language-id checks in the resolver:

- lexical narrowing of nested definitions (`crates/cgg-resolve/src/intra_file.rs:133`)
- the clause-language rules (`intra_file.rs:859`, `intra_file.rs:897`)
- the `import qualified` and `import` arms of the cross-file import table (`crates/cgg-resolve/src/cross_file.rs:1957-1970`, `cross_file.rs:2064`)
- the unqualified-prefix lookup in step 1b and the scoped lookup in step 1c (`cross_file.rs:5944-5975`, `cross_file.rs:5988-6010`)

Verification: the fixtures cited below are under `<scratch>/parity/haskell/` (f1–f6). Each was run with `target/release/cgg` 0.11.0 and `-t json -o out.json`, then dumped with `dump2.py`, which lists callables, edges and the audit's unresolved/stdlib/external buckets.

## 01. Detection & parsing

- **Extensions `.hs` and `.lhs`.** These are the plugin's `extensions()` (`haskell.rs:15-17`). Test `plugin_loads` (`haskell.rs:264`). Fixture f3: `app/Lib.hs` shows `detected_via: extension:.hs` and `Lit.lhs` shows `extension:.lhs`.
- **Shebangs `runhaskell` and `runghc`.** These are `shebangs()` (`haskell.rs:18-20`). Fixture f3: an extension-less `script` starting `#!/usr/bin/env runghc` was analyzed (`detected_via: shebang:runghc`), and its `main → greet` edge resolved `high`.
- **`stack`/`cabal` script shebangs** (`#!/usr/bin/env stack`): not in the list, so not detected. This is unverified by fixture; the evidence is the absence from `haskell.rs:18-20`.
- **Content detection: none.** A grep for `haskell` or `hs` in `crates/cgg-lang/src/detect.rs` returns nothing.
- **`.hs-boot`, `.hsc`, `.chs`, `.cabal`, `package.yaml`, `stack.yaml`: not detected.** None is in `extensions()`. No manifest is read (§04).
- **Grammar.** One grammar, `tree_sitter_haskell::LANGUAGE` (`haskell.rs:21-23`). There is no per-file dialect, because `ts_language_for` keeps its default (`crates/cgg-lang/src/lib.rs:322`).
- **Literate Haskell is parsed as plain Haskell.** There is no un-literating step.
  - Fixture f3: `Lit.lhs` (bird tracks `>`) and `Bird.lhs` (`\begin{code}`) both report `parse_status: "error"`, yet their definitions and calls were recovered from the error tree: `Lit.litFun → Lit.litHelper` and `birdFun → birdHelper`, both `high`.
  - In `Bird.lhs` the `module Bird where` header was not recognised, so its names came out unqualified (`birdFun`, not `Bird.birdFun`).
  - The result depends on error recovery and is not a supported path.
- **Re-parse/fallback: none.** A syntax error is extracted from the recovered tree, and its audit shows `parse_status: "error"` (`crates/cgg/src/lib.rs:457`).
- **CPP (`{-# LANGUAGE CPP #-}`, `#if`/`#else`/`#endif`).** No preprocessor handling. The grammar keeps the first branch and drops the `#else` branch.
  - Fixture f5 `Cpp.hs`: only `cond = one 1` (the `#if` arm, line 5) became a callable.
  - The `#else` arm `cond = two 2` was lost, so `two` was reported dead (a false positive).
  - The same happens in fixture f2 (`A.Ops.cond` at line 50 only).
- **Skip conditions (generic).**
  - Depth: a tree deeper than `MAX_TREE_DEPTH` = 4000 is skipped as `TooDeep` (`crates/cgg/src/lib.rs:155,451`).
  - Minified: the 2,000-byte line check only gates locals, which Haskell does not produce (§07).
  - Long lines: there is no `max_line_bytes` override (`lib.rs:313` default `None`).
- **Framework-detection prefix test.** `crates/cgg/tests/detect_prefixes.rs:66` writes `import <prefix>` into a `.hs` file for every Haskell rule and asserts detection.

## 02. Callables extracted

- **Top-level function equations, `work x = helper x`.** A `function` node becomes a `DefRecord`, `DefVariant::FreeFunction`, named after the first `variable` child (`haskell.rs:169-204`). Tests `free_functions_and_call` (`haskell.rs:272`) and `definitions_are_qualified_by_their_module` (`haskell.rs:247`).
- **Nullary bindings, `nullary = 42` and `main = do …`.** These are `bind` nodes and are extracted the same way (`haskell.rs:80`). Fixture f1: `Data.Thing.nullary`, `Main.main`.
- **Qualified name `Module.name`.**
  - It is formed in `qn()` (`haskell.rs:54-64`) from the `module` child of `header` (`haskell.rs:104-124`). Example: `Data.Thing.work`.
  - Files without a `module … where` header get bare names (test `free_functions_and_call`; fixture f3 `script`: `main`, `greet`).
  - CHANGELOG.md:5860 records the fix that made module qualification work at all.
- **Multi-equation functions.** `helper 0 = 0; helper n = …` gives **one node per equation** under one qualified name (fixture f1: `Data.Thing.helper` at lines 31 and 32). The resolver treats them as clauses of one function (§06).
- **Guards, `where`, `case`.** A guarded equation is one node (fixture f1 `guarded`).
- **Local definitions in `where` and `let`.**
  - `whereUser x = inner x where inner y = …` and `let go z = … in go x` are extracted as their own nodes.
  - They are qualified at **module level**, with no enclosing-function owner: `Data.Thing.inner` and `Data.Thing.go` (fixture f1); `Lib.go` twice, once per enclosing function (fixture f3).
  - Lexical narrowing still binds each call to its own local (§07).
- **Type-class instance methods.**
  - `instance HasArea Shape where areaOf (Circle r) = …` is extracted as a plain module-level function `Data.Thing.areaOf`, with no owner, no instance type and no class link (fixture f1).
  - Two instances of one class method (`instance Pretty Int`, `instance Pretty Bool`) both become `A.Ops.pretty` and are treated as **clauses of one function** (fixture f2, fixture f5 `Inst.sh`).
- **Class default methods.** `prettyDefault x = pretty x` inside `class` is extracted as `A.Ops.prettyDefault` (fixture f2).
- **`kind` is always `function`.** There are no `method`, `constructor` or `class` kinds.
- **`signature_hint`** is the whole equation text, `super::extract_signature(self.text(node))` (`haskell.rs:199`). The `name :: Type` signature is not used.
- **SPURIOUS callables from type variables in function types.**
  - The grammar also uses the node kind `function` for the type `a -> b`. Its first `variable` child is a type variable, so the walker mints a callable named after it.
  - Fixture f1: `Data.Thing.a` came from the class method signature `areaOf :: a -> Double`.
  - Fixture f2: `(|>) :: a -> (a -> b) -> b` produced `B.Util.a`, `B.Util.b`, `B.Util.a`, and the class `Pretty a` produced two `A.Ops.a`.
  - Fixture f5: `Inst.a`.
  - These show up as dead-code findings.
  - Monomorphic signatures (`Int -> Int`) produce nothing, because `Int` is a `name`, not a `variable`.
- **SPURIOUS callables from `do`-block binds.** `args <- getArgs` is a `bind` node, so it became the callable `Main.args` (fixture f1, line 9).
- **NOT extracted:**
  - Infix operator definitions `a <+> b = …` and `x |> g = g x` (fixture f1/f2: no `<+>` or `|>` node). The operator is not a `variable`.
  - Prefix-form operator definitions `(<+>) a b = …`: unverified, but the head would be an `operator`, not a `variable`, so presumably not extracted either.
  - Data constructors (`Circle`, `Lit`), record fields and selectors (`fieldA`, `unWrap`), newtypes.
  - Class method declarations, which exist only as signatures. A class method with no default has no node at all.
  - Pattern synonyms (`pattern P = 1`; fixture f2, no node).
  - `foreign import` declarations (`c_sin`; fixture f2, no node).
  - Type families, type synonyms, GADT constructors (fixture f4: `Lit :: Int -> Expr Int` gives no node).
  - Lambdas `\y -> …`. Their calls are attributed to the enclosing definition (fixture f1: `lambdaUser → helper`, `high`).
  - Template Haskell-generated definitions (`makeLenses ''Rec`, `$(deriveJSON …)`; §08).
  - Pattern bindings `(a, b) = …`: no `variable` direct child. Unverified.

## 03. Call-site forms

- **Bare application `f x`, `f x y`.** An `apply` whose `child(0)` is a `variable` is recorded as a `RefRecord` with an empty `receiver_hint` (`haskell.rs:206-222`). The nested `apply(apply(f,x),y)` shape means that only the innermost apply, which carries the head, is recorded. Fixture f1: `work x = helper x` gives `work → helper`, `high`.
- **Calls nested in arguments and parentheses.** `recordSel = work (helper 3)` records both (fixture f1).
- **Calls inside `do`, `case`, guards, `where`, `let`, `if` and lambdas are all recorded** (fixture f1: `guarded`, `caseUser`, `lambdaUser`, `letUser`, `whereUser`; fixture f3: `main = print (one 1) >> exitSuccess` gives `main → Lib.one`).
- **Right operand of `$` when it is itself an application.** `helper $ work x` records `work` (fixture f1 `dollar → work`) but NOT `helper` (§NOT below).
- **Arity is NOT recorded.** No `RefRecord::arity` is set (`haskell.rs:213-219`). The clause logic substitutes arity 0 for every Haskell equation (`intra_file.rs:897-900`).
- **NOT recorded (no edge, no unresolved record):**
  - Qualified calls `T.helper 2`, `Data.Thing.area (…)`, `M.lookup 1 m`, `B.Other.other x`, `Txt.pack …`. The head is a `qualified` node, not a `variable`, so the site is never emitted. Fixtures f1, f2, f4: none of these appears in edges or in any unresolved/external bucket. This is the single largest gap: idiomatic `import qualified … as M` code produces no edges.
  - Bare references with no argument, such as `doIt` as a `do` statement, `server = hello`, or `usesNullary = nullary + 1`. These are not `apply` nodes (fixture f1: no `main → doIt`, no `usesNullary → nullary`; fixture f6: no `server → hello`).
  - The left operand of an operator, `helper $ work x` (missing `dollar → helper`).
  - Composition `helper . work` (fixture f1 `composed`: no edges) and `mapM_ (print . one) xs` (fixture f3: no `mapper → one`).
  - Functions passed as values, `map helper xs` (fixture f1 `valueArg`: only `map` recorded, as stdlib).
  - Operator calls `a <+> b`, `x |> util`, `getLine >>= putStrLn`, sections `(<+> 1)`. No operator site is recorded (fixtures f1/f2/f3).
  - Backtick infix calls ``a `plusInner` b`` (fixture f1 `infixPlus`: no edge, so `plusInner` was reported dead).
  - Data-constructor application `Circle 1.0`. The head is a `constructor`, not a `variable` (fixture f1 `ctor`: no site).
  - Record field selection `fieldA r` IS recorded as a bare call but never binds, because fields are not callables. It lands in `external_calls` (fixture f2, line 46).
  - Record dot syntax `r.field`: unverified.
- **SPURIOUS reference sites from type applications.** `save :: Int -> m ()` in a class records a call `m` (fixture f4: `external_calls` line 12 `m`). The type application `m ()` is an `apply` with a `variable` head.
- **Calls through a parameter `paramCall g = g 2`.** Recorded as bare `g` and classed **external** (fixture f2: `external_calls 34 g`), not `shadowed-by-local`.
- **No value refs, callbacks or registrar arguments.** `extract` never reads `ExtractCtx::reference_sites` or registrar verbs (`haskell.rs:27`). `--reference-edges` therefore has nothing to add. The `scotty` rule's gap text states this (`crates/cgg-core/src/frameworks/rules.rs:5952`).
- **Constructors, explicit generics, paren-less and method-call forms:** not applicable as separate shapes. Haskell has no receiver syntax. Type applications `f @Int x` are unverified.

## 04. Imports & module resolution

- **`import M`, `import M (a, b)`, `import M hiding (x)`.**
  - Recorded as `ImportRecord { kind: "import", path: "M" }` (`haskell.rs:126-167`). The import list and `hiding` list are **ignored**: only the module name is kept.
  - Resolution pushes the module as an unqualified prefix and maps its last segment as an alias (`cross_file.rs:2064-2077`).
- **`import qualified M [as A]`.** Recorded with `kind: "import qualified"` and `alias` (`haskell.rs:137-158`). Test `import_is_recorded` (`haskell.rs:289`). Resolution maps the alias, or the full name when there is no alias, to the module (`cross_file.rs:1957-1970`). The mapping is never used, because qualified call sites are never emitted (§03).
- **`import M as A`** (unqualified with alias): recorded as `kind "import"` with an alias. The arm at `cross_file.rs:2064` does not read the alias. Unverified end to end.
- **Package-qualified imports `import "pkg" M` and `{-# SOURCE #-}` imports:** unverified. The plugin takes the first `module` child.
- **Step 1b, unqualified-prefix lookup.** A bare `f` in a file with `import Data.Thing` is tried as `Data.Thing.f` against `by_qn` (`cross_file.rs:5943-5970`).
  - The edge is `high` only when exactly one opened module declares the name and the path is certain.
  - Fixture f1: `main → Data.Thing.work` is `high` `cross-file:imports`.
  - Fixture f4: `useLookup → X.A.lookup` is `high`.
- **Step 1c, scoped by-simple lookup.** Restricted to files whose path matches an import prefix (`Text.Pandoc` → `*/Text/Pandoc.hs`), capped at 8 candidates (`cross_file.rs:2163-2200`, `5988-6010`). The comment names Haskell as the motivating case. There is a test with Haskell paths: `scoped_lookup_matches_the_eager_scan` (`cross_file.rs:7444`, fixture paths `/r/text/pandoc.hs`).
- **Name opened from two modules.** `import X.A; import X.B` and a bare `shared` that both define binds to the **first** listed module at `medium`, rather than being refused as ambiguous (fixture f4: `useShared → X.A.shared`, `medium`, mermaid `?`).
- **`hiding` is ignored.** `import B.Other hiding (other)` then `other x` still binds `B.Other.other` at **high** (fixture f2, `viaHidden`). This is a wrong edge.
- **Import lists are ignored.** `import B.Util (util)` plus `import B.Other` (which also defines `util`): `cond → B.Util.util` `medium` (fixture f2). The import list is not consulted to prefer or exclude.
- **External modules.** A name that resolves nowhere is classed `external` (`external_calls`, fixture f4 `notDefinedAnywhere`). There is no `heads.rs` external-head refusal: Haskell is not among `WorkspaceVocab`'s languages (no `haskell` in `crates/cgg-resolve/src/heads.rs`). There is no `not-in-imported-module` refusal either: Haskell is not in `modules.rs`.
- **`FileFacts::packages`: not populated.** No manifest (`.cabal`, `package.yaml`, `stack.yaml`, `cabal.project`) is read. `manifest.rs` has no Haskell reader.
- **Re-exports** (`module Lib (module Data.Thing)`): the export list is not read at all. Unverified whether `lookup_with_reexports` sees anything. It cannot, because no export records are produced.
- **Module export lists** (`module Lib (one, two) where`): ignored. They feed neither visibility nor dead-code roots (§12).

## 05. Type inference & receiver typing

- **Not applicable, in effect: no receiver syntax and no typing facts.** The plugin records no `local_types`, `arg_type`, `typed_receiver`, `scoped_types`, `type_spans` or `class_fields`.
- `type_hints::propagate_types_with_returns` has no Haskell opt-out (`crates/cgg-resolve/src/type_hints.rs:389` exempts only Erlang). It runs over `signature_hint`, which is the equation text, not the `::` signature. Nothing observable results: every Haskell site has an empty `receiver_hint`.
- `split_top_level` treats `x'` as a primed identifier "in Haskell-family languages" (`type_hints.rs:1720-1729`), a generic rule.
- **Type-class dispatch by instance type is not modelled** (§06).
- **No closure parameter typing, implicit receivers or weak receiver types.** None of `closure_types.rs`, `receiver.rs` or `settle_implicit_receivers` mentions Haskell (grep).

## 06. Inheritance, dispatch & overloading

- **Clause language.** `is_clause_language` includes `haskell` (`intra_file.rs:858-860`). Same-named definitions in one file are the equations of one function.
  - Intra-file, a call binds **once to the first equation**, at `high` (`clause_targets`, `intra_file.rs:891-935`). The arity is forced to 0 for Haskell (`intra_file.rs:897-900`).
  - Fixture f1: `helper n = helper (n - 1)` binds `helper@31` (the first equation), `high`.
  - CHANGELOG.md:1882-1890 describes the behaviour.
- **Cross-file calls to a multi-equation function bind the LAST equation at `medium`.** Fixture f1: `doIt → Data.Thing.helper@32` (the second equation), `medium`; fixture f5: `viaOther → Inst.sh@7`, `medium`. Three mechanisms combine:
  - Step 1b's `by_qn` lookup returns one callable per qualified name.
  - `first_clause_per_arity` is applied only for Elixir and Erlang (`cross_file.rs:5549`, `5600`).
  - Certainty fails because several callables carry the name.
  - Consequence: the first equation can be reported dead (fixture f1: `Data.Thing.areaOf@13`, the second instance equation, reported unreferenced; the edge went to the first).
- **`one_method` overload-set exemption excludes clause languages** (`cross_file.rs:4796`). Haskell same-name sets across files stay capped.
- **Type classes and instances: not modelled.**
  - Instances of one class method for different types are merged as clauses of one function.
  - Fixture f5: `useSh = sh True` binds `Inst.sh@5`, the `Int` instance, at **high**. That is wrong; the `Bool` instance is the target.
  - There are no class records, no `trait_impl_target`, no base types and no default-method linkage.
- **`--dynamic-dispatch`: no effect.** Fixture f4 gives identical edges with and without it. Haskell is not in `INHERITANCE_LANGS` (`crates/cgg-resolve/src/dispatch.rs:110`) and emits no `trait_impl_target`.
- **Arity overload narrowing: no.** Haskell is not in `OVERLOAD_ARITY_LANGS` (`type_hints.rs:1583`), and no arity is recorded.
- **Not applicable:** super/base calls, `member_of_calling_class`, `overrides_below`, extension methods, mixins.

## 07. Scoping & shadowing

- **Lexical narrowing of nested definitions: supported.** `haskell` is in the nested-scope language list (`intra_file.rs:115-135`; CHANGELOG.md:575-580). A bare call binds to the innermost visible nested definition.
  - Fixture f3: `one x = go x where go …` and `two x = go x where go …` bind `Lib.one → Lib.go@5` and `Lib.two → Lib.go@9`, each `high`.
  - Fixture f2: a `where`-local `go` binds its own equations, while a top-level `B.Util.go` exists elsewhere.
- **Positional shadowing:** not applied. That rule is for OCaml, F# and Lua only (CHANGELOG.md:579-580).
- **`FileFacts::locals`: not populated.** There is no Haskell spec in `crates/cgg-lang/src/locals.rs:380-394`. Consequences:
  - A parameter shadowing a top-level function binds to the top-level function. Fixture f2: `shadow f = f 1` binds `A.Ops.f` at **high**. That is wrong; `f` is the parameter.
  - A call through a parameter `g 2` is classed external instead of `shadowed-by-local` (fixture f2).
- **Conditional definitions:** none. CPP `#else` arms are dropped by the grammar (§01). `DefRecord::conditional` is never set.

## 08. Macros, preprocessor & codegen

- **CPP:** no handling. The first branch is kept and `#else` is lost (fixture f5, §01). No `macro:<name>` tagging.
- **Template Haskell splices** (`makeLenses ''Rec`, `$(deriveJSON defaultOptions ''Wrap)`) are recorded as ordinary calls to `makeLenses`/`deriveJSON` and classed external (fixture f2: `external_calls 56 makeLenses`, `58 deriveJSON`). Generated definitions (lenses, instances) do not exist in the graph.
- **Quasi-quotes** (`[r|hello|]`, Yesod's `[parseRoutes| … |]`) are opaque. Fixture f2 `quasi`: no sites. The `yesod` rule's gap names this (`rules.rs:5935`).
- **`name_mentions`: never populated** by the Haskell plugin (no assignment in `haskell.rs`).
- **Not applicable:** `macro_rules!` and `#define` expansion (Rust/C only).

## 09. FFI & cross-language links

- **`foreign import ccall "math.h sin" c_sin :: …`:** not extracted as a callable and not linked to C. `useFfi d = c_sin d` records `c_sin`, which is classed external (fixture f2: `external_calls 70 c_sin`). A grep for `foreign` in `crates/cgg-resolve/src/ffi.rs` returns nothing.
- **`foreign export ccall exported :: …`:** not recorded. The Haskell function `exported` is an ordinary node with no FFI attribute, so it is not an `ffi:export` dead-code root (fixture f2; `crates/cgg-resolve/src/deadcode/roots.rs:763-770` needs an FFI attribute).
- **Descriptor links (protobuf rpc → host):** none for Haskell. `descriptor.rs` has no Haskell host mapping (grep).
- **Markup code-behind:** not applicable.

## 10. Frameworks & entry points

Every rule whose `language` is `haskell` (`crates/cgg-core/src/frameworks/rules.rs:5905-6027`). Detection uses `detect` module prefixes matched against the file's imports (fixture f6 detected `servant`, `yesod`, `scotty` and `wai` from `import` lines). No multi-language rule lists Haskell (grep for `haskell` in rules.rs gives only these seven).

| rule id | kind | trigger | enumerates? |
| --- | --- | --- | --- |
| `servant` | Network | import `Servant` | no — `methods`/`attributes`/`registrars` empty; gap: handlers are positional in a `:<\|>`chain (`rules.rs:5905-5920`) |
| `yesod` | Network | import `Yesod` | no — gap: TH-generated `get<Route>R` names (`rules.rs:5922-5937`). The gap says "no glob". CLAUDE.md now documents `prefix*`/`*suffix` patterns, but `get*R` needs both, so it remains unenumerable |
| `scotty` | Network | import `Web.Scotty` | no — gap: no registrar or value refs in the Haskell plugin (`rules.rs:5939-5954`) |
| `wai` | Network | import `Network.Wai` | no — gap: `Application` is an untyped binding (`rules.rs:5956-5971`) |
| `tasty` | Test | import `Test.Tasty`, method name `tests` | yes — `<framework-entry>::test::tasty` (§11) (`rules.rs:5973-5988`) |
| `hspec` | Test | import `Test.Hspec`, method name `spec` | yes — `<framework-entry>::test::hspec` (§11) (`rules.rs:5990-6005`) |
| `haskell-main` | Cli | import `System.Environment`, `System.Exit`, `Options.Applicative` or `System.Console.GetOpt` | no — gap: `main` too common (`rules.rs:6007-6026`); reported under "seen, no rules" (fixture f1, f3) |

- **member_sources / signature_methods / container_attributes / markup_kinds / registrar verbs:** none for Haskell. The plugin records no attributes, containers or markup, and no rule declares registrars.
- **Visibility-based entries:** none (no visibility signal).
- **Framework coverage output** lists the four web frameworks under "seen, no rules" with their gap text (fixture f6).
- **`main`:** not a framework entry. It is a dead-code program-entry root (§12).

## 11. Test harness entries

- **`hspec`.** A definition named `spec` in a file importing `Test.Hspec` gets one harness node `<framework-entry>::test::hspec` with an `entry` edge (confidence `low`, `via framework_entry`). Fixture f3: `test/Spec.hs` gives `<framework-entry>::test::hspec → Main.spec`, with attribute "`spec` satisfies an implicitly-implemented hspec interface".
- **`tasty`.** A definition named `tests` gives `<framework-entry>::test::tasty → TastyMain.tests` (fixture f3).
- **Gating.** These are name-only test rules, so they apply only in a test file or a file importing the harness. Fixture f6 `src/NotTest.hs` defines `spec` and `tests` without the imports, and got no entry.
- **Trigger type:** naming only, an exact name. No annotations exist in Haskell.
- **NOT covered:**
  - HUnit `TestCase`/`TestList`, QuickCheck `prop_*` properties (no rule; `prop_*` would need a `prop_*` pattern), doctest, `tasty-discover` / `hspec-discover` auto-discovery of `*Spec.hs` modules.
  - Spec trees: individual `it`/`describe` blocks are not nodes. Their calls are attributed to `spec` (fixture f3 `Main.spec → Lib.one`).
  - Criterion/gauge benchmarks.
  - Haskell is not in `TEST_CLASS_MEMBERS_ARE_ROOTS` (Python/PHP/Ruby only, per CLAUDE.md), and the concept does not apply.
- **`--no-test-entries`** turns these into root marks (generic behaviour; not run on a Haskell fixture, unverified).
- **Withheld findings.** The dead-code report withheld 2 `only-used-by-tests` findings in fixture f3 (`Lib.one`, `Lib.two`).

## 12. Dead-code analysis

- **Roots:**
  - `main` is a universal program entry, `builtin:main` (`crates/cgg-resolve/src/deadcode/roots.rs:20`, `747-760`). There is no Haskell row in `LANG_ENTRY` (`roots.rs:24-156`).
  - Harness test roots (§11).
  - Fixture f1 shows "roots 1 discovered"; fixture f3 shows 7.
- **Signal coverage: `class degraded`, `vis no`, `attrs no`, `refs no`, `disp no`** (fixtures f1, f3 report header). With every `PluginSignals` false, `signals_complete()` is false and **no Haskell finding can be `high`** (`crates/cgg-resolve/src/deadcode/caps.rs:160-173`, test `a_language_with_no_signals_is_degraded` at `caps.rs:213`). Every finding is `medium` with risks `language-lacks-attributes`, `language-lacks-dispatch-model`, `language-lacks-value-references` and `language-lacks-visibility`.
- **Exports are not roots.** `module Lib (one, two, exportedButUnused) where` is ignored, so `Lib.exportedButUnused` is reported (fixture f3).
- **False positives the extraction gaps cause** (fixture f1/f2/f5):
  - functions used only through qualified calls, operators, composition, backtick infix, `$` left operands or as values (`plusInner`, `nullary`, `doIt`)
  - the `#else` arm's callees (`two`)
  - the spurious type-variable nodes (`Data.Thing.a`, `Inst.a`)
  - the second equation of a multi-equation function reached cross-file (`areaOf@13`)
  - instance methods for other types
- **Caps:** no `implements-external-contract`, `macro-generated` or `reflection` cap applies (no attributes, macros or dynuse).
- **Unresolved-doubt handling:** generic (`no-unresolved-site-with-name` appears in the evidence). Since qualified calls produce no site, they cannot even add doubt.
- **`foreign export`:** not an FFI root (§09).

## 13. Builtins & stdlib tables

- **`crates/cgg-core/src/stdlib/haskell.txt`: 365 lines**, one name per line. It holds Prelude and base names (`map`, `print`, `show`, `pure`, `putStrLn`), mtl/async/containers names (`ask`, `modify`, `async`, `atomically`, `alter`) and some type and class names (`Bool`, `Char`, `Applicative`). Loaded at `crates/cgg-core/src/stdlib.rs:212,253`. It was tuned against corpus external-bucket noise (CHANGELOG.md:5970-5975).
- **Screening.** An unresolved name in the list goes to `stdlib_calls` with `name_screen_applied: stdlib` (fixture f1: `map`, `print`, `pure`, `putStrLn`, `show`; fixture f4: `modify`).
- **A workspace definition beats the screen.** Fixture f4: a user `X.A.lookup` (`lookup` is a Prelude name) bound `high`.
- **`--include-stdlib` / `--include-external`** mint `<stdlib>::print` and `<external>::notDefinedAnywhere` leaf nodes with `std`/`ext` labels (fixture f4 mermaid).
- **No Haskell entries** in `CAST_INTRINSICS` (`stdlib.rs:35-46`), `CONSTRUCTING_METHODS` (`stdlib.rs:58`), the implicit-package tables or any `rust_types`-like table.

## 14. Confidence & certainty

- **`high` paths:**
  - An intra-file bare call to a unique name or to the first equation of a clause set (`clause_targets` returns `High` for one function, `intra_file.rs:928-932`).
  - A lexically narrowed local.
  - A cross-file step-1b hit when exactly one opened module declares the name and that callable is unique (fixtures f1 `main → work`, f3 `spec → one`, f4 `useLookup → lookup`).
- **`medium`:**
  - The same name opened from several modules (fixture f4).
  - A cross-file call into a multi-equation function (fixture f1 `doIt → helper@32`, f5 `viaOther → sh@7`).
  - Copies across files.
- **Framework entry edges are `low`** (fixture f3).
- **False `high`s observed:**
  - parameter shadowing (`shadow → A.Ops.f`, f2)
  - `hiding` ignored (`viaHidden → B.Other.other`, f2)
  - type-class instance selection (`useSh → Inst.sh@5`, f5)
- **No typed-receiver `OwnerFacts` path:** Haskell has no receivers.
- **Mermaid `?` marker:** an arrow with no `high` site gets `?` (fixture f4: `N4 -->|?| N0` for `useShared → X.A.shared`).

## 15. Dynamic use & reflection

- **dynuse: no Haskell coverage.** `crates/cgg-lang/src/plugins/dynuse.rs:25-` (`REFLECTIVE`) has no Haskell row, and the plugin never calls dynuse.
- **Not modelled:** `Data.Dynamic`, `Typeable`/`cast`, GHC `hint`/`plugins` dynamic loading, `lookupEnv`-driven dispatch.

## 16. Known gaps & limitations

1. Qualified calls (`M.f x`, `Data.Map.lookup`) are never recorded. `record_call` accepts only a `variable` head (`haskell.rs:208-210`). Fixtures f1/f2/f4.
2. Operators: neither operator definitions (`a <+> b = …`) nor operator or section uses (`$` left operand, `.`, `>>=`, `<$>`, backticks) produce nodes or sites. Fixtures f1/f2/f3.
3. Unapplied references (`doIt` in `do`, `x = y`, `map f xs`) record no site, so there are no value refs. Fixtures f1/f6. The `scotty` gap text says the same (`rules.rs:5952`).
4. Type classes are merged into clauses: instances of a method for different types become one function, and an intra-file call picks the first instance at `high` (f5).
5. Cross-file calls into a multi-equation function bind the last equation at `medium`. `first_clause_per_arity` is Erlang/Elixir-only (`cross_file.rs:5549,5600`). This can mark the first equation dead (f1 `areaOf@13`).
6. Spurious callables come from type variables in function types (`a -> b` in signatures; `Data.Thing.a`, `B.Util.a/b`) and from `do`-binds (`args <- getArgs` → `Main.args`). Spurious call `m` from type application `m ()` (f1/f2/f4).
7. No locals: parameters do not shadow top-level names (f2 `shadow → f` high), and parameter calls are classed external.
8. `import … hiding (…)` and import lists are ignored (f2). An ambiguous name from two opened modules binds the first at `medium` instead of being refused (f4).
9. Module export lists are ignored, so there is no visibility and exported names are not dead-code roots (f3 `exportedButUnused`). Every dead-code finding is capped at `medium` (`caps.rs`).
10. CPP `#else` arms are lost (f5). Template Haskell and quasi-quotes are opaque (f2).
11. `.lhs` is parsed without un-literating (`parse_status: error`; `\begin{code}` files lose their module name, f3).
12. FFI: `foreign import`/`foreign export` are not linked or rooted (f2).
13. Data constructors, record fields, pattern synonyms, class-method declarations without defaults, and lambdas are not callables. `where`/`let` locals are qualified at module level, with no owner (f1/f2/f3).
14. No arity is recorded. The clause logic hardcodes 0 (`intra_file.rs:897-900`).
15. Frameworks: `servant`, `yesod`, `scotty`, `wai` and `haskell-main` are detection-only, with gap text (`rules.rs:5905-6026`). No QuickCheck or HUnit rule exists.
16. No ignored tests mention Haskell. The plugin's inline tests are only `definitions_are_qualified_by_their_module`, `plugin_loads`, `free_functions_and_call` and `import_is_recorded` (`haskell.rs:246-293`). There is no Haskell fixture under `crates/*/tests/` besides `detect_prefixes.rs:66`.
17. README describes Haskell as "import | — | Top-level bindings" (`README.md:514`). The README benchmark row shows pandoc at 48% (`README.md:1340`).

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01. Detection & parsing | partial | `.hs`/`.lhs`, runghc/runhaskell shebangs; no un-literate, CPP `#else` lost, no content sniff |
| 02. Callables extracted | partial | equations/binds/where/let/instance methods; no operators, constructors, fields, pattern synonyms; spurious type-variable and do-bind nodes |
| 03. Call-site forms | minimal | only `variable`-headed `apply`; no qualified, operator, infix, composition, value or nullary references; no arity |
| 04. Imports & module resolution | partial | `import`/`import qualified [as]` recorded, prefix + scoped lookup; hiding/import lists/exports ignored; no manifests or packages |
| 05. Type inference & receiver typing | n/a | no receivers; no typing facts emitted |
| 06. Inheritance, dispatch & overloading | minimal | clause merging intra-file; type classes merged wrongly; cross-file picks the last equation; no dispatch |
| 07. Scoping & shadowing | partial | lexical narrowing of where/let defs works; no locals, so parameters do not shadow |
| 08. Macros, preprocessor & codegen | none | CPP, Template Haskell, quasi-quotes unhandled; no name_mentions |
| 09. FFI & cross-language links | none | `foreign import`/`export` ignored |
| 10. Frameworks & entry points | minimal | 7 rules; only 2 test rules enumerate; servant/yesod/scotty/wai/haskell-main detection-only |
| 11. Test harness entries | partial | hspec `spec`, tasty `tests` by exact name, import-gated; no QuickCheck, HUnit or discover |
| 12. Dead-code analysis | minimal | `main` + test roots; degraded class caps at medium; exports not roots; many false positives |
| 13. Builtins & stdlib tables | full | 365-name haskell.txt; workspace beats screen; include-stdlib/external work |
| 14. Confidence & certainty | partial | intra-file and single-module cross-file high; observed false highs (shadowing, hiding, instances) |
| 15. Dynamic use & reflection | none | no dynuse row |
| 16. Known gaps & limitations | n/a | 17 gaps listed above |
