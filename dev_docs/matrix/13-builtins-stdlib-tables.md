# 13. Builtins & stdlib tables

This category covers the builtin knowledge cgg has about each language, written as data. That means:

- the per-language stdlib name manifests (`crates/cgg-core/src/stdlib/<lang>.txt`, loaded by `stdlib_names`, `crates/cgg-core/src/stdlib.rs:184-258`);
- the three-bucket classifier that uses them (`classify_external`, `crates/cgg-core/src/external.rs:144-310`);
- the resolver guards that keep stdlib names out of by-name binding (step 1d `cross_file.rs:6022`, step 5 `cross_file.rs:6865`, macro-arg retry `cross_file.rs:7080`, Rust std-owner impls `cross_file.rs:6654`);
- the type-annotation tables (`MAYBE_TYPE_WRAPPERS`, `UNION_TYPE_CONSTRUCTORS`, `NULL_TYPE_MEMBERS`, `CALLBACK_TYPE_CONSTRUCTORS`, `TOP_TYPES`, `stdlib.rs:99-155`);
- the cast and construction tables (`CAST_INTRINSICS`, `CONSTRUCTING_METHODS` at `stdlib.rs:35-64`, and `constructs_by_call` at `crates/cgg-lang/src/receiver.rs:46-51`);
- wrapper see-through tables (`crates/cgg-core/src/rust_types.rs`);
- implicit packages (`SCALA_IMPLICIT_PACKAGES`, `stdlib.rs:71-84`);
- constructor and call-operator names (`cross_file.rs:383-401`);
- platform-header tables (`is_platform_header`, `cross_file.rs:3208-3354`);
- plugin-local builtin drop lists;
- what `--include-stdlib` turns into `<stdlib>::…` exit nodes.

The matrix asks one question per language: when code calls something the language or its standard library provides, does cgg recognise it as such? A recognised call goes to the `stdlib` bucket or becomes a `<stdlib>` node. An unrecognised one is either misfiled as `external` or, worse, bound by name to a workspace function.

Sources: the 46 language catalogues (`dev_docs/languages/<lang>.md` §13 and scorecard row 13), the code cited inline, and new fixtures under `<scratch>/parity/matrix-13/`, all run with `target/release/cgg` 0.11.0:

- `py/`: fan-out guard
- `rs/`, `cpp/`: `::` heads and generic receivers
- `tr/<lang>/`: typed receivers for py, go, ts, kt, cs, dart, scala, swift, php, java
- `hd/<lang>/`: qualified heads for cpp, pl, hs, r, ex

## Legend

| Mark | Meaning |
| --- | --- |
| ✅ | full: the mechanism exists for this language and works on the evidence cited |
| 🟡 | partial: exists but with a documented hole, or only seeded from references and never tuned |
| ⬜ | none: the language has the concept, but cgg has no support for it |
| — | n/a: the concept does not exist in the language, or another mechanism makes it moot |
| n/v | not verified: the precondition exists in code, but the per-language effect was not observed |

## Columns

| Column | Meaning | Where it lives |
| --- | --- | --- |
| **List** | a `stdlib/<lang>.txt` manifest exists. Size is the non-empty line count (no list has comments, duplicates or whitespace; checked with `grep -c .` and `sort -u`). T = tuned against corpus external-bucket noise; S = "seeded from language references only" (CHANGELOG.md:5971-5975) | `stdlib.rs:184-258` |
| **Head** | a qualified call (`fmt.Println`, `List.map`, `std::sort`) is screened as stdlib by its receiver head (rule (a)) | `external.rs:175-185, 232-238` |
| **Typed** | a method on a receiver *typed* as a stdlib type (`xs: List<…>`) is booked stdlib (`<stdlib>::List::add`) | plugin receiver typing plus rule (a) |
| **Alias** | an import alias (`import typing as t`, `from fs import readFileSync`) makes the call stdlib (rules (b)/(d)). Only import records of kind `import`/`from-import` feed it | `external.rs:76-127` |
| **Guard** | stdlib names are never bound by the step-1d global fallback or step-5 duck fan-out. This is generic for every language with a list | `cross_file.rs:6022, 6865` |
| **Drop** | a plugin-local builtin list drops names at extraction, before any bucket | per plugin |
| **Annot** | annotation tables used by `declared_type`: maybe-wrappers, union constructors, null members, top types, callback constructors | `stdlib.rs:99-155`, `annotation.rs:29-67` |
| **Cast/Ctor** | a cast or construction written as the receiver types it (`CAST_INTRINSICS`, `CONSTRUCTING_METHODS`, `constructs_by_call`, syntactic `receiver::peel`) | `stdlib.rs:35-64`, `receiver.rs:46-51, 170-215` |
| **Wrap** | deref-transparent wrapper see-through (`Arc<Mutex<T>>` → `T`) | `rust_types.rs:67-114` |
| **Implicit** | an implicit or default-imported package is modelled, so its qualified heads are stdlib | `stdlib.rs:71-96`, `libscope.rs:631-634` |
| **CallOp** | constructor and call-operator method names (`__init__` / `__call__`, `__construct` / `__invoke`, `initialize` / `call`, `constructor`) | `cross_file.rs:383-401` |
| **LangTbl** | other per-language builtin constants: Kotlin scope functions, Perl base pragmas, Rust adapters, Scala root packages, dynamic-module loaders | `stdlib.rs:10-29, 86-96`, `facts.rs:165` |
| **Hdr** | a system/platform header table, so `<stdio.h>` is never followed into a vendored copy | `cross_file.rs:1815-1817, 3208-3354` |
| **Case** | case-insensitive languages are matched case-insensitively. No such code exists: `external.rs` has no lowercase fold (grep `to_lowercase` = 0) | — |
| **Inc** | `--include-stdlib` mints `<stdlib>::…` exit nodes | `synthesize_exit_nodes` |

