# 08. Macros, preprocessor & codegen

**Category definition.** This category covers what cgg does with code that does not appear literally in the source as it will run. That includes:

- textual and syntactic macros: Rust `macro_rules!`, C/C++/ObjC/Fortran/asm `#define`, Erlang `-define`, Verilog `` `define ``, Elixir/Clojure/Julia/Lean/Scala/Swift macros, CMake `macro()`;
- preprocessor and conditional compilation: `#if`/`#ifdef`, `#[cfg]`, `-ifdef`, `` `ifdef ``, reader conditionals, cppo, VHDL-2019 conditional analysis;
- quote/unquote templates: Elixir `quote`/`unquote`, Clojure syntax-quote `~`, Julia `quote … $(…)`, F# `<@ … @>`, R `quote()`, Haskell quasi-quotes, Scala 3 `'{…}`/`${…}`;
- compile-time code generation that the language defines: derives, decorators that synthesise members, annotation processors, AST transforms, source generators, Template Haskell, ppx, `use`/`__using__`, Zig type functions, Verilog `generate`, YAML anchors/merge keys;
- how generated code is tagged (`macro:<name>`) and what that tag changes downstream: kept out of by-name fan-out (`crates/cgg-resolve/src/cross_file.rs:1026-1032`), kept out of implicit-trait rooting (`crates/cgg-resolve/src/deadcode/roots.rs:849`), capped at medium in dead code (`Evidence::MacroGenerated`, `crates/cgg-resolve/src/deadcode/mod.rs:893-897`);
- `FileFacts::name_mentions`, names that cgg cannot turn into edges and that only the dead-code report reads.

Ordinary generics (type arguments stripped from call names) are category 03. Runtime `eval`/`load`/reflection is category 15. Rooting through attributes is category 12.

**Sources.** Section `## 08. Macros, preprocessor & codegen` and Summary-scorecard row 08 of all 46 `dev_docs/languages/<lang>.md`. I re-read the shared machinery: `crates/cgg-lang/src/plugins/c_macro.rs`, `rust.rs:1448-1852`, `c.rs:120-135`, `cpp.rs:200-210, 627-679`, `crates/cgg/src/lib.rs:524-533`, `crates/cgg-resolve/src/deadcode/roots.rs:616-705`, `type_hints.rs:172-195`, `csharp.rs:66, 678-729`, `elixir.rs:236, 323-328`, `clojure.rs:108-109, 218-222`. All fixtures below ran with `target/release/cgg` 0.11.0 under `<scratch>/parity/matrix-08/`.

