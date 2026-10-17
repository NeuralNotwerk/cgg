# swift

The Swift plugin is `SwiftPlugin` in `crates/cgg-lang/src/plugins/swift.rs` (464 lines including 7 inline tests). It parses `.swift` files with the crates.io grammar `tree-sitter-swift` 0.7 (`Cargo.toml:97`, `crates/cgg-lang/Cargo.toml:35`; `Cargo.lock:1775-1776` pins 0.7.2; `tree_sitter_swift::LANGUAGE`, swift.rs:18-20). Nothing is vendored. It is registered at `crates/cgg-lang/src/plugins.rs:137`.

The plugin is a single recursive `SwiftWalker` (swift.rs:42-288) with two stacks:

- `scope`: type names, which form the qualified name;
- `bases`: the supertypes of the enclosing type.

It records only five node kinds:

- type declarations: `class_declaration`, `struct_declaration`, `enum_declaration`, `protocol_declaration`, which push scope and bases (swift.rs:65-83);
- `protocol_function_declaration` (swift.rs:87-92);
- `function_declaration` (swift.rs:93-97);
- `init_declaration` (swift.rs:98-119);
- `import_declaration` (swift.rs:120-123);
- `call_expression` (swift.rs:124-128).

What the plugin does not do:

- It overrides only `id`, `extensions`, `ts_language` and `extract`. There is no `signals()`, `shebangs()`, `max_line_bytes()` or `ts_language_for()`, so every `PluginSignals` bit is false (defaults in `crates/cgg-lang/src/lib.rs:268-310`).
- It ignores its `ExtractCtx` (`_ctx`, swift.rs:24), so it never records registrar arguments or value references.
- The `LanguagePlugin` trait has no `resolver_kind` method at this head: `grep resolver_kind` finds nothing in `crates/`, so the CLAUDE.md wording is stale. Resolution runs the generic pipeline.

Swift appears in these shared resolver lists:

- `lexically_scoped` (intra_file.rs:119-136);
- `binds_same_name_set` (intra_file.rs:835-848);
- `INHERITANCE_LANGS` (dispatch.rs:110-123);
- the plain `import` alias handling in cross_file.rs:1679-1735;
- `locals.rs` `SWIFT_SPEC` (locals.rs:264-282, 387);
- `receiver.rs`: constructs-by-call, tuple-parens, `!`, and `as`;
- `stdlib.rs` `MAYBE_TYPE_WRAPPERS` and `stdlib/swift.txt`;
- `testfile.rs:107`;
- the deadcode `LIFECYCLE` table (roots.rs:148-155);
- 9 framework rules.

Swift is **absent** from these:

- `heads.rs` external-head refusal (heads.rs:660-671);
- `modules.rs` (`modules.rs:190`);
- `libscope.rs`;
- `OVERLOAD_ARITY_LANGS` (type_hints.rs:1583);
- `constructor_names` / `call_operator_names` (cross_file.rs:383-401);
- `CAST_INTRINSICS`, `CONSTRUCTING_METHODS`, `NULL_TYPE_MEMBERS`, `CALLBACK_TYPE_CONSTRUCTORS`, `TOP_TYPES` (stdlib.rs:35-155);
- `TEST_CLASS_MEMBERS_ARE_ROOTS` (rules.rs:11806);
- every dynuse, FFI and descriptor table.

The fixtures cited below live under `<scratch>/parity/swift/` (f1 … f14):

- Each was run with `target/release/cgg` 0.11.0 and `-t json -o out.json`, and dumped with `show.py` in that directory.
- `f3` and `f9` were also run with `--dead-code --dead-code-confidence low`.

## 01. Detection & parsing

- **Extension `.swift` only** (swift.rs:15-17).
  - Fixture f1 audit: `detected_via: "extension:.swift"`.
  - `.swiftinterface` is not claimed.
- **Shebangs**: none. `shebangs()` is not overridden, so the default `&[]` applies (cgg-lang/src/lib.rs:298-300). A `#!/usr/bin/env swift` script without the `.swift` extension is not detected.
- **Content detection**: none for Swift. `crates/cgg/tests/detect_prefixes.rs:51` builds `import {prefix}` in a `.swift` file only to exercise framework detection prefixes.
- **Grammar**: one grammar, `tree-sitter-swift` 0.7.2. No vendoring, and no `ts_language_for` override.
- **Re-parse or fallback**: none. A file with syntax errors is extracted anyway and marked `parse_status = "error"` (generic, `crates/cgg/src/lib.rs:457-461`). All Swift fixtures here parsed `ok`, including f8 with `#if/#else/#endif`, property observers, subscripts, operators and an `indirect enum`.
- **Grammar quirks seen**:
  - `Wrap<Int>.s()` in expression position parses as two comparisons around an implicit-member call. The plugin repairs it (`misparsed_generic_member_call`, swift.rs:290-329). Tests: `generic_type_static_call_misparsed_as_comparison` (swift.rs:437) and the near-miss test `comparisons_and_implicit_members_are_not_generic_calls` (swift.rs:452).
  - `l.count() + r.count()` inside a `switch` case: the second call's receiver was recorded as the text `l.count() + r`, so the grammar's navigation target spans the additive expression. The site was left unresolved (f8, line 31, `receiver_hint: "l.count() + r"`).
- **Package.swift**: parsed as an ordinary Swift file (f13 `./Package.swift` analyzed). It is not read as a manifest (§04).
- **Depth skip**: shared by every language. A tree deeper than `MAX_TREE_DEPTH` (4,000) is skipped as `SkipReason::TooDeep` (`crates/cgg/src/lib.rs:451-456`).
- **Minified or long lines**: there is no `max_line_bytes` override, so long lines are never skipped. Locals are not collected when any line exceeds 2,000 bytes (`MINIFIED_LINE_BYTES`, `crates/cgg/src/lib.rs:159, 480-485`).
- **Test-file classification** (`crates/cgg-core/src/testfile.rs`):
  - By name: `*Tests.swift` and `*Test.swift` (testfile.rs:107).
  - By directory: any path component `Tests`, `Test`, `tests`, `test`, `spec`, … (TEST_DIRS, testfile.rs:36-46, 129). This covers the SwiftPM `Tests/<Target>Tests/` layout.
  - `*Spec.swift` (Quick/Nimble) is **not** a name rule; it is caught only under a test directory.

## 02. Callables extracted

Qualified names are the enclosing type names joined with `.` (`qn`, swift.rs:55-61). No module or target name is ever part of a QN. An `extension X` is parsed by the grammar as a `class_declaration`, so its members are named `X.m`.

Supported shapes:

