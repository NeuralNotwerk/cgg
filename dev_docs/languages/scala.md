# scala

The Scala plugin is `ScalaPlugin` in `crates/cgg-lang/src/plugins/scala.rs` (662 lines, about 175 of them inline tests). It is one recursive tree walker, `ScalaWalker::walk` (scala.rs:72-254). The walker keeps three stacks:

- a scope stack of named `class` / `object` / `trait` definitions;
- a stack of the enclosing type's bases;
- the `package` currently in effect.

The grammar is the crates.io crate `tree-sitter-scala = "0.26"` (Cargo.toml:101; `tree_sitter_scala::LANGUAGE`, scala.rs:21-23). Nothing is vendored. One grammar parses both Scala 2 and Scala 3 syntax. Fixture f3 `S3.scala` parsed `enum`, `given … with`, `extension` and indentation-based `object Runner:` with `parse_status: ok`.

Scala has no dedicated resolver kind. It runs the generic `intra_file` → `cross_file` pipeline, with Scala-only branches in these places:

| Area | Where |
| --- | --- |
| Library scope: type table, imports, bases, bare calls (WS14b) | `libscope.rs` (`build_scala`, `scala_base`, `resolve_scala_bare`, `scala_member`, `scala_import`) |
| External-head refusal, implicit packages | `heads.rs` (`scala_owners`, `scala_package_head`, `scala_import_bindings`) |
| Bare-call scope, imported objects, anchoring | `cross_file.rs` (`bare_call_reaches`, `scala_wild`, `anchored`, `narrow_owner_paths`, `super_owner_may_be_anonymous`) |
| Lexical nesting, overload sets, abstract-stub drop | `intra_file.rs` |
| `INHERITANCE_LANGS`, `free_variant_is_member` | `dispatch.rs` |
| Arity narrowing, callback types | `type_hints.rs` (`OVERLOAD_ARITY_LANGS`, `accepts_arity`, `callback_param_types`) |

The plugin overrides no `signals()`, so it declares every `PluginSignals` bit false (scala.rs:11-44; default at cgg-lang/src/lib.rs:304-306). Its `ExtractCtx` argument is ignored (`_ctx`, scala.rs:27). Twelve `RuleSpec`s in `crates/cgg-core/src/frameworks/rules.rs` have `language: "scala"`.

Fixtures used below are `f1` … `f8` under `<scratch>/parity/scala/`. They were run with `target/release/cgg` 0.11.0 (branch `remediate/cross-language`). The dump helper is `dump.py` in the same directory: `E` lines are edges, `U` lines are audit records.

## 01. Detection & parsing

- **Extensions `.scala` and `.sc`** (scala.rs:15-17).
  - Fixture f1: `scripts/run.sc` was detected `extension:.sc`, and its `def scriptFn()` was extracted.
- **Shebang `scala`** (scala.rs:18-20; test `plugin_loads`, scala.rs:543-548).
  - Fixture f1: an extensionless `scripts/runme` starting `#!/usr/bin/env scala` was detected `shebang:scala`. `S::main` → `S::hello` bound `high`.
- **Content detection**: none. `crates/cgg-lang/src/detect.rs` contains no `scala` string.
- **Not detected**:
  - `.sbt` build definitions and `.mill` files: they are not in the extension list (scala.rs:16).
  - The `.scala.html` Twirl templates Play uses: not detected as Scala (unverified).
- **Grammar**: a single grammar. There is no `ts_language_for` override, so `.sc` worksheets and `.scala` files share it. The grammar accepts Scala 3 indentation syntax (fixture f3: `object Runner:` with indented `def`s; `Runner::run` → `Runner::helperIndented` bound `high`).
- **Re-parse / fallback**: none. Scala has no conditional compilation, and the plugin has no error-recovery path. A tree with errors is walked as it is.
- **Skip conditions**:
  - Depth: a tree deeper than `MAX_TREE_DEPTH` = 4000 is skipped as `TooDeep` (crates/cgg/src/lib.rs:155, 451). This applies to every language.
  - Long lines: `max_line_bytes()` is not overridden, so no `LongLine` skip applies to Scala (lib.rs:313-315 default `None`).
  - Minified files: `MINIFIED_LINE_BYTES` = 2000 (crates/cgg/src/lib.rs:159) gates only the generic `locals::collect`, and Scala has a locals table (section 07).
- **Test-file classification** (`cgg_core::classify_test_file`):
  - A Maven/sbt `src/test/` path counts for `java | kotlin | scala | groovy` (crates/cgg-core/src/testfile.rs:61-66).
  - File names `*Spec.scala` and `*Test.scala` count (testfile.rs:118).
  - The generic directory rule also applies.
  - `*Suite.scala`, the conventional ScalaTest name, is not matched by name (testfile.rs:118). Such a file is a test file only if it lives under `src/test` or another test directory.

## 02. Callables extracted

Qualified names join the scope stack with `::` (`qn`, scala.rs:64-70). The stack holds only named `class_definition`, `object_definition` and `trait_definition` nodes (scala.rs:133-171). **The package never enters the qualified name**: `org.y.core.Svc.handle` is `Svc::handle`. The package is recorded separately, on `TypeSpan::package` (scala.rs:149-157) and as a `scala-package` import record (scala.rs:94-100).

Every definition except an abstract one is recorded as `DefVariant::FreeFunction`. JSON shows it as `"kind": "function"`; the audit shows `freefunction`. The resolver treats an owner segment in the name as what makes it a member (`dispatch::free_variant_is_member`, dispatch.rs:363-368; `bare_call_reaches`, cross_file.rs:4176-4180).

Supported shapes:

- **Methods of a class, object or trait.** `class Svc { def handle(x: Int): Int = x }` → `Svc::handle`.
  - Evidence: scala.rs:172-182; test `trait_methods_are_owned_by_the_trait` (scala.rs:566). Fixture f1 extracted `Square::area`, `Point::norm`, `Square::apply`, …
- **Companion objects** are a separate scope with the same name. `object Square { def apply(…) }` → `Square::apply`, which shares the `Square::` owner with the class's members in the qualified name.
  - `TypeSpan` keeps them apart by `kind` (`class` vs `object`) (test `types_packages_and_constructor_delegation`, scala.rs:601-632).
  - Fixture f1 printed `Square::area` (class) and `Square::apply` (object) under the same owner text.
- **Nested types** extend the scope stack. `object Ext { implicit class RichInt(…) { def double … } }` → `Ext::RichInt::double` (fixture f2).
- **Abstract members.** `def area(): Double` with no body is a `function_declaration`. It is recorded as `DefVariant::TraitMethod` (JSON kind `method`) tagged `abstract` (scala.rs:183-200; test `abstract_defs_are_bodiless_declarations`, scala.rs:584).
  - It is recorded only inside a type. A top-level bodiless `def` is skipped (scala.rs:193; same test).
  - `@native def nativeFn(i: Int): Int` is also a bodiless declaration, so it becomes `abstract` with attribute `@native` (fixture f4).