| fixture | input (abridged) | result |
| --- | --- | --- |
| `rs` | `macro_rules! mk {($n:ident)=>{fn $n(){}}}`, `macro_rules! rep {($($n:ident),*)=>{$(fn $n(){})*}}`, `mk!(gen_a); rep!(rep_a, rep_b);`, `fn user(){gen_a(); rep_a();}` | `crate::gen_a` minted with attribute `macro:mk`, `user → gen_a` **high**. `rep_a` is not minted: the site goes to `external_calls` with `no-candidate-in-file`. `mk` and `rep` are callables with attribute `macro`. |
| `rs2` | `fn helper(){}`, `macro_rules! call_it {()=>{helper()}}`, `fn user(){call_it!();}` | `user → call_it` is an edge. There is **no** `call_it → helper` edge. `--dead-code` reports `crate::helper` at **high** (`no-incoming-edges, language-signals-complete, no-unresolved-site-with-name, private-visibility`). |
| `c` | `#define FN_DEF(n,b) int n(void){return b;}`, `#define LIST(X) X(alpha) X(beta)`, `#define DECL(n) …`, `FN_DEF(made_a,1)`, `LIST(DECL)`, `user` calls `made_a()` and `beta()` | `made_a` minted with `macro:FN_DEF`, `user → made_a` **high**. `beta` is not generated (no nested expansion): the site goes to `external_calls` with `no-candidate-in-file`. |
| `c2`, `cpp2` | `void do_log(int){}`, `#define LOG(x) do_log(x)`, `void k(){LOG(1);}` (as `.c` and as `.cpp`) | `k → LOG` is an edge. `LOG → do_log` is **not** an edge in either language. In C, dead code gives `do_log` **medium**, with only generic `language-lacks-*` evidence and nothing macro-specific. |
| `c3` | `#define SQ(x) ((x)*(x))`, `int k(void){return SQ(f(2));}` (as `.c` and `.cpp`) | `k → SQ` and `k → f`: calls inside a macro argument are recorded in both languages. |
| `objc`, `objc3`, `objc4` | `#define MKFN(n) void fn_##n(void){}`, `#define SQ(x) …`, `MKFN(one)`, `void user(void){fn_one(); SQ(2); ping();}` | Only `user` is a callable. Its `start_line` is 3, the `MKFN(one)` line, so the invocation was absorbed into its span. The file has `parse_status: error`. No `#define` callable is minted, and no site is recorded at all, because objc.rs has no `call_expression` arm (objc4: `void ping(void){} void user(void){ping();}` gives 0 edges). |
| `hs` | `{-# LANGUAGE CPP #-}`, `#if FOO / f x = g x / #else / f x = h x / #endif` | Only one `A.f` (line 4) is extracted, with `A.f → A.g` **high**. The `#else` branch, and therefore `f → h`, is lost. |
| `jl` | `macro twice(ex) quote $(esc(ex)); $(esc(ex)) end end`, `function user() @twice helper() end` | `twice` is not a callable, and the site `twice` is `external_calls`/`no-candidate-in-file`. `user → helper` is **high**, so calls in macro arguments are recorded. |
| `erl` | `-define(CALL(X), target(X)).`, `u() -> ?CALL(1).` | No `u → target` edge. `target` is an unresolved site with `src: null` and `no-candidate-in-file`/`no-enclosing-callable`, which comes from the macro body. |
| `cl` | `(defmacro my-macro [] \`(inner))`,`(defn uses [] (my-macro))` | `uses → my-macro` and `my-macro → inner`. |
| `ex` | `defmacro mk(name) do quote do def unquote(name)(), do: helper() end end`, `def uses, do: mk(:x)` | `uses → mk` and `mk → helper`. No `unquote` def is minted. |
| `cm` | `function(helper)`, `macro(my_mac) helper() endmacro()`, `function(user) my_mac()` (first as `CMakeLists.txt`, then as `m.cmake`) | As `CMakeLists.txt`: `1 skipped (unknown-extension)`. As `.cmake`: `user → my_mac` and `my_mac → helper`. `my_mac` has no attributes. |
| `swift`, `swift2` | `macro stringify<T>(…) = #externalMacro(…)`; `#expect(helper() == 1)`; `_ = #stringify(helper())`; `@Test func adds(){ let c = Calc(); #expect(c.add(1,2) == 3) }` | The `macro` declaration is not a callable. `user → helper`, `user2 → helper`, `NewTests.t → helper` and `adds → Calc.add` (medium, `?`) are all edges, so calls inside `#expect(…)` **are** recorded. |

## Legend

| mark | meaning |
| --- | --- |
| ✅ | full: the capability works for the language's normal forms |
| 🟡 | partial: works for a subset, or an approximation with known wrong results |
| ⬜ | none: the language has the concept and cgg does nothing with it |
| — | n/a: the concept does not exist in the language |
| `?` suffix | the mark is inferred from code or docs but was not confirmed by a test or fixture (unverified) |

## Columns

| column | capability |
| --- | --- |
| **Def** | Macro definitions are extracted as callables (`#define F(…)`, `macro_rules!`, `defmacro`, CMake `macro()`). |
| **Call** | A macro invocation is recorded as a site and binds to that definition. |
| **Body** | Calls written in a macro's definition body or template are recorded as edges from the macro. |
| **Args** | Calls written inside a macro invocation's arguments are recorded. |
| **Expand** | Item-position invocations are expanded and the definitions they produce are minted as callables. |
| **XFile** | Expansion can use a macro defined in another file (header or include closure, `#[macro_export]`, `.hrl`). |
| **Depth** | Repetition and nested or rescanned expansion: Rust `$(…)*`, C `X(…)` X-macros, a macro calling a macro. |
| **Tag** | Generated callables are tagged `macro:<name>`, with downstream handling: no by-name fan-out, no implicit-trait rooting, dead findings capped at medium. |
| **Root** | A macro invoked outside any function becomes a dead-code root (`toplevel:macro-invocation`, or an equivalent root through the generic `toplevel:invocation`). |
| **Ment** | `FileFacts::name_mentions` is populated: identifiers in unexpanded macro arguments, attribute strings, and similar. |
| **Alias** | Object-like macros, aliases or anchors are substituted or followed: `#define A f`, `?CONST`, `notation`, `Set-Alias`, YAML `*ref`/`<<:`, KV `#:set`. |
| **Cond** | Conditional compilation. ✅ would mean symbols are evaluated (no language has this). 🟡 means every branch is parsed and kept with no evaluation, or there is a repair step. ⬜ means a branch is lost or the file misparses. |
| **Quote** | Quote/unquote and quasi-quote templates are handled: template definitions are not mistaken for real ones, and calls inside them are attributed. |
| **Gen** | Language-defined compile-time codegen is modelled: derives, synthesised members, AST transforms, source generators, TH, ppx, type functions, `generate`. |

