# kotlin

The Kotlin plugin is `KotlinPlugin` in `crates/cgg-lang/src/plugins/kotlin.rs` (1,084 lines including inline tests). It parses `.kt` and `.kts` files with the crates.io grammar `tree-sitter-kotlin-sg` 0.4 (`Cargo.toml:94`, `crates/cgg-lang/Cargo.toml:32`; `tree_sitter_kotlin_sg::LANGUAGE`, kotlin.rs:26-28). Nothing is vendored. It is registered at `crates/cgg-lang/src/plugins.rs:131`.

A single recursive `KtWalker` (kotlin.rs:51-579) keeps four stacks:

- `scope`: package segments and class/object names, which form the qualified name.
- `bases`: the supertypes of the enclosing class or object.
- `implicit`: the implicit receivers of `run`/`apply`/`with` lambdas.
- the source buffer.

The walker records:

- the package (into `FileFacts::packages`) and imports;
- classes and objects (`FileFacts::classes`, plus a `Constructor` def for each class);
- `fun` declarations;
- `property_declaration` local/field types;
- every `call_expression`, with a receiver rebuilt from the tree.

The plugin declares only the `attributes` and `visibility` signals (kotlin.rs:18-24). It does **not** declare `test_defs`, `value_refs`, `dyn_uses`, `exports`, `unreachable` or `impls`. It ignores its `ExtractCtx` (`_ctx`, kotlin.rs:32), so it never records registrar arguments and never records value references.

No `resolver_kind` override exists, so resolution runs the generic pipeline. Kotlin-specific branches exist in:

- `cgg-resolve/src/cross_file.rs`:
  - step 0b, the Kotlin extension index;
  - `member_of_caller`;
  - JVM import handling;
  - `narrow_owner_paths`;
  - `super_owner_may_be_anonymous`.
- `heads.rs`: JVM package heads and external-module refusal.
- `type_hints.rs`:
  - scoped locals;
  - enclosing-function parameters for lambdas;
  - `settle_implicit_receivers`.
- `intra_file.rs`: lexical scoping and overload sets.
- `dispatch.rs`: `INHERITANCE_LANGS`.
- `locals.rs`: `KOTLIN_SPEC`.
- `receiver.rs`: constructs-by-call and `as` casts.

Fixtures cited below live under `<scratch>/parity/kotlin/` (f1 … f10). Each was run with `target/release/cgg` 0.11.0 using `-t json -o out.json`, and dumped with `show.py` in that directory.

## 01. Detection & parsing

- **Extensions `.kt` and `.kts`** (kotlin.rs:15-17). Fixture f8 audit: `./a.kt extension:.kt` and `./build.gradle.kts extension:.kts`. Gradle Kotlin DSL scripts are therefore analyzed as Kotlin.
- **Shebangs**: none. `shebangs()` is not overridden, so the default `&[]` applies (cgg-lang/src/lib.rs:298-300). A `#!/usr/bin/env kotlin` script without a `.kts` extension is not detected.
- **Content detection**: none for Kotlin. `crates/cgg/tests/detect_prefixes.rs:39` uses `import {prefix}.Thing` in a `.kt` file only to exercise the extension path.
- **Grammar**: a single grammar for both extensions. There is no `ts_language_for` override, so `.kts` uses the same grammar. Script top-level statements parse, but they have no enclosing callable (§02).
- **Re-parse or fallback**: none. A file with syntax errors is extracted anyway and gets `parse_status = "error"` (generic, cgg/src/lib.rs:457-461).
  - Grammar quirk: a **one-line nested class** `class Nested { fun n() {} }` inside another class body makes the file `parse_status: error`, although extraction still yields every node. Evidence: bisect fixture `bis/`, where deleting only line 13 (`class Nested { fun n() {} }`) turns `error` into `ok`.
  - A **top-level one-line `object X { fun f() = 1 }`** is misparsed by the grammar as an infix expression (`object` `X` `{…}`). The plugin repairs it with `misparsed_object` (kotlin.rs:89-103, 682-698). Test `a_one_line_top_level_object_keeps_its_scope` (kotlin.rs:722).
- **Newline-sensitive call merging (grammar)**: `nested()` followed by a line starting with `(q as Shape).area()` was read as one chained call. In fixture f1, the `(q as Shape).area()` site on line 60 was attributed to line 59 and fanned out at medium.
- **Depth skip**: shared by every language. A tree deeper than `MAX_TREE_DEPTH` = 4000 is skipped as `SkipReason::TooDeep` (cgg/src/lib.rs:451-456).
- **Minified handling**: Kotlin does not override `max_line_bytes` (no override in kotlin.rs). The generic rule applies: locals are not collected when any line exceeds 2,000 bytes (cgg/src/lib.rs:159, 480-485).
- **Test-file classification** (path based, cgg-core/src/testfile.rs):
  - any `src/test/` Maven/Gradle layout (testfile.rs:61-67);
  - names `*Test.kt` / `*Tests.kt` (testfile.rs:103).
  - `*IT.kt`, `*Spec.kt` (Kotest/Spek) and `src/androidTest/` are **not** listed (testfile.rs:103).

## 02. Callables extracted

Qualified names are `<package>.<Class…>.<name>`, joined with `.` (kotlin.rs:74-80). The package is pushed onto `scope` (kotlin.rs:146-163). **Only classes and named objects push scope.** Functions, companion objects, anonymous objects and lambdas do not.

Supported shapes:

- **Top-level function** `fun topLevel() {}`:
  - In a file **without** a package it is a `FreeFunction` (kotlin.rs:327-333). Test `top_level_function` (kotlin.rs:788).
  - In a file **with** a package it becomes an `InherentMethod` (JSON kind `method`), because the package is on `scope`. Fixture f1: `com.acme.util.topHelper  method`.
  - cross_file.rs:3536-3539 documents this and compensates by indexing top-level functions by QN (`top_level_fns`).
- **Class member** `class Service { fun run() {} }` → `InherentMethod` `com.example.Service.run`. Test `class_methods_with_package` (kotlin.rs:769).
- **Class as constructor**: every named `class_declaration` (`class`, `data class`, `enum class`, `value class`, `interface`, `fun interface`, `sealed interface`) emits a `Constructor` def. Its span is the whole class and it carries the class's annotations (kotlin.rs:168-207).
  - Fixture f1: `com.acme.Shape constructor`, `com.acme.Ev constructor`, `com.acme.Pt constructor`.
  - **Partial**: interfaces get a constructor node too, although they cannot be constructed (f1 `com.acme.Shape`, `com.acme.Ev`).