- **Free function** `func greet(_ name: String) {}` → `FreeFunction` `greet` (swift.rs:159-160). Test `free_function` (swift.rs:387). Fixture f1: `topLevel function`.
- **Instance method** of a class, struct, enum or actor: `class Service { func run() {} }` → `InherentMethod` `Service.run` (swift.rs:161-162). Test `class_methods` (swift.rs:373).
  - Fixture f1: `Point.move` (a `mutating func`), `Dir.flip` (enum), `Counter.inc` (`actor`).
- **Static method** `static func make()` → `StaticMethod` (swift.rs:154-158). The check is `modifiers` text containing `static`.
  - **Partial**: `class func klass()` is not detected as static and becomes `InherentMethod` (f1: `Square.klass` kind `method`).
- **Initializers** `init(...)`, `convenience init()`, failable `init?` → `Constructor` named `<Type>.init` (swift.rs:98-119). Fixture f1: two `Square.init` nodes (L18, L19).
  - A protocol `init(x: Int)` requirement is also a `Constructor` `P.init`, **without** the `abstract` tag (f12).
- **Operator functions** `static func + (l:, r:)` → `StaticMethod` named `+` (f1: `Square.+`). `static func ==` → `Widget.==` (f8).
- **Generic functions** `func generic<T>(_ x: T)` and `where`-constrained ones → ordinary nodes (f1 `generic`, f8 `generic`).
- **Overloads** (same QN, different labels or arity): each is a separate node with the same QN (f1: `Square.overloaded` ×2; f5: `Over.f` ×2, `Over.g` ×2).
- **Nested types** `struct Outer { struct Inner { func a() } }` → `Outer.Inner.a` (f12). `extension Outer.Inner { func b() }` → `Outer.Inner.b`, because the extension's `name` text is used whole (f12).
- **Extensions**:
  - `extension Square { func extra() }` → `Square.extra`, also when it sits in another file (f1 b.swift: `Square.makeIt`).
  - `extension Int { func doubled() }` → `Int.doubled`.
  - `extension Array where Element: Equatable { func uniq() }` → `Array.uniq` (f12).
- **Protocol requirements** `protocol Shape { func area() -> Double }` → `TraitMethod` tagged `abstract`, owned by the protocol (`record_requirement`, swift.rs:185-213). Test `protocol_requirements_are_declarations_and_conformances_are_bases` (swift.rs:416). Fixture f1: `Shape.area attrs=['abstract']`.
- **Protocol extension default implementations** `extension Runner { func run() { prep() } }` → `Runner.run`, a concrete `InherentMethod` beside the `abstract` `Runner.run` requirement (f4).
- **Nested (local) functions** `func topLevel() { func nested() {} }` → a node whose QN **does not include the outer function** (`nested`; f1, f12). A same-named top-level function gets the same QN (f12: two `inner` nodes). Lexical narrowing still picks the right one (§07).
- **Conditional compilation**: both branches of `#if … #else … #endif` are extracted (f8: two `platform` nodes, L6 and L8).
- **Base types**: every `function_declaration` records the enclosing type's supertypes and conformances in `DefRecord::base_types` (swift.rs:180). Generic arguments and module qualifiers are stripped (`swift_bases`, swift.rs:333-350).
  - **Not** recorded on `init` defs (swift.rs:104-116).
  - A type with no methods records no bases anywhere (§06).
- **Visibility**: never recorded. `visibility: String::new()` (swift.rs:113, 178, 206). JSON `vis=None` for `public`, `private` and `internal` alike (f3: `publicApi`, `privateDead`).
- **Attributes**: never recorded. `attributes: Vec::new()` (swift.rs:114, 179); the only tag ever written is `abstract` (swift.rs:209). `@objc`, `@IBAction`, `@main`, `@Test`, `@MainActor`, `override`, `@_cdecl` are all invisible (f3: `VC.tapped`, `VC.pressed`, `MainApp.main` all `attrs=`).
- **Signature hint**: `extract_signature` of the node text (swift.rs:112, 177, 205). This is the text the resolver reads for parameter types and return types.

Not supported:

- **`deinit`**: no node. Calls inside it have no enclosing callable (f1: `cleanup` at L20 → `no-enclosing-callable`).
- **Computed properties and accessors** (`var x: T { … }`, `get`/`set`, `willSet`/`didSet`, `lazy var … = { … }()`): no node. Calls inside them → `no-enclosing-callable` (f1 L16/L17; f8 L14/L15).
  - This is why SwiftUI `var body: some View` has no callable (rules.rs `swiftui` gap).
- **Subscripts** `subscript(i: Int) -> Int { … }`: no node, in classes or in protocols (f1, f8 L21 `lookup` → `no-enclosing-callable`; f12).
- **Protocol property and subscript requirements** (`var v: Int { get }`): not extracted (f12).
- **Closures**: never a node. A call inside a closure is attributed to the enclosing function (f1 L72: `{ (x: Int) in topLevel() }` → `topLevel -> topLevel`; f4 L66: `value { request() }` → `passes -> request`).
- **Top-level code** (`main.swift` statements): no synthetic module callable. Calls there → `no-enclosing-callable` (f8 L2-4). They do root their target by name in dead-code (§12).
- **Macro declarations** (Swift 5.9 `macro`), `typealias`, enum `case` constructors: not extracted.

## 03. Call-site forms

Every `call_expression` is passed to `record_call` (swift.rs:233-287). The callee is `child(0)`. Only three callee shapes are recorded, and anything else returns with nothing (swift.rs:274).

Supported:

- **Bare call** `greet("x")` (`simple_identifier`) → `RefRecord{name, receiver_hint: ""}` (swift.rs:239). Test `call_expressions` (swift.rs:405).
- **Member call** `obj.run()`, `self.honk()`, `Self.factory()`, `super.draw()`, `Engine.shared()` (`navigation_expression`):
  - `name` is the `navigation_suffix` identifier;
  - `receiver_hint` is the target text after `receiver::peel` (swift.rs:240-267).
  - f4: `self.honk()` → `Car.honk` high; `Self.factory()` → `Car.factory` high. f1: `super.draw()` → `Base.draw`.
- **Chained calls** `s.makeIt().area()`: each `call_expression` is recorded, and the inner call becomes the outer receiver text. f1 L90 bound both `Square.makeIt` and `Square.area`, at medium.
- **Peeled receivers** (`crate::receiver::peel("swift", …)`, swift.rs:247), giving a typed receiver (`RefRecord::typed_receiver`, swift.rs:284):
  - `(o as! Foo).m()` and `(o as? Foo).m()` → type `Foo` (`as_expression` `name` field, receiver.rs:179-184). f1 L77: `(s as Base).draw()` → `Base.draw` high.
  - `Foo(…).m()` → type `Foo`, because Swift constructs by call (receiver.rs:46-51).
  - `(x).m()`: Swift spells parentheses as a one-element `tuple_expression` (receiver.rs:63-64).
  - `x!.m()`: `postfix_expression` with a `bang` operation (receiver.rs:75-86).
  - `try`/`await`: `try_expression` and `await_expression` (receiver.rs:65-67).