- **Auxiliary constructors.** `def this(s: String) = this(s.toDouble)` is recorded as a callable named `Square::this` (fixture f1).
- **Operator methods.** `def +(o: Square)` → `Square::+`; `def unary_- : Square` → `Square::unary_-` (fixture f1).
- **Parameterless defs.** `def size: Int = 4` → `Square::size`. This is the shape paren-less sites can bind to (section 03).
- **Nested (local) defs.** `def main(…) = { def nested(x: Int) = … }` → `Main::nested`. The enclosing function is not part of the name (fixture f1; nesting is handled by span, section 07).
- **Overloads.** Two `def ov(…)` in one object both become `Ops::ov` with distinct spans (fixture f4: `Ops::ov` at lines 17, 18, 19).
- **Curried and implicit parameter lists** are kept in `signature_hint` (`super::extract_signature`, scala.rs:354). Fixture f4: `def curried(a: Int)(b: Int)` and `def implicitP(a: Int)(implicit ctx: String)` were both extracted.
- **Scala 3 top-level defs.** `def topLevel3(i: Int)` at file level → `topLevel3`, with no owner (fixture f3).
- **Script files.** `.sc` top-level `def scriptFn()` → `scriptFn` (fixture f1).
- **Attributes.** Annotations preceding a def are collected by the shared collector `attrs::collect_with_preceding` (attrs.rs:70; scala.rs:356-360). The attribute string keeps its arguments. Fixture f3: `@FunctionName("hello")`, `@deprecated("x", "1")`, `@Test`.
- **Bases** are read by `attrs::base_types_with_ctor_args` (attrs.rs:195; scala.rs:138-139). They go on every `DefRecord::base_types` of the type (scala.rs:344-346) and on `TypeSpan::bases`.
  - A base written with constructor arguments (`extends Compactible[Entry](v, spark, p) with Mixin`) is kept with its type arguments (test `a_base_with_constructor_arguments_is_recorded`, scala.rs:653).
- **Anonymous classes.** `new T { … }` is recorded as a `TypeSpan` of kind `anon`, with bases `T` (scala.rs:106-131). It does not get its own scope.

Not supported, or only partly:

- **No callable for a class, object or trait itself, and none for a primary constructor.** Only `function_definition` and `function_declaration` mint callables (scala.rs:172-200).
  - Code in a class body (field initialisers, `test("…") { … }` registrations, `lazy val`) belongs to no callable (section 03).
  - The `play-module` gap text says the same (rules.rs:5517-5533).
- **`val` / `var` / `lazy val` function values are not callables.** Examples: `val f = (i: Int) => …`, `val index = Action { … }`, `val routes: Route = …`. Only `class_fields` records them (scala.rs:206-236).
  - Evidence: fixture f3 `val f: Int => Int` has no node. The `play-mvc` and `akka-http` gap texts say the same (rules.rs:5512, 5555).
- **Lambdas and partial functions are not callables.** Their call sites are attributed to the enclosing def by span (fixture f2: a call inside `withAnimal(d => d.bark())` is an edge from `Zoo::main`).
- **Anonymous-class members are named under the enclosing class.**
  - Fixture f2: `new Runnable { def run() = speak() }` inside `Dog::run` produced a second callable also named `Dog::run`.
  - The `anon` TypeSpan does not push a scope (scala.rs:108-131).
- **Scala 3 `enum`, `given … with` and `extension` members get no owner.** These node kinds are not scopes in the walker.
  - Fixture f3: `enum Color: … def label` → `label`; `given intOrd … def compare` → `compare`; `extension (s: String) def shoutIt` → `shoutIt`.
- **`DefRecord::visibility` is always empty** (scala.rs:355). `private` and `protected` are not read.
- **No `FileFacts::classes` (`ClassDecl`) records.** Scala is not in `CLASS_RECORDS_ARE_EXACT` (dispatch.rs:130). Type information exists only as `TypeSpan`s.

## 03. Call-site forms

Calls are recorded from `call_expression` (`record_call`, scala.rs:388-449) and from paren-less positions (`record_paren_less`, scala.rs:297-324; `paren_less_position`, scala.rs:465-486).

Supported:

- **Bare call.** `helperFn(1)` → name `helperFn`, empty receiver, with arity.
  - Test `member_calls_split_receiver_and_paren_less_sites_are_marked` (scala.rs:509).
- **Member call.** `b.m()` → `m` through receiver `b`. It is never one name `b.m` (scala.rs:400-423; same test). The CHANGELOG records the fix (CHANGELOG.md:826).
- **`this.m()`.** The receiver is `this`, which the Scala bare-call path treats as bare (cross_file.rs:5614-5616). Fixture f2: `this.speak()` → `Dog::speak` `high`.
- **Chained / dotted receivers.** `a.b.m()` keeps `a.b` as the receiver text, whitespace removed (`receiver_text`, scala.rs:259-279).
- **Construction as receiver.** `new Auction(1).behavior(2)` → receiver `Auction`, flagged `typed_receiver` (scala.rs:259-295; test at scala.rs:522-527). `new Box[Int]()` reads as `Box` (scala.rs:271-276).
  - Fixture f1: `(new Square(1.0)).area()` → `Square::area` `high`.
- **Cast as receiver.** `o.asInstanceOf[Foo].m()` → receiver `Foo`, typed. This uses `receiver::peel` with `generic_function` and `CAST_INTRINSICS` (receiver.rs:211-225; stdlib.rs:45).
  - Fixture f1: `s.asInstanceOf[Square].area()` → `Square::area` `high`.
- **Call-form construction as receiver.** `Foo(…).m()` is an instance of `Foo`, because Scala constructs by call (`constructs_by_call`, receiver.rs:47-51).
- **Explicit type arguments.** `f[Int](1)` and `b.m[String](2)` are recorded as `f` and `m`: the `generic_function` wrapper is unwrapped (scala.rs:392-399; test `explicit_type_argument_calls_strip_the_arguments`, scala.rs:637). Fixture f1: `generic[Int](3)` → `Main::generic` `high`.
- **Auxiliary-constructor delegation.** `this(…)` inside `def this` is recorded as a call named after the enclosing class, never `this` (scala.rs:428-439; test `types_packages_and_constructor_delegation`).
  - It binds nothing, because no constructor callable exists. Fixture f1: `Square` at L16 was `no-candidate-in-file`.
