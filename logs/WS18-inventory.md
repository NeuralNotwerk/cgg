# WS18 inventory — harness / framework / registration entries, per language

Legend: **before** = WS18-before (8be891e); **after** = this workstream.
"node" = mints a `<framework-entry>` origin node + `entry` edge;
"root" = dead-code root mark only; "test file" = hidden by file-level
test classification (`classify_test_file`); "—" = not recognised.
Test/bench/fuzz harnesses mint ONE node per harness
(`<framework-entry>::test::<harness>`), every other rule one node per key
(route / event / signal) or per handler when there is no key.

Corpus evidence: `WS17-v7/new/*.dead.json` joined with graph attributes
(`scratchpad/ws18/findings.jsonl`, 1,265,216 findings; 1,292 high).

## Rust
| framework | marker | before | after |
|---|---|---|---|
| libtest + wrappers | `#[test]`, `#[tokio::test]`, `#[async_std::test]` | root (exact) | node `test::cargo-test` |
| path-qualified test macros | `#[ntex::test]` (156 high), `#[crate::rt_test]` (127), `#[actix_web::test]` (25), `#[googletest::test]` (4), `#[sqlx::test]` | — (exact-path table) | node `cargo-test` (last segment `test`), test_role Case |
| `*_test` macros | `#[wasm_bindgen_test]` (2), `#[crate::rt_test]` | — | node `cargo-test` (`*_test` suffix) |
| rstest / test_case / quickcheck | `#[rstest]`, `#[test_case(..)]` | root | node `cargo-test` |
| libtest bench | `#[bench]` | root | node `test::cargo-bench` |
| criterion | `criterion_group!(g, f…)` macro args | gap | gap (macro token tree; follow-up) |
| PyO3 | `#[pymethods] impl` members (`__next__`, `#[getter]`, `#[new]`, …) ~60 high incl. dunders | — (attribute on the impl, not the fn) | node `ffi::ffi-export` per member (container attribute) |
| uniffi | `#[uniffi::export] impl` | — | node (container) |
| async-graphql | `#[Object]`/`#[ComplexObject]`/`#[Subscription]` impl members | — | node `network::async-graphql` (container) |
| juniper | `#[graphql_object]` impl members | — | node `network::juniper` (container) |
| Tauri | `#[tauri::command]` (13 high, 78 medium) | gap | node `network::tauri` (attribute `command`) |
| RTIC | `#[init]`/`#[idle]`/`#[task]` (45 high, 114 medium) | — | node `lifecycle::rtic` (marker-only) |
| rmcp (MCP) | `#[tool]`, `#[prompt]`, `#[tool_handler] impl` | — | node `network::rmcp` |
| ntex routing | `web::resource(p).to(h)`, `.route(p, web::get().to(h))` | value-ref liveness only | node `network::ntex::to("/p")` (registrars) |
| salvo routing | `Router::with_path(p).get(h)`, `.goal(h)` | liveness only, attribute rule | + registrars |
| runtime hooks | `#[panic_handler]`, `#[defmt::panic_handler]`, `alloc_error_handler` | — | root only (compiler hook, not a data entry) |
| embedded-rt / ffi-export / axum / actix / rocket / lambda | attributes / registrars | node | unchanged |

## Python
| framework | marker | before | after |
|---|---|---|---|
| pytest | `def test_*`, `Test*` methods, `@pytest.fixture`, xunit hooks | test_role / test file (hidden), gap | node `test::pytest` (test files or files importing pytest) |
| unittest | `TestCase` subclasses: `test*`, `setUp`… | root, EVERY member (no method filter) | node `test::py-unittest`, only `test*` + setUp/tearDown family (helpers become test-only) |
| marimo | `@app.cell` (52 high) | — | node `lifecycle::marimo` |
| SQLAlchemy events | `@event.listens_for(T, "evt")` (8 high) | — | node `lifecycle::sqlalchemy-events::listens_for("evt")` |
| atexit | `atexit.register(f)` | gap ("register" too common) | node, receiver-qualified registrar `atexit.register` |
| signal | `signal.signal(S, h)` | node | unchanged (but `import a, signal` single statement not detected — follow-up) |
| Pyomo-style kwargs `Var(initialize=f)` | value ref | live (value ref) | unchanged (no framework key) |

