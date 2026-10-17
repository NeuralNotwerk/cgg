# java

The Java plugin (`JavaPlugin`, `crates/cgg-lang/src/plugins/java.rs`, 947 lines, about 390 of them inline tests) is a single recursive tree walker (`JavaWalker::walk`, java.rs:88-214). It records packages, imports, classes with their bases, methods, constructors, call sites, method references, `new` constructions and explicitly typed locals. It parses with the crates.io grammar `tree-sitter-java = "0.23"` (Cargo.toml:87; `tree_sitter_java::LANGUAGE`, java.rs:30-32); nothing is vendored. Java has no dedicated `resolver_kind` override. It goes through the generic pipeline: `intra_file`, then `cross_file`, plus Java-specific branches in `heads.rs` (JVM package vocabulary and external-head refusal), `cross_file.rs` (static imports, `member_of_calling_class`, `overrides_below`, overload sets, owner-path narrowing), `type_hints.rs` (`OVERLOAD_ARITY_LANGS`), `dispatch.rs` (`INHERITANCE_LANGS`, `CLASS_RECORDS_ARE_EXACT`) and the dead-code visibility model (`deadcode/mod.rs:646-680`). The plugin declares these signals: `unreachable`, `visibility`, `attributes`, `impls`, `value_refs` and `dyn_uses` (java.rs:18-28). Java has the most framework rules of any language: 60 `RuleSpec`s with `language: "java"`, against 49 for Python, the next highest (`grep -o 'language: "java"'` on rules.rs).

Fixtures used below live in `<scratch>/parity/java/f1` … `f9`. They were run with `target/release/cgg` (0.11.0 tree, branch `remediate/cross-language`).

## 01. Detection & parsing

- **Extension `.java`**. `h.java` is detected as `java`. Evidence: java.rs:15-17; detect.rs:686 test table `("h.java", "java")`; integration test `nine_language_sample_all_detect` (crates/cgg/tests/detect.rs).
- **Shebangs**: none. The plugin does not override `shebangs()`. A single-file source launch script (`#!/usr/bin/java --source 11`) without the extension is not detected (unverified by fixture; inferred from java.rs having no `shebangs` fn).
- **Content detection**: none. detect.rs has no Java header sniffing.
- **Grammar**: one grammar, `tree-sitter-java` 0.23 (Cargo.toml:87, java.rs:31). There is no `ts_language_for` override. Parser smoke test: parser.rs:154 `("java", b"class A { void m() {} }")`.
- **Re-parse/fallback**: none. A file with parse errors is still extracted. There is no blanking pass like the C# `#if` one (java.rs has no re-parse).
- **Depth skip**: a tree deeper than `MAX_TREE_DEPTH` = 4000 is skipped as `TooDeep` (cgg/src/lib.rs:155, 451). This applies to every language.
- **Minified skip**: `MINIFIED_LINE_BYTES` = 2000 (cgg/src/lib.rs:159) gates `locals::collect`. That is moot for Java, which has no locals table (section 07).
- **Test-file classification** (`cgg_core::classify_test_file`):
  - A Maven/Gradle `src/test/` path counts (testfile.rs:61-67).
  - So do the file-name suffixes `*Test.java`, `*Tests.java` and `*IT.java` (testfile.rs:98-102).
  - Test: `app/src/test/java/com/x/FooTest.java` is a test file and `app/src/main/java/com/x/Foo.java` is not (testfile.rs:188-191).
- **Not handled**:
  - `module-info.java` is parsed as an ordinary Java file. `module`/`requires`/`provides … with …` directives produce no facts. The `java-serviceloader` gap text says so explicitly (rules.rs:7110).
  - `package-info.java` yields only a package record.

## 02. Callables extracted

Qualified names are `package.Outer.Inner.member`, joined with `.` from a scope stack. The scope stack holds the package and then every named class, interface, enum or record (java.rs:80-86, 90-103, 108-138).

- **Instance method** `void run() {}` becomes `DefVariant::InherentMethod`, kind `method` (java.rs:228-249). Test `class_methods_with_package`: `com.example.Service.run`.
- **Static method** `static void s() {}` becomes `StaticMethod`. It is read from the `static` keyword token inside `modifiers`, not from text (java.rs:236-247). Tests `static_method_variant` and `static_is_the_keyword_not_a_substring` (an annotation `@SuppressWarnings("static-access")` does not make a method static).
- **Constructor** `public Foo(int x) {}` becomes `DefVariant::Constructor` with QN `pkg.Foo.Foo` (java.rs:144-154). Test `constructor_variant`. Fixture f1: two constructors `com.acme.svc.Service.Service` (L10, L11) share one QN.
- **Overloads**: each overload is a separate node with the **same QN**. Fixture f1: `com.acme.svc.Service.over` ×3. Same-file same-QN sets bind as one callee (`binds_same_name_set`, intra_file.rs:830-850, lists `"java"`).
- **Interface members**:
  - Abstract `void run();`, `default` and `static` interface methods are all extracted as methods. `Vis` is Public unless the member is explicitly `private` (java.rs:535-542).
  - Test `interface_members_without_a_modifier_are_implicitly_public`. Fixture f1: `com.acme.svc.Runner.run`, `.twice` and `.noop`.
- **Abstract class methods** `public abstract void run();` are extracted, bodiless. Fixture f1: `com.acme.svc.Base.run`.
- **Enum methods and constructors**:
  - Enum methods follow class rules, and an enum constructor with no modifier is Private (java.rs:543-547). Test `enum_and_record_members_follow_class_rules`.
  - Fixture f1: `com.acme.svc.Helper.Mode.flip`.
- **Record methods** `record Point(int x) { int twice() … }` are extracted. Fixture f1: `com.acme.svc.Helper.Point.twice`. **Not extracted**: the record's implicit canonical constructor and its component accessors (`x()`), because no declaration node exists for them (fixture f1: no `Point.x` node).
- **Nested and inner classes** are scoped under the outer class. Fixture f1: `com.acme.svc.Helper.Nested.deep`; fixture f4: `p.Outer.Inner.i`.
- **Local class inside a method** gets QN `Outer.Local.lm`, scoped by class name but not by the enclosing method (fixture f1: `com.acme.app.Main.Local.lm`).
- **Anonymous class bodies**: their methods are extracted under the **enclosing class's** scope. Their `base_types` are the anonymous class's own base (java.rs:189-209; test `anonymous_class_methods_carry_the_anonymous_base`).
  - **Partial**: QN collisions follow. Fixture f1: `new Runnable() { public void run() {…} }` inside `Main.go` became `com.acme.app.Main.run`, indistinguishable from a real `Main.run`.
  - Fixture f4: the enum-constant body `A { void f() {…} }` produced a second `p.E.f` node colliding with the enum's own `p.E.f`.