## The matrix

| # | language | Def | Call | Body | Args | Expand | XFile | Depth | Tag | Root | Ment | Alias | Cond | Quote | Gen | notes: the single most important gap |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | rust | ✅ | ✅ | ⬜ | ✅ | 🟡 | ⬜ | ⬜ | ✅ | ✅ | ✅ | — | 🟡 | — | 🟡 | Calls inside a `macro_rules!` body are never recorded, so a function reached only through an expression-position macro is reported dead at **high** (fixture `rs2`). Repetition arms and other-file or `#[macro_export]` macros are not expanded (fixture `rs`). Proc, attribute and derive macros are invisible. Derives only root implicit-trait impls. |
| 2 | python | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | ⬜ | `@dataclass`/attrs `__init__`/`__eq__`, `property` accessors and `functools.wraps` wrappers are not synthesised (`class-without-explicit-init`). `name_mentions` is never written, even for decorator string arguments. |
| 3 | kivy | — | — | — | — | — | — | — | — | — | ⬜ | ⬜ | — | — | ⬜ | `#:set` constants are ignored, and KV passed to `Builder.load_string` inside Python strings is never extracted. |
| 4 | javascript | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | ⬜ | JSX is not lowered to `createElement`/`jsx()` calls. The only generated nodes are synthetic `handler_at_<line>` closures. No `name_mentions`. |
| 5 | typescript | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | ⬜ | Decorators are recorded only as attribute strings and calls. Codegen output (GraphQL codegen, Prisma, napi `.d.ts`) is seen only when committed. No `name_mentions`. |
| 6 | go | — | — | — | — | — | — | — | — | — | ⬜ | — | ⬜? | — | ⬜ | `//go:generate`, `//go:linkname` and `//export` are ignored as comments (go doc fixture f7). `//go:build` constraints are not evaluated, so every file is analysed whatever its build tags (unverified, not discussed in the doc). |
| 7 | java | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | ⬜ | Annotation processors (Lombok, Dagger, AutoValue, MapStruct) and protoc stubs are invisible (`grpc-java` gap text, `rules.rs:2329-2345`). |
| 8 | kotlin | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | ⬜ | kapt/KSP output (Dagger, Room, `kotlinx.serialization`) and Compose compiler output are not modelled. `@file:JvmName` has no effect. |
| 9 | c | ✅ | ✅ | ⬜ | ✅ | 🟡 | ✅ | ⬜ | ✅ | ✅ | ✅ | ⬜ | 🟡 | — | — | Calls in a `#define` body are not recorded (fixture `c2`). Object-like macros are not recorded at all: c.rs has no `preproc_def` arm, while C++ has `record_object_macro`. There is no nested expansion (`LIST(DECL)`, fixture `c`), and invocations inside function bodies are never expanded. |
| 10 | cpp | ✅ | ✅ | ⬜ | ✅ | 🟡 | ✅ | ⬜ | ✅ | ✅ | ✅ | 🟡 | 🟡 | — | 🟡 | Calls in a `#define` body are not recorded (fixture `cpp2`). Consecutive semicolon-less invocations in a class body can misparse the rest of the file (cpp doc fixture f7). Object-like macros only type receivers (`type_hints.rs:172-195`). Templates are never instantiated. |
| 11 | csharp | — | — | — | — | — | — | — | — | — | ⬜ | — | 🟡 | — | 🟡 | `#if` handling is a parse-repair only (`csharp.rs:66, 678-729`), and `#define` symbols are never evaluated. Source generators and `partial` halves that exist only at build time are invisible. `partial` caps dead findings. |
| 12 | bash | — | — | — | — | — | — | — | — | — | ⬜ | ⬜ | — | — | — | `alias x='…'` is not expanded (bash doc §02 f8). |
| 13 | ruby | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | ⬜ | `define_method`, `attr_*`, `alias_method`, `Struct.new`/`Class.new` blocks and `class_eval` strings are not expanded (ruby doc fixtures p1, p7). |
| 14 | swift | ⬜ | ⬜ | ⬜ | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | — | 🟡 | — | ⬜ | Swift macros (`@freestanding`/`@attached`, `@Observable`, `#Preview`), property wrappers and result builders are not expanded. A `macro` declaration is not a callable (fixture `swift`). Calls in `#expect(…)` arguments **are** recorded (fixture `swift2`). `#if` keeps every branch (swift doc f8). |
| 15 | lua | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | — | Nothing in this category. `load`/`loadstring` belongs to category 15. |
| 16 | php | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | — | No detection of generated files (Doctrine proxies, IDE helpers, `vendor/composer/autoload_*`). `eval`/`create_function` belong to category 15. |
| 17 | dart | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | 🟡 | `*.g.dart`/`*.freezed.dart` parts resolve when committed (dart doc f6, high) but are not tagged as generated. A missing part is `Target::Unknown` (`libscope.rs:40-43`). |
| 18 | scala | ⬜ | ⬜ | ⬜ | ✅? | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | — | — | ⬜ | ⬜ | Scala 2 `def m = macro impl`, Scala 3 `inline` and `'{…}`/`${…}` quotes are not recognised (scala.rs contains no `macro` handling). sbt and Play routes codegen is invisible. |
| 19 | hcl | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | 🟡 | `dynamic` blocks become plain `dynamic::<label>` callables. Template files (`templatefile`, `.tftpl`) are not parsed. |
| 20 | zig | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | 🟡 | Type functions returning `struct {…}` bind `Fn(T).member()` at medium (`libscope.rs:1727-1771`). `comptime` reflection (`@field`, `inline for` over `std.meta.fields`) and `@cImport` are not modelled. |
| 21 | objc | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | 🟡? | — | ⬜ | ObjC shares the C preprocessor but is excluded from `c_macro::expand` (`crates/cgg/src/lib.rs:530`). It has no `preproc_function_def` arm, and C-style calls, macro calls included, are never recorded (fixtures `objc`, `objc4`). |
| 22 | r | — | — | — | — | — | — | — | — | — | ⬜ | — | — | 🟡? | — | `quote()`/`bquote()`/`rlang` NSE are walked as ordinary calls (r.rs:61-64, unverified). No generated-code tag exists for `RcppExports.R`. |
| 23 | groovy | — | — | — | — | ⬜ | — | — | ⬜ | — | ⬜ | — | — | — | ⬜ | AST transforms (`@Builder`, `@Canonical`, `@Delegate`), Spock rewriting and GORM dynamic finders are not modelled, so `Book.builder()` finds no candidate (unverified beyond code absence). |
| 24 | julia | ⬜ | ⬜ | 🟡 | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | — | — | ⬜ | ⬜ | `macro` definitions are not callables, so `@twice` goes to external (fixture `jl`). Calls in `quote` bodies are recorded from no callable. `@eval` and `@generated` methods are invisible. |
| 25 | perl | — | — | — | — | ⬜ | — | — | ⬜ | — | ⬜ | — | — | — | ⬜ | Source filters, string-`eval` subs, `AUTOLOAD`, glob assignment and Moose/`Mojo::Base` `has` accessors define nothing. `builder => '_build_x'` strings are not `name_mentions`. |
| 26 | elixir | ✅ | ✅ | ✅ | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | 🟡 | ⬜ | — | — | 🟡 | ⬜ | `use`/`__using__` is not expanded. A literal-named `def` inside `quote` is minted as a real member of the macro's module, with no `macro:` tag and no medium cap (elixir doc f3 `Base1::injected`). `unquote` defs are dropped (`elixir.rs:323-328`). |
| 27 | erlang | ⬜ | ⬜ | 🟡 | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | 🟡 | ⬜ | ⬜ | 🟡 | — | ⬜ | `-define` is not a callable and `?M(…)` invocations are dropped (fixture `erl`). Calls in a macro body become top-level sites, which root the callee whether or not the macro is used. `?MKFUN(gen1).` generates nothing. |
| 28 | fortran | ⬜ | ⬜ | ⬜ | ⬜? | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | 🟡 | — | ⬜ | `#define`/`#include` in `.F90` are not recorded. `#ifdef` branches both become duplicate callables (fortran doc `a/pre.F90`). `.fypp` files are skipped entirely. |
| 29 | clojure | ✅ | ✅ | ✅ | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜? | ⬜ | — | 🟡 | ✅ | ⬜ | Macro expansion is absent, so code a macro generates (`(defn ~n …)` names) is invisible and untagged. Reader conditionals keep both branches, and `#_` is not honoured. |
| 30 | haskell | — | 🟡 | — | 🟡 | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | CPP keeps only the first branch, so `#else` is lost (fixture `hs`). TH splices are external calls whose generated lenses and instances do not exist. Quasi-quotes are opaque. |
| 31 | ocaml | — | — | — | — | ⬜ | — | — | ⬜ | — | ⬜ | — | ⬜ | ⬜ | ⬜ | ppx (`[@@deriving]`, `[%expr]`, `let%lwt`) is not expanded, and attributes are not even recorded (`ocaml.rs:234`). cppo is not handled. `.mll`/`.mly` are not detected. |
| 32 | powershell | — | — | — | — | — | — | — | — | — | ⬜ | ⬜ | — | — | — | `Set-Alias`/`New-Alias` targets are not followed (powershell doc f1 L46-47). |
| 33 | solidity | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | — | No gap in this category. Yul belongs to categories 02/03. |
| 34 | fsharp | — | — | — | — | — | — | — | — | — | ⬜ | — | 🟡 | 🟡 | ⬜ | `#if` keeps both definitions, and positional shadowing picks the later one regardless of the build (fsharp doc f8). Quotations produce real call edges. Type providers are not modelled. |
| 35 | starlark | ✅ | 🟡 | ✅ | ✅? | — | — | — | — | ⬜ | ⬜ | — | — | — | ⬜ | Legacy Bazel macros are plain `def`s, but their invocations sit in extensionless `BUILD` files that are not analysed. Symbolic `macro(implementation=…)` is not modelled. |
| 36 | cmake | 🟡 | ✅ | ✅ | — | — | — | — | ⬜ | ⬜ | ⬜ | — | — | — | ⬜ | `macro()` is an untagged `FreeFunction`, and edges flow through it (fixture `cm`). `CMakeLists.txt` itself is never analysed (`unknown-extension`). `cmake_language(CALL fn)` strings are not `name_mentions`. |
| 37 | nix | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | — | No gap in this category. Shell embedded in strings belongs to category 15. |
| 38 | verilog | ⬜ | ⬜ | ⬜ | ⬜? | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | 🟡 | — | 🟡 | `` `define ``/`` `INST(x0) `` is neither recorded nor expanded (verilog doc `c/p4_gen.v`). `` `ifdef `` twins cause ambiguity. `generate` instances are recorded, but the generate block is not a scope. |
| 39 | vhdl | — | — | — | — | — | — | — | — | — | ⬜ | — | ⬜? | — | ⬜ | Generic package and subprogram instantiation (`is new`) is not extracted. Whether the grammar parses VHDL-2019 `` `if `` is unverified. |
| 40 | asm | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | 🟡? | — | — | `.macro`, `%macro` and C-preprocessor `ENTRY()`/`SYM_FUNC_START` in `.S` are not expanded, so kernel/libc `.S` yields few callables (asm doc f1). `c_macro` is not wired for asm. |
| 41 | smithy | — | — | — | — | — | — | — | — | — | — | — | — | — | — | No gap in this category. `apply` belongs to category 02. |
| 42 | proto | — | — | — | — | — | — | — | — | — | — | — | — | — | 🟡 | protoc output is not synthesised. Only the descriptor link from rpcs to host-language methods exists (category 16). |
| 43 | graphql | — | — | — | — | — | — | — | — | — | — | — | — | — | ⬜ | Directives are not recorded, and codegen output (gqlgen, Apollo) is not linked back to the SDL. |
| 44 | openapi | — | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | YAML anchors, aliases and `<<:` merge keys are not expanded, so a shape reached only through `*ref` loses its edges (openapi doc fixtures `a/`, `f/`). |
| 45 | asyncapi | — | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | YAML aliases are not expanded (asyncapi doc fixture `alias`). |
| 46 | lean | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | — | ⬜? | ⬜ | `macro`/`syntax`/`elab`/`notation` produce no callable (`lean.rs:22-26`), so `dbl 3` goes to external with no edge to `double`. A `macro …` command parses with an error. `deriving` generates nothing. `attribute [simp] b` is not captured. |

