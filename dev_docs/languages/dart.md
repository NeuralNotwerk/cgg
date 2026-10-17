# dart

The Dart plugin is `DartPlugin` in `crates/cgg-lang/src/plugins/dart.rs` (780 lines including inline tests). It parses `.dart` files with the crates.io grammar `tree-sitter-dart` 0.2 (`Cargo.toml:100`, `crates/cgg-lang/Cargo.toml:38`; `tree_sitter_dart::LANGUAGE`, dart.rs:21-23). Nothing is vendored. It is registered at `crates/cgg-lang/src/plugins.rs:140`.

A single recursive `DartWalker` (dart.rs:58-517) keeps two stacks:

- `scope`: class and mixin names, which form the qualified name.
- `bases`: the supertypes of the enclosing class.

The walker records definitions, imports/exports/`show`/`hide`, `part`/`part of`/`library` directives, and every `call_expression`. On every node it also calls the shared `closure_types::record("dart", …)` (dart.rs:79), which records typed closure parameters. After the walk it reads the nearest `pubspec.yaml` and records the package name in `FileFacts::packages` plus a `package-root` import record (dart.rs:41-53, `manifest.rs:65-85`).

What the plugin does not do:

- It ignores its `ExtractCtx` (`_ctx`, dart.rs:27), so it never records registrar arguments or value references.
- It does not override `signals()`, so every `PluginSignals` flag is false (`cgg-lang/src/lib.rs:269-306`).
- It has no `resolver_kind` override.

Dart-specific resolution lives mostly in `cgg-resolve/src/libscope.rs`, the library-scope index that handles imports, exports, parts, `show`/`hide`, prefixes and `package:` URIs. It is called from `cross_file.rs:5424-5445`. There are smaller Dart branches in:

- `cross_file.rs`: `bare_call_reaches` at 4141-4153, `narrow_copies` at 6005-6013, and import aliases at 2029-2058;
- `intra_file.rs`: 132, 446-460 and 576-590;
- `type_hints.rs`: 1210 and 1306;
- `dispatch.rs`: 120 and 363-368.

Fixtures cited below live under `<scratch>/parity/dart/` (f1 … f12). Each was run with `target/release/cgg` 0.11.0 using `-t json -o out.json`, and dumped with `show.py` in that directory.

## 01. Detection & parsing

- **Extension `.dart` only** (dart.rs:15-17). Fixture f1 audit: `detected_via: "extension:.dart"`.
- **Shebangs**: none (dart.rs:18-20 returns `&[]`). A `#!/usr/bin/env dart` script without the `.dart` extension is not detected.
- **Content detection**: none. `cgg-lang::detect` has no Dart branch; grep for `"dart"` in detect.rs finds nothing. `crates/cgg/tests/detect_prefixes.rs:58-64` only builds Dart import prefixes for framework-detection tests.
- **Grammar**: a single grammar, `tree-sitter-dart` 0.2. There is no `ts_language_for` override.
  - Dart 3 syntax parses cleanly (`parse_status: ok`) in fixture f12:
    - `sealed`, `final`, `base` and `interface` class modifiers;
    - record return types `(int, String) f()`;
    - destructuring `final (a, b) = …`;
    - `if (x case …)`;
    - switch expressions.
  - Extension types parse in f3: `extension type Meters(int v)`.
- **Re-parse or fallback**: none. A file with errors is extracted as-is. Test `malformed_source_does_not_panic` (dart.rs:701-705).
- **Depth skip**: shared by every language. A tree deeper than `MAX_TREE_DEPTH` = 4000 is skipped as `SkipReason::TooDeep` (cgg/src/lib.rs:155, 451-456).
- **Minified handling**:
  - Dart has no `max_line_bytes` override (lib.rs:313-315 default `None`).
  - The generic rule still applies: locals are not collected when a line exceeds 2,000 bytes (`MINIFIED_LINE_BYTES`, cgg/src/lib.rs:159).
  - The walker's minified-file heuristic covers only js/mjs/cjs/css (`cgg-walk/src/lib.rs` `MINIFIABLE_EXTS`).
- **Built-in directory deny list hits Dart conventions**:
  - **`bin/` is in `BUILTIN_DENY_DIRS`** (`cgg-walk/src/lib.rs:57`), so Dart's conventional executable directory (`bin/main.dart`) is never analyzed.
    - Fixture f2 audit: `file_skipped ./bin/main.dart {'kind': 'builtin', 'detail': 'bin'}`.
    - The same file copied to `cli/` was analyzed.
  - `build/` is denied too (lib.rs:56); that is correct for Flutter build output.
  - `.dart_tool/` is **not** denied. Fixture f6: `.dart_tool/x/gen.dart` was analyzed and its `toolGen` became a callable. It is normally skipped only when `.gitignore` lists it.
- **Test-file classification** (`cgg-core/src/testfile.rs`):
  - `*_test.dart` (testfile.rs:108);
  - any `test/` directory component (the generic `TEST_DIRS`, testfile.rs:35-45).
  - Flutter's `integration_test/` is not a listed directory name (testfile.rs:35-45). Unverified whether a `*_test.dart` inside it matters: it is caught by the name rule anyway.

## 02. Callables extracted

Qualified names are `<Class>::<name>`, joined with `::` (dart.rs:70-76). Only `class_declaration` and `mixin_declaration` push scope (dart.rs:82-99). Library names and file paths are **not** part of the qualified name. Every definition records `base_types` from the innermost enclosing class (dart.rs:268), `signature_hint` (dart.rs:265), and empty `visibility` and `attributes` (dart.rs:266-267).