- **Static call on a generic type** `Wrap<Int>.s()`: repaired from the comparison misparse into `("s", "Wrap")` (swift.rs:268-273, 297-329). f1 L83 → `Wrap.s` high.
- **Trailing-closure calls** `value { … }`, `Task { … }`, `DispatchQueue.main.async { … }`: recorded as calls of the callee. f4 L66: `passes -> value` high. The closure body is walked as part of the caller.
- **Optional-chained receiver on a name** `sh?.area()`, `s?.area()`, `w?.start()`: recorded with the variable as receiver (f1 L3, L85; f4 L59).

Partially supported or not supported:

- **Constructor calls** `Square(side: 3)`, `Engine()`: recorded as a **bare call named after the type**, but never bound to `<Type>.init`. Swift is missing from `constructor_names` (cross_file.rs:383-391), so these end as `no-candidate-in-file` (f1 L23/L61/L64/L67; f4 L9/L18/L35; f10).
  - `Engine.Inner()` → `name: Inner, receiver: Engine`, `no-candidate-in-file` (f4 L44).
  - `Widget.init().render()`: receiver text `Widget.init()`, untyped (f8 L12).
- **Implicit member calls** `.shared()` / `.make()`: **not recorded**. A `prefix_expression` callee that is not the generic misparse returns `None` (swift.rs:269-273). Test `comparisons_and_implicit_members_are_not_generic_calls` asserts `.make()` yields no reference. f4 `implicitMember()` has no out-edge.
- **Optional-chained cast receiver** `(any as? Motor)?.start()`: the receiver was recorded as the literal text `(`, unresolved `candidates-in-other-files` (f4 L55).
- **Operators** `a == b`, `a + b` and **subscript use** `a["k"]`: no reference is recorded, so the `Widget.==` and subscript bodies are unreachable (f8 `ops` has no out-edges).
- **Function values and method references**: `value(cb: request)`, `[1,2].map(transform)`, `let f = request; f()`:
  - No value reference is recorded. The plugin ignores `ExtractCtx::reference_sites` (swift.rs:24), and `--reference-edges` produced no `reference` edge (f4 `ref.json`: `[]`).
  - `transform` is reported dead (f1 dead-code run).
  - `f()` and `cb()` calls are recorded as bare calls and land in the unattributed bucket (f4).
- **`#selector(VC.tapped)`, key paths `\.name`, `NSSelectorFromString("…")`**: not references (f3: `VC.tapped` reported dead).
- **Registrar arguments**: never captured (`_ctx` unused, swift.rs:24). The `aws-lambda` swift rule's `registrars: &["run"]` therefore never matches (§10).
- **Arity**: `RefRecord::arity` is never set (swift.rs:279-286 sets only name, receiver, site and `typed_receiver`). Swift is not in `OVERLOAD_ARITY_LANGS` (type_hints.rs:1583).
- **Argument labels**: not recorded, so `f(a:)` and `f(b:)` overloads cannot be told apart at the call site (§06).
- **`paren_less`, `implicit_receiver`, `path_receiver`, `invocant`, `arg_type`, `not_def`**: never set by the Swift plugin.

## 04. Imports & module resolution

- **`import Module`** → `ImportRecord{kind: "import", path, alias: ""}`. The path is the text of the first `identifier` child (swift.rs:215-231). Test `import_captured` (swift.rs:398).
- **`@testable import MyLib`**: recorded the same way as a plain import (f4; no distinct handling in swift.rs).
- **`import struct Alamofire.Session` / `import class UIKit.UIView`**: the path is the dotted text (`Alamofire.Session`). The kind keyword is dropped. In f4 the audit's `unresolved_by_module` attributed `request` to module `Alamofire.Session`.
- **Alias handling in the resolver**: Swift is in the plain `import` branch with Python, Go, JS/TS, Zig, R and Perl (cross_file.rs:1679-1735).
  - A dotted path binds its first segment and targets the full path.
  - A bare `Foundation` binds and targets itself.
  - Swift has no `import X as Y`, so `alias` is always empty.
- **Module-qualified call** `Helpers.util()`: step 2 rebuilds `Helpers.util`. No Swift QN carries a module (§02), so that lookup cannot match. In f13 the site stayed unresolved (`receiver_hint: Helpers`, `candidates-in-other-files`, 2).
- **Bare call narrowed by import directory**: `import Helpers` + `util()` bound to `Sources/Helpers/H.swift`'s `util`, not `Sources/Other/O.swift`'s, at medium.
  - Mechanism: the import's lowercased path fragment matches the directory (step 1c `module_files` / `path_fragments`, cross_file.rs:2170-2215).
  - Fixture f13.
- **Cross-file bare calls need an import**. Swift has no explicit intra-module imports, but the step 1d global fallback runs only when the calling file has at least one import (`has_imports`, cross_file.rs:6015-6020) **and** the name has at most 3 candidates (cross_file.rs:6025).
  - f2: `b.swift` (no imports) calling `helper()` from `a.swift` → **unresolved** `candidates-in-other-files`.
  - f2: `c.swift` (with `import Foundation`) calling `other()` → bound, medium.
  - f1: `crossCaller()` → `topLevel` unresolved, because `b.swift` has no import.
- **`FileFacts::packages`**: never written by the Swift plugin. There is no module or target record.
- **Manifest reading**: none. `Package.swift` is not read through `plugins/manifest.rs`; SwiftPM targets and products are unknown (f13).
- **External-module refusal (`heads.rs`)**: Swift is not in `external_head`'s language list (heads.rs:660-671). No call is filed as `external-module` before name matching. In practice:
  - `AF.request("x")` and `Alamofire.request("y")` were **not** name-matched onto the workspace `Api.request`. The capitalised receiver is treated as a type owner that does not exist, and the sites end as `candidates-in-other-files` (f5).
  - The audit's `unresolved_by_module` still groups them under `Alamofire` (f5) and `Alamofire.Session` (f4).
- **`not-in-imported-module` (`modules.rs`)**: not applicable. `modules.rs:190` covers only JS/TS/Python/Go/Lua.
- **`libscope.rs`**: no Swift branch.
- **Module or target visibility** (`internal` vs `public` across targets): not modelled. Listed as "Not covered (WS14): … Swift module/target visibility" (CHANGELOG.md:1351).

## 05. Type inference & receiver typing

Swift appears in the type-hint tally list (type_hints.rs:340-354). The plugin writes no `local_types`, `class_fields`, `scoped_types` or `type_spans`, so typing comes only from signature text and receiver shapes.

