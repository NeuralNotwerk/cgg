# csharp

The C# plugin is `CSharpPlugin` in `crates/cgg-lang/src/plugins/csharp.rs` (1,261 lines, about 440 of them inline tests). It is one recursive tree walker, `Walker::walk` (csharp.rs:131-288), with a scope stack of namespaces and types. It records:

- namespaces, as `FileFacts::packages`
- `using` directives
- classes, structs, records and interfaces, with their bases (`FileFacts::classes`)
- methods, local functions, constructors, destructors and property accessors
- invocation call sites
- method groups in value positions, as value references
- explicitly typed locals and `var x = new T()` locals

The grammar is the crates.io crate `tree-sitter-c-sharp = "0.23"` (Cargo.toml:91; `tree_sitter_c_sharp::LANGUAGE`, csharp.rs:53-55). Nothing is vendored.

C# has no dedicated resolver kind. It runs through the generic pipeline: `intra_file`, then `cross_file`. C#-specific branches exist in several places:

| Area | Where |
| --- | --- |
| Namespace vocabulary, `global using`, external-head refusal | `heads.rs` |
| `member_of_calling_class`, `overrides_below`, overload sets, namespace-visible type narrowing, extension methods (step 4d), `using static` | `cross_file.rs` |
| Per-function local lookup | `type_hints.rs` |
| `INHERITANCE_LANGS`, `CLASS_RECORDS_ARE_EXACT` | `dispatch.rs` |
| Dead-code visibility model | `deadcode/mod.rs:670` |

The plugin declares these signals: `dyn_uses`, `visibility`, `attributes`, `impls` and `value_refs` (csharp.rs:42-51). It does not declare `unreachable`. There are 41 `RuleSpec`s with `language: "csharp"` in `crates/cgg-core/src/frameworks/rules.rs` (`grep -c 'language: "csharp"'`).

Fixtures used below live in `<scratch>/parity/csharp/f1` … `f11`. They were run with `target/release/cgg` (0.11.0 tree, branch `remediate/cross-language`). The dump helper is `dump.py` in the same directory.

## 01. Detection & parsing

- **Extensions `.cs` and `.csx`** (csharp.rs:39-41).
  - Test: detect.rs:690 `("l.cs", "csharp")` and the integration test crates/cgg/tests/detect.rs:76 `("cs1.cs", "csharp")`.
  - Fixture f7: `script.csx` was detected `extension:.csx` and its local function `ScriptFn` was extracted.
- **Shebangs**: none. The plugin does not override `shebangs()`, so a `#!/usr/bin/env dotnet-script` file without the extension is not detected. This is inferred from csharp.rs having no `shebangs` fn; there is no fixture for it.
- **Content detection**: none. detect.rs has no C# sniffing. The one C#-adjacent content rule goes the other way: a `.h` whose line begins `virtual` is classified as C++ (CLAUDE.md, cgg-lang section).
- **Grammar**: one grammar, `tree-sitter-c-sharp` 0.23. There is no `ts_language_for` override. Parser smoke test: parser.rs:157 `("csharp", b"class A { void M() {} }")`.
- **Re-parse fallback for `#if`/`#else`**:
  - When the tree has errors and the file contains an `#if`, the plugin first blanks every `#if/#elif/#else/#endif` line, keeping offsets and line numbers (`mask_conditionals`, csharp.rs:716-757). It then re-parses (`reparse_without_conditionals`, csharp.rs:683-698).
  - If that copy still has errors, it blanks every `#elif`/`#else` branch too and keeps only the first branch.
  - If neither copy parses cleanly, the original tree stands.
  - Test: `conditional_compilation_split_code_is_reparsed` (csharp.rs:874).
  - Fixture f2 `Cond`: a `#if DEBUG void F() { #else void F(int x) { #endif` header split gave one callable `Web.Cond.F`, and its call `G()` bound `high`.
  - CHANGELOG.md:848 records the fix. It used to produce callables named `if` in Newtonsoft.Json (CHANGELOG.md:813-814).
- **Depth skip**: a tree deeper than `MAX_TREE_DEPTH` = 4000 is skipped as `TooDeep` (crates/cgg/src/lib.rs:155, 451). This applies to every language.
- **Minified skip**: `MINIFIED_LINE_BYTES` = 2000 (lib.rs:159) gates only `locals::collect`. C# has no locals table, so it does not apply (section 07).
- **Test-file classification** (`cgg_core::classify_test_file`):
  - File names `*Test.cs` and `*Tests.cs` count (testfile.rs:104).
  - So does the generic directory rule, which matches exact components `test`, `tests`, `Tests`, `Test`, `spec` and others (testfile.rs:35-46, 126-133).
  - There is no .NET-specific structural rule. A project directory like `MyApp.Tests/` does not match. Fixture f7: `MyApp.Tests/Foo.cs` was not test-tagged, and its private `NeverCalled` was reported as a `high` production finding.
- **Sibling markup is read, not parsed** (`plugins/markup.rs:28-44`):
  - `X.xaml.cs`/`X.axaml.cs` read `X.xaml`/`X.axaml`; `X.razor.cs` reads `X.razor`; `X.cs` reads a same-stem `X.tscn`.
  - The read is capped at 2 MiB (markup.rs:19). See section 09/10.
  - The markup files themselves are skipped as `unknown-extension` (fixture f7: `Counter.razor` and `Main.tscn` skipped).
- **Not handled**:
  - `.razor`/`.cshtml` bodies (`@code { }` blocks) are not parsed as C#. Only their attribute pairs are scanned for event bindings.
  - `.csproj`/`.sln` are never read. There is no project or assembly model, and C# has no `manifest.rs` reading (section 04).

## 02. Callables extracted

Qualified names join the scope stack with `.`. The stack holds every enclosing block namespace (`namespace A.B { }`) and every enclosing type (csharp.rs:313-318). A **file-scoped namespace** (`namespace Acme.App;`) is pushed only while the walker visits that node's own children (csharp.rs:157-159). The type declarations after it are apparently not inside that node, so the namespace does not reach their names. This is inferred from fixture f1, not checked against the grammar: `App.cs` with `namespace Acme.App;` produced `Program.Main`, not `Acme.App.Program.Main`. The heads.rs comment "a file-scoped one does not [prefix the qualified name]" (heads.rs:846-847) agrees. The full namespace is still recorded in `packages` (csharp.rs:139-157).

- **Instance and static methods** (`method_declaration`) become `DefVariant::InherentMethod` with kind `method` (csharp.rs:199-213).
  - Static methods are **not** distinguished. There is no `StaticMethod` variant for C#, unlike Java.
  - Test: `method_qualified_name_with_namespace_and_class` (csharp.rs:983).
  - Fixture f1: `Acme.Lib.Svc.Run`, `Acme.Lib.Repo.Create`.
- **Local functions** (`local_function_statement`) become `InherentMethod`. Their QN uses the enclosing **type**, not the enclosing method.
  - Fixture f1: `void Local() { int Inner(int q) => q; }` gave `Acme.Lib.Svc.Inner`, not `…Local.Inner`.
  - Local functions inside top-level statements have no owner. Fixture f7: `TopHelper`, `Inner`.
- **Constructors** (`constructor_declaration`) become `DefVariant::Constructor` with QN `Ns.Type.Type` (csharp.rs:214-218).
  - Static constructors `static Svc()` are extracted as constructors too (Private visibility). Fixture f1: three `Acme.Lib.Svc.Svc` nodes (L13, L14, L15).
  - Test: `constructor_and_destructor_variants` (csharp.rs:1003).
- **Finalizers** `~Svc()` become `DefVariant::Destructor` named `~Svc` (csharp.rs:219-229). Fixture f1: `Acme.Lib.Svc.~Svc` kind `destructor`.
- **Property, indexer and event accessors** (`accessor_declaration`) become `DefVariant::Property`, named after their **keyword** (`get`, `set`, `init`), not after the property (csharp.rs:230-240).
  - Fixture f1: `public int Count { get {…} set {…} }` gave `Acme.Lib.Svc.get` and `Acme.Lib.Svc.set`. The indexer `this[int i] { get … }` gave a second `Acme.Lib.Svc.get`.
  - Interface property `int Count { get; }` gave `Acme.Lib.IRunner.get`.
  - So all accessors of one type share a QN, and dispatch deliberately skips them (dispatch.rs:393-397).
  - Test: `an_accessor_takes_its_property_accessibility_unless_it_has_its_own` (csharp.rs:1191).
- **Expression-bodied properties** `int Expr => Calc();`, arrow accessors without an accessor list and **field initializers** are not callables.
  - Their calls get `no-enclosing-callable`. Fixture f1: L20 `Calc`. Fixture f9: `System.Func<int> f = () => X();` (L2) and `int Field = X();` (L4).
  - `int Arrow { get => X(); }` **is** extracted (fixture f9: `P.get`, `get -> X` high).