### Per-cell evidence for the non-trivial rows

- **rust**:
  - Def: `record_macro` gives attribute `macro` (`rust.rs:1448, 1472`); in fixture `rs`, `crate::mk` has `['macro']`.
  - Expand: item position only (`is_item_position`, `rust.rs:2871`), and `macro:` is pushed at `rust.rs:1764`. Tested by `same_file_item_macros_are_expanded_into_synthetic_callables` (`rust.rs:3418`).
  - Depth ⬜: in fixture `rs`, `rep!` generates nothing.
  - Root: `roots.rs:662-705` (rule "Rust macro invoked by path outside any function").
  - Ment: serde `attr:<key>` strings (`rust.rs:2195-2261`) and `mentions_from_item_macro` (`rust.rs:1809-1852`). Tests `serde_attribute_strings_and_macro_arg_values_are_mentions` and `item_position_macro_arguments_are_mentions`.
  - Cond 🟡: `#[cfg]` twins both stay (rust doc §07).
  - Body ⬜: fixture `rs2`.
- **c / cpp**:
  - Def: `c.rs:127` → `record_macro_def` with attribute `macro` (`c.rs:306`); `cpp.rs:201` → `record_function_macro` (`cpp.rs:647`).
  - Expand / XFile: `c_macro::expand` is called only on `"c" | "cpp"` files (`crates/cgg/src/lib.rs:530-533`). It uses the include closure (`INCLUDE_DEPTH = 8`, `c_macro.rs:37`) and caps expansions at 64 KiB (`MAX_EXPANSION_BYTES`, `c_macro.rs:34`). It tags `macro:<name>` at `c_macro.rs:864`.
  - Expansion scope: C++ expands at class and namespace scope (`cpp_class_body_macro_generates_methods`, `c_macro.rs:1181`).
  - Root: `roots.rs:616-660`, which requires `same_family(.., "c")` and attribute `macro`.
  - Ment: `credit`, `c_macro.rs:919-990`.
  - Alias: `cpp.rs:205-208` → `record_object_macro` (`cpp.rs:658-679`) → `FileFacts::macro_aliases`. The only consumer is `type_hints::macro_index`/`resolve_macro_types` (`type_hints.rs:172-195`). c.rs has no `preproc_def` arm.
  - Cond: both branches are parsed. Same-file alternatives bind once through `c_include_copies` (CLAUDE.md, cgg-resolve step 3).