Supported:

- **Parameter annotations** `func f(m: Motor) { m.start() }`: Strategy 1 types the receiver from the enclosing def's signature (type_hints.rs:686-700, via `collect_param_types`). f4 L51: `m.start()` → `Motor.start` high.
  - **Nullability is stripped through `cgg_core::declared_type`**: `Foo?`, `Foo!` and `Optional<Foo>` all read as `Foo`.
    - Tests `optional_and_nullable_parameters_are_their_type` (type_hints.rs:3402, Swift case at 3430) and `nullable_markers_are_removed` / `maybe_wrappers_are_their_argument_and_containers_are_not` (annotation.rs:166, 208; Swift at 168-169, 221).
    - f1 L3: `sh: Shape?` and `sh?.area()` → `Shape.area` high.
  - `MAYBE_TYPE_WRAPPERS` swift = `Optional`, `ImplicitlyUnwrappedOptional` (stdlib.rs:119).
- **Return-type text**: `extract_return_type("swift", "func f() -> Foo?") == Some("Foo")` (test `optional_return_types_are_their_type`, type_hints.rs:3480/3494). This text feeds `build_return_type_map`, but see the gaps below.
- **Casts and constructions as receivers** (`typed_receiver`):
  - `(s as Base).draw()` → `Base.draw` high (f1 L77).
  - `Foo(…).m()` is typed `Foo` (receiver.rs:46-51). f1 b.swift L7 `Square(side: 1).cleanup()` bound `Square.cleanup` at medium.
- **`self.m()` / `Self.m()`**: skipped by propagation (type_hints.rs:563-570) and bound intra-file to the caller's type (f4 L25, L27).
- **Variable named like a type**: `find_constructor_assignments` (type_hints.rs:2404-2450) maps a receiver whose name equals a workspace type lowercased (`engine` → `Engine`).
  - f4 L22: `engine.start()` → `Engine.start` high.
  - This is a naming heuristic, not field typing. See the false-`high` gap below.

Not supported (fixture evidence):

- **Local annotations** `let e: Engine = …; e.start()`: untyped; fanned out to `Engine.start` + `Motor.start` at medium (f4 L34; `calls()` L78 the same).
- **Local constructions** `var x = Engine(); x.stop()`: untyped, medium fan-out (f4 L36).
- **Local from a static or instance return type** `let y = Engine.shared(); y.start()` and `let z = y.clone()`: untyped, medium fan-out (f4 L38, L40). f5: `let x = make()` (returns `Int`), then `x.go()` → `fanout-cap-exceeded` 6.
- **Stored properties** `var eng: Engine`; `eng.start()` and `self.engine.stop()`: untyped (f10 L5 medium fan-out; f4 L23 medium fan-out).
- **`if let` / `guard let` bindings** `if let o = Optional(m) { o.start() }`: untyped (f4 L41, L43).
- **Closure parameters** `arr.forEach { (e: Engine) in e.stop() }`: untyped, medium fan-out (f4 L50). `$0.start()` → unresolved `candidates-in-other-files` (f4 L49). Swift is not in `closure_types.rs` and is not in `CALLBACK_TYPE_CONSTRUCTORS` (stdlib.rs:143-147).
- **Aliases** `let k = m; k.stop()`: untyped (f4 L53). `typealias W = Widget` with `x: W` → `x.render()` unresolved (f8 L28).
- **Top types**: `x: AnyObject` → the receiver is typed `AnyObject`, and the call is booked as **stdlib** `<stdlib>::AnyObject::go` (f5 with `--include-stdlib`). Swift has no `TOP_TYPES` entry (stdlib.rs:151-155).
- **Implicit receivers or weak types**: never set (`implicit_receiver` / `weak_receiver_type` are not written by swift.rs).

## 06. Inheritance, dispatch & overloading

- **Supertypes and conformances** are recorded per method in `DefRecord::base_types` (swift.rs:73, 180, 333-350), for classes, structs, enums, protocols and extensions (`extension Foo: P`).
  - `FileFacts::classes` is **not** written.
  - The CHANGELOG notes "Swift and Dart record supertypes" (CHANGELOG.md:656).
  - The comment in cross_file.rs:4086-4088 ("Swift, Objective-C, whose plugins record no base types") and the `aws-lambda` swift gap text ("Swift also records no supertypes", rules.rs:4594) are **stale** on this point.
- **Inherited member through a typed receiver** works through `resolve_via_bases`:
  - `m: Mid` (with `Mid: Base`), then `m.inherited()` → `Base.inherited` high.
  - `l: Leaf` (`Leaf: Mid: Base`), then `l.inherited()` → high (f7).
- **Type with no methods loses its bases**: `class Empty: Base {}` and `e.inherited()` → unresolved `candidates-in-other-files` (f7). The bases ride only on method records.
  - The same applies to a conformer with no own body: f4 `struct R1: Runner {}` and `r.run()` / `r.prep()` → unresolved, so the protocol-extension default implementation is not reached.
