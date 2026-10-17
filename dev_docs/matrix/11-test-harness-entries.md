# 11. Test harness entries

This category covers how cgg recognises the code that a test, benchmark or fuzz runner calls, as opposed to code the tree itself calls. A runner calls test cases, fixture hooks and data providers by convention: by annotation (`@Test`, `#[test]`, `[Fact]`), by base class (`unittest.TestCase`, `XCTestCase`), by name pattern (`Test*`, `*_test`), or by a registration DSL (`it(...)`, `test "x" do`). cgg has five mechanisms for this:

1. **Harness rules of kind `test`** (`TrustKind::Test` in `crates/cgg-core/src/frameworks/rules.rs`; 53 of them). An enumerating rule mints one node per harness, `<framework-entry>::test::<id>`, with an `entry` edge to each case. A rule with no matchers is "detect-only": it reports a `gap` and roots nothing.
2. **Test-class members as roots** (`TEST_CLASS_MEMBERS_ARE_ROOTS`, rules.rs:11806). This applies to Python, PHP and Ruby only.
3. **Built-in dead-code test roots**: the attribute list `TEST_ATTRS` (`crates/cgg-resolve/src/deadcode/roots.rs:211-225`) and the name list `is_test_name` (roots.rs:975-989).
4. **Test-file classification** (`classify_test_file`, `crates/cgg-core/src/testfile.rs:50-134`). It works by file name, by language layout, and by the generic `TEST_DIRS` directory list.
5. **In-file `TestRole` tagging** by the plugin (`PluginSignals::test_defs`). Only Go, Python and Rust declare it (go.rs:42, python.rs:50, rust.rs:44).

`--no-test-entries` turns every harness node into root marks for every language. It is a generic CLI behaviour (CLAUDE.md, phase 6), so it is not a column.

## Legend

| Mark | Meaning |
| --- | --- |
| ✅ | full: the capability works for the language's mainstream harness(es) |
| 🟡 | partial: works for some forms, over-broad, detect-only, or works by accident |
| ⬜ | none: the language has the concept and cgg does not support it |
| — | n/a: the concept does not exist in the language (or the language has no tests) |

Columns:

- **Harness node**: ✅ means an enumerating kind-`test` rule mints `<framework-entry>::test::<id>`. 🟡 means only a detect-only gap rule exists. ⬜ means no rule exists.
- **Annot.**: harness-rule cases selected by an annotation or attribute.
- **Base type**: harness-rule cases selected by a test base class (followed transitively, `find_in_base_chain`, frameworks/mod.rs:893).
- **Name pattern**: harness-rule cases selected by a name pattern (`prefix*` / `*suffix` / exact name). Name-only rules are gated to a test file or a file importing the harness (frameworks/mod.rs:1009-1021).
- **Hooks**: setup and teardown, before and after, and suite callbacks.
- **Bench/fuzz**: benchmark and fuzz harnesses.
- **Data src**: data-provider members that are named by string, `nameof` or a type (`member_sources_for`, rules.rs:11719-11760).
- **Class members**: other members of a test class become test roots (`TEST_CLASS_MEMBERS_ARE_ROOTS`).
- **Builtin attr**: a `builtin:test-attr` root from `TEST_ATTRS`. It needs the plugin to capture attributes.
- **Builtin name**: a `builtin:test-name` root from `is_test_name`. ✅ means a language-specific arm. 🟡 means the generic `test_`/`Test` default arm, or an over-broad arm.
- **File name**: a language-specific test-file name rule (testfile.rs:76-119).
- **Dir/layout**: classification by the generic `TEST_DIRS` (testfile.rs:36-46, 125-131) or a language layout (`src/test/`, Rust `tests/`/`benches/`).
- **TestRole**: in-file `TestRole` tagging (`test_defs`).
- **DSL bodies**: test bodies written as DSL blocks or closures (`it(...)`, `test "x" do`, `TEST_CASE("x")`) get a caller. ✅ means their calls are attributed to a test callable.

## The matrix