## The matrix

| # | lang | List | Head | Typed | Alias | Guard | Drop | Annot | Cast/Ctor | Wrap | Implicit | CallOp | LangTbl | Hdr | Case | Inc | notes (most important gap) |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | rust | ✅ 346 T | 🟡 `Vec::new` → stdlib, but `std::fs::read_to_string` / `std::process::exit` → **external** (fixture `rs`; `module_is_stdlib` splits on `.` only) | 🟡 `let v: Vec<u8>; v.len()` → `<external>::Vec<u8>::len` (fixture `rs`) | ⬜ `use … as` not consumed | ✅ | ✅ 31 noise macros (rust.rs:2637-2673) | — (rust_types instead) | 🟡 own path (`settle_assoc_call_locals`, struct literals), no table | ✅ 8 tables (rust_types.rs) | 🟡 `std`/`core`/`alloc` heads always external (cross_file.rs:7138), not modelled as stdlib | — | ✅ `RUST_UNTYPED_ADAPTER_METHODS`, `STD_CORE_TYPES` (cross_file.rs:53) | — | — | ✅ | `std::a::b()` paths are booked external although `std` is in rust.txt |
| 2 | python | ✅ 1180 T | ✅ `os.getcwd` (python.md f11) | 🟡 `xs: list` → `<stdlib>::list::append`, `s: str` → `<external>::s::upper` (fixture `tr/py`; `is_primitive`) | ✅ import + from-import (external.rs:83-120) | ✅ fixture `py`: `o.join()` unresolved, `o.zap()` bound medium | — | ✅ MAYBE ×8, UNION, NULL; ⬜ no TOP (`object`/`Any`) | ✅ by-call (receiver.rs:49) | — | — (builtins via list) | ✅ `__init__` / `__call__` | ✅ `import_module` loader (facts.rs:165) | — | — | ✅ | `str`-typed receivers never classify (language-blind `is_primitive`) |
| 3 | kivy | ⬜ none (no python.txt fallback) | ⬜ | — | — | ⬜ | ✅ `SKIP_CALLEES` 34 (kivy.rs:32-66) | — | — | — | — | — | — | — | — | ⬜ | KV Python expressions never see python.txt; workspace misses show as `<external>` |
| 4 | javascript | ✅ 569 T | ✅ `path.join` (js.md f2) | n/v (no annotations) | ✅ `readFileSync` from `fs` → stdlib (js.md f2) | ✅ | — | 🟡 NULL row only; inert without annotations (stdlib.rs:134) | ✅ `new`, peel (receiver.rs:197-203) | — | — (globals via list) | ✅ `constructor` (no call operator in JS) | ✅ `require` / `importScripts` loaders (facts.rs:165) | — | — | ✅ | literal receivers (`[1,2].map`) are not stdlib |
| 5 | typescript | ✅ 594 T | ✅ `JSON.parse`, `Math.max` (ts.md f17) | 🟡 `m: Map<…>` → `<stdlib>::Map::get`, `s: string` → external (fixture `tr/ts`); `xs: string[]` external (ts.md) | ✅ (shared JsWalker, typescript.rs:12; ts.md f2 `readFile`) | ✅ | — | 🟡 MAYBE (`NonNullable`, `Readonly`), NULL; ⬜ TOP (`any`/`unknown`) | ✅ `new`, `as`, `<T>x` | — | — | ✅ `constructor` | ✅ `require` | — | — | ✅ | lacks `bind`/`call`/`on`/`emit`/… that javascript.txt has (8 names, `comm -23`) |
| 6 | go | ✅ 986 T | ✅ `fmt.Println` (go.md f1) | ✅ `b *strings.Builder` → `<stdlib>::strings.Builder::WriteString` (fixture `tr/go`) | n/v (`import` records emitted, go.rs) | 🟡 over-broad: `Run`/`Do`/`Handle`/`New` block fan-out to workspace methods (go.md f2-f4) | — | 🟡 `any` only through `is_primitive`; no table rows | ✅ type assertion, composite literal (receiver.rs:175-178) | — | — | — | — | — | — | ✅ | list holds common method names, so untyped workspace calls go unresolved |
| 7 | java | 🟡 399 S | ✅ `Collections.emptyList` (java.md f4) | 🟡 `List<Integer>` → `<stdlib>::List::add`, `String s` → external (fixture `tr/java`) | n/v (`import`/`from-import`, java.rs) | ✅ | — | ⬜ no rows (`Object`, `Consumer`/`Function`); see Doc corrections for the effect | ✅ syntax cast/`new` (receiver.rs:175, 197) | — | ⬜ `java.lang` not modelled (`java`/`javax` in the list cover heads only) | — | — | — | — | ✅ | no implicit `java.lang`; list never tuned |
| 8 | kotlin | ✅ 304 T | n/v | 🟡 `MutableList<Int>` → stdlib, `s: String` → external (fixture `tr/kt`); untyped `list.map` external (kotlin.md f1) | n/v (`import`, kotlin.rs) | ✅ | — | 🟡 generic `?` marker only; ⬜ TOP `Any`; `Unit`/`Nothing` via `is_primitive` | ✅ by-call + `as` | — | ⬜ `kotlin.*` / `java.lang.*` default imports; kotlin.txt has `kotlin` but no `java` | ⬜ `operator fun invoke` | 🟡 scope fns `run`/`apply`/`with` only (stdlib.rs:25-29) | — | — | ✅ | default imports and receiver lambdas beyond run/apply/with are not modelled |
| 9 | c | ✅ 278 T | — | — | — | ✅ | — | — | — | — | — | — | — | ✅ `is_platform_header` | — | ✅ | complete for the concept |
| 10 | cpp | ✅ 553 T | 🟡 `std::sort` → stdlib, `std::this_thread::sleep_for` / `std::chrono::…::now` → external (fixture `hd/cpp`) | ⬜ `std::vector<int> v; v.push_back` → `<external>::v::push_back` (fixture `cpp`) | ⬜ `namespace fs = std::filesystem` not consumed | ✅ | — | 🟡 template pointee only (`#deref`) | ✅ 4 `CAST_INTRINSICS` + by-call | 🟡 generic first-template-arg pointee, not a smart-pointer table | — | ⬜ `operator()` | — | ✅ plus `CPP_STDLIB_HEADERS` (detect.rs:333) | — | ✅ | nested `std::` namespaces and methods on stdlib-typed locals are booked external |
| 11 | csharp | 🟡 380 S | ✅ `Path.Combine`, `System.IO.File.ReadAllText` (csharp.md f1; `System` is in csharp.txt) | 🟡 `List<int>` → stdlib, `string s` → external (fixture `tr/cs`) | ⬜ `using X = …` not consumed | ✅ | — | ✅ MAYBE `Nullable`, CALLBACK `Action`/`Func`, TOP `Object`/`Dynamic` | ✅ syntax cast/`as`/`new` | — | ⬜ SDK `ImplicitUsings` | — | — | — | — | ✅ | implicit global usings not modelled; list never tuned |
| 12 | bash | ✅ 170 T | — | — | — | ✅ | ✅ 29 names (bash.rs:164-198), separate from bash.txt | — | — | — | — | — | — | — | — | ✅ | `curl`/`git`/`jq`/`make` counted external, not stdlib (bash.md f6) |
| 13 | ruby | ✅ 792 T | ✅ `JSON.parse` (ruby.md p1) | — (untyped) | — (`require` does not alias) | 🟡 over-broad: `f.call` on a workspace `call` stays unresolved (ruby.md p6) | — | — | ✅ `CONSTRUCTING_METHODS` `new` (stdlib.rs:58) | — | — | ✅ `initialize` / `call` | — | — | — | ✅ | operator and Enumerable names hide workspace methods outside class context |
| 14 | swift | 🟡 249 S | n/v | 🟡 `x: AnyObject` → stdlib (swift.md f5); `xs: [Int]` and `s: String` → external (fixture `tr/swift`) | n/v (`import`, swift.rs) | ✅ | — | 🟡 MAYBE `Optional`/`ImplicitlyUnwrappedOptional`; ⬜ TOP `Any`/`AnyObject` | ✅ by-call + `as` | — | ⬜ `Swift`/`Foundation` heads not in swift.txt | ⬜ `callAsFunction` | — | — | — | ✅ | sugar types (`[T]`, `T?` literals) and literal receivers are not stdlib |
| 15 | lua | 🟡 123 T | ✅ `string.format`; `assert.are.equal` via head `assert` (lua.md p1) | — | ⬜ `local s = require "string"` | ✅ | — | — | — | — | — | — | — | — | — | ✅ | `loadstring`, `bit32`, LuaJIT `bit`/`ffi`/`jit` missing |
| 16 | php | ✅ 324 T | 🟡 `\strlen` → `<external>::\strlen` (php.md p5) | ✅ `ArrayObject $a` → `<stdlib>::ArrayObject::append` (fixture `tr/php`) | ⬜ `use function` not consumed | ✅ | — | 🟡 NULL row (stdlib.rs:135) | ✅ `new` peel | — | 🟡 global-namespace fallback (libscope.rs:2222-2229) | ✅ `__construct` / `__invoke` | — | — | ⬜ `STRLEN` → external (php.md p3), though the in-tree lookup folds case (libscope.rs:2222) | ✅ | case and the leading `\` defeat the screen |
| 17 | dart | ✅ 394 T | ✅ `Uri.parse` (dart.md f2) | 🟡 `List<int>` → stdlib, `String s` → external (fixture `tr/dart`) | ⬜ `import … as` not consumed | ✅ | — | ✅ CALLBACK `Function`, TOP `Object`/`Dynamic` | ✅ by-call + `as` | — | 🟡 `dart:core` implicit (libscope.rs:631-634), not a table | ⬜ `call()` method | — | — | — | ✅ | `group` (package:test) mis-bucketed as stdlib |
| 18 | scala | 🟡 248 S | n/v | 🟡 `List[Int]` → stdlib, `String` → external (fixture `tr/scala`) | n/v (`import`, scala.rs) | ✅ | — | ✅ NULL `Null`/`Nothing`, TOP `Any`/`AnyRef`/`AnyVal` | ✅ `asInstanceOf` + by-call | — | ✅ `SCALA_IMPLICIT_PACKAGES` ×12, `SCALA_ROOT_PACKAGES` | ⬜ `apply` | ✅ root packages | — | — | ✅ | list never tuned; receiver text leaks into `<external>` names |
| 19 | hcl | 🟡 125 T | — | — | — | ✅ | — | — | — | — | — | — | — | — | — | ✅ | Terraform ≥1.5 functions and `provider::` functions are external |
| 20 | zig | ✅ 160 T | ✅ `std.debug.print` (zig.md p1) | ✅ `b: *std.Build` → `<stdlib>::std.Build::addModule` (zig.md p1x) | n/v (`import`, zig.rs) | ✅ | — | 🟡 `?`/`*` peel only (type_hints.rs:2708-2721) | 🟡 own (`T.init()` via `type_locals`, zig.md:116-117) | — | — | — | — | — | — | ✅ | an in-tree `std` (compiler repo) takes over, which is correct |
| 21 | objc | ✅ 408 T | ✅ `[NSString stringWithFormat:]` (objc.md f5) | n/v | — | ✅ | — | ⬜ `nullable`/`id` | ⬜ `[[X alloc] init]` not typed (objc.md:344) | — | — | — | — | 🟡 shares C's table; `<UIKit/UIKit.h>` not listed (cross_file.rs:3346-3353) | — | ✅ | C-function entries unused because C calls are not recorded |
| 22 | r | 🟡 321 S | ⬜ `stats::median`, `base::paste` → external (r.md f2; fixture `hd/r`) | — | ⬜ `library()` | ✅ | ✅ 26 names (r.rs:152-183) | — | — | — | — | — | — | — | — | ✅ | `pkg::fn` heads never screen; `base`/`stats`/`utils` not listed |
| 23 | groovy | ✅ 339 T | ✅ `Math.max` (groovy.md f12) | 🟡 `List` param → `<stdlib>::List::each`; literal receivers external (groovy.md f12) | n/v (import + from-import, groovy.rs) | ✅ | — | ⬜ | ✅ peel | — | ⬜ default imports (`java.*`, `groovy.*` absent from groovy.txt) | ⬜ `call` | — | — | — | ✅ | default imports not modelled |
| 24 | julia | 🟡 248 S | ✅ `LinearAlgebra.dot`, `Base.length` (julia.md p1) | — | ⬜ `using LinearAlgebra: norm` → `norm` external (julia.md p1) | ✅ | — | — | — | — | 🟡 `Base`/`Core` in the list only | — | — | — | — | ✅ | `using M: f` imports do not screen |
| 25 | perl | ✅ 167 T | ⬜ `File::Spec->catfile`, `POSIX::floor` → external (fixture `hd/pl`) | — | ⬜ `use POSIX qw(floor)` → `floor` external (perl.md f2) | ✅ (moot for step 5, perl.md §05) | — | — | ⬜ `Pkg->new` not a construction (perl.md:182) | — | — | — | ✅ `PERL_BASE_PRAGMAS` / `PERL_BASE_KEYWORDS` | — | — | ✅ | `::` package heads never screen, so the listed module exports only match bare |
| 26 | elixir | 🟡 255 T | ✅ `Kernel.apply`, `Enum.map` (fixture `hd/ex`), `:ets.new` (elixir.md f1) | — | ⬜ `alias` not consumed | ✅ | — | — | — | — | ✅ `Kernel` in the list | — | — | — | — | ✅ | special forms `case`/`with`/`cond`/`raise`/`receive` go external |
| 27 | erlang | ✅ 774 T | ✅ `lists:map`, `gen_server:start_link` (erlang.md f1) | — | n/v (`import`, erlang.rs) | ✅ | — | — | — | — | ✅ `erlang` module in the list | — | — | — | — | ✅ | `eunit`, `cowboy_req` absent; no `m:f` granularity |
| 28 | fortran | 🟡 163 S | — | — | — | ✅ | — | — | — | — | — | — | — | — | ⬜ `SQRT`, `Abs` → external (fortran.md h) | ✅ | case-sensitive match on a case-insensitive language |
| 29 | clojure | 🟡 367 T | 🟡 `str/join` only as a literal entry; `clojure.string/join`, `s/join` external (clojure.md p2) | — | ⬜ `:as` aliases | ✅ | — | — | — | — | ⬜ `clojure.core` head not listed | — | — | — | — | ✅ | `/`-qualified calls only match conventional aliases |
| 30 | haskell | ✅ 365 T | n/v (fixture `hd/hs`: `Prelude.map`, `Map.toList` recorded nothing) | — | ⬜ `import qualified … as` | ✅ | — | — | — | — | 🟡 Prelude names listed, but `Prelude` as a head is not | — | — | — | — | ✅ | qualified calls not observed (category 03/04 issue) |
| 31 | ocaml | 🟡 116 S | ✅ `List.map`, `Printf.printf` (ocaml.md f1) | — | ⬜ `module M = …` | ✅ | — | — | — | — | ✅ `Stdlib` in the list | — | — | — | — | ✅ | `Base`/`Core`/`Lwt` external; list never tuned |
| 32 | powershell | ⬜ | ⬜ | ⬜ | — | ⬜ | ⬜ | — | ⬜ `[T]::new` | — | — | — | — | — | ⬜ (cmdlets are case-insensitive) | ⬜ | all cmdlets are external (powershell.md f5) |
| 33 | solidity | ⬜ | ⬜ | ⬜ | — | ⬜ untyped `total.add` fans out to `Math.add` (solidity.md f1) | — | — | ⬜ `new C()` | — | — | — | — | — | — | ⬜ | `require`/`keccak256`/`abi.*` external, and no fan-out guard |
| 34 | fsharp | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | — | ⬜ | ⬜ (no `peel`, fsharp.md §13) | — | ⬜ `FSharp.Core` auto-open | — | — | — | — | ⬜ | `printfn`, `List.map` external (fsharp.md f1/f8) |
| 35 | starlark | ⬜ (no python.txt fallback) | ⬜ | — | — | ⬜ | — | — | — | — | — | — | — | — | — | ⬜ | `print`/`len`/`struct`/`depset`/`native.*` external (starlark.md t/) |
| 36 | cmake | ⬜ | — | — | — | ⬜ | — | — | — | — | — | — | — | — | ⬜ (commands are case-insensitive) | ⬜ | `message`/`set`/`add_library` external (cmake.md t1) |
| 37 | nix | ⬜ | ⬜ `builtins.*` | — | — | ⬜ | — | — | — | — | ⬜ `builtins` scope | — | — | — | — | ⬜ | every builtin and `lib.*` call is external (nix.md f1-f3) |
| 38 | verilog | ⬜ | — | — | — | ⬜ | — | — | — | — | — | — | — | — | — | ⬜ | `$display`/`$finish` system tasks external (verilog.md b) |
| 39 | vhdl | ⬜ | — | — | — | ⬜ | — | — | ⬜ conversions `unsigned(x)` | — | ⬜ implicit `std`/`work` | — | — | — | ⬜ | ⬜ | `ieee`/`numeric_std` unknown; impact limited (few call sites) |
| 40 | asm | ⬜ (libc calls could use c.txt) | — | — | — | ⬜ | — | — | — | — | — | — | — | — | — | ⬜ | registers and labels (`rax`, `1b`) minted as `<external>` (asm.md f1) |
| 41 | smithy | 🟡 in-plugin `PRELUDE` 21 (smithy.rs:38-60) | — | — | — | — | ✅ prelude dropped | — | — | — | ✅ prelude is the implicit namespace | — | — | — | — | — | later prelude shapes unverified |
| 42 | proto | ⬜ | — | — | — | — | — (scalars separated by the grammar) | — | — | — | ⬜ well-known `google.protobuf.*` | — | — | — | — | ⬜ | `Timestamp`/`Any`/`Empty` external (proto.md a) |
| 43 | graphql | 🟡 5 `BUILTIN_SCALARS` (graphql.rs:23) | — | — | — | — | 🟡 scalars only | — | — | — | — | — | — | — | — | — | built-in directives and `__Schema`/`__Type` not filtered |
| 44 | openapi | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | built-in types are never `$ref`s; remote meta-schema refs external |
| 45 | asyncapi | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | as openapi |
| 46 | lean | ⬜ | ⬜ `Nat.add_zero`, `List.map`, `IO.println` external (lean.md proj) | — | — | ⬜ | 🟡 structural tactic-name skip plus 6 modifiers (lean.rs:100-105, 557-562) | ⬜ `Option`/`Except` | — | — | ⬜ `Init`/Prelude | — | 🟡 tactic modifiers | — | — | ⬜ | all core calls external (`0 stdlib, 18 external`) |

Counts: 30 languages have a manifest (21 T, 8 S, plus Rust, which neither CHANGELOG list names). 16 have none: asm, asyncapi, cmake, fsharp, graphql, kivy, lean, nix, openapi, powershell, proto, smithy, solidity, starlark, verilog, vhdl. Of those, graphql and smithy have an in-plugin substitute, and openapi and asyncapi do not need one.

## Parity gaps

The ranking weighs how much call-graph output is wrong, how many languages are affected and how cheap the fix is.

1. **`::`, `/` and `\` qualified heads are never split for the stdlib screen.** Affects Rust, C++, Perl, R, Clojure and PHP. Python/Go/OCaml/Julia/Elixir/Lua/Zig dotted heads do screen.
   - Cause: `module_is_stdlib` (external.rs:175-185) tests the whole receiver, then its first `.` segment only.
   - Evidence:
     - Rust `std::fs::read_to_string` and `std::process::exit` → `<external>` although `std` is in rust.txt (fixture `rs`).
     - C++ `std::this_thread::sleep_for` → external, while `std::sort` (receiver exactly `std`) is stdlib (fixture `hd/cpp`).
     - Perl `File::Spec->catfile` and `POSIX::floor` → external (fixture `hd/pl`).
     - R `stats::median` and `base::paste` → external (fixture `hd/r`, r.md f2).
     - Clojure `clojure.string/join` → external (clojure.md p2).
     - PHP `\strlen` → external (php.md p5).
   - Effort: **S**. Split on `::`, `/` and `\` per language, and strip a leading `\`.
2. **Ten languages with real builtins have no manifest, so they also get no fan-out guard.** F#, Solidity, PowerShell, Nix, CMake, Starlark, Lean, Kivy, Verilog (`$` system tasks) and asm.
   - Evidence: the fsharp, solidity, powershell, nix, cmake, starlark, lean, kivy, verilog and asm docs, §13 each.
   - Solidity shows the cost: untyped `total.add(amt)` fans out to a workspace `Math.add` (solidity.md f1). In the other languages a user function named like a builtin captures every call by name (step 1d skips only `stdlib_names` hits, cross_file.rs:6022).
   - Cheapest wins: Kivy and Starlark could reuse python.txt, and asm could reuse c.txt.
   - Effort: **S** per language (a `.txt` plus two lines in stdlib.rs).
3. **String-typed receivers never classify as stdlib in any language.**
   - Evidence (fixture `tr/*`): `s.upper()` (Python `str`), `s.toUpperCase()` (Java/TS/Dart/Scala), `s.uppercase()` (Kotlin), `s.ToUpper()` (C#) and `s.uppercased()` (Swift) all → `<external>::s::…`. In the same fixtures, `List`/`Map`/`MutableList` receivers → `<stdlib>`.
   - Cause: the language-blind `is_primitive` table (type_hints.rs:2724-2763: `str`, `String`, `string`, `number`, `any`, `object`, `Int`, `Unit`, `Nothing`, …) keeps these types off the receiver, and rule (e) (external.rs:271-295) then refuses the bare method name.
   - Effort: **M**. Type the receiver for classification only, without enabling binds.
4. **Implicit or default-imported packages are modelled only for Scala.**
   - Scala has `SCALA_IMPLICIT_PACKAGES` (stdlib.rs:71-84). Dart (`dart:core`, libscope.rs:631) and PHP (global namespace) have ad-hoc equivalents.
   - Missing:
     - Java `java.lang` (java.md §13)
     - Kotlin `kotlin.*` / `java.lang.*` (kotlin.md §13; kotlin.txt has no `java`)
     - Groovy default imports (groovy.txt has neither `java` nor `groovy`)
     - C# `ImplicitUsings`
     - F# `FSharp.Core`
     - Swift `Swift` / `Foundation` (absent from swift.txt)
     - Haskell `Prelude` (absent from haskell.txt)
     - Clojure `clojure.core` (absent)
     - Lean `Init`
     - VHDL `std` / `work`
   - Effort: **M**.
5. **The import-alias screen reads only `import` / `from-import` records** (external.rs:83-120). Plugins emitting those kinds: python, javascript (and TS through JsWalker), java, groovy, go, kotlin, scala, swift, zig, erlang, nix, solidity. Every other aliasing form misses:
   - Perl `use POSIX qw(floor)` → `floor` external (perl.md f2)
   - Julia `using LinearAlgebra: norm` → external (julia.md p1)
   - Clojure `:as`, so `s/join` is external (clojure.md p2)
   - unverified: Rust `use … as`, C# `using X =`, Elixir `alias`, Haskell `qualified … as`, Dart `as` and OCaml `module M =` (their import kinds are not the two the screen reads)
   - Effort: **M**. Normalise each plugin's import records into the two `FileAliases` maps.
6. **Generic and sugar receiver types are not normalised before the screen.**
   - Rust `Vec<u8>` keeps its arguments → `<external>::Vec<u8>::len` (fixture `rs`). Java `List<Integer>` → `List`, which works.
   - C++ `std::vector<int> v; v.push_back` keeps the raw `v` → external (fixture `cpp`).
   - Swift `[Int]` → `<external>::xs::map` (fixture `tr/swift`).
   - TS `string[]` → external (typescript.md §13).
   - Effort: **S–M**.
7. **Case-insensitive languages are matched case-sensitively.** No case folding exists in external.rs.
   - Fortran `SQRT` → external (fortran.md h). This misfiles all fixed-form intrinsics.
   - PHP `STRLEN` → external (php.md p3), although PHP's own in-tree function lookup folds case (libscope.rs:2222-2229).
   - PowerShell, CMake and VHDL will need the same once they get manifests.
   - Effort: **S**.
8. **Call-operator names exist only for Python, PHP and Ruby** (cross_file.rs:394-401). Kotlin `operator fun invoke`, Scala `apply`, C++ `operator()`, Swift `callAsFunction`, Groovy `call` and Dart `call` have the same concept with no entry. Effort: **S** (table rows; step 7 is a pure addition).
9. **Construction-as-receiver-type is missing where the construction is a method or cast, not syntax.**
   - Perl `Pkg->new` (perl.md:182, 377)
   - ObjC `[[X alloc] init]` (objc.md:344)
   - PowerShell `[T]::new`
   - Solidity `new C()` / `C(addr)`
   - F#, which does not call `peel` at all
   - Ruby has `CONSTRUCTING_METHODS` `new` (stdlib.rs:58), so the Perl gap is directly comparable.
   - Effort: **M**. Perl and ObjC plugins must route receivers through `receiver::peel`.
10. **Top-type and callback tables cover only Scala, C# and Dart** (stdlib.rs:143-155). Java (`Object`, `Consumer`/`Function`), Kotlin (`Any`), Swift (`Any`/`AnyObject`), TS (`any`/`unknown`) and Python (`object`/`Any`, `Callable`) have no rows.
    - The rows alone would do nothing. Their only consumer, `settle_closure_args` → `concrete_class` (type_hints.rs:1115, 1236, 1356-1363), reads `closure_args`. Only the csharp, dart and scala plugins record those (closure_types.rs:157, the sole users of `closure_types::`).
    - Effort: **L**. This is really a category-05 gap.
11. **Over-broad manifests suppress legitimate fan-out.** The guard (cross_file.rs:6865) is generic, so any common method name in a list blocks duck binding to workspace methods:
    - Go `Run`/`Do`/`Handle`/`New` (go.md f2-f4)
    - Ruby `call` (ruby.md p6)
    - Dart `group` mis-buckets package:test (dart.md §13)
    - Python `join`: fixture `py`, where `o.join()` is unresolved while `o.zap()` binds
    - Effort: **M**. Split "stdlib method vocabulary" from "never fan out".
12. **Manifest content holes:**
    - Elixir `case`/`with`/`cond`/`raise`/`receive`/`__MODULE__`/`:erlang` (elixir.md §13)
    - HCL `strcontains`/`templatestring`/… (hcl.md §13)
    - Lua `loadstring`/`bit32`/LuaJIT (lua.md p1-p2)
    - Fortran MPI/OpenMP (fortran.md §13)
    - Clojure `eval`/`requiring-resolve`/`comment` (clojure.md p1)
    - R `base`/`stats`/`utils` package names
    - Haskell `Prelude`
    - TS lacks 8 names that JS has (`addListener`, `bind`, `call`, `close`, `emit`, `on`, `once`, `removeListener`; `comm -23`)
    - Effort: **S** each.
13. **Eight manifests have never been tuned against corpus noise:** csharp, fortran, java, julia, ocaml, r, scala and swift (CHANGELOG.md:5971-5975). Effort: **M**. It needs a corpus external-bucket sweep, bounded per CLAUDE.md.
14. **ObjC platform headers.** `is_platform_header` lists C/POSIX/Windows names only. `<UIKit/UIKit.h>` and `<Foundation/Foundation.h>` contain a `.` and match no prefix (cross_file.rs:3346-3353), so they are followed when a copy is in the tree. C and C++ are protected for the same scenario. Effort: **S**.
15. **Descriptor builtins:**
    - proto well-known `google.protobuf.*` types are external (proto.md a)
    - GraphQL built-in directives and introspection types are unfiltered (graphql.md §13)
    - OpenAPI remote meta-schema refs are external (openapi.md f)
    - Effort: **S**.
16. **`--include-external` mislabels workspace misses (cross-cutting).** Rule 2 (external.rs:301-306) sends any site whose receiver is not a known name to `external`, so untyped workspace calls become `<external>::recv::m`. Seen in:
    - Go `<external>::s::Handle` (go.md f1)
    - C# `<external>::_repo::Save` (csharp.md f1)
    - Python `<external>::o::m` (python.md f11)
    - Kivy `<external>::root::missing_method` (kivy.md a)
    - Java `<external>::v::run` (java.md f1)
    - Effort: **M**. Give it a distinct bucket or label.

## Doc corrections

- **csharp.md §13**: the doc says "unverified whether `System` itself is listed". It is: `grep -x System crates/cgg-core/src/stdlib/csharp.txt` matches.
- **lua.md §13 and scorecard**: they say "123 lines, ~122 names" and "122-name". The list has **123** names (`grep -c .` = 123, no duplicates, no blank lines).
- **lua.md §13**: "Which screen rule fired [for `assert.are.equal`] was not checked." It is rule (a): receiver `assert.are`, whose first `.` segment `assert` is in lua.txt (external.rs:175-185, 232-238).
- **objc.md §13**: "Whether Apple framework headers like `<UIKit/UIKit.h>` are classified as platform is unverified." From the code, they are **not**. The path contains `.`, no `PREFIXES` entry matches and it is not in `NAMES` (cross_file.rs:3209-3353). The header is followed when an in-tree copy exists.
- **go.md §13** presents the step-5 stdlib-method skip as Go behaviour. It is generic: `cross_file.rs:6865` applies to every language with a manifest. Fixture `py` shows the same suppression for Python `join`.
- **java.md §13**: "`Object` is not a top type for Java … The effect is unverified." From the code, there is **no effect**. `TOP_TYPES` is read only by `concrete_class` inside `settle_closure_args` (type_hints.rs:1236, 1356-1363), which runs only on `closure_args` recorded by the csharp, dart and scala plugins (closure_types.rs:157). The same applies to the missing TOP/CALLBACK rows noted in python.md, swift.md, typescript.md and kotlin.md.
- **rust.md §13** says "`std`, `core` and `alloc` heads are always external (step 4c)". That describes resolution, but the doc omits the bucket outcome: `std::fs::read_to_string` and `std::process::exit` are booked **external**, not stdlib, although `std` is in rust.txt (fixture `rs`). Only single-segment owners (`Vec::new`, `Box::new`) reach `<stdlib>`.
- **perl.md §13** lists `File::Spec` `catfile` etc. as covered. Only a bare `catfile` (imported) screens; `File::Spec->catfile(...)` is external (fixture `hd/pl`).
- **cpp.md §13**: `CAST_INTRINSICS` is at stdlib.rs:35-46, not 35-44. The fixture claim that `std::sort` is stdlib holds only because the receiver is exactly `std`. Deeper namespaces are external (fixture `hd/cpp`).
- **All 46 docs** omit a category-13 table: `is_primitive` (type_hints.rs:2724-2763), a language-blind builtin type list that stops `String`/`str`/`string`/`any`/`object`/`Int`/`Unit`/`Nothing`/… from typing a receiver. It is why string-typed receivers are external in every language (fixture `tr/*`).
- **csharp.md, kotlin.md, dart.md and swift.md** list "no `rust_types`-like wrapper table for `Task<T>` / `Lazy<T>` / `Future<T>` / `StateFlow`" as a gap. These are not deref-transparent wrappers: a method called on a `Task<Foo>` runs `Task`'s method. The `MAYBE_TYPE_WRAPPERS` doc comment (stdlib.rs:99-103) deliberately keeps containers out, so reading `Task<Foo>` as `Task` is correct here. Await or `.Value` peeling belongs to category 05. In the matrix, Wrap is "—" for these languages.
- **Scorecard grading is inconsistent:**
  - ocaml.md grades row 13 "full", but ocaml.txt is one of the eight lists "seeded from language references only" (CHANGELOG.md:5974).
  - javascript.md grades "full" while typescript.md grades "partial", although TS has strictly more tables (MAYBE + NULL vs NULL only).
  - graphql.md grades "minimal" and smithy.md "partial" for the same in-plugin-list design.
- **bash.md, r.md, kivy.md and smithy.md** drop-list counts are correct: 29 (bash.rs:164-198), 26 (r.rs:152-183), 34 (kivy.rs:32-66) and 21 (smithy.rs:38-60) names.
- **php.md §13** should note an internal inconsistency rather than only the case-sensitive screen: PHP's own function lookup tries a lowercased name (libscope.rs:2222-2229), so in-tree resolution folds case while the stdlib screen does not.
