# groovy

The Groovy plugin is `GroovyPlugin` in `crates/cgg-lang/src/plugins/groovy.rs`. The file is 401 lines, of which 282 are the plugin and the rest are 8 inline tests. It parses `.groovy` and `.gradle` files (groovy.rs:15-17) with the crates.io grammar `tree-sitter-groovy` 0.1. The workspace pin is `Cargo.toml:106`; the lock resolves it to 0.1.2. The plugin calls `tree_sitter_groovy::LANGUAGE` (groovy.rs:18-20). Nothing is vendored. The plugin is registered at `crates/cgg-lang/src/plugins.rs:146`.

The grammar is Java-derived. Its `node-types.json` holds Java node kinds (`method_declaration`, `constructor_declaration`, `record_declaration`, `method_reference`, `lambda_expression`) plus Groovy additions (`closure`, `juxt_function_call`, `function_definition`, `map_literal`, `shebang`). It has no `trait` keyword. It also misreads much idiomatic, semicolon-free Groovy: in fixture f1 both files parse as `parse_status: error`.

The extractor is a single recursive `GroovyWalker` (groovy.rs:42-281). It keeps two stacks:

- `scope`: package and class names, which form the qualified name;
- `bases`: the supertypes of the enclosing class, read with `attrs::base_types`.

The walker records:

- methods and script-level `def` functions, as `DefRecord`s carrying base types and preceding annotations;
- imports, as `import` or `from-import`;
- `method_invocation` call sites, with the receiver peeled by `crate::receiver::peel("groovy", …)`.

It matches two node kinds, `function_call` and `closure_expression` (groovy.rs:124-133), that **do not exist in this grammar**, so both arms are dead code. It ignores its `ExtractCtx` (`_ctx`, groovy.rs:24). It overrides neither `signals()`, `shebangs()`, `max_line_bytes()`, `ts_language_for()` nor `resolver_kind`, so it declares no optional signal, not even `attributes`, although it records them. Resolution is the generic pipeline. Groovy-specific branches are listed in each section below.

Fixtures cited below live under `<scratch>/parity/groovy/` (f1 … f13). Each was run with `target/release/cgg` 0.11.0 using `-t json -o out.json`, sometimes with `--dead-code`, `--include-external`, `--include-stdlib` or `--dynamic-dispatch`, and dumped with `show.py` in that directory. Parse trees were dumped with `tsdump/` there, a 20-line Rust program built offline against the same `tree-sitter` 0.26 and `tree-sitter-groovy` 0.1.2.

## 01. Detection & parsing