| Language | Harness node | Annot. | Base type | Name pattern | Hooks | Bench/fuzz | Data src | Class members | Builtin attr | Builtin name | File name | Dir/layout | TestRole | DSL bodies | Notes (most important gap) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rust | ✅ cargo-test, cargo-bench | ✅ last segment + `*_test` | — | — (deliberately off, roots.rs:986) | — | 🟡 `#[bench]` only; criterion gap (rules.rs:9760), no cargo-fuzz | — | — | ✅ | — (attribute-driven) | — | ✅ `tests/`, `benches/` | 🟡 | 🟡 `proptest!{}` not expanded | Helpers in `#[cfg(test)] mod tests` get no TestRole and are reported as **high** production dead code |
| python | ✅ py-unittest, pytest | 🟡 `@pytest.fixture` only | ✅ TestCase family | ✅ `test*`, xunit hooks | 🟡 setUp/xunit hooks yes; conftest `pytest_*` hooks no | ⬜ pytest-benchmark, atheris | ⬜ `indirect=True`, fixtures-by-param | ✅ | 🟡 `pytest.mark` only | ✅ `test_*`, setUp, tearDown | ✅ `test_*.py`, `*_test.py`, conftest | ✅ | ✅ | — | conftest `pytest_*` hooks are not rooted (declared gap, rules.rs:8403) |
| kivy | — | — | — | — | — | — | — | — | — | — | — | — | — | — | No test concept in KV markup. Tests are Python. |
| javascript | 🟡 jest, vitest, playwright gap-only | — | — | ⬜ | ⬜ beforeEach/afterAll | ⬜ | — | — | — | 🟡 default arm | 🟡 `.spec.mjs`/`.spec.cjs` missing | ✅ | ⬜ | ⬜ `it`/`describe` callbacks are not callables | No enumerating harness at all; no mocha/node:test/ava/jasmine/cypress rule |
| typescript | 🟡 jest, vitest, playwright gap-only | — | — | ⬜ | ⬜ | ⬜ | — | — | — | 🟡 default arm | 🟡 `.test.mts/.cts`, `.spec.mts/.cts/.mjs/.cjs` missing | ✅ | ⬜ | ⬜ | Same as JS. Calls in `it(...)` have no enclosing callable. |
| go | ✅ go-testing | — | — | ✅ `Test*`/`Benchmark*`/`Fuzz*`/`Example*`/`TestMain`, gated | ✅ `TestMain` | ✅ Benchmark/Fuzz | — | — | — | 🟡 ungated, no case check (`Testing` rooted) | ✅ `_test.go` | ✅ | 🟡 `Test_under` untagged | ✅ `t.Run` closures attributed to the test | Over-broad `is_test_name`; stale gap text (rules.rs:7642) |
| java | ✅ junit, testng, jmh, caliper, cucumber, junit-sources | ✅ | ✅ JUnit 3 `TestCase` | ✅ JUnit 3 `test*` (base-gated) | ✅ | 🟡 JMH, Caliper; no Jazzer `@FuzzTest` | ✅ MethodSource, ArgumentsSource, `@Parameters` | ⬜ (deliberate, rules.rs:11797-11805) | ✅ `Test` | 🟡 default arm, ungated | ✅ `*Test`/`*Tests`/`*IT.java` | ✅ `src/test/` | ⬜ | — | `@Nested`, `@RunWith(Parameterized)`, `@ExtendWith` gap-only (junit-params, rules.rs:7088) |
| kotlin | ✅ junit, jmh, cucumber | ✅ | ⬜ no JUnit 3 `TestCase` base (Java has) | ⬜ | ✅ | 🟡 JMH only | ⬜ `@MethodSource` provider not rooted (fixture `kt`) | ⬜ (deliberate) | ✅ `Test` | 🟡 default arm | ✅ `*Test.kt`/`*Tests.kt` | 🟡 `src/test/` yes, `androidTest/` no | ⬜ | ⬜ Kotest/Spek lambdas | No `junit-sources` counterpart: data providers stay dead |
| c | ⬜ | — | — | ⬜ Unity/cmocka/Check/CUnit | ⬜ | ⬜ | — | — | — | 🟡 default arm | ✅ `*_test.c`, `test_*`, `*_unittest.*` | ✅ | ⬜ | — | No C test-framework rule of any kind |
| cpp | 🟡 gtest, cpp-test-macros gap-only | — | ⬜ `::testing::Test` not used | ⬜ | ⬜ `SetUp`/`TearDown` reported dead | ⬜ Google Benchmark, libFuzzer | ⬜ `TEST_P` instantiation | ⬜ | — | 🟡 default arm | ✅ as C | ✅ | ⬜ | ⬜ `TEST`/`TEST_F` are callables but reported dead; Catch2/doctest bodies absent | gtest `TEST`/`TEST_F` bodies are extracted and then reported dead at medium (fixture `gt`) |
| csharp | ✅ xunit, nunit-runner, mstest, benchmarkdotnet, xunit-data | ✅ | — | — | ✅ | ✅ BenchmarkDotNet (fuzz ⬜) | 🟡 MemberData, ClassData, TestCaseSource yes; parameter `[ValueSource]` no (fixture `cs`) | ⬜ (deliberate) | ✅ `Test`/`Fact`/`Theory`/`TestMethod` | 🟡 default arm (`TestConnection()` is rooted) | ✅ `*Test.cs`/`*Tests.cs` | 🟡 `.Tests` project dirs not classified | ⬜ | — | xUnit class ctor and `IClassFixture<T>` not rooted; no SpecFlow/Reqnroll |
| bash | ⬜ | — | — | ⬜ bats/shunit2 | ⬜ | — | — | — | — | 🟡 `test_bar` rooted, shunit2 `testFoo` not (fixture `gd`) | ⬜ `*_test.sh`; `.bats` undetected | ✅ (fixture `gd`) | ⬜ | ⬜ bats `@test` blocks | `.bats` files are not even detected |
| ruby | ✅ minitest | — | ✅ Minitest/Test::Unit/ActiveSupport | ✅ `test_*` | ✅ setup/teardown | ⬜ benchmark-ips | — | ✅ | — | ✅ `test_*` | ✅ `_spec.rb`, `_test.rb` | ✅ | ⬜ | ⬜ RSpec `it`, AS `test "x" do` | RSpec is gap-only (rules.rs:7009) |
| swift | ✅ xctest | ⬜ swift-testing `@Test`/`@Suite` gap-only | ✅ `XCTestCase` (transitive) | ✅ `test*` | 🟡 class and instance `setUp` not told apart | ⬜ `measure {}` | — | ⬜ (deliberate) | ⬜ attributes not captured (fixture `sw`: `addition` reported) | 🟡 default arm | ✅ `*Tests.swift`/`*Test.swift` | ✅ | ⬜ | ⬜ Quick/Nimble | swift-testing `@Test` functions are reported dead |
| lua | ⬜ | — | — | ⬜ busted/LuaUnit | ⬜ | ⬜ | — | — | — | 🟡 default arm (LuaUnit `testFoo` no) | ⬜ `_spec.lua` | ✅ | ⬜ | ⬜ busted `describe`/`it` | No busted/LuaUnit rule |
| php | ✅ phpunit | 🟡 `#[Test]`; fully qualified attribute not matched | 🟡 direct base yes; method-less intermediate base no | ✅ `test*` | ✅ setUp family | ⬜ phpbench | ⬜ `#[DataProvider]`/`@dataProvider` | ✅ | unverified (`Test` is in TEST_ATTRS; PHP attributes are captured) | 🟡 default arm | ✅ `*Test.php` | ✅ | ⬜ | ⬜ Pest `test()`/`it()` | DataProvider targets not rooted; no Pest/Behat/Codeception |
| dart | 🟡 flutter-test gap-only | ⬜ `@isTest` | — | ⬜ | ⬜ `setUp(...)` | ⬜ | — | — | ⬜ | 🟡 default arm | ✅ `_test.dart` | ✅ | ⬜ | 🟡 closures folded into `main()` (calls reach, tests are not nodes) | No harness node. A test file's `main` is rooted as a program entry. |
| scala | 🟡 scalatest, specs2 gap-only | ⬜ no JUnit rule for Scala (Java has one) | ⬜ `AnyFunSuite`/`Specification` | ⬜ | ⬜ beforeAll/afterEach | ⬜ JMH | — | ⬜ | ✅ `@Test` (fixture f3) | 🟡 default arm | ✅ `*Spec.scala`/`*Test.scala` | ✅ `src/test/` | ⬜ | ⬜ class-body `test("x"){}` | ScalaTest class-body tests belong to no callable |
| hcl | ⬜ | — | — | ⬜ `run "x" {}` | — | — | — | — | — | — | ⬜ `.tftest.hcl` | ✅ by code (generic) | ⬜ | ⬜ | No `terraform test` rule |
| zig | 🟡 zig-test gap-only | — | — | — | — | ⬜ `std.testing.fuzz` | — | — | — | 🟡 default arm | ⬜ | ✅ | ⬜ | ⬜ `test "x" {}` not callables | Callees of test blocks are credited as **production** `toplevel:invocation` roots |
| objc | ✅ xctest | — | ⬜ no `XCTestCase` base check (Swift has one) | ✅ `test*` | 🟡 `+setUp`/`-setUp` not told apart | ⬜ `measureBlock:` | — | ⬜ | — | 🟡 default arm | ⬜ no `*Tests.m` rule | ✅ | ⬜ | ⬜ Kiwi/Specta | `test*` in any class of an XCTest-importing file is entered |
| r | 🟡 testthat gap-only | — | — | ⬜ | ⬜ | ⬜ bench/microbenchmark | — | — | — | 🟡 default arm | ⬜ `test-*.R` | ✅ | ⬜ | ⬜ `test_that` blocks | Calls inside `test_that` make nothing live cross-file |
| groovy | 🟡 spock gap-only | ⬜ no JUnit rule for Groovy | ⬜ `Specification` | ⬜ | ⬜ setup/cleanup | ⬜ | — | ⬜ | ✅ `@Test` (fixture f9) | 🟡 default arm (`testFoo` no) | ⬜ `*Test.groovy`/`*Spec.groovy` | ✅ `src/test/` | ⬜ | ⬜ Spock feature methods not extracted | Neither JUnit-in-Groovy nor Spock gets a harness node |
| julia | 🟡 julia-test gap-only | — | — | ⬜ | — | ⬜ BenchmarkTools | — | — | — | 🟡 default arm | ⬜ `runtests.jl` | ✅ | ⬜ | ⬜ `@testset` at top level has no caller | `@testset`/`@test` not enumerated |
| perl | 🟡 test-more gap-only | ⬜ Test::Class `: Test` | ⬜ Test::Class base | ⬜ | ⬜ | ⬜ | — | ⬜ | ⬜ | 🟡 default arm | ⬜ `.t` | 🟡 `t/` missing from TEST_DIRS (fixture `gd`) | ⬜ | ⬜ `subtest` | Conventional `t/*.t` files are production code to cgg |
| elixir | ⬜ | — | — | ⬜ | ⬜ `setup` | ⬜ | — | — | — | 🟡 default arm | ✅ `_test.exs` | ✅ | ⬜ | ⬜ `test "x" do`; qualified calls inside make nothing live | No ExUnit rule |
| erlang | ✅ eunit, common-test | — | — | ✅ `*_test`/`*_test_` (arity unchecked) | ✅ CT `init_per_*`/`end_per_*`/`all` | ⬜ | — | — | — | 🟡 default arm (`_test` suffix not used) | ✅ `_SUITE.erl`, `_tests.erl` | ✅ | ⬜ | 🟡 CT testcases named in `all/0` not entered | Common Test cases are not entered; both gap texts are stale |
| fortran | ⬜ | ⬜ pFUnit `@test` (`.pf` undetected) | — | ⬜ test-drive registration | ⬜ | — | — | — | — | 🟡 default arm (fixture `gd`: `test_a` rooted) | ⬜ | ✅ (fixture `gd`) | ⬜ | — | No pFUnit/test-drive rule |
| clojure | 🟡 clojure-test gap-only | — | — | ⬜ | ⬜ `use-fixtures` | — | — | — | — | 🟡 default arm (kebab names never match) | ⬜ `_test.clj` | ✅ | ⬜ | ⬜ `deftest` not a definition | `deftest` bodies are absent from the graph |
| haskell | ✅ hspec, tasty | — | — | ✅ exact `spec`/`tests`, import-gated | — | ⬜ criterion/gauge | — | — | — | 🟡 default arm (`prop_*` no) | ⬜ `*Spec.hs` | ✅ | ⬜ | ✅ `it`/`describe` calls attributed to `spec` | No QuickCheck `prop_*`, HUnit or `*-discover` |
| ocaml | 🟡 alcotest gap-only | — | — | ⬜ | — | ⬜ Crowbar | — | — | ⬜ `let%test` | 🟡 default arm (fixture `gd`) | ⬜ | ✅ (fixture `gd`) | ⬜ | ⬜ ppx_inline_test/ppx_expect | Alcotest cases not enumerated |
| powershell | 🟡 pester gap-only | — | — | ⬜ | ⬜ BeforeAll/BeforeEach | — | — | — | — | 🟡 **misfires**: every `Test-*` verb function is a test root (fixture `ps`) | ⬜ `*.Tests.ps1` | ✅ | ⬜ | ⬜ `Describe`/`It` (callees credited via toplevel) | `Test-` verb functions are rooted as tests |
| solidity | ⬜ | — | — | ⬜ Foundry `test*`/`testFuzz_*`/`invariant_*` | ⬜ `setUp` (only as public) | ⬜ `testFuzz_*` | — | ⬜ internal helpers of test contracts | — | 🟡 default arm | ⬜ `.t.sol` | ✅ | ⬜ | — | Foundry tests are rooted only through `solidity-public` |
| fsharp | ✅ fsharp-nunit (NUnit + xUnit attributes) | ✅ (fixture `fs`) | — | — | ✅ SetUp/TearDown | ⬜ benchmarkdotnet is C#-only | ⬜ `TestCaseSource` provider by string | ⬜ | ✅ by code (attributes `full`, fixture `fs`) | 🟡 default arm | ⬜ | ✅ | ⬜ | ⬜ Expecto `testCase` values | Expecto is gap-only; unattributed fixture members are reported |
| starlark | ⬜ | — | — | ⬜ `analysistest.make`/`unittest.make` | — | — | — | — | — | 🟡 default arm | ⬜ `_test.bzl` | ✅ (fixture `gd`) | ⬜ | — | No Bazel test rule |
| cmake | ⬜ | — | — | ⬜ `add_test` | — | — | — | — | — | 🟡 default arm | ⬜ | ✅ by code (generic); unverified by fixture | ⬜ | — | CTest `add_test` not recognised |
| nix | ⬜ | — | — | ⬜ `lib.runTests` | — | — | — | — | — | 🟡 default arm | ⬜ | ✅ by code (generic) | ⬜ | ⬜ `nixosTest` `testScript` | No nixosTest/runTests rule |
| verilog | ⬜ | — | — | ⬜ UVM/SVUnit | — | — | — | ⬜ UVM `uvm_test` classes | — | 🟡 default arm (`tb_*`/`*_tb` no) | ⬜ `*_tb.v`/`tb_*.sv` | ✅ by code (generic) | ⬜ | — | Testbench naming `tb_*` not recognised |
| vhdl | ⬜ | — | — | ⬜ VUnit `run("x")` | — | — | — | — | — | 🟡 default arm | ⬜ `tb_*.vhd` | ✅ by code (generic) | ⬜ | ⬜ VUnit `run` branches | Port-less testbench entities not rooted |
| asm | — | — | — | — | — | — | — | — | — | — | — | ✅ by code (generic) | — | — | No test concept |
| smithy | ⬜ | ⬜ `@httpRequestTests`/`@httpResponseTests` traits skipped (smithy.rs:205) | — | — | — | — | — | — | — | — | — | ✅ by code (generic) | — | — | Protocol-test traits not entries |
| proto | — | — | — | — | — | — | — | — | — | — | — | — | — | — | No test constructs |
| graphql | — | — | — | — | — | — | — | — | — | — | — | — | — | — | No test constructs |
| openapi | — | — | — | — | — | — | — | — | — | — | — | — | — | — | No test constructs |
| asyncapi | — | — | — | — | — | — | — | — | — | — | — | — | — | — | No test constructs |
| lean | ⬜ | ⬜ `@[test]` captured as a string, not rooted | — | ⬜ | — | ⬜ Plausible | — | — | ⬜ `@[test]` does not match `test` | 🟡 default arm | ⬜ | ✅ by code (generic) | ⬜ | ⬜ `#guard`/LSpec | No test rules |

