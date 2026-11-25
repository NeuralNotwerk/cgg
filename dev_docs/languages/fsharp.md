# fsharp

> Revised 2026-11-25 against cgg 0.13.3: module paths in qualified names, module-qualified call resolution and module abbreviations, constructors and `inherit`, every `let`/`and` binding, operator-folded calls, value references and the locals table now exist, so the stale gaps, fixture results and scorecard were corrected. `file:line` citations not otherwise revised date from the 0.11.0 audit and may have drifted.

The F# plugin (`FsharpPlugin`, `crates/cgg-lang/src/plugins/fsharp.rs`) is 955 lines at 0.13.3, inline tests included. It parses `.fs`, `.fsi` and `.fsx` with the crates.io grammar `tree-sitter-fsharp` 0.3.0 (`Cargo.toml:116` pins `"0.3"`; `Cargo.lock:1518-1519` resolves 0.3.0). It always uses `LANGUAGE_FSHARP` (fsharp.rs:18-20). Nothing is vendored. The crate also ships a `LANGUAGE_SIGNATURE` grammar for signature files (`tree-sitter-fsharp-0.3.0/bindings/rust/lib.rs:32`), and the plugin does not use it.

One recursive `FsharpWalker` (`walk`, fsharp.rs:124-216) handles these node kinds:

- `anon_type_defn` pushes a type scope, records the `inherit` base and the primary constructor (`record_primary_constructor`, fsharp.rs:351).
- `function_or_value_defn` becomes one definition per binding (`record_let`, fsharp.rs:230).
- `member_defn` becomes a method or an additional constructor (`record_member`, fsharp.rs:281).
- `module_defn`/`module_abbrev` holding one long identifier becomes a `module-abbrev` import (`record_module_abbrev`, fsharp.rs:80).
- `namespace`, `named_module` and nested `module_defn` push a module path (`FsharpWalker::modules`); the file-level ones are recorded in `FileFacts::packages`.
- `import_decl` becomes an `open` import.
- `application_expression` becomes a call site (`record_call`, fsharp.rs:404).
- Under `reference_sites`, every node is also checked for value positions (`capture_value_positions`, fsharp.rs:550).

The plugin overrides none of `shebangs`, `ts_language_for`, `max_line_bytes`. The F# locals table is `locals::collect_fsharp` (shared with the plugin's `let` reader, `locals::fsharp_let_bindings`). Resolution adds `LibIndex::resolve_fsharp` (module-qualified calls) to the generic pipeline, and F# is in `lexically_scoped`, ML-style positional shadowing and `binds_same_name_set` in `intra_file.rs`. Seven `RuleSpec`s in `rules.rs` name F#.

Fixtures cited below are under `<scratch>/parity/fsharp/` (f1 … f11). Each was run with `target/release/cgg` 0.11.0 as `-t json -o out.json` and read with `show.py`/`aud.py` in that directory. The `--dead-code` runs used `--dead-code-format json --dead-code-confidence low -o dc.json`. Claims marked 0.13.3 were re-checked on new fixtures under `<scratch>/lang-docs/fsharp/` (n1 = `f1/`, n2 = `f2/`, n3 = `f3/`), run with `target/release/cgg` 0.13.3.

## 01. Detection & parsing

- **Extensions `.fs`, `.fsi` and `.fsx`** (fsharp.rs:15-17).
  - Fixture f7: `W.fs` → `extension:.fs`, `W.fsi` → `extension:.fsi`, `script.fsx` → `extension:.fsx`.
  - Test `detect_prefixes.rs:68` generates F# fixtures as `open {prefix}` in `.fs`.
- **Shebangs: none.** Fixture f7: an extensionless `runme` beginning `#!/usr/bin/env -S dotnet fsi` → `file_skipped` `unknown-extension`.
- **Content detection: none.** `detect.rs` has no F# entry (grep).
- **One grammar for every extension.** `.fsi` signature files are parsed with the implementation grammar and not with `LANGUAGE_SIGNATURE`. Fixture f7 `W.fsi` (re-confirmed 0.13.3 n1 `Sig.fsi`):
  - `val mk : unit -> Widget` yields no callable.
  - It yields a false call site named `val` (an external record, `src: null`, line 2).
- **No re-parse or fallback.** `#if/#else` is not blanked: both branches are parsed and extracted (§08).
- **Depth skip**: shared. A file deeper than `MAX_TREE_DEPTH` (4,000) is skipped as `TooDeep` (`cgg/src/lib.rs`, `cgg_lang::exceeds_depth`). Not exercised for F# (unverified for this language).
- **Minified / long line**: `max_line_bytes` is not overridden; the generic 2,000-byte rule skips locals collection on such files. Not exercised.
- **Parse status**: every fixture parsed `ok`. F# files with syntax errors are still extracted (generic behaviour, unverified for F#).

## 02. Callables extracted

Three variants:

- `FreeFunction` (kind `function`), for a `let` with arguments outside any type scope.
- `InherentMethod` (kind `method`), for a `let` inside a type, or any `member_defn`.
- `Constructor`, for a class's primary constructor and each `new(…) =` (`record_primary_constructor`, `push_constructor`, fsharp.rs:351-403; test `constructors_are_recorded`).