- **Arity.** A parenthesised argument list sets `RefRecord::arity`. A block argument (`f { … }`) leaves it unknown (`call_arity`, scala.rs:451-457; test `a_base_with_constructor_arguments_is_recorded` asserts `Some(2)`).
- **Paren-less sites.** These are a bare `helper` or a `b.size` in an evaluated position: a block statement, an argument, an operand, a definition value, a branch, an interpolation, a receiver (scala.rs:465-486). They are recorded with `paren_less: true` (scala.rs:315-323).
  - `value_refs::prune_paren_less` (value_refs.rs:125) keeps a site only when some parameterless `def` of that name exists (`declares_no_parameters`, value_refs.rs:91-100; test `paren_less_sites_survive_only_for_parameterless_callables`, value_refs.rs:224).
  - A paren-less site that binds nothing leaves no audit record (crates/cgg/src/lib.rs:1070-1076).
  - Fixture f1: bare `lazyThing` → `Main::lazyThing` `high`. Fixture f5: bare `size` → `A::size` `high`; `b.size` with `b: Box` a parameter → `Box::size` `high`.
- **Calls inside string interpolation.** `s"${max2(3)}"` is recorded (`interpolation` is an evaluated position for paren-less sites; call_expression is walked everywhere). Fixture f4: `max2(3)` bound at L37.
- **Calls inside lambdas, `for` comprehensions, `match` cases and `foldLeft` closures** are recorded and attributed to the enclosing def (fixture f3 L20; fixture f4 L38-39).

Not supported, or only partly:

- **Curried / multi-argument-list calls record a garbage outer name.** In `curried(1)(2)` the outer `call_expression`'s function is itself a call, and its text becomes the name.
  - Fixture f4: an external record named `curried(1)` (and `List(1).foldLeft(0)`) next to the correct inner `curried` edge.
  - Fixture f3: a ScalaTest `test("handles") { … }` produced an external record named `test("handles")`.
  - Cause: scala.rs:424-448 uses `self.text(function)` for any non-field callee.
- **Infix method calls are not recorded.** `1 max2 2`, `s + q` and `a max b` produce no reference (fixture f4 L30, fixture f1 L48; there is no `infix_expression` arm in `walk`).
- **Prefix operators are not recorded.** `-s` → `unary_-` produces no reference (fixture f1 L49).
- **`apply` sugar is not linked.** `Ops(3)` and `Square(3.0)` stay `no-candidate-in-file` and never reach `Ops::apply` / `Square::apply` (fixtures f1 L39, f4 L24). Step 7 `call_operator_names` does not cover this shape (unverified for Scala). `Ops.apply(4)` written out does bind (fixture f4 L25, `high`).
- **Case-class construction is not linked.** `Point(1, 2)` has no target, since there is no constructor callable (fixture f1 L37).
- **Calls through a local function value.** `val g = …; g(1)` and `val f: Int => Int; f(2)` land in the **external** bucket, not as `shadowed-by-local` (fixtures f3 L19, f4 L32). The `f(new Dog)` parameter call in fixture f2 L35 does the same.
- **Method references, eta-expansion and functions passed as values.** `map(inc)`, `inc _` and `Option(1).map(ov)` record nothing (fixtures f1 L57-58, f4 L36). The plugin has no `value_ref` support, and `Main::inc` was a dead-code finding.
- **Paren-less member access through a local `val`.**
  - `val b: Box = new Box(1); b.size` and `val c = new Box(2); c.size` bound nothing (fixture f5b).
  - In fixture f5, with an `object B` in the same file, `b.size` bound to **`B::size` at `high`**. That is a false high (see section 14). The mechanism was not confirmed.
- **Paren-less call to a def declared with `()`.** `helper` for `def helper()` is dropped as a value read, by design (fixture f5 L17; value_refs.rs:91-93). Scala 2 auto-application would call it.
- **Registrar arguments**: none. The plugin does not use `registrar.rs`, and user-supplied registrar verbs are ignored (`_ctx`, scala.rs:27).
- **Calls in a class body, an `object` body or `.sc` top level** have no enclosing callable. They are audited `no-enclosing-callable` (fixture f1: `compute` at L20 from `val lazyVal = compute()`; `scriptFn` in `run.sc`). Dead code roots their targets (section 12).

## 04. Imports & module resolution

- **Import records.** `import_declaration` is recorded with kind `import`, and the path is the node text with `import` stripped and whitespace removed (`record_import`, scala.rs:365-386). So `import a.b.{X, Y => Z}` becomes `a.b.{X,Y=>Z}`.
- **Package records.** A `package a.b` clause is recorded as an import of kind `scala-package`, and the package path is chained across successive clauses (`package org.ex` then `package sql` → `org.ex.sql`).
  - A block `package a { … }` applies only inside the block and is **not** recorded as an import (scala.rs:75-104; test `types_packages_and_constructor_delegation`).
  - The plugin does **not** write `FileFacts::packages`. The heads and libscope code read `scala-package` imports instead (heads.rs:126-140; libscope.rs:743-752).
- **Package objects.** `package object core { def pkgFn }` in `package.scala` → `pkgFn` with no owner (fixture f3).
  - An owner-less def is reachable by bare name only through an import of its package, and only from a file whose stem is `package` (cross_file.rs:4181-4192). Fixture f3: `import org.y.core._; pkgFn(4)` → `pkgFn` `medium`.
- **Import forms the binding tables understand** (`scala_import_bindings`, heads.rs:952-990; test `scala_import_bindings_follow_selectors`, heads.rs:1231; `scala_import`, libscope.rs:2311-2343):
  - `a.b.C` binds `C`.
  - `a.b.{C, D => E}` binds `C`, and binds `E` as `a.b.D`.
  - `a.b._` / `a.b.*` / `{_}` are wildcards.
  - `D => _` hides a name.
  - heads.rs splits `import a.b, c.d` into its clauses. libscope's `scala_import` does not (libscope.rs:2313-2343 has no top-level comma split), so a multi-clause import gives libscope one mangled exact binding (code reading, not run).
- **Wildcard-imported objects.** `import a.b.Owner._` / `import a.b.Owner.m` brings an object's members, and those of its bases, into bare-name scope (`scala_wild`, cross_file.rs:2223-2234 and 4207-4237).
  - Fixture f1: `import com.ex.util.Helpers._; helperFn(1)` → `Helpers::helperFn` `medium`. `Helpers.util1()` → `high`.
  - Test `scala_bare_call_reaches_bases_and_imported_objects_only` (cross_file.rs:14085).
- **Relative imports.** `import util.X` inside `a.b` is kept as a possible tree import when its head is a declared package segment (`jvm_segments`, heads.rs:88-91 and 600-611).
- **Imports from a value.** `import spark.implicits._` is not a package root (heads.rs:587-593; test `a_scala_import_from_a_value_is_not_a_package_root`, crates/cgg/tests/resolve.rs:3363).
- **External-module refusal** (`heads.rs`). Scala is one of the `external_head` languages (heads.rs:659-672).
  - An import whose package the tree does not declare (`jvm_workspace`), and whose head is neither a package segment nor a Scala owner or member `val`, marks its bound names external (heads.rs:600-612).
  - Fixture f1: `import org.json4s.jackson.JsonMethods.{render}; render(1)` → external.
  - A bare call to a name explicitly imported from outside the tree is external in `resolve_scala_bare` (libscope.rs:937-962).