- **Secondary constructor and `init {}` blocks**: no node of their own. Their calls are attributed to the class's `Constructor` node, because its span covers them. Fixture f8: edges `u.Op -> u.Op.compute` from `init { compute() }` (L11) and from `constructor(x: Int) : this() { compute() }` (L12).
- **Nested and inner classes** `class Op { class Nested { fun n() {} } }` → `u.Op.Nested` (constructor) and `u.Op.Nested.n`. Fixture f8.
- **Named object** `object Registry { fun lookup() }` → `com.acme.Registry.lookup`. A `ClassDecl` is recorded, but no constructor node is emitted (kotlin.rs:209-231). Fixture f1.
- **Companion object** (named or unnamed): **transparent**. Its members are filed under the enclosing class (kotlin.rs:232-236). Fixture f1: `com.acme.Square.make`. Fixture f8: `companion object Factory { fun create() }` → `u.Op.create`. cross_file.rs:3647-3655 (`kotlin_import_path`) strips `.Companion.` from imports to match.
- **Extension function** `fun String.shout()` → `InherentMethod` named `shout`, owner = enclosing scope (the package). The receiver type is **not** in the QN (kotlin.rs:323-333). Fixture f1: `com.acme.util.shout`. Test `extension_function` (kotlin.rs:797).
  - Extensions are recognised later from the signature text (`is_kotlin_extension`, cross_file.rs:3586-3605; test `kotlin_extension_signatures_are_recognised`, cross_file.rs:8368).
- **Generic, `suspend`, `operator`, `infix`, `@JvmStatic` functions** are all ordinary `function_declaration`s. Fixture f1: `com.acme.generic`, `com.acme.later`, `com.acme.Square.plus`. Fixture f8: `u.shl2`, `u.Op.invoke`, `u.Op.get`, `u.Op.component1`, `u.Op.create` with `@JvmStatic`.
- **Overloads**: two same-QN defs are kept. Fixture f1: `com.acme.overloaded` ×2. Fixture f2: `p.Over.f` ×2.
- **Abstract and interface members** `interface Shape { fun area(): Double }`, `abstract fun draw()` → `InherentMethod`. They are **not** tagged `abstract`, so the duck-typing narrowing that drops abstract declarations does not apply. Fixture f2: `extra -> p.Svc.go`, `SvcA.go`, `SvcB.go`, all medium.
- **Local (nested) functions** `fun outer() { fun helper2() {} }` → a node **without the outer function in its QN** (`p.helper2`). It can collide with a same-named top-level function. Fixture f2: two `p.helper2` nodes, from a.kt line 19 and b.kt line 2. The intra-file call binds to the nested one (high).
- **Object-expression members** `object : Svc { override fun go() {} }` → a node QN'd at the **enclosing scope** (`p.go`), with no anonymous owner. Fixture f2.
- **Visibility** (`vis`) is read from the grammar `visibility_modifier`, defaulting to `Public` (kotlin_vis, kotlin.rs:658-677). Tests `an_annotation_argument_does_not_hide_the_visibility_modifier` (kotlin.rs:1013) and `visibility_defaults_to_public_and_ignores_parameter_names` (kotlin.rs:1031).
  - Constructor defs set no `vis` (kotlin.rs:193). JSON shows `vis=None` (f1).
- **Attributes**: annotations via the shared `attrs::collect`, stored verbatim (`@GetMapping("/x")`, `@Composable`, `@JvmStatic`) (kotlin.rs:194, 353; attrs.rs:32-41). Fixture f5.
  - Modifiers (`override`, `open`, `abstract`, `external`, `suspend`, `operator`) are **not** attributes: `member_modifier` and `function_modifier` are not in `ATTR_KINDS` (attrs.rs:32-41). Fixture f5: `w.Nat.nativeCall attrs=` (empty) for `external fun`.
- **Base types on members**: each method's `DefRecord::base_types` is the enclosing class's or object's supertypes (kotlin.rs:340-342). Test `a_superclass_written_as_a_constructor_call_is_a_base` (kotlin.rs:966).

Not supported or partial:

- **Property accessors** `val p: Int get() = …`: no node. Calls inside them go to the class's constructor node (f8 L9 `u.Op -> u.Op.compute`), or have **no source** at top level.
- **Extension properties** `val String.ext: Int get() = length`: no node (f8).
- **Top-level `val lam = { … }` lambdas, `val local = fun() {}` anonymous functions, and all lambdas**: no node.
  - Inside a function, their calls are attributed to the enclosing function (f1 L57 `main -> topHelper` from `val local = fun() { topHelper() }`).
  - At top level they are unresolved with `src: null` (`no-candidate-in-file`). Fixture f1, site L32.
- **Script (`.kts`) top-level statements**: no callable. Calls are `no-enclosing-callable` (f8: `helperK` at build.gradle.kts L3, L4). Script `fun`s get QN without a package (`helperK`, kind `function`).
- **`typealias`**: nothing recorded (f8).
- **Delegated members** `class D(val d: Svc) : Svc by d`: there is no node for the delegated `go`. Only `Svc` is recorded as a base (f2; kotlin.rs:611).
- **Enum entries with bodies and `data class` generated members** (`copy`, `componentN`, `equals`): no nodes. Unverified beyond fixture f1 (no generated members appeared).

## 03. Call-site forms

Every `call_expression` is recorded by `record_call` (kotlin.rs:469-526). The callee is either a `simple_identifier` (bare) or a `navigation_expression` (receiver call). Any other callee kind is dropped (kotlin.rs:502).

Supported shapes:

- **Bare call** `helper()` → name `helper`, empty receiver. Test `call_expressions` (kotlin.rs:813).
- **Method call** `obj.run()`, `Obj.run()`, `x?.run()` → receiver `obj` / `Obj` / `x`. Test `plain_receivers_and_bare_calls_are_unchanged` (kotlin.rs:954).
- **Chained or complex receivers** are rebuilt from the tree by `receiver_text` (kotlin.rs:534-578). Test `a_chained_receiver_is_kept_not_dropped` (kotlin.rs:847):
  - `other.stream.close()` → `other.stream`
  - `x.a().close()` → `x.a()`
  - `x!!.close()` → `x`
  - `x?.y?.close()` → `x.y`
  - `list.map{}.filter{}.close()` → `list.map().filter()`
  - `"s".close()` → `""`
  - `(y).close()` → `y`
- **Multi-line chains** are recorded without line breaks or comments. Test `a_receiver_chain_is_rebuilt_without_its_line_breaks` (kotlin.rs:888).
- **`this`, `super` and `super<B>`** → `this` / `super`. Test `this_and_super_receivers_are_kept` (kotlin.rs:895).
- **Labelled `this@Outer`** → `Outer` when `Outer` is an enclosing scope; otherwise the text (`this@run`). Test `a_this_labelled_with_an_enclosing_class_is_that_class` (kotlin.rs:904). Fixture f1: `this@Square.draw()` → `Square.draw` (high).
- **Constructor call** `Foo(1)`: a bare call to the class name. It binds to the class `Constructor` node (f1 `main -> com.acme.Square` high; `main -> com.acme.util.Helper` high via import).
- **Constructed or cast receiver** `Foo(x).m()`, `(x as Foo).m()`, `(x as? Foo)?.m()` → typed receiver `Foo` (`typed_receiver = true`) through `receiver::peel` (kotlin.rs:492-499; receiver.rs:46-51 `constructs_by_call` includes `kotlin`; receiver.rs:180-185 `as_expression`).
  - Tests `a_constructed_receiver_is_its_type` (kotlin.rs:916) and `kotlin_constructions_across_lines_and_casts` (receiver.rs:561).
  - Fixture f7: `(x as Svc).go()` and `(x as? Other)?.go()` both resolve high.