A member's qualified name is `Type.name`. A module-level function's qualified name is its module path then its name (`module_qn`, fsharp.rs:62): the file's `namespace` or `module A.B.Mod`, then each nested `module Inner =`. 0.13.3 n1: `Main.helper`, `Acme.Core.Util.format`, `Acme.Core.Util.Nested.deep`.

Supported:

- **Curried / tupled `let` functions**: `let helper x = x + 1`, `let add a b = a + b`.
  - Node `helper` (FreeFunction).
  - Test `let_binding_with_args`. Fixture f1.
- **`let inline`, `let rec`**: `let inline twice f x = …` and `let rec fact n = …` → nodes `twice` and `fact`. Fixture f1.
- **Nested `let` functions**: `let outer y = let inner z = … in inner y` → separate node `inner`. The edge `outer → inner` is `high`. Fixture f1.
- **Every binding of a `let`** (each `and` of `let rec … and …`) is its own callable, and a value pattern that is a lowercase name applied to arguments (`let f (x: T) : R = …`, the shape a return-type annotation produces) is a function (`locals::fsharp_let_bindings`; test `every_function_binding_is_a_callable`). 0.13.3 n1: `let rec even … and odd …` → `Main.even`, `Main.odd`, linked both ways `high`; `let mk () : Service = …` → `Main.mk`.
- **Constructors**: `type Service(name)` → `Service.Service` (primary), `new() = Service("x")` → a second `Service.Service`; `type Base()` → `Base.Base` (0.13.3 n1).
- **Class members**: `type Service(name) = member this.Run() = …` → `Service.Run`.
  - The last identifier of `property_or_ident` is the name (fsharp.rs:182-199).
  - Test `type_members`. Fixture f1.
- **Static members and member overloads**: `static member Create(n)` and `static member Create(n, m)` → two `Service.Create` nodes. Fixture f1.
- **Properties with explicit accessors**: `member this.Prop with get() = 1` → method `Service.Prop`. Fixture f1.
- **`default` / `override` members**: `default this.Virt()` and `override this.ToString()` → `Service.Virt` and `Service.ToString`. Fixture f1.
- **Class-private `let` functions**: `type Service(..) = let priv () = name` → `Service.priv`, kind `method`. Fixture f1.
- **Interface implementations inside a class**: `interface IShape with member this.Area() = …` → `Circle.Area`. Fixture f1.
- **Attributed definitions**: `[<Test>] member this.Adds()` and `[<Function("Hello")>] let run …` carry their attribute text.
  - Collected by `attrs::collect_with_preceding` (fsharp.rs:173, 223), since 0.6.8.
  - Fixtures f6 and f10: `['Test']`, `['SetUp']`, `['Function("Hello")']`, `['EntryPoint']`.
- **Double-tick names**: ``let``free test``() = …`` → node ``` ``free test`` ```. Fixture f6.
  - The mermaid label's leading backtick is escaped as `#96;` (mermaid.rs:356-358, CHANGELOG 0.10.1 "Fixed" block at CHANGELOG.md:1705-1715).
- **Object-expression members**: `{ new IDisposable with member this.Dispose() = () }` → node `Dispose` with **no owner**. Fixture f4.

Not supported or partial:

- **Record, union and type-extension members have no owner** (re-confirmed 0.13.3 n1: `{ X: int } with member this.Double()` → `Double`). `record_type_defn`, `union_type_defn` and `type_extension` do not push scope.
  - Fixture f1: `{ X: int } with member this.Double()` → `Double`; the union member `Tag` → `Tag`.
  - Fixture f4: `type System.String with member this.Shout()` → `Shout`.
- **Value bindings are never callables.** This includes:
  - `let value = 42`;
  - point-free functions: `let partialApp = classify`, `let helloHandler = text "hi"`, `let webApp = choose […]`;
  - Expecto `let tests = testList …`.

  A `let` with no `function_declaration_left` is skipped (fsharp.rs:138-141). Fixtures f1, f4, f10.
- **Operator definitions**: `let (|>>) a b = b a` → no node (re-confirmed 0.13.3 n1). Fixture f1.
- **Active patterns**: `let (|Even|Odd|) n = …` → no node. Fixture f4.
- **Lambdas**: `fun q -> …` → no node. Their calls are attributed to the enclosing function. Fixture f1, line 26.
- **Auto-properties**: `member val Name = … with get, set` → no node. Fixture f1; re-confirmed 0.13.3 n1.
- **Abstract slots and interface declarations**: `abstract member Virt : unit -> int` and `type IShape = abstract member Area : …` → no node.
  - `member_signature` is not handled.
  - Fixture f1: `IShape` declares nothing, and `Service.Virt` exists only through its `default` (re-confirmed 0.13.3 n1: `Base.Virt` from `default` only).
- **Signature-file `val` declarations**: no node. Fixture f7.
- **`extern` declarations** (`[<DllImport>] extern int native_add(…)`, `extern_binding`, grammar.js:183): no node. Fixture f8.
- **Top-level script code** in `.fsx` and `do` blocks: no node. Calls there get `no-enclosing-callable`. Fixture f7.
- **Visibility is not recorded**: `let private hidden` and `let internal inner` have empty `visibility`. Fixture f6; the 0.13.3 dead-code capability row still reports `visibility none`.
- **Generated code**: none (§08).