- **csharp**: blank-and-reparse repair in `csharp.rs:66`, `678-729` (`directive`, the blanked copy). No `#define` evaluation.
- **elixir**: Def at `elixir.rs:236` (`defmacro`/`defmacrop` …). Quote at `elixir.rs:323-328`, test `unquoted_def_names_are_not_definitions` (`elixir.rs:892`). Body/Call: fixture `ex`. Root 🟡 comes through the generic by-name `toplevel:invocation` for `no-enclosing-callable` sites (elixir doc §08, CHANGELOG.md:1539-1541).
- **clojure**: Def at `clojure.rs:108-109`. Quote at `clojure.rs:218-222` (the `~` name skip), with a test fixture at `clojure.rs:557`. Body/Call: fixture `cl`.
- **Tag downstream (all ✅ rows)**: no by-name index entry (`cross_file.rs:1026-1032`), no implicit-trait rooting (`roots.rs:849`), `Evidence::MacroGenerated` (`deadcode/mod.rs:893-897`). Separately, a callable with attribute `macro` (a definition, not generated code) is treated as a non-certain candidate (`cross_file.rs:3727`).
- **Ment writers**: across the repository, only `crates/cgg-lang/src/plugins/rust.rs` and `crates/cgg-lang/src/plugins/c_macro.rs` write `name_mentions` (`grep -rln name_mentions crates --include=*.rs` also lists only `cgg-core/src/facts.rs`, which defines it, and `deadcode/mod.rs`, which reads it). Every other ⬜ in the Ment column rests on that grep.
- **`macro:` writers**: only `rust.rs:1764` and `c_macro.rs:864` (`grep -rn '"macro:' crates`). Every ⬜ in the Tag column rests on that grep.