Supported shapes:

- **Top-level function** `int topLevel(int a) => a + 1;`. Recorded as `FreeFunction` (`function_declaration`, dart.rs:126-140). Test `a_top_level_function_is_captured`; f1 `topLevel`.
- **Instance method** `void run() {…}` in a class. Recorded as `InherentMethod` with qn `Square::run`, spanning the whole `method_declaration` (dart.rs:141-155). Tests `a_method_is_qualified_by_its_class` and `a_method_spans_its_body_and_is_recorded_once` (dart.rs:579-613).
- **Static method** `static Square build() => …`. Recorded as `Square::build` (f1).
- **Getter / setter in a class** `double get perimeter => …`, `set grow(double v) {}`. Recorded as `Square::perimeter` and `Square::grow` (`member_signature_name`, dart.rs:228-232; f1).
- **Operator** `Square operator +(Square o) => …`. Recorded as `Square::operator+` (dart.rs:240-244; f1).
- **Constructor with a body** `K(this.x) { init(); }`. Recorded as `K::K` (dart.rs:233-239; test `a_method_spans_its_body_and_is_recorded_once` expects `K::K`; f3 `K::K`).
- **Named constructor with a body** `K.named() : x = 0 { … }`. Recorded as `K::named`, the last `name` child (f3).
- **Factory constructor with a body** `factory Square.make(double s) => Square(s);`. Recorded as `Square::make` (f1).
- **Abstract member** `double area();` in a class body. A `declaration` holding a bare `function_signature` is recorded as `TraitMethod` and tagged `abstract` (dart.rs:105-125). Test `abstract_members_are_declarations_and_supertypes_are_bases` (dart.rs:741-761); f1 `Shape::area ['abstract']`; f12 `S::s` in a `sealed class`.
- **Mixin members** `mixin Loud { void shout() {} }`. Recorded as `Loud::shout`; the mixin owns its members (dart.rs:81-82; f1).
- **Class modifiers** `sealed` / `final` / `base` / `interface` class members are owned normally (f12).
- **Top-level `function_signature` / `method_signature` outside a declaration** are recorded as a fallback (dart.rs:156-187).

Not supported, or only partly supported:

- **Bodiless constructors (very common in Dart)** are **not** callables. Examples: `Square(this.side);`, `Square.unit() : side = 1;`, `const K();`.
  - The `declaration` branch only looks for `function_signature` (dart.rs:109-111).
  - Grammar evidence: `constructor_signature` and `constant_constructor_signature` children of `declaration` (tree-sitter-dart 0.2 `node-types.json`).
  - Fixtures: f1 has no `Square::Square` or `Square::unit`; f5 has no `const K()` node.
  - Redirecting factory constructors (`factory A() = B;`) fall in the same path and are therefore not recorded either.
- **Abstract getters, setters and operators** (`int get x;`) are not recorded: the `declaration` branch only matches `function_signature` (dart.rs:109-111).
- **Top-level getters and setters** `int get topGetter => 1;` and `set topSetter(int v) {}` are not recorded. Fixture f3 has no `topGetter` or `topSetter`.
- **Enum members** `enum Color { red; void paint() {} }` are recorded **without an owner** (qn `paint`), because `enum_declaration` does not push scope (dart.rs:82; f1).
- **Extension members** `extension SquareExt on Square { void ext() {} }` have no owner either (qn `ext`) (f1).
- **Extension-type members** `extension type Meters(int v) { int doubled() … }` have no owner (qn `doubled`) (f3).
- **Local functions** `void localFn() { … }` inside a body are **not** recorded.
  - `local_function_declaration` has no `name` field. Its `function_signature` child is then skipped because its parent ends with `declaration` (dart.rs:126-137, 160-166).
  - Fixture f1: no `localFn` callable, and its body's call `topLevel(2)` is attributed to `Holder::use`.
  - `locals.rs:323-329` confirms: "the plugin records no callable for it".
- **Closures and lambdas** `() { … }`, `(x) => …` are not callables. Their calls are attributed to the enclosing callable (f1 L81, f2 test closures → `main`).
- **Fields** are not callables. Test `abstract_members_are_declarations_and_supertypes_are_bases` asserts that `n` is absent.
- **Overloads**: Dart has none, so not applicable.
- **Generated code** (`*.g.dart` parts) is extracted like any file, with no `macro:`/generated tag (f6 `_$genFn`).

## 03. Call-site forms

Only `call_expression` nodes are recorded (dart.rs:197-201, `record_call` dart.rs:433-516). The callee must be an `identifier`, a `member_expression` or a `null_aware_member_expression` (dart.rs:448-503); any other callee is dropped (dart.rs:502).

Supported shapes:

- **Bare call** `topLevel(1)`. Name only (dart.rs:449). Test `extracts_references`; f1 L73 → `topLevel` high.
- **Method call** `svc.run()`. Name `run`, receiver `svc` (dart.rs:451-500). Test `a_method_call_on_a_receiver_is_recorded`.
- **`this.m()` / `super.m()`**:
  - The receiver node is `this`/`super` (dart.rs:464-467).
  - f1 L36 `this.shout()` → `Loud::shout` high.
  - f1 L35 `super.describe()` → `Shape::describe` high.
  - f2 L17 `super.over()` → `Base::over` high.
