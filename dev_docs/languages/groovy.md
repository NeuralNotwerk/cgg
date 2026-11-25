# groovy

> Revised 2026-11-25 against cgg 0.13.3: rewritten for the vendored dekobon grammar (constructors, traits, records, string-named methods, paren-less command calls, method pointers, arity, locals and call-result typing now work; Spock now enumerates). `file:line` citations not otherwise revised date from the 0.11.0 audit and may have drifted.

The Groovy plugin is `GroovyPlugin` in `crates/cgg-lang/src/plugins/groovy.rs` (954 lines, 13 inline tests), registered at `crates/cgg-lang/src/plugins.rs:150`. It parses `.groovy` and `.gradle` files (groovy.rs:35-37) with the dekobon/tree-sitter-groovy grammar vendored in `crates/cgg-lang/vendor/groovy` (issue #11; provenance, fork commit and refresh steps in `vendor/groovy/PROVENANCE.md`). `crates/cgg-lang/build.rs:55-64` compiles its `parser.c` and `scanner.c`, with the C symbols renamed `tree_sitter_groovy_dekobon*`; the plugin binds `tree_sitter_groovy_dekobon` through `LanguageFn` (groovy.rs:15-26). The old `tree-sitter-groovy` 0.1 crate and the `groovy_rewrite.rs` source-rewrite layer are gone. `crates/cgg-lang/src/plugins/groovy_next.rs` (test-only) pins the shapes the old crate rejected.

The grammar reads Groovy's own forms as nodes of their own: `command_chain` (a call without parentheses, whose `receiver` field is the callee), `closure`, `method_pointer_expression` (`.&`), `method_reference_expression` (`::`), `safe_navigation_expression` (`?.`), `spread_dot_expression` (`*.`), `quoted_identifier` (string method names), `trait_declaration` and slashy strings.

The extractor is a single recursive `GroovyWalker` (`walk`, groovy.rs:145-221). It keeps two stacks:

- `scope`: package and type names, which form the qualified name;
- `bases`: the supertypes of the enclosing type, read with `attrs::base_types`.

The walker records:

- methods, constructors and script-level methods as `DefRecord`s carrying base types and preceding annotations (`record_method`, `record_def`, groovy.rs:235-301);
- imports (`record_import`, groovy.rs:324-358);
- call sites from `method_invocation` (`record_invocation`, groovy.rs:360-391), `command_chain` (`record_command`, groovy.rs:393-403) and `object_creation_expression` (`record_construction`, groovy.rs:303-322), with receivers peeled by `crate::receiver::peel("groovy", …)` and an argument count;
- `this(…)`/`super(…)` constructor delegation (`constructor_delegation`, groovy.rs:464-514);
- call-result locals for type propagation (`record_call_result_locals`, groovy.rs:516-556);
- method pointers and method references as value references when `ExtractCtx::reference_sites` is set (`push_pointer`, groovy.rs:558-584).

Locals come from `locals::collect_groovy` (`crates/cgg-lang/src/locals.rs:1078`). The plugin overrides neither `signals()`, `shebangs()`, `filenames()`, `max_line_bytes()`, `ts_language_for()` nor `resolver_kind`.

Fixtures cited below live under `<scratch>/lang-docs/groovy/` (f1 … f10) and were run with `target/release/cgg` 0.13.3 using `-t json -o out.json`, with `--dead-code`, `--reference-edges`, `--property-edges`, `--include-external` or `--include-stdlib` where stated.

## 01. Detection & parsing

- **Extensions `.groovy` and `.gradle`** (groovy.rs:35-37). Fixture f1: `detected_via: extension:.groovy`; f5: `build.gradle` `extension:.gradle`.
- **Not detected**: `.gvy`, `.gy`, `.gsh`, a bare `Jenkinsfile`, and an extensionless file starting `#!/usr/bin/env groovy`. Fixture f5: each is `file_skipped … unknown-extension`. `.gradle.kts` is Kotlin's.
- **Shebangs and file names**: none (no `shebangs()` or `filenames()` override).
- **Grammar**: one grammar for both extensions. Idiomatic semicolon-free Groovy, untyped parameters, closure-valued fields and `trait` declarations now parse cleanly: every file in f1 (traits, records, `@interface`, constructors, closures, command calls, method pointers, a string-named method) is `parse_status: ok`.
- **Remaining parse failure seen**: a call with explicit type arguments (`this.<String>id("a")`, `Collections.<String>emptyList()`) puts the file in `parse_status: error`, and error recovery reads each such statement as a bodiless method declaration. Fixture f9 mints bogus callables `U.id` (line 4) and `U.emptyList` (line 5), both tagged `abstract`, and records no call.
- **Re-parse or fallback**: none. A file with errors is extracted anyway.
- **Depth skip**: shared by every language (`MAX_TREE_DEPTH` = 4000, `SkipReason::TooDeep`).
- **Minified or long-line handling**: no `max_line_bytes` override; the generic locals rule (no locals in a file with a line over 2,000 bytes) applies.
- **Test-file classification** (`crates/cgg-core/src/testfile.rs`):
  - the Maven/Gradle `src/test/` layout rule shared with Java, Kotlin and Scala (testfile.rs:61);
  - a file-name rule: `*Spec.groovy`, `*Test.groovy`, `*Tests.groovy` (testfile.rs:126-130);
  - the generic `TEST_DIRS` directory rule.

## 02. Callables extracted

Qualified names are built by `qn()` (groovy.rs:137-143): `<package>.<Type>.<Nested…>.<name>`, joined with `.`. The package comes from `package_declaration`'s `name` field. Every kind in `TYPE_DECLARATIONS` (groovy.rs:68-75: class, interface, enum, trait, record, annotation type) pushes a scope. `visibility` is always the empty string (groovy.rs:292).

Supported shapes (fixture f1 unless stated):

- **Instance method** `void init() {}`: `InherentMethod` `com.ex.Circle.init`.
- **Static method**: `StaticMethod`, decided by a `static` keyword token among the declaration's children (groovy.rs:249-258). f1: `a.Lib.ls`.
- **`def` methods with untyped parameters**: `def scriptFn(a)` parses and is extracted.
- **Constructors** (`constructor_declaration`): `Constructor` named after the class (`com.ex.Circle.Circle`, two overloads at lines 30 and 31), reached by `new Circle(…)` and `this(…)` (§03).
- **Traits**: `trait Greeter { String greet() … }` gives `com.ex.Greeter.greet` and `com.ex.Greeter.helper`.
- **Records**: `record Point(int x, int y) { int sum() … }` gives `com.ex.Point.sum`.
- **Annotation types**: `@interface Marker { String value() default 'x' }` gives `com.ex.Marker.value`, tagged `abstract` (no body).
- **Interface and `abstract` methods**: a method with no body (and not `native`) is tagged `abstract` (`attrs::mark_abstract_from`, groovy.rs:268-272). f1: `com.ex.Base.area` carries `abstract`.
- **String-named methods** `def "it works"() { … }`: a method named by the string's content (`com.ex.Circle.it works`), carrying the `string-named` attribute (groovy.rs:65, 262-264), which the `spock` rule reads.
- **Script-level methods**: a method outside every type, directly under `source_file` or a block, is a `FreeFunction` (kind `function`) under the package (`com.ex.scriptFn`, f1) or bare (`helperFn`, f5 `build.gradle`).
- **Inner and nested types**: scoped by the stack, as before.
- **Annotations on a method**: recorded by `attrs::collect_with_preceding` (groovy.rs:295).
- **Base types**: the enclosing type's `extends`/`implements` are copied onto every member's `DefRecord::base_types` (groovy.rs:162, 283).
- **Overloads**: same-file definitions with one qualified name are separate records and one callee (§06).

Not supported, or partial:

- **Closures** are never callables. Calls inside a closure belong to the enclosing method (test `a_closure_body_still_yields_its_calls`). A closure-valued field `def lam = { x -> x * 2 }` is no node, but no longer swallows the members after it (f1: both constructors and `init` follow it and are extracted).
- **Script top level and `.gradle` bodies** have no synthetic callable. Calls there are `no-enclosing-callable` (f1 line 71 `scriptFn(1)`; f5 line 6 `tasks.register("x") { doLast { helperFn() } }`).
- **Anonymous class bodies**: `new Runnable() { void run() { init() } }` is named under the enclosing class (`com.ex.Circle.run` at f1 line 59, a second `run`). `object_creation_expression` pushes no scope.
- **Properties and fields** are not callables, and no `class_fields` or `FileFacts::classes` (`ClassDecl`) records are written; the hierarchy comes only from members' `base_types`, so a type with no members contributes no bases.
- **Class-level annotations** are not recorded: there is no type DefRecord.

## 03. Call-site forms

Every call becomes a `RefRecord` through `push_call` (groovy.rs:405-462), whose site is the method's name, so the links of a multi-line chain get distinct sites. The callee is an `identifier` or one of `MEMBER_ACCESSES` (groovy.rs:79-84: `field_access`, `safe_navigation_expression`, `safe_chain_dot_expression`, `spread_dot_expression`). `arity` is set for every countable call; named arguments, spreads and an argument list holding an `ERROR` leave it unknown (`uncountable`, groovy.rs:586-592), and a trailing closure counts as one argument.

Supported shapes (fixture f1 unless stated):

- **Bare call** `process("a")`: high intra-file (line 38).
- **Receiver call, safe navigation and spread** `c?.area()`, `c*.area()` with `Circle c`: high (lines 41-42).
- **Static or qualified call** `Lib.ls()` and a static import `ls()`: high cross-file (lines 43, 45).
- **Paren-less command calls** (`command_chain`): `log "x"` binds high to `Circle.log` (line 55); `println "hello"` reaches `<stdlib>::println` under `--include-stdlib` (line 54). A chained command split by the grammar into statements (`module('a') tries('1')`) records the later one as a `dot_call` (`continues_a_command_chain`, groovy.rs:105-121; test `continued_chains_are_method_calls`).
- **Constructions** `new Circle()`: a typed call of the constructor `Circle` (`record_construction`), high to `com.ex.Circle.Circle` (lines 49, 51). An anonymous class body still constructs its base.
- **Constructor delegation** `this(1.0)`: binds high to the other `Circle` constructor (line 31), never the one it is written in (`not_def`). `super(…)` is recorded with receiver `super` and the superclass's name and resolves through the base chain. Elsewhere `this(…)` is skipped.
- **Construction and cast receivers** `new Lib().lm()`, `(b as Circle).init()`: typed, high (lines 44, 53). `as` casts now type the receiver.
- **Dynamic member names and GString interpolation**: `p."getName"()` and `"${helper(6)}"` both bind high (f8 lines 7, 8).
- **Method pointers and references** (`this.&init`, `Circle::init`; also `Type.&m`, `obj.&m`): `value_ref` records under `--reference-edges` or `--dead-code`, bound high as `reference` edges (f1 lines 56, 57). A pointer through an untyped receiver never fans out by name (`cross_file.rs:7823`).
- **Calls inside closures** (`[1,2].each { helper(it) }`, `doLast { helperFn() }`) are recorded and attributed to the enclosing callable.
- **Property reads** under `--property-edges`: `p.name` binds high to `Person.getName` through the parameter type (f8 line 6; `cgg_resolve::property_edges`, property_edges.rs:90, reads `getX()`/`isX()` with no parameters as getters).

Not supported, or partial:

- **Calling a closure variable**: `lam(3)` with `lam` a field is `<external>::lam` under `--include-external` (f1 line 58). A local closure `def helper = { 1 }; helper()` is `shadowed-by-local` (f7 line 11).
- **Explicit type arguments** (`obj.<T>m()`) break the parse and record no call (§01, f9).
- **Operator calls** (`a + b` to `plus`, `a[i]` to `getAt`, `obj()` to `call`) are not recorded.
- **Registrar arguments**: none.

## 04. Imports & module resolution

`record_import` (groovy.rs:324-358) reads the `name` and `alias` fields: `import a.b.C` → (`import`, `a.b.C`, `C`); `import static a.b.U.m` → (`from-import`, `a.b.U.m`, `m`); `import a.b.C as D` → alias `D`; `import static a.b.U.*` → (`from-import`, `a.b.U`, `*`); `import x.y.*` → (`import`, `x.y`, `y`) (test `imports_plain_static_aliased_and_wildcard`).

Supported shapes:

- **Single-type import** `import a.Lib`: binds through the shared JVM arm `"scala" | "java" | "kotlin" | "groovy"` (cross_file.rs:2105). f1: `Lib.ls()` and `new Lib().lm()` high.
- **Static member import** `import static a.Lib.ls`: the bare `ls()` binds high (f1 line 45).
- **Import anchoring of certainty**: `OwnerFacts` for java, kotlin and groovy requires every candidate's owner path to be imported, or the file to have no imports (cross_file.rs:5151).
- **Owner path narrowing** by the written qualifier and by nesting (`narrow_owner_paths`, cross_file.rs:5491) and `qn_carries_module` (cross_file.rs:5303) include groovy.

Not supported, or partial:

- **Aliased import** `import a.Lib2 as Aliased`: the plugin records the alias, but the JVM import arm keys `direct_imports` by the path's last segment and never reads `ImportRecord::alias`, so `Aliased.l2()` is `candidates-in-other-files` (f1 line 46).
- **Wildcard import** `import c.*`: recorded as path `c` with no `*`, so the arm reads it as an import of a class `c` and never reaches its `*` branch. `Lib.ls()` fans out medium to both `a.Lib.ls` and `c.Lib.ls` (f4 line 7).
- **Static wildcard** `import static a.Lib2.*`: the bare `l2()` binds medium by name (f4 line 8), not through the import.
- **Groovy default imports** (`java.lang.*`, `java.util.*`, `groovy.lang.*`, …) are not modelled.
- **No `FileFacts::packages`**: the package exists only as the qualified-name prefix.
- **No external-module refusal**: groovy is not in `heads.rs`. A call on an external class goes through name matching: f1 `def o = new Other(); o.go()` with `Other` imported from `org.external` binds **medium to the unrelated workspace `p.A.go`** (line 48); f4 `Thing t = new Thing(); t.ls()` fans out to both `Lib.ls` (line 11).
- **No `not-in-imported-module`** (no groovy case in modules.rs).
- **No language family**: the JVM family row is commented out (cgg-core/src/lang_family.rs:32), so Groovy never links to Java, Kotlin or Scala definitions.

## 05. Type inference & receiver typing

Supported:

- **Parameter types** from the signature: `run(Circle c, Base b)` types `c` and `b` (f1 lines 41, 60, high).
- **Constructions and casts** (`new T()`, `(T) x`, `x as T`) are typed receivers (f1 lines 44, 53).
- **Declared return types**: groovy is in `TYPE_FIRST_RETURN_LANGS` (type_hints.rs:4005-4006), so `make().speak()` with `static Dog make()` binds **high** (f2 line 20).
- **Call-result locals**: `def d4 = make()` is recorded as the local type `make()` (`record_call_result_locals`) and retyped from `make`'s declared return type, but as a weak type (type_hints.rs:1066), so `d4.speak()` binds `Dog.speak` at **medium** (f2 line 19).
- **Untyped fields holding a call result** (`def untypedField = make()`): typed the same way, weak, wherever no local or parameter of the name is in scope (f2 line 23, medium).

Not supported, or partial:

- **Typed and constructed locals** `Dog d1 = new Dog()`, `def d2 = new Dog()`, `var d3 = new Dog()`: not typed; `d1.speak()` fans out medium to `Dog.speak` and `Cat.speak` (f2 lines 13, 15, 17). With a single same-named method in the tree the site is refused as `candidates-in-other-files` instead (f3 line 5; Java does the same, f3j).
- **Typed fields** (`Dog pet; pet.speak()`, `this.pet.speak()`): medium fan-out (f2 lines 21, 22).
- **Typed closure parameters** `{ Dog d -> d.speak() }`: medium fan-out (f2 line 24); `closure_types.rs` has no groovy case.
- **Implicit and delegate receivers** (`with {}`, `tap {}`, Gradle DSL, Spock blocks): none.

## 06. Inheritance, dispatch & overloading

Supported:

- **Base types** on each member's `base_types` (§02), read by the class hierarchy.
- **Inherited member through a typed receiver** binds through the base chain.
- **`super.m()`**: `super.over()` binds `com.ex.Base.over` at **medium** (f1 line 34); never high, through `super_owner_may_be_anonymous` (cross_file.rs:5225-5227).
- **Inheritance fan-out** (`dispatch::inheritance_fanout`): groovy is in `INHERITANCE_LANGS` (dispatch.rs:148); it runs with `--dynamic-dispatch` and under `--dead-code`.
- **Arity narrowing**: groovy is in `OVERLOAD_ARITY_LANGS` (type_hints.rs:2357-2366), so `process("a")` and `process("a", "b")` each bind their own overload **high** (f1 lines 38, 39). The whole same-file overload set is read even when the count is unknown (cross_file.rs:2830).
- **Same-named owners**: groovy is in `SAME_NAME_OWNER_CAP_LANGS` (cross_file.rs:5576-5584) and `RECEIVER_METHOD_LANGS` (cross_file.rs:5560-5574).

Not supported, or partial:

- **Bare calls are not scoped to the class hierarchy**: `inherited()` inside `Circle extends Base` fans out medium to `com.ex.Base.inherited` **and** the unrelated `com.ex.Unrelated.inherited` (f1 line 40).
- **Bare calls to another class's method in the same file bind high**: in f7, `onlyInA()` in `B1.b` binds **high** to the unrelated `A1.onlyInA` (line 8); a bare `nested()` with the caller's own `B1.nested` and `A1.nested` in the file is `ambiguous-in-file` (line 9): the caller's own class is not preferred.
- **No `overrides_below`**: `b.over()` with `Base b` binds only `Base.over`, high (f1 line 60), not the override `Circle.over`.
- **No `FileFacts::classes`**, so `CLASS_RECORDS_ARE_EXACT` does not apply.
- **Trait composition semantics**, extension modules, categories (`use(Cat) {}`) and `metaClass` are not modelled; traits are scopes like classes.

## 07. Scoping & shadowing

- **`FileFacts::locals`** from `locals::collect_groovy` (locals.rs:1078): method, constructor and script-function parameters (untyped too), closure parameters and a closure's implicit `it`, Groovy 3 lambda parameters, `def`/typed locals from the end of the declaration, classic and `for … in` loop variables, `catch`, try-with-resources and `def (a, b) =`, and a script's own variables in its body and closures. A declaration holding its own `ERROR` binds nothing.
- **Shadowing**: a bare call through a local or parameter is `shadowed-by-local` (f7: `def helper = { 1 }; helper()` at line 11 and the parameter `helper` at line 13, with `Z.helper` in the file).
- **Lexical narrowing of nested definitions**: not applicable; closures are not callables.
- **Conditional definitions**: none.
- **Class scope of a bare call**: none (§06, f7).

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

Framework rules match `rule.language == f.language` exactly (cgg-resolve/src/frameworks/mod.rs:720, 781). Every rule below declares `language: "groovy"`. No multi-language rule covers groovy, and no Java rule (JUnit, Spring, JAX-RS, Micronaut, …) applies to Groovy files. Fixture f9 exercises the rules.

| Rule id | rules.rs | Kind | Trigger | Node | result (f9 = 0.11.0 audit) |
|---|---|---|---|---|---|
| `aws-lambda` | 4529 | network | detect import `com.amazonaws.services.lambda` / `com.amazonaws.lambda`; `base_types` `RequestHandler`, `RequestStreamHandler`; method `handleRequest` | yes | recognised; minted `<framework-entry>::network::aws-lambda::LambdaH.handleRequest` |
| `azure-functions` | 4679 | network | detect `com.microsoft.azure.functions`; attribute `FunctionName` | yes | minted `<framework-entry>::network::azure-functions::FunctionName("hello")` |
| `gcp-functions` | 5245 | network | detect `com.google.cloud.functions`; `base_types` `HttpFunction`, `BackgroundFunction`, `CloudEventsFunction`, `RawBackgroundFunction`, `TypedFunction`; methods `service`, `accept`, `apply` | yes | not run in f9; `crates/cgg/tests/frameworks.rs:1337` case `gcp-functions/groovy` |
| `gradle-task` | 5986 | cli | detect `org.gradle`; **no matchers** (detection only) | yes | `seen_no_rules` |
| `gradle-plugin` | 6003 | lifecycle | detect `org.gradle`; `base_types` `Plugin`; method `apply` | **no** (root mark) | recognised, 1 entry: `MyPlugin.apply` root `gradle-plugin:base-type` (`Plugin<Project>` matched) |
| `spock` | 6020 | test | detect `spock.lang` / `spock.util` / `spock.mock`; attribute `string-named`; `base_types` `Specification`; methods `setup`, `cleanup`, `setupSpec`, `cleanupSpec` | yes | enumerates (f6, 0.13.3): `<framework-entry>::test::spock` → `FooSpec.setup`, `FooSpec.it computes` |
| `grails` | 6040 | network | detect `grails.artefact`, `grails.gorm`, `grails.web`, `grails.plugins`, `grails.boot`, `grails.converters`, `org.grails`; **no matchers** | yes | `seen_no_rules`; `BookController.index` reported dead (medium) |
| `jenkins-pipeline` | 6065 | cli | detect `hudson`, `jenkins.model`, `jenkins.util`, `org.jenkinsci.plugins`, `com.cloudbees.groovy.cps`; **no matchers** | yes | not run; Jenkinsfile is not even detected (f5) |
| `gradle-task-annotations` | 10692 | lifecycle | detect `org.gradle`; attributes `TaskAction`, `Input`, `InputFile`, `InputFiles`, `InputDirectory`, `OutputFile`, `OutputFiles`, `OutputDirectory`, `OutputDirectories`, `Nested`, `Classpath`, `Mutate`, `Model`, `Defaults`, `Finalize`, `Validate` | **no** (root mark) | recognised; `MyTask.doIt` root `gradle-task-annotations:attribute` |

Further evidence:

- `crates/cgg/tests/frameworks.rs:1198` (`aws-lambda/groovy`), `:1319` (`azure-functions/groovy`) and `:1337` (`gcp-functions/groovy`) are integration cases.
- `scripts/benchmark.sh:218` lists the groovy-gradle APPS expectations: `gradle-task`, `gradle-task-annotations`, `gradle-plugin`, `spock`, and others.

Helper tables: groovy appears in **none** of them. A grep for `groovy` in rules.rs finds only `language:` lines and gap prose. The tables are:

- `member_sources_for`;
- `signature_methods_for`;
- `container_attributes_for`;
- `markup_kinds_for`;
- `TEST_CLASS_MEMBERS_ARE_ROOTS` (python, php and ruby only);
- `RESPONSE_TRAITS` and `SELF_CONVERTING_WRAPPERS`.

Visibility entries and registrar verbs: none. The plugin records no visibility and no registrar arguments.

**Stale gap prose** (still present in 0.13.3):

- The `gradle-task` (rules.rs:5999) and `grails` (rules.rs:6061) gap strings say the Groovy plugin records neither annotations nor base types. Both are recorded (groovy.rs:162, 295), and `@TaskAction` and `Plugin<Project>` match.
- The `gradle-task` gap also says the plugin "deliberately skips `closure_expression`". No such kind exists; `closure` nodes are walked but never made callables.
- The `spock` gap (rules.rs:6036) is current.

Not supported:

- Grails controllers, interceptors, TagLibs and GORM hooks;
- Jenkins declarative and scripted pipelines and shared-library `vars/*.groovy` `call`;
- Gradle closures (`doLast`, `doFirst`, `task x {}`, `tasks.register`);
- Spring Boot (`@RestController`, `@Component`), Micronaut and Ratpack in Groovy files (no groovy rules);
- `static void main`. It is rooted only by the generic `builtin:main` dead-code root (§12). It is not a framework entry.

## 11. Test harness entries

- **Spock** (`spock` rule, rules.rs:6020-6039, kind `test`): detected by a `spock.lang`/`spock.util`/`spock.mock` import; enters `string-named` methods (feature methods) and the `setup`, `cleanup`, `setupSpec` and `cleanupSpec` fixtures of a `Specification`. Fixture f6 mints `<framework-entry>::test::spock` with entry edges to `FooSpec.setup` and `FooSpec.it computes`, both `test-entry` roots (`framework:test-harness`).
- **JUnit or TestNG in Groovy**: no groovy rule, so no harness node; `@Test` is rooted only by the generic dead-code `builtin:test-attr` root.
- **Naming patterns**: Groovy's usual `testSomething` (lower-case `test`, no word boundary) is **not** a test name: f6 `App.testSomething` is reported `never-referenced`.
- **`TEST_CLASS_MEMBERS_ARE_ROOTS`**: groovy is not listed.
- **Test-file gating**: by `src/test/`, `TEST_DIRS` or the `*Spec`/`*Test`/`*Tests.groovy` name rule (§01). In f6, `FooSpec.prep` (test file, called only from the fixture) lands in the withheld `only-used-by-tests` category.
- **`--no-test-entries`**: turns the Spock node into root marks.

## 12. Dead-code analysis

Capability (fixture f6 `capabilities` JSON):

| Field | Value |
|---|---|
| `class` | `degraded` |
| `visibility` | none |
| `attributes` | full (observed) |
| `value_references` | full (method pointers) |
| `dispatch` | none |
| `exports` | none |
| `test_tagging` | none |
| `max_confidence` | **medium** |

- The class is `Degraded` because no Groovy callable ever has a visibility (groovy.rs:292), and the plugin declares no `PluginSignals`.
- **Consequence**: Groovy dead-code findings are never `high`. f6 findings carry `language-lacks-dispatch-model` and `language-lacks-visibility` (no longer `language-lacks-value-references`).

Root kinds observed for Groovy:

- `builtin:main`: `static void main` (f6 `App.main`).
- `builtin:test-attr`: `@Test`.
- `framework:test-harness` and `framework:entry-node`: Spock (f6).
- `framework:rule`: `gradle-task-annotations:attribute`, `gradle-plugin:base-type`.
- `toplevel:invocation`: a callee called from a site with no enclosing callable (script top level, `.gradle` bodies).
- **Value references**: `def p = this.&reflected` keeps `App.reflected` live (f6). A method pointer outside any callable credits only its own file's same-named callables, and only through `this` (`FILE_SCOPED_VALUE_REF_LANGS`, deadcode/roots.rs:1369, 1388).

Confidence caps:

- **`implements-external-contract`**: through the recorded `@Override` annotation.
- **`reflection`**: never; there is no dynuse (§15). f6 `App.reflected2`, called only through `invokeMethod("reflected2", null)`, is `never-referenced` with no doubt evidence.
- **`macro-generated`**: not applicable.

Other points:

- **Inheritance fan-out under `--dead-code`** runs (groovy is in `INHERITANCE_LANGS`).
- **Public or exported roots**: none; library code is reported dead at medium (f6 `App.unusedPublic`).
- **Paren-less calls** now leave sites, so a method called only as a command (`log "x"`) is live.
- **Root coverage**: low in script-heavy code, where top-level calls have no callable; f6 reached 27%.

## 13. Builtins & stdlib tables

- **`crates/cgg-core/src/stdlib/groovy.txt`**: 339 names, loaded at stdlib.rs:771. Examples: `println`, `each`, `collect`, `find`, `with`, `tap`, `apply`, `sleep`, `Math`, `ArrayList`, `BigDecimal`, `asType`, `assert`.
- **`--include-stdlib`** (0.11.0 audit fixture f12) mints `<stdlib>` leaf nodes:
  - `println("a")` → `<stdlib>::println`;
  - `xs.each {…}` → `<stdlib>::List::each`, with the receiver typed `List` from the parameter;
  - `xs.collect` → `<stdlib>::List::collect`;
  - `Math.max` → `<stdlib>::Math::max`;
  - `sleep` → `<stdlib>::sleep`.
- **`--include-external`** (0.11.0 audit fixtures; f1 here confirms `lam(3)` → `<external>::lam`): `"s".toUpperCase()`, `[1,2].each` (literal receiver), `p.with`, `p.invokeMethod` and closure-variable calls become `<external>::…` leaves.
- **Groovy rows in `crates/cgg-core/src/stdlib.rs`**: `TOP_TYPES` (`Object`, stdlib.rs:673), the stdlib name table (stdlib.rs:771), `STDLIB_ROOTS` (`groovy`, `java`, `javax`, `org.codehaus.groovy`; stdlib.rs:847-848), `STDLIB_LIBRARY_NAMES`, `PROCESS_SPAWN_APIS` and `LIBRARY_LOAD_APIS` (read by `--externals`, whose import classifier treats Groovy with the JVM arm, `cgg-resolve/src/externals.rs:289`). No entry in `CAST_INTRINSICS`, `CONSTRUCTING_METHODS`, `MAYBE_TYPE_WRAPPERS`, `UNION_TYPE_CONSTRUCTORS`, `NULL_TYPE_MEMBERS` or `CALLBACK_TYPE_CONSTRUCTORS`.

## 14. Confidence & certainty

Paths that yielded **high** in the fixtures:

- intra-file unique-name binding (f1 lines 38, 55; f7 line 8);
- a parameter-typed receiver landing on one owner (f1 lines 41, 42, 60);
- construction, cast and `as` receivers (f1 lines 44, 53);
- a static import and a qualified static call on an imported class (f1 lines 43, 45);
- a call on a bare call's result through its declared return type (f2 line 20);
- one overload of a same-file set selected by arity (f1 lines 38, 39);
- constructions and `this(…)` delegation (f1 lines 31, 49);
- method pointers (f1 lines 56, 57, `reference`).

`OwnerFacts` for groovy requires every candidate's owner path to be imported, or the file to have no imports (cross_file.rs:5151); `qn_carries_module` anchors exact paths (cross_file.rs:5303).

What forces **medium**:

- `super.m()`: always (`super_owner_may_be_anonymous`, cross_file.rs:5225-5227);
- weak call-result types for `def x = make()` and untyped fields (type_hints.rs:1066; f2 lines 19, 23);
- overload sets where more than one overload survives;
- by-name fan-out for typed locals, fields, closure parameters and bare inherited names (f2, f1 line 40);
- calls under a wildcard import (f4).

**Low**: inheritance dispatch edges and framework-entry edges.

**False highs observed**:

- f7: a bare call in `B1` binds high to an unrelated `A1.onlyInA` in the same file. Intra-file unique-name binding ignores class scope.

**Mermaid `?` marker**: generic.

## 15. Dynamic use & reflection

- **No dynuse coverage**: groovy is not in `crates/cgg-lang/src/plugins/dynuse.rs`.
- **Dynamic names written as literals** (`obj."m"()`) are ordinary calls now (f8 line 8). Interpolated names (`obj."$name"()`) name nothing statically.
- **Unhandled reflective forms**: `obj.invokeMethod("m", args)` (f6), `methodMissing`, `propertyMissing`, `metaClass.m = {…}`, `Eval.me`, `GroovyShell.evaluate`, `getClass().getMethod("m")` and `Class.forName`. `methodMissing` is an ordinary method, dead unless called.
- **Module loads**: not applicable.

## 16. Known gaps & limitations

1. **Closures are never callables**, and script, `.gradle`, Jenkinsfile bodies have no enclosing callable: their calls are `no-enclosing-callable` (f1, f5).
2. **Explicit type arguments** (`obj.<T>m()`) break the parse and mint bogus `abstract` callables (f9).
3. **Imports**: `as` aliases are recorded but not read by the JVM import arm; wildcard `.*` imports are recorded without the `*`; default imports are not modelled; no `FileFacts::packages`; no `external-module` refusal, which produced a wrong medium edge to an unrelated workspace method (f1 line 48).
4. **Typing**: parameters, constructions, casts (`(T)` and `as`), declared return types and call-result locals (weak) only. Typed locals (`Dog d = …`, `def d = new Dog()`), typed fields and closure parameters are untyped (f2). No delegate typing.
5. **Bare calls ignore class scope**: they fan out to unrelated classes (f1 line 40), bind high to an unrelated same-file class's method (f7 line 8), and do not prefer the caller's own class (f7 line 9).
6. **No `overrides_below`** for typed receivers, and `super` is always medium.
7. **No JVM-family linking** (lang_family.rs:32 commented out).
8. **Unmodelled constructs**: anonymous class methods are named under the enclosing class; trait composition, AST-transform codegen, GORM dynamic finders and `metaClass` are not modelled.
9. **Dead code is capped at medium** (class `degraded`, no visibility). No reflection screen, no `testFoo` convention, no Grails or Jenkins roots.
10. **Detection**: no `.gvy`, `.gy` or `.gsh`, no `groovy` shebang, no `Jenkinsfile` (f5).
11. **Construction of a class with no declared constructor** leaves an unresolved `no-candidate-in-file` record (f1 line 44 `new Lib()`, f2 lines 8, 12, 14, 16, f6 line 3); Java leaves none for the same shape (f10).
12. **Stale rule prose**: the `gradle-task` and `grails` gap strings (rules.rs:5999, 6061) still say the plugin records neither annotations nor base types, and `gradle-task`'s says it skips `closure_expression`.

The plugin's 13 inline tests (groovy.rs:594-954) cover method pointers, string-named methods, packages, every call form, continued chains, imports, closures, script functions, empty and malformed input, `abstract` tagging, constructors and call-result locals; `groovy_next.rs` pins the grammar's shapes.

## Summary scorecard

| section | support | note |
|---|---|---|
| 01 Detection & parsing | partial | `.groovy`/`.gradle` only; no shebang, `.gvy`/`.gsh` or Jenkinsfile; vendored dekobon grammar parses idiomatic Groovy; explicit type arguments still error |
| 02 Callables extracted | good | methods, constructors, traits, records, annotation types, string-named and script methods; no closures or script body; anonymous classes mis-scoped |
| 03 Call-site forms | good | parenthesised and command calls, constructions, `this(…)`/`super(…)`, `?.`/`*.`, quoted names, method pointers, arity; no operators, closure-variable calls or type-argument calls |
| 04 Imports & module resolution | partial | single and static imports work; `as` aliases and wildcards broken; no packages, external-module refusal or JVM family |
| 05 Type inference & receiver typing | partial | parameters, `new`, `(T)`/`as` casts, declared return types, weak call-result locals; no typed locals, fields or closure parameters |
| 06 Inheritance, dispatch & overloading | partial | bases recorded; arity narrowing; inheritance fan-out; `super` always medium; bare calls unscoped; no `overrides_below` |
| 07 Scoping & shadowing | good | `locals::collect_groovy` parameters, closures, locals; `shadowed-by-local`; no class scoping of bare calls |
| 08 Macros, preprocessor & codegen | n/a | no macros; AST transforms and GORM codegen not modelled |
| 09 FFI & cross-language links | none | no FFI or descriptor links; Groovy↔Java family disabled |
| 10 Frameworks & entry points | partial | 9 groovy rules; 6 enumerate (aws-lambda, azure-functions, gcp-functions, gradle-plugin, gradle-task-annotations, spock); grails, jenkins and gradle-task are detection only |
| 11 Test harness entries | partial | Spock features and fixtures enumerate; `@Test` via generic root only; no `testFoo` convention |
| 12 Dead-code analysis | minimal | class degraded, max medium; value references and attributes full; no visibility or reflection screen |
| 13 Builtins & stdlib tables | partial | `groovy.txt` with 339 names; `TOP_TYPES` `Object`; default imports not modelled |
| 14 Confidence & certainty | partial | high for typed parameters, casts, `new`, imports, return types and arity; false highs for bare same-file names across classes |
| 15 Dynamic use & reflection | none | no dynuse; `invokeMethod` and `methodMissing` invisible |
| 16 Known gaps & limitations | n/a | 12 items above; biggest are closures/script bodies, import aliases/wildcards, typed locals and bare-call class scope |