## Parity gaps

Ranked by impact. In each, a language has the concept but lacks support that a comparable language has.

1. **Objective-C is cut out of the C preprocessor pipeline. (L)**
   - What ObjC lacks, compared with C: `#define` callables, `c_macro` expansion, `toplevel:macro-invocation` roots and macro-argument `name_mentions`.
   - Evidence: `lib.rs:530` filters `c_macro::expand` to `"c" | "cpp"`, and objc.rs has no `preproc_function_def` arm (only `preproc_include`/`preproc_import`, objc.rs:131). Fixture `objc`.
   - The cause sits underneath: objc.rs has no `call_expression` arm (objc.rs:59-160), so even a bound macro would get no caller.
   - Effort: M for the macro wiring alone (reuse `c.rs::extract_fragment`-style re-extraction), L together with C-call recording.
2. **Calls inside macro definition bodies are lost in Rust, C and C++, while Clojure, Elixir and CMake record them. (M)**
   - Fixture `rs2`: a `helper` reached only through an expression-position `macro_rules!` is reported dead at **high**.
   - Fixtures `c2`/`cpp2`: `LOG → do_log` is missing.
   - Fixtures `cl`, `ex`, `cm`: `my-macro → inner`, `mk → helper` and `my_mac → helper` are recorded.
   - Cheapest fix (S): record identifiers called in the body as `name_mentions` (`via: macro-body`), so dead code drops to medium. Full fix (M): attribute the body's calls to the macro callable, as Clojure does.