- **Lambdas** `x -> lambdaTarget(x)` are **not callables**. Their call sites are attributed to the enclosing method (fixture f1: `Main.go -> Main.lambdaTarget` high; fixture f2: `new Thread(() -> lam())` gives `User.use -> User.lam`).
- **Initializers are not callables**: `static { … }`, an instance `{ … }` block and field initializers.
  - Calls in them get `no-enclosing-callable`. Fixture f1: `boot`, `instanceInit` and `compute` at Service.java L5/L7/L8. Fixture f4: `fieldLambda = () -> fromFieldLambda()` was also `no-enclosing-callable`.
  - The `java-runtime` rule gap text documents this (rules.rs:7144-7158).
- **Annotation type elements** `@interface Ann { String value(); }`: **not extracted**. `annotation_type_declaration` is not in the class arm (java.rs:108-111), and elements are `annotation_type_element_declaration`, not `method_declaration`. Fixture f4: `Ann.java` produced no callables.
- **`native` methods** are extracted, and the attribute `native` is appended (java.rs:270-281). Tests `a_native_method_is_marked_native` and `an_annotation_containing_native_is_not_the_modifier`.
- **Attributes**: annotations are kept verbatim through `attrs::collect`, for example `@GetMapping("/users")` and `@Override` (java.rs:268-271). Fixture f1: `Service.run` attributes `['@Override']`.
- **base_types**: a method carries its enclosing class's `extends`/`implements` list (java.rs:282, through `self.bases`). Classes are also recorded as `ClassDecl` with exact bases, bodiless classes included (java.rs:116-127). Test `every_class_is_recorded_with_its_bases_bodiless_ones_included`.
- **Visibility** (`java_vis`, java.rs:525-553):
  - `public`, `protected` and `private` map to Public, Protected and Private.
  - A missing modifier maps to `Internal` (package-private), except that interface and annotation members are implicitly Public and an enum constructor is Private.
  - Tests: `an_annotation_argument_does_not_hide_the_access_modifier`, `modifier_words_outside_the_modifiers_node_are_not_modifiers`, `class_bodies_inside_an_interface_follow_class_rules`.
- **Signature hint**: `extract_signature` of the declaration text (java.rs:265). Arity narrowing and framework signature hooks read it.
- **Not extracted**: fields and properties (there is no getter/setter synthesis), generated code (Lombok, annotation processors), and record implicit members.

## 03. Call-site forms

- **Bare call** `helper()` records name `helper` with an empty receiver (java.rs:334-369). Test `method_invocation_captured`. Fixture f1: `Main.go -> Main.helperLocal` high.
- **Method call** `obj.run()` records `receiver_hint = "obj"`. Test `method_invocation_captured`.
- **Chained call** `a.b().c()`: the receiver text is kept verbatim (`Service.make().self()`). Return types are not followed for Java (section 05), so chains off an untyped call stay unresolved or fan out. Fixture f1: `Service.make().self().run()` left `self` and `run` unresolved (`candidates-in-other-files`).
- **Static/qualified call** `Helper.util(4)` and the fully qualified `com.acme.svc.Helper.util(5)`. Fixture f1: both high via `cross-file:imports`.
- **Receiver shapes peeled** (`receiver::peel("java", …)`, java.rs:351-360; receiver.rs:60, 175, 197):
  - `(f).m()` reads as `f`.
  - `((Foo) o).m()` and `new Foo().m()` are typed `Foo` and set `typed_receiver`.
  - Test `java_parens_casts_and_constructions` (receiver.rs:446). Fixture f1: `((Service) p).run()` was high, and so was `new Service().getHelper()`.
  - Integration test `shaped_java_receivers_resolve_as_their_typed_form` (crates/cgg/tests/resolve.rs).
- **Constructor call** `new Foo(a, b)` records a RefRecord named after the type text, with arity (java.rs:172-188).
  - Works for non-generic types. Fixture f1: `Main.main -> Service.Service` high.
  - **Bug/gap**: the type text is used verbatim, so `new Box<String>()`, `new Box<>()` and `new p.Box<String>("x")` are named `Box<String>`, `Box<>` and `p.Box<String>`. They never match the constructor and fall into the external bucket. Fixture f7: three `<external>::Box<…>` exit nodes and no constructor edges.
- **Explicit constructor invocation** `this(…)` / `super(…)`: **not recorded**. The walker has no arm for `explicit_constructor_invocation` (java.rs:88-214). Fixture f7: no `Box() -> Box(T)` edge and no unresolved record.
- **Explicit type arguments** `this.<String>generic("g")`: recorded under the bare name, because the grammar's `name` field excludes the type arguments. Fixture f1: `Main.go -> Main.generic` high.
- **Method references** `this::m`, `Type::m`, `h::c`, `super::m` and `Type::new`:
  - Recorded only when `ExtractCtx::reference_sites` is set (`--reference-edges` or `--dead-code`). They are marked `value_ref` (java.rs:160-166, 385-441).
  - `Type::new` becomes a reference to the type's constructor. `int[]::new` is skipped.
  - Tests: `method_references_are_value_refs_through_their_receiver` and `method_references_need_reference_sites`.
  - Fixture f1: `xs.forEach(this::refTarget)` gives `Main.go -> Main.refTarget` high `reference`. Fixture f7: `Box::new` gives two medium reference edges, one to each constructor.
- **Lambdas passed as values**: their bodies' calls are attributed to the enclosing method (section 02). The lambda itself is not a value reference.
- **Registrar-argument capture** runs on every call (`registrar::capture`, java.rs:370-373). It is **inert for Java** because no built-in Java rule declares `registrars` (every Java `RuleSpec` has `registrars: NONE`; rules.out extraction) and `registrar_verbs_for` is built from those (rules.rs:11905-11930).
- **Arity**: calls and constructions carry `arity` from `arg_count` (java.rs:182-184, 367; plugins.rs:67). Test `calls_record_their_argument_count`. Method references carry no arity.
- **Switch-expression arms, try-with-resources, enhanced-for bodies**: their calls are recorded normally. Fixture f1: `switch (n) { case 1 -> sw(); … }` gave two high edges.
- **Not recorded**: operator use (Java has no operator overloading), string concatenation's implicit `toString`, auto-unboxing, enhanced-for's implicit `iterator()`, try-with-resources' implicit `close()`. The `java-closeable` rule roots `close` instead (rules.rs:7127).

## 04. Imports & module resolution

- **`import a.b.C;`** is recorded as kind `import`, path `a.b.C`, alias `C` (java.rs:322-331).
  - `heads.rs` binds `C` to package `a.b` only when the last segment is capitalised (heads.rs:531-534).
  - `cross_file` registers `C` as a direct import of `a.b.C` (cross_file.rs:1980-2000).
  - Test `imports_captured`. Integration test `java_cross_file_import_resolves`.