### Summary counts (from the matrix)

- **Enumerating harness node (✅)**: rust, python, go, java, kotlin, csharp, ruby, swift, php, erlang, objc, haskell and fsharp. That is 13 of 46.
- **Detect-only gap rules only (🟡)**: javascript, typescript, cpp, dart, scala, zig, r, groovy, julia, perl, clojure, ocaml and powershell. That is 13.
- **No rule but the language has tests (⬜)**: c, bash, lua, hcl, elixir, fortran, solidity, starlark, cmake, nix, verilog, vhdl, smithy and lean. That is 14.
- **n/a (—)**: kivy, asm, proto, graphql, openapi and asyncapi. That is 6.
- **In-file TestRole**: go, python and rust only (`test_defs: true` at go.rs:42, python.rs:50, rust.rs:44).
- **Language-specific `is_test_name` arm**: go, python, ruby and rust (roots.rs:977-986). Every other language uses the generic `test_`/`Test` default arm (roots.rs:987), whether or not that matches its conventions.
- **Language-specific test-file names**: go, python, js/ts, java, kotlin, csharp, ruby, php, swift, dart, elixir, erlang, c/cpp and scala (testfile.rs:76-118). Every other language falls through to `_ => false` (testfile.rs:119) and relies on `TEST_DIRS`.

