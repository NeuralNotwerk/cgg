# Changelog

All notable changes to `cgg` are documented here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/); this project is
pre-1.0. A minor release (0.x.0) may change the default graph, including
removing edges a better resolver shows were wrong, and says what changed
under *Compatibility*. A patch release (0.x.y) fixes bugs and does not
otherwise change the default graph.

## [Unreleased]

### Fixed

- **`high` no longer reports a guess as certain, nor a certainty as a
  guess (WS19).** Seven ways the `?` marker was wrong:
  - *A guessed receiver type bound its same-file candidate at `high`*
    (19 languages). `service = mk(); service.run()` was typed `Service`
    only because the file declares a `Service`, and intra-file resolution
    ignored the `weak_receiver_type` flag cross-file already honoured. The
    single-candidate, arity-narrowed overload and clause arms now bind at
    `medium` through such a receiver. The same flag now also gates the
    cross-file module-alias arm.
  - *A lone nested function was bound from any function at `high`*
    (Python, JS, TS, Lua, OCaml, F#, Scala, Kotlin, Swift, Rust, Haskell,
    Julia). Lexical narrowing ran only with two or more candidates; a
    single function nested in another that does not enclose the call is
    now kept (a returned closure may be called through a same-named local)
    but at `medium`.
  - *Lambda and closure parameters did not shadow* (`(go) => go()`,
    `lambda go: go()`, `|go| go()`, `fun go -> go ()`, Kotlin
    `{ go: T -> }`). `FileFacts::locals` now marks parameters
    (`LocalBinding::param`); a call through one is never a function nested
    in some other function, and is recorded `shadowed-by-local`.
  - *The return-type map was keyed by bare name, first definition wins.*
    A Swift `make() -> Service` typed a Python `service = make()`, and of
    two Python `make`s the first read decided for both. It is now keyed by
    `(language, name)`, a name is typed only when every definition of it
    in the language declares the same type, and a name the calling file
    defines is read from that definition.
  - *Include-style imports bound an arbitrary same-named copy at `high`*
    (Bash `source`, Ruby `require`, R `source()`, PowerShell dot-source,
    Solidity `import`, Starlark `load`, CMake `include`, Lua `require`): the
    last copy indexed won, whatever the include named. A call now binds the
    copy in the file the include names (as `c_include_copies` does for C).
    An include written as a path suffix several files share (`require
    'helper'` with two `helper.rb`) expands each and binds at `medium`; a
    bare imported name whose qualified name another file also carries
    (vendored copies, `.pyi` stubs) is `medium` unless the copy is in the
    caller's include closure. A Lua `local function` is no longer put in
    a requiring file's scope.

  - *A Kotlin property's declared type read as a guess.* A class is
    recorded as a constructor spanning its body, so `lateinit var vm: Vm`,
    `val x = Vm()` and a constructor's `val`/`var` parameter looked like
    another function's local and the receiver's type like a guess. A
    property's type is now read as a declaration, scoped to the class that
    declares it (two classes of one file declaring `vm` with different
    types each bind their own; a disagreement stays a guess).
  - *Lua file-scope `local`s were not bindings.* `local log = ngx.log` at
    the top of a file did not shadow, so a bare `log(…)` bound a required
    module's `log` at `high`. File-scope `local`s are now recorded; a
    same-named member of a required module (`local feed = t.feed`,
    `local f = require(…).f`) is still that member, and a file-scope
    `local a = m.b` with `m` a `require` is the call `m.b(…)`: it binds
    through the required module, including a field of the table the
    module returns (`return { check = check_phase }`), which `m.check(…)`
    now reaches too.

- **Dead-code false positives (WS20).** Six ways live code was reported
  dead, or reported `high`:
  - *Override markers were recorded for Java, C# and Groovy only.* The
    Kotlin, Swift, Scala and TypeScript `override` modifier, Dart's
    `@override` and C++'s `override`/`final` specifiers are now recorded
    (as the `override` attribute; Dart as `@override`), and PHP 8.3's
    `#[\Override]` now matches. An override of a base outside the tree
    carries `implements-external-contract` and caps at `medium`.
  - *A Python `async def` method skipped that cap*, because it was told
    apart from a function by its kind, and async methods are recorded as
    functions. A definition is now a method when its owner is a class.
  - *Calls after `return`/`throw`/`break` kept their target live.* The
    unreachable regions eight plugins compute under `--dead-code` are now
    read: such a call keeps nothing alive (its callee reports
    `incoming-only-from-unreachable-code`), and its edge is capped at
    `medium` in a `--dead-code` run. Go regions were never found
    (tree-sitter-go nests a block's statements in a `statement_list`); a
    region now ends before a `goto` label nested in a later statement, and
    a Rust block with `#[cfg(…)]` statements is conditional compilation,
    never a region.
  - *Rust items under `#[cfg(test)]`* (a `mod tests`, any nested module,
    `#[cfg(all(test, …))]`, a file-level `#![cfg(test)]`, and attributed
    items) are now test code, not production dead code.
  - *C++ gtest and libFuzzer harnesses were reported dead.* `TEST`,
    `TEST_F`, `TEST_P`, `TYPED_TEST` and `TYPED_TEST_P` bodies in a test
    file or one including gtest enter through the `gtest` harness node;
    `SetUp`/`TearDown` (and the suite hooks) of a class deriving from
    `testing::Test`/`TestWithParam` through the `gtest-fixture` node; and
    C/C++ `LLVMFuzzerTestOneInput` and its optional hooks through
    `libfuzzer`. A base written `::testing::Test` is now read as
    `testing::Test`.
  - *Test-file and test-name conventions were uneven.* New test-file arms:
    Perl `*.t`, Groovy `*Spec`/`*Test(s).groovy`, Lua `_spec`/`_test`,
    PowerShell `*.Tests.ps1`, Solidity `*.t.sol`, Objective-C
    `*Test(s).m(m)`, Haskell `*Spec`/`*Test(s).hs`, R `test-*`/`test_*`,
    Clojure `_test.clj(s/c)`, Terraform `*.tftest.hcl`, Bash `_test.sh`,
    Zig `_test.zig`, Julia `runtests.jl`, and JS/TS `.test.mts`/`.cts`,
    `.spec.mjs`/`.cjs`/`.mts`/`.cts`. A test name now needs a word boundary
    after `Test` (`Testing`, `Tester`, `Testimonials` are not tests), Go
    follows `go test`'s rule for `Test`/`Benchmark`/`Fuzz`/`Example`
    (`FuzzyMatch` is not a fuzz test), and a PowerShell `Test-*` function
    is never a test.

- **Constructions and constructor delegation reached no constructor
  (WS21).** Two kinds of call made no edge:
  - *A construction was no call* in C# (`new T(…)`, `T x = new(…)`),
    Swift (`T(…)`), Ruby (`T.new`), PHP (`new T`), Solidity (`new C(…)`),
    PowerShell (`[T]::new(…)`) and Objective-C (`[[T alloc] initWith…]`).
    Each is now a call of the type's constructor (`T.T`, `init`,
    `initialize`, `__construct`, `constructor`, the `init…` method), looked
    up on the type; a type declared or imported unambiguously binds at
    `high`, Swift's by-name `T(…)` at `medium`. A type outside the tree
    binds nothing in it: C# `new List<int>()` under `using
    System.Collections.Generic`, PHP `new \Exception`, a dotted PowerShell
    .NET type. C# constructions now carry their argument count, which
    narrows constructor overloads as in Java (a `params` parameter takes
    any count).
  - *Constructor delegation recorded no site*: Java `this(…)`/`super(…)`,
    JS/TS `super(…)`, C# `: this(…)`/`: base(…)`, C++ base and delegating
    member initializers (`D() : Base(1)`, `D(int) : D()`), Solidity base
    constructor invocations (`constructor() Base(x)`) and Ruby's bare
    `super`/`super(…)`. Each resolves through the class's base chain (or to
    a sibling constructor, never the one it is written in). A base outside
    the tree is `super-base-out-of-graph`, never a same-named member
    elsewhere; a C++ data-member initializer (`n_(0)`) makes no call and
    no record; a base constructor reached through several same-named class
    copies narrows to the calling file's copy, else the nearest one.
  - Swift initializers now carry their class's bases and `override`
    marker, a typed receiver's own name no longer qualifies a same-file
    method of that name (`new Thing()` inside a method `Thing`), and Ruby
    typed receivers narrow by the namespace path written, as JVM ones do.
    A Ruby `T.new` or `super` that is not structurally certain is refused
    (`candidates-in-other-files`): Ruby bases and receiver types are
    matched by their last segment, so such a guess landed on a same-named
    class of another namespace about half the time.
  - Scala, Groovy and F# constructors were not extracted as callables,
    so their constructions had nothing to reach; fixed by WS22 below.

- **Callables missing from the graph, and declarations counted as call
  targets (WS22).** Four extraction gaps:
  - *Bodiless interface and abstract members were not tagged `abstract`*
    in Java, C#, Kotlin, Groovy, Go (interface method specs), PHP, C++
    (`= 0`), Objective-C (`@protocol`), Solidity, Fortran (`interface`
    bodies) and Rust (trait methods without a default body), so duck-typed
    fan-out kept the declaration beside its implementations. They are now
    tagged as TypeScript, Swift, Scala and Dart members already were. The
    narrowing stays one-sided, and never leaves only the calling method
    itself or only candidates whose parameter list rejects the argument
    count: such a set keeps its declarations. A bare or `this` call to a
    member the caller's own class declares abstract binds to that
    declaration, not to same-named bodies of anonymous classes or enum
    constants; and a declaration whose bodies an enum constant or an
    anonymous class of its file provides is capped at `medium` in the
    dead-code report once one of them is called.
  - *JS/TS class fields holding a function* (`handleClick = () => {…}`,
    `static make = function () {…}`, `private save = (x) => {…}`) were no
    callable, so their calls had no enclosing function and
    `this.handleClick()` bound nothing. They are now methods of the class.
  - *Objective-C recorded no plain C calls*: `helper(x)`, `NSLog(…)` and
    every C function a `.m`/`.mm` file defines read as uncalled. Calls are
    now read as the C plugin reads them (a call through a field, a block
    property, is not a method call). A pointer-returning C function in
    such a file is named by its identifier (`label`, not `label(void)`),
    and a definition error recovery reads out of `if (x) { … }` is no
    function.
  - *Groovy, Scala and F# constructors* are now callables named after
    their type: Groovy `Box(…) { }`, the Scala primary constructor
    (spanning the class header, its parameters and base-constructor
    arguments) and `def this(…)`, and the F# primary constructor and
    `new(…) =`. `new Box(…)` (Groovy, Scala), `Box(…)` (Scala, F#), Groovy
    `this(…)`/`super(…)`, Scala `this(…)` and `extends Base(x)`, and F#
    `inherit Base(x)` reach them, never the constructor they are written
    in. Groovy joins arity narrowing for constructions, and F# records the
    `inherit` base. A Scala class without a parameter list has no
    constructor node.
  - A Java, Kotlin or Groovy variable named like the last segment of an
    imported package (`import a.gui.preferences.Gui;` then
    `preferences.get()`) was read as a call through that package and its
    lone candidate bound at `high`; such a call is now certain only when
    the callable is a member of what the alias names (a class aliased by
    importing its nested type).

### Compatibility

The default graph changes (full corpus, 283 repositories, none excluded):

- 10,381 edges move `high` → `medium`: 6,417 through a guessed receiver
  type, 1,417 to a nested function not visible at the call, the rest
  through an import whose name another file also carries (`.pyi` stubs,
  vendored copies), an include that names several files (Clojure, CMake)
  or an anonymous subclass's receiver (Java).
- 19,483 move `medium` → `high`: 19,471 Kotlin calls through a property
  whose declared type is now read as declared (46 read across 11
  repositories, all to the declared type's member), 12 Lua calls bound
  through the file a `require` names.
- 8,315 edges are removed (660 `high`): calls through closure parameters
  and Lua file-scope locals (`local fmt = string.format`) now
  `shadowed-by-local`, include bindings to the wrong copy, Kotlin duck
  fan-out decoys and wrong same-file self-calls replaced by the
  property's type, and receivers no longer typed by another language's or
  a disagreeing function's return type.
- 10,458 edges are added (1,235 `high`): 832 Kotlin calls to the declared
  property type's member; 364 Lua calls bound through the required
  module — its returned table's fields, member aliases, and `log(…)` on a
  required logging module reaching its `_M.log`; and the include's own
  copy in Erlang, Dart and Bash.

No dead-code finding changes confidence.

WS20 (dead-code fixes; full corpus, 283 repositories, none excluded):

- The default graph gains 5,546 `low` framework-entry edges and no other
  change: gtest (5,255 cases and 77 fixture hooks), libFuzzer (102
  entry points) and 112 XCTest cases in Objective-C test files now
  classified as such. The JSON `attributes` of overriding members in
  Kotlin, Swift, Scala, TypeScript, Dart and C++ now include `override`
  (`@override` in Dart).
- Under `--dead-code`, 7,001 findings leave the report (6 `high`, 6,675
  `medium`, 320 `low`: gtest/libFuzzer roots and the code they reach,
  Groovy, Objective-C, Perl, Zig, Clojure, Lua and Bash test files, Rust
  `#[cfg(test)]` code) and 3 move `high` → `medium` (Python async
  overrides). 26 are newly reported, none `high`: 8 PowerShell `Test-*`
  functions (`low`), 16 callables named `Tester…`/`Testable…`/
  `Testimonials`/`TestsFor…` and Go `Fuzzy…` functions and their callees,
  previously rooted as tests by name, and 2 Go methods whose only call is
  after a `return`.
- In a `--dead-code` run, an edge from an unreachable call site is
  `medium`.

WS21 (constructions and delegation; full corpus, 283 repositories, none
excluded):

- The default graph gains 108,034 edges (70,619 `high`), every one to a
  constructor or, for Ruby `super`, a base's member: PHP 64,609, C#
  21,591, Java 13,315, Swift 3,361, C++ 1,641, TypeScript 1,333, Ruby 803,
  Objective-C 862, JavaScript 451, Solidity 67, PowerShell 1.
- 244 `medium` edges are removed: 243 Python `super().__init__()` calls
  bound to every same-named base class in the tree, now only the calling
  file's (or the nearest) copy, and one Ruby construction bound to a
  same-named class of another namespace.
- Under `--dead-code`, no `high` finding changes. At
  `--dead-code-confidence low`, 10,545 findings leave the report (8,026
  constructors: PHP 4,015, Java 1,810, C# 1,320, Swift 376, C++ 274,
  Objective-C 122, TypeScript 54, JavaScript 48, Solidity 6, PowerShell 1;
  2,519 callables reached only through them); 279 move `medium` → `low`,
  2 `low` → `medium`, and none are newly reported.

WS22 (callable extraction; full corpus, 283 repositories, none excluded):

- 54,226 edges to bodiless declarations are removed from duck-typed
  fan-out (15 `high`): Java 36,313, Go 11,057, C++ 2,318, Kotlin 2,035,
  C# 1,188, Rust 775, Groovy 303, PHP 238, Solidity 51. Without the
  declarations, 14,551 call sites that were refused as
  `fanout-cap-exceeded` now fan out (about 48,000 `medium` edges, Java
  33,877, Go 8,290, Kotlin 2,673, C++ 2,587). No site in these languages
  is newly refused.
- 22,134 edges reach the new Groovy, Scala and F# constructors (10,464
  `high`); 11,204 callables are added (Scala 8,670, Groovy 2,407, F# 127).
  Constructions of a type with no constructor node in the tree (an
  outside class, a Scala class without a parameter list) are now
  recorded as unresolved sites: Scala 105,293, Groovy 10,765.
- 5,786 JS/TS edges are added from or to class-field methods (3,275
  `high`; 2,555 new callables), and 308 JS/TS call sites that bound by
  name are refused as `fanout-cap-exceeded` because the new methods add
  candidates.
- 1,323 Objective-C edges are added from plain C calls (1,129 `high`, 667
  of them into C++ headers the file imports). 15 Objective-C functions
  are renamed (`f(args)` → `f`), and their edges move with them; 14 C
  header prototypes of such functions are now absorbed by their bodies.
- 2,129 Java, Kotlin and Groovy edges move `high` → `medium` (calls
  through a variable named like an imported package), 202 Scala edges
  `high` → `medium` (a call into an abstract overload of the caller's own
  trait now keeps that declaration), 99 edges `medium` → `high`. 170 Scala
  `high` edges rebind from an implementation to the caller's own trait
  declaration; 51 Scala sites whose every concrete candidate rejects the
  argument count are refused instead of fanned out.
- Under `--dead-code`, 240 Objective-C C functions leave the report (all
  `medium`), no finding becomes `high`, one `high` finding moves to
  `medium`, and one leaves the report through a new `medium` fan-out edge.

## [0.11.0] - 2026-10-16

### Changed

- **Mermaid marks uncertain arrows with `?`.** An arrow with no
  `high`-confidence call site leads its label with `?` (`-->|?|`,
  `-->|?3x|`, `-->|?ref 2x|`, and in front of `--locations` site lists);
  unmarked means `high`. One token, only on non-high arrows; the arrow
  stays `-->`, so parsers keyed on it are unaffected. A collapsed arrow
  takes its best site's confidence. Arrows whose tag already says
  they are not a resolved call are never marked: `dyn` dispatch fan-out
  and framework entries (both always minted `low`) and
  `--include-external`/`--include-stdlib` exit nodes. On neurosymbolic_kb,
  20% of arrows carry the mark after the calibration below (45% before).

- **Edge confidence is calibrated: `high` now means the binding is
  structurally certain (WS17).** A call bound to exactly one definition
  through structure was reported `medium` whenever the step that found it
  was not one of the few that granted `high` — 6,346 of 6,638 cross-file
  bindings in one Rust workspace, including `use crate_x::m; m::f()` and
  `crate_x::f()` through a crate-root `pub use`. A single binding is now
  `high` when the step that made it is structural and nothing about it was
  chosen:
  - an exact qualified path the call writes (`a::b::f()`, `pkg.Cls.m()`)
    or an import, `use`, module alias, `crate::`/`self::`/`super::` or
    opened namespace anchors (one opened module only), with one callable
    carrying that name (no vendored copies, `#[cfg]` twins or stubs);
  - Rust: a relative path resolved from inside the caller's crate, when no
    other enclosing module holds the same path; `self.m()` found in
    another `impl` block of the caller's own type;
  - a typed receiver whose type is known — a declared parameter, field or
    local in scope, a cast or construction — looked up on a type that one
    declaration site owns in the tree (one owner path where qualified
    names carry the module; one crate for Rust; one directory for Go/C#;
    one file elsewhere), and, where the calling file imports that type by
    name, imported from that very path. Python and JS/TS record types
    without the module they came through, so there the type must be
    declared in the calling file or imported by name from the candidate's
    own package (Python); a dotted Python type (`pd.Series(...)`) must
    match the candidate's path;
  - a member of the calling class (Java/C#/Ruby bare calls) declared on
    the class itself with nothing below it overriding it;
  - an inherited member or `super.m()`, when the classes involved are each
    the only type of their name, the calling file anchors the class's
    bases (a dependency's `TestCase` is not the workspace's), and exactly
    one ancestor declares the member (Dart mixins, Scala traits and Python
    multiple bases otherwise leave the linearization to guess). Scala,
    Kotlin and Groovy `super` is never certain (anonymous class bodies are
    recorded on the enclosing class);
  - an overload set the argument count cuts to one member (intra-file and
    cross-file).

  It stays `medium` for name matches without an import link, duck/by-name
  fan-out, several candidates, anything over or near the fan-out cap, a
  typed lookup whose type name several declarations share, calls inside
  minified files, and receivers typed by a guess: type propagation now
  marks a rewrite it inferred (`RefRecord::weak_receiver_type`) — a
  variable typed from the file-wide map with no declaration of that type in
  the calling function or at file level, a return-type name match, the
  constructor heuristic, or (Python) a local built through a module
  (`x = mod.Cls()`, `FileFacts::module_typed_locals`). Such a receiver is
  never `high`, through any step. Low stays low.

  Full corpus (283 repositories): 574,663 edges `medium` → `high`, 1,836
  `high` → `medium` (Python, all receivers typed by a guess — pytest
  fixture parameters typed from another function's local, mostly).
  Medium share of all edges 48.2% → 38.6%; share of collapsed mermaid
  arrows with no `high` site 48.1% → 38.7% (Java 78.8% → 50.2%, Rust
  64.7% → 41.0%, Dart 62.8% → 42.2%, C++ 71.6% → 60.4%, Haskell 50.9% →
  28.4%). On `neurosymbolic_kb` 44.6% → 20.2%. Precision of the
  promotions, judged by reading source at site and target: Java 35/35,
  C++ 30/30, Rust 30/30, Kotlin 30/30, C# 30/30, Ruby 30/30, Scala 30/30,
  Python 30/30, Go 30/30, Dart 30/30, after reverting the classes the
  first samples caught (Dart mixin `super`, Python types read through
  `pd.`/dependency bases, Scala imported `Configurable`, Scala anonymous
  `super`, Go `form.F()` through a local named like the package, TS `Log`
  matched by name). What stays `medium` was judged right about half the
  time (40-edge sample across 12 languages).
- **Rust: `use child::x;` and `pub use child::{x, y as z};` in a module
  that declares `mod child` name the child module** (2018 uniform paths),
  so calls through a crate-root re-export (`nkb_community::run_expansion`,
  `nkb_dream::backpressure_snapshot`) bind exactly instead of falling
  through to a by-name fan-out.
- *Performance (WS17):* paired runs on 11 repositories (median of 5),
  none ≥1 s; total 36.9 s → 37.4 s (+1.4%, load-noisy).

### Added

- **`--no-test-entries`** (Python `test_entries=False`, Node
  `testEntries: false`, C ABI `"no_test_entries": true`). Drops the
  harness nodes and their one-arrow-per-case fan from the graph — on
  neurosymbolic_kb, 603 KB of mermaid becomes 571 KB (−5.3%) — while the
  cases stay test roots, so `--dead-code` reports the same findings with
  or without it. Framework entries are untouched.
- **Test, benchmark and fuzz harnesses, and framework-invoked functions
  without a web route, are entries (WS18).** What a harness or framework
  calls now hangs off a `<framework-entry>` origin node with an `entry`
  edge, like a web route, instead of reading as dead code:
  - **One node per harness**, not per test: `<framework-entry>::test::cargo-test`,
    `…::test::pytest`, `…::test::junit`, `…::test::go-testing`, `xunit`,
    `nunit-runner`, `mstest`, `testng`, `py-unittest`, `phpunit`,
    `minitest`, `xctest`, `eunit`, `common-test`, `hspec`, `tasty`,
    `fsharp-nunit`, and the new `cargo-bench`, `jmh`, `caliper`,
    `benchmarkdotnet`, `cucumber`. Every rule of kind `test` now mints its
    node (they only marked roots before) and the dead-code pass treats the
    node as a *test* root, so code only tests reach stays in the
    `only-used-by-tests` bucket.
  - **Rust test attributes are read by their last path segment** and the
    `*_test` macro convention: `#[ntex::test]`, `#[actix_web::test]`,
    `#[googletest::test]`, `#[crate::rt_test]`, `#[wasm_bindgen_test]` were
    matched only as whole paths (315 of the corpus's 1,292 high dead
    findings). A `::`-qualified attribute in the built-in root tables is
    likewise read by its last segment (`#[tauri::command]`); a
    `.`-qualified decorator (`@cache.get`) is not.
  - **Naming conventions with a harness gate**: `Test*`/`Benchmark*`/
    `Fuzz*`/`Example*` (Go), `test*` + `setUp` family (unittest, PHPUnit,
    XCTest, JUnit 3), `test_*` (Minitest), `*_test`/`*_test_` (EUnit),
    Common Test callbacks. Without a base class the name applies only in
    a test file or a file that imports the harness.
  - **Container attributes**: every method of a `#[pymethods]` /
    `#[uniffi::export]` impl (FFI node), of an async-graphql `#[Object]`
    or juniper `#[graphql_object]` impl (one per field), and an rmcp
    `#[tool_handler]` impl.
  - **New framework rules**: Tauri `#[tauri::command]`, RTIC
    `#[init]`/`#[idle]`/`#[task]`, rmcp `#[tool]`, marimo `@app.cell`,
    SQLAlchemy `@event.listens_for` (keyed by event), Unity editor
    attributes (`[MenuItem]` keyed by menu path, `[ContextMenu]`,
    `[DrawGizmo]`, …), Godot `[Rpc]`, .NET exports (`[JSExport]`,
    `[UnmanagedCallersOnly]`, `[DllExport]`, COM register functions);
    registrars for ntex (`.to(h)`, `.service(h)`), salvo (`.get(h)`, …)
    and `atexit.register`. Root-only (no node, in-process wiring): Kotlin
    Dagger/Guice/EventBus, Kotlin and Groovy Gradle task annotations, JPA
    mapping annotations on getters.
  - **Markup bindings** (new `ui` entry kind): a handler named in a C#
    code-behind's sibling markup — `Click="OnSave"` in `X.xaml`/`X.axaml`
    (handler must have the `(sender, …Args)` shape), `@onclick="Save"` in
    `X.razor`, a Godot scene's `[connection signal=… method=…]` in the
    same-stem `.tscn` — enters through one node per event
    (`<framework-entry>::ui::xaml::Click`).
  - PHP stacked attributes (`#[Group('x')] #[Test]`) are recorded one per
    group; they were read as one attribute keyed by the first.
  - Members of a Python/PHP/Ruby test class other than its cases are test
    roots (they were production roots); root coverage counts only the
    tests the built-in conventions recognise, so harness reach does not
    lift a language past the coverage gate on its own.

  *Compatibility:* call edges between real callables are identical across
  the corpus (283 repositories). Entry edges grow 30,857 → 272,162 and
  entry nodes 21,401 → 22,367; mermaid output grows up to ~9% on
  test-heavy trees (neurosymbolic_kb +5.6%, 571 KB → 603 KB). Dead code:
  7,603 findings leave the report (616 high: Rust 468, C# 84, Python 60,
  Java 4); 2,378 appear, 2,365 of them C# in one repository whose
  production root coverage rose from 24% to 25% through the new XAML/Razor
  entries, so its previously withheld findings are reported; none moves up
  in confidence.

### Fixed

- **Pre-0.11.0 audit fixes (WS16).** Each was found by auditing the
  0.10.2→0.11.0 corpus diff and a consumer review of one Rust workspace:
  - **C/C++: a name the `#include` closure declares is read from the
    closure.** `ASSERT(…)` in `erl_trace.c` bound at `high` to the
    `#define ASSERT` of an unrelated test program, and `cmStrCat(…)` to a
    clang-tidy test stub, because the lookup kept one arbitrary callable
    per qualified name. When several definitions share the name, only
    those in the calling file or a file its include closure reaches
    count; a closure that holds only a prototype (`int f(int);`) reaches
    the one body elsewhere, if there is exactly one. Conditional
    alternatives in one header (`#ifdef DEBUG #define ASSERT … #else …`)
    now bind both at `medium` instead of one at `high`.
  - **Solidity: importing a file does not bring a library's functions
    into bare-name scope.** `contains(map, key)` inside `EnumerableMap`
    bound at `high` to `EnumerableSet.contains`. A bare call inside a
    contract or library that declares the name binds to its own functions
    (all overloads of that one function, never handed to an import), and a
    library function (`library-member` attribute) is
    reachable by bare name only from inside its library.
  - **Java/Kotlin/Scala/Groovy: a typed receiver's owner is narrowed by
    the path written.** `Response.Builder().build()` reached every
    `*.Builder.build` (16 in OkHttp); the owners whose path ends with the
    written qualifier are kept, and a type written bare prefers the one
    nested in the caller's own scopes (`MyActor.props` inside
    `Controller4` is `Controller4.MyActor`).
  - **Java/Scala/C++: the argument count picks among overloads.** The
    Java, Scala and C++ plugins record a call's argument count
    (`RefRecord::arity`); an overload set (intra-file, or every overload
    one file declares under the name a cross-file lookup landed on) keeps
    only the overloads whose parameter list can take it — defaults,
    varargs (`T...`, `T*`, packs) and implicit lists read. One-sided: an
    unreadable list stays, and a filter that would empty the set does
    nothing. `new CalculatedField()` no longer also reaches the copy
    constructor, `to_buffer(a, b, c, d, e)` only the five-parameter
    overload. A cross-file lookup that used to return one arbitrary
    member of an overload set now returns the members the count allows.
  - **Rust: `T::from(x)` across several `From<A>` impls** reads the
    argument's type from a variable (`x`, `x.clone()`, `self.f`) — its
    parameter declaration, visible `let` or closure parameter — and binds
    the impl it selects. A field chain or destructured binding is still
    unknown and keeps every impl at `medium`.
  - **Rust: a receiver typed by a crate path** (`kb_common::ConfigSource`)
    binds in that crate, not in every crate that declares a same-named
    type.
  - **Rust: `if let Some(m) = meter` / `Ok(m)`** (also through a tuple
    pattern and `as_ref()`) types `m` as what `meter`'s declared
    `Option`/`Result` holds; before, another function's `let m: Row`
    typed it file-wide and the call was lost.
  - **Rust: `ok`, `err`, `as_deref` are standard-library names**, so an
    untyped `x.parse().ok()` no longer fans out to workspace `ok`
    methods (314 such edges in one workspace).
  - **Protobuf: a `message` is not an RPC.** A package-level message was
    linked to every host method whose owner contained the package's last
    segment (`message Iterator` → 30 `iterator` methods).
  - **asm → C: `jg __dlsym` is `__dlsym`.** The Mach-O symbol prefix is
    one underscore, tried only when the name as written has no C carrier;
    every leading underscore used to be stripped.
  - **Perl: a bareword before `=>` is never a call**, also when the
    grammar parses it as the last argument of a preceding list operator
    (`scalar @{ $x }, releases => …`).
  - **Scala: a base written with constructor arguments**
    (`extends Log[T](a, b)`) is recorded; the class lost its base and
    every inherited bare call (`getLatest()`).
  - **OCaml: `open A.B` opens a submodule of `a.ml`**, so a bare call
    narrows to that file's definition.
  - **Clojure: `alias/f` binds to `f` in the namespace the alias
    requires** (exact qualified name, `high`); namespace metadata
    (`(ns ^{:added …} a.b)`) is no longer part of the namespace name.

  - **The fan-out cap counts distinct callees, not overloads.** A
    candidate set that is one method's overloads (an overloading
    language, one file, one qualified name — after arity narrowing) is
    one certain callee with an open overload: it binds to every surviving
    overload at `medium` whatever `--fanout-cap` is (except `0`, never
    guess), intra-file and cross-file alike. A set spanning owners,
    names, files, a clause language's arities, or same-named functions
    in a language without overloading is still capped.
  - **C: a declaration merged into one file's alternative bodies**
    (`void f(void);` in a header, three `#if` bodies of `f` in one `.c`)
    is reached through the include closure, bound once.

  *Compatibility* (full corpus, 283 repos, against the 0.11.0 candidate):
  Java −183,677 / +56,461 edges (overloads the argument count rules out;
  a lookup that returned one arbitrary overload now returns those the
  count allows: `high` → `medium` on 8,933 such sites); C++ −52,526 /
  +66,877 (include-closure rebinding, arity, uncapped overload sets);
  C −14,668 `high` rebound into the include closure (+14,359 `high`,
  +4,264 `medium`, 1,764 `high` → `medium` where the closure holds
  alternatives in several headers); Scala −21,466 / +9,572 (arity;
  inherited members via recorded bases add 899 `high`); Clojure −3,932 /
  +15,443 (13,436 `high` alias calls); Kotlin −1,503 / +1,525; Rust −401
  / +71; Solidity −52 / +1,726 (own overloads instead of an imported
  library's function); proto −288 (`low`); asm −76; OCaml −306 / +448.
  Overload sets past the cap now bind in C# (+2,722), TypeScript (+2,891),
  Julia (+308), F# (+242), Swift and Groovy (+132 each). Ten Java
  dead-code findings move into the `high` band: overloads whose only
  same-arity callers are in Kotlin (not linked) or nowhere.

- **A call written through a dependency is no longer name-matched onto a
  workspace callable** (Python, JavaScript, TypeScript, Go, Java, Kotlin,
  C#, C++). 0.10.2 taught the Rust resolver that `serde_json::from_str`
  is not the workspace's `from_str`; every other language with an import
  table still bound such calls by name — through duck-typed fan-out, the
  owner-type index, or the bare-name fallbacks — and often at `high`:
  `import requests; requests.post()` → a local `app.util.post`,
  `uuid.NewString()` → a local `IDs.NewString`, `StringUtils.isBlank()`
  (Apache import) → a local `StringUtils`, a Mockito `verify()`/`when()`
  static import → a workspace `verify`/`when`, `ImmutableList.Builder`
  `.build()` → every workspace `Builder.build`, `boost::split` /
  `spdlog::error` → local functions. Whether an import is the
  workspace's is decided structurally from what the tree declares: Python
  module paths and relative imports; `package.json` / `deno.json` names
  and tsconfig `paths` aliases (and `@/`, `~`, `#`, `$` path aliases);
  `go.mod` module paths (with local `replace`s and vendored directories);
  Java/Kotlin `package` declarations; C# namespaces (a bare type name now
  reaches only namespaces the file can see — its own and enclosing ones,
  its `using`s, every `global using`); C++ scopes in qualified names,
  type and namespace aliases, and the file's include closure (so a
  namespace opened through a macro, `cmsys::`, keeps resolving). A call
  chained off a dependency (`requests.get(u).json()`) is refused only
  when the head is the module itself, never off an imported function
  (`verify(mock).run()`). Plugins now record the package / namespace /
  manifest names (`FileFacts::packages`), C++ marks `ns::f()` as a path
  receiver and records `namespace fs = …` aliases.
- A refused site is recorded, never silent: new unresolved reason
  `external-module` (`{"module": …}`), filed in the per-file *external*
  bucket in place of the intra-file record, so it is counted once.
- C++: a scope-qualified call (`ns::f()`, `ctype<char>::f()`) binds only
  to a definition declared in that scope — exact path suffix, template
  arguments included, unnamed namespaces transparent, or the constructor
  `ns::Klass::Klass` — never to a member of a class inside the scope
  (`ranges::begin` is not `ranges::drop_view::begin`, `ui::IsText(role)`
  is not `ui::AXNode::IsText`). A declaration the grammar mis-split under
  a macro (`MACRO std::string to_string(…)` named `std::string
  to_string`) is matched by the scope it really sits in, read back from
  its signature, so `std::to_string(i)` no longer binds to it.
- **Java and C# visibility is read from the grammar's modifier nodes**,
  not from the signature text cut at the first `(`. An annotation or
  attribute argument (`@SuppressWarnings("x") public void f()`,
  `[Obsolete("x")] public void F()`), or a comment containing `(` between
  the annotations and the modifier, ended that cut early: a public method
  read as package-private (Java) or private (C#), which the dead-code
  report took as corroboration (`private-visibility`) and ranked `high`.
  Interface members without a modifier are now public, as JLS 9.4 and the
  C# spec say; an explicit `private` interface member (Java 9, C# 8)
  stays private, and a class body nested inside an interface keeps class
  rules. A Java enum constructor without a modifier is private (JLS
  8.9.2). A C# property accessor without its own modifier takes its
  property's accessibility instead of reading as private.
- **A `protected` member no longer reaches `high` dead-code confidence**
  (Java, C#, Kotlin, TypeScript). It contributed no visibility evidence,
  and because a never-referenced finding starts at `high`, it sat in the
  top band exactly like private code — although any subtype, including
  one outside the analyzed tree, may call it. It now carries the
  `public-visibility` evidence (token `protected`), which caps it at
  `medium`. The text report's `risk:` line now prints that evidence with
  its token (`public-visibility:protected`, `public-visibility:pub`).
- TypeScript (and JavaScript, which shares the walker) record class-member
  visibility: `private`, `protected`, `public`, and `#name` (private); a
  class member without a modifier is public, while free functions and
  object-literal methods stay undetermined. Neither plugin claims the
  visibility signal — module-level `export` visibility is not modelled,
  and TypeScript's `private` is compile-time only — so their findings
  stay capped at `medium` and the new evidence only orders findings
  within a band.
- **A generic parameter is typed as the generic, never as one of its type
  arguments** (FINDINGS A2; every language that types parameters).
  Parameter lists were split on every comma, so `Map<String, Handler> h`
  read as two parameters and typed `h` as `Handler` (`h.get()` →
  `Handler.get` at `high`), `Repo<String, Integer> r` as `Integer` and
  `std::map<std::string, Handler>` as `Handler>`. A list is now split only
  between parameters — respecting `<>`, `()`, `[]`, `{}`, string literals
  and `->`/`=>` — and a generic parameter is its outer type, named as
  `name: Map<K, V>` and Java/Kotlin generic locals already were (a
  one-argument `Box<String>` is `Box`, not the malformed `Box<String`, so
  it now resolves through the file's imports). The parameter list itself
  is found structurally: the first parenthesised group that is not an
  annotation's (`@SuppressWarnings("x")`), an attribute's (`[HttpGet("x")]`,
  `#[DataProvider('p')]`) or a keyword's (Rust `pub(crate)`, C/C++
  `__attribute__((…))`, `decltype(…)`). It used to run from the first `(`
  to the last `)`, so the parameters of a method behind an annotation
  argument, and the first parameter of every Rust `pub(crate)` function,
  were never typed. Dart's named and optional groups (`{Type a}`,
  `[Type a]`) are opened up.
- C++: a class template is its own type (`QMap<Key, Handler>` → `QMap`,
  `std::vector<Widget>` → `vector`), and a call through `->` on a variable
  declared as one reaches its first argument (`sk_sp<DisplayList> dl;
  dl->Dispatch()` → `DisplayList::Dispatch`, whatever the smart pointer is
  called). Every template, containers included, used to be read as its
  first argument for every call, so `widgets.size()` reached
  `Widget::size` and `dl.reset()` `DisplayList::reset`. The plugin marks
  `->` calls (`RefRecord::arrow`) and records the pointee under a `#deref`
  local / field / parameter entry.
- **Kotlin records a call's receiver whatever its shape** (FINDINGS A3).
  Only a plain identifier was kept: `this.x.m()`, `a.b.m()`, `Foo().m()`,
  `x.a().b()`, `(…).m()`, `x!!.m()`, `super.m()` and package-qualified calls
  got an empty receiver and were resolved as bare names —
  `other.stream.close()` bound to the enclosing class's own `close` at
  `high`, `super.onCreate()` to the override itself. Receivers are now
  rebuilt from the tree the way the other plugins record theirs (`a.b`,
  `this.x`, `x.a()`, `(…)`), without line breaks and with argument lists
  (often whole lambda bodies) elided; `x!!` is `x`; a constructor call
  `Foo(…)` is its type, as `val x = Foo(…)` is typed; `this@Outer` is
  `Outer`. `kotlinx.coroutines.runBlocking { }` and `File(p).readText()`
  now reach the external-module refusal.
- Kotlin extension functions. `expr.ext()` resolves to an extension in
  scope — declared in the calling file or imported by name, also through a
  companion object (`okhttp3.HttpUrl.Companion.toHttpUrl`) — unless the
  receiver's known type declares that member; a single import-bound
  extension is `high`. Duck-typed fan-out no longer offers an extension the
  caller cannot see (another package, not imported) nor a top-level
  function that is not an extension. A superclass written as a constructor
  call (`: Base()`) or a delegation (`I by x`) is recorded as a base, and
  every class and object as a class record, so inherited methods resolve.
- **`super.m()` / `super().m()` resolves through the caller's declared
  base chain** (every language whose plugin records bases). It was
  name-matched onto every same-named method except the caller's own:
  `super.onCreate()` reached every workspace `onCreate`. Where the plugin
  records every class (Python, Kotlin), a chain that leaves the tree is
  refused as `super-base-out-of-graph`; elsewhere a miss keeps the old
  reading.
- Java, Kotlin, C#: a bare call to a member the calling class declares or
  inherits, whose name the file also imports from a dependency, is that
  member — not every same-named callable in the tree.
- Kotlin visibility is read from the `visibility_modifier` token, as Java
  and C# now are: an annotation argument no longer hides `private`.
- Python: a receiver chain wrapped over lines (`(self\n  .repo\n
  .quux())`) is rebuilt as `self.repo`, keeping the self-field type the
  one-line form has (FINDINGS A7).
- **A call written through an import of a workspace module binds inside
  that module** (FINDINGS A4, A5; JavaScript, TypeScript, Python, Go).
  JavaScript/TypeScript qualified names carry no module, so `svc.spawnJob()`
  after `import * as svc from './svc'`, `legacy.run()` after `const legacy =
  require('./legacy')` and a named import's `spawnJob()` were looked up by
  bare name and bound to whichever same-named function was indexed last —
  a decoy in `src/zzz/` won. Python and Go module-qualified calls
  (`pkg.sub.jobs.Runner().m()`, `from . import helpers; helpers.f()`, `r :=
  jobs.NewRunner(); r.M()`) bound the right target plus every same-named
  method duck typing found. The import is now followed to the file it
  names — for a relative JS/TS specifier the file Node/TypeScript would
  load (extension and `index` probing, `.js` written for a `.ts` source),
  for a workspace package its own directory (`index` or Deno `mod` entry),
  for a path alias or a Deno root-relative `/x.ts` every file whose path
  ends like it, ranked by the importer's own package and then by
  proximity; a Python module by its dotted name, relative imports made
  absolute; a Go package by its import path below `go.mod` — and the call
  is looked up there, through the module's re-exports (`export *`, `export
  { a as b } from`, `export * as ns`, `module.exports = { … }` /
  `require(…)` / `new X()`, a package `__init__.py`'s `from .x import y`
  and star imports). A typed or constructed receiver binds to that type's
  methods and bases: `new svc.Runner().start()`, `pkg.Runner().m()`, a Go
  value a package function returned (`r, err := jobs.Open(p)`, by the
  function's declared result type — another package's type is looked up
  there, a dependency's is filed as external — typed from the end of its
  `:=` to the end of its block), a parameter of a package type (`t
  *util.T`), and a Go bare call in the calling package's own directory. A
  single exact binding is `high`.
- The call is refused and recorded as `not-in-imported-module`
  (`{"module": …}`) only when that lookup is decisive: the receiver is the
  module itself (`mod.f()`, `import { f }; f()`) and the module — every
  file an alias may name — defines nothing by that name, or the module's
  file cannot be found. A path through a module-level value (`pkg.Var.m()`,
  `mod.instance.m()`, a default export cgg cannot name), the result of a
  call (`useThing().m()`, a builder chain off `new X()`), a class whose
  bases may hold the method, a Go type whose embedded fields may promote
  it, or several files that each provide a different callable, keeps the
  resolver's earlier readings.
- Plugins record what that needs: JS/TS `re-export` and `local-export`
  import records (`export … from`, `export { a as b }`, `export default
  X`, CommonJS `module.exports` / `exports.k`) and TypeScript `import x =
  require('./m')`; Python's importable module name in `FileFacts::packages`;
  Go's own package import path (a `package-path` record) and values made
  through an imported package (`r := jobs.NewRunner()` is typed as that
  call, `w := &jobs.Runner{}` as `jobs.Runner`); Go parameters keep a
  package qualifier (`t *util.T` is `util.T`); a JS/TS file's own package
  (`package-name`) and a default export it cannot name (`<value>`). A Go
  method on a generic type keeps its owner (`func (m *Map[K, V]) Get` is
  `pkg.Map.Get`, was `pkg.Get`).
- **A bare call no longer binds to a method of an unrelated type, nor to
  another package's sub** (FINDINGS A6). The cross-file name fallbacks
  (the scoped and the global by-name lookups) now apply each language's
  unqualified-name scope, as the intra-file linker already did for some:
  Python, Rust, JS/TS and PHP never reach a method (`collect()` is not
  `Dates::collect`, `list(x)` not a generated `Command.list`); Go reaches
  its own package's directory only; Perl its own package only, intra-file
  too (`zlist()` in `package main` is not `Repo::zlist`); Java and C#
  reach constructors, statically imported members, and the members of the
  calling class, its enclosing classes, their bases and — by dispatch —
  the overrides in classes derived from them. A definition nested in the
  calling function is always in scope. Kotlin, Groovy, Scala, C++, Swift,
  Dart and Objective-C are unchanged: lambdas with receiver, closure
  delegates, argument-dependent lookup and unrecorded base types make
  their bare-name scope a matter of types cgg does not have. Perl now
  records `Pkg->m()`'s class as its receiver; it read as a bare `m()`.
- Java `import static a.b.C.m;` resolves exactly (the member name was
  appended to a path that already ended in it, so it never matched and
  fell to name matching), `import static a.b.C.*;` is recorded as a
  wildcard, and C# `using static` brings a class's members into scope.
- Same-named callables are never chosen by indexing order. The scoped
  by-name lookup returns the callable in the file it matched (it returned
  the last callable with that qualified name anywhere — a Zig `assert`
  copied into a test case won over `std.debug`'s). Copies of one qualified
  name are narrowed by evidence to one — the copy in a module the file
  imports (path, segment-aligned: `-import(lists, …)` is never `lists1`),
  else the caller's own file, else the one in its directory — and with no
  evidence bind nothing, recorded; the same holds for the bare-name
  fallback through a module alias (`const Type = @import("Type.zig");
  Type.fromInterned()` is `Type.zig`'s, an external `matcher.expect()`
  none of the workspace's `expect`s).
- A copy is a file: the overloads or clauses one file declares under a
  name stay together (a two-arity Erlang function in the caller's
  directory was dropped as two copies). Copies of the very module a call
  names (`cerl:f` in the compiler and in two test-data suites) resolve to
  the one outside test code when neither the caller's file nor its
  directory holds one.
- A local shadows an import of the same name. Go records a parameter or
  `:=` local named like an import binding (`c *app.RequestContext` in a
  file importing `c ".../client"`) over its scope, and a Python, JS or TS
  parameter named like an import is the receiver, not the module: such a
  call keeps the reading it had rather than being refused as a miss in
  the package (FINDINGS R9).
- Java and C# resolve a bare name to a member of the calling class or one
  it inherits before any import (JLS 15.12.1), binding the whole overload
  set; an import or `import static q.C.*` reaching an overloaded member
  binds the set too. Overloaded inherited methods (`testQuery` on a
  shared test base) and statically imported overloads (`CookieUtil.add`)
  were not bound at all (FINDINGS R10).
- JS/TS: a package's `exports` map (`package.json`, `deno.json`;
  conditional targets and `*` patterns) names the file a sub-path
  specifier reaches (`@std/assert/instance-of` → `instance_of.ts`,
  FINDINGS R11).
- External packages are labelled external, not refused (FINDINGS R12). A
  bare JS/TS specifier is matched only whole and inside the importer's own
  package (`baseUrl`, `NODE_PATH`) unless it is a path alias (`@/`, `~`,
  `#`, `$`, a tsconfig `paths` prefix or catch-all `"*"`) or a package the
  tree names: `next/navigation` no longer reaches a `navigation.ts`, nor
  `react` another package's `react/` directory. A specifier that names no
  file is a dependency; so is one matched only by that guess (a test stub
  of the package) that lacks the name; and a workspace package that
  re-exports a dependency (`import dayjs from "dayjs"; export default
  dayjs`, `export * from "@testing-library/react"`) labels the call with
  the dependency. Go in module mode: an import path no `go.mod` covers is
  a dependency even when a directory shares its name (`reflect`).
- JS/TS refusals need a module cgg can read: a value it cannot
  (`module.exports = { ...x }`, a default export naming a variable,
  TypeScript `export = make()`) is no proof any name is absent;
  `module.exports = { ...require('./x') }` re-exports `./x`'s names;
  `module.exports = require('./m')` and `export = require('./m')` carry
  `./m`'s value as their own (an instance's class methods resolve); a
  relative module cgg did not analyze (`types.d.ts`) is decisive only for
  `mod.f()`; a default-exported class with no constructor is
  `class-without-explicit-init`.
- Zig: `const addRunArtifact = link.addRunArtifact;` is recorded (an
  `alias` import record) and followed to the file `link` is, and a bare
  name the calling file declares is never bound to another file's copy
  (FINDINGS R13).
- Erlang `m:f(…)` names module `m` (FINDINGS R14). The grammar nests the
  call inside a `remote` node and the plugin had read only the call, so
  every remote call was a bare `f`: `wxWindow:refreshRect(…)` inside
  `wxDialog:refreshRect` bound to itself, `lists:map` to a local `map`.
  The call is now `m:f` with receiver `m` and binds `m`'s `f` only — the
  function of the call's arity, `high` when that is one — in-file when
  `m` is the file's own module. Erlang receivers are no longer rewritten
  by type inference (a `file` receiver had become a `File` some function
  returns).
- F#: a bare call never reaches a type's member (FINDINGS R15). Dart: the
  scoped lookup binds only in a file the caller imports, not in a file
  whose path merely contains an imported name (FINDINGS R16).
- Java/C#: the calling class a bare name resolves in is its qualified
  owner, not its bare name (FINDINGS R17). The member lookup added for
  R10 keyed the class by its last segment, so a call inside a nested
  `Nimbus.X._Fields` bound every generated class's
  `_Fields.findByThriftId` (220 targets) and `unset_aze()` reached
  `Supervisor`'s copy; the bare-call scope test likewise accepted any
  class sharing the caller's bare name. A class now counts by its
  qualified path, or through a recorded base.
- Java/C#: copies of one class (one qualified name vendored into several
  samples or modules) narrow to the caller's copy — the one sharing the
  longest directory prefix with the calling file; a tie is refused, never
  fanned out (FINDINGS R18). `InitializeComponent()` in a WinForms sample
  no longer reaches every sample's designer file.
- Java/C#: a bare call to a member that a class derived from the calling
  class overrides is a virtual call: it binds the member and those
  overrides (never a sibling class's), and over `--fanout-cap` it is
  refused and recorded as `fanout-cap-exceeded` (FINDINGS R20). Binding
  the declaration alone had left every override looking uncalled (285
  dead findings newly `high`, `ImmutableActionSet.EmptySet.addOne`).
- Zig: `const assert = std.debug.assert;` follows the alias to the module
  the tree holds (`lib/std/debug.zig`), at `high`; where the aliased
  module is not in the tree (`std` outside the Zig repository) the call is
  external, never name-matched to a workspace `assert` (FINDINGS R19:
  7,255 wrong `assert` edges in `tigerbeetle`, 125 in `ghostty`).

- **Parameters and locals shadow other names** (WS14; JavaScript,
  TypeScript, Python, Lua, Scala, Kotlin, Swift, Rust, Dart, OCaml, Zig).
  Plugins now record every parameter and local variable with the span it
  is in scope over (`FileFacts::locals`, from grammar node kinds). A bare
  call through one — `cb()` with a parameter `cb`, `t("…")` after `const
  { t } = useLocale()`, `f x` inside an OCaml function taking `f`, a
  pytest fixture parameter — calls the value it holds, so it is no longer
  name-matched onto a same-named function elsewhere; the site is recorded
  as the new unresolved reason `shadowed-by-local`. A declaration is in
  scope after its initializer (`let now = now();` still reaches the outer
  `now`); a local that is the import its declaration makes (`const m =
  require('./m')`), one aliasing a same-named member (`var pick =
  H.pick`), or one named like an import of the file keeps the import
  reading; a typed local still enters its type's call operator; a local
  may hold a closure another function of the file returns. A receiver
  headed by a local is that value, not a module of the same name.
  Minified bundles (a line over 2,000 bytes) are left as they were.
- Lexical scope for bare calls beyond Python: JavaScript, TypeScript,
  Lua, OCaml, F#, Scala, Kotlin, Swift, Rust, Dart, Haskell and Julia
  bind a call to the innermost visible nested definition (three local
  `let rec go` in one OCaml file), and in OCaml, F# and Lua a later
  same-scope binding shadows an earlier one.
- **Dart library scope.** A bare call binds to what the file's library
  (its parts) and its imports provide — the imported library's top-level
  functions and everything it `export`s, filtered by `show`/`hide`;
  `package:<name>/x.dart` is `lib/x.dart` of the in-tree `pubspec.yaml`
  naming `<name>`; a prefixed import is reached only as `p.f()`. A name
  no in-tree import provides is external (`package:test`'s `expect` no
  longer binds to a workspace file declaring an `expect`). The plugin now
  records `export`, `show`/`hide`, `part`/`part of` and the pubspec
  package.
- **PHP namespaces.** `X::m()` binds the method of the class `X` names
  through the file's namespace and `use` table (grouped `use A\{B, C}`
  included), a dependency's class of the same short name is external,
  `parent::m()` is the caller's base's, and `f()` is the namespace's
  function, else the global one. Copies of one function in several files
  narrow to the one the caller's `require`/`include` chain reaches, or
  bind nothing. The plugin records each class's qualified name and its
  `require`/`include` statements.
- **Zig module paths.** A call through file-level `@import`s and aliases
  (`std.debug.print()`, a typed receiver `pt.navAlignment()` with `pt:
  Zcu.PerThread`) binds the function the file the path ends in declares,
  following `pub const f = @import("x.zig").f` re-exports; a module a
  `build.zig` declares with `addModule` is the tree's whatever its root
  file is called (`@import("xev")` → `src/main.zig`); a module the tree
  does not hold is external.
- **Scala.** A bare call reaches the calling class, its bases (`extends
  A with B` now records both), the objects the file imports members of
  (`import a.Obj._`, `import a.Obj.m`, with the object's own bases), and a
  package object's members — never an unrelated class's member
  (`assert` is not some suite's `assert`); a member the calling class
  inherits outranks an import. Traits are scopes (their methods are
  `Trait::m`). An import binds only the name it names.
- Elixir: a bare call reaches the calling module and the modules it
  `import`s or `use`s (options after the module name ignored). Perl: a
  bareword before `=>` is an autoquoted key, not a call.

- **Interface and abstract dispatch no longer leaves overrides looking
  dead** (Java, C#, Python, Rust, and declaration nodes for TypeScript,
  Swift, Scala, Dart). Outside Rust, nothing linked a called base or
  interface member to the members that override it, so an override
  reached only through `Validator v = …; v.member()` was reported dead at
  `high`; Java and C# declared a dispatch model they did not have. Under
  `--dead-code` / `--dynamic-dispatch` the declaration → override fan-out
  that Python had (`dispatch:inheritance`) now runs for every language
  that records class bases (Java, C#, Kotlin, Scala, Groovy, TypeScript,
  JavaScript, Swift, Dart, PHP, Ruby), walking bodiless intermediate
  classes and, in Java, an anonymous class body's own base. A base member
  whose override is called is a dispatch surface (`implemented-trait-surface`,
  as for Rust traits). An override of a contract the tree does not declare
  — `@Override`/`override` with no in-tree base member, a C# explicit
  `void I.M()`, a Rust impl of an outside trait, a Python method of a class
  with an outside base — carries the new evidence
  `implements-external-contract` and is never `high`: the runtime calls
  it. An unresolved same-name site whose receiver is a variable, a type
  parameter or an unknown type, and a site refused over `--fanout-cap`, are
  now doubt rather than contradiction, but only when the site could legally
  call the member (a Java/C# `private` member from outside its class, or a
  package-private one from another package, is never such a site's
  target); a `super.m()` refused as outside the tree is no doubt for
  in-tree members (except Rust, whose `super::` is a module).
- Java and C# record every class with its bases (`FileFacts::classes`), so
  `super.m()` / `base.M()` whose chain leaves the tree is refused as
  `super-base-out-of-graph` instead of being name-matched onto an unrelated
  workspace member (`base.OnPaint()` → `Other.OnPaint`). C# `base.M()` is
  read as `super` and now binds to an in-tree base member. An inherited
  member reached through a chain of same-named class copies narrows to the
  copy nearest the caller (`nearest_class_copies`).
- TypeScript interface members and `abstract` methods, Swift protocol
  requirements, Scala abstract `def`s and Dart abstract methods are emitted
  as bodiless declaration nodes (tagged `abstract`), so a call typed to the
  interface binds to its declaration — the Rust model — and dispatch fans
  out from there. Duck-typed fan-out and in-file bare-call resolution prefer
  a concrete candidate over such a declaration. TypeScript `abstract class`
  now owns its members (`A.draw`, was `draw`), `extends A implements B`
  records both bases, Swift and Dart record supertypes, and Dart mixins own
  their members.

- **Scope and type follow-ups to WS14** (WS14b; Rust, Zig, Perl, PHP,
  Scala, Kotlin, OCaml).
  - Rust: a pattern binding is in scope after its value, not in it —
    `if let Some(parts) = parts(..)` / `while let` / `for x in x()` call
    the outer function again, and the binding now covers the `if`/`while`
    body (not its `else`). A macro invocation (`err!(…)`) is never
    shadowed by a value of the same name (`RefRecord::macro_call`).
  - Zig: locals are typed from what is written (`const x: ?*T`, `T{…}`)
    or from what the called function is declared to return (`self.m()
    orelse return`, `try IO.init(…)`, another file's function followed
    through its return type), parameters with optional/pointer/`const`
    markers peeled. A receiver typed as a container reaches that
    container's member — containers are recorded with their spans
    (`FileFacts::type_spans`), including `@This()` aliases and
    container-level aliases such as `pub const frame =
    Functions(@This()).frame;` / `const C = Common(Self)` — and a name the
    container does not declare is refused. A call on a type function's
    result (`list.Managed(T).init()`) reaches the struct it returns, or is
    external / refused, never a same-named `init` of the caller's file.
    A function-local `const X = @import("x.zig").X` is followed; `const
    fatal = std.process.fatal;` in a function no longer shadows. A typed
    receiver the lookup cannot follow is read as written (as C++).
  - Perl: `$self->m()` on the sub's invocant (`my $self = shift;`, `my
    ($c, …) = @_;`, a signature's first parameter, `shift->m`) binds the
    caller package's `m`, else the first up its bases (`use parent`/`use
    base`, `@ISA`, and — from a small stdlib table — `use Mojo::Base
    'X'`, Moose/Moo `extends`/`with`); inherited from a base outside the
    tree it is external. `$self->SUPER::m()` starts at the bases.
  - PHP: of several global definitions none of which the caller's include
    chain reaches, one declared inside a conditional (`if
    (!function_exists('f'))`) yields to the one unconditional definition
    (medium); two unconditional copies stay refused.
  - Scala: a bare call reaches the member of the type around it, else its
    bases, each base resolved through the declaring file's imports
    (exact, relative, wildcard) and package — not by simple name — and a
    companion object's members are not the class's. `this(…)` in an
    auxiliary constructor is a construction of the class, not a call to
    its `def this` members. A bare name an explicit import brings in from
    a package the tree does not declare is `external-module`.
  - Kotlin: inside `x.run { … }`, `x.apply { … }`, `with(x) { … }` a bare
    call is `x`'s member when `x`'s type declares one, else the enclosing
    scope's call it was before (`RefRecord::implicit_receiver`); a
    lambda or object expression sees the parameter types of the function
    around it.
  - OCaml: a call inside a non-`rec` `let f … = …` to `f` is never that
    binding itself (`RefRecord::not_def`); under `let rec` it is.
- Not covered (WS14b): Zig receivers that are field chains
  (`page.rows.ptr()`, `priv.config orelse …`); Perl Moose/Moo `has`
  accessors; PHP functions with two unconditional copies (WordPress
  `__`); Scala bare calls bound in-file to another class's same-named
  member; Kotlin primary-constructor properties as receiver types.

- **Functions passed as values are references in Java, C# and more Python
  positions; reflective lookups by a literal name are doubt** (WS6,
  FINDINGS B3). Java never read a method reference and C# never read a
  method group, yet both plugins declared the value-reference signal, so
  a method whose only use was `xs.forEach(this::onItem)`,
  `new Thread(this::run)`, `Type::new`, `Select(Double)`,
  `evt += OnChanged`, `new EventHandler(this.OnClick)`,
  `Func<int,int> f = Triple` or `nameof(Named)` was reported `high`
  dead code. Such a site is now recorded as the call through its
  receiver it stands for (`RefRecord::value_ref`) — resolved by the same
  steps a call is (typed receivers, `this`/`super`/`base`, the calling
  class and its bases, imports) — and every edge it yields is tagged
  `reference`. A C# member read through a variable (`value.Value`) is not
  taken as a method group: it is a property far more often. Python now
  also reads `return f`, `yield f`, a bare decorator `@deco`, a default
  `cb=f`, a comprehension value `{k: f for …}`, `x is f` (each kept only
  where the file defines or imports the name, or through `self`/`cls` or
  an imported module), and `a = b = f`. A value-position name no callable
  carries, or that a local binding shadows, is dropped before
  resolution.
  - These sites are recorded only when reference edges are kept
    (`--reference-edges`, or `--dead-code`, which turns it on): **the
    default graph is unchanged** (full corpus, 283 repositories: 0 edges
    added, removed or rescored; 37 more Python `value-ref-*` unresolved
    records, all from chained assignments).
  - Reflection: Java `getMethod`/`getDeclaredMethod("m")`, C#
    `GetMethod("M")`, Ruby `send(:m)` and PHP `call_user_func('f')` are
    now captured (Java's call node has no `function` field, and the C#,
    Ruby and PHP plugins never ran the capture), and a reflective lookup
    by a literal name caps a same-named finding at `medium`
    (`name-collides-with-screened-site`, screen `reflection`) — it never
    becomes an edge. A module load (`require("x")`, `import_module("x")`)
    names a file and caps nothing (screen `module-load`).
  - A plain value reference naming a function or method from its own
    file is doubt for that callable as well as for a closure.
  - Dead code, full corpus: 1,873 findings leave `high` and none enter it
    — Java 1,059 (708 live through a reference from live code, 322
    referenced only from dead code, 26 reflection, 3 unresolved
    reference sites), C# 723 (629 / 75 / 1 / 18), Python 91 (85 / 5 /
    0 / 1). JabRef 267 → 12, Avalonia samples 36 → 4, eShop 22 → 4,
    MediatR 11 → 2, Newtonsoft.Json 144 → 116.
- Not covered (WS6): a C# method group or Python attribute reached
  through a variable (`handler.OnClick`); Kotlin/Scala/Swift/Dart
  function references (`::f`, `f _`); a reflective lookup caps every
  same-named callable of the language, not only the receiver's type's.

- **Methods a framework or runtime invokes by annotation, signature or
  registration are dead-code roots** (WS7, FINDINGS B4). Each was
  reported `high` dead code because nothing in source calls it. New
  framework rules (all `node: false`: they mark a root and mint no entry
  node) cover:
  - Java lifecycle and DI: `@PostConstruct`/`@PreDestroy` (JSR-250),
    Spring `@Bean`/`@EventListener`/`@TransactionalEventListener`, Guice
    `@Provides`/`@ProvidesInto*`/`@Inject`, Dagger `@Provides`/`@Binds`/
    `@Multibinds`/`@ContributesAndroidInjector`, JSR-330 `@Inject`,
    Gradle `@TaskAction` and task input/output getters, Gradle's service
    registry (`@Provides`, and `configure` on a
    `ServiceRegistrationProvider`), Gradle model rules, Jackson/Moshi
    (de)serialization members, EventBus `@Subscribe`.
  - Java serialization: `readObject(ObjectInputStream)`,
    `writeObject(ObjectOutputStream)`, `readObjectNoData()`,
    `readResolve()`, `writeReplace()` — matched by name *and* parameter
    list (`signature_methods_for`), so a `readObject(String)` helper
    stays reportable.
  - Test data sources (`--include-tests`): the member a JUnit
    `@MethodSource("p")` (bare form: the test's own name;
    `Class#m` qualifies the owner) or `@ArgumentsSource(X.class)`, an NUnit
    `[TestCaseSource("X")]`/`nameof(X)`/`typeof(T)`, or an xUnit
    `[MemberData(nameof(X), MemberType = typeof(T))]`/`[ClassData(typeof(T))]`
    names (`member_sources_for`); TestNG `@DataProvider` and lifecycle
    annotations; NUnit `[TestCase]`, `[TestCaseSource]` and fixture hooks;
    JUnit 4 `@Parameters`. Names in attribute arguments are read only to
    root a member of the named (or the annotated) class — never as an edge.
  - C# `[OnDeserialized]`/`[OnDeserializing]`/`[OnSerialized]`/
    `[OnSerializing]` and Newtonsoft `[OnError]`.
  - C/C++: a function named by identifier in `signal`, `atexit`,
    `at_quick_exit`, `on_exit`, `pthread_create`, `pthread_once`,
    `pthread_key_create`, `qsort`, `bsearch` (and `std::call_once`). The
    C plugin now runs the registrar-argument capture the C++ plugin
    already had.
  - The C/C++ → asm FFI linker no longer turns a *value* reference into a
    call edge (`atexit(_fini)`, `run_handler(name, …)`).
  - **The default graph is unchanged except one wrong edge** (full corpus,
    283 repositories, `hcl-terraform-aws` timed out on the old side): one
    C++ `ffi` edge removed (`run_handler(name, value)` bound to an asm
    label `name`); 0 added or rescored; no `<framework-entry>` node or
    edge changes. Audit records: +83 C/C++ `value-ref-*` unresolved
    records for registration arguments that name no callable (data
    pointers in `qsort(entries, …)`).
  - Dead code, full corpus, none newly `high`: 1,644 findings leave `high`
    — Java 1,609 (DI providers 704, serialization hooks 469, Dagger
    bindings 217, Gradle model rules 78, Spring 53, Jackson/Moshi 34,
    Gradle tasks 16, service-provider `configure` 13, JSR-250 10, other
    live through a newly rooted caller 15), C# 35 (serialization
    callbacks). With `--include-tests`: 2,178 (Java 2,130, adding 440
    test data sources; C# 48). groovy-gradle 639 → 44, storm 453 → 11,
    kapua 240 → 11, nextcloud-android 164 → 4, JabRef with tests
    304 → 10, Newtonsoft.Json 116 → 83.
- Not covered (WS7): NUnit `[ValueSource]` (parameter attributes are not
  recorded); a data source inherited from a base test class and not
  qualified; Kotlin/Groovy copies of these annotations; C `sigaction`
  handlers, function pointers in struct fields, `std::thread t(fn)` and
  algorithm comparators; Newtonsoft.Json's `#if`-split methods misparsed
  as callables named `if` (extraction, WS8).

- **Extraction bugs across twelve languages (WS8, FINDINGS A8, A9, A10,
  C5).** Each one either hid callables (so their calls had no source or
  no target) or minted callables and edges that do not exist:
  - **Dart**: a class method's callable covered only its signature, so
    every call in a method body had no source (dart-flame: zero edges
    from any method). Methods, getters, setters, operators and
    constructors now span their bodies. A bare call inside a method is
    first the enclosing class's member, and across files reaches only a
    library function or a member of the calling class's ancestry, never
    a method of an unrelated class.
  - **Scala**: `b.m()` was recorded as one name `b.m`; the member and its
    receiver are now split (`new T(…).m()` reads as a `T` receiver).
    Paren-less sites (`b.size`, a bare `helper`) are recorded too, kept
    only when some `def` is declared without a parameter list under that
    name, bound only through a typed receiver or scope (never by name
    fan-out), and leave no audit record when they bind nothing (a value
    read, not a call). The same rule applies to the new paren-less sites
    of Ruby and is what `RefRecord::paren_less` marks.
  - **C**: functions returning a pointer (`char *f(void)`) were dropped;
    a pointer-to-function variable (`int (*fp)(int);`) was recorded as a
    function; a macro invocation with no `;` (`LIST(X)` then `int
    helper(void) {…}`) named the next function after the macro and drew
    an edge to it (A8).
  - **C/C++ headers**: a `.h` whose only C++ is `virtual` members was read
    as C and lost them.
  - **C++**: a local constructed with arguments (`std::ifstream
    info(path);`) became a callable (A9); `ns::Foo x` parameters were
    read as `name: Type` and named `ns`; a definition under a macro
    folded its return type into its qualified name (`std::string
    to_string`, `ERL_NIF_TERM wxeReturn::make_bool`).
  - **Julia**: every `function f()` drew a `high` self-edge from its own
    signature (A10).
  - **C#**: `#if`/`#else` splitting a statement or a method header
    produced callables named `if` and namespaces named after `#if`
    symbols (Newtonsoft.Json); a file that does not parse is re-read with
    its conditional directives blanked (offsets unchanged). `new
    Alpha().Run()` now resolves on `Alpha` and up its bases; a call
    through a typed receiver its type does not declare reaches an
    extension method on that type (`p.Faker()`, `new P().Faker()`).
    `partial` method parts cap at medium in the dead-code report.
  - **Ruby**: a bare paren-less call (`helper`) is recorded unless a local
    of that name is in scope (Ruby's own rule); a bare call reaches the
    calling class's method, a superclass's or an included module's, or a
    top-level one — never an unrelated class's method.
  - **PHP**: `$this->m()` bound nothing; it now resolves on the calling
    class, its bases and its `use`d traits, and through a dependency's
    base is external (`$this->assertCount()` is PHPUnit's).
  - **Lua**: `function M.f()` was named `M.f` as one simple name and
    calls as `M.f` / `obj:m` text, so cross-file calls never matched;
    names and receivers are split, assigned functions (`M.f = function`)
    and named-table fields are callables, a call through a `require`d
    module binds in that module's file (a module no file is: external),
    and a bare call never reaches a table member.
  - **Elixir**: a capture `&f/1` / `&M.f/2` is a value reference of that
    arity (with `--reference-edges` / `--dead-code`), and its operand is
    no longer a phantom call to `f/0`.
  - **Python**: a local bound to its own lambda (`f = lambda: …; return
    f`) is that function when named as a value.
  - **Kotlin**: a top-level one-line `object X { fun f() = 1 }`, which the
    grammar misparses, keeps its scope (`X.f`).
  - **Java**: the `static` checks read the keyword token, not text an
    annotation argument can contain.
- Already fixed by earlier work: TypeScript `private`/`protected`
  visibility (WS2) and `abstract class` scope (WS5).
- **Calls written with explicit type arguments are calls of the named
  function** (WS9, FINDINGS C1, B5; C++, Go, Dart, C#, Scala, Swift, F#,
  Julia). Each was either dropped silently or recorded under a name
  that kept its type arguments, which no callable carries, so the site
  was screened as external:
  - **C++**: `f<int>()` (`template_function`) was skipped outright;
    `b.m<int>()`, `B::s<int>()`, `ns::f<int>()`, `b.template m<int>()`
    and `x.Base::template f<int>()` kept `<int>` in the name. The name
    is now read from the tree without its template arguments or the
    `template` disambiguator, and the scope is split on the tree's
    nested qualified names, so `::` inside an argument (`f<std::string>`)
    no longer cuts the name.
  - **Go**: `F[int](x)` and `pkg.F[*T](err)` parse as a conversion to a
    generic type, and `F[int](a, b)` as a call through an index
    expression; both were dropped. They are recorded as calls when the
    head is a bare name or a package the file imports (Go methods take
    no type parameters, so `x.f[k](v)` on a value stays an element
    read, as does any index that is a literal or an expression).
  - **Dart** `f<int>()` / `b.m<T>()` (`instantiation_expression`) and
    **Scala** `f[Int]()` / `b.m[Int]()` (`generic_function`, named
    `f[Int]`) are calls of what they wrap.
  - **C#**: `a.M<T>()` kept the name `M<string>`; `x?.M()` was dropped
    (B5). Both are recorded as the plain member call, and a
    null-conditional receiver (`x?.y.M()`) reads as `x.y`.
  - **Swift**: `Wrap<Int>.s()` in expression position is parsed as two
    comparisons around an implicit-member call. Comparison operators do
    not associate in Swift, so that nesting (with no space before `<` or
    between `>` and `.`) is read as the static call `Wrap.s()`.
  - **F#** `f<int> x` (a type application) was dropped; **Julia**
    `Foo{Int}(x)` kept `{Int}` in the name, and a short-form method with
    type parameters or a return type (`f(x::T) where {T} = …`,
    `h(x)::Int = …`) was not a definition, its head recorded as a call
    to itself.
  - Java, Kotlin, TypeScript, Rust (turbofish) and Groovy already
    resolved these forms.
- **The step-1c scoped lookup is evaluated per name** (cross-file
  resolver). Building every file's map of same-named definitions in
  files matching its import fragments read every file of the language
  against every fragment, for every file: 20 s of CPU on
  `hcl-terraform-aws`, which therefore timed out at
  `CGG_REPO_TIMEOUT=60` under corpus load. The answer for each name is
  unchanged (same candidates, same order, same cap of 8).
- **A receiver written in a non-bare shape resolves as its bare, typed
  form** (WS10, FINDINGS C2). Every plugin recorded a receiver as its
  raw text, so `(f).m()`, `((Foo) o).m()`, `(o as Foo).M()`,
  `(await x).m()`, `f!.m()` and `(*p).m()` bound nothing, and a
  construction (`new Foo().m()`, whose text starts with a lowercase
  `new`) was fanned out by name to every same-named method, decoys
  included. A shared reader (`cgg_lang::receiver`) now looks through
  parentheses, `await`/`try`, non-null assertions and dereferences, and
  reads casts and constructions as the type they name, by grammar node
  kind: Java/C#/C/C++ casts, `as` (C#, TypeScript, Kotlin `as`/`as?`,
  Swift `as!`/`as?`, Dart), TypeScript `<Foo>x`, Go `x.(T)`, C++
  `static_cast<T*>(x)` and Scala `x.asInstanceOf[T]` (cast operators
  spelled as calls are listed in `cgg_core::stdlib::CAST_INTRINSICS`);
  `new Foo(…)` (Java, C# with or without an initializer, JS/TS, PHP,
  Scala, Groovy), `Foo(…)` where the language constructs by calling the
  type (Python, Kotlin, Swift, Dart, Scala, C++), `Foo{…}` / `&Foo{…}`
  (Go, C++) and Ruby `Foo.new`. Such a receiver is flagged
  (`RefRecord::typed_receiver`): it is looked up on its type and up its
  bases whatever its case (Go's `&server{}`), is never fanned out by
  name, and keeps C# extension-method and Solidity `using` lookups. A
  parenthesised expression with no name (`(a + b).m()`) and a call's
  result (`make().m()`) keep their old reading. Python: a receiver type
  the calling file declares is that file's class, never a same-named
  class of another file.
  - **TypeScript**: `.ts`/`.mts`/`.cts` files are parsed with the
    TypeScript grammar instead of TSX (`LanguagePlugin::ts_language_for`).
    TSX reads `<Foo>x` and `<T>(x) => …` as an unclosed JSX element and
    lost every call after it in the file.
  - **Null-conditional calls**: Dart `x?.m()` and PHP `$x?->m()` were
    never recorded; they are now the member call they are (C# `x?.M()`
    was already fixed by WS9).
  - **Solidity `using L for T;`** (and `for *`): a call on a value typed
    `T` that `T` does not declare reaches library `L`'s function.
  - **Scala**: calls through a dependency are refused like Java's —
    imports are judged against the tree's packages, and a receiver
    headed by a package every file sees without an import (`math.ceil`,
    `sys.error`, `java.util.UUID`; `cgg_core::stdlib::SCALA_IMPLICIT_PACKAGES`)
    is external unless a local, a member `val`, a package segment or a
    class of that name is in the tree. A bare call to a member of the
    caller's class beats an external import, as in Java/Kotlin/C#.
- **Optional, nullable, union and wrapper annotations type a value as the
  class they name** (WS11, FINDINGS C3). Such an annotation used to be
  typed *worse* than no annotation: Python `x: Optional[Foo]` gave the
  receiver `Optional`, which was booked as a stdlib call; `x: Foo | None`,
  `x: "Foo"`, TS `x: Foo | undefined`, Swift `Foo?`/`Foo!`, Kotlin and
  C# `Foo?`, PHP `?Foo $x` / `Foo|null $x`, Dart `Foo? x` and Scala
  `Foo | Null` gave a receiver no type is named, so the call was left
  unresolved. A shared reader (`cgg_core::declared_type`) now removes
  nullability markers, null union members and "maybe" wrappers, and
  unquotes Python forward references. The language-specific names live
  in `cgg_core::stdlib` (`MAYBE_TYPE_WRAPPERS`, `UNION_TYPE_CONSTRUCTORS`,
  `NULL_TYPE_MEMBERS`). Containers (`list[Foo]`, `Option<T>`, Java
  `Optional<T>`) keep their own type. A union of several real types
  (`Foo | Bar`) is left untyped and gets the reading an unannotated value
  gets (medium, capped name fan-out). It is never bound to its first
  member, and a TS return type `Foo | Bar` no longer returns `Foo`.
  The reader applies to parameters, return types (`-> Optional[Foo]`,
  `-> Foo?`, `): ?Foo`), Python class and `self.x: T` field annotations,
  and Kotlin/C# nullable locals and properties. Related fixes:
  - A default value is no longer part of a parameter's type: `x: Foo =
    None` was typed `Foo = None`, and a C# `Foo x = null` was named
    `null`.
  - TS optional parameters (`x?: Foo`) are named `x`.
  - Type-first parameters skip annotations and attributes (`@Nullable Foo
    x`, `[FromBody] Foo x`) and the modifiers Dart `required`/`covariant`,
    C# `ref`/`in`/`params`/`this`, and PHP promoted-property visibility.
  - A Kotlin nested type (`Hpack.Reader?`) is its inner type.
  - C# and Kotlin locals are looked up in the calling function first. A
    file-wide last-write-wins map retyped `JArray a` as another method's
    `IEnumerable? a`.
  - C# extension methods whose type-parameter list carries an attribute
    with arguments are recognised.
- **Closure parameters are typed where the type is written or declared**
  (WS11; Dart, Scala, C#). `(Foo x) => x.m()`, `(x: Foo) => x.m()` and
  `case x: Foo => x.m()` type `x` within the closure's own span
  (`FileFacts::scoped_types`). An untyped closure passed positionally to
  a bare call takes its parameter types from the callee's declared
  callback type (`void each(void Function(Foo) cb)`, `def withTempDir(f:
  File => Unit)`, `Action<Foo>` / `Func<Foo, R>`;
  `FileFacts::closure_args`, `type_hints::settle_closure_args`). This
  happens only when every same-language definition of that name agrees,
  the type is concrete (not a type parameter or `Any`/`Object`), and the
  arity matches; otherwise the closure keeps its old fan-out. The Dart
  grammar reads `(x) => x.m()` as `((x) => x).m()`, which made the
  closure text the receiver; the call is now recorded on the body
  expression. Dart `x!.m()` is read as `x.m()`.
  - A/B (283 repos, none excluded), by confidence:
    - **HIGH net:** Python +415 (+125 of them medium→high), PHP +247,
      Dart +220, Scala +118, TS +68, Java +59, Kotlin +41, C# +24.
    - **MEDIUM net:** Dart −68.6k (Flutter `testWidgets((WidgetTester
      tester) …)` callbacks: each `tester.pump()` bound to 4–5 same-named
      decoys, now to `WidgetTester.pump` only), Java −1.0k, Kotlin −0.9k,
      Python −0.2k, Scala +1.3k, C# +0.6k (per-function locals rebind
      2.7k newtonsoft edges to the declared writer/reader type).
    - **Dead code:** 4 findings leave HIGH (2 C#, 2 Python), none
      become HIGH.
- **Function-defining macros generate their functions** (WS12, FINDINGS
  C4; C, C++). `#define DEF_GETTER(n) int get_##n(void) { … }` invoked as
  `DEF_GETTER(x)` defined `get_x` for the compiler but not for cgg: the
  generated names were not callables, calls to them were external or
  unresolved (or bound to a header prototype only), and the macro read as
  dead. An invocation written where items go — file, namespace or class
  scope, with or without a trailing `;`, however the grammar happened to
  parse it — is now expanded one level (parameters substituted, `#`
  stringizing, `##` pasting, `__VA_ARGS__`, GNU `, ## __VA_ARGS__`; no
  rescanning) when the macro is the last same-named `#define` earlier in
  the file or the first one found in the file's `#include "…"` closure,
  and the function definitions of the expansion that parse cleanly
  become callables of the invoking file: methods of the enclosing class
  and namespace in C++, tagged `macro:<name>` (kept out of by-name
  fan-out, reported dead at no more than medium), spans placed inside
  the invocation, calls inside them attributed to them. The expansion
  runs after every C/C++ file is extracted and before header prototypes
  are unified, so a prototype now merges into the generated body
  (`uv__check_close`, `erts_port_task_pre_alloc_init_thread`). What the
  grammar made of the invocation itself (a prototype or field named after
  the macro) is dropped; a macro used as a function header (`TEST(a, b) {
  … }`), an invocation inside a function body, and a file the grammar
  cannot read as a translation unit are left alone. An item-position
  invocation of a function-like macro is credited to it (a reference at
  its first byte, outside every function; the dead-code root
  `toplevel:macro-invocation`), an unexpanded one names its identifier
  arguments (`name_mentions`) and credits an argument that is itself a
  function-like macro (`LIST(DECLARE)`). A function header the grammar
  folded behind such invocations (`DEF(x)\nint f(void) {…}`) now starts at
  its own return type, so the invocations are no longer inside it.
  Plugins record candidates in `FileFacts::fn_macros` /
  `FileFacts::macro_invocations`; `cgg_lang::plugins::c_macro::expand`
  consumes them.
- **A quoted `def` inside a macro is not a definition** (WS12, FINDINGS
  A11; Elixir, Clojure). Elixir `quote do def unquote(name)(…)` became a
  callable named `unquote` with a HIGH self-edge from every `unquote(…)`
  in its module; Clojure `` `(defn ~n …) `` became a callable named `~n`.
  Definitions whose name is unquoted are no longer extracted (`quote`,
  `unquote`, `unquote_splicing` join the Elixir stdlib list). Elixir
  `defmacrop`, `defdelegate`, `defguard` and `defguardp` are now
  definitions. Generating the callable a same-file macro invocation would
  define is not done (the name is usually computed by interpolation).
- **Elixir calls through a module resolve to that module** (WS12b).
  `Mod.f()`, `&Mod.f/1` and `__MODULE__.f()` never resolved: the plugin
  recorded the whole text (`MyApp.Accounts.get`) as the name. The call is
  now recorded as `f` through the module path (`path_receiver`), and the
  resolver binds it to the `f` of the module the path names — through the
  file's `alias A.B`, `alias A.B, as: C` and `alias A.{B, C}` (now one
  normalized `alias` import each), a module nested in the caller's (which
  Elixir aliases automatically), or the full name — picking the clause
  of the call's arity, never another module's `f`
  (`LibIndex::resolve_elixir`). Modules come from `defmodule` (nested
  modules joined with `.`, recorded as `elixir-module` imports). A module
  the tree does not define, and Erlang modules not in the tree
  (`:ets.new`), are external; `:mod.f()` for an Erlang module in the tree
  binds to its `mod:f`. A tree module that declares no `f` is refused
  (`not-in-imported-module`), or external when it `use`s a dependency
  whose macros may have generated it.
  - A/B (283 repos, none excluded), by confidence:
    - **HIGH net:** Elixir +3,919 (4,060 added; 141 removed, all the
      `unquote` self-edges and fake `unquote` callables), C +643, C++
      +322, Clojure 0 (7 edges re-attributed from fake `~name` callables
      to the enclosing `defmacro`).
    - **MEDIUM net:** C++ +324 (994 added; 670 removed, see the
      specialization cap below), C +156, Elixir −4.
    - **Dead code:** no HIGH finding changes. Medium findings fall in
      the repositories that changed: Elixir −831, C −95, Clojure −22,
      C++ −16.
- **Rust (WS13, FINDINGS C6): `let x = Type::f(..)` is typed by `f`'s
  declared return type**, not read as constructing a `Type` (`AssocCallLocal`,
  settled by
  `type_hints::settle_assoc_call_locals` once every file is parsed): a
  return of `Self`/the type (also behind `Result`, `Option`, a pointer
  or a lock) keeps the reading, another named type retypes the local
  (type arguments kept: `-> web::Data<Ctx>` is `Data<Ctx>`), anything
  else leaves it untyped; no declaration in the tree (an external
  `File::open`) keeps the constructor reading. `let x: T = ..` and an
  unqualified struct literal `let x = T { .. }` now type their local,
  and an untyped re-binding (`let r = r.finish();`, `let (r, _) = ..`)
  hides the earlier `let`'s type, as a later `let` hides a parameter's.
- **A type a file `use`s from outside the workspace is not a same-named
  workspace type** (`use actix_web::web::Data;` vs a test-module
  `struct Data`). Only a trait impl a workspace crate wrote for it can
  be the callee (no other crate can write an inherent impl), a wrapper
  form (`Data<Ctx>`) still reaches its argument, and anything else is
  `external-module`. 721 edges were rebound (`context.pool()` →
  `LemmyContext::pool` instead of a test `Data::pool`), 305 refused as
  external, ~560 `Rc::new`/`Cell::new`-style calls on imported std
  types now screened as stdlib. `pub(crate) use a::B;` was recorded as
  the path `(crate) use a::B`; the `use` clause is now read from the
  grammar.
- **Trait overloads respect `--fanout-cap`.** A typed lookup landing on
  several impls of one generic trait for one type (`<Bytes as
  From<u8>>::from`, `<Bytes as From<&str>>::from`, …) binds to the impl
  the argument's literal type selects (`RefRecord::arg_type`: string,
  byte string, char, bool, suffixed number, `format!`,
  `.to_string()`), and otherwise, over the cap, to the trait's single
  in-graph declaration or nothing: a std trait's set is refused
  (`fanout-cap-exceeded`, or `ambiguous-in-file` where the impls share
  the caller's file; ~1,100 medium edges, each set had at most one
  right target). Within the cap nothing changes.
- Dead code: a method declared in a `pub trait` carries `pub` (it was
  `Unknown` and could reach `high`); names an unexpanded item-position
  macro invocation passes (`make_error! { Io(E): has_source, … }`) are
  `name_mentions`; a macro invoked by path outside any function
  (`crate::for_each_tuple!(…)`) is a `toplevel:macro-invocation` root.
  Bare-name item-position invocations (`new_id!(DocId, "doc")`) were
  already credited through `toplevel:invocation`; impls of outside
  traits (Rocket `Fairing::info`) already cap at medium.
- Builtin type lists moved to `cgg_core::rust_types`
  (`SMART_POINTERS_AND_LOCKS`, `DEREF_TRANSPARENT_WRAPPERS`,
  `FIELD_WRAPPERS`, `LOCK_WRAPPERS`, `RETURN_WRAPPERS`) and the
  web-framework response traits/wrappers to
  `cgg_core::frameworks::rules` (`RESPONSE_TRAITS`,
  `SELF_CONVERTING_WRAPPERS`); verified output-identical on 7 Rust
  repositories.
- Not covered: arguments typed only by inference (`Bytes::from(x)`)
  stay refused over the cap; a derive's generated impls are not
  modelled (a `pub trait` it implements is no longer `high`);
  `#[cfg]` duplicates of one method bind one copy from a typed receiver
  (the other can read `high`); the file-wide fallback for a Rust
  receiver with no visible `let` is kept (refusing it was measured to
  drop 161 high edges, ~70% of them right).

### Performance

- **`--dead-code` no longer takes minutes on a tree with a large dead
  region.** Each dead region's confidence was the weakest of its
  members' findings, found by scanning the whole findings list once per
  member — O(dead members × findings). On `zig-zig` (~328k candidate
  callables, one region holding most of them) that scan alone ran for
  ~170 s, so `--dead-code` took 3 min 33 s against a 36 s graph and
  tripped the corpus per-repo cap. Findings are now looked up by id
  from a map built once; the dead-code phase on `zig-zig` drops to
  ~2.7 s. Paired `--no-graph --dead-code` runs (median of 3, `zig-zig`
  1): `zig-zig` 198.0 s → 39.1 s, `erlang-otp` 23.4 s → 8.6 s,
  `dart-flutter` 12.9 s → 10.2 s, `groovy-gradle` 9.2 s → 8.0 s; eight
  other repositories within ±0.2 s. The default graph is untouched
  (7-repo paired subset −0.9%, none ≥1 s). Output is byte-identical
  (graph, audit and dead-code report).
- **Locals collection no longer stalls on generated byte literals.** The
  parameter/local walk (WS14) asked tree-sitter for the field name of
  every node, which is slow on the children of a node with a huge child
  count; one 6.5 MB Qt resource file (`resources_rc.py`, a single bytes
  literal of ~1.5M escape nodes) spent 2.4 s there. Only identifiers and
  nodes the walk descends into need the field now: 0.2 s, output
  byte-identical. `app-snlquest-kivy` 7.8 s → 5.8 s (0.10.2: 5.1 s).
- **Release measurement vs 0.10.2** (`--jobs 1`, 282 repositories): total
  281.9 s → 263.7 s (−6.5%), median per repository +5.2%, 83 faster.
  Known cost: `zig-zig` (the largest tree) is ~12% slower with an audit
  written (97 s → 109 s at `--jobs 1`), almost all from the WS14 Zig
  library scopes (cross-file reference resolution) and the Zig locals
  walk — correctness work, kept; a follow-up will profile both.

### Compatibility

- **WS17 changes confidences, not edges, except in Rust.** 574,663 edges
  rise to `high` and 1,836 Python edges fall to `medium` (see *Changed*);
  a consumer filtering on `high` now keeps far more of the graph, and a
  rendering that marks non-`high` arrows marks far fewer. The Rust
  child-module `use` fix removes 34 edges and adds 112 (5 repositories):
  removed are wrong bindings to another crate's same-named type
  (`salvo_csrf::finder::HeaderFinder::new` from `salvo_jwt_auth`'s tests,
  the `openssl` `Keycert::new` from the `rustls` module, `AppState::new`
  of another example crate) and a by-name fan-out of
  `nkb_community::rebuild` onto two unrelated trait methods; added are the
  calls those paths actually name (87 `salvo` chained builder calls, 11
  aliased re-exports such as `backpressure_snapshot`, 13 `FakeEnv`
  builder calls through `pub use fake::FakeEnv`). Dead-code findings are
  unchanged.
- **The default graph changes.** Full corpus, 283 repositories:
  146,535 edges removed, 208 added. Removed by language: Go 64,584
  (58,597 in `hcl-terraform-aws`, mostly `knownvalue.StringExact` from
  `terraform-plugin-testing` bound to the provider's own `knownvalue`
  package), Java 54,684, Kotlin 8,422, Python 8,096, TypeScript 5,389,
  JavaScript 3,125, C# 1,201, C++ 1,034. Every removed edge class was
  checked by pattern and sampled: calls into a dependency or the standard
  library bound by name, plus — C++ — free-function calls that had bound
  to a same-named class member. Exceptions: 330 removed edges inside
  bundled or minified files (`*.min.js`, webpack bundles) whose
  dependency sources are inlined, and 6 C++ edges demoted high → medium
  whose target's qualified name is mis-split beyond reading back. The 208
  added C++ edges are the right targets those calls now reach: the free
  function or constructor a member had shadowed (`ui::IsTableColumn`,
  `chrono::day::day`), exact template specializations
  (`external_constructor<value_t::array>::construct`), functions in an
  unnamed namespace (`kythe::(anonymous)::main`), and calls a bogus
  receiver rewrite had hidden (`date::format`, `dap::Error`, nlohmann's
  own `std::swap` specialization).
- Graph-level `unresolved` falls by 477,946: calls into dependencies that
  were filed as resolver gaps (fan-out cap, candidates elsewhere) are now
  `external`. High-confidence dead findings rise by 36 (Java 34, C# 2):
  each one's only callers were the removed wrong edges.
- Not covered: a local variable shadowing an import of the same name is
  read as the import (Kotlin `kotlinx.coroutines.runBlocking {}` and
  `File(x).readText()`, listed here before, are covered by the receiver
  work below); JS aliases defined only in bundler config
  (webpack/babel) without a tsconfig/deno entry or a directory of the same
  name; a call chained off an imported *function* (`verify(mock).run()`,
  a tagged template `` sql`…`.execute() ``) keeps its old duck-typed
  reading.
- Visibility (Java, C#, TypeScript; `protected`): **the default graph
  does not change** — full corpus, 283 repositories, 0 edges added,
  removed or re-scored (`hcl-terraform-aws` excluded: it times out on both
  sides). High-confidence dead findings fall by 19,317, every one to
  `medium`, and none rises. Java −16,770: 8,445 modifier-less interface
  members, 4,186 public methods behind an annotation argument or a
  comment, 4,076 protected members (3,585 of them `@Override`), 63
  protected members behind an annotation argument. C# −2,547: 1,256
  interface members, 141 public and 15 protected members behind an
  attribute argument, 1,135 protected members (1,063 `override`).
  `app-spring-mall` falls from 1,087 to 9 (its MyBatis mapper interfaces)
  and `java-spring-batch` from 299 to 6. Most non-override protected
  members sampled are invoked by a framework or through dispatch (ASP.NET
  page handlers, Guice `@Provides`, visitor methods, a Kotlin subclass); a
  genuinely unused protected helper now reports at `medium`. TypeScript,
  JavaScript and Kotlin: no finding changes band.
- Parameter typing, C++ templates, Kotlin receivers and extensions,
  `super`, Python wrapped receivers (FINDINGS A2, A3, A7): **the default
  graph changes.** Full corpus, 283 repositories (`hcl-terraform-aws`
  excluded: it times out on both sides; `zig-zig` from a re-run, its
  dead-code pass times out on both sides): 43,449 edges removed (2,632
  `high`), 24,121 added (2,411 `high`), 3,477 re-scored, in 123
  repositories. Kotlin −37,725 / +12,895 (2,867 re-scored): calls on
  chained, multi-line, constructed, literal, parenthesised and `!!`
  receivers that had been bound as bare names; out-of-scope extensions and
  self- or sibling-bound `super` calls removed; constructed receivers
  (+2,249), in-scope extensions (+3,622), inherited members and `super`
  bases added, and the remaining chained receivers duck-typed like
  Java's and Python's. Java −4,242 / +5,048: `super` resolved through the
  base chain (+2,871 / −997), generic parameters typed as the generic
  (resolution through the import instead of every same-named nested
  class, −1,783; same-file `medium` → `high` intra-file), comma generics
  (+889 / −376), parameters behind an annotation argument (+1,167 / −819).
  Python +2,827 / −906 and JavaScript/TypeScript +495 / −83: `super`
  calls. C++ +1,806 / −405: templates read as themselves on `.`, pointee on
  `->`. C# +754, PHP +125: parameters behind an attribute. Go +62: a
  `<-chan` parameter no longer mis-slices the signature. Rust +48 / −20:
  `pub(crate)` parameters typed. Every class was sampled; the removed
  edges are calls bound by a coincidence of name.
- Dead code: high-confidence findings rise by 66 (Java +68, Rust +3,
  Python −5). The Java ones are lifecycle callbacks (26 `@PreDestroy` /
  `@PostConstruct`), overrides of an external contract (17), method
  reference targets (3) and members reached only through untyped
  receivers, plus one serialization hook: each was `medium` only because
  an unrelated `super.x()` or same-named site sat unresolved, which the
  report counts as a possible caller. The Rust three are Rocket
  `Fairing::info` impls whose only
  caller was a wrong duck-typed edge. Framework roots for these are FINDINGS
  B2/B3/B4. The 5 Python findings leaving `high` are now reached through
  `super()`.
- Not covered: a receiver typed as an external type loses calls to a
  workspace extension of that type in C# (extension methods) and Solidity
  (`using L for T`), as Kotlin's did before; a nullable `Foo?` parameter
  (Dart, C#) is typed `Foo?` and resolves nothing (FINDINGS C3); a C++
  parameter of a namespace-qualified type (`const ns::Foo& x`) is still
  read by the `name: Type` parser and never typed; a C++ smart pointer
  with no `->` in sight (`out.get()->m()`) is not followed.
- Module-scoped calls, bare-call scope and same-name copies (FINDINGS A4,
  A5, A6, R1–R20): **the default graph changes.** Full corpus, 283
  repositories, none excluded: 299,896 edges removed (46,595 `high`),
  452,851 added (311,469 `high`) and 275,014 re-scored (267,026 `medium` →
  `high`: exact module and same-package bindings), in 208 repositories.
  Removed / added by language: Go −86,001 / +23,909 (−60,143 / +12,369 of
  it in `hcl-terraform-aws`: receivers typed through a dependency's
  package — `d *schema.ResourceData` — filed as external rather than
  duck-matched; bare calls into another package; decoys beside
  package-made values), Java −42,517 / +99,416 (bare calls bound to
  methods of unrelated classes dropped; static imports exact; inherited
  and own-class overload sets, and overrides below the calling class,
  bound), Python −26,391 / +13,604 (builtins such as `list()`/`filter()`
  bound to methods; module-head decoys; relative imports now exact),
  TypeScript −21,791 / +12,938 and JavaScript −14,922 / +5,433 (calls
  through an import bound in the imported module; `app-calcom-nextjs`'s
  generated `prisma.<model>.<op>()` and npm `next/*` imports no longer
  bound to workspace functions; minified bundles), Perl −5,335 / +684
  (`Pkg->m()` exact; other packages' subs), PHP −1,429, C# −554 / +5,317,
  Rust −359. Erlang −49,720 / +255,615: `m:f(…)` now binds module `m`
  (OTP's own `lists`/`io` are in the tree). Outside the module-scoped
  languages the change is the scoped lookup and its same-name copies:
  Zig −32,320 / +30,823 (`const assert = std.debug.assert` external where
  `std` is not in the tree), Scala −6,737 / +877, OCaml −5,541 / +3,929,
  Dart −3,882 / +69, F# −1,921 / +187, and under 150 each in Lua, Kotlin,
  Haskell, Elixir, Clojure, Groovy and Lean — a call now reaches the copy
  in the module it imports or its own directory (the compiler's `cerl`,
  not a test-data copy; `std.debug.assert`), and other modules'
  same-named functions it reached by name (Haskell `lift` in five
  modules, Lua `assert`, a Play `status`) are dropped. Every class was
  sampled.
- Dead code: 209 findings rise to `high` and 50 leave it (Java +189 /
  −46, C# +18, Python +1 / −4, Rust +1). Of the Java/C# rises, 106 lost
  only the doubt of an unrelated same-named call site that now binds to
  its own class or base (a `configure`, `setup` or `write` elsewhere;
  framework-invoked `configure`/`readObject`/`@PostConstruct` hooks among
  them), 74 are overrides reached only by dispatch from a call bound to
  the base declaration (C# `JProperty.RemoveItemAt`, Java
  `DoubleDimensionMergerV9.setupEncodedValueWriter`) that an unrelated
  site or a wrong sibling edge had held, 21 lost wrong incoming edges
  (`getDelegate()` bound across unrelated classes), and 6 are now called
  only from dead code. Each was `medium`
  only because a wrong edge or an unrelated same-named site pointed at it
  (FINDINGS B2, B4, C2).
- Not covered: a Go package variable aliasing a function (`var New =
  sdk.New`), a JS module that builds its exports at run time and an npm
  package the tree also has a directory named like (`next/headers`) are
  refused as `not-in-imported-module` rather than external; Zig
  `@import("x.zig")` aliases are not followed to their file; a JS/TS path
  alias is matched by path suffix, not read from `tsconfig` `paths`
  targets; Go block scope is tracked only for values made through a
  package.

- WS14 (scope model beyond Py/JS/TS/Go). Full corpus, 283 repositories,
  none excluded (ab/WS14-v9), counting re-scored edges: `high` +208.1k
  net, `medium` −358.4k net; dead-code findings unchanged. Per language
  (`high` net / `medium` net): Dart +113.2k / −263.4k (bare calls into
  `package:test` and `dart:` libraries are external; `expect` /
  `testWidgets` bind to the in-tree `flutter_test` at `high`), PHP
  +68.5k / −37.2k (WordPress global functions, namespaced static calls;
  stub copies refused unless the include chain reaches one), Zig +18.9k /
  −16.1k, OCaml +5.5k / −3.3k, JavaScript +2.1k / −1.7k, TypeScript
  +0.1k / −16.9k (calls through destructured props and hook results),
  Python −1.0k / −4.7k (mostly pytest fixture parameters, whose value the
  test calls), Perl −0.3k / −8.2k (fat-comma keys), Elixir −2.4k medium,
  Kotlin +0.2k / −1.2k, F# +0.4k / −1.0k, Scala −0.0k / −1.1k (trait
  members now `Trait::m`; 19.0k calls to unrelated classes' members
  refused, 16.1k rebound), Lua +0.2k / −0.7k, Rust +0.4k / −0.3k,
  Swift −0.1k medium. Share of edges at `high`: Dart 6 → 35%, PHP
  17 → 87%, Elixir 65 → 90%, OCaml 65 → 77%, Zig 45 → 53%, Perl
  17 → 25%, TypeScript 64 → 68%. Latency (perf/WS14.md, 10 repositories,
  median of 5): no repository slower by 1 s or more; `scala-spark`
  −1.1 s; total +0.3%.
- Not covered (WS14): OCaml `open`/local-open/module aliases, F# `open`,
  Ruby `require`, Lua module tables (`function M.f` is not extracted,
  WS8), Swift module/target visibility; Scala self-types; a JS/TS prop
  holding a bound action creator of the same name when the file does not
  import it.

- WS5 (interface/abstract dispatch). Full corpus, 283 repositories,
  `hcl-terraform-aws` excluded (timeout on both sides; ab/WS5-v3). Edges
  (`high` net / `medium` net): Java +2 / +4.9k (C# 6.3k / Java 6.1k
  `medium` added are inherited members reached through now-recorded
  bodiless classes and `base.M()`; 664 Java, 234 C#, 32 Python, 12 Kotlin
  `super` calls into outside bases refused; 1.6k C# and 185 Java edges to
  same-named class copies far from the caller dropped), C# 0 / +4.5k,
  TypeScript +0.4k / +9.4k (calls typed to an interface or abstract member
  bind to its declaration; 5.2k edges re-keyed by the abstract-class owner),
  Scala +0.5k / +0.3k, Dart +19 / +0.7k (1.8k re-keyed by mixin owner),
  Swift +11 / −13, Python 0 / −72, Rust 0 / −16 (wrong copies). The default
  graph gains no `Via::Dynamic` edge; the fan-out runs only under
  `--dynamic-dispatch`, which `--dead-code` turns on. Dead code at `high`:
  Java −757, Rust −630, Python −394, C# −245 (outside-contract overrides
  1.0k, now live through dispatch 274, doubt from untyped sites 620,
  reachable only from dead code 91, base-member surfaces 46); one finding
  newly `high` (its only edge was a wrong `super` binding; its real caller
  is a method reference). Remaining carry-over `high`s are framework or
  reflective entry points (lifecycle and Gradle annotations, Gradle
  service `configure`, serialization hooks, method references → later
  workstreams). Latency (perf/WS5.md, median of 5): default graph, 10
  repositories, none slower by 1 s or more, total +0.7%; `--dead-code`,
  4 repositories, largest +0.5 s (`groovy-gradle`, +3.5%).

- WS14b. Full corpus, 283 repositories, `hcl-terraform-aws` excluded
  (timeout on both sides; ab/WS14b-v9). Dead-code findings unchanged.
  Edges (`high` net / `medium` net): Zig +42.7k / −10.8k (typed receivers
  bind their container's member: 34.5k added at `high`, 8.3k re-scored
  medium → high; 5.9k `medium` name-matched edges removed — rebound to
  the typed container's member, calls on std types now external, type
  function results), Scala +8.6k / −10.4k (bases through imports: 3.9k
  rebound from same-named classes of other packages, e.g. spark's
  `connect.test.QueryTest`; `this(…)` delegation no longer bound to
  `def this`), Perl +3.3k / −0.2k (invocant calls), OCaml −385 / +83
  (self-edges of non-recursive `let`s), PHP 0 / +88 (unconditional
  definition over fallbacks), Rust +21 / +15, Kotlin −12 / −48 (scope
  lambdas: wrong self-class bindings dropped, captured parameters typed).
  WS14 regressions recovered (same target as before WS14): Rust 37 of 49
  `shadowed-by-local` sites (all 22 `if let` ones; the rest are values
  passed as parameters or `quote!` interpolations), Zig 105 of 437
  `candidates-in-other-files` and 86 of 266 `shadowed-by-local` (the
  rest: field-chain receivers, comptime function parameters), Perl 28 of
  291 (every invocant call with a sub; the rest are `key => $obj->m` on
  untyped objects, or Moose accessors), PHP 5 of 39. Latency (perf/WS14b.md, 9
  repositories, median of 5):
  none slower by 1 s or more; largest `scala-spark` +0.67 s (+7.4%),
  `zig-zig` +0.48 s, `app-wordpress` +0.29 s; total +2.1%.
- **WS8 changes the default graph** (full corpus, 283 repositories,
  `hcl-terraform-aws` timed out on both sides; edges by confidence,
  removed / added): C high 950 / 42,385,
  medium 587 / 13,409 (pointer-returning functions; removals are macro
  attribute names now named correctly, swallowed headers, and 139 dual
  C/C++ MIDL headers now read as C++); C++ high 3,569 / 3,650, medium
  14,036 / 7,527, 1,898 medium → high (mostly qualified-name renames of
  macro-misparsed definitions, decoys dropped by `ns::Foo` parameter
  typing, edges to constructed locals removed); C# high 265 / 11,686,
  medium 3,772 / 11,454 (Newtonsoft.Json re-parsed: renames out of the
  misparsed namespaces; 507 sites now over the fan-out cap); Dart high
  4 / 27,101, medium 10,773 / 44,312 (method bodies now sources; the
  removed are duck-typed sites now over the cap, recorded); Scala
  high 0 / 17,353, medium 0 / 80,465; PHP high 0 / 52,599, medium
  1,591 / 22,353 (`$this` resolved; removed: `static::`/`self::` fan-out
  to unrelated classes); Lua high 115 / 7,515, medium 1,638 / 16,379
  (rebound through the split names); Ruby high 0 / 4,482, medium 486 /
  3,150 (removed: bare calls bound to unrelated classes); Julia high 997,
  medium 1,059 removed (signature self-edges); Kotlin 9 rebound;
  Elixir 2 removed (phantom capture calls). Unresolved records: Scala
  +196.6k and Lua +73.7k (receiver-qualified calls formerly screened as
  whole dotted names), PHP −99.3k, C −25.3k, C# −25.4k, Dart −20.3k.
  Dead code: 56 C# findings leave `high` (misparsed `#if` callables
  gone), 22 drop to medium (partial methods), none newly `high`.
  Two audit-merge loops that scanned a file's buckets (or every file
  record) once per cross-file record are now indexed; without that the
  extra Scala/Lua sites cost scala-spark 3.9 s. Latency (perf/WS8.md, 10
  repositories, median of 5): none slower by 1 s or more; largest
  `scala-spark` +0.79 s (+8.4%, 42% more edges), `lua-neovim` +0.12 s;
  total −0.9%.
- **WS9 changes the default graph** (full corpus, 283 repositories;
  `zig-zig`, which timed out under corpus load, diffed on its own: C++
  high 21 / 653, medium 138 / 1,098, the removed all edges from
  `std::forward<_Args>(…)` to a misparsed definition named
  `std::forward<_Args>`; elsewhere, edges by confidence, removed /
  added): Go high 0 / 5,555
  (`hcl-terraform-aws` 5,427: `errs.IsA[*T](err)` and friends); C# high
  0 / 22, medium 0 / 9,185 (Newtonsoft.Json 8,495:
  `JsonConvert.DeserializeObject<T>`); C++ high 0 / 2,337, medium 9 /
  5,903 (the 9 removed are `cmake` calls of an explicit specialization,
  `New<AT>()`, now bound to the primary template `New`); Dart high
  0 / 1,133, medium 0 / 2,433; Scala high 0 / 576, medium 0 / 1,625;
  F# high 0 / 30, medium 0 / 12; Julia high 1 / 27, medium 0 / 71, 3
  high → medium (37 new short-form methods; the removed edge now has
  the nested method as its source). Newly recorded sites that bind
  nothing are unresolved with their reason (Scala +11.6k, Dart +11.0k,
  C++ +3.8k, C# +1.1k), not dropped. Dead code: 4 C# findings leave
  `high` (members called only with type arguments or through `?.`),
  none newly `high`. `hcl-terraform-aws` now finishes the dead-code run
  under the 60 s cap (18.2 s → 13.1 s for the default graph at
  `--jobs 4`). Latency (perf/WS9.md, 14 repositories, median of 5):
  `hcl-terraform-aws` 17.6 s → 6.6 s (−62.7%); none slower by 1 s or
  more, largest `zig-zig` +0.63 s (+1.8%); total −13.2%.
- **WS10 changes the default graph** (full corpus, 283 repositories,
  none excluded; edges by confidence, removed / added, and medium → high
  rescores): Java high 0 / 433 (+196 rescored), medium 4,516 / 10,015;
  PHP high 0 / 5,004 (+213), medium 172 / 849; Go high 0 / 1,345
  (+5,447: `meta.(*conns.AWSClient).X()` in `hcl-terraform-aws`),
  medium 201 / 390; C++ high 0 / 150 (+118), medium 185 / 1,937;
  TypeScript high 88 / 513 (+19), medium 723 / 791; Dart medium 0 /
  1,865, high 0 / 60; Scala medium 1,483 / 700, high 0 / 35; Ruby medium
  0 / 564; Python medium 69 / 378, high 0 / 55; JavaScript medium 298 /
  164, high 0 / 51 (+35); Solidity medium 0 / 163; Kotlin medium 1 /
  124, high 0 / 8; C# medium 0 / 60, high 0 / 13; Groovy medium 109 /
  10, high 0 / 10; Swift medium 11 / 25, high 0 / 6. Removed edges are
  name-matched fan-out from construction and cast receivers onto
  same-named decoys and onto workspace methods of external types (`new
  TextEncoder().encode`, `new Date().getTime`, `pd.Series(…)`,
  `JWTKeyCollection().add`), and Scala `math.*`/`sys.*`/`java.*` calls
  bound to workspace functions (1,423 now external); the 88 TypeScript
  `high` removals are the same calls re-attributed to the method that
  encloses them once the file parses. Dead code: no high-confidence
  finding changes. Not covered: an awaited receiver is typed as the
  value awaited (`(await t).M()` with `t: Task<Foo>` reads `Task`); a
  cast to a C++ template parameter (`static_cast<This*>(this)`) is a
  typed miss; Groovy classes the grammar loses the scope of keep losing
  calls through a constructed receiver; a nullable parameter
  (`Foo? n`, `?Foo $n`) is still untyped (WS11); Go same-package
  parameters are not typed, so `f.M()` stays duck-typed while
  `Foo{}.M()` binds. Latency (perf/WS10.md, 17 repositories, median of
  5): none slower by 1 s or more, largest `groovy-gradle` +0.27 s
  (+3.4%); total +1.5%.
- Optional / nullable / union annotation typing and typed closure
  parameters (WS11): **the default graph changes**. Full corpus, 283
  repositories, none excluded. Edges dropped by language: Dart 86.5k,
  C# 2.8k, Java 2.0k, Kotlin 2.0k, Scala 1.1k, Python 0.5k, TypeScript
  0.1k. Almost all are medium duck-typed decoys of a receiver that now
  has its declared type: 85.6k kept their correct target, and 3.3k were
  rebound to the declared type's member. Some sites are newly
  unresolved (`candidates-in-other-files`): 741 Scala typed patterns
  whose member is a Java interface method or an implicit-class
  extension, which the Scala resolver does not reach; 527 Dart; 379
  Java and 203 Kotlin receivers now typed as a type outside the tree;
  55 C#; 34 TypeScript; 32 Python.
  - HIGH drops (Java 28, Kotlin 29, C# 11, Scala 3) were checked
    individually. Each was either a wrong old edge (self-binding of
    `equals`/`grammar` through an untyped parameter, a static helper of
    another class), now refused or rebound to the declared type, or a
    receiver whose declared type is an interface outside the tree.
  - A multi-type union stays duck-typed at medium. In a 10-edge sample
    per language, Python and TypeScript union fan-outs were the
    incorrect ones (Python 7/10, TypeScript 6/10; HIGH 5/5 each).
- Function-defining macros, quoted definitions and Elixir module calls
  (WS12, WS12b): **the default graph changes**. Full corpus, 283
  repositories, none excluded. Edges removed: C 616 high / 12 medium,
  Elixir 141 high / 12 medium, Clojure 7, C++ 670 medium (all now
  `fanout-cap-exceeded`, see below). Of the
  C removals, 551 came from a misparsed function whose span had swallowed
  the macro invocations after it (a callable named `__BITS`, `if`,
  `lev_child_start`, … "calling" the macro) and now credit the macro from
  outside any function; 65 were rebound from a header prototype to the
  generated body it now merges into. The Elixir removals are the
  `unquote` self-edges; the 12 medium ones were rebound to a
  now-recorded `defmacrop` of the calling module.
  - Precision (logs/WS12-precision-sample.txt): C 11/11 correct; Elixir
    HIGH 6/6, medium 5/5; Clojure 7/7 (re-attributions). C++ HIGH 4/6
    exact, the other 2 bind a call inside a generated method to the
    nested macro's unexpanded template (`TypeOf<STRUCT>::serializeFields`);
    C++ medium 6/8 correct (2 duck-typed decoys: a vendored gtest copy,
    a CRTP `getDerived()` receiver).
  - **Template specializations respect `--fanout-cap`.** A typed owner
    lookup indexes every specialization under the bare template name, so
    `TypeOf<T>::type()` reached all 114 `TypeOf<X>::type` specializations
    (cmcppdap's, generated by the macros above, and hand-written ones
    alike) with no cap. Such a lookup now binds to the specialization the
    receiver writes exactly (`TypeOf<A>::type()`), and otherwise is
    refused as `fanout-cap-exceeded` when more specializations than the
    cap are involved (the cap counts specializations, not the overloads
    each declares). A receiver written without template arguments, and a
    Rust type's trait impls (`<T as From<U>>`), are unaffected. The 670
    removed C++ medium edges are these over-cap sets: `TypeOf<…>` 169,
    `AsStringView<…>` 162, `DenseMapInfo<…>` 110, `DartConverter<…>` 106,
    gtest printers 96, cmake artifact getters 21. No other language
    changes.
  - Not covered: nested expansion (an X-macro list passing a
    function-defining macro credits it but generates nothing); a macro
    defined in a header the invoking file reaches only through `-I` paths
    no `#include "…"` names; Elixir module-body macro calls through a
    module (`MyApp.Gen.defgetter(:x)`) bind but, like every call outside
    a function, draw no edge; functions injected by an in-tree `use`
    macro are refused.
  - Latency (perf/WS12.md, 17 C, C++, Elixir, Clojure, Rust and Java
    repositories, median of 5): none slower by 1 s or more, largest
    `dart-flutter` +0.32 s (+3.5%); total +1.6%.
- Rust leftovers (WS13): **the default graph changes, Rust only.** Full
  corpus, 283 repositories, none excluded, 24 changed. HIGH edges
  −29 / +249, 127 medium→high, 43 high→medium; medium edges −3,557 /
  +1,285. Of the removed edges 721 were rebound to another target, ~1,085
  are trait-overload sets now refused (`fanout-cap-exceeded` 548,
  `ambiguous-in-file` 537 where the impls share the caller's file), 268
  became `candidates-in-other-files`, and the rest are external or std
  calls no longer bound by name, or sets narrowed to the target that
  remains. High-confidence dead-code findings −50 (54 leave the top band,
  4 enter it: two private methods nothing calls, one `#[cfg]` duplicate,
  and one method whose loop-variable receiver borrows another function's
  local type); on one 23-crate workspace 16 → 14.
  - Precision (target/remediation/logs/WS13-precision.txt): foreign-type
    rebinds 4/4 and refusals 5/5 correct, overload picks by literal 4/4,
    std-type screens 3/3; added HIGH 7/8 verified. Of the 29 removed HIGH
    edges about half had the right target, lost where an untyped
    re-binding (`let (resp, _) = resp.into_parts();`) now hides the
    earlier typed `let`.
  - Latency (perf/WS13.md, 14 Rust, Dart, Erlang, Groovy and CMake
    repositories, median of 5): none slower by 1 s or more, largest
    `erlang-otp` +0.17 s (+2.2%); total +1.0%.

## [0.10.2] - 2026-10-11

Rust resolution and dead-code accuracy, from FIX_PLAN.md v2. A bugfix
release: every graph change is a wrong edge removed or a missed edge
found, and the one new flag is opt-in. **The default graph changes**
(see *Compatibility*): measured against 0.10.1 over the
282-repository corpus, 25 repositories change: net edges Rust +5,027,
Java +9 (a builder-method tie-break), Python −29 (below). Every dropped
edge was checked: 923 path calls into external crates or std that had been
name-matched to workspace functions (`serde_json` 437, `mpsc` 129,
cortex-m `debug` 58, `serde_yaml`, `hex`, `toml`, `diesel::update`, …),
326 methods on receivers of external type, 508 fan-out sets narrowed, 319
retargeted, 291 confidence or resolver changes, 25 other. High-confidence
dead-code findings fall 26,873 → 26,018 (on one 23-crate workspace,
105 → 17). Pre-existing, not chased: a `Type::f()` call is still read as
a constructor; `zig-zig` exceeds 60 s under `--dead-code` on both binaries.

### Compatibility

- Shipped as a patch although edges are dropped: each dropped edge was a
  wrong one (a call into an external crate or std, or a duck-typed match
  the receiver's type rules out). Consumers diffing graphs across the
  upgrade will see Rust fan-out shrink and ~5,000 new Rust edges.
- Macro-generated callables (`macro:<name>`) are new nodes in Rust trees
  that define and invoke a `macro_rules!` in the same file.

### Performance

`perf-compare.sh 17b89f3 3`, all 282 repositories, none excluded: total
286.6 s → 267.5 s (−6.7%), median per repository 0.0% (133 faster, 125
slower), median of the 48 repositories over 1 s −1.2%. The total is
three large non-Rust trees, re-sampled at 7 interleaved runs with
byte-identical graphs: `hcl-terraform-aws` (mostly Go) 28.7 s → 18.2 s,
`scala-spark` 13.9 s → 9.5 s. The import-path scan visits each matching
file once per referencing file and used to clone two strings per
definition per visit; definitions are now resolved to ids once. Rust
repositories are flat within noise (ripgrep +1.4%, axum +0.9%, lemmy
+0.4%, vaultwarden +1.4%; ntex +6.9% on a 262 ms run at load 7.5).

### Fixed

> **Since path calls existed, `extcrate::f()` was name-matched to any
> workspace `f`.** A one-segment path receiver (`serde_json`) carries no
> `::`, so step 4c never saw it and step 5 duck-typed it: on one
> 23-crate workspace 450 `serde_json::from_str` sites bound to local
> `FromStr` impls. `RefRecord::path_receiver` now marks path receivers.

- Turbofish callees (`f::<T>()`, `x.parse::<u8>()`) produced no reference
  and no unresolved record.
- `trait_impl_target` was empty for any impl whose type has generics or a
  lifetime (`<Printer<'_> as Output>`), and kept a truncated generic for
  generic traits, so `--dynamic-dispatch` never reached those impls.
- An in-workspace module head (`mod jobs;` + `jobs::Runner::spawn()`) and
  a `use crate::…` import were judged external.
- `Arc::new(a::B::new())` typed the local as `Arc`; `x: Arc<Mutex<dyn T>>`
  typed the parameter as `Arc`. Both now see through wrapper chains, and
  a lock guard (`x.lock()`) types as its contents.
- Rust locals were file-wide last-write-wins; now the `let` in scope.
- `async fn` methods got no `self.<field>` facts; a field typed by the
  struct's generic parameter now uses its trait bound; `Option<T>`/`Vec<T>`
  fields are no longer typed as `T`.
- Receivers are normalized: struct literals read as their type path,
  parens/`?`/`.await` peeled, multi-line receivers joined; inside macros
  `self.x.m()` keeps `self`.
- Cargo dependency renames (`foo = { package = "bar" }`, including
  `workspace = true` renames) and `[lib] name` aliases, glob imports,
  `mod x;` declarations and re-export-only crates now count as
  in-workspace path heads.
- External extractor wrappers (`State<AppState>`) and workspace `Deref`
  wrappers resolve methods on their inner type; type aliases and `Deref`
  targets feed the owner index.
- wasm-bindgen extern methods are indexed under their `this: &T` owner
  (a one-parameter signature lost its closing `)`).
- `self.m()` resolves to the exact owner across files of one crate, and a
  `self`/`this` call with candidates in several impl blocks prefers the
  caller's own block (also fixes a Java builder ambiguity).
- A builder chain is typed as the builder only when it does not end in
  `build`/`finish`/`create`/… ; a bare `X::builder()` is left untyped.
- A bare call no longer binds to an `async` method (Rust, Python). This is
  the Python change: py-falcon and py-litestar lose wrong fan-out edges
  (`list(data)` → a repository's `list` method) and gain high single ones.

### Changed

- Over-cap Rust fan-out made entirely of one workspace trait's impls binds
  to the trait declaration instead of being dropped.
- Item-position invocations of same-file `macro_rules!` are expanded
  (no-repetition arms). Generated callables are real call targets tagged
  `macro:<name>`, kept out of by-name fan-out and implicit-trait rooting,
  and reported dead at no more than medium (`macro-generated`).
- Dead code: new `implemented-trait-surface` evidence and
  `value-position`/`attribute-string` screens (serde attribute strings,
  functions passed as values in macros) cap a finding at medium; an
  `IntoResponse`/`Responder` impl is a root when its type is returned
  unwrapped; `def_variant` is filled in; `--why-live` explains a doubted
  trait declaration.

### Added

- `--guess-unique-names` (opt-in; Python `guess_unique_names`, Node
  `guessUniqueNames`): bind an unplaced Rust call to a globally unique
  name at `low`, resolver `cross-file:name-unique`.

## [0.10.1] - 2026-09-25

Adds `--locations` (#10, Serge Bakharev) and fixes how mermaid and
GraphML labels are escaped. The graph is unchanged: callables, edges and
dead-code findings are identical to 0.10.0 on all 282 corpus
repositories. Default *rendered* mermaid changes in 30 of them, and
GraphML in one, only where a label was misread (see *Fixed*).

### Added

- **`--locations`.** Mermaid, DOT and GraphML can name the file and line
  of each call. Off by default.
  Mermaid and DOT still draw one arrow per caller/callee pair and list
  every site on it (`-->|"src/lib.rs:42,88"|`, `label="src/lib.rs:42,88"`);
  a `via` tag stays in front (`std src/lib.rs:42`). GraphML keeps one
  edge per call site and adds `site_file` and `site_line`. A rolled-up
  arrow has no single site and keeps its `Nx` count, as does an edge
  from a synthetic caller — the marker's line is on the edge, but the
  edge does not carry the marker's file. `-t json` already records
  `site_line` on every edge and the caller's file on the source
  callable; asking for `--locations` there warns rather than changing
  the document. The flag lives on `RunOptions` because `--rollup`
  measures its budget against the rendered document, and a location
  label is larger than a bare arrow.

  Reachable from all four front ends: `--locations` on the CLI,
  `locations=` on `cgg.analyze()` and the render methods in Python,
  `locations` on the Node options and `toMermaid` / `toDot` /
  `toGraphml`, and `"locations"` in the C ABI's options JSON — no new
  exported symbol. Contributed by Serge Bakharev (#10).

### Fixed

> **Since mermaid output existed, a label that began with a backtick
> could make mermaid reject the entire default diagram.** A quoted
> mermaid label that starts with `` ` `` is parsed as a markdown string.
> F#'s ``` ``double-tick`` ``` names and an unclosed tick are not well
> formed, and mermaid then refuses the whole document, not the one node.
> Over the 282-repository corpus this made the default diagram
> unrenderable for **4 repositories** — `fsharp-paket` (1,248 such
> labels), `app-fsautocomplete-expecto`, `app-orleans-samples` and
> `app-riffraff-play` — and dropped the ticks from 15 labels in
> `r-ggplot2`, `app-tweetconf-shiny` and `rust-bat`. Checked by rendering
> each repository's affected labels with mermaid-cli before and after.
> The graph was never affected; `-t json`, DOT and GraphML were not
> either.

- **Mermaid labels are escaped for everything mermaid-cli misreads.**
  Node and edge labels now share one escape, `mermaid_escape`:
  - A leading backtick is written `#96;` (above).
  - `&` before a letter, digit or `#` becomes `&amp;`. The renderer
    decodes HTML entities even without their `;`, so a file named
    `amp&amp.py` displayed as `amp&.py`.
  - `#name;` and `#123;` are mermaid entity codes. Their `#` is written
    `#35;`, so `x#quot;y` no longer displays as `x"y`. A `#` that starts
    no code (`C#`, `Foo#bar`) keeps its bytes.
  - A line break becomes a space. Mermaid does render a break inside
    quotes, so this changes no picture. It keeps each statement on one
    line for tools and agents that read mermaid a line at a time.
    900 labels carry one, most of them a C/C++ prototype's wrapped
    parameter list or a Clojure form (`zig-zig` 625, `app-metabase-compojure`
    91, `dart-flutter` 88).

  The same escape fixes the new `--locations` edge labels. As merged,
  their escape wrote `\|` and `\\`, which mermaid displays literally
  (`p\|q.py`, `back\\slash.py`), and left `<`/`>` raw, so `lt<gt>.py`
  displayed as `lt.py`.

  Default mermaid changes in 30 of 282 corpus repositories: 1,300
  leading-tick labels, 900 multi-line labels and 136 `&` labels. To
  confirm that nothing else changed, each repository's new diagram was
  re-derived from its 0.10.0 diagram by applying only these rules, and
  all 30 match byte for byte. Regression tests:
  `labels_escape_what_mermaid_would_misread` and
  `a_hostile_path_stays_on_one_edge_statement` in
  `crates/cgg-format/src/mermaid.rs`.
- **GraphML no longer emits characters XML cannot carry.** XML 1.0 has no
  form for most C0 control characters, so a parser rejects the whole
  document on the first one. A callable or file whose name contains one,
  which is legal on Linux, made every GraphML run over that tree
  unreadable. `erlang-otp` hits this: a test fixture function named
  `c0_bad_name_` + U+0001 + `_test`. These characters are now written as
  U+FFFD, and every corpus GraphML document parses. Regression test:
  `control_characters_do_not_break_the_document` in
  `crates/cgg-format/src/graphml.rs`.

### Performance

Like for like: the pipeline code is the same in both releases. The
default path changes only in the formatters: one flag test per edge,
and an escape that walks each label once instead of three `replace`
passes. `scripts/release.sh` against v0.10.0, `--jobs 1`, all 282
corpus repositories. Load average was 8–11 on a 64-core host shared
with a running vLLM server:

| | v0.10.0 | 0.10.1 |
| --- | --- | --- |
| total wall | 293.6 s | 298.0 s (+1.5%) |
| median per-repo delta | — | +0.3% |
| repos faster | — | 137 of 282 |
| graphs identical | — | 282 of 282 |

23 repositories over 150 ms moved by more than 5%, the largest being
`app-ocamlorg-dream` at +30.2%. Each was re-sampled as three
interleaved pairs, keeping the minimum of each binary. 19 came back
within ±4.3%. `app-ocamlorg-dream` measured +4.6% and
`app-metabase-compojure` +2.5%, on runs of 15 s and 34 s. `java-gson`
(+8.1%), `app-jupyterhub-tornado` (+7.5%) and `app-terminus-hanami`
(−8.9%) each moved by exactly one 50 ms step of the re-sampler's timer.
With no code change in the pipeline, we read all of it as noise on a
loaded machine, not a regression. `cargo test --workspace` takes 8.7 s
on a warm build and `docs-check.py` 0.2 s.

## [0.10.0] - 2026-09-24

A minor release because the default graph moves substantially. Over the
282-directory corpus it has **5,892,455 edges against 0.9.1's
5,515,949 (+6.8%) and 2,698,195 callables against 2,688,642**. Every
edge and node 0.9.1 had that 0.10.0 lacks was traced to a mechanism
before release: guesses replaced by a certain binding (Java, Scala,
Kotlin, Haskell and Erlang calls that had been bound to a same-named
function in an unrelated class or module), calls now attributed to an
inline route handler's node, what the C grammar invented from C++
headers, C prototypes merged into their bodies, and untyped C++
receivers whose candidate sets grew past `--fanout-cap`. The largest
single gain is Erlang, where every call into a multi-clause function was
dropped as ambiguous until now (+143k edges on OTP).

### Performance

Paired A/B against 0.9.1 over the whole corpus (`scripts/perf-compare.sh
v0.9.1 3`, median of 3 per repository): **304,836 ms → 307,744 ms,
+1.0%**, inside the 1–1.5% noise floor the script reports for its own
total. Like-for-like apart from the new default work: overload and
clause binding, following in-tree `#include <…>` headers, and PR 9's
cross-language lookups all add edges, and the run is 6.8% more edges
for 1.0% more time.

The machine was loaded throughout (load average 5.6–8.9 from unrelated
resident services), which inflates per-repository deltas; the five
repositories that moved by more than ~150 ms in that run were
re-measured paired at `--jobs 1`, median of 5:

| repository | 0.9.1 | 0.10.0 | Δ |
| --- | ---: | ---: | ---: |
| app-bamboobsc-struts | 10,752 ms | 10,869 ms | +1.1% |
| app-rn-expensify | 19,008 ms | 19,669 ms | +3.5% |
| app-metabase-compojure | 33,066 ms | 34,639 ms | +4.8% |
| php-laravel | 8,556 ms | 8,501 ms | −0.6% |
| smithy-protocol-tests | 6,202 ms | 6,276 ms | +1.2% |

None exceeds 5%. The two largest are not new edges (Metabase gains 75,
Expensify 22); `--profile` puts Metabase's in `xfile::import-table`, and
splitting the repository puts all of it in the JavaScript frontend
(436 ms → 1,050 ms; the Clojure backend is flat). The JavaScript plugin
now records `export … from 'm'` and assigned `require('m')` as imports,
and each import's module becomes a path fragment that the import table
tests against **every file of the language** — a scan that is quadratic
in file count and predates this release, now run with more fragments.
Replacing it with an index is the follow-up. `cargo test --workspace`:
8.9 s warm.

### Added

- **Lean 4.** `.lean` files: `def`/`theorem`/`lemma`/`abbrev`/`instance`/
  `axiom`/`opaque`/`constant` declarations, structures and inductives
  (fields and constructors), function application and dot-projection
  calls, and the lemma references inside tactic blocks (`exact`,
  `apply`, `simp [..]`, `rw [..]`). Cross-file resolution follows
  `import`/`open`; `@[simp]`/`@[ext]` attributes and an `instance` marker
  are recorded so dead-code reporting can treat implicitly used
  declarations as live. Contributed by Nathan Howell. The grammar is
  vendored (`crates/cgg-lang/vendor/lean4`), because the published
  crate's `links = "tree-sitter"` cannot coexist with the workspace
  runtime; before landing, a dotted-name bug that named 128 Batteries
  declarations `….rfl`, anonymous instances named after their body, and
  references to a declaration's own binders were fixed, and the grammar
  ran clean under AddressSanitizer and UBSan on 4,766 real, mutated and
  pathological inputs. 46 languages.

- **Inline Go route handlers get a node.** `m.Get("/", func() string
  { … })` — how Martini writes every route — passes a `func_literal`
  that had no name and so no node, and the route had nothing to point
  at. Go now mints `handler_at_<line>` for a closure handed to a
  registrar verb, as JavaScript has since 0.5; `sort.Slice`, goroutines
  and `defer func(){}()` mint nothing. On `app-pixlserv-martini` the
  rule enumerates 3 routes where it enumerated 2.
- **Electron IPC and lifecycle entries.** `ipcMain.handle`/`handleOnce`/
  `on`/`once`, `ipcRenderer.on`/`once`,
  `contextBridge.exposeInMainWorld` and `app.on`/`once` are entry points,
  with the channel as the route (`electron::handle('fonts:list')`).
  Registrars may now name their receiver, and a receiver path matches
  on any suffix, so `window.electron.ipcRenderer.on(…)` binds while a
  jQuery `$(x).on('click', …)` in the same app does not. Electron was a
  detect-only rule; on `app-marktext-electron` it enumerates 179 entries.
- **`#include <header.h>` follows headers in the tree.** A build that
  passes `-I` writes its own headers in angle brackets —
  `<helper_cuda.h>` across the CUDA samples, `<spdlog/spdlog.h>` in
  spdlog's tests — and cgg followed only quoted includes, so every call
  into those headers was unresolved or guessed. A standard, POSIX,
  Windows or compiler header (`<stdio.h>`, `<sys/types.h>`, `<vector>`,
  `<immintrin.h>`) is still never followed, even into a vendored copy.
- **Overload sets bind.** Two same-file candidates with one qualified
  name — `matrix4::element(int, int)` and its `const` twin, Java's
  `add(int)`/`add(String)` — were reported `ambiguous-in-file` and the
  call dropped, although the callee was never in doubt. Worse, the
  dropped call then reached cross-file resolution, which guessed a
  same-named method on an *unrelated class*: on Druid,
  `FunctionTest.testArrayContains` was bound to
  `ExprEvalTest.assertExpr` rather than its own class's overloads, and
  9,567 such edges in that one repository are now rebound to the right
  class. Each overload gets a `medium` edge, bounded by `--fanout-cap`
  (0 still means never guess) — only in languages that overload (C++,
  Java, C#, Kotlin, Scala, Swift, Groovy, F#, Solidity, TypeScript,
  Julia); a name repeated in JavaScript, Python or C is a different
  function in another scope or `#ifdef` arm and stays ambiguous. In the
  clause languages — Erlang,
  Elixir, Haskell — same-name definitions are the clauses of one
  function, or of one function per arity (`f/1` and `f/2`), so the call
  binds once, at `high`, to the first clause of the function whose arity
  it passes; Erlang and Elixir calls now record their arity (a pipe
  `x |> f(a)` counts the piped value).

- **C ↔ C++ ↔ Obj-C cross-language resolution.** A qualified-name
  lookup that misses in the caller's language tries the other languages
  in `cgg_core::lang_family`, so a C++ file that `#include`s a C header
  resolves the call to the function defined in the `.c` file, and the
  reverse. The simple-name and owner-method indexes stay
  single-language, so duck-typed fan-out is unchanged. Only the C
  family is enabled. Contributed by Serge Bakharev (#9).

- **Content-detection for `.h` headers.** A `.h` file is parsed with the
  C++ grammar when its first 8 KiB, after comments and string literals
  are removed, contains `namespace`, `template<`, a class or struct
  base list, an access specifier, `using`, `enum class`, `constexpr`,
  `noexcept`, `::`, a C++ standard-library include (`<vector>`,
  `<string>`, `<experimental/filesystem>`), or an include path
  containing `++` (`<bits/stdc++.h>`). The sibling-file heuristic
  remains the fallback. `extern "C"` is not a trigger, and neither is a
  keyword inside a comment or a string. Contributed by Serge Bakharev
  (#9). Across the corpus this and the entry above add 61k edges and 5k
  nodes; what they remove is what the C grammar invented from C++
  headers — member initialisers read as calls (`f0_`, `value_`,
  `nSize_`), `static_assert`/`throw`/`move` as functions, and
  in-class declarations as free functions.

### Fixed

- **Seven frameworks were never detected on real applications**, each
  for a different extraction gap, and each now detected on the corpus
  app that uses it:
  - OCaml qualified names (`Dream.run`, `Alcotest.check`) record their
    top module as a use, so a library used without `open` is seen —
    dream, alcotest.
  - Erlang `-include_lib("eunit/include/eunit.hrl")` is recorded —
    eunit.
  - JavaScript `export … from 'm'` and `x = require('m')` assigned to an
    existing binding are imports — sails.
  - PowerShell `Import-Module (Join-Path … -ChildPath 'X.psm1')` records
    the child path — DSC resources, 82 entries on NetworkingDsc.
  - Padrino and Puma are detected from `config/apps.rb` and
    `config/puma.rb`, which is how an app declares them when the gem is
    only in the Gemfile.
  - An import pinned to a version (`https://deno.land/x/oak@v12.1.0/mod.ts`,
    `jsr:@oak/oak@^17`) matches its unpinned prefix; `@` in first position
    still opens an npm scope — oak.
  cmdliner's corpus evidence moved from opam, which vendors it under the
  module name `OpamCmdliner`, to ocaml-ci and ocaml.org, which use it
  directly.
- **Elixir one-line definitions were misnamed.** `def g, do: f(1)` was
  extracted as a function named `g, do: f`, so nothing could call it;
  guarded heads (`def h(a) when a > 0`) are named from the call, not the
  guard. Test `one_line_and_guarded_defs_are_named_and_calls_carry_arity`.
- **`#include "../x.h"` resolves.** The joined path
  `src/net/../common/util.h` matched no indexed file, and the `..` also
  defeated the suffix fallback, so every include that climbed a
  directory contributed nothing. Paths are normalised lexically; test
  `c_parent_relative_include_resolves_to_the_named_header`. The same
  lookup serves Dart, Solidity and Nix relative imports and Bash
  `source`, so `import '../src/common.dart'` resolves too — on Flutter,
  `getLocalEngineArguments` and `tick` in the tool and framework tests
  were unreachable from every test that imported them this way.

- **Prototype unification covers C and Objective-C bodies.**
  `unify_declarations` keys off `has_body`, which only the C++ plugin
  set. C function definitions and Objective-C method and function
  definitions now set it, so a `.h` prototype parsed as C is dropped
  when the `.c` or `.m` body is in the tree. Quoted Objective-C
  `#import`s are recorded as `include` and followed like `#include`.
  Contributed by Serge Bakharev (#9).

### Known limitations

- Lean dot-projections (`xs.foldl f`) and applications of names bound
  inside tactic proofs are resolved by name, so they are `medium`
  guesses; cgg does not elaborate Lean types. Macro and notation
  expansion and typeclass-method dispatch need the Lean kernel and are
  reported as unresolved.

### Compatibility

- Calls written inside an inline Go route handler (`router.GET("/x",
  func(c *gin.Context) { … })`) and inside an Electron IPC or lifecycle
  closure are now edges from the minted `handler_at_<line>` node, not
  from the function that registered the route — the registering function
  does not make those calls; the framework does, per request. This is
  how JavaScript and TypeScript route closures have been attributed
  since 0.5. The handler is reached from its `<framework-entry>` node
  (the route), not from the registering function.
- A `.h` header that is now parsed as C++ no longer takes part in C's
  global same-name guessing: a C function it calls binds through its
  `#include` chain or not at all. The guesses this removes were mostly
  wrong — CUDA's `rendercheck_gl.h` had `glBindBuffer` bound to the
  `.c` files of two unrelated Tegra samples.

### Changed

- **Releases are gated on outstanding work and on packaging.** 0.9.0
  shipped without a finished Lean 4 plugin that sat on a contributor's
  fork as a branch, because only open pull requests were checked, and
  that plugin could not have been published as it stood: its grammar was
  patched to a git fork through `[patch.crates-io]`, which is dropped
  from a packaged crate, so the release would have succeeded while
  `cargo install cgg` failed for every user. Now:
  - `scripts/unmerged-inventory.sh` lists open PRs, unmerged branches and
    every contributor fork's branches ahead of `main`; `scripts/release.sh`
    fails on any not acknowledged in `scripts/unmerged-acknowledged.txt`.
  - docs-check **check 14** rejects a `[patch]`/`[replace]` section or a
    git dependency in any Cargo manifest, on every commit.
  - `scripts/release.sh` packages every publishable crate and verifies it
    against crates.io, packs the root and five platform npm packages, and
    builds the Python sdist through `twine check`.
  - The release workflow verifies the crates in a job that PyPI and npm
    publishing wait on, and after publishing requires every npm platform
    package — not only the root — to be served before it reports success
    (on 0.9.1 the 105 MB `linux-x64-gnu` package lagged the root, and a
    fresh `npm install` skipped it).
  - `scripts/compare-release.py` honours `CGG_REPO_TIMEOUT` and
    `CGG_TOTAL_BUDGET` like every other corpus script, and names any
    repository the budget excluded.
- **`scripts/sync-app-manifest.py` no longer erases what it cannot
  measure.** It only measures rules with matcher lists, and rewrote each
  app's claims from that measurement alone, so one run dropped 231
  hand-maintained detect-only claims (`~beego`, `~ktor`, …) and the
  marker-only `solidity-public` claim — 1,495 entries on OpenZeppelin —
  and deleted every comment inside `APPS`. It now keeps claims for rules
  outside its measurement, claims a marker-only rule it saw enumerate,
  and rewrites entries in place. An empty `APPS_UNVERIFIED=( )` is
  parsed correctly by it, `docs-check.py` and `framework-coverage.py`;
  a lazy `\n\)` match had run on into the next array. Every
  enumerating rule now has a real application: 155 of 155.

## [0.9.1] - 2026-09-24

Fixes the 0.9.0 known issue. The graph is unchanged: every one of the
280 corpus repositories renders byte-identical output on 0.9.0 and
0.9.1. What changes is the audit, which now keeps records it used to
erase.

### Fixed

> **Since 0.2.0, the audit erased unresolved, stdlib and external
> records for calls that share a start byte with a resolved call.** A
> chained call — `runners.GetUITaskRunner()->PostTask(...)`,
> `this->Log().Error(...)`, Python's `a.b().c()` — is two calls that both
> begin at `runners`. Reconciliation cleared every audit record at a
> site that ended up with any edge, so when the inner call resolved and
> the outer one did not, the outer call vanished: no edge and no record.
> That is the "dropped without an audit record" known issue in 0.9.0,
> and it was never C++-specific — 0.9.0 only exposed it, because the
> outer calls now correctly exceed `--fanout-cap` instead of carrying a
> wrong guess. Over the corpus, 229,446 unresolved and 284,580 stdlib or
> external records that previous releases erased are now kept; any
> unresolved-call count taken from an earlier release is low by that
> class. Graphs were never affected.

- **Audit reconciliation is per call, not per site.** A record is cleared
  only when an edge at its site lands on a callable of the record's name,
  or on the constructor of the class that name constructs (`Widget(3)`
  binds `Widget.__init__`). The per-site de-duplication of unresolved
  records is keyed the same way. Regression test:
  `chained_call_dropped_at_the_cap_stays_in_the_audit_when_the_inner_call_resolves`
  in `crates/cgg/tests/resolve.rs`, which fails on 0.9.0.

### Performance

The graph is identical, and the change is one lookup in a single
post-processing pass. Timed at `--jobs 1` on the two largest corpus
repositories, one run each: `dart-flutter` 50.7 s on 0.9.0 and 47.2 s on
0.9.1, `zig-zig` 103.2 s and 103.4 s — no difference beyond run-to-run
noise.

## [0.9.0] - 2026-09-23

A new language, Kivy KV (#5); typed C++ resolution (#8); Python
inheritance fan-out and instance-field types (#7); and a fix for
parenthesised Python imports that had been wrong since 0.2.0. Before
release, every node and edge of 0.8.5 was diffed against this tree over
the whole corpus (280 repositories) and each loss was required to have
an explanation; eight regressions in the C++ work were found that way and
fixed before shipping.

> **Known issue: some chained C++ calls are dropped without an audit
> record.** On dart-flutter and cmake, a call through a chained receiver
> — `task_runners.GetUITaskRunner()->PostTask(...)`,
> `this->Log().Error(...)` — that 0.8.5 bound by name-guessing can be
> absent from 0.9.0's graph *and* from its unresolved list. The guesses
> that disappear were wrong in every case checked (`PostTask` went to
> `CompatTaskRunner::PostTask`; the call reaches `fml::TaskRunner`), so
> the graph loses no correct edge that was verified, but a drop is meant
> to be audited, never silent. About 2,500 medium-confidence edges across
> the corpus are in this class. It will be fixed in a 0.9.x release.

### Compatibility

This is a minor release because the default graph changes materially for
C++ and Python: the resolvers now type receivers, so many name guesses
become bindings and many wrong guesses disappear. Output for every other
language that 0.8.5 supported is unchanged. Measured over the
280-repository corpus against 0.8.5:

- Name-guessed (`medium`) edges that a typed receiver now contradicts are
  gone, replaced where the type resolves by a `high` binding.
- C++ fan-out sets that were guessed over an incomplete candidate list —
  0.8.5 never indexed out-of-line method bodies under their real name —
  now see every candidate; where that exceeds `--fanout-cap` the site is
  audited as `fanout-cap-exceeded` instead of carrying up to five wrong
  edges (about 20,000 edges, 99% of them verified to be this case).
- A C++ prototype and its body are one node; edges to the prototype move
  to the body, and nodes renamed with their true owner or anonymous
  namespace keep their edges under the new name.

`--fanout-cap 0` still means "never guess", and every edge still carries
its confidence and resolver, so consumers that want only bindings can
filter on `high`.

### Added

- **Python inheritance fan-out (opt-in).** `--dynamic-dispatch` (and
  therefore `--dead-code`) emits `dyn` edges from a base method to each
  subclass override, walking past intermediate bases that do not define
  the method — including a bodiless `class Middle(Base): pass`, now
  recorded as a class declaration (`FileFacts::classes`) so the chain does
  not end at it. `__init__`/`__new__`/`__del__` and nested functions are
  excluded. The edges carry resolver `dispatch:inheritance`. The default
  graph is unchanged: this pass does not write `trait_impl_target`, so it
  cannot leak declaration→override edges into C++, Java, or any other
  language that already records bases. This is the template-method shape
  (`BaseStep.run` calls `self.execute`, overridden two levels down) from
  the same reachability audit as the import fix.
- **Python `self.field` types (default graph).** `self.controller =
  Controller(...)` in `__init__` and class annotations (`controller:
  Controller`) propagate as `self.controller` types, so
  `self.controller.open()` resolves the same way Rust `self.store.foo()`
  already did. This *does* grow the default Python edge set: a typed
  receiver replaces a same-name guess. Nested classes are matched by
  full qualified name, so a nested `class App` does not share field
  types with a module-level `class App`. A field assigned two different
  types (`self.game = Game()` here, `BaseGame()` there) is left untyped
  rather than taking the last assignment; a class-level annotation is the
  declared type and wins. `Optional[T]`, `T | None` and quoted forward
  references unwrap to `T`; containers (`list[T]`, `dict[str, T]`) do
  not, because `self.items.clear()` on a `list[T]` is `list.clear`.
- **Generic class-field extraction.** The Python plugin now records every
  class-level `name = Type(...)` assignment in `FileFacts::class_fields`,
  framework-agnostically. Useful for any descriptor-driven library (Kivy,
  Django, SQLAlchemy, Pydantic, attrs).
- **Observer callback matching in framework rules.** `RuleSpec` gains an
  `observer_types` field; when a rule lists type names and a class declares
  a matching field, the corresponding `on_<field>` method is recognised as
  a framework-invoked callback.
- **Value references through `or` / `and` / ternary.** `handler = x or
  _default` and `f = a if c else b` now record the callable as a
  reference, reducing false-positive dead-code reports.
- **Kivy KV (`.kv`) language plugin.** Event bindings become callables
  and a kv→python linker resolves them onto Python methods of the matching
  widget class. Multi-statement suites, `app.root` and a built-in `kivy`
  framework rule (`App.build`, `on_start`/`on_stop`, `on_touch_*`,
  `.bind()`, `Clock.schedule_*`, `on_<prop>` observers) are included.
- **Per-language line-length guard (`skip_reason: long-line`).** Some
  tree-sitter grammars parse in time quadratic in line length; a file
  whose longest line exceeds the plugin's threshold is skipped before
  parsing. Currently only Kivy KV sets a cap (32 KB/line). The guard
  bounds cost per line, not per file — the real fix is a parse-time
  budget, which is a pre-existing gap, not this PR's to solve.
- **Colon-less KV rule headers.** `<BaseChipIcon>` without a trailing
  colon (KivyMD style) is now accepted by the vendored grammar.
- **KV rule recovery after parse errors.** A text-level
  `harvest_unparsed_rule_calls` pass attributes calls to the enclosing
  `<Rule>:` even when the grammar fails to wrap it as a tree-sitter
  `rule` node (e.g. after an unclosed parenthesis).  The same pass mints
  a `kv_rule` node for any rule whose property-expression calls (e.g.
  `text: root.get_setting(...)`) have no enclosing `on_*` binding,
  keeping the graph at one node per rule rather than one per property.
- **20 inherited-property observer names** added to the kivy framework
  rule (`on_text`, `on_state`, `on_active`, `on_press`, `on_release`,
  etc.), so subclass observers of built-in Kivy properties are no longer
  reported as dead code.
- **kv→python unresolved references are audited.** Every kv→python
  reference the linker cannot bind now appears in the audit with a reason
  (`no-candidate-cross-file`, `no-enclosing-callable`, `fanout-cap-exceeded`,
  or `kv-python: N candidates, no owner match`), and the linker honours
  `--fanout-cap`.

### Known limitations

- KV embedded in `Builder.load_string('''…''')` inside Python files is
  not seen by the Kivy plugin.
- A `.h` header is parsed as C unless a `.cpp`/`.cc` sibling shares its
  stem (`header_verdict`). Class declarations in such headers are invisible
  to the C++ resolver improvements above; on Carvera_Community_Firmware
  188 of 323 headers are C-parsed.

### Changed

- **Corpus: one checkout per repository, every checkout under test, and
  a real application behind each framework.** Fourteen directories in
  the benchmark corpus were second clones of repositories already
  present, measured twice by every script that walks the corpus; they are
  gone, and the rows that needed them (`kivy-carvera`,
  `solidity-openzeppelin`, the combined xv6 row) read the one checkout,
  the last through a `REPO_DIR` alias in `scripts/benchmark.sh`. Every
  corpus directory is now an `APPS` entry, so framework coverage runs on
  all of them, with claims measured by `scripts/sync-app-manifest.py`
  rather than written by hand. 119 applications were added — one for each
  framework rule no application exercised and one for each framework
  whose only evidence was its own repository — and no repository was
  removed. Rules never detected in any real application fell from 98 to
  7, and each of those seven is a detection gap in cgg with an
  application already in the corpus to fix it against (dream, eunit,
  oak, padrino, puma, sails, powershell-dsc).
- `scripts/release.sh` measured its self-analysis with `-o /dev/null`,
  which cannot create the `/dev/null.audit.json` sidecar; it uses
  `--no-graph`.

- **README benchmark table re-measured.** The per-language table had not
  been regenerated since v0.4.0 (`f433c76`), so 29 of its 45 rows carried
  counts from a resolver several releases old (ripgrep 6,984 edges where
  the current binary finds 7,008; zod 1,795 callables where it finds
  1,701). Regenerated from this tree. Regenerating with the 0.8.5 binary
  produces identical rows for every existing language, so the movement
  is the table catching up, not this change. A Kivy row is added
  (Carvera_Controller, 642 callables; ctags has no KV parser, so its
  recall column is empty).

### Fixed

> **Parenthesised `from … import (…)` blocks never bound their first and
> last names, in every release since 0.2.0.** Calls through those names
> fell to the same-name fan-out, which is a guess; past `--fanout-cap`
> they resolved to nothing and `--report-unreferenced` reported the called
> function as unreferenced. Any dead-code conclusion drawn from a Python
> tree that uses this import style was affected. Details below.

- **Parenthesised from-imports lost their first and last binding.** The
  Python plugin forwards the import's item list to the resolver as one
  comma-separated blob, and the resolver trims whitespace per item. For
  a multi-line block importing `create_remediation_request` first and
  `_create_client as c` last, the first item arrived as
  `(create_remediation_request` and the last as
  `c)`, so neither registered as a binding. Every call through such a
  name then took the duck-typed same-name path: a `medium` edge to every
  same-named definition in the tree (wrong ones included), and once the
  count passed `--fanout-cap`, no edge at all — a live function reported
  as unreferenced. Middle items in a block were unaffected, which is why
  it survived: it only bit the first or last name. Reported from a
  reachability audit of two large Python codebases; the reporter's shape
  is now `parse_import`'s unit test and the end-to-end test
  `parenthesised_from_import_binds_at_high_confidence_past_the_fanout_cap`.
  The items blob is normalised at extraction (parentheses, line breaks,
  trailing comma) and the resolver trims stray parentheses per item.
  Measured over the 175-repository corpus, Python only: 1,572 edges
  upgraded from `medium` guess to `high` binding, 76 wrong fan-out edges
  removed, 12 edges past the cap restored, 0 high-confidence edges lost;
  dead-code findings +10 / −6, every one of the ten a function whose only
  incoming edge was a false fan-out. Affected repositories include
  powertools-lambda (288 upgrades), py-scrapy (95), py-dash (166),
  app-fastapi-dispatch (37) and pyspark (687).
- **`--fanout-cap 0` is documented as "never guess".** A single same-named
  candidate is still a guess — the receiver may be a third-party object
  cgg never saw — and `0` already suppressed it with an audited
  `fanout-cap-exceeded {candidates: 1}` record; the help text and README
  now say so, for reachability audits where a false edge costs more than
  a missing one.

- **kv→python name-only fallback checked the wrong field.** The bare-
  receiver guard (`root`/`self`/`app`) tested `context` (the rule class
  name) instead of `receiver_hint` (the actual receiver), rejecting the
  exact case it was written to keep. Rooted chains now emit
  `Confidence::Low`.
- **`last_event_def` span leaked across rules.** An `on_*` binding in
  `<A>:` could extend its byte span into the next `<B>:`, corrupting
  `--since` line-range intersection. The index is now saved/restored on
  rule and widget entry, and clamped to the enclosing widget's `end_byte`.
- **`refine_owner_via_fields` resolved to the Kivy descriptor type.**
  `ObjectProperty`, `StringProperty`, etc. are now skipped; the lookup
  prefers the rule class when multiple classes declare the same field.
- **Ternary value-ref widening captured the condition.** `a if cond else
  b` now descends only into the consequence and alternative;
  `not_operator` is no longer descended into.
- **PEP 8 private classes (`_MarkerHoverToolTip`) are classes.** Their
  methods were classified as free functions, and a receiver typed as
  `_CollectErrors` was rejected by the cross-file resolver, which only
  accepted a type name starting with an uppercase letter.
- **Python functions nested inside a method were classified as methods,
  and bare calls ignored lexical scope.** A `def` was a method whenever
  *any* enclosing scope looked like a class, so `def _scan()` inside a
  method was a method and a bare `_scan()` call never bound to it; where a
  module-level `_scan` also existed, the call bound there instead, at high
  confidence. Method-ness is now decided by the immediate scope, and a
  bare call sees a nested function only inside the function that defines
  it, with the innermost definition shadowing outer ones. Calls through a
  receiver (`self.getc()` on a callback passed in as a value) still find
  a nested function by name, because values escape their scope. A class
  defined inside a function whose name is defined more than once in the
  file (tests that each declare a local `_CrawlSpider`) is never used as a
  receiver type, because the resolver cannot tell the copies apart.

#### C++

- **C++ out-of-line definitions used the wrong simple name, and class
  method prototypes were never extracted.** `void Widget::draw(...)` in
  a `.cpp` kept simple name `Widget::draw`, so a call looking up `draw`
  never found the body. Class methods declared as `field_declaration`
  were skipped. The simple name is now the last segment; when any body
  shares a prototype's qualified name and parameter list, the empty
  declaration is dropped. Overloads and anonymous-namespace bodies stay
  separate; parameter names, defaults, comments, `std::` prefixes, and
  `T[]` versus `T*` do not keep a pair apart. After a cv-qualifier the
  next identifier is the type, so `void draw(const uint32_t);` still
  matches `void Widget::draw(const uint32_t n)`.

- **C++ calls through typed locals, parameters, and fields did not use
  that type.** The plugin never filled `local_types`, so `w->draw()`
  fell back to a bare-name lookup. Fields declared in a header were
  invisible to calls in a `.cpp`. Locals, parameters, `new T()` as a
  call to `T::T`, and `FieldType` entries are recorded; a run-wide field
  index rewrites `obj->field->method()` / `this->field->method()`.
  `unique_ptr<T>` / `shared_ptr<T>` fields contribute `T`. Locals are
  scoped to the enclosing function, so two `stream` variables in one
  file keep their own types. Global constructors are named `T::T` so they
  unify with the out-of-line body. A typed call reaches every definition
  of a name, not only the last-indexed one.

- **C++ object-like macros used as receivers were left untyped.**
  `#define THE_APP App::instance` appears as an uppercase identifier, so
  the propagator skipped it and `THE_APP->run()` never bound. Object-like
  `#define`s are recorded as `MacroAlias` (function-like macros stay
  callables; numeric replacements are ignored). A run-wide pass expands
  macro chains and `Owner::field` / `base->field` hops against the field
  index, then rewrites bare macro receivers and `MACRO->field->…` like
  typed locals. A parenthesized replacement `(App::instance)` is
  unwrapped.

- **C++ virtual calls and indexed member-pointer tables did not reach
  overrides.** `s->draw()` on a `Shape*` bound only `Shape::draw`.
  `(m->*table[i])()` was not a named call at all. Bases are recorded on
  methods, `virtual` is kept as an attribute, and `&Type::method` in a
  table initializer is a `MemberPtrTake`. A virtual call fans out to
  every override of that static type; an indexed `->*` call reaches
  every address-taken method in the table, then those overrides. That
  set is the vtable, so it is not sent through the duck-typing cap.
  The extra edges are `dyn` / low confidence — a site runs at most one
  of them — and, like every `Via::Dynamic` edge, they are emitted only
  with `--dynamic-dispatch` (which `--dead-code` turns on); the default
  graph is unchanged.

- **Scope of the C++ resolver changes: C++ files only.** The per-site,
  enclosing-callable local-type lookup and the modifier-aware `Type
  name` parameter parser above apply to files the C++ plugin extracted.
  Every other language keeps the file-wide lookup and the parser it
  was written against; as first submitted, the new versions ran for
  all 45 languages and changed resolution in 95 of 174 corpus
  repositories (PHP alone lost 8,142 typed-receiver edges). With the
  scoping, every non-C/C++ repository is byte-identical to 0.8.5.

- **The C++ resolver changes lost calls 0.8.5 resolved; eight are fixed.**
  Found by diffing every node and edge of 0.8.5 against this tree over the
  whole corpus and requiring each loss to be explained. None of these
  shipped — they were regressions inside the unreleased #8 work — and each
  has a regression test in `crates/cgg/tests/resolve.rs` or the C++ plugin:
  - *Class-qualified member calls* (`this->Base::Close()`) recorded
    `Base::Close` as the called name. That only matched while out-of-line
    bodies carried the same misnamed simple name, which #8 fixed; the call
    is now the method `Close` on the class `Base`.
  - *A header declaration merged into a body elsewhere* disappeared from
    the header, so a caller that `#include`s only the header lost the call
    (mocks in the Carvera firmware). Merged declarations are remembered on
    the header (`FileFacts::unified_decls`) and the include closure reads
    them.
  - *Phantom prototypes*: a macro in front of a method
    (`JSON_HEDLEY_NON_NULL(2) token_type scan_literal(...) { ... }`) made
    the first call inside the body read as a declared method named
    `JSON_ASSERT`, which made every real `JSON_ASSERT` call in
    nlohmann/json ambiguous. A declaration with a `{` before its
    declarator is not a prototype.
  - *Bare calls inside a member function* now prefer the enclosing class,
    as C++ name lookup does, instead of going ambiguous against a
    same-named function elsewhere in the file.
  - *Same-named free functions in independent programs* (cuda-samples
    defines `BlackScholesCPU` once per sample) were merged into one
    declaration with several bodies; the call went ambiguous. A free
    function declared here with bodies in more than one other file keeps
    its declaration, as 0.8.5 did.
  - *Type aliases* — `using Rect = TRect<float>;`, `typedef Foo Bar;` —
    are recorded and followed, so a receiver typed `Rect` looks in
    `TRect`. flutter's engine is written this way throughout.
  - *Template members*: the in-class prototype `Cursor::name` and the
    out-of-line body `Cursor<A>::name` were never paired, so a bare call
    to the member inside the class was ambiguous (zig's libunwind).
    Template argument lists are ignored when pairing.
  - *A typed receiver whose class cgg cannot find* (a template, a COM
    interface, a class in a header parsed as C) dropped the call. It now
    falls back to the receiver as written — what 0.8.5 resolved it with —
    at medium confidence, so typing can add precision but not lose a call.

  The macro-receiver rewrite #8 introduced is also now limited to C and
  C++ sources: a Python or Java receiver that shares a C macro's name is
  not that macro.

### Performance

Paired A/B against the 0.8.5 release commit (`scripts/perf-compare.sh
v0.8.5 3`, median of 3 per repository, baseline built in its own
worktree), load 8.3 / 7.8 / 8.0 at measurement time:

| | 0.8.5 | 0.9.0 | delta |
| --- | --- | --- | --- |
| corpus total, 270 of 280 repos | 258,180 ms | 259,808 ms | +0.6% |

Flat: within the script's 1–1.5% noise floor. The 40-minute budget
stopped before `smithy-protocol-tests` and the nine repositories after
it. Only three repositories whose run exceeds a second moved by more than
300 ms: `erlang-otp` +437 ms (+5.6%), which is new default work — its
vendored asmjit C++ now gets typed resolution — and two that got faster,
`app-bamboobsc-struts` −340 ms and `app-symfony-messenger` −366 ms. The
per-change blocks below were measured when each change landed.

#### Kivy KV support (#5)

Paired A/B against the 0.8.5 release commit (`scripts/perf-compare.sh
11986c1 3`, median of 3 samples per repo, baseline built in its own
worktree), machine load 7.68 / 8.98 / 7.78 at measurement time:

| | 0.8.5 | this tree | delta |
| --- | --- | --- | --- |
| corpus total, 170 of 174 repos | 190,746 ms | 194,152 ms | +1.8% |

The 30-minute budget stopped the run before `verilog-picorv32`,
`vhdl-uvvm`, `zig-http` and `zig-zig`. The script's own noise floor for
this total is 1–1.5%, so +1.8% reads as flat, and it is **not
like-for-like**: `.kv` files were unknown-extension skips in 0.8.5 and
are parsed now. The Kivy application rows therefore carry new default
work, not overhead — `app-carvera-kivy` 296 → 411 ms (23 KV files;
KV parse plus extraction is 178 ms of that run), `kivy-kivymd` 205 →
288 ms. Every non-Kivy row over 150 ms that the 3-sample table showed
more than 5% slower was re-sampled with ten interleaved runs of each
binary, minimum taken: `ocaml-dune` −4.3%, `go-fzf` −1.2%,
`csharp-newtonsoft` −2.3%, `graphql-github` +0.8% (jitter);
`go-caddy` +9.9% is +16 ms on a 162 ms run with byte-identical output
and no Go code path touched (jitter at that scale); and
`app-torch-ultralytics` +5.1% is +12 ms on 234 ms, which Python pays
for the widened value-reference capture (194 more references recorded
on that repository) and the class-field scan. No repository moves by
more than the noise band for a reason in the code. Graph output is
byte-identical to 0.8.5 on 165 of 174 corpus repositories; the nine
that differ are the eight Kivy repositories and `py-falcon` (seven
handlers newly reached through ternary branches).

`cargo test --workspace`: 886 tests, 9 s on a warm build. Pre-commit
hook end to end (tests, release build, README regeneration,
docs-check): 9 s warm.

#### C++ resolver (#8)

Paired A/B against the previous `main` commit (`scripts/perf-compare.sh
e673534 3`, median of 3 per repo, baseline in its own worktree), load
10.0 / 14.2 / 20.1 at measurement time (the corpus identity pass had just
finished):

| | main before | with the C++ resolver | delta |
| --- | --- | --- | --- |
| corpus total, 165 of 175 repos | 192,963 ms | 192,694 ms | −0.1% |

Flat within the script's 1–1.5% noise floor; the 30-minute budget
stopped before `smithy-protocol-tests` and the nine repositories after
it alphabetically. Not like-for-like on C++ files, which now get
prototype unification, typed receivers, macro receivers and a run-wide
field index: `app-carvera-firmware-cpp` 281 → 319 ms and
`app-smoothieware-cpp` 193 → 216 ms in the 3-sample table, re-sampled
with ten interleaved runs of each binary as +5.6% (232 → 245 ms) and
+1.8%; the larger C++-bearing repositories sit at +2.3% (`cmake-kitware`),
+2.5% (`erlang-otp`), +4.0% (`dart-flutter`), +4.3% (`cpp-nlohmann-json`,
+0.6% on re-sampling). Every non-C++ row over 150 ms that showed more
than 5% either way was noise under the decaying load: `go-fzf` +9.6% in
the table is −13.7% over ten runs, `app-godot-demos` +10.0% is −1.8%,
and the same table shows `bash-acme` −20.9% and `app-immich-nestjs`
−14.2% with byte-identical output. Outside C/C++, graph output is
byte-identical to the previous `main` on every repository; no default
graph carries a `dynamic` edge.

#### C++ follow-up fixes

Not yet timed. They change which declarations and receivers a call is
looked up through, not how much work the pipeline does; the release
run's paired comparison against 0.8.5 will measure them with everything
else in this entry.

#### Python inheritance fan-out and field types (#7)

Paired A/B against the previous `main` commit (`scripts/perf-compare.sh
b12637e 3`, median of 3 per repo, baseline in its own worktree), load
11.1 / 10.8 / 17.0 at measurement time:

| | main before | with #7 | delta |
| --- | --- | --- | --- |
| corpus total, 165 of 175 repos | 191,146 ms | 192,306 ms | +0.6% |

Flat within the script's 1–1.5% noise floor; the 30-minute budget
stopped before `smithy-protocol-tests` and the nine repositories after
it. Twelve rows over 150 ms moved more than 5% either way. Eight are
languages this change does not touch and whose output is byte-identical
(`ocaml-dune` +14.9%, `app-gin-photoprism` +9.0%, `app-lemmy-actix`
−11.8%, …). The two Python rows and the two largest non-Python rows were
re-sampled with ten interleaved runs of each binary at `--jobs 1`, minimum
taken: `app-pydantic-core` +0.1%, `app-saleor-celery` +0.0%,
`ocaml-dune` +0.1%, `app-gin-photoprism` +0.8%. No repository moves for
a reason in the code.

Graph effect, 175-repository corpus against the previous `main`: 133
repositories byte-identical, and every difference in the other 42 is in
Python callables. Default graphs gain 5,542 high- and 3,284
medium-confidence edges and 1,926 edges move from a medium guess to a
high binding; 3,763 medium-confidence guesses are replaced by the typed
target. Seventeen high-confidence edges leave the default graph: thirteen
were wrong (`set()` — the builtin — bound to `_LazyEvent.set` in
litestar, and nested functions bound across test functions in scrapy and
pyspark), and four in scrapy's `test_item` are kept at medium because
the local class shares its name with the enclosing test class.
Dead-code findings fall from 765,281 to 763,328 corpus-wide (−2,001
Python findings, +48): each new finding is a function whose only
incoming edge was a same-name guess that the typed receiver now replaces,
or a test-only call through a class with a dynamic base
(`class _XMLSpider(self.spider_class)`), which a typed receiver cannot
follow. No default graph carries a `dynamic` edge.

`cargo test --workspace`: 941 tests.

### Credits

- Kivy KV support, the generic class-field/observer machinery and the
  value-reference widening were contributed by @SergeBakharev in #5,
  validated over five review rounds on eight Kivy applications.
- C++ type-inference improvements (out-of-line bodies, typed receivers,
  macro receivers, virtual and member-pointer fan-out) were contributed by
  @SergeBakharev in #8.
- Python inheritance fan-out, `self.field` types and PEP 8 private-class
  classification were contributed by @SergeBakharev in #7.

## [0.8.5] - 2026-09-18

Security fixes from a review done for a downstream third-party import,
with the reproductions kept as regression tests. Two findings; both
were confirmed on the released 0.8.4 binary before anything was changed.

### Fixed

- **A deeply nested source file aborted the whole process, and took any
  host embedding the library down with it.** Every language plugin walks
  its syntax tree by recursive descent, and nothing bounded the
  recursion. The rayon worker stack was the 2 MiB default, which
  overflows somewhere between 1,000 and 2,000 tree levels — reached by
  input as ordinary as a 3,000-term `a + b + c …` expression, a fluent
  `.m().m()…` chain, an `if / else if` ladder or a 3,000-deep array or
  object literal. Reproduced in C, C++, C#, Go, Java, JavaScript,
  TypeScript, Python, Ruby, Rust, Kotlin, PHP, Lua, Swift, Scala, Groovy,
  Clojure and the YAML/JSON descriptor languages; the `-n 0` path DFS
  (`query.rs::dfs_paths`) overflowed the same way on a 50,000-function
  call chain. A stack overflow is not a panic, so the `catch_unwind` in
  the C ABI and the Python and Node modules could not turn it into an
  error: the process died. Affected every release through 0.8.4.

  Three changes, each necessary:
  `analyze` now hosts the run on a dedicated thread with a 128 MiB stack
  and gives the pool workers the same (`ANALYZE_STACK_SIZE`), because
  rayon lets the calling thread join the work and a deep file could land
  on whichever thread had the small stack; extraction is gated on the new
  iterative `cgg_lang::exceeds_depth`, so a tree deeper than
  `MAX_TREE_DEPTH` (4,000 levels — roughly 2,000 nested calls, which cost
  two levels each, or 4,000 nested operators or brackets) is skipped as
  `skip_reason: too-deep` and counted, never walked; and `dfs_paths`
  stops at 100,000 levels and reports it through the existing
  `truncated` signal. A file that sat on the old cliff now analyzes (the
  2,000–4,000 band used to crash); one past the cap is reported instead
  of killing the run. Regression tests:
  `crates/cgg/tests/deep_nesting.rs` (a 100,000-deep file is skipped and
  audited; 3,000-deep lists and a 3,000-term C expression analyze) and
  `cgg-lang::parser::depth_tests` (the check itself must not recurse: it
  returns on a 100,000-deep tree).

- **`--since` executed commands from the analyzed repository's own
  `.git/config`, and accepted git options through the revspec.** It
  shells out to `git diff` inside the tree being analyzed, and git runs
  what that tree's config names: a `core.fsmonitor` command executed on
  a plain `git diff`, and so did a `diff.<driver>.command` routed by
  `.gitattributes`, both verified on 0.8.4. Pointing `--since` at an
  untrusted checkout was arbitrary code execution. Separately, the
  revspec was passed bare, so `--since=--output=/some/path` made git
  write that file (verified); `since` is also a Python, Node and C
  option, so a service forwarding a caller's revspec inherited it.
  Affected every release with `--since`.

  Git is now invoked with `-c core.fsmonitor=false` (command-line `-c`
  overrides repo config), `--no-ext-diff --no-textconv`, and
  `--end-of-options` immediately before the revspec. All three vectors
  are gone and a `-`-leading revspec is a clean error; `--since HEAD`
  still seeds the changed callable. Regression tests:
  `crates/cgg/tests/since_security.rs` (a repository carrying a
  `core.fsmonitor` command and a diff driver, and an `--output=`
  revspec; the sentinels each would have written must not exist).

> **The 0.8.4 npm package cannot be loaded.** `crates/cgg-node/index.js`
> is generated by `napi build` and hardcodes the platform-package
> version it accepts. It was last regenerated at 0.8.3, every Cargo item
> was bumped for 0.8.4 with every gate green, and the published
> `cgg-callgraphgenerator@0.8.4` tarball (verified from the registry)
> demands `0.8.3` bindings in 27 places while its platform packages ship
> as 0.8.4 — so `require("cgg-callgraphgenerator")` throws
> `Native binding package version mismatch` on every platform.
> Affected: 0.8.4 only, npm only; the CLI, crates, wheel and C ABI were
> unaffected. `index.js` is regenerated for this release — and so is
> `index.d.ts`, which had been left without the `nodeIds` option and
> the `toMermaid(nodeIds)` parameter the module already accepted — and
> `docs-check.py` check 13 now fails the commit whenever the embedded
> version and `package.json` disagree, which the pre-commit hook runs.

### Added

- `SkipReason::TooDeep` / audit slug `too-deep`, for the depth gate above.
- `scripts/docs-check.py` check 13: npm binding-version parity.

### Compatibility

- The default graph is unchanged: 25 corpus repositories rendered
  byte-identical mermaid on 0.8.4 and 0.8.5, and no real file in them
  reaches the depth cap. The cap only ever trades an abort for a
  reported skip.
- A `--since` revspec beginning with `-` is now an error instead of a
  git option. A revspec that names a revision is unaffected.
- Memory safety was checked, not assumed: the 44 grammars under
  AddressSanitizer on 315 malformed inputs (nine shapes per grammar,
  budget-bounded) and 145 of the 164 corpus repositories, plus Valgrind
  memcheck on the native binary over two real repositories, found no
  heap, use-after-free or uninitialised-read defect. Every sanitizer
  report reduced to the stack overflow fixed above.

### Performance

`scripts/perf-compare.sh`, 0.8.4 (`668fde7`) against this change built
from the working tree, median of 7 alternating runs per repository at
`--jobs 1`. The corpus sweep is budget-bounded at 30 minutes and
stopped before `erlang-otp`, so this is **98 of the 164 corpus
repositories**, the smaller ones. **Load at measurement: 7.2 rising to
about 10 on a 5-day-up host** — an unrelated vLLM inference server held
roughly five cores the whole time — which inflates every absolute
number and widens the spread; the alternation keeps the *relative*
figure meaningful.

| | 0.8.4 | 0.8.5 | delta |
| --- | --- | --- | --- |
| **total, 98 repos** | **97,449 ms** | **99,647 ms** | **+2.3%** |
| `app-ombi-quartz` | 597 ms | 644 ms | +7.9% |
| `app-django-netbox` | 3,121 ms | 3,327 ms | +6.6% |
| `app-ghostfolio-nestjs` | 223 ms | 236 ms | +5.8% |
| `app-black-click` | 790 ms | 835 ms | +5.7% |
| `app-metabase-compojure` | 6,030 ms | 6,356 ms | +5.4% |
| `elixir-phoenix` | 187 ms | 197 ms | +5.3% |
| `app-avalonia-samples` | 368 ms | 387 ms | +5.2% |
| `app-blazor-samples` | 402 ms | 422 ms | +5.0% |

Those are every repository over 150 ms that moved 5% or more; none
moved the other way. The script's own noise floor on the total is
1–1.5%, so +2.3% is a small real cost, about a point above jitter, and
it has one identifiable source: **the depth gate is one extra iterative
walk over every node of every parsed tree**, before extraction, and its
cost scales with node count. It is corpus-wide rather than one
repository — the rows above are node-dense trees, not outliers — and
the alternative is a bound that cannot be trusted, so it ships. The
larger thread stacks are reserved virtual memory and cost nothing
measurable; the hosting thread is one spawn per analysis.

**Like-for-like:** the only work the new binary does that 0.8.4 did not
is that traversal. No default changed. Output is unchanged: 25 corpus
repositories rendered byte-identical mermaid on both binaries, and no
real file reaches the depth cap.

Test suite: `cargo test --workspace` 17.5 s wall (847 tests) under the
same load. Pre-commit hook: its test run plus release build measured
60.3 s wall directly; the README regeneration and docs-check add about
two seconds.

## [0.8.4] - 2026-09-17

### Fixed

- **The Rust cross-file resolver emitted guessed edges at sites it had
  already bound, into helpers it could not reach, and by bare name where
  the receiver was known.** Measured on cgg's own tree: 1,976 of 5,449
  edges were `cross-file:imports / medium`; 299 landed on a site the
  intra-file pass had already resolved (the dedupe key included the
  destination, so a fan-out to *other* destinations passed), 550 pointed
  into another file's private `mod tests` helper, 212 came from calls
  inside macro token trees whose receiver had been dropped, and 405 came
  from a lowercase path-headed receiver (`super::dynuse::extract`,
  `serde_json::from_str`) treated as a variable and fanned out. Every
  class was found by reading the audit sidecar and confirmed against the
  source before anything was changed.

  Ten resolver changes, each with a unit test that fails on 0.8.3:
  cross-file skips a site intra-file already bound, keyed
  `(src, site_byte, name)` so chained calls at one byte survive; fan-out
  candidates are filtered by `Vis::Private` and by a `::tests::` module
  the caller does not import, with the fan-out cap judged *before* the
  filter so a filter can only shrink an edge set; macro-argument calls
  keep their receiver (`p.id()`, `NodeIds::resolve(..)`,
  `TrustKind::Network.f()`), drop turbofish, retry bare only on a
  crate-wide unique name, and bind through by-name fan-out only when a
  single candidate survives (an impl preferred over its trait's own
  declaration); `use crate::x::y;` and `pub use` chains have their
  leading `crate`/`self`/`super` rewritten so they match the callable
  index; a `::`-path receiver rewrites `crate`/`super`/`self` from the
  caller, tries every prefix as an owner (enum-variant receivers,
  builder chains), and matches an external-crate head only by full
  path; `owner_from_qn` no longer files `<A as TryFrom<B>>::try_from`
  under `B`; `<StdType as Trait>::m` impls are not indexed under the
  bare std owner unless the workspace declares a type of that name;
  let-type inference sees through `Arc/Rc/Box/RefCell/Cell/Mutex/RwLock/
  Pin/Cow` and `.clone()`; FFI pass B requires the language family and
  owner to match; integration-test files are qualified
  `<crate>::tests::<file>` instead of colliding on the crate root.

- **Value references were logged as `ambiguous-in-file` calls.** 940 of
  the 972 such entries on cgg's own tree were bare identifier arguments.
  They now carry `value-ref-ambiguous` / `value-ref-no-enclosing`; the
  dead-code roots and evidence passes accept both alongside the old
  reasons, so a function passed as a value at module scope keeps its
  `toplevel:invocation` root — on flask, 1,024 roots and 127 findings,
  **identical to 0.8.3** (re-measured 2026-09-16 against a fresh clone;
  earlier drafts of this entry and the PR description quoted 113 and 126,
  two different runs against two different flask revisions, and the
  count moves with upstream flask — the identity is the claim, not the
  number). A `Via::Reference` edge is emitted for any target except a
  closure bound to the same name.

- **`--write-roots` wrote a baseline that could not be read back.**
  `render_baseline` interpolated `regex::escape`'s output straight into a
  TOML *basic* string, so any qualified name carrying a regex
  metacharacter emitted `\.` — not one of the escapes TOML recognises
  there. Every dot-joined name hit it, so the command produced an
  unloadable file for any tree containing Python. Escaping now happens
  for the basic string on top of the regex escaping, innermost first; a
  TOML *literal* string would avoid the second layer but cannot hold an
  apostrophe, and Rust lifetimes put those in qualified names
  (`ExtractCtx<'a>::new`). `baseline_round_trips_and_has_no_timestamp`
  missed it because it rendered a *default* report — no findings, so no
  `[[allow]]` line, so the escaping never ran.

- **git invocations inherited the ambient git environment.** Git exports
  `GIT_DIR` and `GIT_INDEX_FILE` to every process it spawns, and they
  outrank both `-C <path>` and `current_dir`. The pre-commit hook runs
  `cargo test --workspace`, so the `--since` fixtures ran `git init` and
  `git add -A` against the real repository that was mid-commit —
  observed setting `core.bare=true` on it and staging the entire working
  tree. `since.rs` had the same hole with a quieter failure: `--since` is
  documented as anchored on the tree being analyzed, but a leaked
  `GIT_DIR` overrode the `-C <toplevel>` meant to enforce that, so a run
  from inside any hook resolved the revspec against whichever repository
  invoked the hook. Both call sites now strip the eight repo-scoping
  variables.

### Changed

- **The graph document schema is `cgg.graph.v2`.** No field changed
  meaning; two `UnresolvedReason` tags were added, which is additive by
  the rule in `cgg_format::json::GRAPH_SCHEMA`. The readers already
  shipped do not treat it that way — through 0.8.3 an unrecognised
  `stage` is a hard serde error, so a 0.8.3 binary rejected the whole
  document at a byte offset (``unknown variant `value-ref-ambiguous` ...
  at line 14346 column 38``) rather than reporting a version it was too
  old to read. The tag makes that failure legible:
  `declares schema "cgg.graph.v2", but this cgg reads "cgg.graph.v1"`.
  `--from-graph` still reads `cgg.graph.v1`, so no document written by an
  earlier release is orphaned, and 0.8.4's own reader no longer has the
  flaw — an unknown `stage` degrades to `UnresolvedReason::Other` — so a
  genuinely additive change after this one will not need a bump.

### Added

- **`--skip-minified`, off by default.** With it, the walker skips
  `.min.{js,mjs,cjs,css}` and any file with one of those extensions
  averaging more than 2,000 bytes per line, recording each as
  `skip_reason: minified` with a summary count. Without it — the
  default — those files are analyzed like any other source.

  Opt-in because a bundle is *real, parseable source*: the filter is
  accurate, but an accurate filter nobody switched on still removes
  callables the caller never agreed to lose, and this is the only
  graph-content switch in cgg that removes rather than adds. Measured
  on `app-wordpress`: 2,313 analyzed / 27,361 callables at the default,
  2,117 / 19,051 with the flag, 612 files skipped; 1,681 -> 990 ms,
  minimum of three at `--jobs 8` after a warm-up.
  `walk::minified_files_are_analyzed_by_default` locks the default and
  fails if the gate is ever removed.
- **The auto `--jobs` cap rises to 32 once a host has 32 physical cores**
  (was fixed at 8). Graph byte-identical at any job count.

### Compatibility

**The default graph shrinks, deliberately.** This departs from the
standing rule that the default edge set only ever grows. On cgg's own
tree the change removes 765 unique edges and adds 79; by call-site byte
the removals break down as 550 into another file's private `mod tests`
helper, 188 at sites that keep a surviving edge (a fan-out narrowed to
one target), 14 FFI edges from a Python stub into the Node binding, and
20 sites left with no edge, every one read by an independent reader
(3 were real calls). On the firecracker sources in `llmitm-v5`
(`vercel_sandbox/src/firecracker`): 2,180 removed (1,040 narrowed, 446
external-head or std-impl, 268 tests-helper, 426 sites left dark: of
357 read individually 25 were real calls, the rest are the
macro-argument width rule, roughly 36 real by the sampled rate), 867
added. **A 0.8.3 binary cannot read a graph this version writes, and says so
badly.** Two `UnresolvedReason` variants are new, and 0.8.3's derived
`Deserialize` has no fallback for a tag it does not know, so
`--from-graph` fails with
`unknown variant value-ref-ambiguous ... at line 98994 column 38`
rather than a version message. The document still declares
`"schema": "cgg.graph.v1"`, which 0.8.3 accepts, so nothing in it
announces the break.

The reader is fixed here rather than the tag: an unrecognised `stage`
now deserializes into the existing `UnresolvedReason::Other(String)`
variant — documented as "any other / legacy reason, preserving the
original text" — keeping the original text so `slug()` and metrics
bucketing still name it. Verified by rewriting 935 `value-ref-ambiguous`
tags in a real graph to an invented one: 0.8.3 refuses it, this version
replays it to a byte-identical mermaid graph. The fallback is narrow on
purpose — it fires only for a tag outside `KNOWN_UNRESOLVED_STAGES`, so
a *known* tag with a malformed `detail` is still a hard error rather
than being quietly reclassified.

`cgg.graph.v1` therefore stays honest from 0.8.4 on: a future reason is
an additive change, not a break. What it does **not** do is help a
0.8.3 binary already installed — that one-time cost stands. A schema
bump is the only change that would reach an installed 0.8.3 (which
already reports a declared mismatch for a schema string it does not
know), at the cost of a second accepted tag; it is left as the
maintainer's decision.

**Dead-code baselines over Rust integration tests must be
regenerated.** Qualifying integration-test callables
`<crate>::tests::<file>::<name>` instead of the crate root fixes a real
collision (`cgg::fixture` was defined in three different test files),
but `--write-roots` writes each accepted finding as an anchored regex
over the qualified name, so every existing entry for a Rust
integration-test callable stops matching and those findings return.

Reproduced on cgg's own tree: 0.8.3 writes a 1,004-entry baseline
with `--write-roots --include-tests`; read back against the same
source with `--dead-code --include-tests`, 0.8.3 reports 0 findings
and this version reports 79 — 68 of them the requalified `::tests::`
names, with 36 more baseline entries stale. It bites
only under `--include-tests`, since test-scope findings are withheld by
default, so the blast radius is anyone running `--include-tests` with
`--fail-on-dead` in CI: **re-run `--write-roots` on upgrade.** Node ids
are content hashes of the qualified name, so id-keyed tooling shifts
for the same callables.

The rename is not reverted: the collision it fixes is worse than the
one-time regeneration. Keying baselines on `path:line` instead of the
qualified name would make them immune to this class of change and is
worth its own discussion — it trades immunity to renames for
sensitivity to every edit that moves a line, which is the more frequent
event.

Not covered, and still wrong: a single-segment external receiver
(`serde_json::from_str(..)` arrives as receiver `serde_json`,
indistinguishable from a local variable) still produces 64 false edges
on cgg's tree.

### Performance

**Per configuration, because the three changes here have nothing to do
with each other and only one of them is on by default.**

`scripts/compare-release.py`, 0.8.3 release binary (`e86b6be`) against
this change, over the 110 repositories `scripts/benchmark.sh` lists,
re-cloned 2026-09-16. Minimum of two alternating runs, **one repo at a
time** — the script's `--jobs 1`, which its own help calls the setting
for published numbers. The identical comparison measured eight
repos concurrently reads −3.0% where the serial one reads +1.3%: the
two arms did not slow equally under load (0.8.3 +13.5%, this change
+8.7%) and the cause was not isolated, so the serial figure is the
one quoted. The pinned-jobs arm is one replicate. Run-to-run spread on
the 0.8.3 arm across the three configurations below is 99.6-100.2 s,
so ~0.6%.

| configuration | wall | graph |
| --- | --- | --- |
| **default, host under 32 physical cores** — the resolver changes alone, both binaries pinned `--jobs 8` | 100.0s -> 101.3s (**+1.3%**) | edges 2,641,237 -> 2,491,103 (-5.7%); callables +9 |
| **default, host at 32+ physical cores** — the same graph, plus `MAX_AUTO_JOBS_WIDE_HOST` | 100.2s -> 70.4s (**-29.7%**) | same counts as the row above: 1,202,660 callables, 2,491,103 edges |
| **`--skip-minified`** (opt-in), both pinned `--jobs 8` | 99.6s -> 88.5s (**-11.1%**) | callables 1,202,651 -> 1,150,838 (-4.3%); edges -9.0%; unresolved sites -8.3% |

Read together: **on an ordinary host with default flags this release
costs about 1% of wall and returns a 5.7% smaller, more precisely
resolved edge set.** The speedups are real and both are available on
request — one by passing a flag, one by having the cores.

Within the same run, the 108 repos whose callable count is unchanged
read +1.2%; in the separate `--skip-minified` run, the 87 repos the
skip never touches read +1.0%. Repos under 150 ms are noise by this
script's own note; restricting to the 63 above it moves nothing
(+1.3% / -30.6% / -11.5%).

**Edges do not fall everywhere.** Across these 110 repos no repo gains,
but that is a fact about this corpus and not about the change — two
repos outside it do gain, measured directly: `rust-ntex`
17,650 -> 19,173 (**+1,523**) and `csharp-mediatr` 418 -> 420 (+2).
Both were reported by a reviewer running a larger corpus, and neither
is in `benchmark.sh`: `compare-release.py` iterates the corpus
*directory*, not the manifest, so two people on the same commit can be
measuring different repo sets with nothing saying so.

`determinism-sweep.py`: 25 repos x 3 runs, 0 nondeterministic. Graph
byte-identical (mermaid) at `--jobs 1/4/8/16/32` on `rust-ripgrep`,
`app-wordpress` and `ts-zod`, with and without `--skip-minified`.

**Independently replicated on a second corpus.** The same three
configurations, measured by a reviewer against the 0.8.3 release binary
over a different and larger corpus — 164 repository directories rather
than the 110 `benchmark.sh` names — on a 32-physical-core host, serial,
minimum of two alternating runs after a discarded warm-up:

| configuration | wall | graph |
| --- | --- | --- |
| resolver alone, both pinned `--jobs 8` | 287.5s -> 286.1s (**-0.5%**) | edges 5,109,423 -> 4,812,000 (-5.8%); callables +12 |
| default `--jobs`, 32 physical cores | 285.6s -> 213.7s (**-25.2%**) | same counts as the row above |
| `--skip-minified`, both pinned `--jobs 8` | 284.8s -> 270.0s (**-5.2%**) | callables -3.6%; edges -8.4%; unresolved -7.4% |

The 0.8.3 arm was re-measured in each of the three sweeps and spread
287.5 / 285.6 / 284.8 s — about 0.9%, which is this harness's noise
floor and wider than the -0.5% pinned result. **The resolver changes are
therefore indistinguishable from wall-neutral on this corpus**, against
+1.3% on the smaller one; the two runs agree that the cost is at most
about a percent, and disagree on its sign, which is what a sub-noise
effect looks like. Edge and callable deltas, being deterministic, agree
closely across both corpora (-5.8% vs -5.7% edges).

The `+12` callables in the pinned row are not analyzed source: they are
`<framework-entry>` nodes. Integration-test handlers in different
`tests/*.rs` files previously collided under one bare crate-root name
and deduplicated into a single entry node; the `tests::` requalification
makes them distinct, which is the collision that change exists to fix.
Real file-backed callables are byte-identical — checked on `rust-salvo`,
17,678 before and after, zero added and zero removed.

Four repos in this corpus gain edges — `rust-ntex`, `csharp-mediatr`,
`rust-bat`, `rust-ripgrep` — where the 110-repo corpus has none. The
last two are the same repositories at different revisions and move in
opposite directions on the two clone dates, so a per-repo edge delta is
a fact about a revision, not about the release.

## [0.8.3] - 2026-08-26

### Fixed

- **`--rollup` and every large tree got dramatically faster: two
  accidental quadratics removed from type propagation.** Strategy 4 of
  the receiver-type propagator asks, per receiver, "which functions
  return a type named like this receiver, and is one of them called
  earlier in this file". It answered both halves by brute force. For the
  first it walked the **corpus-wide** `return_types` map once per
  receiver, allocating a `to_lowercase()` and a `format!()` per entry.
  For the second it scanned every reference in the file, once per
  surviving candidate.

  On `erlang-otp` that measured **817,607,605 map iterations** and
  **1,864,725,063 reference-scan steps**, costing 60.2s of a 91s run —
  66% of wall.

  Both inputs are loop-invariant. The lowercasing depends only on the
  map, so `ReturnTypeIndex` hoists it to once per run. The "called
  earlier" predicate only ever needs the *earliest* bare call per name,
  so it is answered by scanning for the first few lookups and by a
  per-file map after that — whichever is cheaper for the file in hand.

  **`erlang-otp`: 19,819 -> 10,834 ms (-45.3%)**, median of 9 alternated
  pairs on an idle host, and 95,851 -> 33,857 ms (**-64.7%**) at
  `--jobs 1`.

  The variance matters more than the median. 0.8.2's nine runs of that
  repo ranged **19,116-36,222 ms** — a 90% spread on identical input.
  0.8.3's range is 10,271-11,120 ms, an 8% spread. The quadratic's cost
  scaled with allocator and scheduling pressure, so removing it removed
  the unpredictability too. That is why this one repository has poisoned
  every corpus perf sweep it appeared in: the +38.6% attributed to it
  during the 0.8.2 measurements was this variance, not a real delta.

- **`--profile` works on release builds; the README said it did not.**
  `profile.rs` moved from `#[cfg]`-compiling spans out of release to a
  runtime flag, for the reason its own doc comment gives — a debug build
  distorts the ratios you are reading, and a pathological input is
  exactly when you need the truth. The flag table was never updated.
  This mattered: the investigation above depended on profiling a release
  build of a 177 MB tree, which the README described as impossible.

### Added

- **`profile::count` — count-only tallies, reported beside the span
  table.** A span answers "how long"; a tally answers "how many", which
  is the question when the suspicion is an accidentally-quadratic loop.
  Time alone cannot distinguish one slow iteration from ten million fast
  ones, and that distinction was the whole diagnosis here. Same
  `ENABLED` gate as spans, so a run without `--profile` pays one relaxed
  atomic load and a predicted branch.

- Per-language instrumentation of the type propagator, behind
  `--profile`. The attribution is load-bearing rather than decorative:
  `erlang-otp` was assumed to be an Erlang problem, and the first
  measurement put 1.86 billion inner-scan steps in an unnamed catch-all
  bucket. Naming C and C++ showed **`cpp` alone accounts for 93% of
  them, from 171 vendored files**, while Erlang's 4,106 files and
  731,928 references contribute essentially none. The repository is slow
  because it vendors the BEAM VM's C++ sources.

### Compatibility

**No graph changes.** Verified against 0.8.2 across all 164 benchmark
repositories, none excluded: `-t json` byte-identical once timings are
normalised, `-t mermaid` byte-identical, and the ordered callable-id
list identical element-wise. Order was checked as strictly as identity
because 0.8.2 numbers mermaid ids by graph position, so a reordering
that preserved both sets would still rewrite every id and every arrow.

Per-language totals were compared separately, because "no repo changed"
is not the same claim as "no language changed": **47 languages
exercised, 0 changed** — files, callables and edges identical for every
one.

The comparison normalises `cgg_version` out of the JSON document. It
embeds the version, so a released binary and a bumped one differ on
every repo for a reason that has nothing to do with the graph — and an
earlier run of this check passed only because it compared 0.8.2 against
an unreleased branch still calling itself 0.8.2. Mermaid carries no
version string, which is what exposed it.

### Performance

Paired and alternated, median of 3 runs per repo at `--jobs 16`, 164
repos, against the 0.8.2 release binary:

| | 0.8.2 | 0.8.3 |
| --- | --- | --- |
| corpus total | 294.8s | 266.2s (**-9.7%**) |
| median per-repo | — | **-4.3%** |
| repos >5% faster | — | 60 |

Largest wins: `aws-lambda-go` -71.1%, `go-fzf` -67.0%,
`csharp-mediatr` -63.5%, `erlang-otp` -43.2%, `c-redis` -29.2%.

**On regressions, and on a measurement mistake worth recording.** An
earlier revision of this change built the per-file map unconditionally,
which charged an O(references) pass to every file in every language to
fix a cost only some of them pay. That is the failure mode this entry
exists to avoid, and it was caught by per-repo timing rather than by the
total — which read a healthy -8.9% throughout.

The regression list it produced was itself misleading. Filtered at a
150 ms floor it named 15 repos; **14 of those had sub-second baselines
and are not measurable on this host**, where identical runs of the same
two commits gave `fsharp-paket` +49.4% and then -34.3%. Timing here is
now reported only for repos with a baseline of **1 second or more**, and
a regression is not treated as real unless it reproduces across runs. By
that standard no reproducible regression was found at any point.

## [0.8.2] - 2026-08-25

### Added

- **`--node-ids short|hash`, and mermaid now numbers its nodes by
  default.** A mermaid node id is written once in the node's declaration
  and again on both ends of every edge that touches it — on cgg's own
  tree, 2,232 nodes against 3,792 edge pairs — and it was a
  ten-character base36 content hash. That is the worst possible string
  for a BPE tokenizer to carry three times per node, and the primary
  consumer of this format is a coding agent reading it in a context
  window, where those tokens are the whole budget.

  Nodes are now numbered `N0`, `N1`, … in graph order. Across the
  164-repo benchmark corpus that takes the mermaid output from
  **217,119,809 to 172,590,073 bytes — 20.5% smaller** (24.1% on cgg's
  own tree, which is denser in `::`-qualified names than the corpus
  average). The token saving is larger than the byte saving, because
  `N7` is one token where `Cu7kwiat260` is several. `--node-ids hash`
  restores the previous output, which is what you want to diff two
  revisions' diagrams or line one up against `-t json`. Only mermaid's
  default changed; dot and graphml still hash unless asked, and `-t json`
  cannot be renumbered at all — its ids are the identity `--from-graph`
  reads back — so asking there prints a warning rather than quietly
  doing nothing.

  **Verified corpus-wide that no node or edge moved.** Over all 164
  repositories, none excluded: `-t json` from the old and new binaries is
  identical once per-run timings are normalised — every callable, edge,
  id, confidence and resolver provenance — and `--node-ids hash` is
  byte-identical to the old default output. The numbered rendering
  declares exactly the same nodes and draws exactly the same arrows as
  the hashed one: **2,148,745 nodes and 2,433,611 arrows checked, zero
  differences.**

  **The qualified name is deliberately not the id.** It looks like the
  obvious answer and is wrong twice over. Qualified names are not
  unique — 41 of cgg's own 2,232 callables share one with another
  callable, and corpus-wide 17.7% of callables share
  `(language, file, owner, qualified_name)` because C++, C#, Java and
  Erlang have overloads in quantity — so using the name as the id merges
  distinct functions and silently reroutes their edges. And it is
  *bigger*: a name repeated on every edge renders the same graph at
  379,524 bytes, 39% worse than the hash it would have replaced.
  Numbering is the only scheme that is both smaller and collision-free,
  and it is collision-free by construction.

  Reachable from all four front ends: `--node-ids` on the CLI,
  `node_ids=` on `cgg.analyze()` and `Graph.to_mermaid()` in Python,
  `nodeIds` in the Node options and `toMermaid("hash")`, and
  `"node_ids"` in the C ABI's options JSON — no new exported symbol,
  which is what passing options as a document is for.

- **A prebuilt `linux-aarch64` CLI binary.** Wheels have shipped for that
  target for several releases while the CLI did not, so `cargo install`
  — a from-source compile — was the only route on ARM Linux. That was a
  gap in the release matrix, not a limitation: the node job already
  cross-compiles the identical Rust and the identical 44 tree-sitter C
  grammars for `linux-arm64-gnu` on an x86 runner, and the CLI now uses
  the same toolchain and environment.

  Verified before it was added, on hardware rather than by symmetry:
  cross-built on x86_64 and executed on a real aarch64 machine, where
  its mermaid, dot and graphml output over the same tree is
  **byte-identical** to a natively-built binary, and the `--from-graph`
  and `--dead-code` paths both run. CI cannot make that check — an x86
  runner cannot execute an ARM binary, and its ARM smoke tests are
  skipped for exactly that reason — so the job asserts the weaker thing
  it can: that `readelf` reports AArch64, which catches a cross-build
  that silently produced an x86 binary under an ARM name.

### Fixed

- **`--rollup` measured its budget against the wrong renderer once the
  node-id scheme became selectable.** The budget is a claim about the
  *rendered* size, and the rendering is a third larger under
  `--node-ids hash` than under numbering. Measuring the numbered form
  while emitting the hashed one would have let a run sail past the budget
  it was given — silently, because the stderr line reports the figure it
  measured, not the document it wrote. The scheme is resolved once
  (`NodeIds::resolve`) and both the budget and the artifact read that one
  answer; a regression test picks a budget strictly between the two
  renderings of one graph and asserts the schemes disagree about whether
  it fits.

  Caught before release, so no shipped version is affected — the flag and
  the defect arrived in the same unreleased change.

- **`scripts/docs-check.py` check 8 read the wrong `#[pyo3(signature)]`.**
  It searched for the first one in `crates/cgg-py/src/lib.rs`, which was
  `fn analyze`'s only for as long as no other function had one. The first
  renderer to take a keyword argument shifted it, and the check reported
  all 28 of `analyze`'s keywords as missing. It is anchored on
  `fn analyze` now.

### Changed

- **crates.io is published by the release workflow, gated on every other
  registry succeeding first.** It was manual by design, and the design
  was half right: crates.io *is* the one channel where a mistake cannot
  be undone, because a version can never be re-uploaded — only yanked.
  But the answer to that is ordering, not a human. Every other channel is
  recoverable (PyPI takes `skip-existing`, npm can be re-run, a GitHub
  release can be recreated), so they now all prove themselves first and
  the irreversible act happens last, in a `crates` job with
  `needs: publish`. If wheels, npm or the release binaries fail, no crate
  is uploaded and the version number is still free.

  Requires a `CARGO_REGISTRY_TOKEN` repository secret. It keeps
  `environment: release`, so an approval gate can be put in front of it
  in repo settings.

  A `preflight` job checks all three publish credentials before anything
  is published. Without it a missing crates.io token surfaces at the very
  end — GitHub, PyPI and npm ship, then `crates` dies on auth and three
  of four registries sit at the new version. Checking inside `crates`
  would be correct and useless: by then the irreversible ordering has
  already released the recoverable channels.

  Three supporting changes in `scripts/publish-crates.sh`, which the job
  runs rather than reimplementing:

  - `--yes` skips the typed-version confirmation. CI has no tty; the
    prompt stays the default for humans, who can still change their mind.
  - **Resumable.** `already uploaded` now counts as success. Because the
    version is spent either way, a run that dies after three of six
    crates has to be *finishable* — previously a re-run failed on the
    first crate that had landed, leaving the workspace half-published
    with no way forward.
  - The last crate is verified on the index like every other. It was
    skipped because nothing depends on it, which is true for *ordering*
    and wrong for *verification*.

  Resumability creates a hole on its own — a tag whose manifest disagreed
  with it could report success while publishing nothing, since every
  crate would read as already uploaded. The job therefore asserts the tag
  matches `Cargo.toml` before it runs, and independently reads back all
  six crates from the registry afterwards.

### Compatibility

**Mermaid node ids changed shape, and this ships as a patch release.**
`cgg -t mermaid` now writes `N0`, `N1`, … where 0.8.1 wrote
`Cu7kwiat260`. Anything that parses cgg's mermaid ids — rather than its
labels — needs `--node-ids hash` to keep the old form. Nothing else
changed: dot, graphml and json are byte-identical to 0.8.1's, and the
hashed mermaid rendering is byte-identical too.

By this project's own versioning policy a new flag is a MINOR bump, and
a changed default output arguably more so. It ships as 0.8.2 by explicit
decision, on the grounds that the **graph** is provably untouched — see
the verification below — and only the rendering moved. Calling that out
here rather than letting a patch number imply nothing happened.

**Numbered ids make graph order observable, so order was verified too.**
A content hash is a pure function of a callable's identity: before this
release, if a parallel phase had ever emitted callables in a different
order, every id stayed put and the diagram was unchanged. An ordinal has
no such protection — a reorder would shift every id and every edge line
with it. `scripts/determinism-sweep.py` deliberately stopped varying
`--jobs`, and `tests/determinism.rs` covered thread counts only on a
small fixture, so this combination had no coverage on real input.

It does now, three ways. A new test,
`mermaid_is_byte_identical_at_every_thread_count`, pins the rendering at
`--jobs 1/2/3/8/32` with the hashed form as a control. Across the corpus,
**157 repositories were rendered at multiple worker counts and every one
was byte-identical** — 152 at `--jobs 1,2,8,32` and the five largest at
`--jobs 1,2,3,8,32,64`, none excluded. And the corpus A/B below settles
the root question: the JSON is byte-identical to 0.8.1's, and JSON
serialises callables in graph order, so the order these ordinals read is
the order 0.8.1 already shipped.

**The graph is unchanged, and that was checked rather than assumed.**
Across all 164 repositories of the benchmark corpus, none excluded:
`-t json` from 0.8.1 and 0.8.2 is identical once per-run timings are
normalised — every callable, edge, id, confidence level and resolver
provenance — and `--node-ids hash` output is byte-identical to 0.8.1's
default. The numbered rendering declares exactly the same nodes and
draws exactly the same arrows as the hashed one: **2,148,745 nodes and
2,433,611 arrows compared, zero differences.**

### Performance

**The project's own perf gate cannot measure this change, and saying so
is the honest headline.** `scripts/perf-compare.sh` benchmarks with
`-t json -o /dev/null` (`bench_one`). This release touches the *mermaid*
writer; the JSON path it times is byte-identical to 0.8.1's. So the table
below is a regression check on the shared pipeline — walk, parse,
resolve — and nothing in it can be an effect of the change.

Paired A/B, `146bb7b` (post-0.8.1) against this commit, both built on the
same 64-core host, median of 3 runs per repo at `--jobs 1`, 153 of 164
repos (the sweep was stopped at its time budget), load average 2.08 at
start:

| | total | median per-repo | faster / slower |
| --- | --- | --- | --- |
| repos ≥150 ms (90 of them) | — | **+0.0%** | 43 / 40 |
| all 153 repos | 240,095 → 250,373 ms (+4.3%) | | |
| all 153 **excluding `erlang-otp`** | **+0.1%** | | |

**That +4.3% is one repository.** `erlang-otp` went 26,181 → 36,277 ms,
which is **98% of the entire delta** while being 10.9% of the baseline
total. It is the corpus's known pathological case — 177 MB, 3,851 `.erl`
files, the repo CLAUDE.md records as having run for 3h40m in an
unguarded sweep. A formatter change cannot cause it: the timed command
never renders mermaid. Reported here rather than smoothed into the total,
because promoting exactly this shape to a corpus-wide claim is the
mistake 0.6.2 had to retract.

Corroborating that the deltas are noise: `rust-salvo` read **-20.3%** in
one sweep and **-0.4%** in a clean repeat of the same two commits.

**What did change, measured directly** — the rendered artifact, which is
what mermaid's cost actually is for its consumer:

| `cgg ./crates -t mermaid` | bytes | `o200k_base` tokens |
| --- | --- | --- |
| 0.8.1 (hashed ids) | 275,772 | 127,536 |
| 0.8.2 (numbered ids) | 209,237 | 80,360 |
| delta | **-24.1%** | **-37.0%** |

Corpus-wide, all 164 repos: 217,119,809 → 172,590,073 bytes, **-20.5%**.
The token saving outruns the byte saving because a random base36 string
costs roughly one token per two characters while `N7` is one token.

**Recall under `--rollup` improves, and that is the practical effect.**
The budget is a claim about *rendered* size, so a smaller rendering means
a finer granularity fits. At `--rollup 100k` across all 164 repos:
135 repos fold to the same granularity, **29 fold finer, none fold
coarser**, and total nodes emitted go 109,558 → 149,413 — **+36.4% more
of the graph retained for the same budget**. `scala-play` goes from
`package` (11 nodes) to `module` (2,058); `graphql-github` from 3 nodes
to the full 1,625-node graph.

Test suite: 778 tests, unchanged in runtime. The pre-commit hook is
unaffected; its `cgg` invocations now write ~24% less to `target/`.

## [0.8.1] - 2026-08-22

**Output-side only. The analysis is untouched.** No resolver phase, no
plugin, no framework rule and no id derivation changed, and the graph
this release builds for a given tree is the same graph 0.8.0 built —
verified by running both binaries over one fixed tree: `mermaid`, `dot`
and `graphml` come back byte-identical. Everything below is about what
gets *emitted* from that graph and how it can be sliced afterwards.

### Added — `--rollup`, `--rollup-by`, `--from-graph`

- **`--rollup BUDGET` folds the graph to a coarser granularity when the
  rendered output would exceed a token budget.** The default graph of a
  real tree does not fit in a context window — this repo's own `crates/`
  is ~2,200 callables and ~5,200 edges, 123,000 tokens of mermaid —
  and the view usually wanted at that size is "which module calls which",
  not "which function calls which". Budgets accept `100k`, `120000`,
  `1.5m`. A graph already under budget is left **byte-identical**, so the
  flag is safe to leave in a wrapper script.

- **`--rollup-by LEVEL` names the granularity outright:** `callable`,
  `type`, `module`, `file`, `package`, `dir:N`, `language`. With
  `--rollup` it is a floor the budget may coarsen past. `package` is the
  nearest ancestor directory holding a build manifest (`Cargo.toml`,
  `package.json`, `go.mod`, `pyproject.toml` and friends) — the only
  level that reads the filesystem, falling back to `dir:1` where it finds
  none. Measured on `cgg ./crates`: 2196 nodes at `callable`, 365 at
  `type`, 241 at `module`, 141 at `file`, 26 at `package`, 21 at
  `language`.

  Rollup is a `Graph` -> `Graph` transform in the pipeline, not a
  formatter mode, so all four output formats and all four front ends get
  it from one implementation. It runs last — after `--filter`/`-n`, after
  `--exclude-*`, after dead-code marking — and composes with all of them.

- **`--from-graph FILE` re-queries a graph saved by an earlier `-t json`
  run** instead of walking source. `-t json` already wrote a document
  `serde` could read back; this is the reader plus the guardrails one
  needs. `--filter`, `-n`, `--exclude-*` and `--rollup` all apply to the
  loaded graph and produce byte-identical output to the same flags run
  against the tree.

- `-t json` now carries `"schema": "cgg.graph.v1"` and the writing cgg
  version, as two wrapper keys. Node ids are explicitly not comparable
  across versions, so an unlabelled or mismatched document is a warning
  and a different schema is an error rather than a silent misread.

- New `weight` field on an edge: how many call sites it stands for.
  Always `1` in an ordinary graph, and only a fold raises it. `mermaid`
  and `dot` fold it into their existing `Nx` label; `graphml` gains a
  `weight` edge attribute, whose `<key>` is declared only when some edge
  uses it; JSON skips the field entirely when it is `1`.

  **Default output is unchanged.** Verified by running this build and a
  pristine 0.8.0 build over the same fixed tree: `mermaid`, `dot` and
  `graphml` are byte-identical. `-t json` differs by exactly the two new
  wrapper keys (`schema`, `cgg_version`) plus the per-file parse timings
  that already differ between any two runs.

- New `CallableKind::Group` and a `rollup` field on a callable, carrying
  the member/file/language/internal-call counts a group node stands for.
  A distinct kind rather than borrowing `Function`, because a consumer
  filtering `kind == "method"` must not be handed a directory — and it
  can only appear in a graph the caller explicitly asked to roll up.

- New audit events: `rolled_up` (the level chosen, the budget, every
  granularity measured and rejected, and whether the budget was met) and
  `graph_replayed`.

- **All four front ends carry the whole surface.** The CLI, `cgg-py`
  (`rollup=`, `rollup_by=`, `rollup_format=`, `from_graph=`, plus
  `Callable.rollup` and `Edge.weight`), `cgg-node` (the same in
  camelCase, with `RollupInfo` and regenerated `index.d.ts`), and
  `cgg-ffi` — which needed **no ABI change at all**: options cross as a
  JSON document precisely so a new feature does not add an entry point,
  and that held. Verified from C, Python and JavaScript against the same
  trees the CLI was run on.

  `cgg-node`'s graph-type conversions now destructure with no `..` rest,
  the guard `cgg-py` already used. Without it the option keywords reached
  the pipeline while `Callable.rollup` and `Edge.weight` did not, so
  JavaScript could ask for a fold and be unable to observe it, and a
  folded edge's call count was dropped at the boundary. That is a build
  error now.

### Honesty properties

- **A rollup is never silent.** It is announced on stderr — and unlike
  every other advisory, **`-q` does not suppress it**, because it is the
  only thing distinguishing a graph of your code from a graph of your
  directory layout. It is also stated in a comment header inside the
  mermaid itself, so it survives copy-paste of the block, and recorded in
  the audit.
- **A budget that cannot be met says so** rather than returning something
  over it quietly. If no granularity renders smaller than the un-rolled
  graph — which happens on trees small enough for the banner and the
  per-node member tags to dominate — the un-rolled graph is returned,
  because a "budgeted" artifact larger than the unbudgeted one is
  strictly worse than not having the flag.
- **Aggregation follows the logic, not convenience.** A group edge's
  confidence is the **maximum** over the edges it folds ("at least one
  call exists" is a disjunction — as strong as its best evidence);
  `unreferenced` is the **minimum** and is set only when *every* member
  carries it ("nothing calls anything in here" is a conjunction, which
  one referenced member falsifies).
- **`--from-graph` refuses what it cannot honour.** `--dead-code`,
  `--include-external`, `--dynamic-dispatch`, `--since` and `--lang` all
  need analysis-time facts a saved graph does not contain; each is
  rejected with the reason rather than silently producing an ordinary
  graph. A document that was itself filtered is detected (its metrics
  outnumber its contents) and warned about, because replaying it can only
  narrow it further.
- **`<framework-entry>` nodes fold by `(trust kind, framework)`,** not by
  path. They share one sentinel path, so a path-based level would
  collapse every entry in the tree into a single node — destroying the
  one thing an entry node exists to say. Folding on the framework keeps
  that and the trust boundary, and states the count
  (`⟨412 framework entries — INFERRED⟩`). A framework with a single entry
  passes through whole, because folding it would gain nothing and cost
  its route.

  Exempting them from rollup entirely was tried first and is wrong: it
  gives `--rollup` a floor it cannot get under. On `java-spring-batch`,
  10,639 callables fold to **two** language groups while **412** entry
  nodes pass through untouched and are 99% of the output — about 50,000
  tokens no granularity could reduce, so `--rollup 40k` failed at every
  rung and could only warn. Folding on the framework, that repo fits at
  `package` in 1,122 tokens.
- **The token count is an estimate, is documented as one, and is
  calibrated against a measurement.** No tokenizer ships in the binary —
  cgg is offline, deterministic and single-binary, and a BPE vocabulary
  is none of those at the size it would add. The count is
  `max(words x 2.5, bytes / 1.8)`.

  The divisor is measured, not a rule of thumb. `bytes / 3.5` — the usual
  figure for code — was tried first and under-counts cgg's own output by
  about half, which makes the budget not a bound at all: `--rollup 40k`
  returned 65-80k real tokens. Mermaid is far denser than prose because
  40% of it is base36 node ids and `::`-dense qualified names. Both
  columns below are measured, not projected:

  | target | real tokens | `bytes/3.5` | `bytes/1.8` |
  | --- | --- | --- | --- |
  | `cgg/crates` full | 123,639 | 76,496 (0.62x) | 148,781 (1.20x) |
  | `cgg/crates` type | 25,438 | 13,526 (0.53x) | 27,116 (1.07x) |
  | `cgg/crates` file | 14,648 | 7,823 (0.53x) | 15,527 (1.06x) |
  | `cgg/crates` package | 1,890 | 1,216 (0.64x) | 2,424 (1.28x) |
  | `ripgrep` full | 162,291 | 101,521 (0.63x) | 197,402 (1.22x) |
  | `ripgrep` type | 35,505 | 18,750 (0.53x) | 37,491 (1.06x) |
  | `redis` full | 789,069 | 429,660 (0.54x) | 835,451 (1.06x) |
  | `redis` file | 72,666 | 36,950 (0.51x) | 73,615 (1.01x) |

  1.8 is the bottom of the measured 1.78-2.25 range on purpose: the
  estimate should **bound** the real count, not split it. Erring high
  costs one extra rung of folding; erring low hands back an artifact over
  the budget, which is the failure the flag exists to prevent. Range is
  now 1.01-1.28x — never under.

  Calibrated against `o200k_base`, a proxy for token density rather than
  the tokenizer any particular model charges. A 2x error is much larger
  than the spread between BPE families, so the correction holds either
  way; the residual 10-25% slack is the tokenizer-specific part. README,
  `--help` and the skill state the measured range.

### Fixed

- Qualified-name splitting in the rollup keys is bracket-aware. A plain
  `rfind` lands inside a Rust trait-impl wrapper and produced the group
  name `cgg::cli::<cgg_format::OutputFormat` — a dangling bracket, and a
  type nobody wrote.
- `clippy::collapsible_match` in `cgg-lang/src/plugins/clojure.rs`, which
  failed `cargo clippy --workspace --all-targets -- -D warnings` on
  current clippy before any of this work. Unrelated to the feature;
  fixed because it blocked the gate.
- **`CallableNode` no longer carries `RollupMeta` inline.** The struct is
  held one-per-callable for a whole run, and the 64-byte field grew it
  208 -> 272 bytes: a 31% widening of the hottest structure in the
  pipeline, paid by every analysis to carry something that is `None`
  unless someone passed `--rollup`. A corpus-wide paired A/B put the cost
  at +2.9%. Boxed, the field costs 8 bytes and the struct is 216; the
  allocation is paid only by group nodes, which are by construction few,
  and `Box<T>` is serde-transparent so the wire format does not move.
  `CallEdge` was measured too and is unchanged at 88 bytes — `weight`
  landed in padding that already existed. `crates/cgg-core/tests/sizes.rs`
  pins both numbers, because the growth happened and nothing noticed
  until a timing run went looking.

### Performance

Paired A/B against the 0.8.0 baseline, both binaries built from this
machine's cache and run alternately over the same tree — `cgg ./crates`
(2,196 callables, 5,214 edges) at `--jobs 1`, 7 pairs, wall clock from
each run's own summary line:

| | median | min | max |
| --- | --- | --- | --- |
| 0.8.0 baseline | 973.5 ms | 952.0 | 1038.2 |
| 0.8.1 | 965.7 ms | 939.9 | 1028.7 |

Median delta **-0.8%**. Per-pair deltas run +5.1%, -0.9%, -1.6%, +2.0%,
+0.9%, -6.0%, -4.9% — a spread wider than the median difference in both
directions, which is noise, not a move. That is the expected result:
`apply_rollup` returns before rendering anything when neither flag is
set, so the default path gains one branch.

Cost of the new paths, same tree, wall clock per run:

| Run | Times |
| --- | ----- |
| default (no rollup) | 467 / 346 / 356 ms |
| `--rollup 40k` (folds at `type`) | 422 / 437 / 373 ms |
| `--rollup 1k` (walks every rung) | 364 / 374 / 435 ms |
| `--rollup-by module` | 410 / 438 ms |
| `--from-graph` + `--rollup-by module` | 108 / 63 / 74 ms |

Folding is one O(V+E) pass per rung and every render after the first is
of a dramatically smaller graph, so even a budget that walks the whole
ladder stays inside run-to-run variance.

`--from-graph` is the number that matters: ~70 ms against ~360 ms,
because it skips walking, parsing and resolving entirely.

#### Corpus A/B against 0.8.0

`scripts/perf-compare.sh` against `affee30`, median of 5 per repo, on a
64-core Linux host with the 164-repo corpus.

The first run — before the `CallableNode` boxing above — came back
**+2.9%** over 97 repos (142,965 ms -> 147,135 ms) before its 30-minute
budget stopped it. That is above the script's own ~1-1.5% noise floor and
was the signal that found the struct growth; nothing in the rollup pass
explains it, because `apply_rollup` returns before rendering when neither
flag is set.

After boxing, over 14 repos chosen for callable density (258,885
callables — the population the regression scales with, deliberately
excluding `erlang-otp` and `zig-zig`, which add ten minutes and no
signal):

| | 0.8.0 | 0.8.1 | delta |
| --- | --- | --- | --- |
| **total** | 33,745 ms | 33,618 ms | **-0.4%** |
| median per repo | | | **+1.05%** |

Per-repo range -2.1% to +6.6%, with sub-150ms repos flagged noisy by the
script. Both readings sit inside the noise floor, so this is **flat** —
not an improvement. Two caveats, stated rather than buried: `scala-spark`
is 24s of the 33.7s total, so the total is weighted heavily toward one
repo, which is why the median is quoted beside it; and this is a single
run, not the repeated pair the script asks for before *claiming* a
change. Claiming flat needs less evidence than claiming a win, which is
the direction this errs.

Timings are from an x86_64 host; the single-tree numbers above were taken
on aarch64. They are not comparable to each other and are not presented
as such.

## [0.8.0] - 2026-08-19

### Changed — BREAKING: node ids are content-derived, not sequential

- **`CallableId`/`FileId` are now stable, content-derived hashes instead
  of a per-run sequential counter.** Every emitted graph — mermaid,
  json, dot, graphml — previously numbered nodes `0, 1, 2, …` in
  file-discovery order, so the same source tree could mint a different
  id for the same function from one run to the next (a new file
  appearing earlier in the walk order shifts every id after it), and an
  id was meaningless outside the run that produced it. An id is now a
  blake3 hash of the node's own identity — a file's relative path, or a
  callable's `(language, file path, owner qualified name, qualified
  name, signature hint)` — so **the same node gets the
  same id on every run**, and adding, removing or editing an unrelated
  file never changes it.

  The signature is part of the key because the first four components are
  **not unique**. A file that declares several callables sharing a
  qualified name — overloads, which C++, C#, Java and Erlang have in
  quantity — hashes them identically. Over the 113-repo benchmark corpus
  that is **309,347 of 1,745,670 callables (17.7%)**, peaking at 52.8%
  in one repo, and every one of them falls through to the collision path
  where the id is decided by declaration order rather than content. That
  breaks the guarantee this change exists to provide: delete the first
  of two overloads and the second inherits its id, so a consumer diffing
  ids across runs reads "unchanged" while the id now names a different
  function. Including the signature takes that population to **2.25%** —
  the residual being callables cgg genuinely cannot tell apart, where no
  key can do better.

  **The path component is relative to the analysis root, not the path
  you typed.** `cgg-walk` reports each file under the root it was given,
  so hashing the raw path made an id a function of the invocation rather
  than of the code — the same file, same content, same name produced
  four different ids:

  ```text
  cgg /tmp/smoke2   Ck1v6lk3phz
  cgg smoke2        Cf83zjoounv
  cgg .             Cn10c7f0lc7
  cgg t.py          Cvgb78ftyly
  ```

  That defeated the point: two checkouts at different paths, or CI and a
  laptop, would agree on the graph and disagree on every id in it. With
  one root the root is now stripped entirely, so ids are
  location-independent; with several roots the root's directory name is
  kept as a prefix, because `src/m.py` and `tests/m.py` are different
  files and collapsing both to `m.py` would push them into the
  order-dependent redraw. Three regression tests cover it — nothing did
  before.

  `start_byte` was measured as an alternative and rejected. It is unique
  corpus-wide, but it churns on movement — one comment line added at the
  top of `spdlog`'s busiest header moved 135 of 1,157 ids, destroying
  the diffability this change exists for — and it still lets a survivor
  inherit a deleted sibling's id, because removing a definition shifts
  the next one into its byte offset.

  What holds: editing an unrelated file changes nothing, moving code
  within a file changes nothing (verified by running `cargo fmt --all`
  over cgg's own tree — 318 callables, 0 ids changed), and removing one
  overload leaves its siblings alone.

  A collision, if one ever occurs, is resolved by drawing the next
  window from the same node's own hash stream — never by comparing to
  whichever other node it collided with. Every draw is 52 bits wide, so
  **no id ever exceeds 52 bits**; that bound is what keeps `cgg-node`'s
  `number` binding exact.

  **Wire format changed too.** An id used to be a bare integer
  (`{"src": 0, "dst": 1}` in JSON; `C0`, `n0` in mermaid/dot/graphml).
  It is now the type's prefix character followed by lowercase base36
  digits (`{"src": "C4k2j9qh3xz"}`; `Chco17z7ulm`, `nhco17z7ulm`). This
  is unconditional — there is no flag and no fallback to the old
  numeric scheme, because only a handful of consumers depend on raw id
  values today and every one of them (the CLI's own formatters,
  `cgg-py`, `cgg-node`) was updated in this same change.

  **If you store or diff raw ids across runs, this changes what you see
  even though the graph is unchanged**: ids no longer renumber on every
  run, so a diff against a previous run's ids is now meaningful for the
  first time — but a diff against pre-upgrade output will show every id
  as new, once. `cgg-py`'s `Callable.id`/`Edge.src`/`Edge.dst`/
  `File.id` are `u64` now (Python's `int` already covers the range, so
  nothing on the Python side needed a type change beyond that).
  `cgg-node`'s equivalents are `i64` on the Rust side and `number` in
  the generated `index.d.ts` — N-API has no native `u64` binding, and
  `number` is exact here only because every id stays inside 52 bits,
  comfortably under `Number.MAX_SAFE_INTEGER` (2^53-1).
  Verified: across the corpus no `cgg-node` id is negative, unsafe, or
  disagrees with the value the CLI reports for the same callable.
  `cgg-node`'s `Graph.files` getter, which used to return
  `Array<string>` of bare paths indexed by `Callable.file`, now returns
  `Array<{ id, path }>` — matching by id was already how `cgg-py`
  worked, and the old "index equals id" invariant depended on the
  sequential scheme this change removes. `cgg-ffi`'s C ABI is
  unaffected: it never exposed raw ids, only rendered strings/JSON, by
  design.

### Performance

- Measured with `scripts/perf-compare.sh` against `072dbb0`, the commit
  0.7.0 shipped from plus one docs-only commit (`git diff --name-only
  ea9fbef 072dbb0` matches no `.rs`/`.toml`/`.lock` file, so it is
  perf-identical to the 0.7.0 release). Median of 7 runs per repo,
  alternating which binary goes first. Machine load at measurement time:
  **2.90, 3.17, 4.32** on a 64-thread host.

  The corpus budget stopped the sweep after **67 of 113 repos**
  (alphabetical prefix, through `erlang-otp`). That is a partial sweep
  and is stated as one rather than presented as full coverage.

  | comparison | median per-repo | total (excl. `erlang-otp`) |
  | --- | --- | --- |
  | 0.8.0 vs 0.7.0 | **+3.5%** | +2.9% |

  So content-derived ids cost roughly **3%** — a blake3 hash and a
  `HashSet` probe per callable replacing an integer increment. Against
  the harness's own stated noise floor of 1–1.5% on a total, the
  per-repo median is a real cost, not jitter. **The comparison is
  like-for-like**: this release enables no new work by default, and the
  resulting graph is byte-identical to 0.7.0's on all 113 corpus repos
  (nodes and edges by name, including `via` and confidence).

  Regressions over 5% on repos whose baseline exceeds 150 ms:
  `bash-acme` +12.9% (155→175 ms), `app-spring-mall` +10.6%
  (406→449 ms), `app-eshop-aspnet` +7.0%, `app-vaultwarden-rocket`
  +6.8%, `app-flaskbb-flask` +6.1%, `app-thingsboard-concurrent` +5.9%
  (5732→6072 ms), `elixir-phoenix` +5.5%, `app-druid-jaxrs` +5.4%. All
  are the same per-callable hashing cost; none is a repo-shape effect,
  and the move is corpus-wide rather than one repository — 51 of 67
  slower, 14 faster, 2 flat.

  **`erlang-otp` is excluded from the total and the exclusion is the
  point.** The same binary measured 26,651 ms in one sweep and 36,280 ms
  in another — a 36% spread with no code change between them. A direct
  alternating measurement (5 samples, median, default jobs) puts it at
  **26.9 s against 0.7.0's 28.1 s**, i.e. slightly faster, so the large
  positive deltas some sweeps report for it are machine contention, not
  a regression. It is 28% of the corpus total, so leaving it in swings
  the headline number by ten points in either direction. This is the
  trap 0.6.x fell into and CLAUDE.md records: quote the median, not a
  total one repository can dominate.

  Developer-facing latency, on the same host:

  | | timing |
  | --- | --- |
  | `cargo test --workspace` (warm) | 5.9 s / 6.1 s / 14.0 s cold |
  | `.githooks/pre-commit` end to end | 6.8 s / 6.8 s |

  Two changes in this release were made *because* they were measured.
  Keeping the mermaid/dot dedup keys `Copy` avoids two `String`
  allocations per edge plus two more per lookup. And the redundant
  `include_by_last` sort in `cross_file.rs` was dropped rather than
  re-keyed: sorting those buckets by `PathBuf` cost **+30% on
  `erlang-otp`** on its own, and the buckets are already built in
  discovery order, so the sort never did anything.

### Testing

- **The determinism sweep now covers the whole corpus, and runs in
  processes.** Three defaults made it both slower and weaker than it
  read. `--repos` defaulted to 14 of 113, so a green sweep described a
  random eighth of the corpus while sounding like it described all of
  it. `--runs` defaulted to 4, but determinism is a pairwise property —
  every run is compared against the first, so run 2 is the one that can
  detect a difference and later runs are only further chances at an
  intermittent one; with ~5.5k cells across the corpus, breadth samples
  that far better than depth per cell. And the sweep ran strictly
  serially.

  Parallelising it with threads changed nothing — 142s / 139s / 146s at
  4 / 8 / 16 workers over a fixed subset, with python at 102% CPU and
  zero cgg children. The cost is this script, not cgg: `json.loads` ->
  `strip_timings` -> `json.dumps` over multi-megabyte graphs, which is
  GIL-bound. Processes fixed it:

  | executor | 8w | 16w | 32w | 48w |
  | --- | --- | --- | --- | --- |
  | ThreadPool | 140s | 139s | 146s | — |
  | ProcessPool | 80s | 49s | 42s | 37s |

  ~3.8x at the new default of `cores/2`.

- **16 framework rules now have a real application behind them.**
  `APPS_UNVERIFIED` — rules cgg detects but that nothing proves it can
  *enumerate* — goes from 45 to 29, and 121 rules now enumerate entry
  points in a real application. The distinction matters: a rule that
  detects without enumerating leaves every handler of that framework at
  in-degree zero, which is not merely incomplete but wrong.

  The applications are projects *built on* each framework, not the
  framework's own repository. An earlier attempt used the framework
  repos and they do enumerate — but only because their own test suites
  exercise their own decorators, which is close to circular. `martini`
  made the point: `go-martini/martini` does not import itself, so its
  import-path detect never fired at all.

  Two rules stay unverified with their real reason recorded rather than
  boilerplate. `martini` is a **cgg gap**: `martini-contrib/render`
  registers 21 routes as `m.Get("/x", func(...){...})` and cgg records a
  value reference only for a bare identifier, so an inline closure
  handler binds to no callable. `symfony-messenger` has no application
  using the component.

  This also fixed three stale `~` gap markers and eight never-cloned
  cloud APPS entries, so `scripts/framework-coverage.py` exits 0 again.

### Compatibility / migration

- **The default graph is unchanged.** Verified across all 113 benchmark
  repositories: nodes and edges compare identical to 0.7.0 by name,
  including `via` and confidence. Only the *identifiers* changed.
- **Raw ids are not comparable across versions.** A diff of ids against
  pre-0.8.0 output shows every node as new, once. From 0.8.0 forward a
  diff against a previous run is meaningful for the first time.
- **JSON ids are strings, not numbers.** `{"src": 0}` is now
  `{"src": "C4k2j9qh3xz"}`, and the `callables`/`files` objects are
  keyed by the same string. jq expressions calling `tonumber` on an id
  will fail; drop the coercion.
- **`cgg-py`**: `Callable.id`, `Edge.src`, `Edge.dst` and `File.id` are
  `u64`. Python's `int` already covers the range, so no caller change is
  needed unless it assumed ids were small or sequential.
- **`cgg-node`**: the same fields are `number`, and `Graph.files` now
  returns `Array<{ id, path }>` instead of `Array<string>` indexed by
  `Callable.file` — that indexing only worked while ids were sequential.
  Match by id.
- **`cgg-ffi`**: unaffected. The C ABI never exposed raw ids, only
  rendered strings and JSON, which is why it needed no change.

  **`erlang-otp` is excluded from the totals above and the exclusion is
  the point.** The same binary measured 26,651 ms in one sweep and
  36,280 ms in another — a 36% spread with no code change between them.
  A direct alternating measurement (5 samples, median) puts it at
  **26.9 s against `main`'s 28.1 s**, i.e. slightly *faster*, so the
  large positive deltas that appear for it in some sweeps are machine
  contention rather than a regression. It is 28% of the corpus total, so
  leaving it in swings the headline number by ten points in either
  direction. This is the same trap CLAUDE.md records for 0.6.x: quote
  the median, not a total one repo can dominate.

  Two changes in this branch were made *because* they were measured, not
  because they looked cleaner. Keeping the mermaid/dot dedup keys `Copy`
  avoids two `String` allocations per edge plus two more per lookup. And
  the redundant `include_by_last` sort in `cross_file.rs` was dropped
  rather than re-keyed: sorting those buckets by `PathBuf` to restore
  discovery order cost **+30% on `erlang-otp`** on its own, and the
  buckets are already built in discovery order, so the sort was never
  doing anything.

## [0.7.0] - 2026-08-17

### Added

- **Cloud entry points across five platforms and sixteen languages.**
  Google Cloud Functions had no rule in any language; Azure, Cloudflare
  and Deno detected their framework and enumerated nothing from it.
  Ruby and PHP — *first-party managed runtimes* on Lambda and Cloud
  Functions, as officially supported as Python — had no cloud rule at
  all.

  | Platform | Runtimes |
  | --- | --- |
  | AWS Lambda | Go, Java, Python, JS, TS, C#, Rust, **Ruby, PHP, Kotlin, Scala, Groovy, Swift, C++** |
  | AWS CDK | Python, TS — binds `handler="app.lambda_handler"` across files |
  | Google Cloud Functions | Python, JS, TS, Go, Java, C#, **Ruby, PHP, Kotlin, Scala, Groovy** |
  | Azure Functions | C#, Java, Python, JS, TS, **F#, PowerShell, Kotlin, Scala, Groovy** |
  | Firebase | JS, TS, Python |
  | Cloudflare Workers | JS, TS, **Rust** |
  | Deno | JS, TS |

  Verified on real repositories, all now APPS entries:
  `functions-framework-nodejs`, `firebase/functions-samples` (17 Python
  and 2 JS entries), both Azure quickstarts, `cloudflare/workers-rs`,
  `denoland/std`, `powertools-lambda-python` (1,282 entries),
  `aws-cdk-examples`.

- **`--fanout-cap`, `--no-graph`, `--report-unreferenced`**, an
  `unresolved_by_module` audit event, and five specific
  `UnresolvedReason` variants replacing the blanket
  `no-candidate-in-file`.

- **The span profiler is compiled into every build.** It was
  `#[cfg]`-compiled out of release, so the one build anyone runs could
  not answer "where is this dwelling?" — exactly the question a
  pathological input raises, and one a debug build cannot answer because
  it distorts the ratios. `--profile` now enables collection at runtime.
  Four quadratics were found with it within the hour.

### Fixed

> **Four superlinear resolver paths made large inputs unusable.**
> `erlang-otp` ran for **3 hours 40 minutes** without finishing;
> `cmake-kitware`, `dart-flutter` and `zig-zig` timed out. Any run that
> did complete produced a correct graph — no published number was wrong
> — but four repositories in the benchmark corpus could not be analysed
> at all, and the corpus scripts had no timeout, so a sweep containing
> one of them never returned.

- **`enclosing_callable_id` scanned every callable in the graph, per
  reference.** On Zig's compiler that is 572,840 references against
  344,808 callables. The reference loop was 449s of CPU while the
  resolution inside it was 1.8s. Indexed by `(file, start, end)`.
- **The `#include` closure had no memo**, so a C include diamond was
  re-walked exponentially — 25 includes at depth 8. Fixed with a *depth
  map*, not a visited set: `depth` counts down, so a plain set would
  refuse to re-expand a header first reached by a long path and lose the
  deeper definitions, order-dependently.
- **Include resolution scanned every file** per include per file;
  `HashMap` iteration is unordered, so even the exact-match
  short-circuit read half the map and a miss read all of it.
- **The FFI duplicate check scanned the whole edge list** per candidate
  per reference.

- **Multi-argument generics were dropped from every base-type rule.**
  `implements RequestHandler<String, String>` was truncated by a
  `trim_end_matches` that could not tell a generic's `>` from the
  container's delimiter. Not Lambda-specific: **MediatR** went from
  `entries NOT enumerated` to enumerating.
- **Kotlin, Scala and Groovy recorded no base types**, so every
  `base_types` rule for them was inert — including `android-worker`,
  `spring-batch` and `gradle-plugin`, which had never fired. F#, Scala
  and Groovy recorded no attributes.
- **JS/TS options objects were invisible to registrar capture**, which
  is how Azure's v4 model writes every handler.

### Changed

- **Corpus scripts are time-bounded.** `CGG_REPO_TIMEOUT` (60s/repo) and
  `CGG_TOTAL_BUDGET` (1800s/run) now bound `benchmark.sh`,
  `perf-compare.sh` and `framework-coverage.py` — whose per-repo cap was
  900s, which across a hundred repos is not a timeout. A repo that trips
  the cap is named and excluded, never silently averaged in.

- **`perf-compare.sh` now measures the whole corpus.** It measured nine
  hand-picked repos totalling **1.8 seconds** — 0.7% of the corpus's
  258s — and it is the same nine that produced the "+4–6.8% corpus-wide
  regression" this project retracted in 0.6.2. It contained none of the
  repos where this release's quadratics lived, so it would have reported
  *flat* for a fix that took `erlang-otp` from 3h40m to 24.5s.

- **A fifth optimisation was reverted.** An inverted
  `(language, path fragment)` index took `hcl-terraform-aws` to 16.6s;
  the full-corpus sweep caught it losing **9,331 nodes and edges across
  34 repositories**. The match is `path.contains(fragment)`, which
  permits a fragment to begin mid-segment, and no segment-aligned index
  reproduces that. `hcl` is 49s instead of 17s as a direct result.

### Performance

Machine load at measurement: see below. Comparison is **not**
like-for-like in one direction — this release enables cloud rules for
nine more languages that the baseline did not have, so some of the added
edges are new default work.

`scripts/perf-compare.sh` against `v0.6.7`, 9-repo smoke set, `--jobs 1`:

| repo | 0.6.7 | 0.7.0 | delta |
| --- | --- | --- | --- |
| rust-ripgrep | 193 ms | 185 ms | -4.1% |
| python-flask | 84 ms | 89 ms | +6.0% (noise) |
| js-express | 66 ms | 72 ms | +9.1% (noise) |
| go-fzf | 166 ms | 158 ms | -4.8% |
| c-jq | 99 ms | 94 ms | -5.1% (noise) |
| cpp-spdlog | 266 ms | 253 ms | -4.9% |
| csharp-serilog | 94 ms | 98 ms | +4.3% (noise) |
| swift-alamofire | 193 ms | 194 ms | +0.5% |
| cpp-nlohmann-json | 628 ms | 611 ms | -2.7% |
| **TOTAL** | **1789 ms** | **1754 ms** | **-2.0%** |

The script's own noise floor on that total is ~1–1.5%, so **-2.0% is
flat**, not an improvement. Every per-repo move flagged `(noise)` has a
baseline under 150 ms.

**The corpus is the real number, and it is not comparable as a total**,
because 0.6.7 could not finish four of its repositories:

| repo | 0.6.7 | 0.7.0 |
| --- | --- | --- |
| erlang-otp | 3h 40m, never finished | **24.5s** |
| cmake-kitware | timed out | **4.4s** |
| dart-flutter | timed out | **15.3s** |
| zig-zig | timed out | **48.2s** |
| hcl-terraform-aws | 62s | 49.2s |

Whole corpus, 113 repositories, 185,222 files, 1,745,670 callables,
7.7 GB, `--jobs 8`, one pass each: **258s**, with **89 repos under one
second**, 6 between 10s and 60s, and **none over two minutes**.

Graph size against a clean `v0.6.7` build, all 113 repositories:
**nodes 1,000,768 → 1,745,670 (+74.4%)**, **edges 1,904,932 → 2,382,130
(+25.1%)**, with **zero repositories losing a single node or edge**.
Most of that gain is the four repositories that previously produced no
graph at all; excluding them, 49 repositories gain from the new rules
and the capture fixes, led by `app-rails-mastodon` (+897 nodes),
`js-express` (+552) and `app-saleor-celery` (+427 edges).

Test suite: **690 tests in 5.1s**. Gates (test + clippy + fmt +
docs-check) total **6.7s**.

### Compatibility

The default graph remains a superset of 0.6.7's: no repository in the
corpus lost a node or an edge. Two behaviour changes are worth naming:
scoping the registrar-verb gate per language moves JS/TS dead-code
findings (-69 on Ghost, +6 on cal.com), and inline handlers that carry
no route string now register, which adds synthesized handler nodes
(+1.4% on Ghost).

### Added / Fixed / Changed — cloud entry points (filed late)

These three subsections describe work that shipped **in 0.7.0** but
was left under `## [Unreleased]` when that release was cut, so it sat
unattributed until the 0.8.0 bump found it. Moved here rather than
rolled into 0.8.0, which would have claimed already-released features
as new. The summary bullet and language table above cover the same
work; these add the per-platform detail.

### Added — cloud entry points

- **Google Cloud Functions, Azure Functions, Firebase, Cloudflare
  Workers and Deno** — 18 enumerating rules across five platforms.
  Google Cloud Functions had **no rule in any language**; Azure,
  Cloudflare and Deno detected their framework and enumerated nothing
  from it.

  | Platform | Runtimes | Mechanism |
  | --- | --- | --- |
  | Google Cloud Functions | Python, JS, TS, Go, Java, C# | Functions Framework decorators, `functions.http('name', h)`, and the `HttpFunction`/`IHttpFunction` contracts |
  | Azure Functions | C#, Java, Python, JS, TS | `[Function]`/`[FunctionName]`/`@FunctionName`, the v2 Python decorators, v4's `app.http('name', { handler })` |
  | Firebase Functions | JS, TS, Python | v2 trigger registrars, `@https_fn.on_request` |
  | Cloudflare Workers | JS, TS | module-worker `fetch`/`scheduled`/`queue`/`email`, plus legacy `addEventListener('fetch', …)` |
  | Deno | JS, TS | `Deno.serve` handlers, default-exported `fetch` |

  Verified on real repositories, each now an APPS entry:
  `functions-framework-nodejs` (3 entries), `firebase/functions-samples`
  (17 Python + 2 JS), both Azure quickstarts (2 each),
  `cloudflare/workers-rs` (3), `denoland/std` (1).

### Fixed — cloud entry points

- **A registration needed a route string, and three platforms had
  none.** `is_registration_shape` required a leading string literal
  before it would name an inline handler, so `Deno.serve((req) => …)`,
  Firebase's `onRequest((req, res) => …)` and Express middleware
  `app.use(fn)` enumerated nothing.

  The string was never what made that gate safe — the caller's
  *registrar-verb* gate is, and `describe`, `it`, `map`, `then` and
  `setTimeout` are registrar verbs in no rule. Measured before removing
  it: **+1.4% nodes on Ghost, +0.1% on cal.com**, and wall clock
  unchanged to slightly faster.

- **Inline closures inside an options object were invisible.** Azure
  Functions' v4 model writes every handler as
  `app.http("name", { handler: async (req, ctx) => … })`, and only
  argument position was scanned — the whole runtime enumerated nothing.
  `javascript.rs` also carried its own copy of the closure scan rather
  than calling the shared helper, so fixing the helper alone was not
  enough.

- **Cloudflare's detection missed `@cloudflare/workers-types`**, the
  import a TypeScript Worker actually uses, so the commonest Worker in
  existence was not detected at all.

### Changed — cloud entry points

- **Firebase's deprecated v1 vocabulary is deliberately not
  registered.** `onCreate`/`onUpdate`/`onDelete`/`onWrite`/`onRun`
  collide with ORM lifecycle hooks and event emitters across the whole
  language, and verb gating happens at extraction time — carrying them
  cost a measured **5.3% on Ghost and 1.9% on Immich** for a deprecated
  API. Bisected by stripping rule groups one at a time; Azure's verbs,
  by contrast, cost -0.2%. A named v1 handler still binds by value
  reference; only its entry label is lost.

  Net effect of the whole change set, nine paired runs at `--jobs 1`
  against 0.6.7: **-3.1% median, -2.0% min** on Ghost.

## [0.6.7] - 2026-08-14

**CI smoke test. No functional change** — the graph 0.6.7 produces is
byte-identical to 0.6.6's.

0.6.6's `publish` job failed after all fourteen build jobs passed: the
root npm package's `prepublishOnly` re-runs `napi prepublish`, and
`napi` is not on PATH in that step, so `npm publish` exited 127. Five
platform packages shipped at 0.6.6 while the root stayed at 0.6.5 — the
split-brain the post-publish verification check exists to catch, and it
did catch it. The root package was then published by hand and the
workflow fixed to pass `--ignore-scripts`.

That fix cannot be exercised except by tagging, so this release exists
to tag.

**Result: the fix works, and the smoke test earned its keep by finding a
second bug.** `npm publish --ignore-scripts` succeeded
(`+ cgg-callgraphgenerator@0.6.7`) and all three channels shipped — PyPI
five wheels and an sdist, npm root plus five platform packages, three
GitHub release binaries, none of it by hand.

The `publish` job still went red, on the verification check itself: it
read `npm view` **one second** after the publish and got 0.6.6, because
`dist-tags` propagation through the registry CDN takes a few seconds.
A false alarm on the one check whose value is being trustworthy — the
next real split-brain would be waved through as "just the propagation
thing". It now polls for up to five minutes instead of reading once.

## [0.6.6] - 2026-08-14

### Added

- **The three enhancements from the same field report.**

  **Duck-typed fan-out is narrowed by what a candidate can accept.** A
  call passing `data=`/`context=` fanned out to four same-named
  `evaluate` methods, three of which accept neither keyword and require
  four others. Keyword names are now captured at the call site and
  checked against each candidate's `signature_hint` — which the
  extractors already recorded, so no new extraction. Measured on the
  reported shape: **4 candidates → 1**.

  Deliberately one-sided. A candidate is eliminated only when a keyword
  is *provably* not one of its parameters and it has no `**kwargs`; an
  unparseable signature accepts anything, and if narrowing would empty
  the set the original fan-out stands. Narrowing must not become a way
  to lose real edges.

  **`--report-unreferenced`** lists callables nothing points at, in
  place of the graph. Not `--dead-code`: it asks a strictly weaker
  question — "does anything point at this?" — and the weakness is the
  feature. Reachability cascades, so one unrooted framework handler
  drags its whole subtree into the report; a reference check cannot,
  because it never looks past one edge. Entries cgg already treats as
  roots are bucketed separately rather than hidden. The case that
  prompted it: two classes documented as the serialization contract
  between two pipeline stages, imported by nothing, found by grep.

  **Unresolved calls are grouped by external module**, largest first,
  in the audit (`unresolved_by_module`) with a one-line stderr summary.
  On one package in the report, 74 unresolved calls mapped almost
  exactly onto a dependency the reader had no access to, and the tally
  quantified the evidence gap — after they grouped it by hand. On
  Netflix's dispatch it reports 27,974 unresolved across 353 modules,
  led by sqlalchemy (3,726) and alembic (1,334).

  **Cost.** Paired medians of five runs at `--jobs 1` against 0.6.5:
  photoprism -0.6%, dispatch +0.7%, ghost +1.8%, netbox +2.0%, flaskbb
  +3.3%. Two rounds of profiling took the grouping from +4.7% to +1.8%
  on the worst case — a quarter-million unresolved calls were each
  scanning their file's whole import list, then each allocating a key —
  by indexing imports once per file and borrowing the keys.

- **Twelve resolver and reporting fixes from a field report** — a
  call-graph audit of a ~200-file Python service (AWS Lambda +
  Powertools + Pydantic). Every one was reproduced against 0.6.5 before
  being fixed, and two reported issues turned out not to need work:
  `--dead-code-report` with `--dead-code-format json` already wrote JSON
  (the repro re-read a stale file), and the Powertools dead-code cascade
  was closed by this release's Lambda rules.

  **Wrong edges** — worse than missing ones, because they read as facts:

  - `super().m()` resolved to the *calling class's own* `m` when the base
    was outside the analyzed tree. Combined with the real forward edge
    this formed a phantom cycle, so a reader tracing control flow
    concluded infinite recursion. `super()` now excludes the calling
    class, and reports `super-base-out-of-graph` when nothing remains.
  - A bare `helper(2)` resolved to a same-file `Holder.helper` **at
    `high`**, outranking the correct `from lib import helper` target at
    `medium`. A bare identifier cannot reach a method in Python, Rust,
    Go, JS/TS or PHP — Ruby, Java and C# are excluded, where it can — and
    an unambiguous import binding is now `high`, because it is not a
    guess.

  **Silently dropped calls**:

  - Duck-typed fan-out above 5 candidates emitted no edges *and no
    record*: indistinguishable from "there is no call here". One method
    with 24 call sites by grep showed 2 inbound edges with no signal that
    22 were dropped. Now recorded as `fanout-cap-exceeded` with the
    candidate count, and the cap is `--fanout-cap`.

  **Resolution that never existed**:

  - **Class instantiation now links to the constructor.** 107
    constructors in the audited service had zero inbound edges out of
    1206 — "who constructs X?" was unanswerable for every Python class.
  - **Inherited methods resolve through the base chain.** `w.apply()`
    produced no edge when `apply` came from a base while `w.extra()` on
    the same receiver resolved, because nothing walked the MRO.
  - **Calling an instance resolves to `__call__`** (`__invoke`, `call`).
    In the audited service this was the single most load-bearing edge in
    the system — the boundary where application code hands control to a
    model — and it was invisible.

  Measured on the benchmark corpus: **+12% edges on netbox, +15% on
  flaskbb**, for no wall-clock cost (medians of five paired runs at
  `--jobs 1` against 0.6.5: dispatch -0.7%, photoprism +0.0%, flaskbb
  +1.6%, netbox +2.0%).

  **Reasons that were factually wrong.** `no-candidate-in-file` was
  reported for names cgg had parsed and indexed — in one case with nine
  candidates — which reads as "this name does not exist". Five specific
  reasons replace it: `fanout-cap-exceeded`, `candidates-in-other-files`
  (with a count), `not-in-scope-for-bare-call`,
  `class-without-explicit-init` and `super-base-out-of-graph`. Where two
  passes record the same site, the specific reason wins.

  **Reporting**:

  - `metrics` in `-t json` was **all zeros** while the graph was fully
    populated. `confidence_histogram` and `unresolved_calls` are exactly
    what a programmatic consumer reads to gauge trust, and zeros suggest
    a clean, fully-resolved graph.
  - `--dead-code-format json` with nowhere to write **exited 0** after
    discarding the report — a silent-failure trap for scripted use. It
    now fails, and `--no-graph` gives the report stdout.
  - `--why-live` rendered definition-side liveness and a real call path
    with the same `LIVE` label. A function only ever *named* in a
    module-level registry now reads `LIVE (root itself — no call path)`.
  - **Pydantic validators are roots.** `@model_validator` /
    `@field_validator` methods are invoked by Pydantic, so a validator
    that provably runs on every construction was reported as dead — and
    dead code cascades, taking everything it calls with it.
  - **`Protocol` / `ABC` stubs are no longer counted as call targets.**
    A body-less declaration was returned as a third implementation
    alongside two real ones, inflating any count of write paths.

- **AWS Lambda entry points, across all six runtimes.** Lambda was the
  largest hole in framework coverage and the one that mattered most:
  nothing in a handler's own file calls it, so before this every Lambda
  codebase reported its entire handler module as dead. Six rules existed
  and five of them enumerated nothing — they only disclosed the gap.

  | runtime | mechanism | shape |
  | --- | --- | --- |
  | Go | `lambda.Start` and its four variants | B |
  | Java | `handleRequest` on `RequestHandler`/`RequestStreamHandler` | D |
  | Python | `lambda_handler` convention; Powertools routes and `process_partial_response` | A + B |
  | JS / TS | `handler`/`lambdaHandler`, middy, Powertools decorators | A + B |
  | C# | `[LambdaFunction]`, `FunctionHandler` | A |
  | Rust | `service_fn` (already worked) | B |

  Verified against real repositories rather than fixtures alone:
  `powertools-lambda-python` yields 1,282 Powertools entries and 338
  `lambda_handler` entries; `aws-cdk-examples` exercises the TypeScript,
  C#, Java and Go rules at once. All three are now APPS entries.

- **CDK stacks are read as source**, which is the part no convention can
  replace. `_lambda.Function(self, "Api", handler="app.lambda_handler")`
  binds to `lambda_handler` in `app.py` — across files, and whether or
  not the handler's own module imports anything AWS-related. That case
  previously produced no entry by any route.

  This needed two pieces. `decode_string_target` learned the
  `module.function` form every AWS runtime uses (narrowly: the directory
  prefix is stripped only when what remains still carries a dot, so
  `"application/json"` cannot become a claim on a callable named `json`,
  and a filename extension is rejected outright). And the name index
  gained a **file-stem** lookup, because `orders.processOrder` names a
  module, not a type — Python happens to qualify callables that way, but
  JavaScript, TypeScript and Go do not qualify by file at all.

- **`scripts/security-check.sh`** — a pre-publish gate that runs before
  `release.sh`. Eight checks: trufflehog over the working tree *and* git
  history, a direct byte-search for this machine's real tokens in both,
  no credential-shaped files in the repo, `.env` gitignored,
  `cargo-deny` + `npm audit`, no workflow step that could print a secret,
  what `cargo package` and `npm pack` would actually ship, and the
  permissions on local token files.

  Each check was verified to fire by planting the thing it looks for.
  That caught a flaw in the first version: **`--results=verified,unknown`
  finds nothing.** trufflehog only marks a finding `verified` when it can
  authenticate against the live API, so a real credential that has since
  been rotated reports as `unverified` — and three randomly generated
  AWS / GitHub / npm credentials planted in the tree went completely
  undetected. Fixed to `verified,unknown,unverified`, then re-verified.

  The `Lob` detector is excluded because it matches `test_<alnum>`, which
  is every pytest function name in `crates/cgg-py/tests`, and Lob's test
  endpoint accepts any such string — so it reports twelve function names
  as *verified secrets*.

### Changed

- **The registrar-verb gate is now scoped to the file's language.** It
  was the union of every rule in the table, so adding Go's `Start` made
  every Ruby, PHP and Python file pay an argument scan for its own
  `start` calls — a measured +5.6% on `homebox-chi` and +3.6% on
  `photoprism` from the Lambda rules alone. A verb can only ever match a
  rule of its own language, so narrowing loses nothing at match time.

  Paired A/B against 0.6.5 at `--jobs 1`, medians of five warmed runs:
  `homebox-chi` **-3.9%**, `photoprism` **-2.5%**, `netbox` **-1.7%**,
  `ghost` **-1.1%**, `calcom` +0.2% — so the feature lands net faster
  than 0.6.5 rather than merely paying for itself.

  **This changes output on JavaScript and TypeScript trees**, and the
  effect is disclosed rather than buried: string references captured
  under a *foreign* language's verb no longer suppress dead-code
  findings, and inline closures at those call sites are no longer minted
  as `handler_at_N` nodes. Net dead-code findings move by -69 on `ghost`
  and +6 on `calcom`. Go and Python output is byte-identical.

- `release.sh` now gates on the formatters it was missing:
  **`ruff format --check`** (it ran `ruff check` only, and nine files had
  drifted out of format while reporting clean), **markdownlint over all
  22 tracked docs** rather than just README and CHANGELOG, `node --check`
  on the hand-written JavaScript, and a TOML parse of every manifest.
  Extending ruff to `crates/cgg-py/tests/` surfaced 13 real lint errors,
  now fixed — including two `subprocess.run` calls without an explicit
  `check`, both of which genuinely expect a nonzero exit.

  `CLAUDE.md` disables MD013 in-file, with the reason: it is agent-facing
  prose written as long unwrapped paragraphs on purpose. Every other rule
  still applies to it.

### Fixed

- **Multi-argument generics were dropped from every base-type rule.**
  `implements RequestHandler<String, String>` was truncated to
  `RequestHandler<String, String` by a `trim_end_matches` that could not
  tell a generic's `>` from the container's own delimiter, and
  `is_type_name` then rejected it for containing a space. Any rule keyed
  on a multi-argument generic interface silently never fired.

  Not a Lambda-specific bug, and it is how it was found: Java is the one
  runtime where the entry point is a declared contract, so its rule
  should have been the easiest of the six and produced nothing. Measured
  against 0.6.5 on a `IRequestHandler<GetUser, UserDto>` fixture,
  **MediatR** goes from `entries NOT enumerated` to `1 entry`.

- **JS/TS options objects were invisible to registrar capture.**
  `collect_value_refs` descended into `pair` but never into the `object`
  holding the pairs, so every options-bag registration was skipped —
  which is how CDK, and much of the JS ecosystem, passes a handler.

- **Docs claimed a distribution state that three releases had overtaken.**
  All four registries have been on 0.6.5 since the tag, but the prose had
  not caught up, and in two places it steered users away from a working
  install:

  - `README.md` announced "**It is not on npm yet** — `cgg-callgraphgenerator`
    404s on the registry as of 0.6.3" under a heading reading *Node — built,
    not yet published*, and told readers to build from a clone. The package
    has been on npm since v0.6.4. Replaced with the `npm install` line, a
    runnable snippet, and the `optionalDependencies` layout — verified in a
    container: `added 2 packages`, the root plus only `linux-x64-gnu`, and
    the snippet renders a graph.
  - `crates/cgg-py/README.md` and `INSTALL.md` both said the only prebuilt
    wheel was `manylinux_2_17_x86_64` and that macOS, Windows and aarch64
    Linux fall back to a multi-minute source build. Five wheels have
    shipped since 0.6.4. Corrected to the real matrix, with musl Linux and
    Windows-on-arm64 named as what actually reaches the sdist — verified by
    a clean `pip install`, which downloads the 10.1 MB wheel and builds
    nothing.

  `INSTALL.md` also pinned its download URL and `git clone --branch` to
  v0.6.4, and `PUBLISHING.md` still described crates.io as "live at 0.6.3",
  PyPI as one-wheel-per-release, and every GitHub release as `assets=0`.

  None of this is checked by `docs-check.py`: its eleven checks cover
  counts, flags and CHANGELOG structure, but nothing compares a prose claim
  about a registry against that registry.

- **The npm publish step could go green without publishing.**
  `napi prepublish` ships only the per-platform packages; the root
  package — the one users install — needs its own `npm publish`, which
  the workflow never ran. On v0.6.5 that produced five platform packages
  at 0.6.5, a root package still at 0.6.4, and a green job. Caught by
  installing from the registry in a container rather than trusting the
  job status. The workflow now publishes the root and then asserts that
  `npm view` reports the version it just built, failing loudly if not.

  `prepublish` also tries to attach the `.node` files to a GitHub release
  that the workflow already created, which 400s. Those errors are logged
  and swallowed, so a green job proves nothing on its own — hence the
  explicit check.

### Removed

- **`test_results/`** — 63 files, last touched three months before this
  release, regenerated by nothing and linked from no document. It was an
  archive of the original task-by-task build-out (`task01` … `task12`),
  and it had gone from stale to wrong: `task06-stackgraphs-crossfile`
  documents a working "stack-graphs integration", while
  `stack_graphs_resolver.rs` has been a 46-line no-op stub since the
  tree-sitter 0.26 upgrade and CLAUDE.md tells agents not to assume it
  resolves anything. It also cited cgg 0.1.0 and tree-sitter 0.23/0.24.

  cgg's primary reader is a coding agent scanning the repo, so a
  contradicted claim in a plausible-looking directory is worse than no
  directory. It shipped in no artifact — zero files in any crate package
  or the sdist — so nothing users install changes. `git show
  v0.6.5:test_results/...` still has it.

## [0.6.5] - 2026-08-12

> ## ⚠ Node IDs and node order change in this release
>
> **cgg produced a different graph on different machines for the same
> commit, and 0.6.5 fixes that.** The walker returned files in filesystem
> enumeration order, which is not stable across machines, and node ids are
> positional — so `C0`, `C1`, … were assigned differently depending on
> whose disk the files came off.
>
> **If you have committed cgg output, expect a one-time diff** when you
> regenerate it on 0.6.5: node ids are renumbered and nodes are declared
> in a new (sorted) order. **Edges, callables and every semantic property
> are unchanged** — the same graph, written down in a stable order.
>
> After this release, the same commit gives the same bytes on any machine.
> That is what "deterministic" and "diffable in a PR" were supposed to
> mean, and until now they only held for one machine at a time.

### Fixed

- **The graph depended on filesystem enumeration order.** `cgg-walk` used
  `WalkBuilder::build()` with no sort, so it yielded readdir order — which
  differs between machines for byte-identical trees. Node ids are
  positional, so the same commit produced different `C0`/`C1`/… ids and a
  different declaration order on each machine, making the mermaid output
  non-diffable across a team and undercutting both "deterministic" and
  "diffable in a PR".

  Found by installing 0.6.4 through all five distribution channels in five
  containers and diffing the output of one fixture: **three distinct
  graphs**. The enumeration orders were genuinely different
  (`lib.rs app.py mod.go app.js` / `app.js mod.go app.py lib.rs` /
  `lib.rs mod.go app.py app.js`) and the graphs tracked them exactly.

  The `--jobs` determinism test could not catch this: it varies thread
  count against one directory on one host, where readdir order is a
  constant. Fixed with `sort_by_file_path`, covered by a new test that
  asserts sorted order directly, and verified by re-running the same
  binary in all three containers — one identical graph.

  **This changes node ids and node order for any tree the filesystem did
  not already enumerate in sorted order.** Edges and semantics are
  unaffected.

- The release tarballs shipped `libcgg.d` (a make-style dep-info file) and
  `libcgg.rlib` (the Rust lib target's archive, unusable from C), because
  the packaging step globbed `libcgg.*`. Now copies explicit extensions.

### Added

- `INSTALL.md` — the minimum packages each distribution channel actually
  needs, established by installing into bare `ubuntu:24.04` containers and
  adding only what each channel refused to work without. Notable findings:
  Ubuntu's Rust (1.75) cannot build cgg (needs 1.85), a C compiler is
  required even though cgg is a Rust program, and `python3-venv` is
  mandatory on 24.04 because the system Python is PEP 668 managed.

## [0.6.4] - 2026-08-12

Documentation correctness pass. No behaviour change; the code fixes below
are to comments that ship inside published artifacts.

### Fixed

- **PyPI 0.6.2 and 0.6.3 shipped without an sdist**, so `pip install` on
  macOS, Windows and aarch64 Linux failed outright with "no matching
  distribution" — there was no source fallback to build from.
  `scripts/publish-python.sh` only ever built the wheel. It now builds,
  `twine check`s and uploads an sdist alongside every wheel, and the
  missing 0.6.3 sdist was uploaded (verified by forcing a source install
  from PyPI with `--no-binary :all:`).

- **`crates/cgg-node` doc comments were wrong, and they ship to npm.**
  `index.d.ts` is generated from them, so the types users read in their
  editor claimed `toJson()` was "byte-identical to `cgg -t json`" (it
  embeds per-run `parse_ms`/`wall_ms`; mermaid, dot and graphml *are*
  byte-identical) and that the analysis runs on "libuv's thread pool"
  (it is `spawn_blocking` on tokio's, via napi's `tokio_rt`). Both
  corrected at the source and the stubs regenerated.

- **The C example in `crates/cgg-ffi/README.md` did not compile** —
  missing `<stdio.h>`, so `NULL`, `stderr`, `fputs` and `puts` were all
  undeclared. It now builds clean under `-Wall -Wextra`.

- **`CHANGELOG.md` had lost the whole `[0.6.2]` entry** while 0.6.2 was
  live on two registries: an edit adding 0.6.3 replaced the header
  instead of inserting before it, so 0.6.2's content sat under the
  0.6.3 heading. Restored, and `docs-check` check 10 now fails on a
  skipped patch version.

### Documentation

Every factual claim in every markdown file was re-verified against a
command rather than accepted. The corrections that mattered most:

- **A pipeline stage that does not exist.** README documented
  "Stack-graphs resolution" as step 3 of the resolution pipeline;
  descriptor linking was missing entirely.
- **Verification commands that silently prove the wrong thing.** The
  `push` and `docs-sync` skills told an agent to check plugin counts
  with `rg -c 'plugin\.register'` (returns nothing — the calls are
  `reg.register(`, so it "proves" zero plugins), to find
  `trait_impl_target` in `main.rs` (it is in `lib.rs`, so it "proves"
  dynamic dispatch was removed), and used `rg -E` as extended-regex
  (it is `--encoding`, and errors). A verification procedure whose
  commands lie is worse than none.
- **Self-contradictions in `cgg-frameworks`**: bucket A pointed at
  `root_attributes`, which is not a `FrameworkRule` field; "`detect` is
  the gate" was disproven in both directions by live test; "route paths
  are not captured" (they are — the gap is the HTTP verb); "there is no
  `base = …` matcher" while the same doc documented `base_types`.
- **Counts and measurements**: 178 → **211** dependency packages; six →
  **seven** C ABI functions; seven → **eight** trust-boundary kinds
  (`public` was missing); 40 → **44** grammars; `--jobs 32` is 1.4× on
  pandoc, not "roughly twice as fast"; NetBox is ~3.0 s, not 6.8 s; and
  the Python concurrency and speedup figures were re-measured.
- **Unprovable claims deleted**, notably "the highest-confidence band is
  roughly 20-45% precise", which has no provenance anywhere in the repo
  and cannot be established without manually reviewing every finding.

`docs-check` gains check 10 (CHANGELOG integrity) and its docstring's
stale "Seven checks" is now eleven.

## [0.6.3] - 2026-08-12

> Published to crates.io and PyPI. **Not tagged**, and not on npm — the
> npm packages publish from a `v0.6.3` tag, which has not been pushed.

### Added

- **Node.js bindings**, `crates/cgg-node`. The package builds on all five
  platforms in CI but is **not published to npm yet** — `npm install
  cgg-callgraphgenerator` does not work until a `v*` tag runs the publish
  job. Build it locally with `napi build --platform --release`.

  ```js
  const cgg = require("cgg-callgraphgenerator");
  const g = await cgg.analyze("./src");
  console.log(g.toMermaid());
  ```

  `crates/cgg-node` is an N-API module over `cgg::analyze`, a translation
  layer with no analysis logic — the same contract `cgg-py` and `cgg-ffi`
  hold. `analyze` is async and runs on libuv's pool so a server does not
  stall its event loop; `analyzeSync` is there for scripts. TypeScript
  definitions are generated from the Rust, so they cannot drift.

  Shapes mirror the Python module field for field — `Edge` is
  `src`/`dst`/`siteLine`/`siteByte`/`confidence`/`via` — because two
  bindings over one pipeline should not describe the same graph with
  different words. Same single rename from the CLI: `entryNodes: true`.

  15 tests, including byte-identical parity with the binary across all
  four formats and seven option combinations, with a guard that fails if
  no option changed the graph — otherwise parity could pass while the
  module ignored every keyword it was given.

  **Native, not a wrapper over the C ABI.** npm needs a per-platform
  artifact either way, so the C ABI would buy nothing here while costing
  an FFI dependency and a slower boundary.

  **Packaged the way npm does native modules**: one package per platform,
  listed as `optionalDependencies`, so npm installs only the one matching
  the host. That needs no install script, which matters more than it
  looks — `npm ci --ignore-scripts` is common in CI and in
  security-conscious orgs, and a source-build `postinstall` silently
  produces a broken install under it. A source package was considered and
  rejected for that reason: it also needs a Rust toolchain, ~53 s of
  compilation, and leaves 779 MB of build artifacts. npm has no `sdist`
  equivalent to fall back on the way PyPI does.

  Naming: `cgg` is taken on npm (an unrelated ChampionGG API wrapper), so
  the package matches PyPI rather than inventing a third spelling.

- `.github/workflows/release.yml` gains a five-target Node matrix, and the
  publish job now pushes to npm as well as PyPI.

## [0.6.2] - 2026-08-11

> Published to crates.io and PyPI. Not tagged.

### Fixed

- **The PyPI page told people to `pip install cgg` — the wrong project.**
  0.6.1's wheel embedded a stale README: it was edited while the maturin
  build was already running, so maturin had read the previous text. The
  repository was correct the whole time; only the published artifact was
  wrong. PyPI metadata cannot be edited after upload, which is why this
  costs a version rather than a re-upload.

  `scripts/publish-python.sh` now compares the wheel's embedded
  description against `crates/cgg-py/README.md` and refuses to upload on
  a mismatch — catching both that race and a plainly forgotten rebuild.
  Verified to fire on the 0.6.1 wheel.

### Performance — the "0.6.0 regression" was one atypical repository

**There is no corpus-wide regression.** Paired A/B over 29 repositories,
both binaries run back to back within each trial:

| | |
| --- | --- |
| median per-repo delta | **+0.0%** |
| total wall across all 29 | +2.7% |
| within ±5% | 18 of 29 |
| **faster** | **10 of 29** |
| worse than +10% | 2 |

Only `c-jq` regresses in every paired trial. Its magnitude is unstable
between sessions (+23% to +45%), and `cpp-nlohmann-json`, which looked
like a second case at +9.5% in one run, came back +0.0% in another — it
was noise.

**Where the earlier "+4-6.8%" came from.** `scripts/perf-compare.sh` uses
a fixed 9-repo set that contains `c-jq`, so one anomalous repository
carried a ninth of the weight of the headline number. Two earlier entries
in this file reported that as a real, corpus-wide regression. It was not.

**It is not the C grammar.** No tree-sitter crate changed between v0.5.0
and now — 44 of them, all identical versions; the 0.26 upgrade predates
0.5.0 and is inside both binaries. And `c-redis` is 13 MB of C across 950
files and is unaffected (−1.6%).

**What makes `c-jq` different is its shape, not its language:**

| repo | analyzed files | biggest file | top 8 files |
| --- | --- | --- | --- |
| c-jq | 69 | 22% of bytes | 63% |
| c-redis | 950 | 5.1% | 20.7% |
| cpp-spdlog | 157 | 14.2% | 48.3% |

69 work items across 8 workers, one of them a fifth of the total work, so
wall time is the critical path of a single file. That is long-tail load
imbalance, and it amplifies any change in how work is scheduled. The
0.6.x per-call pool does schedule slightly differently — `install` runs
the driver on a pool worker where 0.5.0 drove from the main thread — but
on any tree without that skew the difference does not survive the noise.

**Consequence: this is not worth optimising the pipeline for.** The thing
to fix, if anything, is the load imbalance itself — splitting or ordering
large files so one item cannot dominate a phase — which would help every
skewed repository rather than reverting a change that exists to make
`--jobs` work on the second call.

Recorded because it was diagnosed wrongly twice: first blamed on the
`ExtractCtx` threading, then correctly narrowed to the pool but wrongly
reported as corpus-wide. Both earlier claims came from measuring one
repository, or a repo set weighted towards it.

## [0.6.1] - 2026-08-11

**First release published to crates.io.** `cargo install cgg` works, and
`cgg` is usable as a library dependency. 0.6.0 shipped only as a git tag.

### Packaging

- **Published to PyPI as `cgg-callgraphgenerator`** — `pip install
  cgg-callgraphgenerator`, then `import cgg`. PyPI's `cgg` belongs to an
  unrelated GGUF tool with 49 releases, so the short name was never
  available; the long form spells out what CGG stands for. That package
  also ships a top-level `cgg` module, so **the two must not share an
  environment** — both write to `site-packages/cgg/` and pip will not stop
  you. Documented in both READMEs.

  The wheel is `manylinux_2_17` + `abi3-cp39`: built in maturin's CentOS 7
  container, because PyPI rejects plain `linux_x86_64` wheels and a wheel
  built on a modern box links a glibc newer than most users have. One
  wheel covers every CPython from 3.9 up. 10.1 MB.
  `scripts/publish-python.sh` does the build, the clean-venv test and the
  upload.

- All six library/binary crates now carry the metadata a registry listing
  needs — `repository`, `homepage`, `keywords`, `categories`,
  `rust-version` — inherited from the workspace so the listing cannot
  drift from it, plus a per-crate `README.md`. The root README lives
  outside every package directory, so without one each crates.io page
  would have rendered blank.
- The five internal library crates say so on their own front page: they
  are published only so `cgg` can depend on them by version, and their
  APIs change freely between minors to serve that one consumer.
- `cgg-py` and `cgg-ffi` stay `publish = false`. The artifact anyone
  wants from those is a wheel and a shared library, not a crate.

### Added

- **`crates/cgg-ffi` — a C ABI.** One shared library (`libcgg.so`) or
  static archive (`libcgg.a`) serves C, .NET, Java, Go, Ruby and anything
  else with an FFI, so a binding is source rather than another native
  artifact per language per platform.

  Seven functions. Options cross as a JSON document and results as strings,
  which is the load-bearing decision: adding a cgg flag adds a JSON key,
  not an entry point, so **the ABI does not change when cgg gains a
  feature** and no wrapper needs rebuilding. It is affordable because
  rendering is nearly free next to analysis — 6.5 ms for `to_json()` and
  1.3 ms for `to_mermaid()` against 137.7 ms of analysis on cgg's own
  tree. `cgg_analyze` returns an opaque handle rather than a rendered
  string, so mermaid *and* JSON *and* the metrics cost one analysis.

  Output is byte-identical to the CLI across all four formats, asserted by
  test. Statically linked, a C program depends on nothing but libc and
  libgcc — the CLI's self-contained promise survives the boundary.

  Unknown option keys are an error rather than ignored: C callers
  hand-write the JSON, so a silently-dropped `"hopz"` is the failure this
  boundary is most exposed to.

- **The Rust library needs only one dependency now.** `cgg` re-exports
  `OutputFormat`, `Result`/`Error` and the graph types, so a consumer no
  longer has to name `cgg-format` and `anyhow` in its own manifest just to
  call the API — which also stopped their versions from drifting from the
  ones cgg was built against.

- `RunOptions` derives `Serialize`/`Deserialize`, with `#[serde(default,
  deny_unknown_fields)]`, which is what the JSON options boundary is built
  on.

### Fixed

- **`cgg::analyze` no longer leaks memory per call.** 0.6.0 leaked ~161
  bytes every time it was called. `cgg_resolve::type_hints` had a
  `leak_str` helper that `Box::leak`ed a copy of every parameter name and
  type, with a comment calling that "acceptable because we're in a
  short-lived analysis pass" — true while the pipeline was private to a
  binary that analyzed once and exited, and false the moment 0.6.0 made it
  a library, a Python module and a C ABI callable in a loop. A host
  process analyzing on a loop grew without bound.

  The leak existed only to dodge a borrow conflict: the strings are slices
  of `def.signature_hint`, and `facts` is mutably borrowed later in the
  same function. They now go into an owned `Vec` that the function drops.
  **The allocation count is unchanged** — the leaked version called
  `to_string()` too — so this costs nothing; the strings are simply freed.

  Verified under valgrind: 0 bytes definitely lost on `cgg-walk`,
  `cgg-format`, a single 467-line file and all of `./crates`, where 0.6.0
  lost 161 bytes in 28 blocks. Graph output is byte-identical on
  `./crates`, `c-jq`, `go-fzf` and `python-flask`, and still deterministic
  across `--jobs` 1–32.

  Diagnosis note, because it is the reason 0.6.0 shipped with this:
  **`mimalloc` hides leaks from valgrind entirely.** With the global
  allocator in place valgrind reported no leak summary at all; the 161
  bytes only appeared once it was removed. The CLI was never affected — it
  analyzes once and exits.

- `docs-check.py` check 9: `Box::leak`, `.leak()` and `mem::forget` in the
  pipeline crates must be listed in `ALLOWED_LEAKS` with a reason. A leak
  justified by "the process is about to exit" has to be re-checked when
  that stops being true, and nothing was re-checking. The one remaining
  entry is `profile.rs`, which is bounded by `&'static str` span literals
  and compiled out of release builds.

### Performance

Flat. `scripts/perf-compare.sh` against the previous commit, median of 7
over the standard 9-repo set: **2258 ms → 2260 ms, +0.1%** — far inside
the ~1–1.5% noise floor. Expected: the leak fix changes where the strings
are freed, not how many are allocated.

The "+4–6.8% regression" reported against 0.5.0 was later re-measured
across 29 repositories and is **not corpus-wide** — median per-repo delta
is +0.0% and 10 of 29 are faster. See the 0.6.2 entry.

## [0.6.0] - 2026-08-11

The pipeline is a library, and there is a Python module on top of it.

```python
import cgg
g = cgg.analyze("./src")
print(g.to_mermaid())
```

The 1,035-line `run()` inside `main.rs` was private to a bin-only crate,
so nothing outside the binary could invoke it. `crates/cgg` now has a
`[lib]` target alongside its `[[bin]]`, and `crates/cgg-py` is a PyO3
extension module over it. Both front ends call `cgg::analyze`, so the
resolver ordering that CLAUDE.md calls load-bearing exists in exactly one
place.

**The CLI is unchanged.** Graph output is byte-identical to 0.5.0 across
all four formats, and so is the stdout/stderr interleaving, the exit
codes, and every advisory's `-q` gating. The binary links no libpython
and grew 48 KB.

### Added

- `cgg::analyze(&RunOptions) -> RunOutcome`, performing no I/O beyond
  reading source — no writes, no stdout/stderr, no `process::exit`.
  Everything a run writes comes back as an ordered `Vec<Emission>`.
- `crates/cgg-py`: `import cgg`. Every option that changes the graph is a
  keyword argument, with one rename — `entry_nodes=True` rather than
  `--no-entry-nodes`, same default. Four renderers, `.callables`,
  `.edges`, `.files`, `.metrics`, `.notices`, `.jobs`, `to_dict()`,
  `callable()`, `callers_of()`, `callees_of()`. `abi3-py39`, so one wheel
  per platform serves every CPython >= 3.9. Build with
  `scripts/build-python.sh`.
- `docs-check.py` checks 7 and 8: the self-analysis showcase filter must
  agree across every file that names it and must still produce a graph
  spanning three or more crates, and every `RunOptions` field must be
  reachable from `cgg-py` or listed as deliberately deferred.

### Fixed

Three bugs that a binary running one analysis per process could not
reach, and a second caller can:

- **`--jobs` was ignored after the first analysis in a process.**
  `build_global()`'s error was dropped with `let _ =`, and the global
  pool can only be set once, so later calls silently reused the first
  call's thread count. Now a per-call pool entered with
  `ThreadPool::install`, with `RunOutcome::jobs` reporting
  `rayon::current_num_threads()` read inside it.
- **One project's framework rules suppressed the next project's.**
  `set_extra_registrar_verbs` wrote to a `OnceLock` where only the first
  call took effect, so the second project analyzed in a process could
  lose its entry points. The switch now travels in a per-run
  `cgg_lang::ExtractCtx`; `DEADCODE_SIGNALS`, `EXTRA_REGISTRAR_VERBS` and
  `HAS_EXTRA_VERBS` are gone.
- **`--write-roots` called `std::process::exit(0)` inside the pipeline.**
  Harmless in a binary; linked into CPython it would terminate the
  interpreter with no traceback. It returns the baseline now.

### Performance

**cgg is roughly 4–5% slower on the standard 9-repo comparison set.**
Measured with `scripts/perf-compare.sh` against 0.5.0, median of 7:

| repo | 0.5.0 | unreleased | delta |
| --- | --- | --- | --- |
| rust-ripgrep | 236 ms | 238 ms | +0.8% |
| python-flask | 129 ms | 130 ms | +0.8% |
| js-express | 83 ms | 83 ms | +0.0% |
| go-fzf | 219 ms | 226 ms | +3.2% |
| c-jq | 145 ms | 182 ms | +25.5% |
| cpp-spdlog | 308 ms | 308 ms | +0.0% |
| csharp-serilog | 115 ms | 120 ms | +4.3% |
| swift-alamofire | 238 ms | 242 ms | +1.7% |
| cpp-nlohmann-json | 724 ms | 756 ms | +4.4% |
| **TOTAL** | **2197 ms** | **2285 ms** | **+4.0%** |

Three runs of the full set gave totals of **+4.0%, +5.5% and +6.8%**, with
`c-jq` at +25.5%, +21.5% and +25.9%. The direction is reproducible and
well above the ~1–1.5% noise floor, so this is a real regression, not
jitter. The spread tracks machine load, which was 1.0–2.8 across the
three runs — `perf-compare.sh` warns that a loaded machine invalidates
the comparison, so treat +4% as the floor and re-run on an idle box
before tagging.

The graph is byte-identical to 0.5.0 on every repo in the table, so the
cost buys nothing in output — it is the price of the correctness fixes
above. `parse` CPU rises ~7% while wall time rises more, so the loss is mostly
parallel efficiency rather than extra work.

> **This entry is wrong twice over.** It blamed the `ExtractCtx`
> threading, and it reported a corpus-wide regression. Neither holds: a
> 29-repository paired comparison puts the median per-repo delta at
> +0.0%, with 10 repositories faster, and the only consistently slower
> one is `c-jq` — whose file-size skew makes it uniquely sensitive to
> scheduling. The table below is a 9-repo set containing it. See the
> 0.6.2 entry.

**Recovering this is open follow-up work.** It is a deliberate trade for
now: the globals it replaced made a second analysis in one process return
wrong answers, and correctness at 4% is a better default than speed that
silently lies.

### Compatibility

**No change to the CLI.** Every flag, every output format, every exit
code and the stdout/stderr interleaving are what 0.5.0 produced, verified
byte-for-byte on `./crates` and on the 9-repo comparison corpus. The
default graph does not grow: this release adds no resolver and no rule.

**`cgg-lang` has a breaking API change.** `LanguagePlugin::extract` takes
`&ExtractCtx` as its first argument, and `set_deadcode_signals`,
`deadcode_signals`, `set_extra_registrar_verbs` and `is_registrar_verb`
are gone from the crate root — `ExtractCtx::is_registrar_verb` replaces
the last. All 44 in-tree plugins are updated. An out-of-tree plugin must
add the parameter; `ExtractCtx::plain()` is the no-switches context that
plugin tests use.

**The Rust library API is new, and pre-1.0.** `cgg::analyze`,
`RunOptions`, `RunOutcome`, `Emission` and `cgg::emit` are public as of
this release and may change in any 0.x minor — `RunOptions` in
particular gains a field whenever a graph-affecting flag is added, and it
is deliberately *not* `#[non_exhaustive]`, because `From<&Cli>`
destructures it with no `..` rest so that a new flag fails to compile
until it is routed. Pin an exact minor if you depend on it.

## [0.5.0] - 2026-08-07

Two releases in one. The framework rule table went from 51 rules to 394
across 36 languages, and then the tool was taught to use the machine it
runs on. **Whole-corpus latency fell from 3,202s to 205s.**

### Performance

0.4.2 → 0.5.0, measured on the **shipped default worker count** — which
is what you actually get, not a tuned best case:

| repo | 0.4.2 | 0.5.0 default | 0.5.0 `--jobs 32` |
| --- | --- | --- | --- |
| app-druid-jaxrs | 143.7 s | **18.6 s** (7.7×) | 8.0 s (18×) |
| c-redis | 41.4 s | **7.9 s** (5.2×) | 2.6 s (15.7×) |
| rust-ripgrep | 0.53 s | **0.27 s** (2.0×) | 0.40 s |
| app-django-netbox | 4.07 s | **3.58 s** | 3.74 s |
| python-flask | 0.18 s | **0.13 s** | 0.16 s |

**Two numbers, on purpose.** The default is deliberately conservative —
half the physical cores, capped at 8 — so cgg is a good guest on a shared
host. On a large tree more workers genuinely help, and `--jobs 32`
roughly doubles the default again. Small repositories are *faster* at
the default, where thread-spawn cost dominates.

The corpus-wide figure below was measured before that cap existed, at one
worker per logical CPU. It is the ceiling the parallelism reaches, not
what an untuned run produces:

**Whole corpus: 3,202 s → 205 s (−93.6%)** over 103 repositories, 100 of
them comparable. 73 of 100 repositories are more than 5% faster. **None
is more than 10% slower in absolute terms** — the 9 that show a
percentage regression are
all sub-200ms runs where a few milliseconds of process startup dominates.

**`zig-zig` could not be analysed at all before this release.** It
exceeded a 1,800-second timeout on 0.4.2 and now completes in 498s,
producing 344,807 callables. That is a capability change, not a speed
one, and it is why the raw corpus node and finding totals jump: excluding
that one repository, nodes move +8,107 and dead-code findings move
**−9,146**. `dart-flutter` and `erlang-otp` still exceed 1,800s on both
releases and are excluded from every total here.

Standard 9-repo comparison set:

| repo | latency | nodes | edges | entry | dead |
| --- | --- | --- | --- | --- | --- |
| rust-ripgrep | 399→249 ms | 2,906 | 7,106 | 0 | 1,429→1,420 |
| python-flask | 158→118 ms | 1,732→1,736 | 1,744→1,752 | 444→452 | 174→131 |
| js-express | 102→75 ms | 546→548 | 393 | 0 | 65→66 |
| go-fzf | 263→229 ms | 1,615 | 10,291 | 0 | 131 |
| c-jq | 123→130 ms | 1,119 | 21,724 | 0 | 425→424 |
| cpp-spdlog | 350→315 ms | 1,357 | 11,412 | 0 | 809 |
| csharp-serilog | 118→121 ms | 1,689 | 1,864 | 0 | 663→658 |
| swift-alamofire | 414→313 ms | 2,538 | 6,737 | 0 | 946→826 |
| cpp-nlohmann-json | 911→788 ms | 5,567 | 7,075 | 0 | 2,109→2,105 |
| **TOTAL** | **2,839→2,339 ms** | **19,069→19,075** | **68,346→68,354** | **444→452** | **6,751→6,570** |

Reproduce with `scripts/compare-release.py OLD_BIN NEW_BIN`. Note that
`--jobs 1` is the setting for numbers being published; the default runs
repositories concurrently, which is sound for an A/B delta but not for an
absolute latency claim.

### Parallelism

cgg parallelised exactly one phase before this release — the per-file
parse loop — and everything after it ran on one core. On Druid that was
110% CPU on a 64-core machine for 150 seconds.

Five phases now run in parallel, each verified to produce a
**byte-identical graph** at any thread count:

- **Cross-file resolution.** The single biggest win, and the reason Druid
  went from 150s to 8s. A 637-line per-file loop that read shared indexes
  and wrote only its own output.
- **Intra-file linking and type propagation.** Per-file and independent.
- **Framework matching**, fanned out across rules once the per-language
  indexes are hoisted out of the loop.
- **Audit serialisation** — 569ms of a Druid run spent on one core
  serialising JSON. Output is byte-identical to `to_writer_pretty`.

**The allocator was the ceiling.** Thread scaling stopped paying after
four cores, and the profiler showed why: the *same work* cost 6.8s of CPU
at `--jobs 4` and 10.6s at `--jobs 64` — 56% more CPU to produce
identical output. That is the system allocator serialising under
extraction's load, which allocates a `String` per name, per reference,
per qualified path, on every worker at once. **cgg now uses `mimalloc`
as its global allocator**; on its own that took Druid from 138s to 106s.

Two ordinary inefficiencies turned up in the same pass and are worth
naming because neither needed parallelism to fix: `known_refs` was
rebuilt **once per file** from identical data (1,273 files × ~10,000
names on netbox), and each file's audit record was located by linear
scan, making that step O(files²).

### Added: `--profile`

`RunMetrics::phases` records four coarse buckets. That stopped being
enough once "link" grew a type propagator, a cross-file resolver, an FFI
linker, a descriptor linker, a framework engine with six matchers and a
dead-code pass — a 25% regression inside that bucket is invisible from
outside it.

`--profile` prints a per-span breakdown. It is **compiled out of release
builds**: `span()` is `#[cfg]`-reduced to a constant `None`, so there is
no atomic load, no clock read and no branch to argue about. The numbers
this project publishes are measured on release binaries, and a profiler
that *could* perturb them is a profiler that makes those numbers
arguable. Debug builds collect by default. For release-speed numbers with
attribution, build with `RUSTFLAGS="-C debug-assertions=on"`.

Spans accumulate into per-thread-cached atomics rather than a locked map.
The first version used a global mutex and reported 263% CPU on a run the
plain binary did at 212% — it was measuring the contention it caused.

### Framework coverage: 45 → 343 frameworks, 12 → 36 languages

The rule table went from 51 to 394 `(id, language)` rules.

**The failure this fixes is silence.** An unrecognised framework
previously produced *no coverage line at all* — a real aiohttp app with
two routes reported `recognised (none)` and `seen, no rules (none)`, so
the disclosure pointed the reader at an empty list. Most of the new rules
are deliberately **detect-only**: they enumerate nothing and carry a
`gap` string naming the concrete construct to inspect by hand. A rule
that enumerates badly is worse than one that declines and says why.

Six detection gaps that were defects are closed, each traced to a
specific real-world idiom:

| framework | what was missed |
| --- | --- |
| `actix-actor` | `rust.rs` never populated `base_types`, so no base-type rule could fire on Rust at all |
| `chi` | handlers wrapped in `chain.ToHandlerFunc(...)` |
| `sinatra` | `get "/x" do … end` — Ruby hangs the block off the call's `block` field, never the argument list |
| `worker-threads` | worker modules that identify *themselves* rather than being named at a literal spawn path |
| `spring-messaging` | Spring AMQP puts `@RabbitListener` on the class and `@RabbitHandler` on the method |
| `nestjs-schedule` | nothing — the rule was correct; the corpus app scheduled via `SchedulerRegistry` |

New trust boundaries: **`TrustKind::Public`** for Solidity, where the
language *is* the framework — every `public`/`external` function is
callable by any address on the chain. 1,495 entries on OpenZeppelin, and
dead-code false positives there fell 37%. **`TrustKind::Ffi`** is now
produced, for `#[no_mangle]`/PyO3/wasm-bindgen exports.

**Descriptor → implementation linking** (`Via::Descriptor`). cgg parses
`.proto` and the languages that implement it, so it can now link
`service Greeter { rpc SayHello }` to the Go method serving it — an edge
neither file references. `.proto` rpcs became callables to make this
possible. The match requires the implementing type to *name the service*:
method name alone matches `Get` everywhere, and a missing edge is a gap
while a wrong edge is a lie about where control goes.

### Fixed: two nondeterminism defects, one of them shipped in 0.4.2

> **Read this section even if you skip the rest.** cgg's central claim is
> that the same input yields the same graph. Two code paths broke that,
> and **one of them is in the released 0.4.2 binary**. Neither produced a
> wrong edge *set* — only a varying edge or node *order* — which is
> precisely why they survived: a spot check passes, and only a byte-diff
> of two runs shows it.

- **`--dead-code` produced a different graph on every run, on any
  codebase with traits.** `--dead-code` force-enables
  `--dynamic-dispatch`, and `dispatch::fanout` iterated a `HashMap` to
  emit its declaration→implementation edges. Rust's `RandomState`
  reseeds per process, so the fan-out edges came out in a different order
  each time. Four runs over cgg's own source produced four different
  graphs. **This is present in 0.4.2 and every release that had
  `--dynamic-dispatch`.** If you diffed dead-code output between runs and
  saw churn, this was why. Fixed by sorting the keys.
- **`-n 0 --max-paths N` produced a different graph on every run when the
  cap truncated.** Entry points were walked in `HashMap` order, so *which*
  paths survived the cap varied. Node counts swung 7–9 across four runs of
  the same command. Without truncation the result was identical either
  way, which is why it hid. Fixed by sorting the entry set.

Both are now covered by `crates/cgg/tests/determinism.rs`, and both tests
were verified to *fail* against the unfixed code — a regression test that
has never seen the bug it guards is a guess.

**The determinism test that shipped alongside the parallelism work did
not catch either of these**, because its fixture had no trait with
multiple implementations and never triggered path truncation. That is the
more useful lesson than the bugs themselves: a green determinism suite is
only evidence for the shapes it actually exercises.

### Fixed: four plugin bugs that silently disabled whole languages

> **Read this section.** Each of these made a language's framework
> detection impossible, with no error and no warning — the coverage table
> simply reported nothing and looked correct doing it.

- **Lua recorded every `require` as the literal string `"("`.**
  `arguments.child(0)` returns the `(` token; it needed `named_child(0)`.
  Kong now detects across 744 files where it previously detected none,
  and Lua cross-file resolution gains real edges.
- **C and C++ ignored system includes entirely.** Only quoted includes
  were recorded, so `#include <gtest/gtest.h>` produced nothing and no
  C/C++ rule keyed on a system header could fire. System includes are now
  recorded under a distinct `system-include` kind, so the cross-file
  resolver still correctly ignores them.
- **Erlang recorded only `-import(...)`.** `-behaviour(gen_server)` — the
  only way Erlang declares an OTP contract — was invisible.
- **`FrameworkRule::has_matchers()` omitted `methods`.** Any methods-only
  rule (the structural-typing escape hatch that exists precisely for Go
  interfaces and Elixir OTP behaviours) filed itself under "seen, no
  rules" no matter how many entries it minted — a rule reporting a gap it
  did not have. Elixir went 0 → 359 entries on a real Phoenix app.

Registrar capture was added to the Elixir, Perl and Clojure plugins,
which had none: Phoenix's router now enumerates 151 routes on Plausible,
and Mojolicious 283 on its own tree. Lua was assessed and deliberately
**not** changed — its two rules are declared-gap rules, so capture there
would have had no consumer.

### Added: verification that the table cannot rot

- **`tests/detect_prefixes.rs`** synthesises, for every rule, a file
  importing that rule's own first `detect` prefix, and asserts the rule
  fires. A rule whose prefix does not match how the language actually
  writes that import is worse than no rule, because the coverage table
  then implies the framework was considered and found absent. This test
  found the Lua, C/C++ and Erlang bugs above.
- **`tests/determinism.rs`** asserts the graph is identical at 1, 2, 8
  and 32 threads. It compares *structure*, not bytes: the JSON and audit
  documents embed per-run timings, so a naive hash comparison reports
  nondeterminism that is not there.
- **`scripts/sync-app-manifest.py`** derives the corpus manifest from
  measurement rather than by hand, and **`APPS_UNVERIFIED`** in
  `benchmark.sh` states out loud which rules no real application
  exercises, with a reason each.

### Changed

#### Dependency disclosure: `mimalloc`

**cgg takes a new dependency in this release** — the first since
`b2afced` removed the update check and every network dependency. Stated
plainly because a dependency added for speed is still a dependency:

| | |
| --- | --- |
| crate | `mimalloc` 0.1.52 → `libmimalloc-sys` 0.1.49 |
| licence | **MIT** (both), already on `deny.toml`'s permissive allow-list |
| what it is | Microsoft's general-purpose allocator, set as cgg's `#[global_allocator]` |
| ships C | yes — `libmimalloc-sys` bundles mimalloc's C source and compiles it at build time via `cc` |
| new transitive deps | none. `cc` 1.2.62 was already in `Cargo.lock` |
| generation | **mimalloc v3.3.2** — upstream's recommended line, not the v2 "stable" line. Selected by leaving the `v2` feature off; every number above was measured on v3. Take `features = ["v2"]` for the conservative choice |
| features | none. `override` is **off**, which matters: mimalloc serves only Rust's `Global`, so the 44 tree-sitter C parsers that handle untrusted input keep glibc's hardened allocator |

**It does not change cgg's build requirements.** 45 crates in the graph
already required `cc` — every tree-sitter grammar, plus
`crates/cgg-lang/build.rs` compiling a vendored `parser.c` for the Smithy
grammar — and `skills/cgg-install/SKILL.md` already documents the C
toolchain check. A from-source install without a C compiler was already
broken before 0.5.0. Cost measured: +4.3s on a clean release build, and a
416 KB static library. Nothing changes at runtime; cgg still makes no
network requests.

**One advisory exists and does not apply.** RUSTSEC-2022-0094
(`unsound`, bad alignment) is patched in mimalloc >= 0.1.39; the pin is
0.1.52. `cargo deny check` passes all four checks — advisories, bans,
licenses, sources.

What it buys: thread scaling stopped paying past four cores because the
system allocator serialised under extraction's allocation load. mimalloc
alone took Druid from 138s to 106s, and is a substantial part of the
overall 15× speedup.

If you would rather not ship it, removing the `#[global_allocator]`
attribute in `crates/cgg/src/main.rs` and the two `Cargo.toml` entries
restores the previous allocator; everything else in this release stands
without it.

#### Other changes

- `--profile` is a new flag (see above).
- `scripts/compare-release.py` and `scripts/sync-app-manifest.py` are new
  release tooling.

### Compatibility

The default graph grows, as it only ever does: +8,107 nodes and +20,581
edges across the corpus, excluding the repository that previously timed
out. Dead-code findings *fall* by 9,146 — the new entry points give
previously-unreferenced handlers a caller.

## [0.4.2] - 2026-08-06

One real application per framework, and every detection gap that was a
defect closed. 0.4.1 checked the documentation against the code; this
release checks the *framework rules* against real applications, which
found six gaps and two counting bugs that no fixture had exercised.

### Performance

0.4.1 → 0.4.2: latency **flat**.

Measured against **0.4.0**, not 0.4.1 — 0.4.1 was never committed
separately, so it is not a ref that can be checked out and built. 0.4.1
was itself flat against 0.4.0, so the 0.4.1 → 0.4.2 delta is flat too.
Two full runs, because a single run's per-repo numbers did not reproduce:

| repo | 0.4.0 | 0.4.2 run 1 | 0.4.2 run 2 |
| --- | --- | --- | --- |
| rust-ripgrep | 429 / 421 ms | 430 ms | 426 ms |
| python-flask | 158 / 161 ms | 166 ms | 162 ms |
| js-express | 107 / 100 ms | 99 ms | 101 ms |
| go-fzf | 273 / 274 ms | 291 ms | 283 ms |
| c-jq | 155 / 149 ms | 149 ms | 152 ms |
| cpp-spdlog | 308 / 303 ms | 305 ms | 318 ms |
| csharp-serilog | 137 / 136 ms | 140 ms | 139 ms |
| swift-alamofire | 420 / 427 ms | 429 ms | 427 ms |
| cpp-nlohmann-json | 943 / 939 ms | 908 ms | 920 ms |
| **TOTAL** | **2,930 / 2,910 ms** | **2,917 ms (−0.4%)** | **2,928 ms (+0.6%)** |

**−0.4% and +0.6% — a 1.0% spread against a 1.0–1.5% noise floor, so
flat.** Per-repo deltas are *not* reported as real: they did not
reproduce between runs. `cpp-spdlog` swung −1.0% → +5.0% and `go-fzf`
+6.6% → +3.3%, on code paths this release does not touch. Two baseline
columns are shown for the same reason — the baseline binary itself
measured 2,930 ms and 2,910 ms on identical code.

Graph output on the same 9-repo set, 0.4.0 → 0.4.2, one methodology
(whole repo, default mode; `dead` from `--dead-code`, high-confidence
plus withheld):

| repo | nodes | edges | entry | dead |
| --- | --- | --- | --- | --- |
| rust-ripgrep | 2,906 | 7,100 | 0 | 1,429 |
| python-flask | 1,698→1,732 | 1,734→1,738 | 238→272 | 174 |
| js-express | 546 | 393 | 0 | 65 |
| go-fzf | 1,615 | 9,991 | 0 | 131 |
| c-jq | 1,119 | 5,463 | 0 | 425 |
| cpp-spdlog | 1,357 | 1,464 | 0 | 809 |
| csharp-serilog | 1,689 | 1,864 | 0 | 663 |
| swift-alamofire | 2,538 | 6,437 | 0 | 946 |
| cpp-nlohmann-json | 5,567 | 6,738 | 0 | 2,109 |
| **TOTAL** | **19,035→19,069** | **41,184→41,188** | **238→272** | **6,751** |

Only Flask moves, and only by the entry nodes the collapse fix
un-merged. Dead-code findings are unchanged everywhere — this release
adds entry points, it does not change what counts as unreferenced.

> **These numbers are not comparable to the 0.4.1 table below.** That one
> was gathered per-repo through `scripts/benchmark.sh`, which scans a
> configured *subdirectory* (`js-express` → `lib/`); this one scans whole
> repositories, as `scripts/perf-compare.sh` does. Hence js-express 285
> vs 546 nodes for the same code. Neither is wrong; they answer different
> questions, and mixing them in one column would be.

Reproduce with `scripts/perf-compare.sh` and
`scripts/framework-coverage.py`.

### Framework detection

The corpus went from 38 to **43 of 45 frameworks enumerating entry
points**. The two that remain are architectural limits cgg already
declares — not misses, and not silent.

> **Two counting bugs are disclosed below.** Both overstated or
> understated what cgg found, in the one table a reader consults to size
> an attack surface. Neither was caught by a fixture — both needed a real
> application.

#### One application per framework

`scripts/benchmark.sh` gains an `APPS=( … )` corpus: 35 applications that
*use* a framework, never the framework's own repository. A router's test
suite proves the grammar parses; it does not prove cgg recognises the
hand-off as an application writes it. Tracked two ways:

- **`scripts/docs-check.py`** — new gate, pure text, runs in pre-commit.
  Fails when a rule in `rules.rs` has no application, or an application
  claims a framework with no rule.
- **`scripts/framework-coverage.py`** (`benchmark.sh --apps`) — measures
  against the corpus and fails when a declared framework does not fire.
  Reports registrations and *distinct entry nodes* as separate columns,
  because they are not the same number and only reporting the first is
  how the collapse below stayed invisible.

The corpus, measured end to end. `registrations` is how many
hand-offs the resolver matched; `entry nodes` is how many distinct
nodes they became. The two differ when a framework carries no route
string to key a node on — see the collapse fix below.

| application | nodes | edges | entry nodes | registrations | time |
| --- | --- | --- | --- | --- | --- |
| fastapi-dispatch | 4,662 | 4,264 | 249 | 356 | 775 ms |
| django-netbox | 10,416 | 11,155 | 281 | 281 | 3,896 ms |
| flaskbb-flask | 2,890 | 5,119 | 159 | 164 | 381 ms |
| saleor-celery | 23,721 | 67,661 | 112 | 153 | 13,047 ms |
| black-click | 2,607 | 2,070 | 5 | 7 | 818 ms |
| torch-ultralytics | 3,284 | 5,356 | 1 | 160 | 739 ms |
| ghost-express | 20,320 | 47,954 | 272 | 356 | 8,245 ms |
| ghostfolio-nestjs | 1,908 | 3,252 | 106 | 124 | 416 ms |
| immich-nestjs | 9,351 | 16,955 | 273 | 403 | 12,298 ms |
| calcom-nextjs | 19,160 | 48,715 | 152 | 218 | 9,199 ms |
| spring-mall | 14,264 | 2,896 | 112 | 252 | 1,410 ms |
| thingsboard-concurrent | 49,316 | 151,454 | 587 | 640 | 39,080 ms |
| akka-samples | 526 | 457 | 9 | 9 | 91 ms |
| druid-jaxrs | 92,192 | 395,116 | 531 | 653 | 136,612 ms |
| micronaut-graalapp | 15 | 2 | 1 | 1 | 28 ms |
| gin-photoprism | 11,473 | 63,759 | 44 | 44 | 9,396 ms |
| memos-echo | 6,678 | 11,155 | 6 | 6 | 1,167 ms |
| fiber-recipes | 1,551 | 2,646 | 49 | 67 | 305 ms |
| homebox-chi | 10,181 | 10,282 | 98 | 103 | 2,710 ms |
| temporal-samples | 1,026 | 1,578 | 167 | 171 | 185 ms |
| eshop-aspnet | 2,568 | 873 | 57 | 69 | 266 ms |
| masstransit-sample | 217 | 21 | 13 | 18 | 51 ms |
| ombi-quartz | 14,962 | 7,055 | 429 | 513 | 2,224 ms |
| axum-cratesio | 3,451 | 12,568 | 57 | 70 | 1,223 ms |
| lemmy-actix | 2,330 | 10,046 | 135 | 207 | 749 ms |
| actix-examples | 776 | 599 | 189 | 242 | 126 ms |
| vaultwarden-rocket | 2,613 | 7,160 | 302 | 310 | 639 ms |
| rails-mastodon | 11,698 | 11,961 | 323 | 335 | 11,244 ms |
| resque-sinatra | 789 | 755 | 49 | 49 | 156 ms |
| grape-swagger | 668 | 536 | 142 | 183 | 134 ms |
| monica-laravel | 13,172 | 12,989 | 314 | 342 | 5,978 ms |
| symfony-demo | 241 | 24 | 13 | 14 | 52 ms |
| wordpress | 26,906 | 39,036 | 604 | 1,479 | 6,745 ms |
| codeigniter-starter | 26 | 7 | 2 | 2 | 40 ms |
| cuda-samples | 6,403 | 5,894 | 1 | 1 | 1,076 ms |
| **TOTAL** | **372,361** | **961,370** | **5,844** | **8,002** | **271.5 s** |

Two applications dominate that total: **Druid at 137 s** (92k nodes,
395k edges) and **thingsboard at 39 s**. They are the largest trees cgg
has been run against and are included deliberately — a framework corpus
that only holds small apps would not exercise the resolver at scale.
Excluding them the other 33 finish in 95 s combined.

A `~` prefix marks a framework cgg detects but cannot enumerate. It must
still appear in the coverage table's "seen, no rules" section — the
marker asserts the gap is *disclosed*, not that it is absent — and a `~`
that starts enumerating is flagged as stale.

#### Detection gaps closed

Each was a distinct real-world idiom no fixture had exercised:

| framework | what was missed | entries |
| --- | --- | --- |
| `actix-actor` | **`rust.rs` never populated `base_types`** — every base-type rule was dead on Rust | 0 → 30 |
| `chi` | handlers wrapped in `chain.ToHandlerFunc(...)` | 0 → 103 |
| `sinatra` | `get "/x" do … end` — Ruby hangs the block off the call's `block` field, never the argument list | 0 → 24 |
| `worker-threads` | worker modules that identify *themselves* (`import worker_threads` + `parentPort`) rather than being named at a spawn site | 0 → 7 |
| `nestjs-schedule` | nothing — the rule was correct; the corpus app scheduled via `SchedulerRegistry` and had no `@Cron` | 0 → 5 |
| `spring-messaging` | Spring AMQP puts `@RabbitListener` on the class and `@RabbitHandler` on the *method* | 0 → 1 |

The wrapper case is worth stating precisely, because the code declined to
handle it on purpose. `collect_value_refs` skipped nested calls, reasoning
that the walker visits every call on its own turn. It does — but a
wrapper's own turn *bails*, because its verb is not a registrar verb, so
nothing captured it. Descending is therefore not the double work the
comment feared. Only the innermost callee is the handler, so the recursion
emits a call's own name only when it found nothing deeper: that is what
separates `ToHandlerFunc` from `ctrl.Handle`.

`nextjs` and `blazor` remain detected-but-not-enumerated, each with a
`gap:` string saying why: Next.js routes come from file-system layout,
and `.razor` components carry `@page` in markup cgg does not parse.

#### Fixed: entry nodes collapsed onto shared names

**A routeless entry took only the last segment of its handler's qualified
name.** Every Django view named `get` merged into one
`<framework-entry>::network::django::get` node. NetBox reported **10 entry
nodes for 150 registrations** — and the coverage table said "128 entries",
so the disclosure was honest about the framework and wrong about what
could be queried. Filtering the documented attack-surface query returned 8
nodes for ~128 endpoints, and the fan-out from each was the union of
unrelated handlers.

Now the whole qualified name. NetBox reports **281**; WordPress 604,
Mastodon 323, Ghost 272. Frameworks that carry a route string
(`@app.route("/users")`) were never affected.

#### Fixed: per-language entry counts were summed, not split

`into_coverage` keyed entry counts on `(framework, "")` — an empty
language — so a framework with a rule per language reported the
**combined** total on every row. Ghost printed
`express (network, 349 entries)` twice for one set of 349, inviting the
reader to sum it to 698. Now split correctly: **301 JavaScript + 48
TypeScript**. The coverage table also names the language whenever two rows
would otherwise be indistinguishable.

## [0.4.1] - 2026-08-06

A documentation audit that turned into a correctness release. Every
factual claim in `README.md` and the bundled skills was checked against
a deterministic symbolic check — `cgg` itself, `cargo test`,
`cgg --help`, `rg`, the benchmark corpus. Most claims held. The ones
that did not split into two kinds, and both kinds are fixed here: places
where the documentation described something better than the code did,
and places where the code was quietly wrong.

### Performance

0.4.0 → 0.4.1: latency **flat** (within noise).

Maintenance release. Every metric identical to 0.4.0 — latency, nodes, edges,
entry points and findings all unchanged, as intended.

| repo | latency | nodes | edges | entry | dead |
| --- | --- | --- | --- | --- | --- |
| rust-ripgrep | 433→429 ms | 2,906 | 7,100 | 0 | 1,429 |
| python-flask | 161→157 ms | 1,460 | 1,734 | 238 | 174 |
| js-express | 103→105 ms | 285 | 393 | 0 | 65 |
| go-fzf | 287→284 ms | 1,615 | 9,991 | 0 | 131 |
| c-jq | 143→147 ms | 1,119 | 5,463 | 0 | 425 |
| cpp-spdlog | 310→325 ms | 1,357 | 1,464 | 0 | 809 |
| csharp-serilog | 143 ms | 1,689 | 1,864 | 0 | 663 |
| swift-alamofire | 422→436 ms | 2,522 | 6,437 | 0 | 946 |
| cpp-nlohmann-json | 946→937 ms | 5,567 | 6,738 | 0 | 2,109 |
| **TOTAL** | **2,948→2,963 ms** | **18,520** | **41,184** | **238** | **6,751** |

Measured across a 9-repo, 9-language comparison set. All four releases
built from source and measured together on one machine, interleaved per
repo with a discard warmup and rotated ordering. Reproduce with
`scripts/perf-compare.sh`.

**Latency noise floor is ~1.0–1.5% on the total** — two identical runs
of the same commits differ by that much, so smaller deltas are reported
as flat. Node/edge/entry/finding counts are exact and deterministic.

Zeros mean the feature did not exist in that release, not that it found
nothing. `a→b` marks a value that changed; a single value did not.

### Fixed

> **Read this section even if you skip the rest.** The first entry
> silently fabricated half of Elixir's edges. Like the `#include`
> nondeterminism fixed in 0.4.0, it produced a plausible wrong answer
> rather than an error — so nothing signalled that the graph was wrong.

- **Elixir: a definition head was recorded as a call to itself.**
  `def run(x) do … end` parses its head, `run(x)`, as a nested `call`
  node, and the walker recorded it like any other call site. Each
  phantom reference then resolved either to the function itself (a
  self-loop) or to a same-named function in another module (a bogus
  cross-file edge) — and marked its own function reachable, hiding it
  from `--dead-code`. On phoenix this was **1,404 of 2,858 edges**.
  Removing them changes no real edge: every removed edge sat exactly at
  a `def`/`defp`/`defmacro` head offset inside its own source callable,
  and no edge was added. Suppression is keyed on the head's start
  offset, so default arguments, guards, body calls and genuine
  recursion are all untouched. The benchmark row moves from 3,431 edges
  to 1,723.
- **`--max-paths` truncation was silent.** `-n 0` stopped enumerating
  at the cap and said nothing, so a capped path set was
  indistinguishable from a complete one — the caller asked for every
  route through a callable and got a prefix. Hitting the cap now prints
  a note on stderr and records a `paths_truncated` audit event. The
  event fires only when the cap actually turned away work that had been
  reached, not merely when the count landed on the limit.
- **`--dead-code` with no `-o` produced no report.** The sidecar path is
  derived from `-o`, so with the graph on stdout the report was dropped
  and only a one-line summary survived — while the documented
  invocation was `cgg ./src --dead-code`. The text report now goes to
  stderr in that case. JSON has no stderr fallback (interleaving it with
  the run summary would parse as nothing); it names the flag to pass
  instead.
- **The text report was written to a `.json` file.** The sidecar
  extension now follows `--dead-code-format`: `<output>.deadcode.txt`
  for `text`, `<output>.deadcode.json` for `json`.
- **`--write-roots` silently emitted a graph.** It lives inside the
  dead-code pass, so without `--dead-code` it fell through and printed
  ordinary mermaid — a no-op wearing the costume of a baseline. It now
  implies `--dead-code`, as `--why-live` already did.
- **`--ignore-attributes` named the wrong languages.** The "matched
  nothing" note and the `--help` text both said attribute capture was
  "python, rust" long after seven more plugins had learned it. The note
  now reads the list off the plugin registry, so it cannot rot again.
- **README self-analysis graph could not be regenerated.** It sat
  outside the `cgg:begin`/`cgg:end` markers, so the pre-commit hook
  never touched it and it had drifted to stale node ids. It is now a
  `raw:self` marker block patched on every commit.
- **The README graph generator silently dropped edges.** `clean()`
  matched edges with `" --> " in line`, which is false for cgg's own
  collapsed form `A -->|3x| B`. Every multi-site edge vanished from the
  README graphs with no error — the nodes stayed and only the arrow went
  missing. Fixed, with a `--self-test` the hook now runs.

### Changed

- **The cache feature is removed.** cgg never had an on-disk cache: the
  flags were declared in the initial commit as part of a planned task
  list and no implementation ever landed. What shipped was a hollow
  shell — `--cache DIR` and `--no-cache` parsed and were never read, an
  unused `bincode` workspace dependency, a `RESOLVER_FORMAT_VERSION`
  constant existing only to salt cache keys, a `.cgg-cache/` gitignore
  entry, and a `CacheMetrics` block emitted in **every** audit file as
  `{"hits":0,"misses":0,"bytes_read":0,"bytes_written":0}` — which reads
  as "the cache ran and got no hits", not "there is no cache". All of it
  is gone.

  The `cgg` skill had also been advising agents to leave the cache on
  because it "makes re-runs near-instant".

  **Breaking:** `--cache` and `--no-cache` are no longer accepted, so a
  command line passing either now exits 2. Unlike `--stack-graphs` and
  `--no-update-check` — kept as inert flags because they once had
  behavior a script might depend on suppressing — these never did
  anything, so nothing can depend on their effect.

  **Breaking (audit schema):** `metrics.cache` is no longer present in
  the audit document.
- `--include-tests` help text no longer claims to be "a reserved future
  knob, honored as a no-op". It has been live since 0.4.0: it widens the
  dead-code *report*, not the analysis.
- `--roots` help text now describes the discovery order it has actually
  used since 0.4.0 — analyzed paths first, then the working directory.

### Documentation

- **Three bundled skills, not two.** `skills/cgg-frameworks/SKILL.md`
  shipped in 0.4.0 and was never listed in the README;
  `install-skill.sh` had been installing it all along.
- **The audit jq recipe in the `cgg` skill did not run.** The audit
  document is a JSON array of events, so `.unresolved[]` errored with
  `Cannot index array with string`. Replaced with working queries, plus
  one that buckets unresolved sites by `reason.stage`.
- **The `cgg` skill listed six languages as lacking cross-file
  resolution that have it** (Bash, Clojure, Elixir, Erlang, Fortran,
  Julia), contradicting the README's own language table and the
  benchmark numbers. Three genuinely yield none: HCL, Verilog/SV and
  Assembly. Verilog is the subtle one — it parses `` `include ``, so it
  looks resolved, but task/function calls are never captured and so
  nothing ever crosses a file; the README table has always marked its
  cross-file column `—`, and the benchmark measures picorv32 at 0%.
- **`cgg-frameworks` contradicted itself on attribute capture**, saying
  nine languages in Step 2 and "Rust and Python only" in the limitations
  section, and pointed at `crates/cgg-core/src/frameworks_rules.rs`,
  which does not exist (it is `frameworks/rules.rs`). Its verification
  recipe also assumed the old `.deadcode.json` naming.
- Benchmark table: added the five interface/descriptor languages
  (smithy, proto, graphql, openapi, asyncapi) that had plugins and
  benchmark entries but no row, and corrected `xv6 (c+asm)` from 2,087
  to 2,092 edges. All 45 rows now reproduce exactly.
- License section enumerated seven licenses; the dependency tree
  actually uses Zlib (`foldhash`) and Unicode-3.0 (`unicode-ident`) too.
  It now points at `deny.toml` as the authority.
- Verilog's language-table row explains that `` `include `` yields no
  edges because task/function *calls* are not captured — only module
  instantiation is.
- **The `## CLI` usage synopsis never listed `--since`.** The flag
  shipped with a table row and its own worked example, but the usage
  block above them was never updated. `docs-check.py` had only ever
  validated the flag *table*, so nothing noticed.
- **The license section pointed at the wrong artifact.** It claimed
  `MIT OR Apache-2.0 OR LGPL-2.1-or-later` was "the only copyleft
  identifier anywhere in `Cargo.lock`" — but lockfiles record no license
  fields at all, so that check could never have run. The claim itself
  holds (`r-efi` is the sole crate offering an LGPL disjunct across all
  176 packages); it now names the dependency tree, which is where the
  evidence actually lives.
- `docs-check.py` grew a sixth check so the synopsis cannot drift again:
  every flag in `cgg --help` that still does something must appear in
  the usage block, and a flag named there that no longer exists fails
  the commit. Deprecated no-ops (`--stack-graphs`, `--no-update-check`)
  are exempt — they identify themselves with "No effect" in their help
  text, and the synopsis is the wrong place to advertise a flag that
  does nothing.

## [0.4.0] - 2026-08-06

Two features that ask opposite questions of the same graph.

**Dead-code reporting** asks what nothing calls. **Framework entry
points** answer why so much of the apparent answer was wrong: cgg
resolves calls it can see in source, and frameworks invoke user code by
means that are not calls. A route handler rendered as a node with
in-degree zero — which is not merely an incomplete graph but a false
claim that nothing calls it, and which then cascaded into a dead-code
finding for the handler *and* for every private helper reachable only
from it.

They ship together because neither is honest without the other. Both are
**best effort by construction**, both state their evidence, and both say
plainly what they could not see.

**Also fixes a correctness bug that silently affected every C/C++ graph
cgg has ever produced** — `#include` resolution was nondeterministic, so
C/C++ edge counts varied run to run. See *Fixed* below.

### Performance 0.4.0

0.3.0 → 0.4.0: latency **flat** (within noise).

**+2,159 edges (+5.5%) with no node change and no latency cost.** Five
repos moved, for three distinct reasons:

- **ripgrep +1,520** — Rust macro-argument call extraction. Calls inside
  `format!`/`writeln!`/`vec!` produced no edge at all before, because
  tree-sitter leaves macro bodies as unstructured token trees.
- **flask +440, express +180, alamofire +24** — of flask's gain, 238 are
  the entry-node edges themselves; the rest is new extraction reaching
  call sites it previously missed.
- **spdlog: a range replaced by a value.** The table shows 1,469 → 1,464,
  but that −5 is not a decrease — 0.3.0 has no stable edge count on this
  repo. Ten runs of the *same* 0.3.0 binary on the *same* input give
  1460 ×3, 1463 ×4, 1466 ×1, 1469 ×2; the collection run simply drew
  1,469. Ten runs of 0.4.0 give 1464 every time. `collect_include_defs`
  resolved each `#include` by taking the first `HashMap` iteration
  match, and Rust reseeds its hasher per process, so when several files
  matched an include suffix — routine in C/C++, where many directories
  carry their own `common.h` — the winner varied per run and a different
  header meant a different set of imported definitions. 0.4.0 prefers
  the exactly-resolved path, then the lowest `FileId`.

  This is also why the 0.2.0 and 0.3.0 totals in these tables are ±6
  edges run-to-run, and why their spdlog rows should be read as
  "1460–1469", not as the single number shown.

Entry points and dead-code reporting appear here for the first time.
Entry nodes are ON by default, so this is new default work absorbed at
no measurable latency cost, offset by removing the inert stack-graphs
orchestration.

| repo | latency | nodes | edges | entry | dead |
| --- | --- | --- | --- | --- | --- |
| rust-ripgrep | 442→433 ms | 2,906 | 5,580→7,100 | 0 | 0→1,429 |
| python-flask | 137→161 ms | 1,460 | 1,294→1,734 | 0→238 | 0→174 |
| js-express | 101→103 ms | 285 | 213→393 | 0 | 0→65 |
| go-fzf | 315→287 ms | 1,615 | 9,991 | 0 | 0→131 |
| c-jq | 177→143 ms | 1,119 | 5,463 | 0 | 0→425 |
| cpp-spdlog | 307→310 ms | 1,357 | 1,469→1,464 | 0 | 0→809 |
| csharp-serilog | 142→143 ms | 1,689 | 1,864 | 0 | 0→663 |
| swift-alamofire | 450→422 ms | 2,522 | 6,413→6,437 | 0 | 0→946 |
| cpp-nlohmann-json | 932→946 ms | 5,567 | 6,738 | 0 | 0→2,109 |
| **TOTAL** | **3,003→2,948 ms** | **18,520** | **39,025→41,184** | **0→238** | **0→6,751** |

Measured across a 9-repo, 9-language comparison set. All four releases
built from source and measured together on one machine, interleaved per
repo with a discard warmup and rotated ordering. Reproduce with
`scripts/perf-compare.sh`.

**Latency noise floor is ~1.0–1.5% on the total** — two identical runs
of the same commits differ by that much, so smaller deltas are reported
as flat. Node/edge/entry/finding counts are exact and deterministic.

Zeros mean the feature did not exist in that release, not that it found
nothing. `a→b` marks a value that changed; a single value did not.

### Dead-code reporting

cgg already computed the thing a dead-code finder
needs — a resolved call graph — but had no way to ask "what does nothing
call?". `--dead-code` answers that, annotating the normal graph output
rather than replacing it.

The report is **best effort by construction**: cgg reports what it could
not find a caller for, which is not the same as proving no caller
exists. Every output surface says so, every finding carries the evidence
both for and against it, and `--why-live` inverts the question so the
reasoning can be checked in the opposite direction. cgg never modifies
code and takes no position on what should be done about a finding.

#### Added

- **`--dead-code`.** Marks callables nothing appears to reference as
  `unreferenced` in whatever `-t` selects — mermaid label + `classDef`,
  dot dashed node + tooltip, a graphml `<data>` key, a json field. The
  detailed report (evidence, roots, per-language capability table) goes
  to a `<output>.deadcode.json` sidecar, the same convention the audit
  already used.
- **`--why-live PATTERN`.** Prints the shortest path from a root proving
  a callable is live, preferring high-confidence direct edges and
  non-test roots. Answers "why do you think this is used?" and, when no
  path exists, says so as a derivation rather than an assertion.
- **`cgg-deadcode.toml`.** `roots` entries are entry points and confer
  liveness transitively; `[[allow]]` entries are reviewed findings that
  are suppressed *without* being made live, so accepting one hides it
  and nothing else. Parsed with `deny_unknown_fields`, so a typo is a
  hard error rather than a silently ignored line. `--write-roots`
  generates a baseline; `--roots FILE` pins it.
- **Supporting flags:** `--dead-code-format`, `--dead-code-confidence`,
  `--dead-code-report`, `--ignore-names`, `--ignore-attributes`,
  `--fail-on-dead` (exit 3, opt-in).
- **Calls inside Rust macro arguments are now extracted.** tree-sitter
  leaves macro bodies as unstructured token trees, so a real call like
  `writeln!(out, "{}", xml_escape(s))` produced no edge. Rust edge
  counts rise ~12-27% depending on macro density; no other language is
  affected.
- **New extraction signals:** normalized `Vis` for 7 languages (was 2),
  `TestRole` and test-file classification, `ExportRecord` (Rust
  `pub use`, Python `__all__`), `DynUse` reflection hints
  (suppression-only, never an edge), and `UnreachableRegion` for
  statements after an unconditional terminator across 6 language
  families.
- **`LanguagePlugin::signals()`** — a per-plugin manifest of which
  optional signals it actually extracts, so a report can distinguish
  "this definition genuinely has no attributes" from "cgg never looked".

#### Removed

- **The update check, and with it every network call cgg makes.**
  `update_check.rs` made one `GET` to `api.github.com` per day to read a
  release tag. Its dependency, `minreq`, carried the entire HTTP/TLS
  stack — `rustls`, `rustls-webpki`, `webpki-roots` — and with it three
  RustSec advisories (RUSTSEC-2026-0098/0099/0104).

  Clearing those advisories in place meant `minreq` 2 → 3, which pulls
  `aws-lc-rs`/`aws-lc-sys` and a build-time C toolchain — a poor trade
  for a feature whole exploit surface was "someone lies to you about the
  latest version number". Removing the feature clears them outright and
  makes *offline* a property of the code rather than a default that can
  be flipped: the workspace now contains zero network call sites.

  `--no-update-check` is still accepted and does nothing, so existing
  command lines keep working. To keep an installed binary current, use
  `cargo install-update -a` (from the `cargo-update` crate) or re-run
  `cargo install --git`.

### Framework entry points

cgg resolves calls it can see in source; frameworks invoke user code by
means that are not calls. That did not merely leave the graph
incomplete — it made it **wrong**: a route handler rendered as a node
with in-degree zero, which is a claim ("nothing calls this") and a false
one.

`<framework-entry>` nodes fix that, mirroring the exit nodes
`--include-external` already minted for control leaving the tree. They
are **on by default**, deliberately unlike the exit-node flags: an exit
node tells you nothing you did not already know from reading the call,
while an entry node tells you something the source cannot state at all.

Entry nodes are an **inference, not an observation** — nothing in your
source says the call happens — so coverage is disclosed rather than
implied. Every run prints which frameworks were recognised, which were
seen and not understood, and which languages have no rules at all.

#### Added

- **`<framework-entry>` nodes.** One per entry point with real identity
  — a route, a queue, a command — carrying a trust-boundary kind in the
  qualified name (`<framework-entry>::network::flask::route("/users")`).
  Edges are `Via::FrameworkEntry(framework)` at `Confidence::Low`,
  tagged `entry` in mermaid, bold purple in dot, and `framework-entry`
  in a new graphml edge attribute.
- **Trust-boundary kinds** — `network`, `queue`, `schedule`, `cli`,
  `ffi`, `lifecycle`, `test` — filterable because they are part of the
  name: `cgg ./src --filter '<framework-entry>::network::' -n 3`
  enumerates attack surface and its blast radius in one query. Only
  `network` is asserted to carry untrusted input; `queue` depends on who
  can enqueue, which cgg cannot see.
- **Framework rules for 40+ frameworks** across python, javascript,
  typescript, java, kotlin, go, ruby, php, csharp, rust and cpp,
  covering all six hand-off shapes: attribute markers (Flask, FastAPI,
  Spring, Jakarta/Quarkus, Micronaut, NestJS, ASP.NET MVC, Symfony,
  Rocket, Actix, Celery, Click), value refs (Express, Gin, Echo, Fiber,
  Chi, net/http, Axum, Django `urls.py`, Temporal), inline closures,
  base types (PyTorch, Quartz, MassTransit, Sidekiq, Akka,
  `BackgroundService`, `Runnable`), string targets (Rails
  `'photos#index'`, Laravel's `@` string *and* `[C::class,'m']` array,
  WordPress hooks) and module paths (`worker_threads`, piscina).
- **A coverage table on every run.** Three sections, stated separately:
  what was recognised (with entry counts), what was *seen and not
  enumerated* (with the reason), and which languages have no rules.
  A framework that is recognised but matched nothing is reported as a
  gap too, because "flask (network, 0 entries)" reads as "this app has
  no routes". Also emitted as an `AuditEvent::FrameworkCoverage`, with
  `FRAMEWORK_ENTRY_DISCLAIMER` copied in by the engine so no formatter
  can drop it.
- **`[[framework]]` blocks in `cgg-deadcode.toml`,** so the gap list is
  actionable: a framework cgg does not ship rules for can be covered
  locally without waiting for a release.
- **`--no-entry-nodes`** to opt out, and **`--framework-coverage`** to
  print the table even when nothing was recognised.
- **CUDA kernels are entry points.** `tree-sitter-cpp` parses
  `saxpy<<<a,b>>>(x)` as nested comparison operators, so the launch
  produces no edge and the kernel plus every `__device__` helper read as
  dead. Treating `__global__` as a root qualifier fixes the cascade
  without fighting the grammar.

#### Extraction

- **Attribute capture** for java, csharp, typescript, javascript, php,
  kotlin and cpp (previously rust and python only). Stored **verbatim**,
  because `python.rs` refines a `DefVariant` from raw decorator text and
  `--ignore-attributes` matches what the user actually wrote. This also
  raises those languages' dead-code confidence ceiling.
- **Value-reference capture** for python, javascript, typescript, go,
  java, csharp, php and ruby (previously rust only), with two long-
  standing gaps closed: `intra_file` could only bind a value ref within
  one file, and a value ref resolved across files was tagged
  `Via::Direct` — claiming a call site that does not exist and escaping
  the `--reference-edges` flag meant to gate it.
- **Base-type capture** (`DefRecord::base_types`) for python, java,
  csharp, javascript, typescript, php and ruby, including Ruby's
  `include Sidekiq::Job` mixins. This is the principled replacement for
  the hardcoded `LIFECYCLE` name list.
- **PHP import capture** (`use`/`namespace`) and **PHP static calls**
  (`C::m()`), neither of which was extracted before. PHP's graph on the
  Laravel corpus goes from **329 edges to 16,355** (0 → 15,408
  cross-file); the run costs ~70% more wall time as a result, which is
  the price of a language whose call graph was previously ~1% resolved.
- **TypeScript signal manifest.** `TypeScriptPlugin` reused `JsWalker`
  but declared no signals and skipped the unreachable/reflection passes,
  so the dead-code capability table said cgg had never looked. Both
  fixed.
- `RefRecord` gains `context` and `route`; `DefRecord` gains
  `base_types`; `CallableNode` gains `framework_entry`. All additive and
  serde-defaulted.

#### Verified against real applications

Seven applications *using* each framework — not the frameworks' own
repositories, which never import themselves and exercise no rule:

| app | framework | entries found |
| --- | --- | --- |
| NetBox | Django | 128 network · 22 cli |
| Netflix Dispatch | FastAPI | 318 network · 38 cli |
| Mastodon | Rails + Sidekiq | 199 network · 109 queue |
| macrozheng/mall | Spring Boot | 250 network · 1 schedule |
| PhotoPrism | Gin + Chi | 44 network |
| crates.io | Axum | 70 network |
| Ultralytics | PyTorch | 159 lifecycle (root-marked, no nodes) |

Both payoffs move in the right direction on those applications, which
is the test a phase has to pass to earn its place — entry nodes up,
dead-code findings down:

| app | findings without entry nodes | with |
| --- | --- | --- |
| Ultralytics | 1,400 | 1,169 (−17%) |
| Netflix Dispatch | 2,564 | 2,133 (−17%) |

That exercise found five defects that fixtures had not:

- **A UTF-8 panic aborted the entire run.** `detect.rs` sliced a file's
  head at byte 2048 without checking the char boundary, so any file
  whose first 2 KiB contain non-Latin text crashed the process. Mastodon
  ships ~90 such translation catalogues. `type_hints.rs` had the same
  bug on `ty[..1]` for a non-ASCII identifier.
- **Rust value refs lost their route.** The registration context was
  emitted as a *second* record sharing the first's `(name, site_byte)`,
  and the context-less one won — so every axum route resolved anonymous.
- **Ambiguous verbs matched ordinary code.** `crate_ids.get(id)` and
  `session.get("user_id")` became "routes" in an axum project. A match on
  a verb like `get`/`add`/`use` now needs corroboration: an identity, or
  a receiver-less call (axum's `get(handler)` is a free function; a map
  lookup is not).
- **String routing applied everywhere.** Decoding a string into a
  handler name is now opt-in per rule (`string_targets`), set only for
  the four frameworks that route that way.
- **A marker-only rule detected everywhere.** CUDA has no import to gate
  on, so it counted as "detected" in every repository containing a C++
  file and was reported as a coverage gap in all of them.

Three coverage gaps closed as a direct result:

- **Inherited framework contracts.** A real application never inherits
  the framework base directly — NetBox writes `class
  CircuitListView(generic.ObjectListView)` and only three levels up does
  anything name Django's `View`. Base-type matching now walks the
  inheritance chain (depth-capped, cycle-guarded): Django 65 → 128
  entries, PyTorch 143 → 159, Sidekiq 96 → 109.
- **`utoipa::path`.** The `utoipa-axum` pattern registers handlers
  through `.routes(routes!(a, b, c))`, a proc-macro whose token tree
  cgg cannot read — but every one of those handlers carries its method
  and path in a `#[utoipa::path]` attribute. crates.io went 7 → 70.
- **Sidekiq workers carry no import.** Rails autoloads, so
  `app/workers/*.rb` names `Sidekiq::Worker` without requiring it; the
  convention directory is the only marker. Mastodon 0 → 109.

### Fixed

> **Read this section even if you skip the rest.** The first entry
> changed results *silently, on every run, for multiple releases*. A bug
> that returns a plausible wrong answer is worse than one that crashes:
> nothing prompts you to go looking, and any number you published in the
> meantime was wrong without saying so.

**`#include` resolution was nondeterministic — this silently affected
every C/C++ graph cgg has ever produced.** `collect_include_defs` picked
its target with `HashMap::values().find(...)`, and Rust seeds its hasher
per process, so when several files matched an include suffix — routine
in C/C++, where many directories carry their own `common.h` — the winner
varied run to run, and a different header meant a different set of
imported definitions.

Measured on `cpp-spdlog`: the same binary on the same input produced
1460 ×3, 1463 ×4, 1466 ×1, 1469 ×2 across ten runs. It now prefers the
exactly-resolved path, then the lowest `FileId`, and gives 1464 every
time.

Consequences worth knowing:

- Any C/C++ edge count published before 0.4.0 — including this
  project's own README benchmark table — was one draw from a range, not
  a fixed value.
- Determinism is a headline claim in the README. It did not hold for
  C/C++, and no test covered it. `dead_code_output_is_byte_stable` and
  the `edge_order_invariance` unit tests now do.
- The bug predates 0.3.0; it is fixed here rather than in a patch
  release because it was found while building the dead-code engine,
  whose whole model assumes a stable graph.

- **Invalid `--filter` / `--exclude-*` patterns are now a hard error.**
  A bad regex was silently mapped to match-everything, while
  `apply_exclusions` silently dropped it — two opposite silent failures
  for the same mistake.
- **Config discovery was working-directory-relative,** so
  `cgg /path/to/project` from anywhere else silently ignored that
  project's `cgg-deadcode.toml`. Discovery now searches upward from each
  analyzed path first.
- **`cross_file` de-duplicated edges with an O(edges) scan per resolved
  reference.** Invisible while PHP resolved almost nothing; ~4s of a
  Laravel run once it started resolving properly. Now indexed.
- GraphML dropped the edge `via` tag entirely, so a consumer could not
  tell an inferred edge from a resolved call.
- **Haskell definitions were never qualified by their module.**
  `extract_module` looked for a `module_name` node that
  `tree-sitter-haskell` 0.23 does not have (the kind is `module`, and
  the keyword is an anonymous token of the same name), so every Haskell
  callable came out as a bare `work` rather than `Data.Thing.work` and
  same-named functions in different modules were indistinguishable.
  Silent, because an unqualified name is still a perfectly good name.
  Haskell now joins with `.`, matching how modules are written and
  imported; on pandoc this resolves ~250 previously-unresolved calls.

### Changed

- **`--stack-graphs` has no effect** and its help text now says so. The
  integration was removed in the tree-sitter 0.26 upgrade (upstream
  pins tree-sitter 0.24); the orchestration around the resulting stub
  still ran on every invocation, deep-copying the graph, the facts and
  every file's source bytes into a thread before blocking on a
  60-second timeout. Removing it, and the retained source-byte corpus
  it kept alive, made ordinary runs measurably faster.
- Dead-code-only extraction is gated behind the mode, so a run without
  `--dead-code` does not pay for it.

### Compatibility

Default output is unchanged except for the two edge-count effects noted
above (Rust macro-argument calls, C/C++ `#include` determinism), both of
which only ever *add* or *stabilise* edges. `--stack-graphs` is still
accepted. `--include-tests`, previously parsed and never read, now has
real semantics.

**The default graph grows.** Entry nodes are on by default, so node and
edge counts move for every language with framework rules. This follows
the project's standing rule that the default graph only ever grows in
default mode; `--no-entry-nodes` restores the previous shape exactly.

Adding `Via::FrameworkEntry` is a compile error in exactly the two
`match` arms that classify edges for output, so no formatter can
silently ignore it.

## [0.3.0] - 2026-06-30

Five interface/descriptor languages, taking cgg from 39 to **44**
languages. These map an API model's shape graph onto the call-graph
model, so a descriptor renders as a topology of
service → operation → message/structure → field-type edges. Purely
additive: no existing language's graph changes.

### Performance

0.2.0 → 0.3.0: latency **flat** (within noise).

Five interface/descriptor languages added. Graph unchanged on this set (+3
edges) because those languages are not present in it.

| repo | latency | nodes | edges | entry | dead |
| --- | --- | --- | --- | --- | --- |
| rust-ripgrep | 432→442 ms | 2,906 | 5,580 | 0 | 0 |
| python-flask | 149→137 ms | 1,460 | 1,294 | 0 | 0 |
| js-express | 95→101 ms | 285 | 213 | 0 | 0 |
| go-fzf | 317→315 ms | 1,615 | 9,991 | 0 | 0 |
| c-jq | 170→177 ms | 1,119 | 5,463 | 0 | 0 |
| cpp-spdlog | 310→307 ms | 1,357 | 1,463→1,469 | 0 | 0 |
| csharp-serilog | 147→142 ms | 1,689 | 1,864 | 0 | 0 |
| swift-alamofire | 452→450 ms | 2,522 | 6,413 | 0 | 0 |
| cpp-nlohmann-json | 934→932 ms | 5,567 | 6,738 | 0 | 0 |
| **TOTAL** | **3,006→3,003 ms** | **18,520** | **39,019→39,025** | **0** | **0** |

**Caveat on this table:** cgg's `#include` resolution is nondeterministic
in this release (fixed in 0.4.0). `cpp-spdlog`'s edge count varies
1460–1469 across runs of this same binary, so its row — and the totals —
are one draw from a range, not a fixed value.

Measured across a 9-repo, 9-language comparison set. All four releases
built from source and measured together on one machine, interleaved per
repo with a discard warmup and rotated ordering. Reproduce with
`scripts/perf-compare.sh`.

**Latency noise floor is ~1.0–1.5% on the total** — two identical runs
of the same commits differ by that much, so smaller deltas are reported
as flat. Node/edge/entry/finding counts are exact and deterministic.

Zeros mean the feature did not exist in that release, not that it found
nothing. `a→b` marks a value that changed; a single value did not.

### Added

- **Smithy, Protobuf, GraphQL, OpenAPI/Swagger, and AsyncAPI plugins.**
  - Smithy: `service → operation → structure → shape-member` edges;
    traits and prelude primitives skipped. The published
    `tree-sitter-smithy` crate pins an incompatible `tree-sitter 0.20`,
    so its generated `parser.c` is **vendored** under
    `crates/cgg-lang/vendor/smithy/` (MIT, see `PROVENANCE.md`),
    compiled by a new `crates/cgg-lang/build.rs`, and bound through
    `tree_sitter_language::LanguageFn`.
  - Protobuf: message field types + gRPC `service` rpc →
    request/response message edges.
  - GraphQL: SDL `type → field-type`, `implements`, and `union` member
    edges; built-in scalars skipped.
  - OpenAPI/Swagger and AsyncAPI: YAML **or** JSON (both parsed with the
    YAML grammar), content-detected by their root `openapi:` /
    `swagger:` / `asyncapi:` key via a new `cgg-lang::detect` rule, so
    ordinary `.yaml`/`.json` config/data files are untouched.
    Operation → schema and schema → schema (`$ref`) edges; AsyncAPI adds
    channel/message edges.
- **Cross-file resolution for descriptor languages.** References in
  Smithy/Protobuf/GraphQL/OpenAPI/AsyncAPI resolve by global simple-name
  within the model (bounded to ≤4 candidates) — see
  `cgg-resolve::cross_file`.

### Changed / Improved

- **Per-language stdlib filter audit.** 21 stdlib lists (bash, c, cpp,
  clojure, dart, elixir, erlang, go, groovy, haskell, hcl, javascript,
  kotlin, lua, objc, perl, php, python, ruby, typescript, zig) tuned
  against real-world `external`-bucket noise. Eight remain seeded from
  language references only (csharp, fortran, java, julia, ocaml, r,
  scala, swift).
- Docs synced to the code: README language table/count (44), embedded
  mermaid graphs, self-stats, and the Limitations / Potential-future-
  improvements sections; `skills/cgg/SKILL.md`; `CLAUDE.md`; and the
  `scripts/benchmark.sh` targets for the five new languages.

### Compatibility / migration

- **To keep the previous behavior: do nothing.** The five new languages
  only add graphs for file types that previously produced none. No
  existing language's nodes or edges change. `.yaml`/`.json` files are
  analyzed only when their root key marks them as an OpenAPI/AsyncAPI
  document.

## [0.2.0] - 2026-06-18

A resolver-precision pass (the `necessary_fixes.md` program) plus four
opt-in output modes. Verified against a 38-language real-world corpus:
**the default graph is a strict superset of the previous one — 0 nodes
and 0 edges lost in any language** (checked at per-call-site,
overload-distinguishing granularity), and faster.

### Performance

Baseline for the series; 0.1.0 predates this CHANGELOG and was not
measured. Dead-code reporting and framework entry points did not exist.

| repo | latency | nodes | edges | entry | dead |
| --- | --- | --- | --- | --- | --- |
| rust-ripgrep | 432 ms | 2,906 | 5,580 | 0 | 0 |
| python-flask | 149 ms | 1,460 | 1,294 | 0 | 0 |
| js-express | 95 ms | 285 | 213 | 0 | 0 |
| go-fzf | 317 ms | 1,615 | 9,991 | 0 | 0 |
| c-jq | 170 ms | 1,119 | 5,463 | 0 | 0 |
| cpp-spdlog | 310 ms | 1,357 | 1,463 | 0 | 0 |
| csharp-serilog | 147 ms | 1,689 | 1,864 | 0 | 0 |
| swift-alamofire | 452 ms | 2,522 | 6,413 | 0 | 0 |
| cpp-nlohmann-json | 934 ms | 5,567 | 6,738 | 0 | 0 |
| **TOTAL** | **3006 ms** | **18,520** | **39,019** | **0** | **0** |

**Caveat on this table:** cgg's `#include` resolution is nondeterministic
in this release (fixed in 0.4.0). `cpp-spdlog`'s edge count varies
1460–1469 across runs of this same binary, so its row — and the totals —
are one draw from a range, not a fixed value.

Measured across a 9-repo, 9-language comparison set. All four releases
built from source and measured together on one machine, interleaved per
repo with a discard warmup and rotated ordering. Reproduce with
`scripts/perf-compare.sh`.

**Latency noise floor is ~1.0–1.5% on the total** — two identical runs
of the same commits differ by that much, so smaller deltas are reported
as flat. Node/edge/entry/finding counts are exact and deterministic.

Zeros mean the feature did not exist in that release, not that it found
nothing. `a→b` marks a value that changed; a single value did not.

### Added

- **Update check.** A best-effort, **opt-out**, once-a-day "newer
  release available?" notice. It runs on a background thread that
  overlaps the analysis, prints a single line to stderr only in an
  interactive terminal, and caches its result in
  `$XDG_CACHE_HOME/cgg/update-check.json` (so the network is hit at most
  once per 24h). It is cgg's *only* network access, never affects the
  graph/output/exit-code, and is disabled by `--no-update-check`,
  `--quiet`, a non-interactive invocation, or `CGG_NO_UPDATE_CHECK` /
  `DO_NOT_TRACK` / `CI`. (Adds cgg's first network dependency, `minreq`
  - rustls — binary stays self-contained, no system OpenSSL.)
- **`--include-external` / `--include-stdlib`** — surface calls into
  third-party / standard-library code as deduplicated leaf "exit nodes"
  (one node per `(language, receiver, name)` symbol; every call site
  collapses onto it with multiplicity). Edges tagged `ext` / `std`.
- **`--dynamic-dispatch`** — for interface/trait dispatch, emit fan-out
  edges from each method *declaration* to every concrete *implementation*
  (one low-confidence edge per impl). The exact call-site → declaration
  edge is always emitted; this flag adds the over-approximated dispatch.
  Edges tagged `dyn`. (Plugin capture wired for Rust; resolver/format
  machinery is language-agnostic.)
- **`--reference-edges`** — when a function is passed *by name* as a
  value (`register(handler)`), emit a reference edge distinct from a
  call edge, repairing the "registered handler looks like dead code"
  distortion. Edges tagged `ref`. (Rust.)
- New `Via` edge kinds (`External`, `Stdlib`, `Reference`) and
  `CallableNode` fields (`synthetic`, `trait_impl_target`), rendered as
  label tags in mermaid (`ext`/`std`/`dyn`/`ref`), edge styles in dot,
  and serialized in json/graphml.
- **Structured unresolved-call audit** — each unresolved record now
  names the resolution *stage* that rejected it
  (`no-candidate-in-file`, `ambiguous-in-file`, `no-candidate-cross-file`,
  …) plus the evidence it had (candidate counts, which name-screen was
  applied). The unresolved population is now sliceable by category for
  regression tracking.

### Changed / Improved (default mode — no flags needed)

- **Toolchain.** Moved to the Rust **2024 edition**; minimum supported
  Rust is now **1.85** (was 1.80). No API changes — a one-line
  match-ergonomics adjustment was the only code impact.
- **Cache format.** `RESOLVER_FORMAT_VERSION` bumped to `2` so stale
  `.cgg-cache` entries from 0.1.x are re-extracted (the new
  function-as-value records and edge kinds need a fresh pass).
- **Owner-qualified disambiguation.** Same-name candidates
  (`Parser::new` vs `Cursor::new`, and `Self::new` inside an impl) are
  now disambiguated by the call's owner qualifier instead of being
  abandoned as ambiguous.
- **Cross-file receiver resolution.** Method calls on a receiver of
  known type now resolve through an `(owner type, method)` index —
  including through import aliases (`use a::b::Engine as Motor`) and
  multi-segment receiver paths. This also made resolution **faster**:
  the index replaces a per-call-site O(callables) scan with an O(1)
  lookup (≈ −40% wall time on method-heavy Kotlin, −33% on Rust).
- **Standard-library name-collision ordering.** A project method whose
  name collides with stdlib vocabulary (`EntityId::len`) is no longer
  siphoned into the stdlib bucket — owner ownership is checked first.

### Fixed

- The summary line's `cross-file` count used a formula that predated
  edge deduplication; it now counts actual inter-file edges of the whole
  analysis and stays consistent with the `edges` total even under
  `--filter`/`-n`.
- A latent subtract-overflow panic in the summary computation
  (surfaced by the new synthetic edges).

### Compatibility / migration

- **To keep the previous behavior: do nothing.** All new behavior is
  either a strictly-additive precision improvement or gated behind an
  opt-in flag. With no new flags, the default graph contains every node
  and edge the previous version produced (verified across 38 languages),
  plus newly-resolved direct edges — nothing is removed or retargeted at
  the unique-edge level.
- **To get the new structural views:** add the opt-in flags above. They
  only *add* tagged edges/nodes. Downstream consumers can include or
  exclude them by the mermaid label tags (`ext`/`std`/`dyn`/`ref`) or by
  the `via` / `confidence` fields in json/graphml.
- **Audit consumers:** the unresolved `reason` field is now a structured
  object (`{"stage": …}`); the deserializer still accepts the old
  free-form string, so existing tooling that only reads other fields is
  unaffected.