## C#
| framework | marker | before | after |
|---|---|---|---|
| xUnit | `[Fact]`/`[Theory]` | root (roots.rs), rule had no matchers | node `test::xunit` |
| NUnit | `[Test]`,`[TestCase]`,`[SetUp]`… | root / node:false | node `test::nunit-runner` |
| MSTest | `[TestMethod]`… | node:false | node `test::mstest` |
| BenchmarkDotNet | `[Benchmark]`, `[GlobalSetup]`… (56 medium) | — | node `test::benchmarkdotnet` |
| Unity editor | `[MenuItem]`, `[ContextMenu]`, `[DrawGizmo]`, `[InitializeOnLoadMethod]`… (11 high) | — | node `ui::unity-editor` (keyed by menu path) |
| Godot RPC | `[Rpc]` (2 high) | — | node `network::godot-rpc` |
| Godot scene signals | `[connection signal=… method=…]` in sibling `.tscn` (14 high) | gap | node `lifecycle::godot-signal::<signal>` (same-stem scene only) |
| XAML / AXAML events | `Click="H"` in `X.xaml` for `X.xaml.cs` (47 high) | — | node `ui::xaml::<event>` (handler must have `(sender, …Args)` shape) |
| Razor | `@onclick="H"`, `OnX="H"` in `X.razor` for `X.razor.cs` (20 high) | — | node `ui::razor::<event>` |
| .NET exports | `[JSExport]` (3), `[UnmanagedCallersOnly]`, `[DllExport]`, `[ComRegisterFunction]`/`[ComUnregisterFunction]` (2) | root via ffi classify (JSExport/COM: —) | node `ffi::dotnet-ffi-export` |
| `+=` event subscription | `btn.Click += H` | live (value ref) | unchanged; no key worth a node (in-process wiring) |
| `Rpc("Name")` string dispatch | string | — | covered by `[Rpc]` attribute on the target |

## Java / Kotlin / Groovy / Scala
| framework | marker | before | after |
|---|---|---|---|
| JUnit 4/5 | `@Test`, `@BeforeEach`… | node:false root | node `test::junit` |
| JUnit 3 | `TestCase` subclass `test*`/`setUp`/`tearDown` | — | node `test::junit` (base type + name) |
| JUnit sources / TestNG | `@MethodSource`, `@DataProvider`… | node:false | node (harness) |
| JMH | `@Benchmark`, `@Setup`, `@TearDown` (167+41 medium) | — | node `test::jmh` (Java, Kotlin) |
| Caliper | `@BeforeExperiment` (4 high), `@Benchmark` | — | node `test::caliper` |
| Cucumber | `@Given/@When/@Then/@And/@But`, hooks (1,087 medium) | — | node `test::cucumber` (Java, Kotlin) |
| Dagger / Guice (Kotlin) | `@Provides` (140 medium) | Java only | Kotlin twins, root only (DI wiring, no outside data) |
| EventBus (Kotlin) | `@Subscribe` (128) | Java only | Kotlin twin, root only (in-process bus) |
| Gradle task annotations | `@TaskAction`, `@Input*`… Kotlin (160), Groovy (85+94+76) | Java only | Kotlin + Groovy twins, root only (build-tool lifecycle) |
| JPA property access | `@Column`/`@Id`… on getters (331 medium) | — | root only `jpa-mapping` (serialization-like) |
| Spock / ScalaTest / specs2 | string-named features | gap (node:true, no matchers) | gap; node now named per harness |
| JSF EL `#{bean.m}` in `.xhtml` | markup | — | follow-up (markup reader) |
| JAXB `@XmlType(factoryMethod="newX")` | string attribute arg | — | follow-up (member_sources_for row) |

## Go
| `testing` | `Test*`/`Benchmark*`/`Fuzz*`/`Example*`/`TestMain` | test_role + roots.rs name rule; rule had no matchers | node `test::go-testing` (test files / files importing testing) |
| net/http, gin, echo, chi, cobra… | registrars | node | unchanged |

## Swift / Objective-C
| XCTest (Swift) | `XCTestCase` subclass `test*`, `setUp`… | — (gap) | node `test::xctest` (base type + name) |
| XCTest (ObjC) | `- (void)test*` (559 medium in `*Test.mm`) | — | node `test::xctest` (files importing XCTest; ObjC records no bases) |
| swift-testing | `@Test` | gap | gap — the Swift plugin records no attributes (follow-up) |

## PHP / Ruby
| PHPUnit | `TestCase` subclass members | root, EVERY member | node `test::phpunit`, only `test*` + setUp family and `#[Test]` |
| Minitest | `Minitest::Test` subclass members | root, EVERY member | node `test::minitest`, only `test_*` + setup/teardown |
| RSpec | blocks | gap | gap (closures) |
| WordPress `add_action('h','fn')` / Rails callbacks | registrars | node / root | unchanged |