### Rule inventory (all 53 `TrustKind::Test` rules)

Extracted mechanically from rules.rs (A = attributes, B = base types, M = method patterns). "gap-only" means no A/B/M/registrars.

| Language | Rule (line) | Matchers |
| --- | --- | --- |
| clojure | clojure-test (3350) | gap-only |
| cpp | gtest (7879), cpp-test-macros (7896) | gap-only |
| csharp | xunit (2577) | A `Fact`,`Theory` (also carries gap text) |
| csharp | nunit (2594) | gap-only (superseded by nunit-runner) |
| csharp | nunit-runner (2613) | A `Test`,`TestCase`,`TestCaseSource`,`Theory`, SetUp/TearDown/OneTime*; member sources TestCaseSource/ValueSource (11733) |
| csharp | xunit-data (2642) | member sources MemberData/ClassData (11745) |
| csharp | mstest (7563) | A `TestMethod`,`DataTestMethod`, init/cleanup… |
| csharp | benchmarkdotnet (9402) | A `Benchmark`, Global/Iteration setup and cleanup, `ArgumentsSource`, `ParamsSource` |
| dart | flutter-test (5453) | gap-only |
| erlang | common-test (5861) | M `all`,`suite`,`groups`,`init_per_*`,`end_per_*` (+ stale gap) |
| erlang | eunit (5888) | M `*_test`,`*_test_` (+ stale gap) |
| fsharp | expecto (6165) | gap-only |
| fsharp | fsharp-nunit (6182) | A NUnit + `Fact`/`Theory` (+ stale gap) |
| go | go-testing (7629) | M `Test*`,`Benchmark*`,`Fuzz*`,`Example*`,`TestMain` (+ stale gap) |
| groovy | spock (5699) | gap-only |
| haskell | tasty (5973), hspec (5990) | M `tests` / `spec` |
| java | junit (7057) | A JUnit 4/5; B `TestCase`; M `test*`,`setUp`,`tearDown` |
| java | junit-params (7088) | gap-only |
| java | junit-sources (7444) | A `Parameters`; member sources MethodSource/ArgumentsSource (11721) |
| java | testng (7464), jmh (9428), caliper (9463), cucumber (9489) | A |
| javascript | vitest (8715), jest (8749), playwright (8819) | gap-only |
| julia | julia-test (6363) | gap-only |
| kotlin | junit (7495), jmh (9445), cucumber (9521) | A only (no B, no M, no member sources) |
| objc | xctest (5368) | M `test*`,`setUp`,`tearDown`, no B (+ stale gap) |
| ocaml | alcotest (6063) | gap-only |
| perl | test-more (3237) | gap-only |
| php | phpunit (7026) | A `Test`,`Before`,`After`,…; B TestCase family; M `test*`, setUp family |
| powershell | pester (6466) | gap-only |
| python | py-unittest (6835) | B TestCase family; M `test*`, setUp/tearDown/async variants |
| python | pytest (8380) | A `fixture`,`pytest.fixture`; M `test*` + xunit hooks (+ gap) |
| r | testthat (6249) | gap-only |
| ruby | minitest (6979) | B Minitest/Test::Unit/ActiveSupport; M `test_*`, setup/teardown |
| ruby | rspec (7009) | gap-only |
| rust | cargo-test (9173) | A `test`,`*_test`,`rstest`,`test_case`,`quickcheck` |
| rust | cargo-bench (9191) | A `bench` |
| rust | criterion (9760) | gap-only |
| scala | scalatest (5631), specs2 (5648) | gap-only |
| swift | xctest (5122) | B `XCTestCase`; M `test*`, setUp/tearDown/WithError (+ stale gap) |
| swift | swift-testing (5145) | gap-only |
| typescript | vitest (8732), jest (8773), playwright (8797) | gap-only |
| zig | zig-test (6595) | gap-only |

