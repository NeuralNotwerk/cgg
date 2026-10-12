# Cross-language audit findings (2026-10-12, cgg 0.10.2 @ 5b4bdec)

Source: four fixture-driven audits of whether the Rust FIX_PLAN v2 bug classes
(/storage/cgg/FIX_PLAN.md, /storage/cgg/EDGE_CASE_TAXONOMY.md) exist in other
languages. Fixtures (sources only) are under
/storage/cgg/target/remediation/fixtures/{agent-shapes,agent-dispatch,agent-mentions,agent-heads,verify}/.
Line numbers are as of 0.10.2 and will drift — re-locate by symbol.
Prevalence numbers came from heuristic scripts over ~20 repos; they are
pointers, not measurements. Every fix must be measured over the WHOLE corpus.

Structural fact behind most of this: every 0.10.2 fix is Rust-gated.
- `CallableNode::trait_impl_target` is filled only from Rust `<X as T>` qns
  (crates/cgg/src/lib.rs `trait_impl_target_from_qn`). `dispatch::fanout`,
  F2 `TraitSurfaces` (cgg-resolve/src/deadcode/mod.rs), F6
  `homogeneous_trait_declaration` (`lang == "rust"` gate, cross_file.rs ~3410)
  and `IMPLICIT_RUST_TRAITS` rooting all key on it.
- Step 4c external-head refusal: `lang == "rust"` (cross_file.rs ~3326, ~3128);
  `RefRecord::path_receiver` is only ever true from rust.rs.
- F7 `--guess-unique-names`: `lang == "rust"` (cross_file.rs ~1867).
- F10 wrapper typing: `parse_colon_param_rust` / `RUST_WRAPPERS` (type_hints.rs ~751-784).
- F5 `FileFacts::name_mentions`: only rust.rs emits them.
- F12/F4: implemented inside rust.rs (`join_lines` etc.).

## A. WRONG EDGES (highest severity)

A1. External module/package head name-matched to a workspace callable.
  Langs: Python, JS, TS, Go, C++, Java, Kotlin, C#. (Correct: Ruby, Swift,
  Erlang; Scala/Elixir/Lua only "correct" because qualified cross-file calls
  never resolve there at all.)
  - Python `import requests; requests.post()` -> unrelated `app.util.post` at HIGH,
    in a file that does not import app.util (fixture agent-heads/py-c1).
    `from requests import post; post()` -> local fn AND a method.
  - JS/TS `axios.post`, `fs.readFile`, `mongoose.connect` (HIGH) -> local fns/methods (js-c1, ts-c1).
  - Go `uuid.NewString`, `errors.Wrap` -> local type methods, HIGH (go-c1).
  - Java `StringUtils.isBlank` (Apache import) -> local `StringUtils` class; `java.lang.Integer.compare` -> local `compare` (java-c1).
  - C# `Path.Combine` with `using System.IO` -> `Acme.Util.Path` never imported (cs-c1).
  - Kotlin `kotlinx.coroutines.runBlocking`, `File(x).readText` (kt-c1).
  - C++ `boost::split`, `spdlog::info`, `absl::StrCat` (cpp-c1).
  Mechanism: step 4c refusal Rust-only; step 5 duck fan-out (cross_file.rs
  ~3343-3489) fans out on any lowercase receiver incl. an imported module
  alias, screened only by the stdlib NAME list; single-candidate +
  module-alias receiver is PROMOTED to high (cross_file.rs ~1897-1910) even
  when the module is external; step 4 type-name match (~3155-3241) has
  the external-head check Rust-only, so Java/C#/Kotlin match any local
  class of the same name. `--include-external` does not help (only
  unresolved sites become external nodes).
  Prevalence (heuristic, 18 repos): ~4,200 wrong edges; app-gin-photoprism
  1,781, app-ghost-express 1,607 (1,447 high), app-waybar-catch2 363,
  app-kythe-bazel 234, app-memos-echo 101.