- **Null-aware call** `x?.m()`, read as `x.m()` (dart.rs:450-451). Integration test `null_conditional_calls_are_recorded_in_dart_and_php` (`crates/cgg/tests/resolve.rs:3196-3210`).
- **Null assertion** `x!.m()`, peeled to `x` (`receiver.rs:69-72`). Test `a_call_in_an_expression_bodied_closure_is_on_the_body_value` (`go` on `z`).
- **Explicit type arguments** `f<int>(1)`, `b.m<String>('a')`. The `instantiation_expression` is unwrapped (dart.rs:439-446). Test `explicit_type_argument_calls_are_recorded`; f1 L83 `generic<int>(1)` → `Holder::generic` high.
- **Static or qualified call** `Util.tool()`, `Square.make(2)`. Receiver is the class name. f2 `Util.tool()` → `Util::tool` high (cross-file); f1 L63 high.
- **Named-constructor call with a body** `K.named()` is recorded as name `named`, receiver `K`. f3 L4 → `K::named` high (cross-file).
- **Chained call** `p.self().go()`. The receiver is the inner `call_expression` (dart.rs:464-467). f10 L17: `Foo::self` high. The outer `go` is not typed by return type: medium on two candidates.
- **Constructor-as-receiver** `Square(2).area()` and `new Square(3).run()`. `receiver::peel` reads a capitalised callee as a construction (`constructs_by_call`, receiver.rs:46-50). f1 L60 → `Square::area` high; L62 → `Square::run` high.
- **Cast receiver** `(s as Square).shout()`. `type_cast_expression` is peeled (receiver.rs:185-189). f1 L59 → `Loud::shout` high (inherited via mixin).
- **`await` receiver** `(await f).m()` is peeled through the unary/await wrapper (receiver.rs:86-100). Unverified by fixture.
- **Expression-bodied closure** `xs.forEach((x) => x.run())`. The grammar reads it as `((x) => x).run()`; `closure_body_receiver` (dart.rs:519-533) makes the receiver `x`. Test `a_call_in_an_expression_bodied_closure_is_on_the_body_value`.
- **Unnamed constructor call** `Square(2)`, `K(1)`. It is recorded as a bare call named `Square`.
  - It binds only when a `K::K` body-constructor exists **in the same file**: f3 L18 `makeK → K::K` high (intra-file).
  - Otherwise it is unresolved or external:
    - f1: `Square` sites are `no-candidate-in-file`, because there is no constructor callable.
    - f3 b.dart L3: `K(1)` is `external-module dart:core`, although `a.dart` declares `K::K`. `libscope` only indexes top-level names (libscope.rs:225-270), and `constructor_names` has no Dart entry (cross_file.rs:383-391).

Not supported:

- **Cascades** `p..run()..shout()` and `k?..run()` are not recorded at all. `cascade_section` is not a `call_expression`. Fixtures f1 L71 and f5 L13-14: no edge, no audit record.
- **`new K.make()` / `const K()` as calls**: `new_expression` and `const_object_expression` are not `call_expression`. f5 L10-11: no record. They are read only as a receiver shape.
- **Call operator** `typed()` where `typed` is an object with `call()`. It is recorded as a bare call `typed`, which goes external (`dart:core`). `call_operator_names` has no Dart entry (cross_file.rs:394-401). f1 L69 is external; `Square::call` stayed unreferenced.
- **Operator invocation** `a + b` is not a call site (f1 L70). `Square::operator+` has no inbound edge.
- **Getter and setter access** `x.perimeter` / `x.grow = 1` are not call sites. Getters and setters are only reachable by name mention (none recorded).
- **Tear-offs and method references** `xs.forEach(take)`, `register(handler)`, `final t = K.make;`: no reference recorded, even with `--reference-edges`. f5: no `Via::Reference` edge to `handler`. f1 L77: `Holder::take` reported unreferenced.
- **Calls through a function-typed variable** `final f = handler; f();` are recorded as bare `f`, which goes external (f5 L17 `<external>::f` with `--include-external`).
- **Registrar arguments**: the `ExtractCtx` is ignored (dart.rs:27).
- **Arity**: `RefRecord::arity` is never set (dart.rs:508-515). Dart is not in `OVERLOAD_ARITY_LANGS` (type_hints.rs:1583).
- **Paren-less calls**: not applicable (getters are not modelled).
- **Index operators** `a[i]` are not recorded.

## 04. Imports & module resolution

Directives recorded (dart.rs:188-196, 273-431):