## Parity gaps

Ranked by impact: ecosystem size × how wrong the output is. "Comparable language" is the one that already has the capability.

1. **JS/TS have no enumerating test harness at all, and test callbacks are not callables. Effort: L.** Java, Python, Go and C# each mint a harness node. JS/TS have only gap rules (vitest/jest/playwright, rules.rs:8715/8749/8819 JS and 8732/8773/8797 TS), and there is no mocha, node:test, ava, jasmine or cypress rule. Calls inside `it(...)` are `no-enclosing-callable` (javascript.md §11 fixture f3; typescript.md §11 fixture f5), so cross-file helpers used only by tests stay dead findings (TS f5 `exportedButUnused`). A fix needs anonymous `it`/`test`/`describe` callbacks to become synthetic callables, plus a registrar-verb rule scoped to test files. The verb set is global per language today (typescript.md §11).
2. **C++ gtest `TEST`/`TEST_F` bodies are extracted and then reported dead. Effort: S–M.** The callables exist (fixture `gt`: findings `TEST`, `TEST_F` and `Fix::SetUp`, all medium never-referenced), but `gtest` (rules.rs:7879) is gap-only. Java's junit and C#'s nunit-runner root the equivalent. A rule that enters callables named `TEST`, `TEST_F`, `TEST_P`, `TYPED_TEST` in gtest-including files, and `SetUp`/`TearDown` on subclasses of `::testing::Test` (B), would close it. Catch2/doctest (`cpp-test-macros`, 7896) need parsing work (L). libFuzzer `LLVMFuzzerTestOneInput` is a fixed name and is not rooted anywhere (grep of `crates/` finds no occurrence; S).
3. **Swift's swift-testing `@Test`/`@Suite` is not rooted, because the Swift plugin captures no attributes. Effort: M.** C#, Java and Kotlin root annotated tests. Fixture `sw`: `addition` (`@Test`) is a medium never-referenced finding, and the capability row shows `attributes: none` (so `TEST_ATTRS` cannot help either). swift-testing is gap-only (rules.rs:5145).
4. **Kotlin lacks Java's `junit-sources` and the JUnit 3 base. Effort: S.** Java roots `@MethodSource("provider")` providers (fixture `jv`: `JTest.provider` rooted by `junit-sources`). Kotlin does not (fixture `kt`: `KTest.provider` never-referenced). `member_sources_for` has no `("junit-sources","kotlin")` arm (rules.rs:11719-11760), and Kotlin `junit` has `base_types: NONE` (rules.rs:7495-7520), where Java has `TestCase` (7080).
5. **Rust `#[cfg(test)]` helpers are reported as high production dead code. Effort: M.** Go and Python tag test-support code by file. Rust's in-file test modules are not tagged, despite the testfile.rs:69-71 comment saying the `#[cfg(test)]` signal handles them (rust.md §11, fixture `fx_crate::tests::test_helper`). This produces false **high** findings, the band the user prioritises.
6. **Scala and Groovy have no JUnit rule, though they run on the same JVM harness as Java/Kotlin. Effort: S for JUnit, L for ScalaTest/Spock.** `@Test` reaches them only through the `builtin:test-attr` root mark (scala.md §11 f3, groovy.md §11 f9), so there is no harness node, and the hooks (`@Before*`) and data sources are missing. ScalaTest/specs2 (5631/5648) and Spock (5699) are gap-only, and their bodies are class-body statements or unextracted feature methods.
7. **DSL-style test blocks have no caller in eleven languages. Effort: L, a shared mechanism.** The languages are Ruby RSpec / AS `test "x" do`, Elixir ExUnit, Clojure `deftest`, Zig `test "x" {}`, R `test_that`, Julia `@testset`, PowerShell Pester, Perl `subtest`, Lua busted, Dart `test()` and Kotlin Kotest. Go and Haskell attribute such closures to the enclosing test (go.md §11 f5 `TestA -> helperT`; haskell.md §11 f3 `Main.spec → Lib.one`). Zig is the worst case: callees are credited as **production** `toplevel:invocation` roots (zig.md §11 p1), and Elixir module-qualified calls inside tests make nothing live (elixir.md §11 f1).
8. **The generic `is_test_name` default arm misfires or misses for 30+ languages. Effort: S.** Only go, python, ruby and rust have arms (roots.rs:977-986). Misfires: PowerShell approved-verb `Test-*` functions are rooted as tests (fixture `ps`: `Test-Thing` → `builtin:test-name`), and C# `TestConnection()`-style production methods are rooted ungated (csharp.md §11, code). Misses: shunit2 `testFoo` (fixture `gd`: `testFoo` reported), LuaUnit `testFoo`, Groovy `testSomething`, Erlang `*_test`, QuickCheck `prop_*`, Verilog `tb_*`, Clojure kebab names. The Go arm is itself over-broad: it is ungated and has no case check, so `Testing` is rooted (go.md §11 f5/f11).
9. **Test-file name rules are missing for 13 languages that have conventions. Effort: S each.** testfile.rs:76-119 has no arm for groovy (`*Test.groovy`/`*Spec.groovy`, though Java/Kotlin/Scala have arms), objc (`*Tests.m`, though Swift has one), haskell (`*Spec.hs`), lua (`_spec.lua`), r (`test-*.R`), perl (`.t`; `t/` is also absent from `TEST_DIRS`, fixture `gd`: `t/basic.t` has no test_role), powershell (`*.Tests.ps1`), solidity (`.t.sol`), clojure (`*_test.clj`), hcl (`.tftest.hcl`), bash (`*_test.sh`, `.bats`), zig, or julia (`runtests.jl`). JS/TS miss `.spec.mjs`, `.spec.cjs`, `.test.mts`, `.test.cts`, `.spec.mts` and `.spec.cts` (testfile.rs:84-96).
10. **In-file `TestRole` exists only for go, python and rust. Effort: M.** Every annotation-driven language (java, kotlin, csharp, fsharp, swift, php, scala) reports `test_tagging: none` (fixtures `cs`, `fs`, `sw`, `gt` capabilities). Annotated test code outside a test-named file or directory is therefore not test scope.
11. **Objective-C XCTest has no `XCTestCase` base check, while Swift has one. Effort: S.** Swift `xctest` has `base_types: &["XCTestCase"]` (rules.rs:5131). The objc rule has none (5368-5384), so any `test*` method in any class of an XCTest-importing file is entered (objc.md §11). Swift fixture f9 shows the base check rejecting a non-XCTestCase `testLooksLike`.
12. **Erlang Common Test testcases are not entered, and EUnit arity is unchecked. Effort: M.** Only the fixed callbacks are entered (rules.rs:5861; erlang.md §11 f1: `case_one/1` not entered). Entering testcases needs reading the list `all/0` returns. Haskell's exact-name approach does not generalise here.
13. **F# lacks BenchmarkDotNet and data sources that C# has. Effort: S.** `benchmarkdotnet` is `language: "csharp"` only (rules.rs:9402). `member_sources_for` has no F# arm, so `[<TestCaseSource("x")>]` providers stay dead. Expecto (6165) is gap-only. This is fsharp.md §11.
14. **PHP `#[DataProvider]` and `@dataProvider` targets are not rooted, unlike Java MethodSource and C# MemberData. Effort: S.** There is no `("phpunit","php")` arm in `member_sources_for` (rules.rs:11719-11760). Also, a fully qualified `#[\PHPUnit\Framework\Attributes\Test]` does not match, and inheritance through a method-less intermediate class is lost (php.md §11 p2/p9).
15. **No C test framework rule. Effort: M.** Unity, cmocka, Check, CUnit and Criterion are all missing (c.md §11). Many register tests through calls (`cmocka_unit_test(fn)`, `RUN_TEST(fn)`). Whether the registrar-argument capture already keeps those functions live is unverified.
16. **Benchmark and fuzz harnesses are inconsistent. Effort: S–M each.** Go roots Benchmark/Fuzz, Java roots JMH/Caliper, and C# roots BenchmarkDotNet. Missing: Rust criterion (gap rule 9760) and cargo-fuzz `fuzz_target!`, Python pytest-benchmark and atheris, Java Jazzer `@FuzzTest`, C++ Google Benchmark and libFuzzer, Haskell criterion, Kotlin/Scala JMH beyond Kotlin's own rule (Scala has none). A grep for `FuzzTest|fuzz_target|LLVMFuzzer|atheris` in `crates/` finds nothing.
17. **Python `test*` does not take `testfoo`; the same applies to the PHP, Swift, ObjC and Java JUnit 3 `test*` patterns. Effort: S (deliberate trade-off).** `name_pattern_matches` requires a non-lowercase character after an alphanumeric-ending prefix (frameworks/mod.rs:570-579). The real runners (unittest, PHPUnit, XCTest, JUnit 3) run any `test…` method. Python's builtin `is_test_name` only catches `test_*`, so `testfoo` in a TestCase is caught by the class-member root but not entered as a case. This is documented as intended ("`Test*` does not take `Testing`", CLAUDE.md), so it is listed as a disclosed divergence, not a bug.
18. **Solidity Foundry tests are rooted only as public functions. Effort: S.** There is no `foundry` rule, so `test*`, `testFuzz_*`, `invariant_*` and `setUp` get no harness node, and `internal` helpers in test contracts stay findings (solidity.md §11 f7).
19. **Not a gap (deliberate): `TEST_CLASS_MEMBERS_ARE_ROOTS` excludes static languages.** rules.rs:11797-11805 says "Statically typed languages (Java, Swift) bind such calls and keep their unused helpers reportable". Fixture `jv`: `JTest.helper` is a high finding. This is consistent with that intent, and is recorded so later agents do not "fix" it.