- **Lambdas and anonymous methods** are **not callables**. Their calls are attributed to the enclosing method. Fixture f1: `System.Func<int,int> f = x => Calc();` inside `Local` gave the edge `Svc.Local -> Svc.Calc` high.
- **Overloads**: each overload is its own node with the same QN.
  - Fixture f1: `Acme.Lib.Svc.Over` ×3. Fixture f8: `X.C.Over` ×2.
  - Same-file same-QN sets bind as one callee (`binds_same_name_set`, intra_file.rs:830-850, lists `"csharp"`).
- **Interface members**:
  - Bodiless and default-implemented members are extracted, with Public visibility unless a modifier says otherwise (csharp.rs:799-807).
  - Test: `interface_members_without_a_modifier_are_public` (csharp.rs:1149).
  - Fixture f9: default method `P.IDef.D` and its call `X()` bound high.
- **Abstract members** `public abstract void Abs();` are extracted bodiless (fixture f1: `Acme.Lib.Base.Abs`). They are not tagged `abstract`; CLAUDE.md lists the `abstract` tag only for TS/Swift/Scala/Dart.
- **Structs, records, record structs** contribute scope (csharp.rs:165-169).
  - Fixture f1: `Acme.Lib.Pt.Move`, `Acme.Lib.Rec.R`, `Acme.Lib.RS.S`.
  - **Not extracted**: the positional record's synthesized constructor, properties and `Deconstruct`, because no declaration node exists for them.