- **`super.m()`**: `super.over()` in `Mid.over` → `Base.over` high (f7). In f1, `super.draw()` → `Base.draw` medium (the class also conforms to `Shape: Base`).
  - When the base is outside the tree: `super-base-out-of-graph` (f1 `super.init()`; f3 `super.viewDidLoad()`, `super.setUp()`).
  - Swift is not in the list of languages that record every class (Python/Kotlin/Java/C#). The refusal is reached here because every candidate is excluded.
- **Bare calls to inherited members**:
  - `inherited()` in `Leaf.leafOnly` → `Base.inherited` high, intra-file (f7).
  - `over()` where `Base.over` and `Mid.over` both exist in the file → **`ambiguous-in-file`** (f7). There is no nearest-override choice: Swift is not in `member_of_calling_class`, and cross_file.rs:4086 lists Swift as "unchanged".
- **Protocol (interface) dispatch**:
  - A call typed to a protocol binds to the `abstract` requirement: `useGreeter(g: Greeter) { g.greet() }` → `Greeter.greet` high (f3); `useP(x: P) { x.p() }` → `P.p` high (f12).
  - Under `--dynamic-dispatch` (and `--dead-code`), `inheritance_fanout` adds `Greeter.greet -> English.greet / French.greet` and `Base.over -> Mid.over` as `low dynamic dispatch:inheritance` (f3, f7). Swift is in `INHERITANCE_LANGS` (dispatch.rs:110-123).
  - Static requirements were not fanned out: `P.s -> Outer.s` was absent (f12).
- **Abstract declarations yield to concrete ones**:
  - Duck-typed fan-out drops `abstract` stubs when a concrete candidate survives (cross_file.rs:1153-1160).
  - In-file bare calls prefer a concrete candidate (intra_file.rs:575-586; test `a_bodiless_declaration_yields_to_a_concrete_same_file_candidate`, intra_file.rs:1449-1478).
  - f1 L62: `s.area()` → only `Square.area`, not `Shape.area`.
- **Overloads**:
  - In-file, a same-QN set is one callee (`binds_same_name_set`, intra_file.rs:835-848) and every overload is bound at medium (f6 c.swift: `Same.use -> Same.f` ×2; f8 `setup -> platform` ×2).
  - Untyped cross-file fan-out also binds all of them (f1 L80-81: `Square.overloaded` ×2 per site).
  - There is **no arity narrowing**: Swift is not in `OVERLOAD_ARITY_LANGS` (type_hints.rs:1583), and the plugin records no arity or labels.
  - **Bug: a cross-file typed-receiver lookup keeps only one overload.** It does not expand to the set: that expansion is gated on `OVERLOAD_ARITY_LANGS`, cross_file.rs:2621. In f6:
    - `o.f(1)` and `o.f(1, 2)` (`o: Over`) **both** bound only to the last-declared `Over.f` (L3, the 1-argument one).
    - So `o.f(1, 2)` gets a wrong edge and the 2-argument overload gets no edge.
    - f5: `o.f(1)` bound to the 2-argument `f`.
- **Extension methods**: members of `extension T` are ordinary members of `T` (§02). There is no extension-specific step (C#-style step 4d and Kotlin step 0b are not Swift).
  - `3.doubled()` with `extension Int { func doubled() }` in the same file → unresolved `candidates-in-other-files` (f1 L88): a literal receiver is not typed.
- **Mixins and protocol extensions**: a protocol extension's methods are owned by the protocol (`Runner.run`, `Runner.prep`). They are reachable only through a receiver typed as the protocol, or by duck fan-out, never through a conformer type (f4).
- **Class hierarchy for dead-code** (`ClassHierarchy::build(facts)`) reads the method `base_types`. Inheritance fan-out therefore works for types that declare at least one method.

## 07. Scoping & shadowing

- **`FileFacts::locals`** is collected by `cgg-lang/src/locals.rs` with `SWIFT_SPEC` (locals.rs:264-282, dispatched at 387). It covers:
  - Binders:
    - `parameter`, `lambda_parameter` (Param);
    - `pattern` → `bound_identifier`, in scope after the `property_declaration` (Decl);
    - `if_statement` bound identifiers, scoped from the `{` (Own);
    - `guard_statement` bindings (Decl).
  - Function scopes: `function_declaration`, `lambda_literal`, `init_declaration`, `computed_property`.
  - Blocks: `statements`, `for_statement`.
  - Test `dart_ocaml_lua_python_rust_kotlin_swift_scala_zig` (locals.rs:840; Swift case at 892-897: `a`, `k`, `it` bound; `f`, `xs` not).
- **`shadowed-by-local`**: `let start = { … }; start()` in a file with `Engine.start`/`Motor.start` → `shadowed-by-local`, with no edge (f4 L47).
- **Lexical narrowing of nested definitions**: Swift is `lexically_scoped` (intra_file.rs:119-136).
  - `func outerFn() { func inner() {}; inner() }` binds the nested `inner` (L7).
  - `other() { inner() }` binds the top-level `inner` (L9). Both high (f12).
  - CHANGELOG.md:577-581.
- **Positional shadowing** (a later same-scope binding shadows an earlier one): not applied. Positional shadowing is OCaml, F# and Lua only (CHANGELOG.md:580-581).
- **Conditional definitions**: no `DefRecord::conditional` for Swift. `#if` branches are both extracted and bound as an overload set at medium (f8).
- **Receiver head shadowing an import**: generic, via `locals.shadows_receiver_head`. It is moot for Swift because heads.rs does not run for it.

## 08. Macros, preprocessor & codegen

- **`#if` / `#elseif` / `#else`**: no preprocessing. The grammar parses every branch and the plugin extracts all of them (f8: both `platform` definitions; calls in both branches produce edges).
- **Swift macros** (`@freestanding`/`@attached`, `#expect(...)`, `#Preview { }`, `@Observable`): not expanded. No `macro:<name>` tagging and no `FileFacts::fn_macros` (swift.rs never writes them).
  - Calls inside a `#expect(...)` argument are not recorded as calls: the macro expression is not a `call_expression` (f3 `NewTests.swift`).
- **Property wrappers and result builders** (`@State`, `@ViewBuilder`): invisible (no attributes, §02).
- **Generated code tagging**: none.
- **`name_mentions`**: never written by the Swift plugin.
- **Generics**: generic declarations are ordinary nodes. Specializations do not create owners such as `Wrap<Int>`, because the type arguments are dropped (swift.rs:343, 297-329).

## 09. FFI & cross-language links

None of the `ffi::link_ffi` families covers Swift (ffi.rs:1-14: c-abi, pyo3, wasm-bindgen, napi, jni, pinvoke, kv-python). Every family keys on attributes, and Swift records none.

- **`@_cdecl("name")` exports**: not recognised. In f11, the C `swift_exported()` call bound to the C prototype; a C call to `swiftExported` stayed `no-candidate-in-file`.
- **Calling C or Rust from Swift** (bridging header or module map): `c_add(1, 2)` and `rust_ffi(3)` from Swift → `no-candidate-in-file`, with no FFI edge (f11).
- **Objective-C interop** (`@objc`, selectors, `NSObject` subclasses called from `.m` files): no link. Swift and objc are separate languages with no bridge pass.
- **Descriptor links** (Protobuf rpc → host method): `descriptor.rs` has no Swift-specific handling, and Swift methods would need a generated-code owner match. Unverified for swift-grpc; there is no test or fixture.
- **Markup code-behind** (`plugins/markup.rs`): C# only. Storyboard and XIB `IBAction`/`IBOutlet` connections are not read.

## 10. Frameworks & entry points

All framework rules whose `language` is `"swift"` (rules.rs). There are no multi-language rules: each `RuleSpec` has a single `language`.

| rule id | kind | detect (imports) | trigger | line | status |
| --- | --- | --- | --- | --- | --- |
| `aws-lambda` | network | `AWSLambdaRuntime`, `AWSLambdaEvents` | `methods: ["handle"]`; `registrars: ["run"]`; `node: true` | rules.rs:4580-4598 | **works for `handle`**: f9 minted `<framework-entry>::network::aws-lambda::H.handle`, and integration test `aws-lambda/swift` (crates/cgg/tests/frameworks.rs:1204-1208). `Lambda.run { … }` is never bound (no argument capture), confirmed in f9: `legacy`/`work` are reported dead. |
| `uikit-lifecycle` | lifecycle | `UIKit`, `AppKit`, `Cocoa`, `WatchKit` | `methods`: 26 names (`viewDidLoad`, `loadView`, `viewWillAppear`, …, `layoutSubviews`, `touchesBegan`, `awakeWithContext`); `node: false` (root marks only) | rules.rs:4978-5019 | works. The match is by method name in any type of a file importing UIKit/…, so f9 `class NotAVC` with no base still had `viewDidLoad` and `layoutSubviews` rooted (`uikit-lifecycle:base-type`). |
| `uikit-appdelegate` | lifecycle | same | `methods`: `application`, `applicationDidFinishLaunching`, … `didDiscardSceneSessions` (19 names); `node: false` | rules.rs:5022-5056 | works (f9: `AppDelegate.application`, `AppDelegate.applicationWillTerminate` rooted). |
| `cocoa-delegate` | lifecycle | `Foundation`, `UIKit`, `AppKit`, `Cocoa`, `WatchKit`, `WebKit` | `methods`: `urlSession`, `tableView`, `collectionView`, …, `observeValue` (22 names); `node: false` | rules.rs:5059-5103 | works (f3: `VC.tableView` rooted). Because it detects on `Foundation`, almost every Swift file triggers its "seen, no rules" coverage line (f1). |
| `swiftui` | lifecycle | `SwiftUI` | none, disclosed gap | rules.rs:5105-5120 | gap: `var body` is a property, which is not extracted, and `@main` is invisible. Calls inside `body` are `no-enclosing-callable` and root their targets by name (f9: `greet` rooted as `toplevel:invocation`). |
| `xctest` | test | `XCTest` | `base_types: ["XCTestCase"]`, `methods: ["test*", "setUp", "tearDown", "setUpWithError", "tearDownWithError"]`; `node: true` | rules.rs:5122-5143 | works (§11). Its `gap` text ("cgg has no entry rules for XCTest … no prefix matcher") is **stale**: the rule enumerates entries. |
| `swift-testing` | test | `Testing` | none, disclosed gap | rules.rs:5145-5160 | gap: `@Test`/`@Suite` are attributes, which are not recorded (f3: `addition`, `S.t` reported). |
| `combine` | lifecycle | `Combine` | none, disclosed gap | rules.rs:5162-5177 | gap: closure arguments and key paths are not recorded. |
| `vapor` | network | `Vapor` | none, disclosed gap; `node: true` | rules.rs:5179-5194 | gap: routes are trailing closures and `RouteCollection.boot`. f9: `routes`, `hello`, `Ctl.boot` reported dead. |

Helper tables:

- `member_sources_for`: none (rules.rs:11719-11764).
- `signature_methods_for`: none (11835-11861; Java serialization only).
- `container_attributes_for`: none (11770-11795; Rust only).
- `markup_kinds_for`: none (11812-11819).
- `visibility_entries_for`: none (11686-11691; Solidity only).
- `self_module_markers_for`: none.
- `detect_calls_for`: Swift rules have `detect_calls: NONE`.
- **Registrar verbs**: `registrar_verbs_for("swift")` = `["run"]` (from `aws-lambda`, rules.rs:11929-11955). It is never used, because the plugin does not consult `ExtractCtx::is_registrar_verb`.

Entry nodes:

- Network or test rules with `node: true` mint `<framework-entry>` nodes (f9 aws-lambda; f3 xctest).
- Lifecycle rules with `node: false` only root-mark (f3/f9 `roots` with `framework:rule`).
- The README benchmark expects `cocoa-delegate,uikit-appdelegate,uikit-lifecycle` on swift-alamofire (`scripts/benchmark.sh:400`) and `~swift-testing` on app-ora-swifttesting (`scripts/benchmark.sh:309`).

## 11. Test harness entries

- **XCTest** (`xctest` rule, kind test, rules.rs:5122-5143):
  - **Base-type plus naming pattern**: a class whose bases include `XCTestCase` has its `test*` methods and the setup/teardown hooks entered from one harness node, `<framework-entry>::test::xctest` (f3: `AppTests.testGreet`, `AppTests.setUp`).
  - Works transitively: `class Deep: Checks` (`Checks: XCTestCase`) → `Deep.testTwo` entered (f14).
  - Works in a file not named `*Tests.swift` (f14 `Checks.swift`).
  - A `testLooksLike` method on a non-`XCTestCase` class in an XCTest-importing file was **not** entered (f9 `notest.swift`: reported dead).
  - `prefix*` patterns are case-sensitive (`test*`).
  - Not listed:
    - `override class func setUp/tearDown` is matched only by the name `setUp`/`tearDown`, so class-level and instance-level are not told apart;
    - `invokeTest`, `setUp() async throws` (matched by name), `measure { }` closures.
- **swift-testing** (`@Test`, `@Suite`): not enumerated, a disclosed gap (rules.rs:5158). In f3, `addition` and `S.t` appeared as medium dead-code findings under `--include-tests`.
- **Quick/Nimble** (`QuickSpec.spec()`, `describe`/`it` closures): no rule. Closure bodies are attributed to `spec()` (by §02/§03 mechanics); `spec` itself is unrooted. Unverified by fixture.
- **`TEST_CLASS_MEMBERS_ARE_ROOTS`**: Swift is deliberately excluded (rules.rs:11797-11806: "Statically typed languages (Java, Swift) bind such calls and keep their unused helpers reportable"). f3: `AppTests.localHelper` (called) is reported only as `D004`; `AppTests.notATest` is reported.
- **Test-file gating**: test files are those named `*Tests.swift` / `*Test.swift` or under a test directory (§01). `PluginSignals::test_defs` is false, so there is no in-file `TestRole` tagging: `test_tagging: none` in the f3 capabilities.

## 12. Dead-code analysis

f3 capabilities row for swift:

```text
class: degraded, visibility: none, attributes: full, value_references: none, dispatch: none, exports: none, test_tagging: none, max_confidence: medium
```

- **Max confidence medium**:
  - `LanguageLacksVisibility`, `LanguageLacksValueReferences` and `LanguageLacksDispatchModel` are always pushed (deadcode/mod.rs:913-924), and each caps at medium (cgg-core/src/deadcode.rs:323-330).
  - Every Swift finding is therefore at most `medium` (f3, f9: all findings medium).
- **`attributes: full` is spurious**: capability measurement treats any callable with non-empty `attributes` as proof the language records attributes (deadcode/caps.rs:134-136). The only Swift attribute is the hardcoded `abstract` tag on protocol requirements. Consequences:
  - In a tree with a protocol requirement, Swift is shown as recording attributes, and `LanguageLacksAttributes` is not pushed (f3 risk lists lack it).
  - In a tree without one, it is pushed.
  - The capability is therefore data-dependent.
- **`dispatch: none`** while inheritance fan-out does run for Swift: the measurement reads only `trait_impl_target` (caps.rs:137-139), which Swift never sets.
- **Roots**:
  - `builtin:main`: any callable named `main` (roots.rs:20). f3: `MainApp.main` rooted.
  - `builtin:lifecycle` (roots.rs:148-155): `viewDidLoad`, `viewWillAppear`, `applicationDidFinishLaunching` by name, in any file. Also `CONVENTIONAL_METHODS` (`describe`, `toString`, …, roots.rs:162-171), language-agnostic.
  - Framework rules and test harness (§10, §11).
  - `toplevel:invocation`: any callable whose simple name matches a call with no enclosing callable (roots.rs:552-611). For Swift this covers:
    - `main.swift` top-level statements;
    - **calls inside computed properties, accessors, `deinit`, subscripts and property initialisers** (f9: `greet` rooted from `var body`).
    - It matches by name across the whole language, so it can over-root.
- **Not rooted**:
  - `@objc` / `@IBAction` / `@IBOutlet` / `#selector` targets (f3: `VC.tapped`, `VC.pressed` reported);
  - `Codable` hooks `encode(to:)` / `init(from:)` (f3: `Codec.encode` reported);
  - `@main` types (only a function literally named `main` is rooted);
  - `public`/`open` API: no visibility, no `exports` (f3: `publicApi` reported like `privateDead`);
  - operator functions (f1: `Square.+` reported);
  - functions passed as values (f1: `transform` reported);
  - extension methods called on literals (f1: `Int.doubled` reported);
  - `init`s, which are never reached by `Type(...)` (§03), unless something calls `self.init`.
- **Confidence caps**:
  - `implements-external-contract`: never fires for Swift. The `override` keyword is not recorded, and `OVERRIDE_MARKERS` (deadcode/mod.rs:409) needs it in `attributes`.
  - `macro-generated`: never fires (no `macro:` tags).
  - `reflection`: never fires (no dynuse).
- **Unresolved-doubt handling**: generic. A same-name unresolved site lowers a finding (evidence `NameMatchesUnresolvedSite`). Swift has many unresolved sites (constructors, untyped cross-file calls), so doubt is common.
- **Low root coverage**: the f1 run withheld 21 `reachable-only-from-dead-code` and 2 `dead-cycle` findings (root coverage below 25%), which is typical when Swift entry points are invisible.

## 13. Builtins & stdlib tables

- **`crates/cgg-core/src/stdlib/swift.txt`**: 249 lines. It is included at stdlib.rs:199 and registered at stdlib.rs:240.
  - It holds Swift and Foundation names: `print`, `map`, `compactMap`, `Array`, `Bundle`, `Calendar`, `Codable`, `AnyObject`, `Actor`, `AsyncStream`, …
  - Used for stdlib bucketing, and as a skip for the step 1d fallback (cross_file.rs:6022-6023).
  - f5: `print("hi")` → `<stdlib>::print` under `--include-stdlib`; `x: AnyObject` → `<stdlib>::AnyObject::go`.
  - `"abc".uppercased()` and `[1].map { $0 }` were booked as **external** (`<external>::"abc"::uppercased`, `<external>::[1]::map`). A literal receiver is not recognised as a stdlib type.
- **`MAYBE_TYPE_WRAPPERS`**: `("swift", ["Optional", "ImplicitlyUnwrappedOptional"])` (stdlib.rs:119).
- **No Swift entries** in `CAST_INTRINSICS` (stdlib.rs:35-45), `CONSTRUCTING_METHODS` (58), `UNION_TYPE_CONSTRUCTORS` (125), `NULL_TYPE_MEMBERS` (131-137), `CALLBACK_TYPE_CONSTRUCTORS` (143-147) or `TOP_TYPES` (151-155). Swift casts are syntax (`as`, `as?`, `as!`), handled by `receiver.rs` instead.
- **No `rust_types`-like wrapper table**: `Array<T>`, `Result<T, E>`, `Task<T>` and `@MainActor` wrappers are not seen through.
- **No implicit-package list**: there is no Swift counterpart of `SCALA_IMPLICIT_PACKAGES`. `Swift`, `Foundation` and `_Concurrency` are not modelled.
- **`--include-stdlib` / `--include-external`**: these mint deduplicated exit nodes as for every language (f5 out2.json). External attribution is by receiver text rather than by import, because heads.rs does not run for Swift.

## 14. Confidence & certainty

Observed `high` paths for Swift:

- intra-file bare call with exactly one candidate (f1 `topLevel -> nested`, `generic`, `risky`);
- `self.m()`, `Self.m()` and intra-file static `Type.m()` (f4);
- intra-file `Wrap<Int>.s()` (f1 L83);
- parameter-typed receiver naming one class or protocol, cross-file (f1 `sh?.area()` → `Shape.area`; f3 `testGreet -> English.greet`). For f3's `English().greet()`, the receiver is a construction;
- cast receiver `(s as Base).draw()` (f1 L77);
- inherited member through a typed receiver, and `super.m()` (f7).

Forced to `medium`:

- every duck-typed fan-out (untyped locals, properties, closures; f4);
- cross-file static calls `Square.make()` from another file (f1 b.swift L5). Swift QNs carry no module, so owner uniqueness is per file (`unique_owner_names`, cross_file.rs:4391-4392);
- construction receivers cross-file (`Square(side: 1).cleanup()`, f1 b.swift L7);
- overload sets (f6);
- the step 1d by-name fallback (f2, f13).

**False `high` from the naming heuristic**: `let engine = Motor(); engine.start()` bound to **`Engine.start` at high** (f10 L6). `find_constructor_assignments` maps `engine` → `Engine` by name (type_hints.rs:2437-2450), and the actual local type is never read.

There is no `low` call edge for Swift except dispatch, entry and exit edges.

Mermaid marks arrows with no `high` site with a leading `?` (f1: `N6 -->|?| N2` for `crossCaller -> Square.area`; `N6 --> N0` for the high `Shape.area`).

## 15. Dynamic use & reflection

- **dynuse** (`plugins/dynuse.rs`) covers Python, JS, Java, C#, Ruby and PHP only. Swift declares `dyn_uses: false` (default signals).
- **Not screened**:
  - `perform(NSSelectorFromString("pressed"))`, `#selector(VC.tapped)`, `value(forKey:)`, `NSClassFromString`, `Mirror` reflection, `@dynamicMemberLookup`, `@dynamicCallable`.
  - None produces a `reflection` screen or a cap. f3: `VC.pressed` and `VC.tapped` were reported dead at medium with no reflection evidence; `NSSelectorFromString` and `perform` landed in the unattributed bucket.
- **Module load**: none. Swift has no runtime module loading in the tree-sitter view; `dlopen`/`Bundle.load` are not screened.

## 16. Known gaps & limitations

From code comments, `gap_for()` and the CHANGELOG:

- `gap_for("swiftui", "swift")`: no `body` callable, and `@main` is invisible (rules.rs:5118).
- `gap_for("swift-testing")`: no attributes, so `@Test`/`@Suite` are invisible (rules.rs:5158).
- `gap_for("combine")`: closures and key paths are not recorded (rules.rs:5175).
- `gap_for("vapor")`: trailing-closure routes and `RouteCollection.boot` are not seen (rules.rs:5192).
- `gap_for("aws-lambda","swift")`: `Lambda.run { … }` is not bound (rules.rs:4594). Its "records no supertypes" clause is stale.
- `gap_for("xctest")`: the text says there are no entry rules, but the rule now matches `XCTestCase` + `test*` (rules.rs:5139). The text is stale.
- CHANGELOG.md:455-460: bare-name scope is unchanged for Swift ("unrecorded base types make their bare-name scope a matter of types cgg does not have").
- CHANGELOG.md:1349-1352: "Swift module/target visibility" is not covered (WS14).
- CHANGELOG.md:751-754: "Kotlin/Scala/Swift/Dart function references (`::f`, `f _`)" are not covered (WS6).
- cross_file.rs:4086-4088 lists Swift among plugins that "record no base types". This is stale, but the behaviour it describes (no `member_of_calling_class`) is current.

Found while cataloguing (fixture evidence):

1. **Constructor calls never bind to `init`.** `Foo(...)` is `no-candidate-in-file`, because Swift is missing from `constructor_names` (cross_file.rs:383-391). (f1, f4, f8, f10)
2. **Cross-file bare calls fail in files without an `import`**, and for names with more than 3 definitions. They resolve only through the step 1d fallback (cross_file.rs:6015-6056). (f2, f1)
3. **No local, property, return-type, `if let`/`guard let` or closure-parameter typing.** Only parameters, casts and constructions type a receiver. (f4, f10)
4. **False `high`**: a local whose name equals another type's lowercased name is typed as that type (f10).
5. **Cross-file typed overload lookup keeps only the last overload**, a wrong edge (f5, f6). There is no arity or label narrowing at all.
6. **Properties, accessors, observers, subscripts, `deinit`, closures and top-level code are not callables.** Their calls are `no-enclosing-callable`. (f1, f8)
7. **Not recorded**: implicit member calls (`.shared()`), operator and subscript uses, function values, `#selector`, key paths, optional-chained casts (`(x as? T)?.m()` → receiver `(`). (f4, f8)
8. **No visibility, no attributes and no `override` marker.** Every dead-code finding is capped at medium. `@objc`/`@IBAction`/`@main`/`@Test` are invisible. The capability table reports `attributes: full` because of the `abstract` tag. (f3)
9. **Bases are carried only on method records.** A conformer or subclass with no methods (`class Empty: Base {}`, `struct R1: Runner {}`) loses its inheritance. (f4, f7)
10. **A bare call to a method overridden in the same file is `ambiguous-in-file`**, with no nearest-override rule (f7).
11. **`static func` requirements are not fanned out by `--dynamic-dispatch`** (f12). `class func` is not detected as static (f1).
12. **Module-qualified calls** `Module.f()` never bind: QNs carry no module (f13). `Package.swift` targets are not read.
13. **No external-module refusal (heads.rs)**, no `FileFacts::packages`, and no FFI (`@_cdecl`, C interop) (f5, f11).
14. **Literal receivers** (`3.doubled()`, `"abc".uppercased()`) are not typed and are misfiled as external (f1, f5).
15. **`#if` branches are all extracted** and bound as an overload set (f8).

Ignored tests: `grep -n '#\[ignore' crates/cgg-lang/src/plugins/swift.rs` finds none, and there are no Swift fixtures under `crates/cgg-lang/tests`. Swift coverage is limited to:

- the 7 inline tests in swift.rs;
- one `locals.rs` case;
- one intra_file test;
- 2 type_hints tests and 2 annotation tests;
- the `aws-lambda/swift` integration case;
- the `detect_prefixes.rs` import form.

## Summary scorecard

| section | support | one-line note |
| --- | --- | --- |
| 01. Detection & parsing | partial | `.swift` only; no shebang; `Wrap<Int>.s()` misparse repaired; no fallback re-parse |
| 02. Callables extracted | partial | funcs, methods, inits, operators, protocol requirements, extensions; no properties, accessors, subscripts, deinit, closures or top-level; no visibility or attributes |
| 03. Call-site forms | partial | bare, member, chained and trailing-closure calls; casts and parens peeled; implicit members, operators, subscripts, function values, arity and labels not recorded |
| 04. Imports & module resolution | minimal | `import X` recorded; cross-file bare calls only by the ≤3-candidate fallback and only in files with an import; no packages, manifests or heads.rs refusal |
| 05. Type inference & receiver typing | minimal | parameter annotations (with `?`/`!`/`Optional`) and cast/construction receivers only; locals, properties, returns and closures untyped; name-coincidence false high |
| 06. Inheritance, dispatch & overloading | partial | bases on methods; inherited and `super` through typed receivers; protocol dispatch fan-out; method-less types lose bases; typed overload bug; no arity |
| 07. Scoping & shadowing | full | `SWIFT_SPEC` locals, shadowed-by-local and lexical nesting work; no positional shadowing (by design) |
| 08. Macros, preprocessor & codegen | none | `#if` branches all extracted; Swift macros and property wrappers invisible |
| 09. FFI & cross-language links | none | no `@_cdecl`, C/ObjC interop or descriptor link |
| 10. Frameworks & entry points | partial | 9 rules: uikit-lifecycle, uikit-appdelegate, cocoa-delegate, xctest and aws-lambda `handle` work; swiftui, swift-testing, combine and vapor are disclosed gaps |
| 11. Test harness entries | partial | XCTest by base type + `test*` (transitive); swift-testing and Quick not enumerated; no in-file test tagging |
| 12. Dead-code analysis | minimal | max confidence medium; no visibility, attributes or override; spurious `attributes: full`; `toplevel:invocation` covers property bodies |
| 13. Builtins & stdlib tables | partial | `swift.txt` (249 names) plus Optional wrappers; no cast, top-type or implicit-package tables; literals misfiled as external |
| 14. Confidence & certainty | partial | high for parameter/cast/self/intra-file/inherited; cross-file static medium; false high from the name heuristic |
| 15. Dynamic use & reflection | none | no dynuse for selectors, KVC or `NSClassFromString` |
| 16. Known gaps & limitations | n/a | 15 cataloged gaps; stale gap texts for xctest and aws-lambda and a stale cross_file.rs:4086 comment |