A2. Generic parameter types split on every comma.
  `collect_param_types` splits the param list on every `,` (type_hints.rs ~745),
  so `Map<String, Handler> h` types `h` as `Handler`: Java `h.get()` ->
  `Handler.get` HIGH; C# `Dictionary<string,Handler>` -> `Handler.ContainsKey`
  HIGH; C++ `std::map<std::string,Handler>` -> `Handler::at` medium;
  Java `Repo<String,Integer> r` typed `Integer`. Survives --fanout-cap 0.
  Fixtures agent-shapes/f10-generic-comma-*, f10-java-generic. ~7.3k Java params of that shape.

A3. Kotlin receiver built only from `simple_identifier` children
  (kotlin.rs ~338-366): `this.x.m()`, `a.b.m()`, `Foo().m()`, `x.a().b()`,
  `(..).m()`, `x!!.m()` get an EMPTY receiver and are treated as bare calls.
  `other.stream.close()` and `this.stream.close()` -> workspace `Conn.close` HIGH
  (agent-shapes/kt-emptyrecv). ~35k chained receivers in Kotlin corpus.

A4. JS/TS namespace / CommonJS module imports bind order-dependently:
  `import * as svc from './svc'; svc.f()` and `const m = require('./legacy'); m.f()`
  bind to whichever same-named function is indexed LAST (a decoy in
  src/zzz/ wins) — step 2 fallback looks up the bare name (cross_file.rs ~2931),
  name index keeps last entry per name (~695). Fixtures agent-heads/js-c2*, ts-c2*.
A5. Module-head decoys: Python `pkg.sub.jobs.Runner().m()` and
  `from . import helpers; helpers.f()` -> correct target PLUS a decoy;
  Go `r := jobs.NewRunner(); r.M()` adds a decoy (py-c2, go-c2).
A6. Bare free-function call bound to a method: PHP `collect()` -> `Dates::collect`
  (php-c1); Perl bare `zlist()` in package main -> `Repo::zlist` HIGH (agent-shapes/bare2-perl).
A7. Python multi-line self receiver `(self\n .repo\n .quux())` loses the
  self-field type and gains a wrong duck edge (agent-shapes/f4b-py). Rare.
A8. C: a macro invocation with no trailing `;` swallows the next function,
  creating a wrong edge (agent-heads/c4-c*).
A9. C++: a local declared with constructor args (`std::ifstream info(path);`)
  is extracted as a callable; app-waybar-catch2's 357 spdlog edges point at these.
A10. Julia: every `function f()` gets a HIGH self-edge f->f from its own signature (agent-heads/jl-self).
A11. Elixir/Clojure macros: the quoted `def` inside defmacro becomes a fake
  callable (Elixir `unquote` with a wrong self-edge; Clojure `~n`).

## B. FALSE HIGH DEAD CODE (only rust/python/java/csharp can reach high)

B1. Java/C# visibility parsing.
  `java_vis` (java.rs ~377-395) and `csharp_vis` (csharp.rs ~388-405) cut
  the signature at the first `(`, so `@SuppressWarnings("x") public void f()`
  / `[Obsolete("x")] public` reads as non-public -> `private-visibility` -> HIGH.
  Interface members with no modifier are read as package-private/private,
  but are implicitly public in both languages. `Vis::Protected` adds no
  evidence at all (deadcode/mod.rs ~648).
  Prevalence: app-spring-mall 1,070 of 1,087 highs are modifier-less interface
  methods (MyBatis mappers); spring-batch 143 + 28 annotation-arg; Newtonsoft 53/347; eShop 30/173.