- **`import a.b.*;`** reaches the plugin as path `a.b` (the asterisk is a sibling token) with kind `import`. heads.rs binds nothing for it ("reaches the plugin as `a.b` and binds nothing", heads.rs:529-530).
  - Workspace wildcard: fixture f2 `import a.*;` then `sh.draw()` with `Shape sh` was high.
  - External wildcard: fixture f4 `import org.ext.lib.*;` then `ExtThing.call()` went to the external bucket (`<external>::ExtThing::call`).
  - The plugin does **not** mark the non-static wildcard (only the static form gets `*`, java.rs:303-314), so `cross_file` sees suffix `b`. That this is harmless is unverified.
- **`import static a.b.C.m;`** is kind `from-import`, alias `m`. `cross_file` keeps the path as the member's QN (cross_file.rs:1650-1658). Fixture f1: `util(3)` gave `Helper.util` high. Test `java_static_imports_reach_members_of_another_class`.
- **`import static a.b.C.*;`** is recorded with alias `*` (java.rs:303-312). `cross_file` adds `a.b.C` to `unqualified_prefixes` (cross_file.rs:1644-1649). Test `static_wildcard_import_is_recorded_as_star`. Fixture f2: `import static b.Util.*;` then `stat1()` was high.
- **Packages** (`FileFacts::packages`): the `package` declaration is recorded and also pushed as the outermost QN scope (java.rs:90-103). Test `package_is_recorded`.
  - Workspace vocabulary: `jvm_packages`, `jvm_prefixes`, `jvm_roots` and `jvm_segments` come from Java/Kotlin packages (heads.rs:142-151).
  - The packages are also used by libscope.rs:755 and by dead-code package-private reachability (deadcode/mod.rs:648-652).
- **External-module refusal** (`heads.rs`, `external_head`, Java listed at heads.rs:666):
  - An imported type whose package is not a workspace package is external (heads.rs:531-538).
  - A dotted receiver headed by a lowercase package path (`java.util.Objects.requireNonNull`, `org.x.Y.f`) is refused by `jvm_package_head` (heads.rs:774-810) when the head is a stdlib root, a file import root or a workspace root, and the package is not a workspace one.
  - Tests: `java_type_imported_from_a_dependency_is_not_the_local_class`, `java_package_qualified_receiver_outside_the_tree_is_external` (cross_file.rs:11370, 11408), `chained_calls_are_refused_only_off_a_module_binding` (heads.rs:1141), `jvm_package_parts` (heads.rs:1083).
  - Fixture f1: `Strings.isNullOrEmpty` (imported `com.google.common.base.Strings`) became `<external>::Strings::isNullOrEmpty`.
- **Same-package visibility without import**: a bare type name resolves to the caller's package's class. Fixture f2: `Dup d; d.m()` in package `c` bound `c.Dup.m`, not `b.Dup.m` (medium).
- **Owner-path narrowing** (`narrow_owner_paths`, cross_file.rs:4835-4860): `Response.Builder` matches owners whose path ends with the qualifier as written, and a bare name prefers the type nested in the caller's scope. Covers java, kotlin, scala and groovy.
- **Copies of one class**: copies across directories narrow to the nearest (`nearest_class_copies`; CHANGELOG lines 544-548). There is a unit test at cross_file.rs:8445-8459 (`has("java","Svc")`, `!has("java","Dup")`).
- **Not applicable**:
  - `not-in-imported-module` and `modules.rs` `ModuleIndex` cover JS/TS/Python/Go/Erlang only. Java is absent there (grep of modules.rs finds no `"java"`).
  - There is no manifest reading. `pom.xml`, `build.gradle` and `settings.gradle` are not read, so module or artifact names are never workspace vocabulary.
  - There are no re-exports in Java.
- **Kotlin/Scala/Groovy interop**: **none**. The JVM language family is commented out (lang_family.rs:32). Fixture f8: Kotlin `j.jm()` and `J.js()` on a Java class J both gave `no-candidate-in-file`.

## 05. Type inference & receiver typing

- **Parameter types** are read from the signature, type-first, with `split_top_level` (type_hints.rs:1422-1470).
  - Generic parameters type as their outer type: `Map<String, Handler> h` is `Map`.
  - Annotations and `final` are skipped.
  - Tests: `java_style_type_first_param` (type_hints.rs:3372), `a_generic_parameter_is_its_generic_type_not_an_argument`, and the `lang_params("java", "void f(@Nullable Foo a, final @NonNull Bar b, int c)")` case (type_hints.rs:3471-3476).
  - Fixture f1: `void go(Service p, …) { p.run(); }` was high.