## 03. Call-site forms

Only `application_expression` produces a call site (`record_call`, fsharp.rs:404-548). The walker descends leftward through nested applications to the callee. When the head is an `infix_expression` (the grammar reads `xs |> f a` as `(xs |> f) a`), the callee is the operator's right operand, unless the application is a folded block (`applies_to_block`, fsharp.rs:684). It records the callee in one of three ways:

- A `long_identifier` is split on identifiers: the last one is the name and the prefix, joined with `.`, is `receiver_hint`.
- A `dot_expression` whose text is a plain dotted path (`A.B` dot `f`) is the same dotted call.
- A plain `identifier` is recorded as is (with `not_def` for a `new()` delegating to its own type).

Under `reference_sites`, value positions are recorded with `RefRecord::value_ref` (below). No `arity`, `typed_receiver`, `path_receiver`, `paren_less` or `implicit_receiver` field is set.

Supported:

- **Bare application**: `greet "x"`, `helper 3`, `fact (n - 1)`, `inner y`. Test `application_captured`. Fixture f1: high intra-file edges.
- **Unit call**: `run ()` and `fetch ()`. Fixtures f2 and f4.
- **One-dot method or qualified call**: `s.Run()`, `this.Run()`, `Service.Create("b")`, `Util.format 1`, `File.ReadAllText "x"` → name plus a one-segment receiver. Test `application_captured` (`Run` with receiver `s`). Fixtures f1 and f2.
- **Explicit type arguments**: `f<int> 1` and `N.h<string> "a"` are recorded as `f` and `h`/`N`, through `typed_expression` (fsharp.rs:238-244).
  - Test `type_application_calls_are_recorded`.
  - CHANGELOG.md:881-908 (WS9, FINDINGS C1).
- **Calls inside computation expressions**: `async { let! x = fetch () … }` → `ce → fetch` (high). Fixture f4.
- **Calls inside quotations**: `<@ handler () @>` → recorded as a real call (an edge to `Other.handler`). Fixture f8.
- **Calls inside lambdas**: `fun q -> helper q` → attributed to the enclosing `let`. Fixture f1.
- **Multi-segment module paths**: `X.Y.zed 1` → `X.Y.zed` `high` (0.13.3 n4 = `f4/`). `System.Activator.CreateInstance(…)` is recorded and goes external.
- **Calls folded into an operator expression** (since WS26b): `[1] |> X.Y.f 2` → call of `X.Y.f` `high` (0.13.3 n4; test `a_call_folded_into_an_operator_is_recorded`).
- **Value references** (only under `--reference-edges` or `--dead-code`): `f`/`M.f` as application arguments, operands of `fsharp::FUNCTION_OPERATORS` (pipelines, composition, monadic/applicative; fsharp.rs:676), record and object-expression values, list/array/tuple elements and a `let`'s or member's value; never a capitalized bare name or a member through a variable (test `function_values_are_reference_sites`). 0.13.3 n1 with `--reference-edges`: `1 |> helper`, `List.map helper` and `List.map classify` each give a `reference` edge `high`.

Not supported or partial:

- **Chained calls** `o.ToString().Trim()` and `d.Use().ToString().Length` record only the first call. Fixtures f9 and f4.
- **Calls on a parenthesised receiver**: `(makeIt ()).Go()` and `(a :> Animal).Speak()` record no `Go`/`Speak` site. Fixtures f9 and f5; re-confirmed 0.13.3 n4 `(Widget()).Render()`.
- **Calls on a record expression**: `{ X = 1 }.Double()` is not recorded. Fixture f1.
- **Pipelines and composition are not call edges**: `1 |> zed`, `zed >> zed` record `zed` only as a value reference, so the default graph (without `--reference-edges`) has no edge for them (0.13.3 n1: `1 |> helper` → no `direct` edge).
- **User-defined operator calls**: `3 |>> helper` → nothing. Fixture f1.
- **Constructor calls** `Service("a")` bind the type's constructors; with no arity narrowing (F# is not in `OVERLOAD_ARITY_LANGS`) both `Service.Service` nodes are reached at `medium` (0.13.3 n1).
- **Duplicate sites for curried applications.** In `printfn "%d" x`, each nested `application_expression` records its own site at the same byte (re-confirmed 0.13.3 n4: two `printfn` external records at byte 301).
- **Registrar arguments**: not captured. The plugin never calls `registrar::*` (grep), and no F# rule declares registrars.
- **Arity**: never recorded (`RefRecord::arity` is not set).

## 04. Imports & module resolution