- **Package-qualified call** `kotlinx.coroutines.runBlocking {}` → receiver `kotlinx.coroutines`. Test `a_package_qualified_call_keeps_its_package` (kotlin.rs:941).
- **Explicit type arguments** `generic<Int>(1)`, `gen<Svc>()`: recorded under the bare name. Fixture f1: `main -> com.acme.generic` (high). Fixture f7: `r4 -> t.gen` (high).
- **Trailing-lambda calls** `list.forEach { … }`, `runBlocking { }` are ordinary call expressions. The lambda body is walked, and its calls belong to the enclosing function.
- **Scope-function implicit receivers**: inside `x.run { m() }`, `x.apply { m() }` and `with(x) { m() }`, a bare call carries receiver `x` with `implicit_receiver = true`.
  - The receiver set is `KOTLIN_RECEIVER_SCOPE_EXTENSIONS` = `run`, `apply` and `KOTLIN_RECEIVER_SCOPE_FUNCTIONS` = `with` (stdlib.rs:25, 29; kotlin.rs:110-142, 250-276).
  - Test `scope_function_lambdas_give_bare_calls_their_receiver` (kotlin.rs:1047). Fixture f1: `with(q) { draw() }` → `Square.draw` (high) and `q.apply { area() }` → `Square.area` (high).
- **Unrecoverable receiver**: `dot_call = true` keeps the call from being read as bare (kotlin.rs:508-510).

Not supported:

- **Arity**: `RefRecord::arity` is never set (kotlin.rs:516-525), so there is no overload narrowing (§06).
- **Method and function references** `::topHelper`, `a::byRef`, `list.forEach(::println)`: no value-ref record, so no edge, even under `--reference-edges`. Fixture f1: no edge for `::topHelper`. Fixture f3: `MyAct.byRef` is reported dead. The `kotlinx-coroutines` gap text says the same (rules.rs:7529).
- **Operator calls by syntax**: `a + b` (`plus`), `o()` (`invoke`), `o[1]` (`get`), destructuring `val (a) = o` (`component1`), `in` (`contains`), `by` delegates (`getValue`). None is a call site. Fixture f8: no edges to `Op.invoke`, `Op.get`, `Op.component1` or `Op.plus`. Fixture f1: `q + q` has no edge to `Square.plus`.
- **Infix function calls** `1 shl2 2` (an `infix_expression`, not a `call_expression`): not recorded. Fixture f8: no edge to `u.shl2`.
- **Property reads through getters** (`o.p`) and extension properties (`"s".ext`): not calls (f8).
- **Callbacks passed as values** (`register(::handler)`, a lambda passed to a framework): there is no registrar capture, because the plugin ignores `ExtractCtx` (kotlin.rs:32), and there are no Kotlin rules with `registrars` (§10).
- **Calling a local lambda variable** `val target = { 1 }; target()`: refused as `shadowed-by-local` (f6 L12). That is correct, but the call yields no edge to the lambda body (no node).

## 04. Imports & module resolution

Import records:

- `import a.b.C` → `ImportRecord{kind:"import", path:"a.b.C", alias:"C"}`.
- `import a.b.f as g` → alias `g` (record_import, kotlin.rs:358-395).
- `import a.b.*` → path `a.b.*` (the `identifier` text). Test `imports_captured` (kotlin.rs:804).

Package: `package com.acme.util` → `FileFacts::packages = ["com.acme.util"]`, also pushed as the QN prefix (kotlin.rs:146-163). Test `package_is_recorded` (kotlin.rs:831).

Resolution:

- **Import tables**: Kotlin uses the shared Scala/Java/Groovy branch (cross_file.rs:1982-2000). `a.b.*` / `a.b._` → `unqualified_prefixes`; `a.b.C` → `direct_imports[C]`.
- **Constructors and top-level functions through an import** `import com.example.Foo` + `Foo()`, `import com.example.helper` + `helper()` → `module_aliases` (cross_file.rs:5758-5768). Fixture f1: `main -> com.acme.util.Helper` (high) and `main -> com.acme.util.topHelper` (high).
- **Star imports** `import s.other.*` + `starFn()` → resolved at medium (fixture f6 L20).
- **Imported extensions** (`import com.acme.util.shout`, `"x".shout()`) → step 0b. They are **high** when the import names the extension exactly (cross_file.rs:2738-2753). Fixture f1 L40 is high. Test `kotlin_extension_calls_resolve_through_their_scope` (resolve.rs:2974).
- **Companion-imported extensions** `import okhttp3.HttpUrl.Companion.toHttpUrl` → `kotlin_import_path` strips `.Companion.` (cross_file.rs:3652-3654). Test `a_companion_imported_kotlin_extension_resolves` (resolve.rs:3107).
- **External-module refusal** (`heads.rs`):
  - Each Kotlin `import` binds its local name to its JVM package. It is external unless `vocab.jvm_workspace(pkg)` (heads.rs:509-545, Kotlin arm 538-541).
  - A dotted receiver whose head is a package root is read by `jvm_package_head` (heads.rs:734, 770-800). Workspace packages come only from declared `package` lines (heads.rs:33-34, 70-74, 142).
  - Fixture f6: `delay(1)` after `import kotlinx.coroutines.delay` → `<external>::delay`.
  - Fixture f1: `File("a").readText()` → `java.io.File` bucket; `OkHttpClient().newCall()` → `okhttp3.OkHttpClient` bucket.
  - Test `kotlin_receivers_are_never_read_as_bare_calls` (resolve.rs:2804).
- **Member beats import**: a bare call to a member of the calling class (or one inherited) is that member, even when the name is imported from outside (cross_file.rs:5348-5375). Test `a_bare_call_to_an_inherited_member_is_that_member` (resolve.rs:3079).
- **Typed owner anchoring**: a by-name import of a type that names another path is a different type (`anchored`, cross_file.rs:4623-4626).
- **Owner narrowing**: a qualified JVM receiver type `Response.Builder` narrows same-named owners (`narrow_owner_paths`, cross_file.rs:4847). Test `a_qualified_jvm_receiver_type_narrows_same_named_owners` (cross_file.rs:14374).
- **Kotlin packages feed Scala's package vocabulary** (libscope.rs:755).

Not supported or partial:

- **`not-in-imported-module`**: not applicable. `modules.rs` (`ModuleIndex`) has no Kotlin branch (grep finds no `kotlin` there).
- **Import alias of an object member** `import u.Q.qf as aliased; aliased()`: **no edge**. Fixture f8: `aliased` is in the `(unattributed)` bucket.
- **Import alias of an object** `import com.acme.util.Fmt as F; F.fmt()`: unresolved, `candidates-in-other-files` (fixture f1 L41).
- **No manifest reading**: `build.gradle(.kts)`, `settings.gradle` and Android `AndroidManifest.xml` are not read. There is no `manifest.rs` use in kotlin.rs.
- **No JVM cross-language linking**: `language_family` leaves the JVM family commented out (cgg-core/src/lang_family.rs:32). Kotlin → Java and Java → Kotlin calls in one tree produce **no edges**. Fixture f9:
  - `J.sj()`, `x.m()` and `J()` from Kotlin are `no-candidate-in-file` and unattributed;
  - `K.Companion.kf()` and `KUtilKt.top()` from Java are external.
  - CHANGELOG.md:263-264 notes Java findings whose only callers are "in Kotlin (not linked)".