- **Explicitly typed locals** `Service s = …;` and `Gen<String> one = …;` (outer type) are recorded as `LocalType` with `scope_byte` = the declaration (java.rs:443-479). Fixture f4: `Gen<String> one = gens.get(0); one.map(null)` was high.
- **`var x = new Foo()`** is typed from the construction (java.rs:481-494).
- **File-wide locals, last write wins**: Java is not in the "look up in the calling function first" set (C#/Kotlin only, per CLAUDE.md).
  - Fixture f5: `one() { A x = new A(); x.go(); }` and `two() { B x = new B(); x.go(); }` led `one`'s `x.go()` to bind **`p.B.go`** (wrong edge, medium).
  - Confidence is downgraded when the declaration is not in scope. Test `java_declared_receiver_through_import_is_high_file_wide_guess_is_medium` (crates/cgg/tests/resolve.rs).
- **Return types: not extracted for Java**. `extract_return_type` (type_hints.rs:2632-2672) handles `->`, `): T` and Go's trailing type, but not a type written before the name. `public Tool getTool()` yields nothing (the comment at 2650 says "Java/C#" but the code has no such branch). Consequences, seen in fixture f4:
  - `var t = makeTool(); t.work()` fanned out.
  - `getTool().work()` fanned out.
  - Both bound **`q.Tool.work` and `q.Other.work`** at medium, one of them wrong.
  - Fixture f1: `var v = Service.make(); v.run()` was unresolved (`candidates-in-other-files`, 4).
- **Field types: not recorded**. The plugin writes no `class_fields`; only Python, Scala and Kivy do (grep of `class_fields.push`).
  - Fixture f4: `tool.work()` and `this.tool.work()` with `private Tool tool;` both fanned out to Tool and Other at medium.
  - Fixture f1: `field.run()` and `this.field.run()` were unresolved.
- **Assignment from a typed local** `Tool local = tool;` is typed by its declaration. Fixture f4: `local.work()` was high.
- **Casts and constructions** as receivers are typed (section 03).
- **Not typed**:
  - Enhanced-for variables `for (Gen<String> g : gens)`: fixture f4 `g.map` was unresolved.
  - Catch parameters: fixture f4 `e.printStackTrace()` went to the external bucket.
  - Try-with-resources resources `try (Gen<String> r = make())`: fixture f4 `r.map` went external.
  - Array elements: `arr[0].map` went external.
  - Pattern variables (`o instanceof Foo f`, record patterns): unverified, and there is no plugin code for them.
  - Lambda parameters: `closure_types.rs` covers Dart, C# and Scala only.
- **Nullability/wrappers** (`declared_type`): Java has no `MAYBE_TYPE_WRAPPERS` entry. `Optional<Foo>` stays the container `Optional<Foo>` (annotation.rs:230 test; stdlib.rs:100-123 comment "Java's `Optional<T>`" is deliberately absent). `@Nullable` annotations on parameters are stripped.
- **Implicit receivers / weak receiver types**: not applicable. `implicit_receiver` and `weak_receiver_type` are Kotlin and Python only, and java.rs never sets them.
- **`this`/`super` receivers**: `this.m()` resolves in the calling class. `Outer.this.o()` is **not** understood: fixture f4 put it in the external bucket as `<external>::Outer.this::o`.

## 06. Inheritance, dispatch & overloading

- **Base-class records**: every class, interface, enum and record is written to `FileFacts::classes` with its exact `extends`/`implements` list (java.rs:116-127). Java is in `CLASS_RECORDS_ARE_EXACT` (dispatch.rs:130), so a method's `base_types` are not unioned into its owner.
- **Inherited method via base chain** (`resolve_via_bases`). Fixture f1: `Service.run` calling `inherited()` defined in `Base` gave `Base.inherited` high. Fixture f2: `Square.viaBare` calling `draw()` gave `Shape.draw` high.
- **`member_of_calling_class`**: a bare call binds to the calling class's member or an inherited one before any static import (cross_file.rs:5353-5375, 5650-5668). Tests `java_static_import_does_not_hide_an_inherited_member` (cross_file.rs:11446), `java_bare_member_is_the_qualified_calling_class` and `java_bare_call_reaches_bases_not_unrelated_classes`.
- **`overrides_below`**: a bare call to a member that classes derived from the caller override also binds those overrides. This needs `derived_index`, which is built for Java and C# only (cross_file.rs:422-441). Tests `java_bare_call_reaches_overrides_in_derived_classes` and `java_bare_call_to_an_overridden_member_reaches_the_overrides`. In fixture f2 the call and the base method were in one file, so intra-file bound only `Shape.render` (high).
- **`super.m()`** resolves through the base chain. Fixture f2: `Square.render` calling `super.render()` gave `Shape.render` high.
  - When the chain leaves the tree it is refused as `super-base-out-of-graph`. Fixture f2: `super.add("x")` with `extends java.util.ArrayList` was refused this way, and so was fixture f4 `super.handle()`. Test `java_and_csharp_super_into_an_outside_base_is_refused`.
  - An anonymous class body's `super` is its own base (java.rs:189-209).
- **Interface dispatch / `--dynamic-dispatch`**: Java is in `INHERITANCE_LANGS` (dispatch.rs:110-123). Fixture f1 with `--dynamic-dispatch` gave `Runner.run -> Service.run` and `Base.run -> Service.run` (low `dynamic`). Fixture f2 gave `Shape.render -> Square.render, Circle.render`. Tests `java_interface_and_abstract_members_fan_out_to_implementations` and `java_chain_runs_through_a_class_with_no_methods` (dispatch.rs:707, 730).
- **Typed-receiver call on an interface** binds to the interface declaration. Fixture f1: `Runner r = s; r.run()` gave `Runner.run` high. The implementations are reached only with `--dynamic-dispatch`.
- **Arity-based overload narrowing**: Java is in `OVERLOAD_ARITY_LANGS` (type_hints.rs:1583), and `accepts_arity` handles varargs `T...`.
  - Intra-file: fixture f3 `fmt(1)` bound `fmt(int)` and `fmt(String...)`, and `fmt(1, 2)` bound `fmt(int,int)` and `fmt(String...)`, both medium. Test `java_overload_cut_to_one_by_arity_is_high`: an intra-file set cut to one is high.
  - Cross-file through a typed receiver, cut to one: **medium**. Fixture f1: `s.over(1)` gave exactly `over(int)` but at medium; fixture f2 `sh.over(1)` did the same. Overloads share one QN, so the key is in `qn_collisions` (cross_file.rs:1013-1021) and `CertainCtx::path` refuses it (cross_file.rs:4488-4497). That this is the cause is inferred, not traced.
  - Java and C# exact lookups expand to the whole overload set first (`with_overloads`, cross_file.rs:5669-5681). Test `java_overload_sets_through_bases_and_static_wildcards`.
- **One overload set is one callee** (`one_method`) and is not refused by the fan-out cap except at `--fanout-cap 0` (cross_file.rs:4790-4800).
- **Not applicable**: extension methods, mixins/traits, included modules. Default interface methods act as inherited members: fixture f1 `r.twice()` gave `Runner.twice` high.
- **Gap**: `Outer.this.m()` and enum-constant receivers. Fixture f4: `A.f()` inside `enum E` went to `<external>::A::f`, so an enum constant is not typed as its enum.

## 07. Scoping & shadowing

- **`FileFacts::locals`: not supported**. `locals::spec` has no Java entry; the list is JS/TS, Python, Lua, Scala, Kotlin, Swift, Rust, Dart, OCaml and Zig (locals.rs:380-393). So:
  - There is no `shadowed-by-local` refusal for Java.
  - No receiver is recognised as a local rather than an import head. heads.rs only checks `locals.shadows_receiver_head`, which is empty for Java. A local variable named like an imported class would read as the class (unverified by fixture).
- **Lexical narrowing of nested definitions**: Java is not in the list. Java has no nested functions, so this is mostly not applicable. Local and anonymous classes are flattened into the enclosing class's scope (section 02).
- **Positional shadowing**: not applicable.
- **Conditional definitions**: not applicable. Java has none.
- **Scope of typed locals**: they are file-wide with `scope_byte`, so the declaration offset only affects confidence (section 05).

## 08. Macros, preprocessor & codegen

- Not applicable: Java has no macros or preprocessor. No `macro:<name>` tagging and no `name_mentions` are recorded (java.rs never writes `name_mentions`).
- **Generics** are not expanded. Generic arguments are stripped from parameter and local types (outer type kept). They are **not** stripped from `new` constructions (section 03 bug).
- **Annotation processors, Lombok, Dagger/AutoValue codegen and protoc output** are invisible: the `grpc-java` gap text (rules.rs:2329-2345) and the `java-runtime` gap text say so.

## 09. FFI & cross-language links

- **JNI, Java side**: a `native` method gets attribute `native` (java.rs:270-281), and `classify_ffi` maps it to `("jni", Import)` (ffi.rs:664-668, 823; test at ffi.rs:1039).
  - Imports never get a peer edge: "Only exports get a speculative cross-language peer edge" (`detect_ffi_family`, ffi.rs:678-686).
  - Fixture f5: `public native int compute(int)` plus a C `JNIEXPORT jint JNICALL Java_p_Use_compute(...)` produced **no** Java↔C edge. Dead-code reported `Java_p_Use_compute` as an unreferenced C function.
  - A Java `native` method is not a dead-code root either. Fixture f3: `public native void jni()` was reported dead at medium (public).
- **JNI, C side**: the `jniexport` attribute maps to `("jni", Export)` (ffi.rs:664). Whether the C plugin records `JNIEXPORT` as an attribute is unverified. In fixture f5 it did not root the function.
- **Protobuf rpc → Java implementation**: the descriptor linker lower-cases the first letter. Fixture f3: `rpc SayHello` gave `Greeter.SayHello -> w.GreeterImpl.sayHello` (low `descriptor`). Test `java_lowercases_the_first_letter_and_still_links` (descriptor.rs:205).
- **Not applicable**: PyO3, wasm-bindgen, napi, P/Invoke, asm and markup code-behind (`markup_kinds_for` is C# only, rules.rs:11812-11818).

## 10. Frameworks & entry points

All 60 rules with `language: "java"` are listed below, extracted from `crates/cgg-core/src/frameworks/rules.rs` (line = start of `RuleSpec`). Kind is the `TrustKind`. "node" means an entry node is minted; otherwise the rule only marks roots. Base-type matching follows the transitive chain: in fixture f9, `MainAct extends BaseAct extends AppCompatActivity` rooted `MainAct.onResume` through `android-activity:base-type`, even though `MainAct.java` imports nothing. It also accepts generic interfaces: fixture f9's `implements RequestHandler<String,String>` minted an `aws-lambda` entry.

| rule id | line | kind | node | trigger |
| --- | --- | --- | --- | --- |
| spring | 601 | network | yes | attrs RequestMapping, Get/Post/Put/Delete/PatchMapping, MessageMapping, ExceptionHandler, InitBinder, ModelAttribute; detect `org.springframework.web/boot`. Fixture f3: `<framework-entry>::network::spring::GetMapping("/users")`; test `shape_a_java_spring_mapping_keeps_its_route` |
| spring-jobs | 629 | schedule | yes | attrs Scheduled, Async (f3: `spring-jobs::w.Web.tick`) |
| spring-messaging | 646 | queue | yes | attrs KafkaListener, RabbitListener, JmsListener, StreamListener, RabbitHandler, KafkaHandler |
| jakarta-rs | 679 | network | yes | attrs GET/POST/PUT/DELETE/PATCH/HEAD/OPTIONS/Path |
| micronaut | 700 | network | yes | attrs Get/Post/Put/Delete/Patch/Head/Options/Controller |
| akka | 726 | queue | yes | bases AbstractBehavior/AbstractActor/UntypedAbstractActor/AbstractLoggingActor; methods createReceive/onReceive/preStart/postStop |
| java-concurrent | 750 | lifecycle | no | bases Runnable/Callable/Thread/TimerTask; methods run/call |
| servlet | 2231 | network | yes | bases HttpServlet/GenericServlet/Filter/HttpFilter/OncePerRequestFilter/listeners; methods doGet…doTrace, service, doFilter, doFilterInternal, context/session/request lifecycle |
| vertx | 2274 | network | yes | detect only; **gap** (verticles, lambda routes) |
| dropwizard | 2291 | network | yes | detect only; **gap** |
| struts | 2308 | network | yes | detect only; **gap** |
| grpc-java | 2329 | network | yes | detect only; **gap** (`QueryGrpc.QueryImplBase` bases are generated and matched by exact name) |
| android-activity | 3367 | lifecycle | no | bases Activity/AppCompatActivity/…; methods onCreate…onNewIntent… |
| android-fragment | 3414 | lifecycle | no | bases Fragment/DialogFragment/…; lifecycle methods |
| android-service | 3456 | lifecycle | no | bases Service/IntentService/…; onCreate/onDestroy/… |
| android-service-bind | 3497 | network | yes | same bases; onBind/onUnbind/onRebind |
| android-service-start | 3531 | network | yes | same bases; onStartCommand/onHandleIntent/onHandleWork/onAccessibilityEvent |
| android-broadcastreceiver | 3570 | network | yes | bases BroadcastReceiver/AppWidgetProvider/…; onReceive/onUpdate/… |
| android-contentprovider | 3605 | network | yes | bases ContentProvider/FileProvider/…; query/insert/update/delete/… |
| android-worker | 3651 | queue | yes | bases Worker/CoroutineWorker/…; doWork/startWork/createWork/onStopped |
| android-jobservice | 3675 | queue | yes | bases JobService/JobIntentService; onStartJob/onStopJob |
| android-application | 3692 | lifecycle | no | bases Application/MultiDexApplication; onCreate/onTerminate/… |
| aws-lambda | 4145 | network | yes | bases RequestHandler/RequestStreamHandler; method handleRequest (test `aws_lambda_java_request_handler_is_a_declared_contract`; gap text: bare-method lambdas invisible) |
| azure-functions | 4307 | network | yes | attr FunctionName (gap: trigger annotation on a parameter not read) |
| gcp-functions | 4844 | network | yes | bases HttpFunction/BackgroundFunction/CloudEventsFunction/RawBackgroundFunction/TypedFunction; service/accept/apply (test `gcp_functions_binds_every_runtime_shape`) |
| junit | 7057 | test | yes | see section 11 |
| junit-params | 7088 | test | yes | detect only; **gap** (indirect sources, @Nested, extensions) |
| java-serviceloader | 7110 | lifecycle | no | detect only; **gap** (META-INF/services, `provides … with`) |
| java-closeable | 7127 | lifecycle | no | bases AutoCloseable/Closeable; method close |
| java-runtime | 7144 | lifecycle | no | detect only; **gap** (initializers, shutdown hooks, finalize, reflection) |
| jsr250-lifecycle | 7163 | lifecycle | no | attrs PostConstruct/PreDestroy (f3: `w.Web.init` rooted via `jsr250-lifecycle:attribute`; test `lifecycle_annotations_root_only_the_annotated_method`) |
| spring-context | 7182 | lifecycle | no | attrs Bean, EventListener, TransactionalEventListener |
| guice | 7203 | lifecycle | no | attrs Provides, ProvidesIntoSet/Map/Optional, Inject |
| dagger | 7228 | lifecycle | no | attrs Provides, Binds, BindsOptionalOf, Multibinds, ContributesAndroidInjector |
| jsr330-inject | 7252 | lifecycle | no | attr Inject |
| gradle-task | 7271 | lifecycle | no | attrs TaskAction, Input*, Output*, … |
| gradle-services | 7308 | lifecycle | no | attr Provides; base ServiceRegistrationProvider; method configure (test `a_service_provider_contract_roots_configure_only`) |
| gradle-model-rules | 7326 | lifecycle | no | attrs Model/Mutate/Defaults/Finalize/Validate/Rules/ComponentType/… |
| jackson | 7357 | lifecycle | no | attrs JsonCreator/JsonValue/JsonAnySetter/JsonAnyGetter/JsonGetter/JsonSetter/JsonProperty |
| moshi | 7386 | lifecycle | no | attrs FromJson/ToJson |
| eventbus | 7404 | lifecycle | no | attr Subscribe |
| java-serialization | 7424 | lifecycle | no | `signature_methods_for` (see below); f3 `readObject(ObjectInputStream)` rooted |
| junit-sources | 7444 | test | yes | attr Parameters + `member_sources_for` |
| testng | 7464 | test | yes | attrs Test/DataProvider/Factory/Before*/After*… |
| jmh | 9428 | test | yes | attrs Benchmark/Setup/TearDown |
| caliper | 9463 | test | yes | attrs Benchmark/BeforeExperiment/AfterExperiment/BeforeRep/AfterRep/Macrobenchmark |
| cucumber | 9489 | test | yes | attrs Given/When/Then/And/But/Before/After/BeforeStep/AfterStep/BeforeAll/AfterAll |
| jpa-mapping | 9686 | lifecycle | no | attrs Column/Id/EmbeddedId/JoinColumn/OneToMany/… |
| camel | 9777 | network | no | detect only; **gap** |
| spring-batch | 9794 | queue | yes | attrs BeforeStep/AfterStep/…; bases Tasklet/ItemProcessor/ItemReader/ItemWriter/…; methods execute/process/read/write/… |
| spring-integration | 10000 | queue | no | detect only; **gap** (@ServiceActivator etc.) |
| jpa-lifecycle | 10034 | lifecycle | no | attrs PrePersist/PostPersist/PreUpdate/PostUpdate/PreRemove/PostRemove/PostLoad |
| hibernate-listeners | 10084 | lifecycle | no | bases Pre/Post Insert/Update/Delete/Load EventListener; onPre*/onPost* |
| kafka-streams | 10184 | queue | no | detect only; **gap** |
| flink | 10201 | queue | no | detect only; **gap** |
| storm | 10218 | queue | yes | bases BaseRichBolt/BaseBasicBolt/…/spouts; execute/prepare/cleanup/declareOutputFields/nextTuple/open/… |
| vaadin | 10288 | network | no | detect only; **gap** |
| jsf | 10305 | network | no | detect only; **gap** (EL strings) |
| javafx | 11169 | lifecycle | no | attr FXML; bases Application/Preloader/Initializable/EventHandler/ChangeListener/…; start/init/stop/initialize/handle/changed/… |
| swing | 11216 | lifecycle | no | methods actionPerformed/mouseClicked/… (listener methods by name) |

- **member_sources_for** (rules.rs:11719-11733): `("junit-sources","java")` handles `MethodSource` (`default_self: true`) and `ArgumentsSource` (`type_members: ["provideArguments"]`). Fixture f3: `@MethodSource("cases")` rooted `WebTest.cases` via `<framework-entry>::test::junit-sources`. Tests `method_source_roots_the_named_member_and_not_its_neighbours` and `member_sources_need_the_framework_detected`.
- **signature_methods_for** (rules.rs:11835-11859): `("java-serialization","java")` covers `readObject(ObjectInputStream)`, `writeObject(ObjectOutputStream)`, `readObjectNoData()`, `readResolve()` and `writeReplace()`. Test `serialization_hooks_match_by_name_and_parameter_list`.
- **container_attributes_for**: none for Java (Rust only, rules.rs:11770-11794).
- **markup_kinds_for**: none for Java (C# only).
- **visibility_entries_for**: none for Java (Solidity only, rules.rs:11686-11690).
- **Registrar verbs**: none. No Java rule declares `registrars` (all `NONE`), and `string_targets` is `false` on every Java rule.
- **Built-in non-rule roots** (deadcode/roots.rs), in addition to the framework rules:
  - `main` (`UNIVERSAL_ENTRY`, roots.rs:20).
  - Java `LIFECYCLE` names `main`, `run`, `call`, `onCreate`, `onStart`, `onResume`, `onPause` and `onDestroy` (roots.rs:119-130). These are by name alone, in any class.
  - `CONVENTIONAL_METHODS` `toString`, `equals`, `hashCode`, … (roots.rs:162-170). Fixture f3: `w.Web.toString` was rooted `builtin:lifecycle`.
- **Not covered** (gap texts above): Vert.x, Dropwizard, Struts, grpc-java, Camel, Spring Integration, Kafka Streams, Flink, Vaadin, JSF, ServiceLoader, and the `java-runtime` constructs.

## 11. Test harness entries

- **junit** (rules.rs:7057, kind test, one node `<framework-entry>::test::junit`):
  - Attributes: Test, ParameterizedTest, RepeatedTest, TestFactory, TestTemplate, BeforeEach/AfterEach/BeforeAll/AfterAll, Before/After/BeforeClass/AfterClass.
  - Bases: `TestCase` and `junit.framework.TestCase`, with methods `test*`, `setUp` and `tearDown` (JUnit 3).
  - Fixture f3: setup, works and param were entries. Fixture f6: JUnit 3 `LegacyTest extends TestCase` rooted `setUp` and `testA`, but not `helper` or `notATest`.
- **junit-sources** (7444): `@Parameters` plus the MethodSource/ArgumentsSource member sources (section 10).
- **testng** (7464). Fixture f6: `@DataProvider data` and `@Test ng` were entries.
  - **Quirk**: fixture f6's `NgTest.ng` (TestNG `@Test`) was *also* attached to `<framework-entry>::test::junit`, because another test file in the tree imported `junit.framework`. A name-only attribute like `Test` applies in any test file once the harness is detected.
- **jmh** (9428) and **caliper** (9463) are benchmark harnesses of kind test. Fixture f6: `@Benchmark b` became an entry under `test::jmh`.
- **cucumber** (9489): step definitions by `@Given`/`@When`/`@Then`/… (not run by fixture).
- **junit-params** (7088) is a gap-only rule: `@Nested`, `@RunWith(Parameterized.class)` constructors and `@ExtendWith` extensions are not enumerated.
- **Name-based tests** (`is_test_name`, roots.rs:975-990): Java uses the default arm, `test_*` or `Test*` names. These root anywhere, not only in test files. Fixture f6: `test_snake` and `TestPascal` were rooted `builtin:test-name`.
- **TEST_ATTRS** (roots.rs:211-226) includes `Test`, so `@Test` is a test root even without the junit rule.
- **TEST_CLASS_MEMBERS_ARE_ROOTS** is `["python","php","ruby"]` (rules.rs:11806). Java test-class helpers are **not** roots. Test `test_class_helpers_are_test_roots_only_in_dynamic_languages` (frameworks/mod.rs:2434). Fixture f6: `LegacyTest.helper` was reported medium dead under `--include-tests`.
- **Test-file gating**: `src/test/` or the `*Test.java`/`*Tests.java`/`*IT.java` names (section 01). The dead-code capability row reports `test_tagging: none` for Java (fixture f3 capabilities). Java has no in-file test signal.
- `--no-test-entries` turns harness nodes into root marks, as for every language.

## 12. Dead-code analysis

- **Capabilities row** (fixture f3 `--dead-code-format json`):
  - `visibility: full`, `attributes: full`, `value_references: full`, `dispatch: full`.
  - `exports: none`, `test_tagging: none`.
  - `max_confidence: high`, `class: analyzable`.
- **Root kinds seen for Java**:
  - `ProgramEntry` (`main`).
  - `LifecycleCallback` (the LIFECYCLE names, `toString`/`equals`/`hashCode`).
  - `FrameworkCallback` (framework rules, both node and root-mark kinds).
  - `TestEntry` (harnesses, `builtin:test-name`, `builtin:test-attr`).
  - `UserDeclared` (`cgg-deadcode.toml`).
  - There is no `ExportedApi` root: Java records no `exports`, so `public` is **not** a root. It is evidence: `public-visibility` caps a finding at medium (deadcode/mod.rs:960-966). Fixture f3: `unusedPub` was medium and `unusedPkg`/`unusedPriv` were high.
- **Visibility model**:
  - `private` reaches within the file, which stands in for the top-level class. Package-private reaches within the package (deadcode/mod.rs:646-680).
  - An unresolved site that could not legally call the member is no doubt. Test `a_site_that_cannot_legally_call_a_java_private_member_is_no_doubt` (deadcode/mod.rs:2452).
- **`implements-external-contract`**: `@Override` (in `OVERRIDE_MARKERS`, deadcode/mod.rs:405-409) on a member whose base is outside the tree caps the finding at medium. Fixture f5: `@Override handle()` and `unusedOverride()` on `extends org.ext.Handler` were both medium with `implements-external-contract`. Tests `an_override_of_an_out_of_tree_base_member_is_not_high` and `an_override_of_an_in_tree_base_member_carries_no_external_contract`.
- **Dispatch surface**: a base member whose override is called counts as live through dispatch. `--dead-code` turns on `inheritance_fanout`. Test `a_base_member_whose_override_is_called_is_a_dispatch_surface`.
- **Reflection cap**: a literal in `forName`/`getMethod`/`getDeclaredMethod`/`getField` caps same-named findings at medium (`name-collides-with-screened-site`, screen `reflection`; deadcode/mod.rs:620-638). Fixture f3: `getMethod("byReflection")` made `byReflection` medium.
- **Unresolved doubt**: an untyped, generic or over-cap receiver site with the same name counts as doubt only where it could legally reach the member. Test `a_variable_or_generic_receiver_is_doubt_and_an_unrelated_declared_type_is_not`.
- **Unreachable-code signal**: `cfg::JAVA` (cfg.rs:104-119) treats return, throw, break, continue and yield as terminators, with `local_variable_declaration` hoisted. It is extracted only under `deadcode_signals` (java.rs:52-55).
- **False positives caused by extraction gaps**:
  - Methods called only from `static {}`, instance initializers or field initializers read as dead (section 02). The `java-runtime` gap text says so.
  - Methods called only through `this(…)`/`super(…)` constructors (section 03), or only through `new Generic<>()` (section 03).
  - JNI `native` methods (section 09).
  - Members reached only through a field or a method return value can get *wrong* liveness (wrong-target edges, section 05).
- **Not applicable**: `macro-generated` (no macros).

## 13. Builtins & stdlib tables

- **`stdlib/java.txt`**: 399 names (`wc -l`), parsed into the stdlib table at stdlib.rs:230. It is used by `--include-stdlib` and by `jvm_package_head`'s `std_root` (heads.rs:793-794). Fixture f1 with `--include-stdlib`: `xs.add`/`xs.forEach` became `<stdlib>::List::add` / `::forEach`, and `new Runnable(){}` became `<stdlib>::Runnable`. Fixture f4: `Collections.emptyList()`, `E.valueOf`, `values()` and `gens.get(0)` were stdlib.
- **`--include-external`** produces deduplicated `<external>::…` exit nodes. They include misattributions from the gaps above (fixture f1: `<external>::field::run`, `<external>::v::run`; fixture f7: `<external>::Box<String>`).
- **`CAST_INTRINSICS`**: no Java entry. Java casts are syntax (`cast_expression`), handled by `receiver::peel` (receiver.rs:175).
- **`CONSTRUCTING_METHODS`**: no Java entry (Ruby only, stdlib.rs:58). `new` is syntax (receiver.rs:197).
- **`MAYBE_TYPE_WRAPPERS`, `UNION_TYPE_CONSTRUCTORS`, `NULL_TYPE_MEMBERS`, `CALLBACK_TYPE_CONSTRUCTORS`, `TOP_TYPES`**: no Java entries (stdlib.rs:104-155). `Object` is not a top type for Java, so an `Object o` parameter types `o` as `Object`. The effect is unverified.
- **Implicit packages**: there is no Java equivalent of `SCALA_IMPLICIT_PACKAGES`. `java.lang` is not modelled; names like `String` and `Math` reach stdlib by the name list. `java`/`javax` appear only in Scala's list (stdlib.rs:73-75, 96).
- **rust_types-like tables**: none.

## 14. Confidence & certainty

- **Paths that yield `high`** (`CertainCtx`, cross_file.rs:4468-4515; `unique_owner_names` treats Java owner sites by QN path, cross_file.rs:4411-4413; `qn_carries_module` includes java, cross_file.rs:4775):
  - An intra-file unique bare call (fixture f1).
  - A static or qualified call through an import or a full path (fixture f1: `Helper.util`, `com.acme.svc.Helper.util`).
  - A static import (fixture f1: `util(3)`).
  - A receiver typed by a parameter, a cast, a construction or an in-scope typed local whose class is imported or same-package and unique (fixture f1: `p.run()`, `((Service)p).run()`; fixture f4: `local.work()`).
  - An inherited member or `super` when the imports confirm the bases and exactly one ancestor declares it (fixture f1: `inherited()`; fixture f2: `super.render()`). The import check is `"java"|"kotlin"|"groovy"` at cross_file.rs:4623-4626.
  - An intra-file overload set cut to one by arity (test `java_overload_cut_to_one_by_arity_is_high`).
- **Forced `medium`**:
  - Overloads that stay several after arity (fixture f3).
  - Overloads cut to one but reached cross-file. Fixture f1/f2: `over` was medium, likely from `qn_collisions`.
  - By-name fan-out on an untyped receiver (fixture f4: `tool.work()` gave two medium edges).
  - A receiver typed only by another method's local (test `java_declared_receiver_through_import_is_high_file_wide_guess_is_medium`; fixture f5).
  - Same-name class narrowing by package (fixture f2: `d.m()`).
  - Method-reference edges to a constructor overload set (fixture f7).
- **`low`**: `--dynamic-dispatch` fan-out, framework-entry edges, descriptor (proto) edges, and exit nodes.
- **Mermaid `?` marker**: an arrow with no `high` site is labelled `?`. Fixture f1 mermaid: `N1 -->|?| N15` for the `over` overloads and `N14 -->|?2x| N7` for `helper.assist()` through an untyped field.
- **CHANGELOG** (lines 77-80): the share of Java arrows with no `high` site dropped from 78.8% to 50.2% after the confidence work.

## 15. Dynamic use & reflection

- **Reflection by literal name** (dynuse.rs:41-44): the callees `forName`, `getMethod`, `getDeclaredMethod` and `getField` are matched on Java's `method_invocation` `name` field (dynuse.rs:87-105). A literal must be identifier-shaped (dots allowed).
  - Test `java_get_declared_method_names_a_method` (dynuse.rs:183).
  - Fixture f3: `Class.forName("w.Web").getMethod("byReflection")` capped `byReflection` at medium.
  - This is suppression-only. It never creates an edge (dynuse.rs:7-12).
- **Not covered**:
  - `getDeclaredField`, `getConstructor`/`getDeclaredConstructor`, `getDeclaredMethods()` enumeration.
  - `MethodHandles.lookup().findVirtual/findStatic(…, "name", …)`, `java.beans` property names, `ServiceLoader`, `Proxy.newProxyInstance`.
  - Spring EL / JSF EL strings, and annotation strings other than `@MethodSource`/`@ArgumentsSource`.
  - Evidence: absence from dynuse.rs:41-44; the `java-runtime`/`jsf`/`java-serviceloader` gap texts.
- **Module load screen**: not applicable. `DYN_MODULE_LOADERS` is require/importScripts/import_module (facts.rs:165).

## 16. Known gaps & limitations

1. **No return-type extraction for Java** (type_hints.rs:2632-2672 handles only `->`, `): T` and Go). `var x = f()` and `f().g()` chains fan out by name or stay unresolved, which produces wrong-target medium edges (fixture f4: `getTool().work()` bound `q.Other.work`).
2. **No field types** (`class_fields` is never written by java.rs). `field.m()` and `this.field.m()` fan out or stay unresolved (fixtures f1, f4).
3. **File-wide last-write-wins locals**: the same variable name in two methods types the wrong class (fixture f5: `one()`'s `x.go()` bound `B.go`). Java is not in the C#/Kotlin "calling function first" set.
4. **Generic constructions never resolve**: `new Box<>()` and `new Box<String>()` keep their type arguments in the RefRecord name (java.rs:174-187). Fixture f7: three external exit nodes.
5. **`this(…)`/`super(…)` constructor chaining is not recorded** (no `explicit_constructor_invocation` arm, java.rs:88-214; fixture f7).
6. **Initializers are not callables**: `static {}`, instance `{}` and field initializers give `no-enclosing-callable` (fixtures f1, f4). Acknowledged in the `java-runtime` gap (rules.rs:7144-7158).
7. **Anonymous classes and enum-constant bodies flatten into the enclosing class's QN**, which causes collisions (fixture f1: `Main.run`; fixture f4: duplicate `p.E.f`).
8. **Annotation-type elements are not extracted** (fixture f4: `Ann`). Record implicit accessors and canonical constructors are not extracted either.
9. **Untyped binding sites**: enhanced-for variables, catch parameters, try-with-resources resources, pattern variables, lambda parameters, array elements, `Outer.this` and enum constants as receivers (fixture f4).
10. **Cross-file overloads cut to one by arity stay `medium`** (fixtures f1/f2). CLAUDE.md and the WS17 test describe this case as `high`; it holds only intra-file.
11. **No locals table**: no `shadowed-by-local`, and no local-versus-import disambiguation of receiver heads (locals.rs:380-393).
12. **JNI is one-way**: `native` methods are FFI imports with no peer edge to the C `Java_pkg_Cls_m` implementation. Both sides read as dead (fixture f5).
13. **No JVM-family cross-language resolution**: Kotlin, Scala or Groovy calling Java and the reverse are unresolved (lang_family.rs:32 commented out; fixture f8).
14. **No build-manifest reading** (`pom.xml`, Gradle) and no `module-info.java` directives.
15. **Framework gaps declared by `gap_for`**: vertx, dropwizard, struts, grpc-java, junit-params, java-serviceloader, java-runtime, camel, spring-integration, kafka-streams, flink, vaadin, jsf (the table in section 10).
16. **Harness attribution quirk**: once JUnit is detected anywhere, a TestNG `@Test` method is also attached to the junit harness (fixture f6).
17. **Reflection coverage is narrow**: only four callee names (section 15).
18. **Non-static wildcard import** `import a.b.*;` is not marked as a wildcard by the plugin (java.rs:322-331). Behaviour in `cross_file` depends on suffix handling (cross_file.rs:1980-2000), and its impact is unverified.

- No `#[ignore]`d Java tests were found (`grep -rn '#\[ignore' crates` shows none naming Java; unverified exhaustively).

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01 Detection & parsing | full | `.java` only, one grammar, no shebang or fallback; Maven test layout recognised |
| 02 Callables extracted | partial | methods, constructors, overloads, interface/enum/record members; no lambdas, initializers or annotation elements; anonymous bodies flattened |
| 03 Call-site forms | partial | bare, method, qualified, casts, method refs, arity; generic `new` broken; `this()`/`super()` missing |
| 04 Imports & module resolution | full | single, static, static-wildcard and wildcard imports; packages; external refusal; no manifests or JVM interop |
| 05 Type inference & receiver typing | partial | params, typed locals and `new` only; no return types or fields; file-wide locals |
| 06 Inheritance, dispatch & overloading | full | class records, base chains, super, overrides_below, inheritance fan-out, arity narrowing |
| 07 Scoping & shadowing | none | no locals table; not much needed for Java, but no shadowing at all |
| 08 Macros, preprocessor & codegen | n/a | Java has none; codegen invisible |
| 09 FFI & cross-language links | minimal | JNI `native` marked but never linked; proto rpc → Java impl works |
| 10 Frameworks & entry points | full | 60 rules (most of any language), member-source and signature hooks; 13 gap-only rules |
| 11 Test harness entries | full | JUnit 3/4/5, TestNG, JMH, Caliper, Cucumber; test-class helpers not rooted |
| 12 Dead-code analysis | full | Java visibility/package model, external-contract cap, reflection cap; extraction gaps cause false findings |
| 13 Builtins & stdlib tables | partial | 399-name java.txt; no wrapper, top-type or implicit-package tables |
| 14 Confidence & certainty | full | CertainCtx paths reach high; cross-file arity-to-one stays medium |
| 15 Dynamic use & reflection | partial | forName/getMethod/getDeclaredMethod/getField only |
| 16 Known gaps & limitations | n/a | 18 listed: return types, fields, file-wide locals, generic `new` most impactful |