- **Nested types** are scoped under the outer type (fixture f3: `Alpha.Util.Nested.Deep`; fixture f9: `P.Inner.K`).
- **Enums** produce no callables (fixture f1: `enum Color` gave nothing).
- **Operators and conversion operators are NOT extracted**. `operator_declaration` and `conversion_operator_declaration` are not in the walker's match. Fixture f1: `public static Svc operator +(…)` and `implicit operator int(Svc s)` produced no nodes.
- **Events**: `event EventHandler Changed;` produces nothing. Explicit `add`/`remove` accessors would be `Property` accessors (unverified by fixture).
- **Delegates** (`delegate void D();`) produce no callable.
- **Top-level statements** (C# 9 `Program.cs` without a class) are not wrapped in a synthetic `Main`.
  - Their calls get `no-enclosing-callable`, and the dead-code pass roots the targets as `toplevel:invocation`.
  - Fixture f7: `TopHelper();` at L3 unresolved/no-enclosing-callable, and `TopHelper` rooted `top-level-invocation`.
- **Partial methods**: `partial void OnLoaded();` and its implementing part are both extracted. Each carries the `PARTIAL_DECLARATION` attribute (csharp.rs:200-210).
  - Test: `construction_receivers_name_their_type_and_partial_methods_are_marked` (csharp.rs:896).
  - Fixture f2: two `Web.Part.OnLoaded` nodes.
- **Partial classes** across files are separate declarations with the same owner QN. Nothing merges them, but C# `private` reachability treats them as one class (section 12).
- **Attributes** are kept verbatim through `attrs::collect` (csharp.rs:330). Examples: `[HttpGet("/users")]` and `DllImport("libm")` (fixture f6).
- **`override` marker**:
  - The plugin appends `override` for the `override` modifier and for an explicit interface implementation (`void IFoo.Bar()`) (csharp.rs:331-344).
  - Test: `base_receiver_reads_as_super_and_overrides_are_marked` (csharp.rs:1209).
  - Fixture f1: `Svc.Hook`/`Svc.Abs` attributes `['override']`.
- **`base_types`**:
  - Each method carries its enclosing type's base list (csharp.rs:345).
  - Every type, bodiless or not, is also recorded as `ClassDecl` with its exact bases (csharp.rs:170-189). Generic and qualified bases are stripped to bare names on the resolver side (`push_bare_base`, cross_file.rs:417-424).
- **Visibility** (`csharp_vis`, csharp.rs:774-819) is read from `modifier` token nodes:
  - `public` > `protected` > `private` > `internal`. So `protected internal` and `private protected` both read as Protected.
  - Without a modifier: an accessor inherits its property's visibility, an interface member is Public, and everything else is Private. That includes class, struct and record members, local functions, static constructors, finalizers and explicit interface implementations.
  - Tests: `an_attribute_argument_does_not_hide_the_access_modifier` (1105), `modifier_words_outside_modifier_nodes_are_not_modifiers` (1129), `combined_access_modifiers_reach_subtypes` (1139), `class_struct_and_record_members_default_to_private` (1174).
  - **Partial**: the default for a top-level type's member is applied, but a member of an `internal` type declared `public` reads as Public. The enclosing type's accessibility is not intersected (by code reading of csharp.rs:774-819).
- **Signature hint**: `extract_signature` of the declaration text (csharp.rs:329). It is read by extension-method detection, closure-arg settling and markup handler matching.
- **Not extracted**: fields, auto-properties without bodies (`{ get; set; }` gives `get`/`set` accessor nodes with no body; unverified whether bodiless accessors are kept outside interfaces), operators, delegates, events, source-generator output, records' synthesized members, `[DllImport]`-less `extern` methods (unverified).

## 03. Call-site forms

All call sites come from `invocation_expression` (csharp.rs:267-286, `ref_from_invoke` 564-618).

- **Bare call** `Helper()`: name only, empty receiver (csharp.rs:567). Fixture f1: `Svc.Run -> Base.Helper` high (inherited, intra-file).
- **Member call** `obj.M()`, `Ns.Type.M()`: receiver is the expression text (csharp.rs:568-585).
  - Fixture f1: `s.Run()` gave `Svc.Run` high.
  - Fixture f3: `Alpha.Util.Nested.Deep()` gave high; `Beta.Util.Go()` gave medium.
- **`this.M()`**: receiver `this`. Test: `base_receiver_reads_as_super_and_overrides_are_marked` records `("Paint","this")`.
- **`base.M()`**: receiver rewritten to `super` (csharp.rs:573-578), so the resolver's base-chain reading applies.
  - Fixture f1: `Svc.Hook -> Base.Hook` high.
  - Fixture f3: `base.OnLoad()` with an external base became `super-base-out-of-graph`.
- **Constructions and casts as receivers** (`receiver::peel`, csharp.rs:621-629; receiver.rs:456 test `csharp_as_await_and_initializers`):
  - `new Foo().M()`, `new Foo { X = 1 }.M()`, `((Foo)o).M()` and `(o as Foo).M()` mark `typed_receiver` with type `Foo`.
  - `(x).M()`, `(await t).M()` and `(f!).M()` read as `x`/`t`/`f`.
  - Fixture f1: `((Svc)null).Hook()` gave `Svc.Hook` high, and `new Repo().Shout()` gave the extension `Ext.Shout` medium.
- **Null-conditional** `x?.M()` and `x?.y.M()`:
  - The binding names the method. The receiver is the condition, and `x?.y` is spelled `x.y` (csharp.rs:591-599, 648-660).
  - Test: `generic_member_and_null_conditional_calls` (csharp.rs:1239).
  - Fixture f1: `s?.Run()` gave high.
- **Explicit generic arguments** `G<int>()`, `b.M<string>()`, `B.S<int>()`, `b?.M<int>()` are recorded under the bare name (`callee_name`, csharp.rs:633-646). Fixture f1: `Generic<int>()` gave `Program.Generic` high.
- **Chained calls** `a.B().C()`: the receiver is the text `a.B()`. There is no return-type typing for C# (section 05), so the chain falls to by-name lookup.
  - Fixture f8: `Foo.Make().M()` unresolved `candidates-in-other-files` (2).
  - `new Foo().Self().M()` fanned out to `Foo.M`/`Bar.M` medium.
- **Constructor calls `new T(…)` are NOT call edges**. `object_creation_expression` is not recorded as a reference, and neither are `: this(…)`/`: base(…)` initializers.
  - Fixture f9: `new P(); new P(2);` and `public P() : this(1)` produced no edges to `P.P`.
  - Fixture f1: no edge `Main -> Svc.Svc`. Constructors appear in dead code only through `implicitly-invokable-kind` (section 12).
- **Delegate / local-function-variable invocation** `a()`, where `a` is a `System.Action` local:
  - It is recorded as a bare call.
  - Fixture f1: `a()` went to the external bucket (`name_screen_applied: external`).
  - Fixture f11b: `void F(System.Action Work) { Work(); }` with a same-class method `Work` gave a **false `high` edge** `B.F -> B.Work` (section 07).
- **Operator use** (`a + b`, implicit conversions, indexers `x[i]`, `foreach` → `GetEnumerator`, `using` → `Dispose`, `await` → `GetAwaiter`) produces **no call sites**. No operator nodes exist (section 02).
- **Method groups / value references** (only when `ExtractCtx::reference_sites`, i.e. `--reference-edges` or `--dead-code`; csharp.rs:249-266, 456-562):
  - Value positions read: `argument`, `assignment_expression` (right side, which covers `evt += H`), `variable_declarator`, `return_statement`, `arrow_expression_clause` and `initializer_expression`.
  - Recognised values:
    - a bare identifier
    - `this.H`
    - `base.H`, as `super`
    - `Type.H` / `Ns.Type.H`, when every segment is capitalised
    - a cast of a method group `(Del)F`
    - the branches of `c ? F : G`
    - parenthesised forms, up to depth 4
  - Each is recorded as a `RefRecord` with `value_ref: true`.
  - Tests: `method_groups_are_value_refs` (924), `member_reads_through_a_variable_are_not_method_groups` (963), `method_groups_need_reference_sites` (977).
  - Fixture f1: `Act(Handler)`, `new System.Action(Program.Handler)` and `System.Action a = Handler;` gave `Main -> Handler` high `reference`.
  - Fixture f2: `Changed += OnChanged` gave a `Wire -> OnChanged` reference.
  - Fixture f3: `nameof(Helper2)` gave a reference edge to `Helper2`, and `Func<int> f = Helper` too.
  - `value_refs::prune` drops a value ref no C# callable names (test `a_value_ref_naming_no_callable_is_dropped`, value_refs.rs:297).
- **Not recorded as method groups**: `obj.Handler` through a lowercase variable or field. The plugin rejects it deliberately (csharp.rs:494-500), and CHANGELOG.md:752 lists it under "Not covered (WS6)".
- **Registrar arguments**: every invocation is also passed to `registrar::capture` with context `recv.Name` (csharp.rs:276-281). This is how `app.MapGet("/x", Handler)` reaches the `aspnet-minimal` rule.
  - Fixture f2: entry `<framework-entry>::network::aspnet-minimal::mapget("/x") -> Startup.Handler`.
  - **Partial**: `registrar.rs` IDENT_KINDS (registrar.rs:41-62) lists `member_expression` but not C#'s `member_access_expression`. A handler passed as `Handlers.List` may not be captured (unverified by fixture).
- **Lambdas passed to registrars**: `app.MapPost("/y", () => Lambda())` gave a direct edge `Configure -> Lambda`, but **no** `aspnet-minimal` entry for `/y` (fixture f2: only the `mapget` entry was minted).
- **Arity**: the C# plugin never sets `RefRecord::arity`. C# is not in `OVERLOAD_ARITY_LANGS` (type_hints.rs:1583, `["java","scala","cpp"]`).
- **Not set for C#**: `paren_less`, `path_receiver`, `implicit_receiver`, `weak_receiver_type`, `arrow`, `invocant`, `arg_type` (`grep` of csharp.rs shows none of these fields assigned).

## 04. Imports & module resolution

- **`using Ns;`** becomes `ImportRecord { kind: "using", path: "Ns" }` (csharp.rs:425-454).
  - In `cross_file`, it registers the module alias `last-segment -> full` (cross_file.rs:1767-1775).
  - In `heads.rs`, it makes `Ns` and its prefixes visible, resolved against every enclosing namespace too: `using Pipeline;` inside `MediatR.Tests` can mean `MediatR.Pipeline` (heads.rs:546-583; test `csharp_using_inside_a_namespace_is_relative_to_it`, heads.rs:1219).
- **`using A = Ns.X;`** registers the alias `A -> Ns.X` (cross_file.rs:1770-1771) and binds it in heads (heads.rs:580-582). Fixture f1: `using R = Acme.Lib.Repo; R.Create()` gave `Repo.Create` high.
- **`using static Ns.Type;`** has kind `using-static`. Every member of `Type` becomes callable unqualified (cross_file.rs:1777-1789; bare-call check cross_file.rs:4199-4203).
  - Fixture f1: `Create()` gave `Repo.Create` high.
  - Fixture f3: `using static System.Math; Abs(-1)` went to the stdlib bucket.
  - Fixture f10b: `using static Lib.Helpers; Assist()` gave high, even with a second `Other.Assist` in the tree.
- **`global using Ns;`** is recognised by `cs_global_using` (heads.rs:869-879). It makes `Ns` visible to **every** C# file (heads.rs:160-164, 556). Fixture f5: `GlobalUsings.cs` with `global using Alpha;` gave `Util.Go()` in another file `Alpha.Util.Go` only, not `Beta.Util.Go`.
- **`global using static Ns.Type;` is NOT honoured**:
  - The plugin tests `starts_with("using static ")` on the raw text (csharp.rs:428), so a `global using static` directive becomes a plain `using` with path `global using static Lib.Helpers`.
  - `cs_global_using` then rejects it because it is static (heads.rs:875).
  - Fixture f10: `global using static Lib.Helpers;` + `Assist()` was unresolved `candidates-in-other-files` (2). The same directive as a file-local `using static` bound high (f10b).
- **Namespaces as packages**: every block or file-scoped namespace, nested parts joined with `.`, is pushed to `FileFacts::packages` (csharp.rs:134-163). Test: `declared_namespaces_are_recorded_in_full` (csharp.rs:1085). They feed `WorkspaceVocab::cs_prefixes`/`cs_roots`/`cs_file_namespaces` (heads.rs:152-166).
- **Namespace-visible type narrowing**:
  - A bare type name receiver (`Util.Go()`, or a local typed `Util`) reaches only types in namespaces the file can see: its own, enclosing ones, `using`s and every `global using`.
  - Code: `cs_scoped` (cross_file.rs:6458-6464) and `cs_candidate_visible` (heads.rs:836-865).
  - Test: `csharp_type_name_reaches_only_visible_namespaces` (cross_file.rs:11470).
  - Fixture f4: `using Alpha; Util.Go()` gave `Alpha.Util.Go` only (medium, because two `Util` owners exist).
- **External-module refusal** (heads.rs:665-680, 736, `cs_namespace_head` 815-834):
  - A dotted receiver headed by a root the file `using`s, or by a C# stdlib root, that no workspace namespace shares is recorded as `external-module`.
  - Test: `csharp_fully_qualified_system_receiver_is_external` (cross_file.rs:11510).
  - Fixture f1: `System.IO.File.ReadAllText` went to stdlib; `JsonConvert.SerializeObject` (with `using Newtonsoft.Json`) went to external.
- **Member of the calling class beats an import** for bare names (cross_file.rs:5353-5379, Java/Kotlin/Scala/C#).
- **Not applicable to C#**: `not-in-imported-module` and `ModuleIndex` (modules.rs) cover JS/TS/Python/Go/Erlang/Lua; `using` names namespaces, not files. There is no `manifest.rs` reading: `.csproj` `RootNamespace`, `PackageReference` and project references are ignored. `extern alias` is not read (`grep extern csharp.rs` finds nothing).
- **Copies of a class** in several directories narrow to the caller's nearest copy (`nearest_class_copies`, cross_file.rs:499; listed for Java/C# in CLAUDE.md). `unique_owner_names` keys C# owners by directory (cross_file.rs:4415). Test: `class_copies_narrow_to_the_callers_copy` (cross_file.rs:13298) uses C# facts.

## 05. Type inference & receiver typing

- **Explicitly typed locals** `Svc t = …;`, `Foo? x`, `Circle tt = new();` and `using var u = new Circle();` become `LocalType` (csharp.rs:351-423).
  - Nullable is unwrapped. Test: `a_nullable_local_is_typed_as_its_type` (csharp.rs:854).
  - Fixture f3: L23 `using var u = new Circle(); u.Draw()` gave `Circle.Draw` high. L25 `Circle tt = new(); tt.Draw()` gave high.
- **`var x = new T(…)`** types `x` as `T` (csharp.rs:394-408). Fixture f1: `var s = new Svc(); s.Run()` gave high.
- **`var x = <anything else>` is untyped**. Neither the return type nor a cast is read:
  - Fixture f8: `var q = (Foo)null; q.M()` and `var w = null as Foo; w.M()` fanned out to `Foo.M`/`Bar.M` medium.
  - Fixture f3: `var many = GetShape(); many.Draw()` fanned out to three medium edges.
- **Lowercase-initial type names are never typed**: `string`, `int`, `object`, `dynamic` (csharp.rs:409, 414 `starts_with(char::is_uppercase)`).
- **Per-function lookup**: C# (with Kotlin) looks a local up in the calling function before the file-wide map (type_hints.rs:785-799). The CHANGELOG.md:989 example is `JArray a` versus another method's `IEnumerable? a`.
- **Parameters**: the type-first parser `parse_type_first_param` (type_hints.rs:2166-2210):
  - It skips the modifiers `ref`/`in`/`params`/`this`/`scoped`/`readonly` (TYPE_FIRST_MODIFIERS, type_hints.rs:2139-2156) and `[Attr]` prefixes, and strips defaults.
  - It reads `Foo?` and `Nullable<Foo>` as `Foo` through `declared_type` with `MAYBE_TYPE_WRAPPERS` `("csharp", ["Nullable"])` (stdlib.rs:118).
  - Test: `nullable_type_first_parameters_are_their_type` (type_hints.rs:3445).
  - Fixture f8: `Foo? nf` → `nf.M()` high; `Nullable<Foo> nn` → `nn.M()` high. Fixture f1: `UseParam(Svc p) { p.Run(); }` high.
  - **Generic parameters type as the generic**: `List<Square> sq` is `List`, so `sq.First()` goes to stdlib.
- **Return types: NOT inferred for C#**. `extract_return_type` (type_hints.rs:2632-2671) reads `-> T`, `): T` and Go's `) T`. Its comment claims "Java/C#: return type is before the function name", but no branch reads a type before the name. Fixture f1: `var r = s.MakeRepo(); r.Save()` bound by name, medium. Fixture f8: `Foo.Make().M()` was unresolved.
- **Fields and properties are NOT typed**. The C# plugin records no `field_types`/`class_fields`.
  - Fixture f1: `_repo.Save()` with `private readonly Repo _repo` was unresolved `candidates-in-other-files` (1).
  - Fixture f3: `_field.Inst()` was unresolved (2), `Prop.Draw()` unresolved (3), and `this._field.Inst()` fanned out to `Alpha.Util.Inst` + `Beta.Util.Inst` medium.
- **Pattern, `out`, `foreach`, `catch` and deconstruction bindings are NOT typed**. `closure_types.rs` records C# typed patterns? No: only `lambda_expression` (closure_types.rs:38-50, 122-130).
  - Fixture f1: `foreach (Svc each in …) each.Run()` and `o is Svc pat; pat.Run()` were unresolved (2 candidates).
  - Fixture f3: `o is Circle cc`, `out Square outSq` and `var (a, b) = (new Circle(), 1)` fanned out medium.
- **Typed lambda parameters** `(Foo x) => x.Run()` are recorded by span in `FileFacts::scoped_types` and win over other bindings (closure_types.rs:38-50; test `csharp_typed_lambda_parameters_are_scoped_to_the_lambda`, closure_types.rs:318).
- **Untyped lambda parameters passed to a bare call** (`settle_closure_args`):
  - They are typed from the callee's declared `Action<T>`/`Func<T,R>` parameter, through `CALLBACK_TYPE_CONSTRUCTORS` `("csharp","Action",false)` and `("csharp","Func",true)` (stdlib.rs:144-145; tests at type_hints.rs:3608, 3634).
  - Fixture f1: `Lam(x => x.Run())` with `Lam(System.Action<Svc> f)` gave `Svc.Run` high.
  - It does not apply to member calls on external types: fixture f3 `sq.ForEach(s => s.Draw())` fanned out medium.
- **`TOP_TYPES`** `("csharp", ["Object","Dynamic"])` (stdlib.rs:153). These are capitalised, while C# writes `object`/`dynamic`. A lowercase type is already never typed, so the entry only matters for `System.Object`-style spellings. Fixture f7: `dynamic d; d.Zap()` was `fanout-cap-exceeded` (6).
- **Await**: `(await t).M()` with `Task<Foo> t` reads the receiver `t`, typed `Task` (the generic), so the call goes to stdlib. Fixture f8 L17: stdlib `Task.M`. CHANGELOG.md:1474-1476 lists this as not covered.
- **Casts/constructions in receiver position** are typed (section 03). `CAST_INTRINSICS` has no C# entry (C# casts are syntax, stdlib.rs:35-46), and neither does `CONSTRUCTING_METHODS` (stdlib.rs:58).
- **Not applicable**: implicit receivers (Kotlin), weak receiver types (Python), `#deref` (C++), `ARG_VAR_PREFIX` (Rust).

## 06. Inheritance, dispatch & overloading

- **Base records**: every type becomes a `ClassDecl` with exact bases (csharp.rs:170-189). C# is in `CLASS_RECORDS_ARE_EXACT` (dispatch.rs:130), so method `base_types` are not unioned into owners.
- **`base.M()`** becomes the receiver `super`, resolved up the base chain.
  - When the chain leaves the tree it is refused as `super-base-out-of-graph`.
  - Test: `java_and_csharp_super_into_an_outside_base_is_refused` (crates/cgg/tests/resolve.rs:2893).
  - Fixture f3: `class Ext : System.Windows.Forms.Form { base.OnLoad(null); }` gave `super-base-out-of-graph`.
- **`member_of_calling_class`** (cross_file.rs:566, used at 5650-5667): a bare call first tries the calling class by its qualified owner path, then its declared bases.
  - Fixture f1: `Svc.Run` → `Helper()` gave `Base.Helper` high.
  - Fixture f3: bare `Draw()` in `Caller`, which has no `Draw`, was refused `candidates-in-other-files` (3), not fanned out.
- **`overrides_below`**: a bare call to a member that derived classes override binds the declaration plus the overrides at medium. The derived index covers only `"java" | "csharp"` (cross_file.rs:429). Fixture f3: `Shape.Paint` → `Draw()` gave `Shape.Draw`, `Circle.Draw` and `Square.Draw`, all medium.
- **Inherited member through a typed receiver** uses `resolve_via_bases` (depth 8). It is exercised through the `Base.Helper` case above; there is no cross-file typed-receiver fixture.
- **Interfaces**: an interface-typed receiver binds the interface declaration. Fixture f3: `IThing thing; thing.Act()` gave `IThing.Act` high.
- **`--dynamic-dispatch` / `inheritance_fanout`**:
  - C# is in `INHERITANCE_LANGS` (dispatch.rs:108-121).
  - Base and interface members link to same-named overrides at `low` with `Via::Dynamic` (`dispatch:inheritance`). This also runs under `--dead-code`.
  - Static members and C# accessors never override (dispatch.rs:386-397; test `static_members_and_csharp_accessors_do_not_override`, dispatch.rs:751). Note that the static check depends on `StaticMethod`, which C# never emits (section 02).
  - Fixture f3 `--dynamic-dispatch`: `Shape.Draw -> Circle.Draw/Square.Draw` and `IThing.Act -> T1.Act/T2.Act`, all low.
  - Without either flag there are no dispatch edges (fixture f3 plain: 0).
- **Extension methods (step 4d)** (cross_file.rs:6779-6810, `csharp_extension_target` 4001):
  - For `x.M()` on an uppercase or typed receiver that neither `T` nor its bases declare, a static `M(this U …)` is accepted when `U` is `T`, a base of `T`, or a type parameter of `M`.
  - Test: `csharp_extension_targets_are_read_from_the_signature` (cross_file.rs:11789).
  - Fixture f1: `r2.Shout()` and `new Repo().Shout()` gave `Ext.Shout` medium.
  - Fixture f8: `s.MyExt()` on a `string` parameter and `xs.ListExt()` on a `List<int>` both bound medium (by name: the receivers are lowercase and untyped).
  - `_f.GenExt()` through an untyped field was unresolved.
  - Extension bindings are never `high`.
- **Overloading**:
  - A same-file, same-QN overload set is one callee: `binds_same_name_set` (intra_file.rs:841) and `with_overloads` for exact Java/C# lookups (cross_file.rs:5669-5681).
  - Over the fan-out cap, `one_method` still binds it (CHANGELOG.md:261: "Overload sets past the cap now bind in C# (+2,722)").
  - **No arity narrowing**: C# is not in `OVERLOAD_ARITY_LANGS` and the plugin records no arity. Fixture f8: `Over(1)` and `Over(1, 2)` each bound **both** overloads, medium. Fixture f1: `t.Over(1)` bound all three `Svc.Over` medium.
- **Explicit interface implementation** `void IThing.Tick()` is tagged `override` (dead-code contract handling, section 12). It is not modeled as reachable only through the interface for resolution. Its QN is `Ns.Type.Tick`.
- **Partial classes**: no merge. The members of each part are owned by the same QN, so owner lookups see them together.
- **Not modeled**: `new` (member hiding) versus `override` (both are just same-named members), `sealed`, generic constraints (`where T : IFoo` does not type `T`), default interface method dispatch, and covariance.

## 07. Scoping & shadowing

- **`FileFacts::locals`: not populated for C#**. `locals::spec` has no C# table (locals.rs:380-394: JS/TS, Python, Lua, Scala, Kotlin, Swift, Rust, Dart, OCaml, Zig).
- **Consequence: a parameter or local delegate shadowing a same-class method gives a false `high` edge.** Fixture f11b: `class B { void Work(){} void F(System.Action Work) { Work(); } }` gave `B.F -> B.Work` high intra-file.
- **Cross-file**: a bare call to a parameter delegate is not bound to another class's same-named method. `member_of_calling_class`/`bare_call_reaches` refuses non-members. Fixture f11: `Work()` and `Inst()` were unresolved with reason `candidates-in-other-files`, not `shadowed-by-local`.
- **Lexical narrowing of nested definitions**: C# is not in the list (CLAUDE.md: JS/TS, Lua, OCaml, F#, Scala, Kotlin, Swift, Rust, Dart, Haskell, Julia). Local functions are owned by the type (section 02), so two same-named local functions in different methods of one class share a QN.
- **Local type scoping**: the local *type* map is per-function first (type_hints.rs:785). That is type scoping, not name shadowing.
- **Positional shadowing**: not applicable (OCaml/F#/Lua only).
- **Conditional definitions**: `#if` alternatives keep only the clean parse (section 01/08). There is no `DefRecord::conditional` (PHP only).

## 08. Macros, preprocessor & codegen

- **No macros** in C#. There is no `macro:<name>` tagging and no `name_mentions` from the C# plugin (`grep name_mentions csharp.rs`: none).
- **`#if`/`#elif`/`#else`/`#endif`**: handled only as a parse-repair step (section 01). On a clean parse, every branch is in the tree and both alternatives of a method are extracted. On a repaired parse, only the first branch survives when the all-branches copy fails. `#define` symbols are never evaluated. `#region`, `#pragma` and `#nullable` are ignored by the grammar.
- **Generics**: type arguments are stripped from callee names (section 03) and from bases (`push_bare_base`). Generic owners are bare names (`Gen<T>` becomes owner `Gen`; fixture f7 `Gen.Put`). There is no specialization narrowing, because `narrow_specializations` applies to `Name<args>` owners, which C# QNs never carry.
- **Source generators, `partial` codegen, `.Designer.cs` and `*.g.cs`** are parsed as ordinary C# when present on disk. Generated halves that exist only at build time are invisible. Partial methods carry `PARTIAL_DECLARATION`, which caps dead findings (section 12).
- **T4 templates (`.tt`)**: not read.

## 09. FFI & cross-language links

- **P/Invoke `[DllImport]`**:
  - It is classified `("pinvoke", Import)` (ffi.rs:667). Imports get **no** cross-language edge (ffi.rs:687-694, `detect_ffi_family` returns `""` for imports).
  - The `static extern` stub is a C# node, and C# calls bind to it.
  - Fixture f6: `Main -> Native.add` high intra-file, and **no** edge to the C `add` in `lib.c`.
  - `[LibraryImport]` (.NET 7 source-generated P/Invoke) is not in `classify_ffi` (ffi.rs:653-672).
- **`[UnmanagedCallersOnly]`** is classified `("c-abi", Export)` (ffi.rs:661).
  - C-family call sites of the same name link to it.
  - Fixture f6: the C prototype `managed_cb -> Native.managed_cb` medium `ffi-linker`.
  - It is also a `dotnet-ffi-export` framework entry (section 10).
- **`[DllExport]`** is classified c-abi Export (ffi.rs:663). `[JSExport]` and `[ComRegisterFunction]` are entries only (rule `dotnet-ffi-export`), not FFI edges.
- **Blazor JS interop** `[JSInvokable]` is an entry through the `blazor-jsinterop` rule. No JS→C# edge is created.
- **Protobuf → C# (`descriptor::link_descriptors`)**: an rpc links to a same-named C# override at `low`. Fixture f6: `Greeter.SayHello (proto) -> GreeterImpl.SayHello` low `descriptor-linker`. The protoc-generated `Greeter.GreeterBase` is not in the tree, so the C# method's base is unresolvable (rule `grpc-dotnet` gap).
- **Markup code-behind** (`plugins/markup.rs`): XAML/AXAML `Event="Handler"`, Razor `@onX="H"`/`OnX="H"` and Godot `.tscn` `[connection signal=… method=…]` become `FileFacts::markup_bindings`.
  - They are read only by framework detection, and never become an edge (markup.rs:1-12).
  - The XAML match requires the `(sender, …EventArgs/…Args)` signature (markup.rs:136-150).
  - Tests: `xaml_binds_only_event_handler_shaped_methods` (markup.rs:203), `razor_binds_dom_and_component_events` (markup.rs:221), `a_xaml_binding_enters_its_handler_keyed_by_the_event` (frameworks/mod.rs:2451).
  - Fixtures f2 (xaml `Click`), f7 (razor `@onclick`, `OnSaved`; tscn `timeout`).
- **Not covered**: COM interop (`[ComImport]`), `extern` methods without `[DllImport]`, `Marshal.GetDelegateForFunctionPointer`, function pointers `delegate* unmanaged`, JNI from Xamarin, and C++/CLI.

## 10. Frameworks & entry points

There are 41 rules with `language: "csharp"` (rules.rs). Kinds: N = `node: true` (an entry node is minted), R = `node: false` (root mark only). "Gap" means the rule has an enumeration gap text (`gap_for`).

| Rule id (rules.rs line) | Kind | Node | Detect (import prefix) | Trigger |
| --- | --- | --- | --- | --- |
| `aspnet-mvc` (1153) | network | N | `Microsoft.AspNetCore.Mvc`, `Microsoft.AspNetCore` | attributes `HttpGet/HttpPost/HttpPut/HttpDelete/HttpPatch/HttpHead/HttpOptions/Route`; bases `Controller`, `ControllerBase`, `PageModel` |
| `aspnet-minimal` (1179) | network | N | `Microsoft.AspNetCore.Builder`, `Microsoft.AspNetCore` | registrars `MapGet/MapPost/MapPut/MapDelete/MapPatch/MapMethods/MapGroup/MapHub` (handler in argument position) |
| `dotnet-hosting` (1205) | lifecycle | R | `Microsoft.Extensions.Hosting` | bases `BackgroundService`, `IHostedService`; methods `ExecuteAsync`, `StartAsync`, `StopAsync` |
| `quartz` (1222) | schedule | N | `Quartz` | base `IJob`, method `Execute` |
| `masstransit` (1239) | queue | N | `MassTransit` | base `IConsumer`, method `Consume` |
| `blazor` (1256) | network | N | `Microsoft.AspNetCore.Components` | no matchers. Gap: the `@page` route is in markup |
| `azure-functions` (2496) | network | N | `Microsoft.Azure.Functions.Worker`, `Microsoft.Azure.WebJobs` | attributes `Function`, `FunctionName`. Gap: trigger kind is on the first parameter and not read; every entry is `network` |
| `wcf` (2544) | network | N | `System.ServiceModel`, `CoreWCF` | no matchers. Gap: `[OperationContract]` through `[ServiceContract]` interfaces |
| `grpc-dotnet` (2561) | network | N | `Grpc.Core`, `Grpc.AspNetCore`, `Grpc.Net.Client` | no matchers. Gap: generated `*Base` classes |
| `xunit` (2578) | test | N | `Xunit` | attributes `Fact`, `Theory` (the gap text says the runner-built class, fixtures and `IAsyncLifetime` are not covered) |
| `nunit` (2595) | test | N | `NUnit.Framework`, `NUnitLite` | no matchers. Gap: `[ValueSource]`, `[SetUpFixture]` |
| `nunit-runner` (2614) | test | N | `NUnit.Framework` | attributes `Test/TestCase/TestCaseSource/Theory/SetUp/TearDown/OneTimeSetUp/OneTimeTearDown`; member sources `TestCaseSource`, `ValueSource` (type member `GetEnumerator`) (rules.rs:11738-11749) |
| `xunit-data` (2643) | test | N | `Xunit` | member sources `MemberData`, `ClassData` (type member `GetEnumerator`) (rules.rs:11750-11761) |
| `dotnet-serialization` (2661) | lifecycle | R | `System.Runtime.Serialization`, `Newtonsoft.Json` | attributes `OnDeserialized/OnDeserializing/OnSerialized/OnSerializing/OnError` |
| `unity` (3931) | lifecycle | R | `UnityEngine`, `UnityEditor` | bases `MonoBehaviour/ScriptableObject/NetworkBehaviour/StateMachineBehaviour/EditorWindow/Editor/PropertyDrawer`; about 90 message methods (`Awake`, `Start`, `Update`, `OnTriggerEnter`, `OnGUI`, …) |
| `aws-lambda` (4233) | network | N | `Amazon.Lambda` | attributes `LambdaFunction`, `LambdaStartup`; method `FunctionHandler`. Gap: handlers named only in `aws-lambda-tools-defaults.json`/SAM |
| `gcp-functions` (4952) | network | N | `Google.Cloud.Functions` | bases `IHttpFunction/ICloudEventFunction/ITypedFunction`, method `HandleAsync` |
| `mstest` (7564) | test | N | `Microsoft.VisualStudio.TestTools.UnitTesting` (also the literal `global using …` form) | attributes `TestMethod/DataTestMethod/TestInitialize/TestCleanup/ClassInitialize/ClassCleanup/AssemblyInitialize/AssemblyCleanup/DataRow/DynamicData/TestProperty` |
| `csharp-disposable` (7596) | lifecycle | R | `System` | bases `IDisposable`, `IAsyncDisposable`; methods `Dispose`, `DisposeAsync` |
| `csharp-runtime` (7613) | lifecycle | R | `System` | no matchers. Gap: event `+=`, finalizers, static constructors, serialization constructors, `Task.Run` lambdas, `Activator`/`GetMethod`/DI |
| `unity-editor` (9328) | ui | N | `UnityEditor`, `UnityEngine` | attributes `MenuItem/ContextMenu/ContextMenuItem/DrawGizmo/InitializeOnLoadMethod/RuntimeInitializeOnLoadMethod/DidReloadScripts/PostProcessBuild/PostProcessScene/OnOpenAsset` |
| `godot-rpc` (9358) | network | N | `Godot` | attribute `Rpc` |
| `dotnet-ffi-export` (9379) | ffi | N | none (always on) | attributes `JSExport/UnmanagedCallersOnly/DllExport/ComRegisterFunction/ComUnregisterFunction` |
| `benchmarkdotnet` (9403) | test | N | `BenchmarkDotNet` | attributes `Benchmark/GlobalSetup/GlobalCleanup/IterationSetup/IterationCleanup/ArgumentsSource/ParamsSource` |
| `xaml` (9725) | ui | N | none | markup kind `xaml` (rules.rs:11815) |
| `razor` (9744) | ui | N | none | markup kind `razor` (rules.rs:11816) |
| `signalr` (10340) | network | N | `Microsoft.AspNetCore.SignalR` | bases `Hub`, `DynamicHub`, `HubWithPresence` |
| `orleans` (10357) | network | N | `Orleans` | bases `Grain`, `IGrainWith*Key`, `IGrainObserver`, `GrainService`, call filters, `IRemindable`, … (15 bases) |
| `hangfire` (10390) | queue | R | `Hangfire` | no matchers. Gap: jobs captured in `Expression<Action>` |
| `mediatr` (10407) | lifecycle | R | `MediatR` | bases `IRequestHandler/INotificationHandler/IPipelineBehavior/…` (14); methods `Handle/HandleAsync/Process/Execute` |
| `ef-core` (10439) | lifecycle | R | `Microsoft.EntityFrameworkCore` | bases `DbContext/IEntityTypeConfiguration/Migration/…interceptors…` (20); methods `OnModelCreating/OnConfiguring/Configure/SaveChanges/…` |
| `maui` (10515) | lifecycle | R | `Microsoft.Maui` | no matchers. Gap text says `.xaml` is not parsed, which is stale now that the `xaml` rule reads sibling markup |
| `wpf-winforms` (10532) | lifecycle | R | `System.Windows.Forms`, `System.Windows.Controls`, … | no matchers. Gap: `.Designer.cs` `+=` wiring and converters; the same stale `.xaml` statement |
| `godot` (10864) | lifecycle | R | `Godot` | bases `Node`, `Node2D`, `Control`, … (about 50); Godot virtual methods (`_Ready`, `_Process`, …) |
| `godot-signal` (10964) | lifecycle | N | `Godot` | markup kind `tscn` (rules.rs:11817). Gap: only the same-stem scene is read |
| `monogame` (10981) | lifecycle | R | `Microsoft.Xna.Framework` | bases `Game/GameComponent/DrawableGameComponent/GameWindow`; methods `Initialize/LoadContent/Update/Draw/…` |
| `avalonia` (11016) | lifecycle | R | `Avalonia` | bases `Application/Window/UserControl/Control/…/IValueConverter`; about 30 lifecycle overrides |
| `avalonia-xaml` (11083) | lifecycle | N | `Avalonia.Markup`, `Avalonia.Controls` | no matchers. Gap text says `.axaml` is not parsed, which is stale for `Click=` (the `xaml` rule reads `.axaml`), but `Command=`/`{Binding}` remain invisible |
| `mvvm-toolkit` (11100) | lifecycle | R | `CommunityToolkit.Mvvm` | attribute `RelayCommand` |
| `blazor-lifecycle` (11117) | lifecycle | R | `Microsoft.AspNetCore.Components` | bases `ComponentBase/OwningComponentBase/LayoutComponentBase/InputBase/ErrorBoundaryBase`; methods `OnInitialized(Async)/OnParametersSet(Async)/OnAfterRender(Async)/SetParametersAsync/ShouldRender/BuildRenderTree/…` |
| `blazor-jsinterop` (11153) | ffi | N | `Microsoft.JSInterop` | attribute `JSInvokable` |

Fixture results:

- f2: `aspnet-mvc` (3 entries), `aspnet-minimal` (1), `quartz` (1), `xaml` (1), `xunit` (2), `nunit-runner` (5), `xunit-data` (1), and the roots `mediatr` `H.Handle`, `dotnet-hosting` `W.ExecuteAsync`, `unity` `Player.Update`, `csharp-disposable` `Proc.Dispose`.
- f6: `dotnet-ffi-export`.
- f7: `razor` (2), `godot-signal` (1), `godot` `_Ready`.

Helper tables and their C# entries:

- **`member_sources_for`**: `nunit-runner` (`TestCaseSource`, `ValueSource`) and `xunit-data` (`MemberData`, `ClassData`). These read string, `nameof(…)` and `typeof(T)` arguments.
  - Test: `xunit_and_nunit_sources_follow_nameof_and_typeof` (frameworks/mod.rs:3270).
  - Fixture f2: `[MemberData(nameof(Data))]` rooted `XTests.Data`, and `[TestCaseSource(nameof(Cases))]` rooted `NTests.Cases`.
  - `[ValueSource]` sits on a parameter, and parameter attributes are not recorded (nunit gap; CHANGELOG.md:809).
- **`signature_methods_for`**: no C# entries. Only `java-serialization` has any (rules.rs:11835-11866). C# deserialization constructors `(SerializationInfo, StreamingContext)` are not rooted (`csharp-runtime` gap).
- **`container_attributes_for`**: no C# entries (Rust `impl` blocks only, rules.rs:11770-11795).
- **`markup_kinds_for`**: `xaml`, `razor`, `godot-signal` (rules.rs:11812-11819).
- **`visibility_entries_for`**: Solidity only. C# has none.
- **`self_module_markers_for`**: JS/TS only.
- **Registrar verbs**: only `aspnet-minimal`'s `Map*` verbs. User verbs through `--registrar` reach C# through the generic `registrar::capture`.

Issues found while cataloguing:

- **`aspnet-mvc` mints an entry for every method of a `Controller`/`ControllerBase` subclass, including `private` ones.** Fixture f2: `<framework-entry>::network::aspnet-mvc::Web.UsersController.Unused -> UsersController.Unused` for a `private void Unused()`. ASP.NET only routes public non-`[NonAction]` methods.
- Lambdas passed to `MapPost` mint no entry (section 03).
- The `maui`, `wpf-winforms` and `avalonia-xaml` gap texts still say `.xaml` is not parsed (rules.rs:10515, 10532, 11083), while `markup.rs` now reads sibling `.xaml`/`.axaml` for `Click=`-style events.

## 11. Test harness entries

- **Attribute harness rules** (kind `test`, one node per harness: `<framework-entry>::test::<id>`):
  - `xunit` (`Fact`, `Theory`)
  - `nunit-runner` (`Test`, `TestCase`, `TestCaseSource`, `Theory`, `SetUp`, `TearDown`, `OneTimeSetUp`, `OneTimeTearDown`)
  - `xunit-data` (member sources)
  - `mstest` (`TestMethod`, `DataTestMethod`, init/cleanup hooks, `DataRow`, `DynamicData`, `TestProperty`)
  - `benchmarkdotnet` (`Benchmark`, setup/cleanup, `ArgumentsSource`, `ParamsSource`)
  - Fixture f2: one `<framework-entry>::test::xunit` node with entry edges to `XTests.A` and `XTests.B`, and one `nunit-runner` node to `Init`, `T1`, `T2`, `Cases` and `XTests.B` (`Theory` is in both lists).
- **Built-in dead-code test roots**:
  - `TEST_ATTRS` contains `Test`, `Fact`, `Theory` and `TestMethod` (deadcode/roots.rs:211-225; rule `builtin:test-attr`).
  - `is_test_name` falls through to `name.starts_with("test_") || name.starts_with("Test")` for C# (roots.rs:975-988; rule `builtin:test-name`). **Any C# method whose name begins `Test` (for example a production `TestConnection()`) is a test root regardless of file or attribute.** This is from code reading; there is no fixture for it.
- **Test-file gating**:
  - `*Test.cs`/`*Tests.cs` and the generic `test`/`tests`/`Test`/`Tests` directories count (testfile.rs:104, 35-46). Fixture f2: findings in `Tests.cs` were withheld from the report.
  - A `.Tests` project-name directory does not count (fixture f7).
  - The capability table reports `"test_tagging": "none"` for C# (fixture f7 dead-code JSON `capabilities`).
- **Name-only test rules**: no C# harness rule is name-only. All are attribute-based, so the "test file or importing the harness" gate (frameworks/mod.rs:2392) does not apply.
- **`TEST_CLASS_MEMBERS_ARE_ROOTS`** = `["python","php","ruby"]` (rules.rs:11806). C# is excluded, so test-class helpers are reportable. Fixture f2: `XTests.Orphan` sat in a test file and was not reported in production findings.
- **`--no-test-entries`** turns harness nodes into root marks (CLAUDE.md, generic).
- **Not covered**:
  - xUnit test-class constructors and `IClassFixture<T>` fixture types (`xunit` gap)
  - `IAsyncLifetime.InitializeAsync`/`DisposeAsync` (only `DisposeAsync` is rooted, through `csharp-disposable`/`blazor-lifecycle` names)
  - NUnit `[SetUpFixture]`, `[ValueSource]`
  - SpecFlow/Reqnroll `[Given]/[When]/[Then]` (no rule; `grep -c Given rules.rs` for C# finds none, unverified for other spellings)
  - FsCheck/CsCheck property tests

## 12. Dead-code analysis

- **Signal coverage**: the capability row for C# reads `visibility: full, attributes: full, value_references: full, dispatch: full, exports: none, test_tagging: none, max_confidence: high` (fixture f7 dead-code JSON).
- **Root kinds seen for C#**:
  - `builtin:lifecycle`: `LIFECYCLE` C# names `Main`, `Dispose`, `ToString`, `Equals`, `GetHashCode` (roots.rs:144-147), plus `CONVENTIONAL_METHODS` (`Dispose`, `toString`, …). These are rooted by name in any class. Fixture f2: `Proc.ToString` rooted `builtin:lifecycle`; fixture f1: `Program.Main`.
  - `builtin:test-attr`, `builtin:test-name` (section 11).
  - `toplevel:invocation`: calls from field initializers, expression-bodied properties and top-level statements. Fixture f1: `Svc.Calc` from the `Expr => Calc()` property; fixture f7: `TopHelper`, `Gen.Put`, `ScriptFn`.
  - `framework:rule`, `framework:entry-node`, `framework:test-harness` (section 10).
  - `ffi:export` from `FFI_EXPORT_ATTRS` (roots.rs:173-183) contains no C# attribute. C# FFI exports are rooted through the `dotnet-ffi-export` framework rule instead.
- **Visibility evidence**:
  - `Private`/`Internal` give `private-visibility` (corroborates `high`).
  - `Public`/`Protected` give `public-visibility` (caps at medium).
  - Read from `vis` (deadcode/mod.rs:945-970).
  - Fixture f1: `Program.Dead` (private) high, `Pt.Move` (public) medium.
- **Reachability of unresolved doubt**: a C# `private` member counts an unresolved same-name site only from the same file, or from a caller whose owner stem equals the member's owner (partial classes across files) (deadcode/mod.rs:668-676).
- **`implements-external-contract` cap (medium)**:
  - `override` modifier members, and explicit interface implementations (tagged `override`), whose ancestors declare no in-tree member of that name.
  - `partial` method parts: the contract is reported as `partial` (deadcode/mod.rs:485-495).
  - Tests: `csharp_explicit_interface_impl_of_an_outside_interface_is_not_high` (deadcode/mod.rs:2289), `a_partial_method_part_is_a_contract_member_not_high` (2247).
  - Fixture f2: `Proc.Tock` medium `implements-external-contract`, and both `Part.OnLoaded` parts medium `partial`.
  - **Mislabel**: `Proc : IExternal, IDisposable` reported the contract as `IDisposable`, the first outside ancestor (`find`, deadcode/mod.rs:520-523), not the `IExternal` the method implements.
  - **Gap**: an *implicit* interface implementation (`public void Tick()` in `class Proc : IExternal`) carries no marker, so it gets no contract evidence. It is medium only because it is public (fixture f2 `Proc.Tick`).
- **`implemented-trait-surface`**: an interface or abstract member whose implementation is called (fixture f1: `IRunner.Run`, `Base.Abs` medium).
- **`implicitly-invokable-kind`**: constructors and property accessors are never `high`. Fixture f1: every `Svc.Svc` constructor and every `get`/`set` came out medium. Destructors are excluded from findings entirely (deadcode/mod.rs:599-601; fixture f1: `~Svc` not reported).
- **Reflection cap**: `name-collides-with-screened-site` from `dynuse` (section 15). Fixture f2: `Startup.ByReflection` medium after `GetMethod("ByReflection")`.
- **`incoming-only-from-dead-code`**: medium (fixture f2: `Startup.Lambda`, `Ev.OnChanged`, `Cond.G`).
- **`macro-generated`**: not applicable.
- **False positives observed**:
  - Event handlers wired by `+=` are live only through a reference edge from the wiring method. Fixture f2: `Ev.Wire` itself was reported high, because nothing calls it.
  - Private methods only called through an untyped field receiver (`_repo.Save()`) keep a `name-matches-unresolved-site` doubt, which caps them.
  - Constructors are never reached by `new` (section 03), so every constructor relies on the `implicitly-invokable-kind` cap.
- **No `exports` signal**: `exports: none`. There is no assembly-level public API model (`InternalsVisibleTo`, `public` in a library project). Public members are capped at medium instead.

## 13. Builtins & stdlib tables

- **`crates/cgg-core/src/stdlib/csharp.txt`**: 380 lines (`wc -l`), loaded at stdlib.rs:193/234. It holds BCL type and member names (`Abs`, `Action`, `Activator`, `Add`, `Aggregate`, `Any`, `ArgumentException`, …).
  - It is used to screen unresolved calls into the `stdlib` bucket (fixture f1: `Path.Combine`, `System.IO.File.ReadAllText`; fixture f3: `Abs`, `First`, `ForEach`, `nameof`).
  - It also supplies the stdlib-root check in `cs_namespace_head` (heads.rs:824-825).
- **C# rows in other tables**:
  - `MAYBE_TYPE_WRAPPERS`: `Nullable` (stdlib.rs:118)
  - `CALLBACK_TYPE_CONSTRUCTORS`: `Action`, `Func` (stdlib.rs:144-145)
  - `TOP_TYPES`: `Object`, `Dynamic` (stdlib.rs:153)
  - No C# row in `CAST_INTRINSICS`, `CONSTRUCTING_METHODS`, `UNION_TYPE_CONSTRUCTORS` or `NULL_TYPE_MEMBERS`.
- **No `rust_types`-like wrapper table** for `Task<T>`, `Lazy<T>`, `IEnumerable<T>` and the like. So `Task<Foo>` reads as `Task` (section 05).
- **Implicit packages**: no C# equivalent of `SCALA_IMPLICIT_PACKAGES`. Implicit global usings from the SDK (`ImplicitUsings` in `.csproj`: `System`, `System.Linq`, …) are not modelled. The stdlib-root check covers `System.*` heads only when the root word is in csharp.txt (unverified whether `System` itself is listed; `cs_namespace_head` passed for `System.IO.File` in fixture f1).
- **`--include-stdlib` / `--include-external`**: these mint `<stdlib>::Path::Combine` and `<external>::JsonConvert::SerializeObject` leaf nodes (fixture f1 mermaid, N38-N44).
  - **Misattribution**: unresolved workspace calls in the external bucket also become `<external>` nodes. Fixture f1: `<external>::each::Run`, `<external>::pat::Run` and `<external>::_repo::Save`, which are workspace methods cgg could not type.

## 14. Confidence & certainty

- **`high` is reached for C# through**:
  - Intra-file exact or inherited members (fixture f1: `Svc.Run -> Base.Helper`).
  - Typed receivers naming one class: locals, params, `new`, casts and nullable params (fixture f1: `s.Run`, `((Svc)null).Hook`; fixture f8: `nf.M`, `nn.M`).
  - `using static` and aliases (fixture f1 L23-24).
  - Unique nested type paths (fixture f3: `Alpha.Util.Nested.Deep`).
  - `base.M()` with an in-tree base (fixture f1).
  - Settled lambda parameters (fixture f1 L39).
  - Interface-typed parameters (fixture f3: `IThing.Act`).
- **Forced to `medium`**:
  - Overload sets with more than one survivor (no arity narrowing).
  - By-name matches (fixture f1: `r.Save()`).
  - Extension methods (fixture f1: `Shout`).
  - Any owner whose bare name occurs in two directories (`unique_owner_names` keys C# by directory, cross_file.rs:4415). Fixture f3/f4: `Util.Go` medium although namespace narrowing left one candidate.
  - `overrides_below` sets and duck fan-out (fixture f3).
- **`low`**: `dispatch:inheritance` edges, framework entry edges, descriptor links (fixture f6).
- **FFI**: `ffi-linker` edges into `[UnmanagedCallersOnly]` are medium (fixture f6).
- **Mermaid `?` marker**: an arrow with no `high` site gets `?` (fixture f4 mermaid: `N5 -->|?| N0`, `N5 -->|?2x| N1`).
- **False `high` risk**: local/parameter delegates shadowing a same-class method (section 07, fixture f11b).

## 15. Dynamic use & reflection

- **`dynuse` coverage**: C# reflective callees are `GetMethod`, `GetType`, `CreateInstance` and `GetProperty` (dynuse.rs:45-48).
  - An identifier-shaped string literal argument (dots allowed) becomes a `DynUse`, which caps every same-named C# finding at medium (`reflection` screen). It never creates an edge.
  - Test: `csharp_get_method_names_a_method` (dynuse.rs:195).
  - Fixture f2: `typeof(Startup).GetMethod("ByReflection")` gave `Startup.ByReflection` medium `name-collides-with-screened-site`.
  - `Activator.CreateInstance(typeof(Startup))` passes a `typeof`, not a string, so it captures nothing.
- **Not covered**:
  - `InvokeMember`, `GetField`, `GetEvent`, `GetMember`, `Type.GetType(...)` followed by `.GetMethods()` enumeration
  - `dynamic` dispatch (fixture f7: `d.Zap()` was `fanout-cap-exceeded`, which counts as doubt for every `Zap`)
  - `Delegate.CreateDelegate`, `Expression.Call(..., "Name")`
  - DI container registration by `typeof`/generic (`services.AddTransient<IFoo, Foo>()`)
  - Source-generator and `[DynamicDependency]` attributes
- **Module loads**: there is no C# entry in `DYN_MODULE_LOADERS` (facts.rs:165: `require`, `importScripts`, `import_module`). `Assembly.Load("x")` is not captured.
- **`nameof(X)`** is not reflection. It is recorded as a value reference, and its target gets a `reference` edge (fixture f3).

## 16. Known gaps & limitations

Each item below has evidence above.

1. **No constructor call edges**: `new T()`, `: this()`, `: base()` and target-typed `new()` produce no edge to `T.T` (fixture f9, f1).
2. **No return-type inference** for C# signatures, which put the type before the name (type_hints.rs:2632-2671). `var x = Make(); x.M()` and chains fall to by-name or fan-out (fixtures f1, f3, f8).
3. **No field or property typing**: `_repo.M()` and `Prop.M()` stay unresolved; `this._f.M()` fans out (fixtures f1, f3, f8).
4. **No `FileFacts::locals`**: parameter and local delegates shadowing a same-class method give false `high` edges (fixture f11b).
5. **No arity narrowing of overloads**: C# is not in `OVERLOAD_ARITY_LANGS` (type_hints.rs:1583) and has no `RefRecord::arity`. Every overload binds at medium (fixture f8).
6. **Pattern, `out`, `foreach`, `catch` and deconstruction variables are untyped** (fixtures f1, f3).
7. **`var x = (T)y` / `y as T` locals are untyped** (fixture f8). Only `var x = new T()` is read.
8. **`await` receivers** read as `Task`, not `T` (fixture f8; CHANGELOG.md:1474).
9. **Operators, conversion operators, delegates, events and record synthesized members are not callables** (fixture f1). Accessors are named `get`/`set`/`init`, not after their property, so all accessors of a type share one QN.
10. **Expression-bodied properties and field initializers are not callables**. Their calls are `no-enclosing-callable` (fixtures f1, f9). Top-level statements likewise (fixture f7).
11. **Local functions are owned by the type, not the enclosing method** (fixture f1).
12. **File-scoped namespaces do not prefix qualified names** (fixture f1: `Program.Main`), so they are inconsistent with block-namespace files.
13. **`global using static` is ignored** (csharp.rs:428; fixture f10).
14. **`aspnet-mvc` enters private controller methods** (fixture f2).
15. **Implicit interface implementations get no `implements-external-contract` evidence**, and the contract name reported for explicit ones can be the wrong base (fixture f2).
16. **C# methods named `Test*` are test roots by name** (roots.rs:987).
17. **`.Tests` project directories are not test-tagged** (fixture f7).
18. **Method groups through lowercase variables or fields are not value refs** (csharp.rs:494-500; CHANGELOG.md:752).
19. **No static/instance distinction**: no `StaticMethod` variant, so the "static members never override" rule in dispatch.rs:390 never fires for C#.
20. **`[LibraryImport]` is not FFI, and P/Invoke never links to an in-tree C body** (ffi.rs:653-672; fixture f6).
21. **`dynuse` misses `InvokeMember`/`GetField`/`Assembly.Load`** (dynuse.rs:45-48).
22. **Stale gap texts**: `maui`, `wpf-winforms` and `avalonia-xaml` say `.xaml` is not parsed (rules.rs:10515, 10532, 11083).
23. **No .NET project model**: no `.csproj` `RootNamespace`, `ImplicitUsings` or project references.
24. Rule gaps listed by `gap_for`: `blazor` (routes), `azure-functions` (trigger kinds), `wcf`, `grpc-dotnet`, `xunit` (fixtures/constructors), `nunit` (`ValueSource`, `SetUpFixture`), `aws-lambda` (JSON-named handlers), `gcp-functions`, `csharp-runtime`, `hangfire`, `maui`, `wpf-winforms`, `godot-signal` (other scenes), `avalonia-xaml`.
25. The plugin header comment (csharp.rs:19-23) says that generics, extension methods and explicit interface implementations are "best-effort". It also says partial classes merge "only by exact name".
26. **No `#[ignore]`d C# tests** (`grep -rn 'ignore]'` finds none touching C#).

## Summary scorecard

| section | support | one-line note |
| --- | --- | --- |
| 01. Detection & parsing | full | `.cs`/`.csx`, `#if` re-parse repair, sibling markup read; no shebang, `.razor` bodies unparsed |
| 02. Callables extracted | partial | methods, ctors, dtors, accessors, local functions; no operators/delegates/events/record members; accessors named `get`/`set` |
| 03. Call-site forms | partial | bare/member/generic/`?.`/`base`/casts/method groups; no `new T()` constructor edges, no arity |
| 04. Imports & module resolution | partial | using/alias/static/global using + namespace visibility + external refusal; `global using static` ignored, no `.csproj` |
| 05. Type inference & receiver typing | partial | locals, `var = new`, params, nullable, typed/settled lambdas; no return types, fields, patterns, await |
| 06. Inheritance, dispatch & overloading | partial | base records, `base.`, member_of_calling_class, overrides_below, extensions, inheritance fan-out; no arity narrowing |
| 07. Scoping & shadowing | minimal | no locals table, so parameter delegates shadowing members bind falsely `high` |
| 08. Macros, preprocessor & codegen | partial | `#if` repair only; no symbol evaluation; generated code only if on disk |
| 09. FFI & cross-language links | partial | `UnmanagedCallersOnly` export links, protobuf rpc links, markup bindings; DllImport/LibraryImport never link to C |
| 10. Frameworks & entry points | full | 41 rules incl. ASP.NET, Unity, Godot, Blazor, EF Core, MediatR, XAML/Razor markup; private MVC methods over-entered |
| 11. Test harness entries | full | xUnit/NUnit/MSTest/BenchmarkDotNet + member sources; `Test*` name rule overbroad, `.Tests` dirs untagged |
| 12. Dead-code analysis | partial | vis, override/partial contract caps, reflection cap; implicit interface impls uncapped, ctors never reached |
| 13. Builtins & stdlib tables | partial | 380-name csharp.txt, Nullable/Action/Func/TOP_TYPES; no Task/Lazy wrapper table, no implicit usings |
| 14. Confidence & certainty | full | high via typed/exact/static-using paths; medium for overloads/extensions/by-name; `?` marker works |
| 15. Dynamic use & reflection | minimal | GetMethod/GetType/CreateInstance/GetProperty literals only; no module-load or DI coverage |
| 16. Known gaps & limitations | n/a | 26 items listed above, all with fixture or code evidence |
