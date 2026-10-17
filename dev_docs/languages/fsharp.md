# fsharp

The F# plugin (`FsharpPlugin`, `crates/cgg-lang/src/plugins/fsharp.rs`) is 384 lines, inline tests included. It parses `.fs`, `.fsi` and `.fsx` with the crates.io grammar `tree-sitter-fsharp` 0.3.0 (`Cargo.toml:116` pins `"0.3"`; `Cargo.lock:1518-1519` resolves 0.3.0). It always uses `LANGUAGE_FSHARP` (fsharp.rs:18-20). Nothing is vendored. The crate also ships a `LANGUAGE_SIGNATURE` grammar for signature files (`tree-sitter-fsharp-0.3.0/bindings/rust/lib.rs:32`), and the plugin does not use it.

One recursive `FsharpWalker` (fsharp.rs:41-291) handles five node kinds:

- `anon_type_defn` pushes a scope.
- `function_or_value_defn` becomes a definition, but only when it has a `function_declaration_left`.
- `member_defn` becomes a method.
- `import_decl` becomes an `open` import.
- `application_expression` becomes a call site.

Everything else is only descended into. The plugin overrides none of these: `signals()` (the default is all `false`, lib.rs:304-306), `shebangs`, `resolver_kind`, `ts_language_for`, `max_line_bytes`. It does not use the shared `locals`, `registrar`, `dynuse`, `closure_types`, `receiver::peel`, `cfg` or `markup` helpers (grep for `fsharp` in those files returns nothing). There is no resolver code of its own. F# appears by name in only these places:

- `intra_file.rs:127`: `lexically_scoped`.
- `intra_file.rs:405`: ML-style positional shadowing.
- `intra_file.rs:846`: `binds_same_name_set`.
- `cross_file.rs:1916` and `2064`: `open` handling.
- `cross_file.rs:4137`: R15, a bare call never reaches a member.
- Seven `RuleSpec`s in `rules.rs`.

Fixtures cited below are under `<scratch>/parity/fsharp/` (f1 … f11). Each was run with `target/release/cgg` 0.11.0 as `-t json -o out.json` and read with `show.py`/`aud.py` in that directory. The `--dead-code` runs used `--dead-code-format json --dead-code-confidence low -o dc.json`.

## 01. Detection & parsing

- **Extensions `.fs`, `.fsi` and `.fsx`** (fsharp.rs:15-17).
  - Fixture f7: `W.fs` → `extension:.fs`, `W.fsi` → `extension:.fsi`, `script.fsx` → `extension:.fsx`.
  - Test `detect_prefixes.rs:68` generates F# fixtures as `open {prefix}` in `.fs`.
- **Shebangs: none.** Fixture f7: an extensionless `runme` beginning `#!/usr/bin/env -S dotnet fsi` → `file_skipped` `unknown-extension`.
- **Content detection: none.** `detect.rs` has no F# entry (grep).
- **One grammar for every extension.** `.fsi` signature files are parsed with the implementation grammar and not with `LANGUAGE_SIGNATURE`. Fixture f7 `W.fsi`:
  - `val mk : unit -> Widget` yields no callable.
  - It yields a false call site named `val` (an external record, `src: null`, line 2).
- **No re-parse or fallback.** `#if/#else` is not blanked: both branches are parsed and extracted (§08).
- **Depth skip**: shared. A file deeper than `MAX_TREE_DEPTH` (4,000) is skipped as `TooDeep` (`cgg/src/lib.rs`, `cgg_lang::exceeds_depth`). Not exercised for F# (unverified for this language).
- **Minified / long line**: `max_line_bytes` is not overridden, and there is no locals table, so the 2,000-byte locals rule has nothing to skip. Not exercised.
- **Parse status**: every fixture parsed `ok`. F# files with syntax errors are still extracted (generic behaviour, unverified for F#).

## 02. Callables extracted

There are only two variants:

- `FreeFunction` (kind `function`), for a `let` with arguments outside any type scope.
- `InherentMethod` (kind `method`), for a `let` inside a type, or any `member_defn` (fsharp.rs:157-161, 211).