## Doc corrections

1. **c.md §11 names a flag that does not exist.** It says findings in `test_b.c` are "withheld from findings unless `--show-test-findings` is passed". `cgg --help | grep -c show-test-findings` prints 0. The flag is `--include-tests`.
2. **cpp.md §11 says there is no gtest or Catch2 rule.** It says "No other harness has a rule … No kind-`test` rule mints a harness node for C++". The second half is right, but detect-only kind-test rules `gtest` (rules.rs:7879) and `cpp-test-macros` (rules.rs:7896, doctest/Catch2) exist and report gaps. Separately, the `cpp-test-macros` gap text says "gtest's TEST(Suite, Name) takes identifiers and does parse, which is why gtest is enumerated". gtest is **not** enumerated: its rule has no matchers, and fixture `gt` reports `TEST`/`TEST_F` dead.
3. **csharp.md §11 gives the wrong reason for `[ValueSource]` not being covered.** `ValueSource` is in `member_sources_for` for `nunit-runner` (rules.rs:11740). It is still not rooted, because the attribute sits on a parameter, which is not read. Fixture `cs`: `[Test] void T([ValueSource(nameof(MakeVals))] int x)` leaves `MakeVals` a medium `only-used-by-tests` finding, while `TestCaseSource(nameof(Cases))` on the method roots `Cases`. csharp.md also does not mention the gap-only `nunit` rule (rules.rs:2594), which sits beside `nunit-runner`.
4. **Several docs say a language has no test-file gating.** fortran.md, ocaml.md ("Test-file gating — none for OCaml"), starlark.md ("`tests/` files are analyzed like any other file"), cmake.md, vhdl.md and verilog.md ("not applicable") all say so. The generic `TEST_DIRS` rule applies to every language (testfile.rs:125-131, no language guard). Fixture `gd`: `tests/a.f90` (fortran), `tests/b.ml` (ocaml), `tests/c.bzl` (starlark) and `tests/s.sh` (bash) all get `test_role: directory`. What these languages lack is a *file-name* rule, not gating. (Incidental and unverified: in the same fixture, `tests/CMakeLists.txt` was skipped as `unknown-extension`, which is a §01 detection question.)
5. **powershell.md §11 leaves `*.Tests.ps1` unverified.** It can now be settled from code: `classify_test_file` has no powershell arm (testfile.rs:76-119 falls to `_ => false`). A `*.Tests.ps1` file is test code only under a `TEST_DIRS` directory.
6. **rust.md §11 says "the `proptest` attribute is listed".** It is listed in `TEST_ATTRS` (roots.rs:217) and in `rust_test_role` (rust.rs:3069-3100), but **not** in the `cargo-test` harness attributes (rules.rs:9173: `test`, `*_test`, `rstest`, `test_case`, `quickcheck`). A `#[proptest]` function would get a `builtin:test-attr` root and TestRole, but no harness entry edge. This is from code only; unverified by fixture.
7. **Stale `gap` prose in rules.rs that cgg prints to consumers.** These are not language-doc errors, but they contradict both the code and the docs:
   - `go-testing` (rules.rs:7642) says Benchmark/Fuzz/Example "are not root-marked". They are (roots.rs:977-983; go.md f5).
   - `eunit` (rules.rs:5901) says cgg "has neither a suffix matcher nor a preprocessor, so no EUnit test is enumerable". `*_test` suffix patterns work (frameworks/mod.rs:580-581; erlang.md f1).
   - `common-test` gap says it declines to enumerate the fixed callbacks, but it does enumerate them (erlang.md §11).
   - `xctest` swift (rules.rs:5141) and objc (5381) say "no prefix matcher, so they cannot be enumerated". `test*` prefix patterns enumerate them (frameworks/mod.rs:571-579; fixture `sw`: `AppTests.testGreet` entered).
   - `fsharp-nunit` (rules.rs:6205) says "The fsharp plugin hardcodes `attributes: Vec::new()` … there is not even an attribute stream to read". Fixture `fs` shows `attributes: full` in the capability row and `MyTests.Adds` entered by `<framework-entry>::test::fsharp-nunit`.
   - `cpp-test-macros`: see item 2.