3. **Rust expansion is narrower than C/C++ expansion. (M)**
   - C and C++ expand from the `#include` closure at depth 8 (`c_macro.rs:37`, test `header_macro_generates_functions_in_the_including_file`). Rust expands only same-file, non-repetition arms (rust doc §08; fixture `rs`, `rep_a` not minted).
   - `#[macro_export]` and `$(…)*` arms are where most real `macro_rules!` item generators live.
   - Effort: M for cross-file, M for single-level repetition.
4. **`name_mentions` exists only in Rust and the C family. (S-M per language)**
   - Rust records serde attribute strings. Comparable string-named references go unrecorded elsewhere:
     - Python decorator and `__all__`-style strings;
     - Perl `builder => '_build_x'` and `->can('x')` (perl doc §08);
     - CMake `cmake_language(CALL fn)` and `variable_watch` (cmake doc §08 fixture `t1`, findings with `no-unresolved-site-with-name`);
     - Elixir/Erlang/Clojure/Julia macro arguments (no writer, by grep).
   - Each is a per-plugin push into an existing field. The dead-code reader (`deadcode/mod.rs`, `mentions.evidence_for`) already exists.
5. **Erlang `-define` is treated far worse than C `#define`. (M)**
   - C `#define F(x)` is a callable whose invocations bind. In Erlang, `?F(x)` is dropped entirely, and the macro body's calls leak as file-level roots (fixture `erl`; erlang doc fixtures f1, f6, f8).
   - Recording `pp_define` with parameters as a callable tagged `macro`, and `macro_call_expr` as a site, would give C parity without expansion.
6. **C lacks object-like macro handling that C++ has. (S)**
   - `cpp.rs:205` records `#define NAME expr` into `macro_aliases` for receiver typing (`type_hints.rs:172-195`). c.rs has no `preproc_def` arm.
   - Neither language follows `#define ALLOC my_alloc` as a call alias (c doc fixture f6: `ALLOC` lands in external). Following it is S-M on top of the existing `macro_aliases` field.
7. **Julia macro definitions are not callables, while Elixir, Clojure and Rust macro definitions are. (S)**
   - In fixture `jl`, `@twice` goes to external `no-candidate-in-file`.
   - Extracting `macro_definition` as a callable tagged `macro` would let `@m` invocations bind, and would give the `quote` body's calls an owner.
8. **Haskell CPP loses the `#else` branch, while C, C++, Fortran, Swift, F# and Verilog keep every branch and C# repairs. (S-M)**
   - Fixture `hs`: only the `#if` branch's `f` exists.
   - Porting the C# blank-and-reparse step (`csharp.rs:678-729`) would keep both branches as in the other languages.
9. **Elixir quote-template defs are minted as real definitions with no `macro:` tag. (S)**
   - Rust and C tag generated code `macro:<name>` and cap dead findings at medium. Elixir mints a literal `def` inside `quote` as a plain member of the macro's module (elixir doc f3 `Base1::injected`).
   - Tagging defs inside `quote` with `macro:<enclosing defmacro>` would reuse all downstream handling.
10. **Assembly and Fortran `.S`/`.F90` share the C preprocessor but get none of `c_macro`. (M)**
    - In asm, `ENTRY(memcpy)`/`SYM_FUNC_START` define the callables of kernel/libc assembly (asm doc f1). In Fortran, `#define` and `#include` are not recorded (fortran doc §08).
    - Similar gap: Verilog `` `define `` (verilog doc `c/p4_gen.v`).
11. **No conditional compilation is evaluated anywhere. (L)**
    - Rust `#[cfg]` twins, C/C++ `#if 0`, F#, Swift, Fortran, Erlang, Verilog and Clojure reader conditionals all keep every branch. The F# doc notes that positional shadowing then picks the later twin regardless of build (fsharp doc f8).
    - This is consistent across languages: a design choice, not a parity gap. A shared "conditional twin" attribute (`DefRecord::conditional` exists for PHP) would let dead code and confidence treat twins uniformly. M.
12. **CMake `macro()` is not tagged `macro`, while C, C++ and Rust macro definitions carry the attribute. (S)**
    - Fixture `cm`: `my_mac` has no attributes.
    - Practical impact is low because CMake macro bodies already produce edges. Separately, `CMakeLists.txt` is not analysed (cmake doc §01), which dwarfs this gap.