- `import 'uri' [as p] [show a, b] [hide c];` produces an `import` record with the alias, plus one `show`/`hide` record per combinator.
- `export 'uri' show k;` produces an `export` record (it brings no names into the file's scope) plus combinators.
- `part 'x.dart';` → `part`.
- `part of 'lib.dart';` → `part-of`.
- `part of a.b;` → `part-of-library`.
- `library a;` → `library`.

Test `dart_directives_are_told_apart` (dart.rs:709-739) and `an_aliased_import_keeps_the_alias`.

**Manifest**: `pubspec.yaml` `name:` at top level (`manifest.rs:65-85`). It goes into `FileFacts::packages` and a `package-root` import (dart.rs:41-53).

**Library scope** (`libscope.rs` header 1-15, `resolve_dart` 556-700; CHANGELOG "Dart library scope", lines 583-590):

- **Own library first**: a bare call sees the file's own declarations, which are left to intra-file (libscope.rs:569-577), and then every `part` of its library (libscope.rs:578-593). f2: `partFn ↔ libFn` across `part`/`part of`, both high.
- **Imports**: an unprefixed import brings in the imported library's top-level names and everything it `export`s, transitively up to `MAX_DEPTH` = 16 (libscope.rs:30, 498-554). `show`/`hide` are honoured (`Directive::allows`, libscope.rs:56-59). f2:
  - `greet()` via `export … show greet` → high;
  - `open()` via `export … hide secret` → high;
  - `secret()` and `notShown()` are **not** bound; they are recorded external (see the module-name caveat below).
- **Parts share imports**: a part's imports are its library's (libscope.rs:594-600).
- **Prefixed import** `u.notShown()` / `h.open()` binds only through that prefix (libscope.rs:644-700). f2: `u.notShown` → `notShown` high, and `h.open` → `open` high.
  - A name filtered out by `show`/`hide` is refused as `not-in-imported-module`. f2: `h.secret()` → `not-in-imported-module package:myapp/src/hidden.dart`.
  - `p.Type.m()` (prefix plus member) is not judged here (libscope.rs:666-670).
- **`package:<name>/x.dart`** maps to `<root>/lib/x.dart` of the in-tree pubspec naming `<name>`. When several packages share a name, the one sharing the longest path prefix wins (libscope.rs:385-421). f2: `package:myapp/...` imports bind.
- **External libraries**: `dart:` URIs, packages no in-tree pubspec declares, and other schemes are `Target::External` (libscope.rs:387-389, 392-394, 422-425).
  - A bare name that no import supplies is `External("dart:core")`, because `dart:core` is implicit (libscope.rs:629-634).
  - f2 externals: `Text`, `setState`, `test`, `expect`, `setUp`, `http.get`.
  - Cross-file unit test `dart_bare_call_needs_an_imported_file` (cross_file.rs:13573) and the `expect`/`package:matcher` case (cross_file.rs:12765-12777).
- **Unknown target**: a path into the tree with no analyzed file (generated code, filtered languages) is `Target::Unknown`, which leaves the call to other steps (libscope.rs:40-43, 627).
- **Copies narrowed by import**: `narrow_copies(…, lang == "dart")` only counts candidates in files the caller imports (cross_file.rs:6005-6013; CHANGELOG R16, line 533). f2: two `dup()` in `dup1.dart`/`dup2.dart` bind to the one each caller imports, both high.
- **Prefix alias**: the generic import pass maps the alias to the file stem (cross_file.rs:2029-2058).
- **Unit test** `dart_package_uris_exports_and_combinators` (cross_file.rs:13621).

Caveats and gaps:

- **Wrong module name on the external record**: when a name is filtered out by `show`/`hide`, the external record names the *first external import* rather than the library that hid it. f2: `secret()` and `notShown()` are labelled `external-module dart:io`.
- **A prefixed call to a name that exists nowhere** (`u.missing()`) lands in the external bucket as `no-candidate-in-file` instead of `not-in-imported-module` (f2 L16).
- **`heads.rs` / `modules.rs`** (the `WorkspaceVocab` external-head refusal) have **no Dart branch**; Dart's equivalent is `libscope`. grep finds no `dart` in heads.rs or modules.rs.
- **Conditional imports** `import 'a.dart' if (dart.library.io) 'b.dart'`: unverified. `find_uri_text` takes the first `template_chars_*` found by a stack walk (dart.rs:278-306).
- **Deferred imports** `import … deferred as p` keep the alias through `find_alias` (dart.rs:308-331). Unverified by fixture.

## 05. Type inference & receiver typing

Supported:

- **Parameter types** `void use(Square p) { p.area(); }` are typed from the signature. f1 L54 → `Square::area` high; f10 L16 `p.go()` → `Foo::go` high.
  - Dart named and optional groups are handled: a `{…}` group stops positional reading (type_hints.rs:1210-1213), and `[...]`/`required`/`covariant` are stripped (CHANGELOG line 350; test `dart_named_and_optional_parameter_groups_hold_parameters`, type_hints.rs:3903).
  - Nullable `Foo?` is stripped through `declared_type` (annotation.rs:41-46; test `declared_type("dart", "Foo<T>?")`, annotation.rs:171; test `lang_params("dart", "void f(Foo? a, {required Foo b, Foo? c})")`, type_hints.rs:3467).
- **Typed closure parameters** `(Square y) => y.shout()` are recorded in `scoped_types` (`closure_types.rs:25-37`, 229-247). f1 L76 → `Loud::shout` high; f7 L8 high. Test `dart_typed_closure_parameters_are_scoped_to_the_closure` (closure_types.rs:304).
- **Untyped closure passed to a callback-typed parameter** `void each(void Function(Foo f) cb)` with `each((x) { x.run(); })`. Implemented by `settle_closure_args` (type_hints.rs:1115-1190), `CALLBACK_TYPE_CONSTRUCTORS ("dart", "Function", false)` (stdlib.rs:146), and Dart's named callback parameters (type_hints.rs:1306-1312).
  - f7 (same file) and f8 (cross-file): `x.run()` → `Foo::run` high.
  - Unit test `an_untyped_closure_takes_the_callees_declared_callback_types` (type_hints.rs:3580).
  - **Fails when the type name is a single letter**: f9 `void Function(K k)` left `x.run()` fanned out over `K::run`/`A1::run` at medium, and f3/f4 hit the fan-out cap. The cause is likely the type-parameter heuristic; root cause unverified.
- **Construction and cast receivers** `Square(2).area()`, `new Square(3).run()`, `(s as Square).shout()` (receiver.rs:46-50, 185-189; f1).
- **`is` promotion inside an `if`** `if (p is Foo) { p.go(); }`: f10 L20 was high. The parameter type already was `Foo`, so flow typing itself is not demonstrated (unverified).
- **Top types**: `TOP_TYPES ("dart", &["Object", "Dynamic"])` (stdlib.rs:154). `Object o; o.run()` is not typed (f3 L9 shows `receiver_hint: "Object"`, `candidates-in-other-files`, and a stdlib bucket record).
  - The entry spells `Dynamic`, but Dart's keyword is lowercase `dynamic`. Lowercase names already fail `looks_like_type_name` (cgg-core/facts.rs:914-919), so this is harmless.

Not supported (fixture f10, every case fanned out at medium over `Foo::go` + `Bar::go`):

- **Annotated locals** `Foo typed = Foo(); typed.go();` (L9).
- **`var`/`final` locals from a construction** `var inferred = Foo(); inferred.go();` (L11).
- **Locals from a function's return type** `final fromFn = mk(); fromFn.go();` (L13).
- **Fields** `Foo field = Foo(); … field.go();` (L14), and `late Bar other; other.go();` (L15).
- **Nullable locals** `Bar? nb; nb?.go();` (L19).
- **Method return types in chains** `p.self().go()` (L17).
- Evidence: the Dart plugin writes neither `local_types` nor `class_fields`. grep `local_types` in `cgg-lang/src/plugins/*.rs` lists c_macro, cpp, csharp, go, java, kotlin, python, rust and zig only. Dart is not in `lang_tallies_table!` (type_hints.rs:335-353).
- **Implicit receivers**: not applicable. `settle_implicit_receivers` is Kotlin-only (type_hints.rs:1388).
- **Weak receiver types** (`RefRecord::weak_receiver_type`): never set by Dart (dart.rs:508-515).

## 06. Inheritance, dispatch & overloading

Supported:

- **Base records**: `extends A with M implements I` puts all three on each member's `base_types` (dart.rs:89, 268; `attrs::base_types`, `split_type_list` splits on ` with `, attrs.rs:283-285). Test `abstract_members_are_declarations_and_supertypes_are_bases` asserts `["Shape", "M", "I"]`.
- **Inherited member through a typed receiver**: `(s as Square).shout()` → `Loud::shout` high, via the mixin (f1 L59).
- **Bare call to an inherited member** `inherited()` in `class Child extends Base` → `Base::inherited` high, cross-file (f2 L15).
  - `bare_call_reaches` for Dart: a bare call reaches top-level functions or members of the caller's class and its ancestors only (cross_file.rs:4141-4153).
  - Intra-file: a bare call in a method prefers the enclosing class's member (intra_file.rs:446-460). f11: `C::m` → `C::helper`, not the top-level `helper`.
- **`super.m()`** → base member, high (f1 L35, f2 L17). In the CHANGELOG, Dart mixin `super` was a revert case (CHANGELOG lines 82-83).
- **Abstract declarations**: a call typed to the abstract class binds to the declaration (f1 L58 `s.area()` → `Shape::area` high). Intra-file and duck fan-out prefer a concrete candidate over an `abstract`-tagged one (intra_file.rs:576-590; cross_file.rs:1155).
- **`--dynamic-dispatch` / inheritance fan-out**: Dart is in `INHERITANCE_LANGS` (dispatch.rs:110-123). Under `--dynamic-dispatch`, f1 produced `Shape::area → Square::area` (low, `dispatch:inheritance`).
  - Dart is also in `free_variant_is_member` (dispatch.rs:363-368; test `scala_and_dart_members_recorded_as_free_functions_still_override`, dispatch.rs:767).
- **`one_declaring_ancestor`** has a Dart unit test (cross_file.rs:8394-8430). The `bases_by_owner` "declared in two files" case is at 8443-8457.

Not supported, or partial:

- **Extension methods** `extension SquareExt on Square { void ext() {} }`: there is no extension index (unlike C# step 4d or Kotlin step 0b). `made.ext()` bound by name at medium (f1 L64). The extension's `on` type is not recorded, and its members have no owner (§02).
- **Overloads and arity**: not applicable; Dart has no overloading. `OVERLOAD_ARITY_LANGS` excludes Dart (type_hints.rs:1583).
- **`member_of_calling_class` / `overrides_below`**: these are the Java/C# paths. Dart uses `bare_call_reaches` instead. No override-below handling was found for Dart (unverified beyond grep).
- **Mixin `on` constraints** (`mixin M on Base`): unverified whether `on` types become bases.
- **Interface `implements` of an external type** (`implements Comparable<Square>`) gives no contract evidence (§12).
- **Inheritance edges for mixin/enum/extension members**: enum and extension members have no owner, so they cannot take part.

## 07. Scoping & shadowing

- **`FileFacts::locals`** has a Dart spec, `DART_SPEC` (locals.rs:317-342, selected at 389). It records:
  - binders: formal parameters, `initialized_variable_definition`, `initialized_identifier`, for-loop variables, catch parameters, and local function names (locals.rs:319-330);
  - function scopes: `function_declaration`, `method_declaration`, `local_function_declaration`, `function_expression` (locals.rs:334-339);
  - blocks: `block`, `for_statement`.
  - Test `dart_ocaml_lua_python_rust_kotlin_swift_scala_zig` (locals.rs:840-856): `args`, `cb`, `tester`, `inner`.
- **shadowed-by-local**: `void user(Function greet) { greet(); }`, where another file declares `greet`, is refused as `shadowed-by-local` (f11 L4). The same name in another function binds normally (f11 L9 `other → greet` high).
- **Locals shadow import prefixes**: `heads.locals.shadows_receiver_head(r)` guards the libscope call (cross_file.rs:5428-5430).
- **Lexical narrowing of nested definitions**: Dart is in the intra-file lexical-narrowing list (intra_file.rs:120-136). This matters little, because local functions are not callables (§02).
- **Own library beats imports**: f11 `helper()` in b.dart → b.dart's `helper`, not a.dart's (high).
- **Positional shadowing**: none (OCaml, F# and Lua only, per the CLAUDE.md architecture notes).
- **Conditional definitions**: not applicable (`DefRecord::conditional` is PHP only).
- **Gap**: a call to a local function or a local closure variable (`localFn()`, `cb()`, `h()`) is recorded external `dart:core` rather than `shadowed-by-local` (f1 L80/L82, f11 L7). It produces no edge, and the local function's body is folded into its parent anyway.

## 08. Macros, preprocessor & codegen

- Dart has no preprocessor or macros. Dart 3 macros were cancelled upstream and have no plugin support (no Dart branch in `c_macro.rs` or `rust_macro.rs`).
- **Code generation** (`build_runner`, `*.g.dart`, `*.freezed.dart`): these are ordinary files when present. A `part 'a.g.dart'` resolves through the library scope (f6 `u → _$genFn` high). Nothing tags them as generated (no `macro:`/generated attribute; dart.rs:267).
- When a generated part is absent from the tree, `Target::Unknown` keeps calls into it from being refused (libscope.rs:40-43).
- **`name_mentions`**: never written by the Dart plugin (dart.rs has no `name_mentions`).
- Not applicable: `#define`, `#if`, templates, quote/unquote.

## 09. FFI & cross-language links

- **`ffi::link_ffi`**: no Dart coverage. grep finds no `dart` in `crates/cgg-resolve/src/ffi.rs`. `dart:ffi` (`DynamicLibrary.lookupFunction`, `@Native`) and Flutter platform channels (`MethodChannel.invokeMethod`) produce no cross-language edges.
- **Descriptor links**: there is no Protobuf → Dart host link. grep finds no `dart` in `descriptor.rs`. Generated `*.pb.dart` / `*.pbgrpc.dart` are analyzed as plain Dart.
- **Markup code-behind**: none. `markup.rs` serves C# only.

## 10. Frameworks & entry points

Every rule whose `language` is `"dart"` lives in `crates/cgg-core/src/frameworks/rules.rs:5402-5500`. None names Dart in a multi-language list. **None enumerates entries**: all have empty `attributes`, `registrars`, `base_types` and `methods`, and each carries a `gap` string shown in the coverage table under "seen, no rules".

| rule id | kind | detect (import prefix) | node | what it does |
|---|---|---|---|---|
| `flutter` | Lifecycle | `package:flutter` | false | Detection only. The gap names `build`, `createState`, `initState`, `didChangeDependencies`, `didUpdateWidget`, `deactivate`, `dispose`, `reassemble` as too common to match by name (rules.rs:5402-5418). |
| `flame` | Lifecycle | `package:flame` | false | Detection only. The gap names `onLoad`, `onMount`, `onRemove`, `onGameResize`, `onParentResize`, `onChildrenChanged`, `update`, `render` (rules.rs:5419-5435). |
| `dart-main` | Cli | `dart`, `package:flutter`, `package:flame` | false | Detection only. Any `dart:` import triggers it; f1 shows "dart-main — found in 1 file(s), entries NOT enumerated" (rules.rs:5436-5452). |
| `flutter-test` | Test | `package:flutter_test`, `package:test`, `package:test_api`, `package:flame_test`, `package:integration_test` | true | No entries. The gap explains that test bodies are closures and the plugin records no value or closure arguments (rules.rs:5453-5475). |
| `shelf` | Network | `package:shelf`, `package:shelf_router`, `package:shelf_static`, `package:shelf_io` | true | No entries. Handlers are values passed to `Router()..get(...)`, which the plugin cannot see (rules.rs:5476-5498). |

- Fixture f2 coverage output: `recognised (none)`; `seen, no rules: dart-main, flutter, flutter-test`.
- `member_sources_for`, `signature_methods_for`, `container_attributes_for`, `markup_kinds_for` and `visibility_entries_for` have no Dart entries (grep in rules.rs finds `dart` only at 5404-5478).
- **Registrar verbs**: none, because the plugin ignores `ExtractCtx`.
- **Framework-entry nodes**: none are ever minted for Dart (f2 has no `<framework-entry>` callables).
- Benchmark corpus: `dart-flame` (scripts/benchmark.sh:77), and `APPS` entries `app-dartpad-shelf`, `dart-flame` and `dart-flutter` (benchmark.sh:301, 360-361). The README benchmark row reads `flame | dart | 1,591 | 9 | 0%` (README.md:1329). The 9 edges predate the method-span fix (CHANGELOG lines 819-820), so the row is stale per the user's memory note on README staleness; unverified against a fresh run.

## 11. Test harness entries

- **`flutter-test`** (kind `Test`, `node: true`) is detected by import but enumerates nothing (rules.rs:5453-5475). No `<framework-entry>::test::flutter-test` node is minted (f2).
- **Test cases** `test('x', () { … })`, `testWidgets(…)`, `group(…)`, `setUp(…)` are recorded only as calls to external names. f2 audit: `test` / `expect` / `setUp` are external; `group` is in the stdlib bucket because `group` is listed in `stdlib/dart.txt`.
  - The closures' calls are attributed to the file's `main()` (f2: `main → greet`, `main → helperOnlyFromTest`, both high).
  - So a helper called only from a test body is **reached from `main`** rather than "reported as dead" as the rule's gap text says. The gap text is inaccurate for that case.
- **Test-file gating**: `_test.dart` and the `test/` directory classify a file as test code (§01). Dead-code hides its findings unless `--include-tests` is set.
- **Test roots**: a test file's `main` is rooted as `program-entry builtin:main`, not as a test root (f2 dead-code roots).
- **`TEST_CLASS_MEMBERS_ARE_ROOTS`**: not applicable (Python/PHP/Ruby).
- No annotation-based (`@isTest`) or naming-pattern test rule exists for Dart.

## 12. Dead-code analysis

- **Capabilities** (f1/f2 `--dead-code-format json`): `class: degraded`, with `visibility`, `value_references`, `dispatch`, `exports` and `test_tagging` all `none`.
  - `attributes` is measured at runtime: it is `full` when an `abstract` tag exists (f1) and `none` otherwise (f2).
  - **`max_confidence: medium`**: every Dart finding is capped at medium, so the default high threshold shows **zero** Dart findings (f1: "0 shown (high) · 6 withheld (medium) · 1 withheld (low)").
- **Roots**:
  - `builtin:main` program entry for every `main` (f1, f2), including test files' `main`.
  - `builtin:lifecycle` from `CONVENTIONAL_METHODS` (`initialize`, `to_s`, `toString`, `equals`, `hashCode`, `Dispose`, `describe`; roots.rs:101-111), which applies to every language. f1 rooted `Shape::describe`.
  - Dart has **no `LIFECYCLE` entry** (roots.rs:96-150), so Flutter `build`/`initState`/`dispose` and Flame `onLoad`/`update` are not roots. f2: `MyWidget::build` was reported `never-referenced` (medium).
- **Override markers**: `OVERRIDE_MARKERS` includes Dart's `@override` (deadcode/mod.rs:405-409). However, the Dart plugin never records attributes (dart.rs:267), so `implements-external-contract` never fires for Dart. f2: `MyWidget::build` (`@override` of the external `StatelessWidget`) carries no contract evidence.
- **Evidence kinds** seen on Dart findings: `language-lacks-dispatch-model`, `language-lacks-value-references`, `language-lacks-visibility`, `language-lacks-attributes`, `no-incoming-edges`, `name-matches-unresolved-site` (f1 `compareTo` → low), `implemented-trait-surface` (f2 `Base::over`), `incoming-only-from-dead-code`.
- **Public API**: none. Dart has no visibility signal (`_private` naming is not read), and `export` directives are not turned into `FileFacts::exports`. Library code under `lib/` that nothing in the tree calls is reported (f2 `secret`, `neverCalled`).
- **Unresolved doubt quirk**: f1 `paint` (`Color.red.paint()` unresolved in the same file) shows `no-unresolved-site-with-name`. Likewise f2 `secret` despite the refused `h.secret()` site. Root cause unverified.
- **Unreachable code / test tagging / dynamic-use signals**: none (all `PluginSignals` false).

## 13. Builtins & stdlib tables

- **`crates/cgg-core/src/stdlib/dart.txt`**: 394 names (e.g. `abs`, `add`, `addAll`, `ArgumentError`, `base64`, `BigInt`, `print`, `group`, `writeln`, `exit`, `parse`), loaded at stdlib.rs:201 and 242.
  - f2/f3: `print`, `exit`, `stdout.writeln`, `Uri.parse` and `group` land in the stdlib bucket.
  - `group` (package:test) is in the list, so it is mis-bucketed as stdlib.
- **`CALLBACK_TYPE_CONSTRUCTORS`**: `("dart", "Function", false)` (stdlib.rs:146).
- **`TOP_TYPES`**: `("dart", &["Object", "Dynamic"])` (stdlib.rs:154).
- **`CAST_INTRINSICS`, `CONSTRUCTING_METHODS`, `MAYBE_TYPE_WRAPPERS`, `UNION_TYPE_CONSTRUCTORS`, `NULL_TYPE_MEMBERS`**: no Dart entries (stdlib.rs:35-139). Nullability comes from the generic `?` suffix in `declared_type`.
- **Implicit packages**: `dart:core` is treated as an implicit import in `libscope.rs:629-634`; it is not a table.
- **No `rust_types`-like wrapper tables** exist for Dart (`Future<T>`, `FutureOr<T>` and `Stream<T>` are not unwrapped; unverified beyond grep).
- **`--include-stdlib`**: f5 minted `<stdlib>::print` (low, `stdlib exit-node`). **`--include-external`**: `<external>::f` (f5).

## 14. Confidence & certainty

`high` is produced by:

- intra-file single candidates (f1, f3, f12);
- libscope bindings to exactly one library namespace entry (`certain: true`, libscope.rs:589-592, 635-638, 686-689; `module_certain`, cross_file.rs:5435);
- parameter-typed receivers (f1 L54, f10 L16);
- construction and cast receivers (f1 L59-62);
- typed closure parameters (f1 L76, f7);
- `super`/`this` and inherited bare calls (f1 L35-37, f2 L15-17);
- static or qualified class calls (f2 `Util.tool`).

`medium` is produced by:

- by-name fan-out on untyped receivers (every local or field receiver; f10);
- several namespaces providing a name (libscope.rs:639-642, 690-693: `certain: false`);
- field receivers (f1 L55-57);
- extension calls (f1 L64).

`low` appears only on dynamic-dispatch edges (f1 `--dynamic-dispatch`) and exit nodes.

Mermaid marks a medium edge with a leading `?` (f3: `N13 -->|?2x| N2`, `N13 -->|?| N10`).

## 15. Dynamic use & reflection

- **No `dynuse` coverage**: `dyn_uses` is false, and `dynuse.rs` has no Dart branch (grep). `dart:mirrors` (`reflect(o).invoke(#name)`), `noSuchMethod`, `Function.apply` and `dynamic`-typed calls are not screened.
- A `dynamic` receiver `d.run()` is treated as untyped and fanned out by name, refused over the cap (f3 L8 `fanout-cap-exceeded`, 7 candidates).
- **Module-load** (`DYN_MODULE_LOADERS`): no Dart entry. Deferred `loadLibrary()` is not modelled.

## 16. Known gaps & limitations

- **`bin/` is skipped wholesale** (`BUILTIN_DENY_DIRS`, cgg-walk/src/lib.rs:57). Every Dart CLI entry point is invisible (f2).
- **No constructor callables for bodiless constructors**, and no constructor binding cross-file: `Square(1)` from another file is `external-module dart:core` (f1, f3). `constructor_names` has no Dart entry (cross_file.rs:383-391).
- **Local variable and field types are never inferred**: `Foo x = …`, `var x = Foo()`, `final x = f()` and fields all fan out by name at medium (f10). Only parameters, casts, constructions and closure parameters are typed.
- **Cascades (`..m()`), `new`/`const` constructions, operators, call-operator objects and getters/setters are not call sites** (f1, f5).
- **No value references or tear-offs** (`forEach(take)`, `register(handler)`), so shelf handlers and test closures cannot be linked. This is stated in the `flutter-test` and `shelf` gap text (rules.rs:5472, 5494).
- **Enum, extension and extension-type members have no owner**; top-level getters/setters, abstract getters and local functions are not callables (f1, f3).
- **No `@override`/attribute recording**, so the dead-code report has no external-contract cap. No Flutter/Flame lifecycle roots exist, and Dart findings are capped at medium (§12).
- **Framework rules are detection-only**: `flutter`, `flame`, `dart-main`, `flutter-test` and `shelf` each carry a `gap_for` text (rules.rs:5415, 5432, 5449, 5472, 5494; `gap_for`, rules.rs:11876).
- **Single-letter type names** defeat the callback closure typing (f9).
- **External-module labelling** for `show`/`hide`-filtered names cites an unrelated import (f2 `dart:io`).
- **`.dart_tool/` is not denied** (f6).
- **No ignored tests** mention Dart (grep `#[ignore` with dart finds nothing).
- **Stale README benchmark row** for flame (README.md:1329, 9 edges); unverified.

## Summary scorecard

| section | support | note |
|---|---|---|
| 01 Detection & parsing | partial | `.dart` only, Dart 3 syntax parses; `bin/` dir is deny-listed, `.dart_tool/` is not |
| 02 Callables extracted | partial | methods/getters/operators/body-ctors ok; bodiless ctors, enum/extension owners, top-level accessors, local fns missing |
| 03 Call-site forms | partial | bare/member/`?.`/`!`/generic/static ok; cascades, `new`/`const`, operators, call operator, tear-offs missing |
| 04 Imports & module resolution | full | library scope with parts, exports, show/hide, prefixes, `package:` via pubspec; minor external-label quirk |
| 05 Type inference & receiver typing | partial | params, casts, constructions, closure params; no local/field/return-type typing |
| 06 Inheritance, dispatch & overloading | partial | bases incl. mixins, super, inherited bare calls, INHERITANCE_LANGS; no extension-method index |
| 07 Scoping & shadowing | partial | DART_SPEC locals and shadowed-by-local; local fns/closure vars go external |
| 08 Macros, preprocessor & codegen | n/a | no macros; `.g.dart` parts resolve but are untagged |
| 09 FFI & cross-language links | none | no dart:ffi, platform channels or protobuf→Dart links |
| 10 Frameworks & entry points | minimal | 5 rules (flutter, flame, dart-main, flutter-test, shelf), all detection-only gaps |
| 11 Test harness entries | minimal | flutter-test detected, no entries; closures folded into `main` |
| 12 Dead-code analysis | minimal | degraded class, max medium, no lifecycle roots, `@override` unread |
| 13 Builtins & stdlib tables | partial | dart.txt 394 names, Function callback, TOP_TYPES; no wrapper tables |
| 14 Confidence & certainty | partial | high via libscope/params/casts; most local receivers stuck at medium |
| 15 Dynamic use & reflection | none | no dynuse, mirrors or noSuchMethod handling |
| 16 Known gaps & limitations | n/a | see list: bin/ skip, ctors, locals typing, cascades, value refs |