## 05. Type inference & receiver typing

Supported shapes:

- **Parameters** `fun r2(s: Svc) { s.go() }` → `name: Type` from `signature_hint` (type_hints.rs:1425-1446, `parse_colon_param`; unit test type_hints.rs:2993). Fixture f10 L7: high.
- **Nullable parameters** `s: Svc?` → `Svc`, via `declared_type` (annotation.rs:167, 173). Unit test `lang_params("kotlin", "fun f(a: Foo?, b: List<Foo>?)")` (type_hints.rs:3426). Fixture f10 L8: high.
- **Property and local declared types** `val q: Square`, `private var h: Foo?`, `lateinit var e: Svc` → `LocalType` (record_local_type, kotlin.rs:397-444).
  - Nullable is stripped.
  - A nested type `Hpack.Reader` → `Reader`.
  - A package-path type `okio.Buffer` → not typed.
  - Test `a_nullable_property_is_typed_as_its_type` (kotlin.rs:739). Fixture f7: H4 (`private val d: Svc = Svc()`) and H5 (`lateinit var`) are high.
- **Constructor-initialised locals** `val x = Foo(…)` → `Foo`, only when the callee is a single capitalised identifier (kotlin.rs:446-466). Fixture f6 L21-22 `late`/`n` are high.
- **Scoped locals**: a Kotlin local is looked up in the calling function before the file-wide map (type_hints.rs:785-800; CLAUDE.md "C# and Kotlin locals are looked up in the calling function before the file-wide map").
- **Lambdas and object expressions see the enclosing function's parameter types** (type_hints.rs:700-717, "Kotlin (WS14b)"). The type's trailing `?` is trimmed. Primary-constructor spans (`Constructor` defs) are excluded on purpose.
- **Casts and constructions** (§03): `as`, `as?` and `Foo(…)` give `typed_receiver`.
- **Implicit receivers**: `settle_implicit_receivers` (type_hints.rs:1373-1406) keeps `x`'s receiver for a bare call in a `run`/`apply`/`with` lambda only when `x`'s type declares that member. Otherwise the call reverts to bare. Test `implicit_receivers_settle_by_the_receivers_members` (type_hints.rs, around line 4160).
- **Return types**: `extract_return_type` reads `fun f(): Foo` (type_hints.rs:2652-2660). Test `extract_return_type("kotlin", "fun f(): Foo? = null")` (type_hints.rs:3498).
  - **But see below**: return types do not type a Kotlin chained receiver or a local.

Not supported or partial:

- **Primary-constructor properties as receiver types**: `class H1(private val a: Svc) { fun f() { a.go() } }` fans out by name. This is an explicit gap (CHANGELOG.md:709 "Not covered (WS14b) … Kotlin primary-constructor properties as receiver types"). Fixture f7 H1/H2/H3: `Svc.go` + `Other.go`, both medium.
  - It only appears to work when a single candidate exists (f6 `Holder.a -> Svc.go`, high through the single intra-file candidate).
- **Locals from a function result** `val r = ret(); r.go()`: untyped, fanned out (fixture f7 L11, fixture f10).
- **Chained calls through return types** `ret().go()`: untyped, fanned out (fixture f10 L6). `Registry.lookup("k").area()` is unresolved (`candidates-in-other-files`, fixture f1 L44).
- **Lambda parameters** `f { s -> s.go() }`, `it.go()`: untyped. `closure_types.rs` covers only Dart/C#/Scala, and `CALLBACK_TYPE_CONSTRUCTORS` has no Kotlin row (stdlib.rs:143-147). Fixture f10 L12.
- **Smart casts** `if (x is Svc) x.go()`: the receiver keeps the declared `Any` type and is unresolved (`candidates-in-other-files`, fixture f7 L13).
- **`Any` is not a top type for Kotlin**: `TOP_TYPES` lists Scala, C# and Dart only (stdlib.rs:151-155).
- **Delegated properties** `val g by lazy { Svc() }`: untyped (fixture f7 H6, medium fan-out).
- **Generic containers** `List<Svc>`, `vararg s: Svc` → the container. `s.first().go()` fans out (fixture f10 L9, L10).
- **No Kotlin entries** in `MAYBE_TYPE_WRAPPERS`, `NULL_TYPE_MEMBERS` or `UNION_TYPE_CONSTRUCTORS` (stdlib.rs:104-140). Nullability is handled only by the `?` marker.
- **Weak receiver types** (`RefRecord::weak_receiver_type`): never set by the Kotlin plugin.
- **Locals are file-wide in the plugin** (`scope_byte` = declaration start). Scoping is applied in the resolver (above), not via `scoped_types`.

## 06. Inheritance, dispatch & overloading

Supported:

- **Base-class records**: `kotlin_bases` (kotlin.rs:589-626) reads every `delegation_specifier`: bare `I`, `Base()`, `Exception("x")`, `I by impl` and `pkg.Gen<T>()`.
  - These go both to `FileFacts::classes` (one `ClassDecl` per class and object, including bodiless ones, kotlin.rs:288-298) and to each member's `base_types`.
  - Tests `a_superclass_written_as_a_constructor_call_is_a_base` (kotlin.rs:966) and `a_one_line_top_level_object_keeps_its_scope`.
- **Inherited members**: `l.shared()` on a `Leaf` → `Base.shared` (fixture f2 L14, high), through `resolve_via_bases`.
- **`super.m()`**: resolves through the base chain. Fixture f2: `Leaf.start -> Base.start` through `Mid`, medium.
  - It is **never high** for Kotlin: `super_owner_may_be_anonymous` (cross_file.rs:4694-4699) exists because an object expression's `super` is recorded against the enclosing class. CHANGELOG.md:57.
  - Test `super_resolves_through_the_declared_base_chain` (resolve.rs:2862).
  - A chain leaving the tree is refused as `super-base-out-of-graph` (Kotlin records every class; CLAUDE.md step 3).
- **`member_of_calling_class`**: a bare call reaches the calling class's own or inherited member before any import (cross_file.rs:5348-5375). Fixture f2 L8: `Leaf.start -> Base.shared` (high).
- **Overrides below a base member**: in f2, `Base.start`'s `draw()` bound to both `Base.draw` and `Square.draw` (fixture f1 L11, medium). This is the override fan-out.
- **`--dynamic-dispatch` / inheritance fan-out**: `kotlin` is in `INHERITANCE_LANGS` (dispatch.rs:110-123). Fixture f2 `--dynamic-dispatch` produced:
  - `Base.start -> Leaf.start`, `Base.hook -> Leaf.hook`;
  - `Svc.go -> SvcA.go`, `Svc.go -> SvcB.go`;
  - all `low`, `dynamic dispatch:inheritance`.