13. **No language synthesises compile-time codegen. Partial modelling exists only in Rust, C#, Dart, Zig, Verilog, HCL and Proto. (L each)**
    - Unmodelled:
      - Python dataclass/attrs;
      - Java/Kotlin annotation processors and kapt/KSP;
      - Groovy AST transforms;
      - Swift macros and property wrappers;
      - Scala macros and inline;
      - Haskell TH;
      - OCaml ppx;
      - Lean `deriving`/`macro`/`notation`;
      - Ruby `define_method`/`attr_*`;
      - Perl Moose `has`;
      - Elixir `use`.
    - The cheapest step is a uniform "generated-file" tag for committed outputs (`*.g.dart`, `*.Designer.cs`, `*.g.cs`, `*_pb2.py`, `*.pb.go`), applied in the walker or detector, which today tags none of them (dart doc §08 at `dart.rs:267`; csharp doc §08). S-M.
14. **YAML anchors and aliases are not expanded in OpenAPI or AsyncAPI. (M)** `structured::unwrap` skips `anchor`/`tag` children (`structured.rs:18-34`), but a `*ref` alias never resolves to its anchor (openapi fixtures `a/`, `f/`; asyncapi `alias`).

## Doc corrections

1. **swift.md §08 is wrong about `#expect`.**
   - The doc says: "Calls inside a `#expect(...)` argument are not recorded as calls: the macro expression is not a `call_expression` (f3 `NewTests.swift`)."
   - Fixtures `swift` and `swift2` show otherwise: `user → helper`, `NewTests.t → helper` and `adds → Calc.add` (`#expect(c.add(1, 2) == 3)` inside an `@Test func`) are all edges, and the file has `parse_status: ok`.
   - The doc's f3 result may have come from a different shape (not reproduced). The general claim does not hold.
2. **rust.md §08 omits the consequence of unexpanded expression-position macros.**
   - Calls written in a `macro_rules!` body are not recorded at all. No `name_mentions` are recorded either.
   - So a callee reached only through the macro is reported dead at **high** (fixture `rs2`: `crate::helper high`, evidence `no-unresolved-site-with-name`).
   - The doc lists "Expression-position macro bodies" under "Not expanded" but does not say the dead-code verdict is high-confidence wrong.
3. **c.md §08 and cpp.md §08 cite `INCLUDE_DEPTH` at `c_macro.rs:36`.** It is defined at `c_macro.rs:37`. Lines 35-36 are its doc comment. `MAX_EXPANSION_BYTES` at `:34` is correct.
4. **objc.md §08 can be made concrete.**
   - It says the `#if` behaviour is "unverified for objc specifically", and it omits what happens to the span.
   - Fixture `objc`: a file-scope `MKFN(one)` with no semicolon makes the file `parse_status: error` and is absorbed into the next function's span (`user` `start_line` 3, the invocation's line). This is the same failure the C doc describes for a `;`-less invocation before a `static` function.
5. **README.md:1459 is still stale, as cpp.md §08 already reports.** It says "C/C++ function-like macros are extracted as callables but not expanded (no …", which contradicts `c_macro::expand` (`crates/cgg/src/lib.rs:530-533`) and fixture `c` (`made_a` minted with `macro:FN_DEF`). This is a README correction, not a language-doc one.
6. **Scorecard ratings are inconsistent across languages that have the concept.**
   - Kotlin, Java, Python, Groovy, Dart, TypeScript and R rate row 08 `n/a`, yet each has compile-time codegen that cgg does not model: annotation processors, kapt/KSP, dataclass, AST transforms, `build_runner`, decorators, NSE.
   - Scala, OCaml, Lean, Swift and Haskell rate the analogous situation `none`.
   - Groovy 2.5+ also has macro methods (`macro { … }`, `@Macro`). That is a language fact and was not checked against cgg code.
   - In this matrix those languages show `—` for the macro columns and `⬜` for **Gen**, which is the consistent reading.
7. **cmake.md §08 is correct but understates one point.** `macro()` behaves like `function()` for edges: in fixture `cm`, `user → my_mac → helper` flows through the macro. The `macro()` itself is only reachable in `.cmake` files, because `CMakeLists.txt` is `unknown-extension` (fixture `cm`, first run).
8. **No other contradictions found.** The Haskell "first branch kept, `#else` lost", Erlang "`?M()` dropped, body leaks top-level sites", Julia "macro head external", C "no nested expansion" and Elixir/Clojure quote claims were each re-confirmed by fixtures `hs`, `erl`, `jl`, `c`, `ex` and `cl`.