- **Implicit packages.** A receiver headed by `scala`, `java`, `javax`, `math`, `collection`, `concurrent`, `util`, `io`, `sys`, `reflect`, `annotation` or `runtime` (`SCALA_IMPLICIT_PACKAGES`, stdlib.rs:71-84) is external when nothing in the tree binds the head (`scala_package_head`, heads.rs:741-767). Non-root heads are reported as `scala.<head>` (`SCALA_ROOT_PACKAGES`, stdlib.rs:96).
  - Test `scala_calls_through_standard_and_dependency_packages_are_external` (crates/cgg/tests/resolve.rs:3250).
  - Fixture f1: `math.ceil(1.0)` → external. Fixture f2: `java.util.UUID.randomUUID()` and `scala.util.Try(1)` → external.
- **Workspace vocabulary.** It is built from `scala-package` imports, `class_fields` names and the owner segments of Scala qualified names (`WorkspaceVocab::build`, heads.rs:126-140 and 183-188).
- **Scoped lookup file match.** Scala imports point `ScopedSimple` at the imported package directory only when they can name a package-object member: a wildcard or a lowercase selector (cross_file.rs:2007-2025, 2209-2211).
- **Type anchoring.** A typed-receiver candidate counts as anchored when no import names the type, or when the candidate's **file path contains the imported package as a directory path** (`anchored`, cross_file.rs:4626-4643).
  - Fixture f3, with directories that mirror the package (`src/main/scala/org/y/core/Svc.scala`): `s.handle(1)` → `high`.
  - Fixture f2, with directories that do not (`a/Base.scala` for `org.x.a`): `a.speak()` → `medium`.
- **Not supported, or only partly:**
  - **Renamed-import receivers are not followed.** `import com.ex.util.{Tools => T}; T.tool()` ended as external plus `candidates-in-other-files` (fixture f1 L45), while `Other.go()` from the same selector bound `high`.
  - **Scala 3 top-level defs are unreachable from other files, even when imported explicitly.** Fixture f8: `import p._; top3(1)` and `import p.top3; top3(1)` are both `candidates-in-other-files`. Only a file named `package.scala` qualifies (cross_file.rs:4184-4191, whose comment reads "Scala 2 has no top-level `def`").
  - **Scala → Java calls in the same tree do not resolve.** The JVM language family is commented out (crates/cgg-core/src/lang_family.rs:32).
    - Fixture f6: `j.jm()` with `j: org.v.J` (a Java class in the tree) and `J.js()` were both `no-candidate-in-file`, and the Java methods became dead-code findings.
  - **No manifest reading** (`build.sbt`, `build.sc`). `manifest.rs` has no Scala branch (grep).
  - **An unknown capitalised receiver head is reported external, not unresolved.** Fixture f2: `UnknownThing.call()` → external. The exact rule that fires was not confirmed.
  - **`not-in-imported-module`** (modules.rs) is not used for Scala: modules.rs has no Scala branch (grep for `scala` in modules.rs: none).

## 05. Type inference & receiver typing

- **Parameters.** `def go(s: Svc)` types `s` through the generic `name: Type` reader (type_hints.rs:2000 comment). `cgg_core::declared_type` strips `Foo | Null` / `Foo | Nothing` (`NULL_TYPE_MEMBERS`, stdlib.rs:136; test at annotation.rs:187; type_hints.rs:3434).
  - A union of two real types stays untyped (type_hints.rs:3434-3436).
  - `Option[Foo]` stays a container (annotation.rs:233; the stdlib.rs:99-104 comment says this is on purpose).
  - Fixture f2: `n: Animal | Null; n.speak()` → `Animal::speak`. `o: Option[Animal]; o.get.speak()` fell back to by-name fan-out over `Dog::speak` and `Cat::speak` at `medium`.
- **Top types.** `Any`, `AnyRef` and `AnyVal` say nothing about the receiver (`TOP_TYPES`, stdlib.rs:152). Fixture f7: `x: Any` → `fanout-cap-exceeded` (6).
- **Locals.** `val s = new Square(2.0)`, `val c: Cat = new Cat` and `val local = new Svc()` are typed by the generic file-wide propagation. Calls through them bind, at `medium` (fixture f1 L36 `s.area()`; fixture f3 L15 `local.handle(3)`; fixture f2 L39 `c.go(…)`). Which pass assigns the local type, and why the edge is not `high`, was not confirmed.
- **Fields and constructor parameters.** `class App(svc: Svc)` and `private val helper: Svc = new Svc` are recorded in `class_fields` with their written type (scala.rs:206-236). Calls through them bind at `medium` (fixture f3 L12, L13).
- **Casts and constructions as receivers** are marked `typed_receiver` (section 03; scala.rs:285-295).
- **Typed closure parameters and typed patterns** are recorded by span in `FileFacts::scoped_types`. This covers `(d: Dog) => d.bark()` (lambda `bindings`) and `case d: Dog => d.bark()` (`typed_pattern`) (closure_types.rs:53-75, 131-135, 197; test `scala_typed_lambdas_and_patterns_are_scoped`, closure_types.rs:397). They win over every other binding.
  - Fixture f2 L16-17: both → `Dog::bark` `high`.
- **Untyped closures passed to a typed callback.** `withAnimal(d => d.bark())`, where `def withAnimal(f: Dog => Unit)`, takes `Dog` from the callee's `=>` type (`callback_params_of` / `callback_param_types`, type_hints.rs:1199-1265; test `an_untyped_closure_takes_the_callees_declared_callback_types`, type_hints.rs:3583).
  - A type parameter (`T => Unit`), a top type (`Any => Unit`) or an arity mismatch leaves the closure untyped (near-miss cases in the same test).
  - Fixture f2 L37: `Dog::bark` `high`.
- **Implicit receivers**: not applicable. `settle_implicit_receivers` is Kotlin-only (CLAUDE.md, cgg-resolve step list). Scala implicit conversions and implicit classes are not modelled: fixture f2 `3.double`, with `implicit class RichInt`, bound nothing. The CHANGELOG notes 741 corpus sites that reach an implicit-class extension or a Java interface method as `candidates-in-other-files` (CHANGELOG.md:1491-1493).
- **Weak receiver types** (`weak_receiver_type`): not set by the Scala plugin (grep: no `weak_receiver_type` in scala.rs).
- **Not supported:**
  - The return type of a call used as a receiver (`pick().q()`) was not typed. Fixture f7 `val w = pick(); w.q()` → `fanout-cap-exceeded`.
  - Self-types (`self: T =>`) are not supported (CHANGELOG.md:1351).
  - `MAYBE_TYPE_WRAPPERS`, `UNION_TYPE_CONSTRUCTORS` and `CALLBACK_TYPE_CONSTRUCTORS` have no Scala entries (stdlib.rs:99-147). Scala's `=>` callback syntax is parsed directly instead (type_hints.rs:1243-1265).