## Erlang / Elixir / Haskell / OCaml / F#
| EUnit | `*_test()`, `*_test_()` | gap | node `test::eunit` (modules including eunit) |
| Common Test | `all/0`, `init_per_*`, `end_per_*` | gap | node `test::common-test` (suite callbacks; cases named only in `all()` list stay a gap) |
| ExUnit | `test "…" do` macro blocks | test file | unchanged (closures) |
| hspec / tasty | `spec`, `tests` | node:false | node (harness) |
| F# NUnit / xUnit | `[<Test>]`, `[<Fact>]` | gap | node `test::fsharp-nunit` |
| Expecto / Alcotest | value lists | gap | gap |

## Others
| Solidity Foundry | `test*`, `setUp`, `invariant_*` | already entries via `solidity-public` (public/external) | unchanged (a separate rule would be deduped away) |
| C/C++ gtest/Catch2/doctest | `TEST(…)`, `TEST_CASE("…")` macros | gap | gap (macro bodies are not callables; follow-up) |
| Unity C (`void test_x(void)` + `RUN_TEST`) | macro args | — | follow-up |
| JS/TS jest/vitest/mocha/playwright | `describe/it/test` closures | test file / gap | gap (anonymous callbacks); node renamed per harness |
| Dart flutter_test, Kotlin kotest, Lua busted, R testthat, Julia @testset, Pester, Perl Test::More, Zig `test {}`, OCaml `let%test` | closures / blocks | test file / gap | unchanged gaps |
| Fortran pFUnit, Nix, Bash | — | — | not applicable / no corpus evidence |

## Node-vs-root decisions (coordinator rule 1)
- **Node (data enters from outside the program):** every test/bench harness
  (one node per harness), GraphQL fields, Tauri commands (IPC from the
  webview), rmcp tools (MCP client), Godot `[Rpc]` (remote peers), .NET and
  PyO3/uniffi exports (foreign host), XAML/Razor/Godot-scene UI events
  (user input; key = event), Unity editor menus (key = menu path),
  SQLAlchemy events (key = event), marimo cells, RTIC init/idle/tasks
  (hardware interrupts), ntex/salvo routes, `atexit.register`.
- **Root only (in-process wiring, no outside data):** Kotlin Dagger/Guice
  providers, Kotlin EventBus subscribers (posted by the app itself),
  Gradle task/input annotations (build-tool lifecycle, Java rule was
  already root-only), JPA mapping getters (persistence provider,
  like `jackson`), Rust `#[panic_handler]`/`alloc_error_handler`
  (compiler hooks), Python/PHP/Ruby test-class helpers (test roots).
- **Existing `node: false` rules NOT flipped:** DI (`spring-context`,
  `guice`, `dagger`, `jsr330-inject`), serialization (`jackson`, `moshi`,
  `java-serialization`, `dotnet-serialization`, `serde`), language
  runtime/trait contracts (`rust-std-traits`, `java-runtime`,
  `csharp-runtime`, `go-runtime`, `java-closeable`, `csharp-disposable`),
  lifecycle base-type contracts (`unity`, `godot`, `monogame`, Android,
  JPA lifecycle, Hibernate listeners), `kivy`, `py-threading`,
  `c-callbacks`/`cpp-callbacks` (mixes signal handlers with qsort
  comparators — splitting it is a follow-up). All are runtime or
  in-process callbacks; one node per handler would be noise.

## Results (ab/WS18-v5, 283 repos, none excluded)
- Real call edges (non-entry): identical in every repo.
- Entry nodes 21,401 -> 22,367; entry edges 30,857 -> 272,162.
- Dead findings removed 7,603 (high 616: Rust 468, C# 84, Python 60,
  Java 4); added 2,378 (C# 2,365 in app-orleans-samples, previously
  withheld below the 25% coverage gate, crossed 24% -> 25% through the new
  production XAML/Razor entries; Rust 12 py-robyn same mechanism;
  Haskell 1 `Hadolint.Shell.spec`, a ShellCheck value the old node:false
  hspec rule rooted as a production root); 0 raised in confidence.
- Sampled removed findings per class (entry classes, root-only classes,
  downstream): no false roots found in the samples; cargo-test attribute
  census: 6,650 targets under `test`, `tokio::test`, `ntex::test`,
  `crate::rt_test`, `actix_web::test`, `traced_test`,
  `googletest::test`, `wasm_bindgen_test`, `test_case` — all test
  attributes. XCTest ObjC targets with arguments: 0.