- **Extension functions** (step 0b, cross_file.rs:5310-5333):
  - `expr.ext()` binds to an extension declared in the calling file or imported by name, unless the receiver's known type declares a same-named member (members win, `kotlin_extension_in_scope`, cross_file.rs:3611-3645).
  - Duck-typed fan-out drops out-of-scope extensions and non-extension top-level functions (`receiver_call_target`, cross_file.rs:3544-3580, 6992-7008).
  - Tests `kotlin_extension_calls_resolve_through_their_scope`, `kotlin_duck_typing_offers_only_extensions_in_scope` (resolve.rs:2974, 3017).
- **Overload sets**: `kotlin` is in `binds_same_name_set` (intra_file.rs:828-850), so same-file same-QN overloads are one callee. They bind every overload at medium and are not cap-refused (cross_file.rs:4779-4790).

Not supported or partial:

- **No arity narrowing**: `OVERLOAD_ARITY_LANGS = ["java", "scala", "cpp"]` (type_hints.rs:1583). The plugin never sets `arity`.
  - Fixture f1: `overloaded(1)` and `overloaded(1, 2)` each bind both overloads at medium (4 edges).
  - Fixture f2: `o.f(1)` and `o.f(1, "a")` likewise.
- **Same-file member of another class is ambiguous**: a bare `hook()` in `Base.start` with `Base.hook` and `Leaf.hook` both in the file is refused as `ambiguous-in-file`. Intra-file does not prefer the enclosing class's member for Kotlin, as it does for Dart/C++. Fixture f2: two `ambiguous-in-file` records (L3, L8).
- **`CLASS_RECORDS_ARE_EXACT`** excludes Kotlin (`["java", "csharp"]`, dispatch.rs:130). Member `base_types` are merged into the owner's bases.
- **Bare-call scope is not restricted** for Kotlin: `bare_call_reaches` documents Kotlin as "unchanged where the bare-name scope depends on types cgg does not have" (cross_file.rs:4083-4085).
- **Object-expression methods** have no anonymous owner (they are QN'd at the enclosing scope, f2 `p.go`). Calls through the object variable fan out (f2 L3: `o.go()` → `Svc.go`, `SvcA.go`, `SvcB.go`, medium).
- **Interface members are not tagged `abstract`** (§02), so they are never dropped in favour of a concrete candidate.
- **`impls` signal not declared**: the dead-code table reports `dispatch: none` for Kotlin (fixture f4 capabilities). Every finding carries `language-lacks-dispatch-model`.
- **Class delegation** `: Svc by d`: no forwarding edges.

## 07. Scoping & shadowing

Supported:

- **`FileFacts::locals`**: Kotlin has a `locals.rs` table (`KOTLIN_SPEC`, locals.rs:244-261; dispatch at locals.rs:386).
  - Binders: `parameter`, `variable_declaration` (in scope after its `property_declaration`), `lambda_parameters`.
  - Functions: `function_declaration`, `lambda_literal`, `anonymous_function`, `secondary_constructor`.
  - Blocks: `statements`, `for_statement`, `control_structure_body`.
  - Patterns: `multi_variable_declaration`.
  - Tests `dart_ocaml_lua_python_rust_kotlin_swift_scala_zig` (locals.rs:840; `fun f(a: Int) { val k = 1; xs.map { e -> e } }` binds `a`, `k`, `e`) and `a_declaration_is_not_in_scope_in_its_own_initializer` (Kotlin case `val now = now()`).
- **`shadowed-by-local`**: a bare call through a parameter or local is never another file's function. Fixture f6: `fun shad(target: () -> Unit) { target() }` and `val target = { 1 }; target()` → both `shadowed-by-local`.
- **Lexical narrowing of nested definitions**: `kotlin` is in `lexically_scoped` (intra_file.rs:120-138). A class span (Kotlin `Constructor` def) is not treated as a lexical scope (intra_file.rs:369-376). Unit test at intra_file.rs:1207-1217 ("a Kotlin member is not a local of its class's constructor span").
- **External-head shadowing**: a receiver headed by a local is not an import (heads.rs, `shadows_receiver_head`).

Not supported:

- **Positional shadowing**: OCaml/F#/Lua only (intra_file.rs:405).
- **Conditional definitions** (`DefRecord::conditional`): PHP only. Not applicable to Kotlin.
- **Nested functions carry no parent in their QN** (§02). Two local functions of the same name in different functions of one file share a QN, and only lexical narrowing tells them apart.

## 08. Macros, preprocessor & codegen

Not applicable. Kotlin has no macros or preprocessor.

- No `macro:<name>` tagging occurs.
- No `name_mentions` are recorded (kotlin.rs never writes `name_mentions`).
- Annotation processors and compiler plugins are not modelled: kapt/KSP-generated code (Dagger components, Room DAOs, `kotlinx.serialization` serializers, Compose compiler output). Generated sources are analyzed only if they exist on disk in the tree.
- `@file:JvmName("AKt")` has no effect on naming (fixture f8).

## 09. FFI & cross-language links

- **JNI**: not supported for Kotlin. `ffi::classify_ffi` keys `native` / `jniexport` attributes (ffi.rs:664, 668), but Kotlin declares native methods with the `external` modifier, which the plugin does not record as an attribute. Fixture f5: `w.Nat.nativeCall attrs=` (empty), and no FFI edge.
- **PyO3, wasm-bindgen, napi, P/Invoke, `extern "C"`, asm**: not applicable.
- **Descriptor links** (`descriptor::link_descriptors`) are language-agnostic: a Protobuf rpc links to any method whose owner contains the service name. Fixture f5: `g.Greeter.SayHello -> w.GreeterImpl.sayHello` (low, `descriptor-linker`).
- **Markup code-behind** (`markup.rs`): C# only. Android XML layouts (`android:onClick="handler"`) and data-binding expressions are not read.
- **JVM interop with Java/Scala/Groovy files in the same tree**: no edges (§04, fixture f9; lang_family.rs:32).

## 10. Frameworks & entry points

Rules are keyed `(id, language)` and matched against `rule.language == "kotlin"` only (cgg-resolve/src/frameworks/mod.rs:82-111, 596). `cgg-core/src/frameworks/rules.rs` has **30 Kotlin rules**:

| id | line | kind | node | trigger |
| --- | --- | --- | --- | --- |
| `spring` | 769 | network | yes | detect `org.springframework.web`/`.boot`; attributes `RequestMapping`, `GetMapping`, `PostMapping`, `PutMapping`, `DeleteMapping`, `PatchMapping`. Fixture f5: `<framework-entry>::network::spring::GetMapping("/x") -> w.Ctl.get`. |
| `ktor` | 2346 | network | yes (none minted) | detect `io.ktor.server`/`routing`/`application`; **gap**: routes are trailing lambdas, so there is nothing to attach to. |
| `android-worker` | 3716 | queue | yes | detect `androidx.work`; methods `doWork`, `startWork`, `createWork`, `onStopped`. Fixture f5: `<framework-entry>::queue::android-worker::w.W.doWork`. |
| `android-broadcastreceiver` | 3733 | network | yes | detect `android.content.BroadcastReceiver`, appwidget, glance, admin; method `onReceive`. |
| `android-jobservice` | 3755 | queue | yes | detect `android.app.job`; methods `onStartJob`, `onStopJob`. |
| `android-service-bind` | 3772 | network | yes | detect Service/IntentService/LifecycleService/…; methods `onBind`, `onUnbind`, `onRebind`. |
| `android-service-start` | 3797 | network | yes | same detect; methods `onStartCommand`, `onHandleIntent`, `onHandleWork`. |
| `android-activity` | 3822 | lifecycle | no | detect Activity/appcompat/activity/fragment/flutter; **gap only**, nothing enumerated. Fixture f3 shows it under "seen, no rules". |
| `android-fragment` | 3845 | lifecycle | no | **gap only**. |
| `android-service` | 3866 | lifecycle | no | **gap only**. |
| `android-contentprovider` | 3891 | network | yes (none minted) | **gap only**. |
| `android-application` | 3913 | lifecycle | no | **gap only**. |
| `aws-lambda` | 4166 | network | yes | detect `com.amazonaws.services.lambda`; base types `RequestHandler`, `RequestStreamHandler`; method `handleRequest`. Fixture f5: `…aws-lambda::w.H.handleRequest`. Test `aws-lambda/kotlin` (crates/cgg/tests/frameworks.rs:1186). |
| `azure-functions` | 4324 | network | yes | detect `com.microsoft.azure.functions`; attribute `FunctionName`. Test `azure-functions/kotlin` (frameworks.rs:1307). |
| `gcp-functions` | 4870 | network | yes | detect `com.google.cloud.functions`; bases `HttpFunction`, `BackgroundFunction`, `CloudEventsFunction`, `RawBackgroundFunction`, `TypedFunction`; methods `service`, `accept`, `apply`. Test `gcp-functions/kotlin` (frameworks.rs:1325). |
| `junit` | 7495 | **test** | yes | see §11. |
| `kotlinx-coroutines` | 7529 | lifecycle | no | **gap only**: coroutine bodies are lambdas; no value references. |
| `kotlin-runtime` | 7546 | lifecycle | no | detect `kotlin`, `kotlinx`, `java` (fires on almost every file); **gap only**: `init {}`, accessors, `operator fun`, `toString`/`equals`/`hashCode`, delegates, `::` refs. Fixture f1 lists it under "seen, no rules". |
| `jmh` | 9445 | **test** | yes | detect `org.openjdk.jmh`, `kotlinx.benchmark`; attributes `Benchmark`, `Setup`, `TearDown`. |
| `cucumber` | 9521 | **test** | yes | detect `io.cucumber`, `cucumber.api`; attributes `Given`, `When`, `Then`, `And`, `But`, `Before`, `After`, `BeforeStep`, `AfterStep`, `BeforeAll`, `AfterAll`, `ParameterType`, `DataTableType`, `DocStringType`…. |
| `dagger` | 9556 | lifecycle | no (root mark) | detect `dagger`; attributes `Provides`, `Binds`, `BindsOptionalOf`, `Multibinds`, `ContributesAndroidInjector`. Fixture f5: `w.M.prov` root `dagger:attribute`. |
| `guice` | 9579 | lifecycle | no | detect `com.google.inject`; attributes `Provides`, `ProvidesIntoSet`, `ProvidesIntoMap`, `ProvidesIntoOptional`, `Inject`. |
| `eventbus` | 9602 | lifecycle | no | detect Guava/greenrobot eventbus; attribute `Subscribe`. |
| `gradle-task` | 9619 | lifecycle | no | detect `org.gradle.api`; attributes `TaskAction`, `Input*`, `Output*`, `Nested`, `Classpath`…. |
| `spring-batch` | 9901 | queue | yes | detect `org.springframework.batch`; listener attributes (`BeforeStep`, `AfterChunk`, `OnReadError`…); bases `Tasklet`, `ItemProcessor`, `ItemReader`, `ItemWriter`, `ItemStream*`, `StepEx…`; methods `execute`, `process`, `read`, `write`, `open`, `update`, `close`, …. |
| `spring-integration` | 10017 | queue | no | **gap only**. |
| `jpa-lifecycle` | 10059 | lifecycle | no | detect `jakarta.persistence`/`javax.persistence`; attributes `PrePersist`, `PostPersist`, `PreUpdate`, `PostUpdate`, `PreRemove`, `PostRemove`, `PostLoad`. |
| `android-compose` | 10322 | lifecycle | no | **gap only**: `setContent { }` roots are lambdas. |
| `compose` | 11278 | lifecycle | no (root mark) | detect `androidx.compose`, `org.jetbrains.compose`, `androidx.activity.compose`; attribute `Composable`. Fixture f5: `w.Screen`, `w.Inner` rooted by `compose:attribute`. |
| `ktor-client` | 11299 | lifecycle | yes (none minted) | **gap only**. |

Helper tables (rules.rs, after line 11660): Kotlin has **no** rows in any of these:

- `member_sources_for` (only `junit-sources`/java, nunit, xunit);
- `signature_methods_for` (only `java-serialization`/java);
- `container_attributes_for` (Rust only);
- `markup_kinds_for` (C# only);
- `visibility_entries_for` (Solidity only);
- `detect_calls_for`;
- `self_module_markers_for`.

There are **no registrar verbs**: no Kotlin rule sets `registrars`, so `registrar_verbs_for("kotlin")` is `None`, and the plugin ignores `ExtractCtx` anyway.

Missing compared with Java:

- There is no Kotlin `junit-sources` (`@MethodSource`) and no `java-serialization` row.
- There are no Kotlin rows for Micronaut, Quarkus, Vert.x, Javalin, Exposed, Room, Retrofit, Koin, Hilt (`@HiltAndroidApp`, `@AndroidEntryPoint`) or kotlinx.serialization. Grep shows none with `language: "kotlin"`.
- The **gap texts of the five Android lifecycle rules and `kotlin-runtime` are stale**. They say "cgg's Kotlin plugin does not record class supertypes" (rules.rs:3822-3913, 7546), but supertypes have been recorded since 0.6.8 (kotlin.rs:56-61, `kotlin_bases`). The Activity, Fragment, Service and Application rules could now match by `base_types`, as `aws-lambda`/`gcp-functions` already do for Kotlin.

## 11. Test harness entries

- **`junit`** (rules.rs:7495, kind `test`):
  - detect `org.junit`, `kotlin.test`, `junit.framework`;
  - attributes `Test`, `BeforeTest`, `AfterTest`, `BeforeClass`, `AfterClass`, `ParameterizedTest`, `RepeatedTest`, `TestFactory`, `TestTemplate`, `BeforeEach`, `AfterEach`, `BeforeAll`, `AfterAll`, `Before`, `After`, `Ignore`.
  - It mints one harness node `<framework-entry>::test::junit` with `entry` edges to each case. Fixture f5: `junit -> w.CtlTest.setUp`, `junit -> w.CtlTest.works`. Dead-code roots: `framework:test-harness`, kind `test-entry`.
- **`jmh`** (rules.rs:9445) and **`cucumber`** (rules.rs:9521) are also kind `test` (annotation based).
- **Annotation vs naming**: all Kotlin test rules are annotation based. The built-in name rule `is_test_name` falls to the default `test_` / `Test` prefix for Kotlin (deadcode/roots.rs:975-988). Kotlin backtick names (`` fun `does a thing`() ``) without `@Test` are not tests (fixture f5: `` w.CtlTest.`backtick name` `` reported).
- **Built-in `TEST_ATTRS`** includes `Test` (roots.rs:211-225), so `@Test` is also a `builtin:test-attr` root without the junit import.
- **`TEST_CLASS_MEMBERS_ARE_ROOTS`**: Python/PHP/Ruby only (rules.rs `TEST_CLASS_MEMBERS_ARE_ROOTS`). Kotlin test-class helpers stay reportable (fixture f5: `w.CtlTest.helperT`).
- **Test-file gating**: by path only (`src/test/`, `*Test.kt`, `*Tests.kt`; testfile.rs:61, 103). The plugin does not tag `TestRole` (`test_defs` not declared). The dead-code capabilities show `test_tagging: none` (fixture f4/f5).
- **Not supported**: Kotest (`StringSpec`, `FunSpec { test("…") { } }`), Spek and MockK DSLs (lambda bodies, no rule); Android instrumentation (`androidTest/` not a recognised test path).

## 12. Dead-code analysis

Capabilities (fixture f4/f5, `--dead-code-format json`): `class: analyzable`, `visibility: full`, `attributes: full`, `value_references: none`, `dispatch: none`, `exports: none`, `test_tagging: none`, `max_confidence: high`. These follow from the declared signals (deadcode/caps.rs:150-200).

Root kinds that fire for Kotlin:

- `builtin:main` (`main`, roots.rs:20).
- `builtin:lifecycle`: Kotlin `LIFECYCLE` = `main`, `run`, `onCreate`, `onStart`, `onResume`, `onPause`, `onDestroy` (roots.rs:132-142), plus `CONVENTIONAL_METHODS` `toString`, `equals`, `hashCode`… (roots.rs:162-171). Fixture f3: `onCreate`, `toString` and `Impl.run` are not reported.
- `builtin:test-attr`, `builtin:test-name`.
- Framework roots `framework:entry-node`, `framework:rule` and `framework:test-harness` (fixture f5 roots list).

Visibility:

- `private` and `internal` → `private-visibility` evidence (corroborating).
- `public` and `protected` → `public-visibility` risk (deadcode/mod.rs:945-965).
- Fixture f3: `deadPriv`, `deadInternal` and top-level `private fun deadTop` carry `private-visibility`; `deadPub` and `deadProt` carry `public-visibility:public` / `:protected`.

Confidence caps:

- **Every Kotlin finding is capped at medium** in practice, because the language lacks value references and a dispatch model (`language-lacks-value-references`, `language-lacks-dispatch-model`, deadcode/mod.rs:918-924). Fixture f3: all 10 findings are medium. Fixture f4: `e.C.priv` is medium despite being private.
- **`implements-external-contract` never fires for Kotlin**. The `override` modifier is not captured as an attribute, so `OVERRIDE_MARKERS` (`@Override`, `override`, `@override`, deadcode/mod.rs:409) never matches. Kotlin is not Python (the only unmarked case, mod.rs:497-502). Fixture f4: `class C : Consumer<String> { override fun accept(…) }` → `e.C.accept` has no `implements-external-contract` evidence.
- `implicitly-invokable-kind` applies to class constructor nodes (fixture f3 `d.Impl`).
- `macro-generated`: not applicable.
- `reflection`: there is no Kotlin `dynuse` (§15).

Unresolved-doubt handling is generic: `no-unresolved-site-with-name` appears on findings (fixture f3).

Not supported:

- Operator functions (`plus`, `invoke`, `get`, `componentN`) are reported dead even when used by syntax (fixture f3: `d.MyAct.plus` reported while `a + a` is written in `main`).
- `::ref` targets are reported dead (fixture f3 `byRef`).
- `exports`: Kotlin has no export records. Public API is reflected only through visibility.

## 13. Builtins & stdlib tables

- **`stdlib/kotlin.txt`**: 304 lines (`wc -l`), loaded at stdlib.rs:190, 231. It lists names like `listOf`, `println`, `with`, `apply`, `buildList`, `arrayOf`, `Any`, `ArrayList`….
  - With `--include-stdlib`, bare calls to these become `<stdlib>::name` exit nodes. Fixture f1: `<stdlib>::println`, `<stdlib>::listOf`, `<stdlib>::with` (low, `stdlib exit-node`).
  - **Partial**: receiver calls on untyped stdlib values (`list.map {}`, `list.forEach {}`, `x.toDouble()`, `this.uppercase()`) go to `<external>::list::map` etc., not stdlib (fixture f1). `q.apply { }` on a typed `Square` becomes `<external>::Square::apply`.
- **Kotlin-specific constants**: `KOTLIN_RECEIVER_SCOPE_EXTENSIONS = ["run", "apply"]` and `KOTLIN_RECEIVER_SCOPE_FUNCTIONS = ["with"]` (stdlib.rs:25, 29).
  - **Partial**: `let`, `also` and `takeIf` (which bind `it`, not `this`) correctly have no implicit receiver.
  - **Gap**: `T.run`/`apply` with a non-identifier receiver (`make().apply { }`) gives no implicit receiver (kotlin.rs:112-120 accepts only a `simple_identifier`; test case `with(make()) { other() }` → `("other", "", false)`). Other receiver lambdas (`buildString { append() }`, `also`, DSL builders) are not modelled.
- **`CAST_INTRINSICS`**: none for Kotlin (stdlib.rs:35-46). Kotlin casts are syntax (`as`/`as?`) and are handled in `receiver.rs`.
- **`CONSTRUCTING_METHODS`**: Ruby only (stdlib.rs:58). Kotlin constructs by call (`constructs_by_call`, receiver.rs:46-51).
- **Implicit packages**: none. There is no Kotlin analogue of `SCALA_IMPLICIT_PACKAGES`, so `kotlin.*`, `kotlin.collections.*` and `java.lang.*` default imports are not modelled. A dotted `kotlin.x.y()` head counts as a package only when `stdlib_names("kotlin")` contains it, or the file or tree names it (heads.rs:788-797).
- **No `rust_types`-like wrapper table** for Kotlin (`Lazy`, `Deferred`, `StateFlow`, `MutableState` are not seen through).

## 14. Confidence & certainty

Paths that yield `high` for Kotlin:

- A single in-file candidate (intra-file). Fixtures f1, f6.
- An import naming the callable's full path, with `qn_carries_module("kotlin")` true (cross_file.rs:4775-4780, 5758-5768). Fixture f1: `main -> util.Helper`, `main -> util.topHelper`.
- An imported extension named by a direct import (cross_file.rs:2738-2753). Fixture f1 L40.
- A typed receiver from a declaration, cast or construction that names one class (`OwnerFacts`), with JVM anchoring through imports (`anchored`, cross_file.rs:4623-4626). Fixtures f7 (`as`/`as?`, `lateinit`, `private val d: Svc = Svc()`) and f10 (typed parameters).
- The caller's own class or an inherited member when exactly one ancestor declares it (fixture f2 `Leaf.start -> Base.shared`).

Forced to medium or low:

- `super` calls are never certain for Kotlin (`super_owner_may_be_anonymous`, cross_file.rs:4694-4699; CHANGELOG.md:57).
- Overload sets bind every overload at medium (no arity narrowing).
- Duck-typed by-name fan-out is medium: untyped constructor properties, lambda parameters, function results.
- Inheritance fan-out (`--dynamic-dispatch`) is low.
- Framework entries, descriptor links and exit nodes are low.

Mermaid labels an edge with no high site with a leading `?` (fixture f2: `N19 -->|?2x| N16`).

## 15. Dynamic use & reflection

None. Kotlin is not in `dynuse.rs`'s `REFLECTIVE` table, which covers Python, Ruby, Java, C#, JS, TS, PHP and Go (dynuse.rs:26-55). The plugin does not declare `dyn_uses`.

- Fixture f3: `MyAct::class.memberFunctions.first { it.name == "byReflect" }` gives no reflection screen, and `byReflect` is reported dead at the ordinary medium.
- `Class.forName("…")` and `::class.java.getMethod("…")` written in Kotlin are not screened either. The Java table is keyed `"java"` only (unverified by fixture; inferred from dynuse.rs:42).
- There is no module-load screen (`DYN_MODULE_LOADERS`) for Kotlin (`ServiceLoader.load`).

## 16. Known gaps & limitations

From code comments, rule gaps and the CHANGELOG:

- Primary-constructor properties are not receiver types (CHANGELOG.md:709; fixture f7).
- Kotlin is not linked to Java in mixed trees (CHANGELOG.md:263-264; lang_family.rs:32; fixture f9).
- Coroutine bodies, `::` references and lambdas have no nodes and no value refs (rules.rs:7529 `kotlinx-coroutines` gap).
- `init {}`, companion initialisers, property accessors, `operator fun`, `by` delegates and `toString`/`equals`/`hashCode` are invoked by syntax (rules.rs:7546 `kotlin-runtime` gap).
- Ktor routes, the Ktor client, Compose `setContent {}`, Spring Integration and the Android Activity/Fragment/Service/ContentProvider/Application lifecycles are gaps (rules.rs:2346, 11299, 10322, 10017, 3822-3913).
- **Stale gap prose**: six Kotlin rule gaps (rules.rs:3822, 3845, 3866, 3891, 3913, 7546) still claim "the Kotlin plugin does not record class supertypes", which has been false since 0.6.8 (kotlin.rs:56-61). The framework-coverage output repeats the stale claim (fixture f3).
- `bare_call_reaches` deliberately leaves Kotlin's bare-call scope unrestricted (cross_file.rs:4083-4085).

Found while cataloguing (fixtures):

- No arity on call sites, so overloads always bind all members at medium (f1, f2).
- A same-file bare member call is refused as `ambiguous-in-file` when another class in the file declares the same name (f2).
- Import aliases of objects or object members do not resolve: `import …Fmt as F; F.fmt()`, `import u.Q.qf as aliased` (f1, f8).
- `Op.Factory.create()` through a **named** companion is unresolved, although `Op.create()` resolves (f8).
- Function return types do not type locals or chains in Kotlin (f7, f10).
- Lambda parameters, smart casts and `by lazy` properties are untyped (f7, f10).
- Operators, infix calls, `invoke`, indexing and destructuring produce no edges (f1, f8).
- `override` and `external` modifiers are not recorded, so:
  - there is no `implements-external-contract` evidence (f4);
  - there is no JNI FFI classification (f5).
- Interfaces get a constructor node, and interface members are not `abstract` (f1, f2).
- Nested local functions and object-expression members are QN'd at the enclosing scope, which allows QN collisions (f2).
- In a packaged file, top-level functions are `InherentMethod` (kind `method`), not `FreeFunction` (f1; cross_file.rs:3536-3539).
- `.kts` script top-level statements are `no-enclosing-callable` (f8).
- No Kotest/Spek rules; no `*IT.kt`/`androidTest` test-path recognition (testfile.rs:103).
- No reflection or `dynuse` screens (dynuse.rs:26-55).
- Grammar: a one-line nested class body marks the file `parse_status: error` (bisect fixture). A newline before `(` can merge two statements into one call (f1 L59/60).

Ignored tests: `grep -rn '#\[ignore' crates --include=*.rs` finds no `#[ignore]` test anywhere in the workspace, so none is Kotlin-specific.

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01. Detection & parsing | partial | `.kt`/`.kts` only; no shebang or content detection; the one-line top-level object misparse is repaired, but other grammar quirks remain |
| 02. Callables extracted | partial | functions, members, class-as-constructor, extensions; no accessors, lambdas, init blocks or script bodies; local and anonymous-object functions lose their owner |
| 03. Call-site forms | partial | bare, chained, cast and constructed receivers, generics and scope-function receivers; no arity, `::` refs, operators, infix calls or registrar args |
| 04. Imports & module resolution | partial | JVM imports, star imports, external-head refusal, extension imports; aliases of objects fail; no Java↔Kotlin linking |
| 05. Type inference & receiver typing | partial | params, declared and constructed locals, casts, enclosing params in lambdas; no constructor properties, return types, lambda params or smart casts |
| 06. Inheritance, dispatch & overloading | partial | bases recorded, super, inherited members, extensions, inheritance fan-out; no arity narrowing; same-file member ambiguity |
| 07. Scoping & shadowing | full | locals table, shadowed-by-local, lexical narrowing; positional and conditional shadowing n/a |
| 08. Macros, preprocessor & codegen | n/a | no macros; kapt/KSP codegen not modelled |
| 09. FFI & cross-language links | minimal | descriptor links only; `external fun` (JNI) not classified; no Java interop |
| 10. Frameworks & entry points | partial | 30 rules (Spring, Android workers and services, Lambda, Azure, GCP, Compose and Dagger roots); 12 gap-only; stale "no supertypes" prose |
| 11. Test harness entries | partial | JUnit, kotlin.test, JMH and Cucumber annotations; no Kotest/Spek; no TestRole tagging |
| 12. Dead-code analysis | partial | visibility and attributes full; all findings capped medium (no value refs or dispatch); no external-contract evidence for `override` |
| 13. Builtins & stdlib tables | partial | 304-name stdlib list; run/apply/with tables; no implicit packages or wrapper tables |
| 14. Confidence & certainty | partial | high via imports, typed receivers and single candidates; `super` never high; overloads always medium |
| 15. Dynamic use & reflection | none | Kotlin absent from `dynuse` |
| 16. Known gaps & limitations | n/a | catalogued above |