The qualified name is `scope.join(".") + "." + name`. The scope stack is pushed **only** by `anon_type_defn`, which is a `type T(...) =` class (fsharp.rs:75-89, 51-57).

Supported:

- **Curried / tupled `let` functions**: `let helper x = x + 1`, `let add a b = a + b`.
  - Node `helper` (FreeFunction).
  - Test `let_binding_with_args`. Fixture f1.
- **`let inline`, `let rec`**: `let inline twice f x = …` and `let rec fact n = …` → nodes `twice` and `fact`. Fixture f1.
- **Nested `let` functions**: `let outer y = let inner z = … in inner y` → separate node `inner`, unqualified. The edge `outer → inner` is `high`. Fixture f1.
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

- **Module and namespace scope is not part of the qualified name.**
  - `module Lib.A`, `namespace Acme.Core`, `module Util = …` and nested `module Nested = …` push no scope.
  - Fixture f1: `Nested.deep` is node `deep`. Fixture f2: `Acme.Core.Util.format` is node `format`.
  - Two modules' same-named functions therefore get identical QNs (fixture f3: `Util.only` and `Other.only` are both `only`).
- **Record, union and type-extension members have no owner.** `record_type_defn`, `union_type_defn` and `type_extension` do not push scope.
  - Fixture f1: `{ X: int } with member this.Double()` → `Double`; the union member `Tag` → `Tag`.
  - Fixture f4: `type System.String with member this.Shout()` → `Shout`.
- **Value bindings are never callables.** This includes:
  - `let value = 42`;
  - point-free functions: `let partialApp = classify`, `let helloHandler = text "hi"`, `let webApp = choose […]`;
  - Expecto `let tests = testList …`.

  A `let` with no `function_declaration_left` is skipped (fsharp.rs:138-141). Fixtures f1, f4, f10.
- **Operator definitions**: `let (|>>) a b = b a` → no node, because the plugin requires an `identifier` child (fsharp.rs:142-144). Its body's call `b` is then recorded with `src: null`. Fixture f1.
- **Active patterns**: `let (|Even|Odd|) n = …` → no node. Fixture f4.
- **Lambdas**: `fun q -> …` → no node. Their calls are attributed to the enclosing function. Fixture f1, line 26.
- **Constructors**: there are no constructor nodes.
  - Neither the primary constructor of `type Service(name)` nor `new() = Service("x")` (`additional_constr_defn`, grammar.js:1868) produces a node.
  - The `new()` body's call is recorded with `src: null`. Fixture f1, line 45.
- **Auto-properties**: `member val Name = … with get, set` → no node. Fixture f1.
- **Abstract slots and interface declarations**: `abstract member Virt : unit -> int` and `type IShape = abstract member Area : …` → no node.
  - `member_signature` is not handled.
  - Fixture f1: `IShape` declares nothing, and `Service.Virt` exists only through its `default`.
- **Signature-file `val` declarations**: no node. Fixture f7.
- **`extern` declarations** (`[<DllImport>] extern int native_add(…)`, `extern_binding`, grammar.js:183): no node. Fixture f8.
- **Top-level script code** in `.fsx` and `do` blocks: no node. Calls there get `no-enclosing-callable`. Fixture f7.
- **Visibility is not recorded**: `let private hidden` and `let internal inner` have empty `visibility` (fsharp.rs:167, 217). Fixture f6.
- **Generated code**: none (§08).

## 03. Call-site forms

Only `application_expression` produces a site (fsharp.rs:115-119, 228-290). The walker descends leftward through nested applications to the callee. It records the callee in one of two ways:

- A `long_identifier` is split on identifiers: the last one is the name and the prefix, joined with `.`, is `receiver_hint`.
- A plain `identifier` is recorded as is.

No `arity`, `dot_call`, `value_ref`, `typed_receiver`, `path_receiver`, `paren_less` or `implicit_receiver` field is ever set.

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

Not supported or partial:

- **A receiver of two or more segments drops the site with no audit record.**
  - Affected calls, all in fixtures f1, f8 and f9: `X.Y.zed 1`, `X.Y.zed(1)`, `x.y.zed 1`, `Nested.Deeper.deeper ()`, `S2.M.Over(1)`, `System.Activator.CreateInstance(t)`, `w.Value.Render()`.
  - None produces an edge, an unresolved record or an external record, even with `--include-external --include-stdlib` (fixture f8 `ext.json`).
  - Probable cause: the grammar parses these as `dot_expression` (grammar.js:931), which `record_call`'s `match` drops in its `_ => {}` arm (fsharp.rs:250-285). The exact parse is unverified.
- **Chained calls** `o.ToString().Trim()` and `d.Use().ToString().Length` record only the first call. Fixtures f9 and f4.
- **Calls on a parenthesised receiver**: `(makeIt ()).Go()` and `(a :> Animal).Speak()` record no `Go`/`Speak` site. Fixtures f9 and f5.
- **Calls on a record expression**: `{ X = 1 }.Double()` is not recorded. Fixture f1.
- **Pipelines and composition are not calls**:
  - `1 |> zed`, `zed <| 1`, `zed >> zed` and `x |> ignore` record nothing for `zed`/`ignore` (`infix_expression` is not handled). Fixture f9.
  - Only an application on either side is recorded, for example `List.iter (fun …)` in `[1;2] |> List.iter (fun …)`.
  - This is the dominant F# call style, so the loss is large.
- **User-defined operator calls**: `3 |>> helper` → nothing. Fixture f1.
- **Constructor calls** `Service("a")`, `Widget(3)` and `Circle(1.0)` are recorded as calls of the type name. They never bind, because no constructor node exists; they end as `no-candidate-in-file`. Fixtures f1, f2, f7.
- **Functions passed as values**: `List.map helper […]`, `List.map classify`, `routef "/x/%s" handleGet`.
  - No value reference is recorded, even with `--reference-edges`. Fixtures f1, f4, f10.
  - In f10 `handleGet` is reported dead with evidence `no-unresolved-site-with-name`.
- **Duplicate sites for curried applications.** In `List.map helper [1;2;3]` and `printfn "%d" x`, each nested `application_expression` records its own site, and the leftward descent reaches the same callee every time (fsharp.rs:115-118 calls `record_call` and then `walk_children`). Fixture f1: two `map` and two `printfn` external records at the same byte.
- **Registrar arguments**: not captured. The plugin never calls `registrar::*` (grep), and no F# rule declares registrars.
- **Arity**: never recorded (`RefRecord::arity` is not set; fsharp.rs:267-283).

## 04. Imports & module resolution

- **`open A.B`** → `ImportRecord { kind: "open", path: "A.B" }` (fsharp.rs:100-113). Test `open_is_import`.
- **Cross-file handling** (cross_file.rs:1914-1925 and the generic branch at 2064-2079):
  - The path is pushed as an unqualified prefix.
  - Its last segment becomes a module alias (`Util` → `Acme.Core.Util`).
  - Step 1b tries `<prefix>.<name>` (cross_file.rs:5943-5970). It is certain only when exactly one opened prefix declares the name.
  - Because F# qualified names never carry the module (§02), this `<prefix>.<name>` lookup cannot match a module-level function.
- **What resolves in practice** (observed; exact mechanism unverified):
  - **Bare call after `open M`, where `M` matches a file stem.** Fixture f3: `open Other` + `only ()` binds only `Other.fs`'s `only` (medium). Without the `open`, `only ()` is `candidates-in-other-files` (2).
  - **Module-qualified call whose module was opened and matches a file stem.** Fixture f2: `open Acme.Core.Util` + `Util.format 1`, with the module in `Util.fs` → `high`.