## 06. Inheritance, dispatch & overloading

- **Base records.** Every def carries its enclosing type's bases (scala.rs:344-346). Every type has a `TypeSpan` with its kind, its bases, its package and its dotted path (`Outer.Inner`) (scala.rs:140-158).
- **Bases resolved through imports** (WS14b). `scala_base` resolves a written base in this order (libscope.rs:798-901):
  1. a type of the same file;
  2. an exact import, also prefixed with each wildcard and the type's package;
  3. the type's package;
  4. a wildcard import;
  5. as a by-name fallback flagged as guessed, the single type of that simple name.
  - Companion objects are excluded as bases (libscope.rs:820-828).
  - Test `scala_base_is_resolved_through_the_declaring_files_imports` (cross_file.rs:13987).
- **Bare calls to inherited members.** `resolve_scala_bare` looks the name up in the innermost type around the site, then up its bases breadth-first to depth 8, then in the enclosing types (libscope.rs:907-1000; cross_file.rs:5610-5631). A companion object's members are not the class's.
  - Fixture f2: `greet()` in `Dog` → `Animal::greet` `high`; `shout()` → `Loud::shout` `high`.
- **A member beats an import.** A member the calling class declares or inherits outranks a wildcard or explicit import of the same name (SLS 2) (cross_file.rs:5861-5880; also `member_of_caller` in the external-head path, cross_file.rs:5353).
- **`super.m()`.** It binds through the base chain. Fixture f2: `super.common(n)` → `Animal::common`.
  - It is **never `high`**, because `super_owner_may_be_anonymous` includes Scala (cross_file.rs:4693-4699; CHANGELOG.md:56-57).
  - Scala is not in the languages that refuse `super-base-out-of-graph`; that list is Python, Kotlin, Java and C# (CLAUDE.md).
- **Trait linearization is not modelled.** With two mixins declaring `shout`, `Cat extends Animal with Quiet with Loud` binds a bare `shout()` and `c.shout()` to **both** `Quiet::shout` and `Loud::shout` at `medium` (fixture f2 L30, L40). Scala would pick `Loud` (the last mixin). `one_declaring_ancestor` keeps such a case off `high` (cross_file.rs:4648-4690).
- **`member_of_calling_class` / `overrides_below`** are Java, C# and Ruby only (cross_file.rs:5631-5645). Scala uses `resolve_scala_bare` instead.
- **Owner narrowing.** A typed receiver over several same-named owners narrows by the qualifier written, or by the caller's own nested scope (`narrow_owner_paths`, cross_file.rs:4847).
- **Abstract declarations.** These are bodiless `def`s tagged `abstract`.
  - Duck-typed fan-out drops them when a concrete candidate survives (cross_file.rs:1153-1160).
  - Intra-file prefers a concrete candidate over them (intra_file.rs:575-585).
  - Fixture f1: `r: Shape; r.area()` → `Square::area` (`medium`), not `Shape::area`.
- **`--dynamic-dispatch` / `inheritance_fanout`.** Scala is in `INHERITANCE_LANGS` (dispatch.rs:108-122). Members recorded as free functions still override (`free_variant_is_member`; test `scala_and_dart_members_recorded_as_free_functions_still_override`, dispatch.rs:767).
  - Fixture f2 with `--dynamic-dispatch`: `Animal::speak` → `Dog::speak`, `Cat::speak`; `Loud::volume` → `Dog::volume`, `Cat::volume`; `Animal::common` → `Dog::common`. All are `low`, resolver `dispatch:inheritance`, `Via::Dynamic`.
  - The pass also runs under `--dead-code`.
- **Overload sets.** Same file plus same qualified name is one callee (`binds_same_name_set`, intra_file.rs:836-850).
  - Scala is in `OVERLOAD_ARITY_LANGS` (type_hints.rs:1583). `accepts_arity` (type_hints.rs:1585-1600) handles defaults, `T*` varargs and implicit / `using` lists (an implicit list takes 0 arguments).
  - Fixture f4: `ov(1)` → `Ops::ov@17` and `@18` (the second has `b: Int = 2`), excluding `@19`; `ov("a",1,2)` → `@19` only; `implicitP(1)` → bound; `varargs(1,2,3)` → bound.
  - All the overload edges were `medium`, including the one narrowed to a single overload. The CHANGELOG says arity-narrowed-to-one can be `high` (CHANGELOG.md:58-59), but that was not observed here.
- **Interface/trait impls (`trait_impl_target`)** are not recorded (`impls` signal false). Dead code reports `dispatch: none` (fixture f1 capability row).
- **Extension methods** (Scala 3 `extension`, Scala 2 implicit classes) are not linked (fixtures f2 `3.double`, f3 `"hi".shoutIt`). C#-style step 4d is C#-only.
- **Mixins** are treated as ordinary bases (fixture f2). Self-types are not supported (CHANGELOG.md:1351).

## 07. Scoping & shadowing

- **`FileFacts::locals`** comes from the generic `locals::collect` with `SCALA_SPEC` (crates/cgg-lang/src/locals.rs:221-242, 385; test `dart_ocaml_lua_python_rust_kotlin_swift_scala_zig`, locals.rs:840). The spec covers:
  - binders: `parameter`, `class_parameter`, lambda `parameters`, `binding`, `val` / `var` patterns and `case` patterns;
  - function scopes: `function_definition` and `lambda_expression`;
  - blocks: `block`, `case_clause` and `for_expression`;
  - patterns: tuple, case-class, typed, infix, capture and alternative.
- **`shadowed-by-local`.** `val helper = () => 5; helper()` is refused rather than bound to `A::helper` (fixture f5 L21: `shadowed-by-local`).
  - A call through a local function value whose name matches no definition lands in the external bucket instead (section 03).
- **Local receivers are not package heads.** A receiver headed by a parameter or local in scope is never read as a package (heads.rs:676-678; heads.rs:745-750 comment).
- **Member values shadow packages.** A class `val io` keeps a receiver `io.x` from reading as `scala.io` (scala.rs:206-236; heads.rs:93-95, 126-128).
- **Lexical narrowing of nested defs.** Scala is in `lexically_scoped` (intra_file.rs:118-135). A def nested in a function is visible by bare name only inside it.
  - Fixture f5: `three()` → its own `A::inner` `high`. `A::four` did not bind to `A::inner`, which is nested in `three`.