- **Extensions `.groovy` and `.gradle`** (groovy.rs:15-17). Evidence: fixture f1 audit shows `detected_via: extension:.groovy`, and f11 shows `f11/build.gradle extension:.gradle`.
- **Extensions that are not detected**: `.gvy`, `.gy`, `.gsh`, `.gradle.kts` (Kotlin's), and a bare `Jenkinsfile`.
  - Fixture f11: `a.gvy`, `a.gsh` and `Jenkinsfile` are each `file_skipped … unknown-extension`.
  - The `jenkins-pipeline` rule gap text says the same (rules.rs:5760).
- **Shebangs**: none. `shebangs()` is not overridden, so the default `&[]` applies (cgg-lang/src/lib.rs:298-300). In fixture f11, an extensionless file starting `#!/usr/bin/env groovy` is skipped as `unknown-extension`. The grammar does have a `shebang` node, so such a file would parse if it were detected.
- **Content detection**: none. `crates/cgg/tests/detect_prefixes.rs:50` only exercises the extension path with `import {prefix}.Thing`.
- **Grammar**: one grammar for both extensions, with no `ts_language_for` override.
- **Grammar error recovery is poor on idiomatic Groovy**. The plugin still extracts from inside `ERROR` nodes, because the walker recurses into every child (groovy.rs:139-149). The failure modes below were confirmed with the tree dump:
  - The last statement of a block without a `;` becomes an `ERROR` node. For example, `void help() { inner() }` in f1 `Helper.groovy` puts `inner()` under `ERROR`. The call is still recorded.
  - Untyped parameters are an `ERROR` (`def scriptFn(a)`, `void helper(x)`). Fixtures f1 and f3 show `ERROR > type_identifier "a"`. The definition is still recorded.
  - A closure-valued field (`def lam = { x -> x * 2 }`) without a trailing `;` swallowed the next constructor and method into one `ERROR`. In f1, `Service()` and `void init() {}` were lost:
    - `Service.init` is not a node;
    - the `Service()` constructor became a `method_invocation` named `Service`, reported as `no-candidate-in-file` at line 31.
  - `trait Greeter { … }` is not a declaration: the grammar has no `trait`. In f2 it is a `juxt_function_call` named `String`, and `Greeter.greet` is not extracted.
  - A paren-less statement call inside a method (`helper 2;`) can parse as a `local_variable_declaration` (f3 tree.txt lines 125-133).
- **Re-parse or fallback**: none. A file with errors is extracted anyway and gets `parse_status = "error"` (generic, cgg/src/lib.rs). Every non-trivial fixture here (f1, f2, f3) is `error`. f11 `build.gradle` was `ok`.
- **Depth skip**: shared by every language. A tree deeper than `MAX_TREE_DEPTH` = 4000 is skipped as `SkipReason::TooDeep` (cgg/src/lib.rs).
- **Minified or long-line handling**: no `max_line_bytes` override. The plugin collects no locals, so the generic 2,000-byte locals rule is moot.
- **Test-file classification** (cgg-core/src/testfile.rs):
  - Groovy shares the Maven/Gradle `src/test/` layout rule with Java, Kotlin and Scala (testfile.rs:61-67).
  - There is **no file-name rule** for Groovy. The `by_name` match (testfile.rs:69-119) has no `groovy` arm, so `FooSpec.groovy` and `BarTest.groovy` outside `src/test/` are not test files.
  - The generic `TEST_DIRS` directory rule still applies (testfile.rs:125-131).

## 02. Callables extracted

Qualified names are built by `qn()` (groovy.rs:59-65): `<package>.<Class>.<Nested…>.<name>`, joined with `.`. The package text is pushed onto `scope` from `package_declaration` with `;` stripped (groovy.rs:69-79). Only `class_declaration`, `interface_declaration` and `enum_declaration` push a scope (groovy.rs:84-102). The plugin sets `visibility` to the empty string always (groovy.rs:187).

Supported shapes:

- **Instance method**: `void run() {}` gives `InherentMethod` `pkg.Class.run`, via `record_method` (groovy.rs:151-168). Test `class_methods_with_package` (groovy.rs:305) asserts `com.example.Service.run`.
- **Static method**: `static void sm() {}` gives `StaticMethod`. The check is whether the `modifiers` text contains `static` (groovy.rs:159-166). Fixture f2: `com.ex.Plain.sm`.
- **`def` method, typed or untyped parameters**: `def untyped(a, b)` and `def defNoParams()` are both nodes (f2), even though untyped parameters parse with `ERROR`.
- **Visibility modifiers**: `private`, `protected` and `public` methods are extracted (f2), but the modifier is not recorded as visibility.
- **Getter and setter methods**: `getName()` and `setName(..)` are ordinary methods (f2). Property access `obj.name` is not linked to `getName` (unverified; nothing in the plugin does it).
- **Operator-overload method** `Plain plus(Plain o)`: extracted as a plain method (f2). The `+` operator is never a call site (§03).
- **`call` method**: extracted as a method (f2). It is not linked from `obj(…)` (§03).
- **Inner and static nested classes**: `com.ex.Plain.Inner.innerM` and `com.ex.Plain.Nested.nestedM` (f2).
- **Interface methods**, abstract and `default`: `com.ex.Api.a` and `com.ex.Api.b` (f2). They are not tagged `abstract`; only TS/Swift/Scala/Dart bodiless declarations are.
- **Abstract class methods**: `abstract double area()` is a node (f1 `com.ex.Base.area`).
- **Enum methods**: `com.ex.Color.paint` (f2). Enum constants are not callables.
- **Script-level `def`/typed function** (`function_definition`): `def scriptLevel(String s)` and `void typedScript(int i)` give `FreeFunction` (kind `function`) named under the package (`com.ex.scriptLevel`, f2), or bare when there is no package (`helperFn`, f11 `build.gradle`). Test `a_script_level_method_without_a_package_is_captured` (groovy.rs:380).
- **Annotations on a method**: recorded as attribute strings by `attrs::collect_with_preceding` (groovy.rs:192). Fixture f9: `@TaskAction`, `@FunctionName("hello")`, `@Test`.
- **Base types**: the enclosing class's `extends`/`implements` are copied onto every method's `DefRecord::base_types` (groovy.rs:89-101, 176-178), and are read by framework `base_types` rules (§10).

Not supported, or partial:

- **Constructors** (`constructor_declaration`) are not extracted: there is no arm for the kind. Fixture f2: `Plain() { init(); }` and `Plain(int x) {…}` produce no node, and their `init()` calls are `no-enclosing-callable` (f2 unresolved lines 4 and 5).
  - In f1, `Circle(double r) { this.r = r }` appeared as `com.ex.Circle.Circle` only because error recovery turned it into a `method_declaration`.
- **Closures** (`closure` node) are never callables: the plugin's `closure_expression` arm names a nonexistent kind (groovy.rs:129-133). Calls inside a closure are attributed to the enclosing method (test `a_closure_body_still_yields_its_calls`, groovy.rs:367). A closure-valued field `def lam = {…}` is no node (f1, f2).
- **Script top level and `.gradle` bodies**: there is no synthetic script/module callable. Every top-level statement's call is `no-enclosing-callable`:
  - f1 `scriptFn(1)`;
  - f11 `build.gradle` `doLast { helperFn() }` and `tasks.register("x") { helperFn() }`.
- **Spock feature methods with string names** (`def "it works"() { … }`) are not extracted: f9 has no node, and `compute()` inside is `no-enclosing-callable`. Rules.rs:5712 says the same.
- **Traits** (`trait T { … }`) are not extracted: the grammar has no `trait` (f2).
- **Records**: `record Point(int x, int y) { int sum() … }` does not push a scope, so the method is named `com.ex.sum` (f2). `record_declaration` is not in the scope arm (groovy.rs:84).
- **Annotation types** (`@interface Marker {}`): `annotation_type_declaration` has no scope arm, and members would be named under the outer scope (f2 shows no members, so unverified for elements).
- **Anonymous class bodies**: `new Runnable() { void run() {…} }` methods are named under the enclosing class (`com.ex.Plain.run`, f2). `object_creation_expression` pushes no scope.
- **Properties and fields** (`Dog pet`, `static int count`) are not callables, and no `class_fields` are recorded (f7 §05).
- **No `Constructor` def per class**: unlike Kotlin, the plugin does not mint one, so `new Foo()` has no callee node (§03).
- **No `FileFacts::classes` records**: the plugin never writes a `ClassDecl` (no `classes` push in groovy.rs). The class hierarchy comes only from methods' `base_types`, so a class with no methods contributes no bases.
- **Class-level annotations** are not recorded anywhere: there is no class DefRecord.
- **Overloads**: same-file definitions with one qualified name are extracted separately (f1: two `com.ex.Service.process`, two `com.ex.Service.overloaded`). They are treated as one callee (§06).

## 03. Call-site forms

The plugin emits a `RefRecord` only from `method_invocation` (groovy.rs:119-123, 231-263). It sets:

- `name`: the `name` field;
- `receiver_hint`: the peeled `object` field text, or its cast/construction type;
- `typed_receiver`: true when `peel` returned a type.

It never sets `arity`, `value_ref`, `paren_less`, `path_receiver`, `invocant`, `implicit_receiver`, `arrow`, `arg_type` or `weak_receiver_type` (no such field in groovy.rs).

Supported shapes:

- **Bare call** `helper(1)`: receiver empty. Fixture f3 line 12 gives a high intra-file edge. Test `method_invocation_captured` (groovy.rs:318).
- **Method call on a variable** `p.t()`: receiver `p` (f3 L14, high via the parameter type).
- **Safe navigation** `p?.t()` and **spread** `p*.t()`: both parse as `method_invocation` with the object `p` (f3 L16, L17 high). In f1 L59 and L60, `c?.area()` and `c*.area()` fanned out medium, because an `ERROR` lost the parameter typing there.
- **Static or qualified call** `Target.st()` and `Helper.sstat()`: receiver is the class name (f3 L22 high; f1 L46 high cross-file).
- **Chained call** `p.chain().t()`: both sites are recorded. The inner one binds high. The outer one's receiver is the call text, so it binds by name at medium (f3 L15, L24). Return types are not propagated (§05).
- **Construction receiver** `new Target().t()`: `peel` maps `object_creation_expression` to its type (receiver.rs `shape_type`), so it is a typed receiver and binds high (f3 L23, f7 L20).
- **Cast receiver** `((Dog) x).speak()`: `cast_expression` gives a typed receiver, bound high (f7 L18).
- **Explicit `this.`** `this.helper(7)`: binds high intra-file (f3 L30).
- **Named arguments** `helper(name: "x")`: f3 L37, high.
- **Call inside a closure argument** `[1,2].each { helper(it) }` and `p.cb { helper(5) }`: the inner call is recorded and attributed to the enclosing method (f3 L25, L27).
- **Call inside a `with {}` closure** `p.with { t() }`: the bare `t()` is recorded with no receiver. It bound high to `Target.t` only because that name is unique in the file (f3 L18). Delegate typing does not happen (cross_file.rs:4084: "Groovy (closure delegates — the Gradle DSL, Spock)" left unchanged).

Not supported, or partial:

- **Paren-less (command) calls** `println "y"`, `a "x"`, `process "paren-less"` and `helper 2;` are **silently dropped**: no edge and no unresolved record (f4, f3 L13, f1 L49).
  - The grammar produces `juxt_function_call` or misparses the statement, and the plugin has no arm for it.
  - This is most Gradle DSL (`implementation 'a:b'`, `id 'java'`) and much Spock and Jenkins code.
- **`function_call` arm is dead**: groovy.rs:124-128 matches a kind the grammar does not have.
- **Method pointer** `this.&helper` and **method reference** `Target::st`: not recorded, and no value refs (f3 L19, L20, L26).
- **Calling a closure variable**: `ptr(3)`, `l(1)` and `lam(3)` are recorded as bare calls, unresolved and rendered as `<external>::ptr` / `<external>::l` (f3 L21, L34; f1 L53, L54). `l.call(1)` is `<external>::l::call`.
- **Dynamic method name** `p."t"()` gives no site: it parses as `template_expression` (f3 L35). `"${helper(6)}"` GString interpolation gave no site in f3 L28 (misparse).
- **`Outer.this.m()`**: receiver `Calls.this` gives unresolved `candidates-in-other-files` (f3 L31).
- **Constructor calls** `new Foo(…)` are not call sites to a constructor: no `Constructor` callable exists (§02), and `object_creation_expression` is not an arm.
- **Operator calls** (`a + b` to `plus`, `a[i]` to `getAt`, `obj()` to `call`) are not recorded. `call_operator_names` covers only python, php and ruby (cross_file.rs:394-399).
- **Explicit generic type arguments**: no special handling. The grammar's `method_invocation` keeps `name`, and CHANGELOG.md:913 says Groovy "already resolved these forms". Not tested here: unverified.
- **Arity**: not recorded, so there is no overload narrowing (§06).
- **Registrar arguments**: none, because `_ctx` is ignored (groovy.rs:24).
- **Reference sites**: no `value_ref` sites are recorded even under `--reference-edges` or `--dead-code`.

## 04. Imports & module resolution

`record_import` (groovy.rs:197-229) takes the first `scoped_identifier` or `identifier` child of `import_declaration` as `path`. It sets `alias` to the last dotted segment of that path, and `kind` to `from-import` when the text contains `static`, or `import` otherwise.

Supported shapes:

- **Single-type import** `import a.Lib`: binds `Lib` in `direct_imports` through the shared JVM import arm `"scala" | "java" | "kotlin" | "groovy"` (cross_file.rs:1982-2010). In f5, `Lib.ls()` and `new Lib().lm()` bind high to `a.Lib.*`. Test `a_plain_import_is_recorded` (groovy.rs:341).
- **Static member import** `import static a.Lib.ls`: a `from-import` with alias `ls`. The bare `ls()` binds high (f5 L11, f1 L45). Test `a_static_import_is_a_from_import` (groovy.rs:353).
- **Import anchoring of certainty**: `OwnerFacts` for java, kotlin and groovy requires every candidate's owner path to be imported, or the file to have no imports (cross_file.rs:4623-4626).
- **Owner path narrowing** by the written qualifier (`Response.Builder`) and by nesting in the caller's scope: `narrow_owner_paths` includes groovy (cross_file.rs:4847). CHANGELOG.md:187 states the same. Not separately fixture-tested.
- **Qualified names carry the package**: `qn_carries_module` includes groovy (cross_file.rs:4775), and so does `unique_owner_names` (cross_file.rs:4411).

Not supported, or partial:

- **Aliased import** `import a.Lib2 as Aliased`: the recorded alias is `Lib2`, not `Aliased` (groovy.rs:220). `Aliased.l2()` is unresolved `candidates-in-other-files` (f5 L13; f1 L47, `Renamed.go`).
- **Wildcard import** `import c.*`: the grammar makes the `*` a sibling `asterisk` node, and the walker takes only the identifier child, so the path recorded is `c`. Evidence: the tsdump of f5 `Wild.groovy` shows `import_declaration > identifier "c", ".", asterisk`. The `* → unqualified_prefixes` branch (cross_file.rs:1988) is therefore never reached.
  - Fixture f5 `Wild.groovy` with `import c.*`: `Lib.ls()` fans out medium to both `a.Lib.ls` and `c.Lib.ls`.
- **Static wildcard** `import static a.Lib2.*`: the bare `l2()` binds medium by name (f5 L12), not through the import.
- **Groovy default imports** (`java.lang.*`, `java.util.*`, `java.io.*`, `java.net.*`, `groovy.lang.*`, `groovy.util.*`, `java.math.BigInteger/BigDecimal`) are not modeled. Only the stdlib name table helps (§13).
- **No `FileFacts::packages`**: the plugin never writes it. The package exists only as the qualified-name prefix.
- **No manifest reading** (`build.gradle` dependencies, `settings.gradle`): `manifest.rs` is not used by groovy.rs.
- **No external-module refusal**: `heads.rs` `external_head` is limited to python, javascript, typescript, go, java, kotlin, scala, csharp and cpp (heads.rs:661-671).
  - A call on an imported external class is not refused as `external-module`. It goes through name matching instead.
  - Fixture f6: `def o = new Other(); o.go()`, with `Other` imported from `org.external`, bound **medium to the unrelated workspace `p.A.go`**.
  - Capitalised external receivers (`Thing.go()`, `Other.run()`) end as `candidates-in-other-files` rather than `external-module`. In the audit's unresolved-by-module table they are attributed to the import (`org.external.Thing 1 ['go']`, f5).
- **No `not-in-imported-module`**: `modules.rs` and `ModuleIndex` have no groovy case (grep finds no `groovy` in modules.rs).
- **No language family**: the JVM family row (`java | kotlin | scala | groovy`) is commented out (cgg-core/src/lang_family.rs:32). Groovy never links to Java, Kotlin or Scala definitions, and a Groovy call on a Java class in the same repository finds nothing.

## 05. Type inference & receiver typing

Supported:

- **Parameter types** from the signature text: `collect_param_types` (type_hints.rs:1414) reads `Type name` parameters from `signature_hint`, which `extract_signature` fills (groovy.rs:186).
  - f7: `void t(Cat c) { c.speak() }` binds high.
  - f8: `use(Base b, Iface i, Kid k)` gives high edges.
  - f6: a parameter typed with an external class rewrites the receiver to `Thing` and refuses rather than fanning out.
- **Constructions** `new Dog().speak()` are typed receivers (receiver.rs `object_creation_expression`, CHANGELOG.md:936 lists Groovy). f7 L20 is high.
- **Casts** `((Dog) x).speak()`: `cast_expression` makes a typed receiver (f7 L18 high).
- **Parenthesised receivers** are peeled (receiver.rs:60).

Not supported, or partial:

- **Typed locals** `Dog d1 = new Dog()`, `def d2 = new Dog()`, `var d3 = new Dog()`, `final Cat c2 = c`: none are typed, and calls fan out by name at medium (f7 L12-L22; f5 L18; f1 L39, L41).
  - The plugin writes no `local_types` and no `locals`.
  - `locals.rs` has no groovy spec (`spec()`, locals.rs:380-393).
- **Field types** (`Dog pet; pet.speak()`, `this.pet.speak()`): untyped, medium fan-out (f7 L9, L10; f1 L37). No `class_fields` are recorded.
- **Return types** `make().speak()`: medium fan-out (f7 L17). Chained calls in f3 behave the same.
- **`as` casts** `(x as Dog).speak()`: the receiver is lost entirely. The site is unresolved `ambiguous-in-file` with no receiver hint (f7 L19). In f1 L44, `(d as Helper).help()` was medium.
- **Wrappers and containers**: `Optional<Cat> oc` reads as receiver `Optional`, and `List<Dog>` as `List` (f7). Both are containers by design, and no `MAYBE_TYPE_WRAPPERS` entry exists for groovy (stdlib.rs:104-121). The Groovy-only nullability spelling does not exist (`Cat? nc` is not Groovy syntax).
- **Closure parameter typing** (`scoped_types`, `settle_closure_args`): none. `closure_types.rs` has no groovy case. A typed closure parameter `{ Dog d -> d.speak() }` is not used (unverified by fixture, but no code path exists).
- **Implicit and delegate receivers** (`with {}`, `tap {}`, Gradle DSL, Spock `then:`): none. There is no `implicit_receiver` (cross_file.rs:4084 comment).
- **Weak receiver types**: never set.
- **Groovy is absent from `lang_tallies_table!`** (type_hints.rs:342-360), so no per-language typing tallies are emitted.

## 06. Inheritance, dispatch & overloading

Supported:

- **Base types**: the class's `extends`/`implements` text is recorded on each method's `base_types` (groovy.rs:89, 176) and read by `ClassHierarchy` / `bases_by_owner`. Comment groovy.rs:46-51 says "Recorded from 0.6.8".
- **Inherited member through a typed receiver**: `k.inherited()` with `Kid k` binds high to `x.Base.inherited` through the base chain (f8 L24).
- **`super.m()`**: `super.over()` binds `x.Base.over` at **medium** (f8 L5; f1 L23). It is never high, because `super_owner_may_be_anonymous` includes groovy (cross_file.rs:4694-4699; CHANGELOG.md:57).
- **Inheritance fan-out** (`dispatch::inheritance_fanout`): groovy is in `INHERITANCE_LANGS` (dispatch.rs:116). It runs with `--dynamic-dispatch` and under `--dead-code`.
  - f8 `--dynamic-dispatch`: `x.Base.over → y.Kid.over`, `→ y.GrandKid.over`, `y.Kid.over → y.GrandKid.over`, `x.Base.abs → y.Kid.abs`, `x.Iface.im → y.Kid.im`, all `low dynamic dispatch:inheritance`.
  - f1 shows the default-run equivalent appearing under `--dead-code`.
- **Overload sets**: groovy is in `binds_same_name_set` (intra_file.rs:835-855), so two same-file `process` overloads are one callee. `one_method` keeps them out of the fan-out cap, and they bind every overload at medium (f1 L50, L51; f3). CHANGELOG.md:262 says "Swift and Groovy (+132 each)".

Not supported, or partial:

- **No arity narrowing**: `OVERLOAD_ARITY_LANGS = ["java", "scala", "cpp"]` (type_hints.rs:1583), and the plugin sets no `RefRecord::arity`. `process("a")` binds both `process(String)` and `process(String,String)` at medium (f1 L50, L51).
- **Bare calls are not scoped to the class hierarchy**. `bare_call_reaches` leaves Groovy "unchanged" (cross_file.rs:4084-4085), and `member_of_calling_class` runs only for java, csharp and ruby (cross_file.rs:5650). Fixture f8:
  - `inherited()` inside `Kid` fans out to `x.Base.inherited` **and** the unrelated `x.Unrelated.inherited`, both medium;
  - `other()` binds medium to `x.Unrelated.other`, a class unrelated to `Kid`;
  - `over()` inside `Kid`, which declares `over` itself and has a same-file subclass `GrandKid.over`, bound only to `x.Base.over` and not to `Kid.over` (f8 L10).
- **Bare calls to another class's method in the same file bind high**. In f13, `onlyInA()` in `B1.b` binds **high** to `A1.onlyInA`, although `A1` is unrelated. A bare `nested()` with the caller's own `B1.nested` plus `C1.nested` in the file is `ambiguous-in-file`: the caller's own class is not preferred.
- **No `overrides_below`**: overrides below the typed class are not bound by default. That logic is for Java/C# `derived_index` only (cross_file.rs:423-432). `b.over()` with `Base b` binds only `Base.over` at high (f8 L21).
- **No `FileFacts::classes`**, so `CLASS_RECORDS_ARE_EXACT` does not apply (dispatch.rs, java and csharp only). An anonymous class's methods carry the anonymous base on the enclosing class's methods (§02).
- **Traits and mixins** (`trait`, `implements SomeTrait`, `@Mixin`, `withTraits`): not parsed or modelled (§01).
- **Extension methods** (Groovy extension modules, categories `use(Cat) {}`, `metaClass.foo = {}`): not modelled. No code mentions them.
- **`--dynamic-dispatch` trait fan-out** (`dispatch::fanout` through `trait_impl_target`): never set for Groovy, so only `inheritance_fanout` applies.

## 07. Scoping & shadowing

- **`FileFacts::locals`**: not collected. `locals.rs` `spec()` has no groovy entry (locals.rs:380-393), so `shadowed-by-local` never fires.
  - f13: `int onlyInA = 3;` after the call does nothing.
  - A closure variable `l(1)` is not treated as a local (f3 L34, `<external>::l`).
- **Lexical narrowing of nested definitions**: not applicable, because there are no nested function definitions. Closures are not callables. Groovy is not in the lexical-narrowing language list in CLAUDE.md.
- **Positional shadowing**: none.
- **Conditional definitions**: none (`DefRecord::conditional` is never set).
- **Scope of a bare call**: by-name through intra-file then cross-file, with no class scoping. See §06 and f13.

## 08. Macros, preprocessor & codegen

- **Not applicable**: Groovy has no textual macros or preprocessor.
- **AST transformations** are compile-time codegen, and none are modelled. No generated callables are minted, and no `macro:<name>` tag is used. Examples:
  - `@Canonical`, `@TupleConstructor`, `@Builder`, `@Delegate`, `@Immutable`, `@Singleton`, `@Lazy`;
  - Spock's feature-method rewriting;
  - Grails GORM dynamic finders (`Book.findByTitle`).
- **Calls to generated members** (`Book.builder()`, `findAllByX`) find no candidate, or bind by name. Not fixture-tested: unverified beyond the absence of any code path.
- **`name_mentions`**: never written by groovy.rs.

## 09. FFI & cross-language links

- **`ffi::link_ffi`**: no Groovy coverage (no `groovy` in crates/cgg-resolve/src/ffi.rs). JNI is Java-side only.
- **JVM interop**: Groovy calling Java, Kotlin or Scala classes in the same tree is **not linked**. The family row is commented out (lang_family.rs:32), and qualified-name fallback across the JVM family is disabled.
- **Descriptor links** (protobuf rpc → host): no `groovy` in descriptor.rs.
- **Markup code-behind**: not applicable. `markup.rs` is C#-only.

## 10. Frameworks & entry points

Framework rules match `rule.language == f.language` exactly (cgg-resolve/src/frameworks/mod.rs:648, 709). Every rule below declares `language: "groovy"`. No multi-language rule covers groovy, and no Java rule (JUnit, Spring, JAX-RS, Micronaut, …) applies to Groovy files. Fixture f9 exercises the rules.

| Rule id | rules.rs | Kind | Trigger | Node | f9 result |
|---|---|---|---|---|---|
| `aws-lambda` | 4210 | network | detect import `com.amazonaws.services.lambda` / `com.amazonaws.lambda`; `base_types` `RequestHandler`, `RequestStreamHandler`; method `handleRequest` | yes | recognised; minted `<framework-entry>::network::aws-lambda::LambdaH.handleRequest` |
| `azure-functions` | 4360 | network | detect `com.microsoft.azure.functions`; attribute `FunctionName` | yes | minted `<framework-entry>::network::azure-functions::FunctionName("hello")` |
| `gcp-functions` | 4924 | network | detect `com.google.cloud.functions`; `base_types` `HttpFunction`, `BackgroundFunction`, `CloudEventsFunction`, `RawBackgroundFunction`, `TypedFunction`; methods `service`, `accept`, `apply` | yes | not run in f9; `crates/cgg/tests/frameworks.rs:1337` case `gcp-functions/groovy` |
| `gradle-task` | 5665 | cli | detect `org.gradle`; **no matchers** (detection only) | yes | `seen_no_rules` |
| `gradle-plugin` | 5682 | lifecycle | detect `org.gradle`; `base_types` `Plugin`; method `apply` | **no** (root mark) | recognised, 1 entry: `MyPlugin.apply` root `gradle-plugin:base-type` (`Plugin<Project>` matched) |
| `spock` | 5699 | test | detect `spock.lang` / `spock.util` / `spock.mock`; **no matchers** | yes | `seen_no_rules` |
| `grails` | 5716 | network | detect `grails.artefact`, `grails.gorm`, `grails.web`, `grails.plugins`, `grails.boot`, `grails.converters`, `org.grails`; **no matchers** | yes | `seen_no_rules`; `BookController.index` reported dead (medium) |
| `jenkins-pipeline` | 5741 | cli | detect `hudson`, `jenkins.model`, `jenkins.util`, `org.jenkinsci.plugins`, `com.cloudbees.groovy.cps`; **no matchers** | yes | not run; Jenkinsfile is not even detected (f11) |
| `gradle-task-annotations` | 9649 | lifecycle | detect `org.gradle`; attributes `TaskAction`, `Input`, `InputFile`, `InputFiles`, `InputDirectory`, `OutputFile`, `OutputFiles`, `OutputDirectory`, `OutputDirectories`, `Nested`, `Classpath`, `Mutate`, `Model`, `Defaults`, `Finalize`, `Validate` | **no** (root mark) | recognised; `MyTask.doIt` root `gradle-task-annotations:attribute` |

Further evidence:

- `crates/cgg/tests/frameworks.rs:1198` (`aws-lambda/groovy`), `:1319` (`azure-functions/groovy`) and `:1337` (`gcp-functions/groovy`) are integration cases.
- `scripts/benchmark.sh:218` lists the groovy-gradle APPS expectations: `gradle-task`, `gradle-task-annotations`, `gradle-plugin`, `~spock`, and others.

Helper tables: groovy appears in **none** of them. A grep for `groovy` in rules.rs finds only `language:` lines and gap prose. The tables are:

- `member_sources_for`;
- `signature_methods_for`;
- `container_attributes_for`;
- `markup_kinds_for`;
- `TEST_CLASS_MEMBERS_ARE_ROOTS` (python, php and ruby only; rules.rs:11806);
- `RESPONSE_TRAITS` and `SELF_CONVERTING_WRAPPERS`.

Visibility entries and registrar verbs: none. The plugin records no visibility and no registrar arguments.

**Stale gap prose**:

- The `gradle-task` (rules.rs:5678), `spock` (rules.rs:5712) and `grails` (rules.rs:5737) gap strings say the Groovy plugin "records neither annotations nor base types". Both have been recorded since 0.6.8 (groovy.rs:46-51, 188-192; CHANGELOG.md:3873-3876), and f9 proves it: `@TaskAction` and `Plugin<Project>` matched.
- The `gradle-task` gap also says the plugin "deliberately skips `closure_expression`". In fact that kind does not exist in the grammar; `closure` nodes are walked but never made callables.

Not supported:

- Grails controllers, interceptors, TagLibs and GORM hooks;
- Spock feature methods and fixtures;
- Jenkins declarative and scripted pipelines and shared-library `vars/*.groovy` `call`;
- Gradle closures (`doLast`, `doFirst`, `task x {}`, `tasks.register`);
- Spring Boot (`@RestController`, `@Component`), Micronaut and Ratpack in Groovy files (no groovy rules);
- `static void main`. It is rooted only by the generic `builtin:main` dead-code root (§12). It is not a framework entry.

## 11. Test harness entries

- **No harness rule of kind `test` enumerates anything for Groovy.** The only Groovy test rule is `spock` (rules.rs:5699). It has no attributes, methods or base types, so it mints no `<framework-entry>::test::…` node and only reports `seen_no_rules` (f9).
- **JUnit or TestNG in Groovy**: no groovy rule, so no harness node.
  - In f9, `@Test void testBar()` in `src/test/groovy/BarTest.groovy` was rooted by the generic dead-code root `builtin:test-attr` (roots.rs:211-224, `TEST_ATTRS` contains `Test`). It is a root mark only, not an entry node.
- **Naming patterns**: the dead-code `is_test_name` default arm (`test_*` or `Test*`, roots.rs:975-988) applies. Groovy's usual `testSomething` (lower-case `test` with no underscore) is **not** matched: f10 `App.testSomething` was reported `never-referenced`.
- **Spock**: feature methods are not extracted (§02). In f9, calls inside them become `toplevel:invocation` roots for the callee (`FooSpec.compute`, detail "called from module top level"), which is incidental. The `setup`, `cleanup`, `setupSpec` and `cleanupSpec` fixtures are not rooted (f9 `FooSpec.setup` has no root).
- **`TEST_CLASS_MEMBERS_ARE_ROOTS`**: groovy is not listed (rules.rs:11806).
- **Test-file gating**: by the `src/test/` layout or `TEST_DIRS` only (§01). f9's `BarTest.barHelper` in `src/test` landed in the withheld `only-used-by-tests` category (`reason: missing-signal`).
- **`--no-test-entries`**: no effect, because no harness nodes exist.

## 12. Dead-code analysis

Capability (fixture f9 `capabilities` JSON):

| Field | Value |
|---|---|
| `class` | `degraded` |
| `visibility` | none |
| `attributes` | full only when some callable carries one (observation upgrade, caps.rs:113-160); f1 showed `attrs no` |
| `value_references` | none |
| `dispatch` | none |
| `exports` | none |
| `test_tagging` | none |
| `max_confidence` | **medium** |

- The class is `Degraded` because no Groovy callable ever has a visibility (caps.rs:167-172; groovy.rs:187). The plugin declares no `PluginSignals` (no `signals()` override; default all-false, lib.rs:304-306).
- **Consequence**: Groovy dead-code findings are never `high`. Every finding carries the evidence `language-lacks-dispatch-model`, `language-lacks-value-references` and `language-lacks-visibility` (f9, f10).

Root kinds observed for Groovy:

- `builtin:main`: `static void main` (f10 `App.main`; roots.rs:20, 741-757).
- `builtin:test-attr`: `@Test` (f9).
- `framework:rule`: `gradle-task-annotations:attribute` and `gradle-plugin:base-type` (f9).
- `framework:entry-node`: aws-lambda and azure (f9).
- `toplevel:invocation`: a callee called from a site with no enclosing callable (script top level, `.gradle` bodies, Spock features; f9, f11).

Confidence caps:

- **`implements-external-contract`**: works through the recorded `@Override` annotation (`OVERRIDE_MARKERS`, deadcode/mod.rs:409, 498-530). In f10, `@Override void hook()` in `App extends org.ext.Base` carries it.
- **`reflection`**: never. There is no dynuse (§15). In f10, `reflected()`, called only through `invokeMethod("reflected")` and `this."reflected"()`, is reported `never-referenced` with no doubt evidence.
- **`macro-generated`**: not applicable.

Other points:

- **Inheritance fan-out under `--dead-code`**: runs, since groovy is in `INHERITANCE_LANGS`. An override reached through a base declaration in the tree is live (f1 dead-code run, `Base.area → Circle.area` low edges).
- **Public or exported roots**: none. Groovy members are public by default, but with no visibility recorded nothing is rooted as public API. Library code is therefore reported dead at medium.
- **Unresolved-doubt handling**: the generic same-name unresolved-site check applies (`no-unresolved-site-with-name` evidence), but paren-less calls leave no unresolved site at all (§03). A method called only paren-less (`log "x"`) looks fully unreferenced.
- **No `BLIND_SPOTS` entry** for groovy (caps.rs:44).
- **Root coverage**: low in typical code. f1 reached 9%, under the 25% threshold, so findings were withheld as "root coverage too low".

## 13. Builtins & stdlib tables

- **`crates/cgg-core/src/stdlib/groovy.txt`**: 339 names, loaded at stdlib.rs:205 and 246. Examples: `println`, `each`, `collect`, `find`, `with`, `tap`, `apply`, `sleep`, `Math`, `ArrayList`, `BigDecimal`, `asType`, `assert`.
- **`--include-stdlib`** (f12) mints `<stdlib>` leaf nodes:
  - `println("a")` → `<stdlib>::println`;
  - `xs.each {…}` → `<stdlib>::List::each`, with the receiver typed `List` from the parameter;
  - `xs.collect` → `<stdlib>::List::collect`;
  - `Math.max` → `<stdlib>::Math::max`;
  - `sleep` → `<stdlib>::sleep`.
- **`--include-external`** (f12, f3, f1): `"s".toUpperCase()`, `[1,2].each` (literal receiver), `p.with`, `p.invokeMethod` and closure-variable calls become `<external>::…` leaves.
- **No Groovy entry** in `CAST_INTRINSICS`, `CONSTRUCTING_METHODS`, `MAYBE_TYPE_WRAPPERS`, `UNION_TYPE_CONSTRUCTORS`, `NULL_TYPE_MEMBERS`, `CALLBACK_TYPE_CONSTRUCTORS`, `TOP_TYPES` or the implicit-package tables. A grep of `crates/cgg-core/src/stdlib.rs` for `groovy` finds only lines 205 and 246.
- **No `rust_types`-like table.** The default imports are not modelled (§04).

## 14. Confidence & certainty

Paths that yielded **high** in the fixtures:

- intra-file unique-name binding (f3 L12, L30);
- a parameter-typed receiver landing on one owner, in the same file or imported (f3 L14; f1 L35; f8 L21-L24);
- construction and cast receivers (f7 L18, L20; f1 L42, L43);
- a static import (f1 L45; f5 L11);
- a qualified static call on an imported class (f1 L46; f5 L10);
- an inherited member through a typed receiver when one ancestor declares it (f8 L24).

`OwnerFacts` for groovy requires every candidate's owner path to be imported, or the file to have no imports (cross_file.rs:4623-4626). `qn_carries_module` anchors exact paths (cross_file.rs:4775).

What forces **medium**:

- `super.m()`: always, through `super_owner_may_be_anonymous` (cross_file.rs:4698);
- bound overload sets (f1 L50-L57);
- by-name fan-out for untyped locals, fields, return values and bare inherited names (f7, f8);
- calls under a wildcard import (f5).

**Low**: inheritance dispatch edges and framework-entry edges.

**False highs observed**:

- f13: a bare call in `B1` binds high to an unrelated `A1.onlyInA` in the same file. Intra-file unique-name binding ignores class scope.
- f3 L18: the bare call in `with {}` was correct only by coincidence.

**Mermaid `?` marker**: generic. An arrow whose sites are all non-high is labelled with a leading `?`.

## 15. Dynamic use & reflection

- **No dynuse coverage**. `REFLECTIVE` in `crates/cgg-lang/src/plugins/dynuse.rs:25-50` lists python, ruby, java, csharp, javascript, typescript and php, and the plugin declares no `dyn_uses` signal.
- **Unhandled reflective forms**:
  - `obj.invokeMethod("m", args)` becomes a plain unresolved or external call (f3 L36; f10).
  - `obj."$name"()` and `obj."m"()` parse as `template_expression` and record nothing (f3 L35).
  - `methodMissing`, `propertyMissing`, `metaClass.m = {…}`, `Eval.me`, `GroovyShell.evaluate`, `this.getClass().getMethod("m")` and `Class.forName` in Groovy are not modelled.
  - `methodMissing` is extracted as an ordinary method (f1) and treated as dead unless called.
- **Module loads** (`DYN_MODULE_LOADERS` = `require`, `importScripts`, `import_module`; facts.rs:165): not applicable.

## 16. Known gaps & limitations

1. **Constructors are never extracted.** The plugin has no `constructor_declaration` arm (groovy.rs:67-137), so code in constructors is unattributed (f2).
2. **Closures are never callables, and the `closure_expression` and `function_call` arms are dead.** Neither kind exists in tree-sitter-groovy 0.1.2's `node-types.json` (groovy.rs:124-133). The `closure` and `juxt_function_call` kinds are unhandled.
3. **Paren-less command calls are silently dropped** (`println "x"`, `implementation '…'`, `sh 'make'`). There is no edge and no unresolved record (f4, f1 L49).
4. **Script, `.gradle`, Jenkinsfile and Spock bodies have no enclosing callable.** Their calls are `no-enclosing-callable`, and Spock string-named features are not extracted (f9, f11; rules.rs:5712, 5760).
5. **Weak grammar.** Semicolon-free statements, untyped parameters, closure fields and `trait` produce `ERROR` trees that drop or misattribute definitions (f1: `Service.init` lost; f2: trait lost). There is no re-parse fallback.
6. **Imports**:
   - `as` aliases are recorded under the original name;
   - wildcard `.*` imports lose the `*`;
   - default imports are not modelled;
   - there is no `FileFacts::packages`;
   - there is no `external-module` refusal (heads.rs:661-671), which produced a wrong medium edge to an unrelated workspace method in f6.
7. **Typing covers parameters, casts and `new` only.** Locals (`def`, `var`, typed), fields, return types and `as` casts are untyped (f7). Groovy has no locals spec (locals.rs:380-393) and no closure or delegate typing.
8. **Bare calls ignore class scope.**
   - Groovy is excluded from `bare_call_reaches` narrowing (cross_file.rs:4084) and from `member_of_calling_class` (cross_file.rs:5650).
   - Bare calls fan out to unrelated classes (f8).
   - Bare calls bind **high** to an unrelated same-file class's method (f13).
   - The caller's own method is not preferred over a same-file subclass copy (f8 L10, f13 L7).
9. **No arity on sites**: Groovy is not in `OVERLOAD_ARITY_LANGS` (type_hints.rs:1583), so every overload binds at medium.
10. **No JVM-family linking.** `lang_family.rs:32` is commented out, so Groovy and Java code in one repository do not see each other.
11. **Unmodelled constructs**:
    - records and annotation types are not scoped (methods named under the package);
    - anonymous class methods are named under the enclosing class;
    - traits are not parsed;
    - AST-transform codegen, GORM dynamic finders and `metaClass` are not modelled.
12. **Dead code is capped at medium** (class `degraded`, no visibility). There is no reflection screen, no harness entries, no `testFoo` name convention, and no Grails, Spock or Jenkins roots.
13. **Detection**:
    - no `.gvy`, `.gy` or `.gsh` extensions;
    - no shebang `groovy`;
    - no `Jenkinsfile` detection;
    - no test-file name rule (`*Spec.groovy`, `*Test.groovy`) outside `src/test/` (testfile.rs:69-119).
14. **Stale documentation**:
    - The README language table says "Closures, methods" (README.md:507), but closures are not callables.
    - It gives Type inference as "—", although parameters, casts and `new` are typed (f7).
    - The gap strings for `gradle-task`, `spock` and `grails` (rules.rs:5678, 5712, 5737) still claim no annotations or base types are recorded.
15. **CHANGELOG follow-up**: "Groovy classes the grammar loses the scope of keep losing calls through a constructed receiver" (CHANGELOG.md:1477).
16. **No ignored tests.** The plugin's 8 inline tests (groovy.rs:304-400) cover only:
    - class methods with a package;
    - bare and receiver calls;
    - plain and static imports;
    - calls in a closure;
    - a script-level `def`;
    - empty and malformed input.

    Nothing tests constructors, overloads, typing or frameworks inside groovy.rs. The framework integration cases are at `crates/cgg/tests/frameworks.rs:1198, 1319, 1337`.

## Summary scorecard

| section | support | note |
|---|---|---|
| 01 Detection & parsing | partial | `.groovy`/`.gradle` only; no shebang, `.gvy`/`.gsh` or Jenkinsfile; weak grammar puts most idiomatic files in `parse_status: error` |
| 02 Callables extracted | partial | methods, static methods and script `def`s; no constructors, closures, traits, Spock features or script body; records and anonymous classes mis-scoped |
| 03 Call-site forms | partial | `method_invocation` only; paren-less calls silently dropped; no method pointers or refs, arity, operators or dynamic names |
| 04 Imports & module resolution | partial | single and static imports work; `as` aliases and wildcards broken; no packages, external-module refusal or JVM family |
| 05 Type inference & receiver typing | minimal | parameters, `new` and `(T)` casts only; no locals, fields, return types, `as`, closure or delegate typing |
| 06 Inheritance, dispatch & overloading | partial | bases recorded; typed inherited lookup and inheritance fan-out work; `super` always medium; bare calls unscoped; no arity narrowing |
| 07 Scoping & shadowing | none | no locals spec; no shadowing; no class scoping of bare calls |
| 08 Macros, preprocessor & codegen | n/a | no macros; AST transforms and GORM codegen not modelled |
| 09 FFI & cross-language links | none | no FFI or descriptor links; Groovy↔Java family disabled |
| 10 Frameworks & entry points | partial | 9 groovy rules; 5 enumerate (aws-lambda, azure-functions, gcp-functions, gradle-plugin, gradle-task-annotations); spock, grails, jenkins and gradle-task are detection only |
| 11 Test harness entries | minimal | no harness nodes; `@Test` via generic root only; Spock unsupported; no `testFoo` convention |
| 12 Dead-code analysis | minimal | class degraded, max medium; `@Override` external-contract works; no visibility, reflection or test tagging |
| 13 Builtins & stdlib tables | partial | `groovy.txt` with 339 names; no cast or constructing tables; default imports not modelled |
| 14 Confidence & certainty | partial | high for typed parameters, casts, `new` and imports; false highs for bare same-file names across classes |
| 15 Dynamic use & reflection | none | no dynuse; `invokeMethod`, `"$name"()` and `methodMissing` invisible |
| 16 Known gaps & limitations | n/a | 16 items above; biggest are constructors, closures, paren-less calls, script bodies and local typing |