8. **Kotlin junit roots `@Ignore`.** kotlin.md §11 lists `Ignore` among the junit attributes, which is accurate (rules.rs:7518). Rooting `@Ignore` as a test entry is questionable, since an ignored test is not run. This is a rule-quality note, not a doc error.
9. **elixir.md §11 states "None of the 53 `TrustKind::Test` rules … is elixir".** Confirmed: the mechanical extraction above finds 53 rules and none with `language: "elixir"`.

## Evidence index (fixtures run for this matrix)

All fixtures are under `<scratch>/parity/matrix-11/`. Each was run with `target/release/cgg <dir> --dead-code --include-tests --dead-code-format json -o <dir>.out.json`, and the results were read from `<dir>.out.json.deadcode.json` and `.audit.json`.

| Fixture | What it shows |
| --- | --- |
| `gt/tests/a_test.cpp` | gtest `TEST`, `TEST_F`, `Fix::SetUp` reported medium never-referenced; cpp `test_tagging: none`, no root rules active |
| `sw/tests/ATests.swift` | XCTest `testGreet` entered via `<framework-entry>::test::xctest`; swift-testing `@Test addition` and `notATest` reported; swift `attributes: none` |
| `fs/tests/T.fs` | `[<Test>] Adds` entered via `fsharp-nunit`; `NotATest` reported; fsharp `attributes: full` |
| `ps/tests/A.Tests.ps1` | `Test-Thing` rooted `builtin:test-name`; `Get-Other` (called in `It {}`) rooted `toplevel:invocation` |
| `cs/tests/VTests.cs` | `TestCaseSource(nameof(Cases))` roots `Cases`; parameter `[ValueSource(nameof(MakeVals))]` leaves `MakeVals` a finding |
| `kt/src/test/kotlin/KTest.kt` | `@ParameterizedTest p` entered via junit; `@MethodSource("provider")` target `provider` and `testLegacy` reported |
| `jv/src/test/java/JTest.java` | Java `provider` rooted via `<framework-entry>::test::junit-sources`; `helper` high never-referenced |
| `gd/` | generic `tests/` dir gives `test_role: directory` for fortran, ocaml, starlark, bash; `t/basic.t` (perl) has none; `test_a`/`test_x`/`test_bar` rooted `builtin:test-name`, shunit2 `testFoo` reported |