- **What fails**:
  - **Module-qualified call without an `open`**: `Util.format 1`, `Other.render ()` and `Y.zed 1` → `candidates-in-other-files`, never bound. Fixtures f3 and f9.
  - **Opened module in a file with a different stem**: `module Util` inside `Helpers.fs`, opened as `open Acme.Core.Util`. `Util.format 1` is unresolved, and the bare `format 2` fans out to both `Helpers.fs` and an unrelated `Dup.fs` (medium). Fixture f11.
  - **Bare call with several opens**: in fixture f2, `parse "x"` bound both `Util.parse` and the never-opened `Other.parse` (medium).
- **`open type X`** is accepted by the grammar (grammar.js:284). The plugin reads only the `long_identifier`, so it is recorded like a plain `open` (unverified by fixture).
- **Module abbreviations** `module U = Acme.Core.Util` (`module_abbrev`) are not recorded. `U.format 2` → unresolved `candidates-in-other-files`. Fixture f2.
- **`#load "x.fsx"` / `#r "nuget: …"`** (`fsi_directive_decl`): not recorded (no handler).
- **Namespace / package records**: `FileFacts::packages` is never filled. `namespace`/`module` declarations are ignored.
- **Manifest reading**: none. `.fsproj` compile order and `paket.dependencies` are not read; `manifest.rs` has no F# entry.
- **External-module refusal (`heads.rs`)**: F# is not covered. The `match lang` arms are Python, JS/TS, Go, Java/Kotlin, C#, Scala, Solidity and C++ (heads.rs:413-625).
  - Calls like `List.map` and `printfn` reach the external bucket only through the generic name screen (`name_screen_applied: "external"`, stage `no-candidate-in-file`). Fixture f1.
- **`not-in-imported-module` (`modules.rs`)**: not covered. `modules.rs:190` lists only JS, TS, Python, Go and Lua.
- **Language family**: C#/F# family sharing is commented out (lang_family.rs:34), so there is no C# ↔ F# resolution.
- **Test**: `fsharp_bare_call_never_reaches_a_member` (cross_file.rs:13535-13563, R15). With `open Paket.Domain`, a bare `PackageName` does not bind a type's member `PackageTypes.PackageName`.

## 05. Type inference & receiver typing

The plugin records no `local_types`, `scoped_types`, `class_fields`, `arg_type` or typed receivers. All typing comes from the generic `type_hints` passes over `signature_hint`.

Supported:

- **Annotated parameter in the first parenthesised list**: `let typed (x: Widget) = x.Render()` and `let tupled (g: Gadget, w: Widget) = w.Render()`.
  - The receiver is typed and the edge is `high`.
  - Mechanism: the generic `name: Type` parse (`parse_colon_param`, type_hints.rs:1446-1462).
  - Fixtures f2 and f7. In f9, `o.ToString()` was rewritten to receiver `obj`.
- **`this.` receivers**: `this.Run()` and `this.Helper()` bind to the enclosing type (or the single same-named method) at `high`. Fixtures f1 and f4.

Not supported or partial:

- **Curried parameters after the first group**: in `let curried (g: Gadget) (w: Widget) = w.Render()`, `w` is untyped, and `Render` fans out to `Widget.Render` and `Gadget.Render` (medium). Fixture f7.
- **Locals from construction**: `let w = Widget(3)` then `w.Render()` → medium by name. Fixture f2. Fixture f1: `s.Run()` → medium.
- **Pattern-annotated locals**: `let (w3: Widget) = Widget.Make()` → `w3.Render()` medium. Fixture f2.
- **Return-type annotations**: `let mk () : Widget = …` then `let w = mk ()` → `w.Render()` fans out to both owners (medium). Fixture f7. `extract_return_type` has no F# `) : T` shape (type_hints.rs:2632-2671).
- **Lambda parameter annotations**: `fun (x: Widget) -> x.Render()` → medium fan-out. Fixture f7. `closure_types.rs` has no F# entry.
- **Upcasts and downcasts** `(a :> Animal)` and `:?>`: not peeled, because `receiver::peel` is not used and the site is dropped (§03). Fixture f5.
- **Option wrappers**: in `(w: Widget option)` → `w.Value.Render()` the site is dropped (multi-segment receiver). F#'s postfix `option` is not exercised through `declared_type` (unverified).
- **Implicit receivers** (Kotlin-style) and **weak receiver types**: not applicable; nothing is recorded.