B2. Interface/abstract declarations and overrides (F2/F6 analog).
  Java/C# interface decl called via a dropped site is HIGH; protected /
  package-private overrides and C# `protected override` / explicit
  `void I.Process()` are HIGH even when dispatched to. Over-cap sets
  dropped as FanoutCapExceeded. No decl->impl link outside Rust (Python
  has `inheritance_fanout` over base_types; C++ `cpp_virtual_dispatch`
  only from a site already bound to the base decl; `resolve_via_bases`
  walks upward only). Java/C# declare `impls: true`, which removes the
  `language-lacks-dispatch-model` cap though no dispatch edges are made.
  evidence.rs ~143-171 counts an unresolved same-name site as doubt only
  if its receiver_hint is empty/self/this/owner — a variable (`p`,
  `orderService`) or generic (`T`) hint is read as contradicting.
  TS/Swift/Scala/Dart emit NO declaration node for interface/abstract signatures.
  Prevalence: 6 Java/C# repos, 1,280 of 1,402 highs (91%) dispatch-shaped
  (spring-mall, gson, marquez, eShop, MediatR, Serilog). Fixtures agent-dispatch/<lang>-<variant>-<mode>.
B3. Functions passed as values.
  Java: `X::m`, `this::m`, `new Thread(this::m)` — java.rs never handles
  `method_reference`. C#: method groups `Select(F)`, `evt += H`,
  `new EventHandler(this.H)`, `Func f = F`, `nameof(F)`. registrar.rs
  name-like kinds (~38-62) lack `method_reference` / `member_access_expression`;
  Java/C# only call `registrar::capture` (framework verbs). Yet java.rs and
  csharp.rs declare `value_refs: true`, suppressing the lacks-value-refs cap.
  Python: `return _f` / `yield _f`, `@_deco`, default arg `cb=_f`,
  `{k:_f for…}`, `x is _f`, chained `a = b = _f` (python.rs ~351 accepts
  assignment RHS but registrar.rs ~577 value collector has no branch).
  Reflection: Python `getattr(o,"_m")` evidence recorded but caps nothing
  (deadcode.rs ~331 caps only value-position/attribute-string); Java
  `getDeclaredMethod("m")` records nothing (dynuse.rs ~92 reads a `function`
  field that Java `method_invocation` lacks); csharp/ruby/php dynuse entries unused.
  The 0.10.2 value-ref doubt applies only when the target is a CLOSURE (deadcode/mod.rs ~236).
  Prevalence: JabRef 260/580 Java highs; C# Avalonia 30/47, MediatR 7/26, eShop 16/173, Newtonsoft 30/347; Python ~9/116.
B4. Names given in framework annotations/attributes (F5 analog), with --include-tests:
  JUnit5 `@MethodSource("p")`, TestNG `@DataProvider`/`dataProvider="x"`
  (JabRef 222/902), NUnit `[TestCaseSource("X")]`/nameof, xUnit
  `[MemberData(nameof(X))]` on private members. Gap texts already in
  cgg-core/src/frameworks/rules.rs (~6972 JUnit, ~2576/2593 NUnit/xUnit, ~7472/7497 C/C++ callbacks).
  Java builtins/frameworks: private `readObject`/`writeObject`/`readResolve`
  (Java serialization — builtin), `@PostConstruct`, `@EventListener` HIGH (`@Scheduled` is rooted).
B5. C# `a.M<T>()` (kept as name `M<string>`, external-screened, audit-only)
  and `x?.M()` (silently dropped) -> callee false HIGH (agent-shapes/f12-cs-private, dc-cs-condaccess).

## C. SILENT DROPS / UNRESOLVED (recall; mostly medium-capped languages)

C1. Explicit type-argument callees (F12 analog):
  C++ `f<int>()` SILENT (cpp.rs ~609-643 `_ => return` on `template_function`);
  `b.m<int>()`, `B::s<int>()`, `ns::f<int>()`, `b.template m<int>()` keep `<int>` in the name -> external.
  Go `F[int](x)`, `pkg.F[*T](err)` SILENT (parsed as type_conversion_expression;
  go.rs ~152 handles only call_expression); `F[int,string]()` resolves.
  Measured: hcl-terraform-aws `errs.IsA` 0 incoming edges, ~2k sites.
  Dart `f<int>()`, `b.m<T>()` SILENT (dart.rs ~304 `instantiation_expression`).
  C# member generic (B5). Swift `Wrap<Int>.s()` misparsed as comparison.
  Scala `f[Int]()` name `genF[Int]`.