- **Positional shadowing**: not applied. It is OCaml, F# and Lua only (CLAUDE.md).
- **Conditional definitions**: not applicable (PHP only).
- **Known false binding.** A bare call from one object to a name declared only in **another object of the same file** binds intra-file at `high`.
  - Fixture f5: `A::four` `inner()` → `B::inner` `high`. Fixture f5b, with the object renamed `Q`: `Q::inner` `high`.
  - The CHANGELOG lists this as not covered: "Scala bare calls bound in-file to another class's same-named member" (CHANGELOG.md:708-709).
  - Across files the same call is refused (fixture f5 `C.scala`: `candidates-in-other-files`).

## 08. Macros, preprocessor & codegen

- **Not applicable / none.**
  - Scala 2 `def m: T = macro impl` and Scala 3 `inline` / `${ … }` quotes are not expanded or specially tagged. The plugin has no macro handling (grep: no `macro` in scala.rs).
  - No `#define` / `#if`.
  - No `macro:<name>` tagging and no `name_mentions` (grep: neither appears in scala.rs).
- **Code generated at build time is invisible.**
  - sbt source generators, the Play routes compiler and protobuf `GreeterGrpc` stubs are not in the tree, so they are not seen. The `play-mvc` gap text covers the routes compiler (rules.rs:5512).
  - Fixture f4: `extends GreeterGrpc.Greeter` names an out-of-tree generated base.

## 09. FFI & cross-language links

- **`ffi::link_ffi`**: there are no Scala-specific passes (ffi.rs:7-14 lists the families).
  - A Scala `@native def` carries the attribute `@native`. The JNI classifier keys on `native` / `JNIEXPORT` (ffi.rs:664-668, 812-823). Whether `@native` (with `@`) classifies was not confirmed. Fixture f4 produced no FFI edge, but it had no C side, so this is unverified.
  - No PyO3, wasm-bindgen, napi or P/Invoke analogues apply.
- **Descriptor links** (`descriptor::link_descriptors`) are host-language-agnostic.
  - Fixture f4: proto `rpc SayHello` in `service Greeter` → `GreeterImpl::sayHello`, `low`, resolver `descriptor-linker`, `Via::Descriptor`.
  - A proto `message` never links to a Scala callable (test `a_proto_message_in_a_package_links_to_nothing`, descriptor.rs:216-222, which uses a Scala callable).