## 06. Inheritance, dispatch & overloading

- **Base records**: none. `inherit Base()` and `interface I with` are not recorded as bases. The plugin never fills `base_types` or `classes` (fsharp.rs).
- **`base.M()`** is resolved by name.
  - Fixture f5: inside `Dog.Speak`, `base.Speak()` → `Animal.Speak` **and** `Cat.Speak` (medium, `cross-file:imports`). Cat is a sibling, so this is a wrong edge.
  - Fixture f4 got the single right answer (`high`) only because the base was literally named `Base`. That matches the receiver text `base` case-insensitively (coincidence; unverified mechanism).
- **Inherited `this.M()`**: `this.Helper()` in `Derived` → `Base.Helper`, high, but only because exactly one `Helper` exists in the file. Fixture f4. There is no `resolve_via_bases` support without base records.
- **Untyped receiver fan-out**: `x.Speak()` → all three `Speak` (medium; `?` in mermaid). Fixture f5.
- **`member_of_calling_class` / `overrides_below`**: F# is not wired into either path. `bare_call_reaches` treats F# like Python/Rust: a bare name never reaches a member (cross_file.rs:4067-4069, 4137-4139).
- **Interface → implementation**: abstract members are not nodes (§02), so no declaration exists to dispatch from.
- **`--dynamic-dispatch` / `inheritance_fanout`**: F# is not in `INHERITANCE_LANGS` (dispatch.rs:110-123). Fixture f5 with `--dynamic-dispatch`: no `dyn` edges.
- **Type extensions / extension members**: `type System.String with member this.Shout()` → owner-less `Shout`. `s.Shout()` on `(s: string)` binds by name (medium). Fixture f4.
- **Overload sets**: same-file, same-QN overloads are one callee (`binds_same_name_set` includes `fsharp`, intra_file.rs:829-850).
  - Fixture f1: `Service.Create("b")` → both `Service.Create` overloads, medium.
  - CHANGELOG.md:258-262 (overload sets past the cap bind in F#, +242 edges).
- **Arity narrowing**: F# is not in `OVERLOAD_ARITY_LANGS` (`["java", "scala", "cpp"]`, type_hints.rs:1583), and no arity is recorded.
- **Mixins and traits**: not applicable.

## 07. Scoping & shadowing

- **`FileFacts::locals`**: not populated. `locals::spec` has no F# arm (locals.rs:380-393).
  - Consequence: no `shadowed-by-local`. A call through a parameter binds to another file's same-named function.
  - Fixture f8: `let apply g = g 1` → `Other.g` (medium). `let shadowParam handler = handler ()` → `Other.handler` (medium). Both are wrong edges.
- **Lexical narrowing of nested definitions**: supported (`lexically_scoped` includes `fsharp`, intra_file.rs:118-135; CHANGELOG.md:576-582).
  - Fixture f4: in `let shadow () = let fetch () = 2 in fetch ()`, the call binds the inner `fetch` (high), not the module-level one.
  - Fixture f1: `outer → inner`.
- **Positional (ML-style) shadowing**: supported. A later same-scope binding before the site wins (intra_file.rs:400-416, `"ocaml" | "fsharp" | "lua"`).
  - Fixture f8: with two `cond` definitions in `#if/#else`, `useCond` binds the second (line 16) at high.
- **Conditional definitions**: no `DefRecord::conditional` (§08).
- **Module scope**: since modules are not scopes (§02), nested-module functions are lexically top-level for narrowing purposes.

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
| `azure-functions` (rules.rs:2518-2541) | network | `Microsoft.Azure.Functions.Worker`, `Microsoft.Azure.WebJobs` | attributes `Function`, `FunctionName` | **Yes** |
| `fsharp-entrypoint` (rules.rs:6097-6113) | cli | `System`, `Argu`, `CommandLine` | none | No, detect-only gap |
| `giraffe` (rules.rs:6114-6130) | network | `Giraffe` | none | No, gap |
| `saturn` (rules.rs:6131-6147) | network | `Saturn` | none | No, gap |
| `suave` (rules.rs:6148-6164) | network | `Suave` | none | No, gap |
| `expecto` (rules.rs:6165-6181) | test | `Expecto` | none | No, gap |
| `fsharp-nunit` (rules.rs:6182-6207) | test | `NUnit.Framework`, `FsUnit`, `NUnitLite` | attributes `Test`, `TestCase`, `TestCaseSource`, `SetUp`, `TearDown`, `OneTimeSetUp`, `OneTimeTearDown`, `Fact`, `Theory` | **Yes** (§11) |

Notes on each rule:

- **`azure-functions`**: `[<Function("X")>] let run …` mints `<framework-entry>::network::azure-functions::Function("Hello")` with an `entry` edge.
  - Test `azure-functions/fsharp` (crates/cgg/tests/frameworks.rs:1278-1282). Fixture f10.
  - The trigger kind on the first parameter is not read (gap text).
- **`fsharp-entrypoint`**: it detects the framework but has no matchers. `[<EntryPoint>]` is not a matcher, and `main` is rooted only by name (§12).
- **`giraffe` and `suave`**: `route … >=> handler` and `choose […]` live in operator and list position, which is not recorded. Fixture f10 prints "seen, no rules giraffe — entries NOT enumerated".
- **`saturn`**: computation-expression `router { … }` operations are not recorded.
- **`expecto`**: `[<Tests>]` values are not callables, and argument-position lambdas are not recorded. Fixture f10 prints "seen, no rules expecto".
- **`fsharp-nunit`**: matches attributes on `let` functions and members.
- **Stale gap text**:
  - The `expecto` gap says "the fsharp plugin populates no attributes".
  - The `fsharp-nunit` gap says the plugin "hardcodes `attributes: Vec::new()`".

  Both have been false since 0.6.8 (fsharp.rs:168-173), and fixtures f6 and f10 show attributes recorded and `fsharp-nunit` matching.
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

- **Capability class: `degraded`**, with `"max_confidence": "medium"`. Fixture f6 `capabilities`:
  - visibility `none`
  - attributes `full`, measured from the run even though `signals().attributes` is false
  - value_references `none`
  - dispatch `none`
  - exports `none`
  - test_tagging `none`

  `signals_complete` needs both visibility and attributes (caps.rs:88-96, 166-172), so **no F# finding can be `high`**.
- **Evidence on every finding** (fixture f6): `language-lacks-dispatch-model`, `language-lacks-value-references`, `language-lacks-visibility`, `no-incoming-edges`, `no-unresolved-site-with-name`.
- **Roots**:
  - `main` by name (`UNIVERSAL_ENTRY`, roots.rs:20). F# has no `LANG_ENTRY` row (roots.rs:23-32).
  - Framework entries and test harnesses (§10, §11).
  - `cgg-deadcode.toml` user roots (generic).
- **`[<EntryPoint>]` with a name other than `main` is reported dead.** Fixture f10: `[<EntryPoint>] let start argv = 0` → finding `start` (medium).
- **Visibility**: `private` and `internal` are not recorded, so `hidden`/`inner` are judged the same as public functions (fixture f6). There are no public/exported roots.
- **Caps**:
  - `implements-external-contract`: never applied. `override` members are not recognised as overriding anything, and there are no base records.
  - `macro-generated`: not applicable.
  - `reflection`: not applicable (§15).
- **False positives the gaps cause**:
  - functions used only as values (`handleGet` in f10, `classify` passed to `List.map`);
  - functions called only via `|>`/`<|`/`>>`;
  - functions called through a multi-segment path (§03);
  - Giraffe/Suave/Saturn handlers;
  - Expecto tests.

  The `no-unresolved-site-with-name` evidence is then misleading, because those sites were never recorded.
- **Unreachable code** (`unreachable`): not declared, so there are no statements-after-terminator findings.

## 13. Builtins & stdlib tables

- **There is no `stdlib/fsharp.txt`.**
  - The `include_str!` list (stdlib.rs:184-213) has no F# entry, and `crates/cgg-core/src/stdlib/` has no such file.
  - `printfn`, `List.map`, `string`, `ignore` and the like go to the external bucket through the generic name screen. With `--include-stdlib` they never become `std` exit nodes. Fixtures f1 and f8: `0 stdlib`.
- **`rust_types`-like wrapper tables**: none for F# (`option`, `list`, `Async<T>`, `Task<T>`).
- **`CAST_INTRINSICS` / `CONSTRUCTING_METHODS`**: unused, because the plugin does not call `receiver::peel`.
- **Implicit packages** (like `SCALA_IMPLICIT_PACKAGES`): none. F#'s auto-opened `FSharp.Core` modules are not modelled.
- **`--include-external`**: works generically. Fixture f8: `<external>::t::GetMethod`.

## 14. Confidence & certainty

- **`high` is reached through these paths**:
  - **Intra-file single candidate**: a bare call to a same-file `let` (`usesThings → helper`), the innermost nested definition, positional shadowing, and `this.M()` with one candidate. Fixtures f1, f4, f8.
  - **Annotated first-group parameter receiver naming exactly one type** (`typed → Widget.Render`, `tupled → Widget.Render`). Fixtures f2 and f7.
  - **Module-qualified call through an opened module whose file stem matches** (`Util.format`, `Widget.Make`, `cross-file:imports`). Fixture f2.
- **`medium` is forced by**:
  - by-name fan-out on untyped receivers (`s.Run()`, `x.Speak()`, `base.Speak()`);
  - same-QN overload sets (`Service.Create`);
  - bare calls after `open` (fixture f3: `run2 → Other.only` medium);
  - a parameter shadow bound to another file's function (fixture f8).
- **Framework entries** are `low` (fixtures f6 and f10).
- **`low` through `--guess-unique-names`**: Rust only; not applicable.
- **The mermaid `?` marker** prefixes every non-high arrow label (fixture f5: `N1 -->|?| N0`).
- **Measured history**: CHANGELOG.md:1440-1446 records "F# high 0 / 30, medium 0 / 12" from the type-argument fix, and CHANGELOG.md:1338-1341 records "F# +0.4k / −1.0k".

## 15. Dynamic use & reflection

- **Not covered.** `dyn_uses` is not declared, and `dynuse.rs` covers Python, JS, Java, C#, Ruby and PHP only (CLAUDE.md, step 7; grep shows no F# entry).
- `t.GetMethod("Length")`, `Activator.CreateInstance(t)`, `typeof<…>` and `nameof` do not cap any finding.
  - Fixture f8: `GetMethod` is an external call with receiver `t`.
  - `CreateInstance` is dropped entirely, because it is reached through a multi-segment receiver.
- **Dynamic operator `?`** and **`Assembly.Load`** module loads: not recognised (no handler).

## 16. Known gaps & limitations

From code, rule gap texts and CHANGELOG:

- `gap_for("fsharp-entrypoint" | "giraffe" | "saturn" | "suave" | "expecto", "fsharp")`: entries are not enumerated (rules.rs:6110, 6127, 6144, 6161, 6178).
- The `fsharp-nunit` and `expecto` gap texts are **stale**: they claim attributes are never populated (rules.rs:6178, 6205; contradicted by fsharp.rs:168-173 and fixture f6).
- CHANGELOG.md:1349: "Not covered (WS14): … F# `open`". F# has no module-scoped lookup.
- F#/C# family sharing is disabled (lang_family.rs:34).
- No `#[ignore]`d F# tests. The plugin has five inline tests (fsharp.rs:314-383) and one cross-file test (`fsharp_bare_call_never_reaches_a_member`), plus the cloud-framework case and the detection-prefix generator.

Found while cataloguing (fixtures f1–f11):

1. **Multi-segment receivers are silently dropped** (`A.B.f x`, `x.y.m()`, `System.Activator.CreateInstance`), with no audit record (§03).
2. **Pipelines, composition and operator calls are not call sites** (`x |> f`, `f <| x`, `f >> g`) (§03).
3. **Modules and namespaces are absent from qualified names**, so module-qualified calls resolve only by file-stem coincidence, and opens do not narrow reliably (§02, §04).
4. **No locals**: calls through parameters bind to other files' functions (§07).
5. **No base or interface records**: `base.M()` fans out to siblings, and `--dynamic-dispatch` does nothing (§06).
6. **Missing callables**: operators, active patterns, constructors, `member val`, abstract slots, `.fsi` `val`s, `extern` (P/Invoke), and point-free value bindings (§02).
7. **Record, union and type-extension members are owner-less** (§02).
8. **No visibility**, so dead code is capped at medium; **`[<EntryPoint>]` is not a root** (§12).
9. **No value references**: higher-order arguments look dead (§03, §12).
10. **Duplicate call sites** for curried applications (§03).
11. **`.fsi` parsed with the implementation grammar** produces a false `val` call site; `LANGUAGE_SIGNATURE` is unused (§01).
12. **`.fsx` shebang scripts without an extension** are not detected; top-level script calls are `no-enclosing-callable` (§01, §02).
13. **Return types, curried-parameter types, pattern annotations, lambda annotations and casts** are not used for receiver typing (§05).
14. **No stdlib table**, so `--include-stdlib` produces no F# `std` nodes (§13).
15. **`#if` branches are both extracted**, and the later one wins (§08).
16. **Quotations are treated as executed code** (§08).

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01. Detection & parsing | partial | `.fs/.fsi/.fsx` by extension; no shebang; `.fsi` uses the implementation grammar and yields a false `val` site |
| 02. Callables extracted | partial | `let` functions and class members; no module/namespace in QNs; no operators, active patterns, constructors, abstract slots, auto-properties; record/union/extension members owner-less |
| 03. Call-site forms | partial | bare, one-dot and type-applied applications; multi-segment receivers silently dropped; no pipelines/operators, chains, value refs or arity; duplicate curried sites |
| 04. Imports & module resolution | minimal | `open` recorded; resolution only by file-stem coincidence; no module abbreviations, packages, manifests, heads.rs or modules.rs |
| 05. Type inference & receiver typing | minimal | first parenthesised parameter annotations only; no locals, return types, curried params, lambdas or casts |
| 06. Inheritance, dispatch & overloading | minimal | same-QN overload sets bind at medium; no base records, `base.` fans to siblings, not in INHERITANCE_LANGS or arity langs |
| 07. Scoping & shadowing | partial | lexical narrowing and ML positional shadowing work; no locals, so parameters do not shadow |
| 08. Macros, preprocessor & codegen | n/a | no macros; `#if` not handled (both branches kept); quotations walked as code |
| 09. FFI & cross-language links | none | `extern`/DllImport not extracted; no C# family link; no markup code-behind |
| 10. Frameworks & entry points | partial | 7 rules: azure-functions works; giraffe/saturn/suave/fsharp-entrypoint/expecto are gaps; stale gap texts |
| 11. Test harness entries | partial | fsharp-nunit attributes mint a harness node; Expecto invisible; fixture members not rooted |
| 12. Dead-code analysis | partial | class `degraded`, max medium; no visibility; `[<EntryPoint>]` non-`main` reported dead; value-use false positives |
| 13. Builtins & stdlib tables | none | no `stdlib/fsharp.txt`, no wrapper or cast tables |
| 14. Confidence & certainty | partial | high for intra-file single candidate, typed first param, stem-matched module calls; most cross-file is medium |
| 15. Dynamic use & reflection | none | no dynuse coverage; reflection calls neither cap nor (for multi-segment) record |
| 16. Known gaps & limitations | n/a | 16 catalogued gaps; stale rule gap texts; WS14 "F# open not covered" |