- **`open A.B`** → `ImportRecord { kind: "open", path: "A.B" }` (fsharp.rs:100-113). Test `open_is_import`.
- **Module-qualified calls (WS34, 0.11.2).** A call through a capitalized path binds the `f` of the module that path names, written whole or relative to the file's own namespaces and modules or its `open`s, nearest first (`LibIndex::resolve_fsharp`). One target is `high`, copies in several files `medium`; a path no in-tree module declares (`List.map`) falls through and stays external. 0.13.3 n1: `Util.format 1` (after `open Acme.Core`) → `Acme.Core.Util.format` `high`; `Acme.Core.Util.Nested.deep ()` and `Util.Nested.deep ()` → `Acme.Core.Util.Nested.deep` `high`. This works independently of file stems.
- **Module abbreviations** (`module L = A.B`, a `module-abbrev` import record, fsharp.rs:80-107): the alias heading a path is read as its target. 0.13.3 n2: `module L = Llm_v2` then `L.classify x` → `Llm_v2.classify` `high`; `module V = Util` (with `open Acme.Core`) then `V.format 3` → `Acme.Core.Util.format` `high`.
  - **Bug at 0.13.3**: an abbreviation whose target is a dotted path does not resolve. n2/n3: `module U = Acme.Core.Util` then `U.format 2`, and `module M = A.B.Mod` then `M.h 1`, are both `candidates-in-other-files` (1 candidate), while the same call written in full binds `high`.
- **Bare calls after `open`**: still name-based. 0.13.3 n1: with only `open Acme.Core`, `only ()` fans out at `medium` to both `Acme.Core.Util.only` and the never-opened top-level module `Other.only`, a wrong edge.
- **`open type X`** is accepted by the grammar (grammar.js:284). The plugin reads only the `long_identifier`, so it is recorded like a plain `open` (unverified by fixture).
- **`#load "x.fsx"` / `#r "nuget: …"`** (`fsi_directive_decl`): not recorded (no handler).
- **Namespace / package records**: `FileFacts::packages` holds the file's `namespace`/`module` names (fsharp.rs:170-191).
- **Manifest reading**: none. `.fsproj` compile order and `paket.dependencies` are not read; `manifest.rs` has no F# entry.
- **External-module refusal (`heads.rs`)**: F# is not covered. The `match lang` arms are Python, JS/TS, Go, Java/Kotlin, C#, Scala, Solidity and C++ (heads.rs:413-625).
  - Calls like `List.map` and `printfn` reach the external bucket only through the generic name screen (`name_screen_applied: "external"`, stage `no-candidate-in-file`). Fixture f1.
- **`not-in-imported-module` (`modules.rs`)**: not covered. A module path no in-tree module declares falls through to the ordinary steps instead.
- **Language family**: C#/F# family sharing is commented out (lang_family.rs:34), so there is no C# ↔ F# resolution.
- **Test**: `fsharp_bare_call_never_reaches_a_member` (cross_file.rs:13535-13563, R15). With `open Paket.Domain`, a bare `PackageName` does not bind a type's member `PackageTypes.PackageName`.

## 05. Type inference & receiver typing

The plugin records no `local_types`, `scoped_types`, `class_fields`, `arg_type` or typed receivers. All typing comes from the generic `type_hints` passes over `signature_hint`.

Supported:

- **Annotated parameter in the first parenthesised list**: `let typed (x: Widget) = x.Render()` and `let tupled (g: Gadget, w: Widget) = w.Render()`.
  - The receiver is typed and the edge is `high`.
  - Mechanism: the generic `name: Type` parse (`parse_colon_param`, type_hints.rs:1446-1462).
  - Fixtures f2 and f7. In f9, `o.ToString()` was rewritten to receiver `obj`.
- **`this.` receivers**: `this.Run()` and `this.Helper()` bind to the enclosing type (or the single same-named method) at `high`. Fixtures f1 and f4; 0.13.3 n1 `this.Virt()` → inherited `Base.Virt` `high`.

Not supported or partial:

- **Curried parameters after the first group**: in `let curried (g: Gadget) (w: Widget) = w.Render()`, `w` is untyped, and `Render` fans out to `Widget.Render` and `Gadget.Render` (medium). Fixture f7; re-confirmed 0.13.3 n4.
- **Locals from construction**: `let w = Widget(3)` then `w.Render()` → medium by name. Fixture f2. 0.13.3 n1: `let s = Service("a")` then `s.Run()` → `medium` even with one `Run` in the tree.
- **Pattern-annotated locals**: `let (w3: Widget) = Widget.Make()` → `w3.Render()` medium. Fixture f2.
- **Return-type annotations**: `let mk () : Widget = …` then `let w = mk ()` → `w.Render()` fans out to both owners (medium). Fixture f7; 0.13.3 n1 `w.Run()` after `let w = mk ()` → `medium`.
- **Lambda parameter annotations**: `fun (x: Widget) -> x.Render()` → medium fan-out. Fixture f7. `closure_types.rs` has no F# entry.
- **Upcasts and downcasts** `(a :> Animal)` and `:?>`: not peeled, because `receiver::peel` is not used and the site is dropped (§03). Fixture f5.
- **Option wrappers**: in `(w: Widget option)`, `w.Value.Render()` is recorded with an untyped receiver and fans out to every `Render` at `medium` (0.13.3 n4).
- **Implicit receivers** (Kotlin-style) and **weak receiver types**: not applicable; nothing is recorded.

## 06. Inheritance, dispatch & overloading

- **Base records**: `inherit Base(…)` is recorded on the type's scope (fsharp.rs:136-141) and its constructor call reaches `Base`'s constructor (0.13.3 n1: `Service.Service -> Base.Base` `high`). `interface I with` is not a base.
- **`base.M()`** is resolved by name.
  - Fixture f5: inside `Dog.Speak`, `base.Speak()` → `Animal.Speak` **and** `Cat.Speak` (medium, `cross-file:imports`). Cat is a sibling, so this is a wrong edge.
  - Fixture f4 got the single right answer (`high`) only because the base was literally named `Base`. That matches the receiver text `base` case-insensitively (coincidence; unverified mechanism).