- **Java interop inside one tree is not linked** (section 04, fixture f6).
- **Markup code-behind**: not applicable (`markup.rs` is C#-only).

## 10. Frameworks & entry points

Twelve rules have `language: "scala"` in `crates/cgg-core/src/frameworks/rules.rs`. No rule lists Scala among several languages; every multi-runtime framework has its own per-language row (rules.rs:4203 comment). Detection is by import prefix (`detect`).

| rule id | line | kind | trigger | binds? |
| --- | --- | --- | --- | --- |
| `aws-lambda` | 4189 | network | import `com.amazonaws.services.lambda` / `com.amazonaws.lambda`; `base_types` `RequestHandler`, `RequestStreamHandler`; method `handleRequest` | yes. Fixture f3: `<framework-entry>::network::aws-lambda::H::handleRequest` → `H::handleRequest`. Test `aws-lambda/scala` (crates/cgg/tests/frameworks.rs:1192) |
| `azure-functions` | 4343 | network | import `com.microsoft.azure.functions`; attribute `FunctionName` | yes. Fixture f3: `@FunctionName("hello")` entry → `App::hello`. Test `azure-functions/scala` (frameworks.rs:1313) |
| `gcp-functions` | 4898 | network | import `com.google.cloud.functions`; `base_types` `HttpFunction`, `BackgroundFunction`, `CloudEventsFunction`, `RawBackgroundFunction`, `TypedFunction`; methods `service`, `accept`, `apply` | yes. Test `gcp-functions/scala` (frameworks.rs:1331) |
| `play-mvc` | 5499 | network | import `play.api.mvc`, `play.mvc`, `play.api.routing` | **no**: gap-only (no attributes, bases, methods or registrars) |
| `play-module` | 5516 | lifecycle | import `play.api.inject`, `play.api.ApplicationLoader`, `play.api.BuiltInComponents` | **no**: gap-only |
| `akka-http` | 5537 | network | import `akka.http.scaladsl` / `javadsl`, `org.apache.pekko.http.*` | **no**: gap-only |
| `akka-scala` | 5559 | queue | import `akka.actor`, `org.apache.pekko.actor` | **no**: gap-only (`receive`, `preStart` and the like are "named too commonly") |
| `akka-typed` | 5576 | queue | import `akka.actor.typed`, `org.apache.pekko.actor.typed`, `akka.persistence.typed`, … | **no**: gap-only |
| `http4s` | 5598 | network | import `org.http4s` | **no**: gap-only |
| `spark` | 5615 | cli | import `org.apache.spark` | **no**: gap-only |
| `scalatest` | 5632 | test | import `org.scalatest`, `org.scalatestplus` | **no**: gap-only. Fixture f3 reported it under `seen_no_rules` with its gap text |
| `specs2` | 5649 | test | import `org.specs2` | **no**: gap-only |

- A gap-only rule is still detected. It surfaces in `--framework-coverage` / the audit `framework_coverage.seen_no_rules` with its `gap` text (fixture f3).
- **Entry nodes.** These are `<framework-entry>::<kind>::<id>::…` nodes with `entry` edges at `low` (fixture f3). `--no-entry-nodes` suppresses them (generic).
- **Helper tables**: `member_sources_for`, `signature_methods_for`, `container_attributes_for`, `markup_kinds_for`, `visibility_entries_for` and `self_module_markers_for` have no Scala entries. Grep of rules.rs:11620-12123 shows only `java` keys.
- **Registrar verbs**: none (section 03).
- **Not covered**: Spring for Scala, JUnit for Scala, Finatra, ZIO, Cats Effect `IOApp`, Scalatra, Lift, Lagom. There is no Scala rule for any of them, and `junit` exists only for `java` and `kotlin` (rules.rs:7058, 7496).
- **Stale gap prose.** The `play-mvc` gap says "the Scala plugin records no base types and no annotations" (rules.rs:5512). Both have been recorded since 0.6.8 (scala.rs:52-55, 356-360), and fixtures f2/f3 show them. The `akka-scala` gap ("scala carries no base type cgg can constrain them with", rules.rs:5571) is stale for the same reason. The claims about classes, vals and closures in those texts are still accurate.

## 11. Test harness entries

- **No working Scala test harness rule.** `scalatest` and `specs2` are kind `test`, but they have no attributes, methods or registrars (rules.rs:5632-5664). So they mint no `<framework-entry>::test::…` node and root nothing (fixture f3: `nodes_minted` 2, both network).
  - ScalaTest bodies (`test("…") { … }`, `"x" should "y" in { … }`) are class-body statements, which belong to no callable (section 02).
  - Fixture f3: `new Svc().handle(1)` inside `test("handles")` was `no-candidate-in-file`.
  - The curried registration recorded a junk external name `test("handles")` (section 03).
- **Annotation roots.** The dead-code `TEST_ATTRS` list includes `Test` (roots.rs:211-225), so a JUnit-style `@Test def junitStyle()` in Scala becomes a `builtin:test-attr` test root (fixture f3).
- **Naming roots.** The default `is_test_name` applies: a `test_` or `Test` prefix (roots.rs:987). `testSomething` does not match, and the Spec/Suite naming conventions are not used for callables.
- **`TEST_CLASS_MEMBERS_ARE_ROOTS`**: Python, PHP and Ruby only. Scala is not covered.
- **Test-file gating** (section 01): a `src/test` path and the `*Spec.scala` / `*Test.scala` names.
  - Findings in test files, and findings used only by tests, are withheld without `--include-tests`. Fixture f3: `AppSpec::testSomething` and `AppSpec::beforeAll` were not shown.
  - `test_tagging: none` in the capability row: the plugin sets no `TestRole`.
- **Fixture hooks**: `beforeAll`, `afterEach` and `withFixture` are not rooted (no Scala `LIFECYCLE` entry, roots.rs:97-157).

## 12. Dead-code analysis

- **Capability row** (fixtures f1, f3): `class: degraded`, `visibility: none`, `value_references: none`, `dispatch: none`, `exports: none`, `test_tagging: none`, `max_confidence: medium`.
  - `attributes` shows `full` only when the run happens to contain an annotated def, or an `abstract`-tagged declaration (fixture f1, where only `abstract` tags existed). Capability is upgraded from observation (caps.rs:136-178), and the plugin declares nothing. Fixture f6, with no annotations, shows `language-lacks-attributes`.
- **Consequence: no Scala finding can reach `high`.** `signals_complete` needs `Analyzable`, which needs visibility, and the plugin never sets visibility (caps.rs:88-96, 168-175; scala.rs:355). Every Scala finding in fixtures f1, f3 and f6 was `medium`.
- **Roots that fire for Scala:**
  - `builtin:main`: `main` via `UNIVERSAL_ENTRY` (roots.rs:20, 740-757). Fixture f1: `Main::main`, `S::main`.
  - `builtin:lifecycle`: only through the cross-language `CONVENTIONAL_METHODS` (`toString`, `equals`, `hashCode`, `describe`, …; roots.rs:159-167). There is no Scala row in `LIFECYCLE`.
    - Fixture f1: `Shape::describe` was rooted as a lifecycle callback purely by its name.
  - `toplevel:invocation`: the target of a class-body or script-level call with no enclosing callable is rooted by name (fixture f1: `scriptFn`, `Square::compute`). This partly offsets the missing class-body callables.
  - `builtin:test-attr`: see section 11.
  - `framework:entry-node`: `aws-lambda`, `azure-functions`, `gcp-functions` (fixture f3).
  - `user:roots`: generic.
- **Not rooted:**
  - `apply` / `unapply` (fixture f1: `Square::apply` and `Square::unapply` were findings);
  - Scala 3 `given` members (fixture f3: `compare`);
  - `extension` methods (`shoutIt`) and enum members (`label`);
  - operator methods (`Square::+`, `Square::unary_-`);
  - auxiliary constructors (`Square::this`);
  - methods passed as values (`Main::inc`).
- **Evidence kinds seen:** `language-lacks-dispatch-model`, `language-lacks-value-references`, `language-lacks-visibility`, `no-incoming-edges`, `no-unresolved-site-with-name`, `name-matches-unresolved-site` (fixture f1 `Tools::tool`), and `implemented-trait-surface` (fixture f1 `Shape::area`).
- **Confidence caps:**
  - `implements-external-contract`: the CLAUDE.md list (`@Override`/`override`, …) does not name Scala, and Scala has no `override` handling in the plugin. Unverified whether `override def` of an out-of-tree base is capped.
  - `macro-generated`: not applicable.
  - `reflection`: not produced (section 15).
- **Unresolved-doubt handling**: generic. A same-name unresolved Scala site counts as doubt (fixture f1 `Tools::tool`). An unresolved Scala site does not create doubt for a Java callable of that name (fixture f6: `org.v.J.jm` had `no-unresolved-site-with-name`).

## 13. Builtins & stdlib tables

- **`crates/cgg-core/src/stdlib/scala.txt`**: 248 non-comment names (for example `Actor`, `ActorRef`, `apply`, `Array`, `ArrayBuffer`, `asInstanceOf`, `assert`, `Await`, `BigDecimal`, …), loaded at stdlib.rs:202 and 243.
  - Fixtures f1, f2 and f4: `println`, `List`, `Seq`, `Some`, `Option`, `ArrayBuffer`, `map` and `foldLeft` sites went to the `stdlib` bucket.
- **`SCALA_IMPLICIT_PACKAGES`** (stdlib.rs:71-84) and **`SCALA_ROOT_PACKAGES`** (stdlib.rs:96): section 04.
- **`CAST_INTRINSICS`**: `("scala", &["asInstanceOf"])` (stdlib.rs:45). `isInstanceOf` is not listed; it is a test, not a cast.
- **`CONSTRUCTING_METHODS`**: no Scala entry (stdlib.rs:56; Ruby only). Construction by call (`Foo(…)`) is handled structurally (receiver.rs:47-51).
- **`NULL_TYPE_MEMBERS`**: `Null`, `Nothing` (stdlib.rs:136). **`TOP_TYPES`**: `Any`, `AnyRef`, `AnyVal` (stdlib.rs:152).
- **No `rust_types`-like wrapper tables** for Scala (`Future`, `Option`, `Try` and similar are not seen through).
- **`--include-stdlib` / `--include-external`** mint `<stdlib>::println` and `<external>::Class::forName`-style leaves. The external leaf names the receiver text verbatim: fixture f6 printed `<external>::classOf[S.type]::getMethod` and `<external>::'abc'::length`.

## 14. Confidence & certainty

- **`high` was observed for:**
  - intra-file direct and nested-scope calls (fixtures f1, f4, f5);
  - `this.m()` and bare members of the calling type (fixture f2);
  - inherited members reached by a bare call when exactly one ancestor declares them (fixture f2 `greet`, `shout` in `Dog`);
  - object members through an imported object (`Helpers.util1()`, `Other.go()`, `Util.fmt("x")`);
  - a typed parameter whose imported type is anchored by the directory layout (fixture f3 `s.handle`);
  - casts and constructions as receivers (fixture f1);
  - typed closure parameters and typed patterns, including callback-settled closures (fixture f2).
- **`medium` was observed for:**
  - receivers typed through a local `val`, a constructor parameter or a class `val` (fixtures f1, f3);
  - typed parameters whose package is not mirrored by the directory (fixture f2);
  - `super.m()`, which is never high for Scala (cross_file.rs:4693-4699);
  - wildcard-imported object members (fixture f1 `helperFn`) and package-object members (fixture f3 `pkgFn`);
  - every overload binding, even when narrowed to one (fixture f4);
  - two declaring mixins (fixture f2 `shout`);
  - by-name fan-out (fixture f2 `o.get.speak()`).
- **`low`**: inheritance dispatch edges, framework entry edges and descriptor edges.
- **False highs observed:**
  1. A bare call to another same-file object's member: fixture f5 `A::four` → `B::inner` (also CHANGELOG.md:708-709).
  2. A paren-less `b.size` through a local `val b: Box` bound to `B::size` of an unrelated `object B` (fixture f5). The mechanism was not confirmed.
- **Mermaid `?` marker**: present on arrows with no `high` site. Fixture f2 mermaid: `N8 -->|?| N2` (`Dog::common` → `Animal::common` via `super`) and `N14 -->|?2x| N0`.
- **Fan-out cap**: `fanout-cap-exceeded` with the candidate count (fixture f7: 6 candidates over the default cap of 5).

## 15. Dynamic use & reflection

- **None.** `dynuse.rs` has no Scala row (dynuse.rs:30-56 lists Python, Ruby, Java, C#, JS, TS, PHP and Go), and the plugin never calls `dynuse::extract` (grep).
  - Fixture f6: `classOf[S.type].getMethod("hidden")` and `Class.forName("org.v.S")` are plain external calls. `S::hidden` stayed a `medium` finding with no reflection evidence.
- Structural types / `Dynamic` (`selectDynamic`, `applyDynamic`) and `scala.reflect` / runtime-mirror lookups are not modelled (code reading: no handling in scala.rs or dynuse.rs).
- Module loads (`DYN_MODULE_LOADERS`) have no Scala entry (unverified beyond the dynuse table).

## 16. Known gaps & limitations

From code comments, `gap` texts and the CHANGELOG:

- Scala bare calls bound in-file to another class's same-named member (CHANGELOG.md:708-709). Reproduced as a false `high` in fixture f5.
- Scala self-types (CHANGELOG.md:1351).
- Implicit-class extensions and Java interface methods behind typed patterns are unresolved (`candidates-in-other-files`, CHANGELOG.md:1491-1493).
- The framework gap texts for `play-mvc`, `play-module`, `akka-http`, `akka-scala`, `akka-typed`, `http4s`, `spark`, `scalatest` and `specs2` (rules.rs:5499-5664) list:
  - no callable for classes, objects, constructors, `val`s or closures;
  - routes in `conf/routes` are not read;
  - UDFs are registered by string;
  - partial-function routes.
- "Scala 2 has no top-level `def`" (cross_file.rs:4184-4191). This makes Scala 3 top-level defs unreachable cross-file.
- The JVM family is commented out pending tests (lang_family.rs:29-32), so Scala↔Java calls in one tree do not resolve.

Found while cataloguing (fixtures):

- Curried calls record the outer call under a junk name (`curried(1)`, `test("handles")`) (fixtures f3, f4; scala.rs:424-448).
- Infix (`a max b`, `s + q`) and prefix (`-s`) operator calls are not recorded (fixtures f1, f4).
- `Foo(…)` → companion `apply` and case-class construction are not linked. `this(…)` delegation has no target (fixtures f1, f4).
- No value references: eta-expansion and method references do not count (fixtures f1, f4).
- No visibility, so Scala dead code is capped at `medium` (section 12).
- Anonymous-class members collide with the enclosing class's names (fixture f2 `Dog::run` ×2).
- Scala 3 `enum`, `given` and `extension` members have no owner (fixture f3).
- Renamed-import receivers (`{Tools => T}`) are not followed (fixture f1).
- Calls through a local or parameter function value land in the external bucket rather than `shadowed-by-local` (fixtures f2, f3, f4).
- Trait linearization is not modelled; both mixins are bound (fixture f2).
- Paren-less access through a local `val` binds nothing, or binds wrong (fixtures f5, f5b).
- `*Suite.scala` is not a test-file name (testfile.rs:118).
- libscope's `scala_import` does not split multi-clause imports, while heads.rs does (libscope.rs:2313 vs heads.rs:959-960). This is from code reading, not run.
- The stale "records no base types and no annotations" claim in the `play-mvc` and `akka-scala` gap prose (section 10).

Ignored tests: none found for Scala (grep `#[ignore]` near Scala tests: none in scala.rs; not exhaustively checked across the repo, unverified).

## Summary scorecard

| section | support | one-line note |
| --- | --- | --- |
| 01 Detection & parsing | full | `.scala`/`.sc`, `scala` shebang, one grammar handles Scala 2 and 3; `.sbt` not detected |
| 02 Callables extracted | partial | defs, abstract decls, aux ctors, operators, overloads; no class, primary-ctor, `val`-function or lambda callables; Scala 3 enum/given/extension members ownerless; no visibility |
| 03 Call-site forms | partial | bare, member, generic, cast/new receivers, arity, paren-less; curried calls mangled, infix/prefix ops, `apply` sugar and value refs missing |
| 04 Imports & module resolution | partial | selectors, wildcards, renames, implicit packages, external refusal; renamed receivers, Scala 3 top-level defs and Scala→Java unresolved |
| 05 Type inference & receiver typing | partial | params, `T \| Null`, typed lambdas/patterns, callback settling; local/field typing only medium; no implicit conversions |
| 06 Inheritance, dispatch & overloading | partial | bases via imports, inherited bare calls, super, inheritance fan-out, arity overloads; no linearization, no extension methods |
| 07 Scoping & shadowing | partial | locals table, shadowed-by-local, lexical nesting; same-file cross-object bare calls bind falsely at high |
| 08 Macros, preprocessor & codegen | none | no macro/inline/quote handling, no name_mentions |
| 09 FFI & cross-language links | minimal | protobuf rpc → Scala method (low); no JNI confirmation, no Java interop |
| 10 Frameworks & entry points | partial | 12 rules; only aws-lambda, azure-functions, gcp-functions bind; 9 gap-only |
| 11 Test harness entries | minimal | scalatest/specs2 gap-only; only `@Test` attr and file gating |
| 12 Dead-code analysis | minimal | degraded class, max medium; main / toplevel / conventional-name roots only |
| 13 Builtins & stdlib tables | full | scala.txt (248), implicit packages, cast, null and top types |
| 14 Confidence & certainty | partial | high on structural paths; reproducible false highs (f5) |
| 15 Dynamic use & reflection | none | no dynuse table, no reflection doubt |
| 16 Known gaps & limitations | n/a | catalogued above |