C2. Receiver shapes: parenthesized/cast/await receivers (`(f).m()`,
  `((Foo)o).m()`, `(o as Foo).M()`, `(await x).m()`) unresolved in EVERY
  language while the bare form resolves typed. Construction receivers
  `new Foo().m()` (Java/C#/TS/JS, C# object initializer) never typed ->
  duck fan-out incl. wrong decoy; Python `Foo().m()`, C++ `Foo().m()` /
  `Foo{}.m()`, Go `Foo{A:1}.M()` / `(&Foo{}).M()`, Swift/Kotlin `Foo().m()`
  unresolved. Lowercase-first gate cross_file.rs ~3350-3355; no non-Rust plugin normalizes receivers.
  Null-conditional SILENT DROP: C# `x?.M()` (csharp.rs ~361), Dart `x?.m()`
  (dart.rs ~304), PHP `$x?->m()` (php.rs ~125 no nullsafe kind). TS/Kotlin/Ruby/Swift fine.
C3. Optional/union/wrapper annotations typed WORSE than untyped (F10 analog):
  Python `x: Optional[Foo]` -> hint `Optional` (booked as stdlib call);
  `x: Foo | None`, `x: "Foo"` unresolved; TS `x: Foo | undefined`;
  Swift `x: Foo?`/`Foo!`; Kotlin `x: Foo?`. `parse_colon_param`
  (type_hints.rs ~887-909) cuts only at `<` and `[`; rewrite at ~703 replaces a duck-resolvable hint.
C4. Macro-generated callables (F9 analog): C/C++ function-like macros
  that define functions (`#define DEF_GETTER(n) int get_##n(void){...}`)
  — generated names not callables, calls external/unresolved, macro itself medium-dead.
  Elixir/Clojure generated calls missed (see A11).
C5. Extraction bugs (verified):
  - Dart: a class method's callable covers only its signature (dart.rs ~93/~110
    records just `function_signature`); calls in method bodies get src=null.
    dart-flame: 3,807 of 4,095 methods span one line, ZERO edges from any method.
  - Scala: whole callee text used as name, no receiver split (scala.rs ~178-194);
    `b.m()` -> name "b.m"; scala-play ~17k dotted names external-screened.
  - C: functions returning a pointer (`char *f(void)`) dropped — c.rs
    `fn_name_from_declarator` (~173-203) never descends into pointer_declarator.
  - C++ `.h` with only a struct with virtuals classified as C (header-heuristic:c) -> pure-virtual decls lost.
  - C#: `new Alpha().Run()` does not resolve to inherited `Base.Run`.
  - Ruby: paren-less bare method call gets no edge. PHP: `$this->m()` no edge.
    Lua: `function M.f()` gets no node. Elixir: `&f/1` no edge.
    Python: lambda assigned to a variable has no edge from the enclosing function.
  - TypeScript records no visibility despite `private`/`protected`.
C6. Rust leftovers: the 21 `macro_rules!` invocation sites on nkb are still
  logged `no-enclosing-callable`; `Type::f()` is still read as a constructor
  (0.10.2 CHANGELOG "pre-existing, not chased").

## Not affected (verified)
- F1 async-method self-field gap was Rust-only.
- "bare call must not bind to a method" (intra_file.rs is_async_method etc.) is language-agnostic, correct for Py/JS/TS/PHP/Go.
- Multi-line receivers resolve the same as single-line in Java/C#/TS/JS/Kotlin/Go/C++/Swift.
- pytest fixtures, Rails callbacks, PHPUnit data providers, Go http.HandleFunc, JS addEventListener are already rooted/live.
- `--reference-edges` is forced on in dead mode (lib.rs ~238-246); the default-graph prune does not affect dead-code bands.