- **Inherited `this.M()`**: `this.Helper()` in `Derived` → `Base.Helper`, high, but only because exactly one `Helper` exists in the file. Fixture f4. There is no `resolve_via_bases` support without base records.
- **Untyped receiver fan-out**: `x.Speak()` → all three `Speak` (medium; `?` in mermaid). Fixture f5.
- **`member_of_calling_class` / `overrides_below`**: F# is not wired into either path. `bare_call_reaches` treats F# like Python/Rust: a bare name never reaches a member (cross_file.rs:4067-4069, 4137-4139).
- **Interface → implementation**: abstract members are not nodes (§02), so no declaration exists to dispatch from.
- **`--dynamic-dispatch` / `inheritance_fanout`**: F# is not in `INHERITANCE_LANGS`. Fixture f5 with `--dynamic-dispatch`: no `dyn` edges (0.13.3 n1: none either).
- **Type extensions / extension members**: `type System.String with member this.Shout()` → owner-less `Shout`. `s.Shout()` on `(s: string)` binds by name (medium). Fixture f4.
- **Overload sets**: same-file, same-QN overloads are one callee (`binds_same_name_set` includes `fsharp`, intra_file.rs:829-850).
  - Fixture f1: `Service.Create("b")` → both `Service.Create` overloads, medium.
  - CHANGELOG.md:258-262 (overload sets past the cap bind in F#, +242 edges).
- **Arity narrowing**: F# is not in `OVERLOAD_ARITY_LANGS`, and no arity is recorded (0.13.3 n1: `Service.Create(1)` and `Service.Create(1, 2)` each bind both overloads at `medium`).
- **Mixins and traits**: not applicable.

## 07. Scoping & shadowing

- **`FileFacts::locals`** (WS27, `locals::collect_fsharp`): `let`-function and member parameters (curried, tupled, typed, `?opt`, patterns) and a member's self identifier over the definition, `fun` patterns over the lambda, `match`/`function`/`try … with` rule patterns over their rule (guard included), local `let`/`let!`/`use`/`use!` from the end of the binding to the end of its `in` body (`let rec` from its own start) and `for` variables. Module-level and class `let`s are not locals; a capitalized pattern head is a union case; a qualified call is never shadowed.
  - 0.13.3 n1: `let apply handler = handler ()` and a local `let helper = fun x -> x` then `helper 2` are both `shadowed-by-local` (the 0.11.0 wrong edges are gone).
- **Lexical narrowing of nested definitions**: supported (`lexically_scoped` includes `fsharp`, intra_file.rs:118-135; CHANGELOG.md:576-582).
  - Fixture f4: in `let shadow () = let fetch () = 2 in fetch ()`, the call binds the inner `fetch` (high), not the module-level one.
  - Fixture f1: `outer → inner`.
- **Positional (ML-style) shadowing**: supported. A later same-scope binding before the site wins (intra_file.rs:400-416, `"ocaml" | "fsharp" | "lua"`).
  - Fixture f8: with two `cond` definitions in `#if/#else`, `useCond` binds the second (line 16) at high.
- **Conditional definitions**: no `DefRecord::conditional` (§08).
- **Module scope**: modules name functions (§02) but are not lexical scopes for narrowing; nested-module functions are lexically top-level. A function nested in another is never a bare-name candidate from another file (`cross_file::nested_function_ids`).

## 08. Macros, preprocessor & codegen

- **Macros**: F# has none. Not applicable.
- **`#if` / `#else` / `#endif`**: not handled. Both branches are extracted as definitions with the same QN. Fixture f8: two `cond` nodes. Positional shadowing then picks the later one (§07), whichever branch the build would use.
- **Quotations** `<@ … @>` / `<@@ … @@>`: the contents are walked as ordinary code, so calls inside are recorded as real call edges. Fixture f8 `quoted → handler`.
- **Type providers and generated code**: not modelled. There is no `macro:<name>` tagging.
- **`name_mentions`**: never filled by the F# plugin.
- **Computation-expression custom operations** (`router { get "/" h }`): not recorded as calls (rules.rs:6144 gap text).

## 09. FFI & cross-language links

- **P/Invoke from F#**: `[<DllImport("native.so")>] extern int native_add(int a, int b)` produces no callable. The call `native_add(1, 2)` is `no-candidate-in-file`, and there is no edge to the C `native_add` in the same tree. Fixture f8.
  - `ffi.rs`'s `pinvoke` pass is documented for C# only (ffi.rs:13, 629, 799-846).
- **PyO3, wasm-bindgen, napi, JNI and `extern "C"`**: not applicable to F#.
- **Descriptor links**: no Protobuf/gRPC → F# link. `descriptor.rs` has no F# reference (grep).
- **Markup code-behind**: none. `markup.rs` is read by the C# plugin only, and `markup_kinds_for` has only C# arms (rules.rs:11812-11816). Avalonia/WPF F# code-behind is not linked.
- **C# ↔ F# in one solution**: no family link (lang_family.rs:34 is commented out).

## 10. Frameworks & entry points

Every `RuleSpec` with `language: "fsharp"` is listed below. Each spec has a single language, and no multi-language rule includes F#.

| rule id | kind | detect | trigger | effective? |
| --- | --- | --- | --- | --- |
| `azure-functions` (rules.rs:2746) | network | `Microsoft.Azure.Functions.Worker`, `Microsoft.Azure.WebJobs` | attributes `Function`, `FunctionName` | **Yes** |
| `fsharp-entrypoint` (rules.rs:6422) | cli | `System`, `Argu`, `CommandLine` | none | No, detect-only gap |
| `giraffe` (rules.rs:6439) | network | `Giraffe` | none | No, gap |
| `saturn` (rules.rs:6456) | network | `Saturn` | none | No, gap |
| `suave` (rules.rs:6473) | network | `Suave` | none | No, gap |
| `expecto` (rules.rs:6490) | test | `Expecto` | none | No, gap |
| `fsharp-nunit` (rules.rs:6507) | test | `NUnit.Framework`, `FsUnit`, `NUnitLite` | attributes `Test`, `TestCase`, `TestCaseSource`, `SetUp`, `TearDown`, `OneTimeSetUp`, `OneTimeTearDown`, `Fact`, `Theory` | **Yes** (§11) |

Notes on each rule:

- **`azure-functions`**: `[<Function("X")>] let run …` mints `<framework-entry>::network::azure-functions::Function("Hello")` with an `entry` edge.
  - Test `azure-functions/fsharp` (crates/cgg/tests/frameworks.rs:1278-1282). Fixture f10.
  - The trigger kind on the first parameter is not read (gap text).
- **`fsharp-entrypoint`**: it detects the framework but has no matchers. `[<EntryPoint>]` is not a matcher, and `main` is rooted only by name (§12).
- **`giraffe` and `suave`**: `route … >=> handler` and `choose […]` live in operator and list position; since WS26b those handlers are value references under `--dead-code`/`--reference-edges`, but no rule enumerates them as entries. Fixture f10 prints "seen, no rules giraffe — entries NOT enumerated".
- **`saturn`**: computation-expression `router { … }` operations are not recorded.
- **`expecto`**: `[<Tests>]` values are not callables, and argument-position lambdas are not recorded. Fixture f10 prints "seen, no rules expecto".
- **`fsharp-nunit`**: matches attributes on `let` functions and members.
- **Stale gap text**:
  - The `expecto` gap says "the fsharp plugin populates no attributes".
  - The `fsharp-nunit` gap says the plugin "hardcodes `attributes: Vec::new()`".

  Both have been false since 0.6.8, and fixtures f6 and f10 show attributes recorded and `fsharp-nunit` matching. Both texts are still unchanged at 0.13.3 (rules.rs:6502, 6529); 0.13.3 n1 records `['EntryPoint']` on `Main.start`.
- **`member_sources_for`**: no F# arm. `TestCaseSource` data members named by string are not rooted. Only `nunit-runner`/`xunit-data` for C# exist (rules.rs:11719-11760).
- **`signature_methods_for`**, **`container_attributes_for`**, **`markup_kinds_for`**, **`visibility_entries_for`** and **`self_module_markers_for`**: no F# arm (rules.rs:11666-11866).
- **Registrar verbs**: no F# rule declares `registrars`, so `registrar_verbs_for("fsharp")` is `None` (rules.rs:11929-11955), and the plugin does no capture.
- **Corpus apps**: `benchmark.sh` covers each rule with an app:
  - `app-fsautocomplete-expecto` (`~expecto`), line 253;
  - `app-cardmgmt-giraffe` (`~giraffe`), line 258;
  - `app-swate-saturn` (`~saturn`), line 299;
  - `app-fssnip-suave` (`~suave`), line 308;
  - `~fsharp-entrypoint` and `~fsharp-nunit` in `app-avalonia-samples`/`app-orleans-samples`, lines 203 and 209.

  The `~` prefix marks rules expected to be gap/unverified.

## 11. Test harness entries

- **`fsharp-nunit` (kind `test`)** mints one harness node, `<framework-entry>::test::fsharp-nunit`, with `entry` edges to each attributed case.
  - Fixture f6: `[<Test>] member this.Adds()`, `[<SetUp>] member this.Init()` and ``[<Test>] let``free test``()`` get edges at `low` confidence, `via framework_entry`.
  - Dead-code roots list them as `test-entry` (`framework:test-harness`).
  - It is attribute-based, so the test-file gating for name-only rules does not apply.
  - CHANGELOG.md:105-115 lists `fsharp-nunit` among the harness nodes (WS18).
  - xUnit `Fact`/`Theory` are included in the same rule's attribute list (rules.rs:6197-6198).
- **Unattributed members of a `[<TestFixture>]` class are not roots.** Fixture f6: `MyTests.NotATest` is reported never-referenced. F# is not in `TEST_CLASS_MEMBERS_ARE_ROOTS` (`["python", "php", "ruby"]`, rules.rs:11806).
- **`expecto`**: kind `test`, but no matcher (§10). `testCase`/`testList`/`testProperty` values are invisible.
- **FsCheck properties, `[<Property>]`, xUnit through `FsUnit.Xunit`, BenchmarkDotNet in F#**: no F# rule. `benchmarkdotnet` is C#-only.
- **`--no-test-entries`**: generic. It turns the harness node into root marks (not exercised for F#).
- **Test tagging (`TestRole`)**: none. `test_defs` is not declared; capabilities show `"test_tagging": "none"` (fixture f6). One `only-used-by-tests` finding was withheld for `missing-signal`.

## 12. Dead-code analysis

- **Capability class: `degraded`**, with `"max_confidence": "medium"`. 0.13.3 n1 `capabilities`:
  - visibility `none`
  - attributes `full`
  - value_references `full` (was `none` in the 0.11.0 fixture f6)
  - dispatch `none`
  - exports `none`
  - test_tagging `none`

  `signals_complete` needs both visibility and attributes, so **no F# finding can be `high`**.
- **Evidence on every finding** (0.13.3 n1): `language-lacks-dispatch-model`, `language-lacks-visibility`, `no-incoming-edges`, and `no-unresolved-site-with-name` or `name-matches-unresolved-site`.
- **Roots**:
  - `main` by name (`UNIVERSAL_ENTRY`, roots.rs:20). F# has no `LANG_ENTRY` row (roots.rs:23-32).
  - Framework entries and test harnesses (§10, §11).
  - `cgg-deadcode.toml` user roots (generic).
- **`[<EntryPoint>]` with a name other than `main` is reported dead.** Fixture f10: `[<EntryPoint>] let start argv = 0` → finding `start` (medium); re-confirmed 0.13.3 n1 (`Main.start`).
- **Visibility**: `private` and `internal` are not recorded, so `hidden`/`inner` are judged the same as public functions (fixture f6). There are no public/exported roots.
- **Caps**:
  - `implements-external-contract`: never applied. `override` members are not recognised as overriding anything, and there are no base records.
  - `macro-generated`: not applicable.
  - `reflection`: not applicable (§15).
- **False positives the gaps cause**:
  - Giraffe/Suave/Saturn handlers reached only through routing values with no entry rule;
  - Expecto tests;
  - functions reached only through user-defined operators or a parenthesised receiver (§03).

  Functions used as values or through pipelines, and multi-segment calls, are credited since WS26b/WS34.
- **Unreachable code** (`unreachable`): not declared, so there are no statements-after-terminator findings.

## 13. Builtins & stdlib tables

- **There is no `stdlib/fsharp.txt`** (still absent at 0.13.3).
  - `printfn`, `List.map`, `string`, `ignore` and the like go to the external bucket through the generic name screen. With `--include-stdlib` they never become `std` exit nodes. Fixtures f1 and f8: `0 stdlib`; 0.13.3 n4: `printfn` and `System.Activator::CreateInstance` external.
  - `--externals` has F# rows (stdlib.rs): `STDLIB_ROOTS` (`System`, `FSharp.Core`, `Microsoft.FSharp`), `STDLIB_LIBRARY_NAMES` (`FSharp.Core`), `PROCESS_SPAWN_APIS` and `LIBRARY_LOAD_APIS`. They affect only the externals report.
- **`rust_types`-like wrapper tables**: none for F# (`option`, `list`, `Async<T>`, `Task<T>`).
- **`CAST_INTRINSICS` / `CONSTRUCTING_METHODS`**: unused, because the plugin does not call `receiver::peel`.
- **Implicit packages** (like `SCALA_IMPLICIT_PACKAGES`): none. F#'s auto-opened `FSharp.Core` modules are not modelled.
- **`--include-external`**: works generically. Fixture f8: `<external>::t::GetMethod`.

## 14. Confidence & certainty

- **`high` is reached through these paths**:
  - **Intra-file single candidate**: a bare call to a same-file `let` (`usesThings → helper`), the innermost nested definition, positional shadowing, and `this.M()` with one candidate. Fixtures f1, f4, f8.
  - **Annotated first-group parameter receiver naming exactly one type** (`typed → Widget.Render`, `tupled → Widget.Render`). Fixtures f2 and f7.
  - **Module-qualified call naming one in-tree module function** (`LibIndex::resolve_fsharp`; 0.13.3 n1 `Util.format`, `X.Y.zed`), including through a single-segment module abbreviation (n2).
- **`medium` is forced by**:
  - by-name fan-out on untyped receivers (`s.Run()`, `x.Speak()`, `base.Speak()`);
  - same-QN overload sets (`Service.Create`);
  - bare calls after `open` (fixture f3: `run2 → Other.only` medium);
  - constructions of a type with several constructors (0.13.3 n1 `Service("a")`).
- **Framework entries** are `low` (fixtures f6 and f10).
- **`low` through `--guess-unique-names`**: Rust only; not applicable.
- **The mermaid `?` marker** prefixes every non-high arrow label (fixture f5: `N1 -->|?| N0`).
- **Measured history**: CHANGELOG.md:1440-1446 records "F# high 0 / 30, medium 0 / 12" from the type-argument fix, and CHANGELOG.md:1338-1341 records "F# +0.4k / −1.0k".

## 15. Dynamic use & reflection

- **Not covered.** `dyn_uses` is not declared, and `dynuse.rs` covers Python, JS, Java, C#, Ruby and PHP only (CLAUDE.md, step 7; grep shows no F# entry).
- `t.GetMethod("Length")`, `Activator.CreateInstance(t)`, `typeof<…>` and `nameof` do not cap any finding.
  - Fixture f8: `GetMethod` is an external call with receiver `t`.
  - `System.Activator.CreateInstance` is recorded as an external call (0.13.3 n4), with no reflection screen.
- **Dynamic operator `?`** and **`Assembly.Load`** module loads: not recognised (no handler).

## 16. Known gaps & limitations

From code, rule gap texts and CHANGELOG:

- `gap_for("fsharp-entrypoint" | "giraffe" | "saturn" | "suave" | "expecto", "fsharp")`: entries are not enumerated (rules.rs:6110, 6127, 6144, 6161, 6178).
- The `fsharp-nunit` and `expecto` gap texts are **stale**: they claim attributes are never populated (rules.rs:6178, 6205; contradicted by fsharp.rs:168-173 and fixture f6).
- F#/C# family sharing is disabled (lang_family.rs:34).
- No `#[ignore]`d F# tests.

Fixed since the 0.11.0 audit (re-checked with 0.13.3 fixtures): multi-segment module paths, operator-folded calls, module paths in qualified names and module-qualified resolution, module abbreviations (single-segment targets), the locals table, constructors and `inherit`, every `let`/`and` binding and return-annotated functions, value references.

Still open at 0.13.3:

1. **Pipelines, composition and operator operands are value references, not call edges** (`x |> f`, `f >> g`); user-defined operator calls record nothing (§03).
2. **Bare calls after `open` fan out by name**, reaching never-opened modules (§04).
3. **Module abbreviations with a dotted target** (`module U = A.B.C`) do not resolve (§04).
4. **No interface records**: `base.M()` fans out to siblings, and `--dynamic-dispatch` does nothing (§06).
5. **Missing callables**: operators, active patterns, `member val`, abstract slots, `.fsi` `val`s, `extern` (P/Invoke), and point-free value bindings (§02).
6. **Record, union and type-extension members are owner-less** (§02).
7. **No visibility**, so dead code is capped at medium; **`[<EntryPoint>]` is not a root** (§12).
8. **Duplicate call sites** for curried applications (§03).
9. **`.fsi` parsed with the implementation grammar** produces a false `val` call site; `LANGUAGE_SIGNATURE` is unused (§01).
10. **`.fsx` shebang scripts without an extension** are not detected; top-level script calls are `no-enclosing-callable` (§01, §02).
11. **Return types, curried-parameter types, pattern annotations, lambda annotations and casts** are not used for receiver typing (§05).
12. **No stdlib table**, so `--include-stdlib` produces no F# `std` nodes (§13).
13. **`#if` branches are both extracted**, and the later one wins (§08).
14. **Quotations are treated as executed code** (§08).
15. **No arity narrowing** for overloads and constructors (§06).

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01. Detection & parsing | partial | `.fs/.fsi/.fsx` by extension; no shebang; `.fsi` uses the implementation grammar and yields a false `val` site |
| 02. Callables extracted | partial | every `let`/`and` function and class member, constructors; module paths in QNs; no operators, active patterns, abstract slots, auto-properties; record/union/extension members owner-less |
| 03. Call-site forms | partial | bare, dotted (any depth), type-applied and operator-folded applications; value refs under `--reference-edges`; pipelines are values not calls; no chains or arity; duplicate curried sites |
| 04. Imports & module resolution | partial | module-qualified calls via `resolve_fsharp` (high); module abbreviations (single-segment targets only); packages recorded; bare calls after `open` still by name |
| 05. Type inference & receiver typing | minimal | first parenthesised parameter annotations only; no locals, return types, curried params, lambdas or casts |
| 06. Inheritance, dispatch & overloading | minimal | same-QN overload sets bind at medium; `inherit` reaches the base constructor; `base.` fans to siblings, not in INHERITANCE_LANGS or arity langs |
| 07. Scoping & shadowing | good | locals table (parameters, patterns, local lets); lexical narrowing and ML positional shadowing |
| 08. Macros, preprocessor & codegen | n/a | no macros; `#if` not handled (both branches kept); quotations walked as code |
| 09. FFI & cross-language links | none | `extern`/DllImport not extracted; no C# family link; no markup code-behind |
| 10. Frameworks & entry points | partial | 7 rules: azure-functions works; giraffe/saturn/suave/fsharp-entrypoint/expecto are gaps; stale gap texts |
| 11. Test harness entries | partial | fsharp-nunit attributes mint a harness node; Expecto invisible; fixture members not rooted |
| 12. Dead-code analysis | partial | class `degraded`, max medium; no visibility; value refs credited; `[<EntryPoint>]` non-`main` reported dead |
| 13. Builtins & stdlib tables | none | no `stdlib/fsharp.txt`, no wrapper or cast tables |
| 14. Confidence & certainty | partial | high for intra-file single candidate, typed first param, module-qualified calls; receiver calls and bare calls after `open` are medium |
| 15. Dynamic use & reflection | none | no dynuse coverage; reflection calls neither cap nor screen |
| 16. Known gaps & limitations | n/a | 15 gaps open at 0.13.3; stale rule gap texts |
