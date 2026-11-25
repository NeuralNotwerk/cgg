# Backlog

Work that is known and deliberately deferred. Nothing here is scheduled.
Each item says why it is waiting and roughly what it would take. When an
item ships, delete it here and describe it in `CHANGELOG.md`.

Versioning: a new grammar, parse mechanism or language is a minor release
(`0.x.0`); a fix to an existing parser or resolver is a patch.

## Patch-level follow-ups

Small gaps found during the 0.11.2 remediation (WS32–WS38). Each is a fix
to an existing plugin or resolver.

- **Kotlin `typealias`.** Return types and annotated locals written
  through an alias stay untyped (2 correct edges lost in WS35).
- **Kotlin trailing lambdas in the arity check.** The count is right, but
  a trailing lambda is not placed against the last parameter, so
  overloads whose last parameter is a defaulted lambda stay `medium`
  (12 edges that could be `high`).
- **C entry points `wmain` / `WinMain`.** Not recognised as roots, so
  their callees are reported dead.
- **Clang `__overloadable__` C functions.** Should bind as an overload
  set; today they are `ambiguous-in-file`.
- **C macros redefined after `#undef`.** Should bind the definition in
  effect at the call.
- **Clojure `letfn` body values.** Forms in a `letfn` body are not read as
  value references, so a `letfn` function used only as a map value is
  reported dead (1 corpus finding).
- **F# files whose parse fails.** The `module X.Y` header no longer
  encloses later declarations, so their functions keep bare names and
  module-qualified calls to them fall back to by-name binding.
- **Nested functions in Scala, Kotlin, Swift, Dart and Rust.** WS33
  keeps a function nested in another function out of other files' bare-name
  candidates, but only in `NESTED_SCOPE_LANGS`. These five were left out
  because their member names carry the type, not the enclosing function,
  and a span a parse recovered past made type members look nested. Revisit
  with a nesting test that survives parse recovery.
- **Solidity Yul.** Inline-assembly calls are not recorded.
- **C/C++ type names read as value references** (29 `medium` corpus
  edges).
- **Spock `ProviderSpec`** fixtures not rooted (20 `medium` findings).
- **Fortran arrays brought in with `use`** are read as calls (1 site).
- **JS/TS coincidental roots** hide some findings in vendored code.
- **Dart bases declaring no methods** (393 `medium` findings).
- **Perl `$handler` registrar values.** The registrar capture strips the
  sigil, so `get "/x" => $handler` binds an unrelated `sub handler`.

## Found in the 0.13.4 documentation audit (2026-11-25)

Reported by the doc-audit agents from fixtures run against 0.13.3; each
needs a fixture test and a corpus A/B before it ships. Fixes to existing
resolvers, so patch-level.

Wrong `high` edges (fix first):

- Go: `n := NewServer()` types `n` by the `New*` name over the declared
  result type, so `n.Zap()` binds the wrong owner.
- TS/JS: a parameter does not shadow an imported name of the same name.
- Solidity: bare calls bind across unrelated contracts in one file.
- Nix: a bare name binds an imported file's attribute.
- R: a call binds a function nested inside another function.
- Clojure: `(comment …)` bodies produce call edges.
- Erlang: `Mod:f()` through a variable binds the local `f`.
- Erlang/Elixir: a wrong-arity call binds the single candidate.
- CMake: the include arm always tries the literal `CMakeLists.txt`, so
  any `include()` reaches every `CMakeLists.txt`'s functions at `high`.

Other resolver and extraction gaps:

- Lua: a `local function` is reachable from a requiring file (medium).
- Starlark: a `load`ed name loses to a same-named nested function.
- Fortran: an interface body plus a same-named function is
  `ambiguous-in-file`.
- F#: `module U = A.B.C` (dotted target) does not resolve; bare calls
  after `open` reach modules never opened.
- Groovy: `this.<T>id()` breaks the parse; import aliases ignored and
  wildcard imports read as class imports; no external-module refusal;
  `new X()` with no constructor leaves an unresolved record.
- Python: `f = self.loader.load; f()` stays untyped.
- JS/TS: nested one-line `it` callbacks get duplicate `handler_at_N`
  names; `process.on('exit', function onExit(){})` mints two callables.
- Kotlin: `get()` on the next line missed; an enclosing-class member
  call is ambiguous when an unrelated class declares the name; a
  one-line named companion loses its class.
- Scala: `Square(3.0)` with a companion `apply` binds the constructors.
- Java: generic `new Box<>()` never binds.
- C#: `global using static` ignored; `aspnet-mvc` mints an entry for a
  private method.
- C: a file-scope initializer naming another file's function is not
  credited, so that function is reported dead.
- Duplicate bucket records for alias calls: Zig, Dart, PHP, Clojure
  (with `--include-external`).
- Rust dead code: a function used as `value_parser = f` inside a clap
  attribute is reported dead at `high` (cgg's own `parse_externals`).
- `--externals`: `use rayon::prelude::*` absorbs std `Vec` methods;
  PowerShell `Start-Process` reported as stdlib, not a process site;
  `--include-external` leaf names embed receiver expressions
  (`File::open(path)::with_context`).
- Kivy: a `<A,B>:` rule after another rule links nothing.
- Stdlib tables: HCL (`strcontains`, …), PHP (`get_class_methods`).
- Verilog tasks falsely rooted through the `task` attribute; Lean
  visibility never captured; Swift has no `signals()` override.
- Stale gap text in `rules.rs` (gradle-task, grails, flutter-test,
  shelf, expecto, fsharp-nunit, scotty, xctest, go-testing), the
  `go.rs` header and the Rust `BLIND_SPOTS` entry in `caps.rs`.
- Labels: withheld `only-used-by-tests` rows say "(signal not
  extracted)"; the stderr "N unresolved call(s)" line counts external
  calls too.
- `scripts/framework-coverage.py` has no `CGG_TOTAL_BUDGET`.
- Haskell: `import C hiding (h)` still binds `h` at `high`; qualified
  calls (`U.helper`) make no edge to in-tree modules.
- Python: `import os, flask` hides flask from detection; comprehension
  and `for` variables do not shadow (binds the module function, `high`).
- Swift: `let m = Motor(); m.start()` leaves `m` untyped.
- Starlark: `load(":a.bzl", "f")` / `load("//:a.bzl", "f")` make no edge.
- PHP: `call_user_func` missing from `PHP_CALLABLE_ARG_FUNCS`.
- ObjC: `#define MKFN(n)` then `MKFN(one)` breaks the parse.
- CMake: a top-level call to an `include()`d function is unresolved, so
  the function is reported dead.
- Kotlin dead code: capability output says `max_confidence: high` while
  every finding is capped at medium.
- PowerShell: native commands get an empty `--externals` symbol
  (`git::`); dot-source paths written with `\` do not resolve.
- Perl: `POSIX::floor` records an extra `POSIX` external site.

## 0.14.0 partials and follow-ups (2026-11-25)

The audit's P1 items shipped in 0.14.0 (see `CHANGELOG.md`). These parts
of them did not, or surfaced while merging; P2/P3 items below stay
scheduled for 0.14.x.

- Kotlin and Dart record registrar slots but do not capture registrar
  arguments, so registrar rows (slotted or not) are inert there.
- Elixir `use MyAppWeb, :live_view` does not expand `__using__`; the
  Phoenix rows stay name-matched tree-wide.
- JS/TS: Cloudflare Agents `schedule(…, "method")` (target in the second
  string); nested options-object slots (Claude Agent SDK
  `query({ options: { canUseTool, hooks } })`, a Vercel AI tool's name
  from its object key); typed literals (`const g: InputGuardrail = {…}`);
  a renamed import used only as a module-level value.
- Prism and MCP PHP tool names set in an earlier chained call
  (`Tool::as('weather')->…->using(fn)`) route by the function name;
  mcp-ruby methods inside `class << self`; C# `Create(_svc.M)` and
  `MapGrpcService<T>` mint no entry node.
- Base-type rows with no method list root private helpers too (netty,
  grpc-java/kotlin/dotnet, slick, abp-appservice, graphql-ruby, thor;
  Thor `no_commands`).
- Detect-only or partial rows: `jane-street-command`, `targets` (a
  top-level call into a sourced script does not bind), `zig-httpz` route
  nodes, Spring AI `FunctionToolCallback`, Room migrations as anonymous
  objects, Hubitat `subscribe`/`runIn` handlers, Temporal Java
  interface-only methods, Forge `addListener(this::m)`, Vapor route
  closures, React Native's Swift `RCT_EXTERN_METHOD`, Boost.Test cases
  still named after the macro, ESMF spellings, `.ino` sketches,
  fixed-form Fortran continuation lines, assembly include closures.
- A Dash app's `@app.server.route`/`@app.server.errorhandler` are Flask
  decorators in a file importing only `dash`; per-file evidence drops
  them (py-dash: one false high). Let `dash` count as Flask evidence for
  attributes reached through `.server`, or add a dash row.
- C#: calls inside a lambda in a field initializer
  (`static readonly … = Register(…, (o, x) => o.M(x))`) are not recorded
  (predates 0.14.0; app-openkh-mcmaster `SetDebugLayoutRenderer` is now a
  false high because field typing removed the doubt that hid it).
- Dart: after `with`-clause bases are recorded, the facade narrowing
  drops a mixin delegatee's direct edge for an untyped receiver
  (dart-flutter −398 edges, reachability kept). Consider exempting a
  delegatee that is a mixin.
- C++: a namespace closed by a macro (`NLOHMANN_JSON_NAMESPACE_END`)
  leaks an `(anonymous)::` scope to the rest of the file (json.hpp: 337
  qnames; edges resolve).
- C++: Catch2 `TEMPLATE_TEST_CASE("d", "[t]", types…)` bodies are not
  callables (doctest's forms are).
- `zig-zig` runs ~38 s alone, within ~20 s of the 60 s corpus cap; under
  parallel load it trips it. deer-flow runs 21–45 s under load.
- app-pfr-lean has no APPS row (a formalization, not an application).

## Framework audit and research (2026-11-25)

Per-language audit agents read the rule table at HEAD (0.13.4) and ran the
0.13.3 binary on fixtures and real apps; an independent agent re-ran every
finding (refuted ones are gone). "Reproduced": a fixture showed it on 0.13.3;
"code only": read, not run. Web research went through nkb, where the sources are
ingested. Rows and matcher fixes are patch-level; a new parse mechanism is
minor. MCP and LLM-tool rows are in the AI agent section.

### Cross-cutting

- **Per-file gating for every rule kind** (all languages; matcher).
  `match_registrars` and `match_methods` gate on per-language detection (only
  Test rules check the file), and attributes are import-scoped only in
  Java/Kotlin, so one import lets a rule claim the whole language: a FastAPI
  handler is also `flask` (bugbug 31), invoke `@task` is `celery`, discord
  `@bot.command` is `click`; Express verbs take `$.get`, `Map.get`, Playwright
  `page.route`, Angular `http.get` (Ghost 328 entries, 149 in Ember; every
  `express` entry on ghostfolio and cal.com is false); one chi import doubles
  175 gin routes in photoprism and net-http claims asynq handlers; rails labels
  Sinatra, Grape and Padrino routes; phoenix with plug-router and mojolicious
  with dancer2 mint each route twice; NUnit `Theory` takes xUnit theories; tasty
  enters an hspec file; Erlang `handle_*`, Elixir `run`/`call`, Swift `handle`,
  Swing listener names and Kotlin Android `onReceive` root in unrelated modules.
  Accept a non-test match only where the file carries the rule's evidence
  (`Detection::file_carries`) or imports an in-tree file that does (Flask
  blueprints, Elixir `use MyAppWeb, :x`), else prefer the most specific rule.
  Extends the agent section's Per-file framework evidence; prerequisite for most
  rows below. Reproduced (fixtures; photoprism, Ghost, ghostfolio, Plausible,
  lobsters).
- **Path- and owner-scoped matchers** (all languages; new rule field, patch).
  Some names are safe only in one file set or owner: Rails route verbs in
  `config/routes*`, Next.js/Remix/SvelteKit/Astro `GET`/`POST` in route files,
  Jenkins `vars/*.groovy` `call`, `build.zig` `build`, Kong phases in the table
  `handler.lua` returns, Dash `layout` beside `register_page`, Mix tasks under
  `Mix.Tasks.*`, LÖVE `love.*`, GMod `GM:`, Rack/Sidekiq middleware named at
  `use`/`chain.add`, Grails `grails-app/controllers/*Controller`. `detect_paths`
  gates detection only. Add a per-rule path, owner and owner-suffix gate for
  `methods` and registrars. Reproduced (prototype rules).
- **Network registrars fire in test files** (all languages; matcher). Rails
  request specs mint `post('/tags')` routes, ~140 of Ghost's express entries are
  tests, Laravel feature tests mint five routes on monica. Skip non-Test
  captures in files `classify_test_file` marks as tests. Reproduced (fixture,
  lobsters, monica).
- **Route-string gate** (`AMBIGUOUS_VERBS`; all registrar languages). Any string
  passes, so URLs (`$ua->get('https://…')`), event names (`.on('SIGTERM')` as
  bullmq/hono routes), cache keys (`Cache::get('x')`, `$request->get('status')`)
  and a piped `Map.put("k", x.y)` become routes. Reject URL schemes; pair with
  per-file gating. The gate also drops real shapes: path-less `use(mw)`
  (Express, polka, hono, Socket.IO, Koa, Go gin/echo/chi/fiber/hertz/martini
  `Use`), Bull `process(fn)`, Swift `get(use: h)`, tide verbs; accept a function
  value in a gated file. Reproduced (fixtures).
- **Route extraction in `route_for`** (Go, Rust, JS/TS, PHP, Python; matcher). A
  leading HTTP method becomes the route (Go `Handle("GET", p, h)`, chi `Method`,
  echo/fiber `Add`, robyn `add_route("GET")`); constants and `""` are dropped
  (`r.GET(usersPath, h)`, echo `g.GET("")`, lemmy `.route("")`, Laravel
  `Route::get('')` then named by `->name()`); the route comes from a later
  chained call (`.service(a).route("/x")`, `app.use(auth).get('/users')`) and
  never from the receiver chain (`route('/x').get(h)`, tide `app.at`, salvo
  `with_path`, actix `web::resource`, go-restful `ws.GET`). Skip a method-name
  first string, read package-level const strings, keep `""`, ascend only out of
  `arguments`, take the innermost receiver-chain string. Reproduced (fixtures,
  app-lemmy-actix).
- **Registrar capture missing** (Kotlin, Scala, Dart, Groovy, F#, OCaml,
  Haskell, Lua, Zig, Julia, R, Starlark, PowerShell, Bash, CMake; extraction,
  patch each). These plugins set no registrar context or route, so every
  registrar row and `--registrar` verb is inert (prototype rows: 0 entries in
  each). Java captures no method reference or lambda (`collect_value_refs` has
  no `method_reference` arm: Javalin, Vert.x Web, WebFlux routes, Helidon SE);
  C# and PHP mint no `handler_at_<line>` for lambdas and closures (`MapGet("/",
  () => …)`, `Route::get('/x', function …)`, Pest). Overlaps the agent section's
  Registrar capture in more plugins.
- **DSL block callables** (Ruby, Elixir, Clojure, R, PowerShell, Perl, Zig, Lua,
  Dart, Kotlin, C, C++; extraction). Test and route blocks need a
  `handler_at_<line>` callable as JS `it()` has: Ruby `get do`/`before
  do`/`setup do` (`is_registration_shape` returns at `arguments_of`; barong has
  51 Grape blocks), ExUnit `test`, Clojure `deftest`/`defspec`, testthat, Pester
  `It`, Perl `hook`/`helper`, Zig `test` blocks (their callees read as
  production roots), busted, Dart `test()`, Ktor trailing lambdas,
  Catch2/doctest `TEST_CASE`, gMock `MATCHER`, Criterion `Test(s, n)`. Matrix 11
  item 7. Reproduced (fixtures).
- **Duplicate `handler_at_N`** (extends the listed JS/TS item). Clojure nested
  one-line routes mint two `handler_at_21` too; name by line and column in every
  minting plugin (Go, JS/TS, Ruby, Perl, Elixir, Clojure, Swift). Reproduced
  (fixture).
- **Registrar values bind through locals** (all; matcher).
  `NameIndex::by_simple` takes the one tree-wide definition of a captured name
  and ignores locals and parameters: `router.get('/x', handler)` with a
  parameter `handler` enters another file's `handler`, Clojure `(fn [node]
  (mc/options node))` enters `util/node`, Rails `resources :reports` binds any
  `reports`. Apply the value-ref scope rules. Reproduced (fixtures).
- **Base-type name patterns** (Python, Java, Kotlin, Go, C#, Swift, Rust;
  matcher). `base_type_matches` never calls `name_pattern_matches`, so
  `*Servicer`, `*ImplBase`, `Unimplemented*`, `*AsyncProvider` and gdext `I*`
  cannot match; every gRPC server row waits on it (grpc-java/-go/-dotnet are
  detect-only in 10 corpus apps). Reproduced (prototype rules).
- **Bases matched by last segment** (all; matcher). Bases are not import-scoped
  (Camel `Processor` claims Kafka Streams processors; `Service`, `Filter`,
  `Tasklet`, C++ `X::Service` are generic), and `BaseTypeIndex` keys `by_owner`
  by bare name, last insert wins: mastodon's `< ActiveRecord::Base` walks into
  an in-tree `CLI::Base < Thor` (726 wrong roots), kamal loses 28 of 89 Thor
  commands. Key by full owner, match by qualified suffix, accept a base only
  through a detect-prefix import or an in-tree chain. Reproduced (fixtures,
  mastodon).
- **Base-type rules root private and non-invokable members** (Java, C#, Ruby,
  PHP, Scala, Dart; matcher plus visibility capture). The method list is flat
  and an empty `methods` names every member: private `validate` in an
  `ItemProcessor`, Storm `combine`, `[NonAction]` methods, constructors and
  accessors in `aspnet-mvc` (extends the listed private-method item),
  `signalr`/`orleans` helpers, Ruby `actioncable`, PHP `codeigniter` privates,
  Play controller helpers, Dart `_` helpers in serverpod. Skip private members;
  record visibility in Ruby, PHP, Scala and Dart. Reproduced (fixtures).
- **Abstract declarations get entries** (Swift, Solidity and every plugin
  tagging `abstract`; matcher). `match_methods`, `match_base_types` and
  `match_visibility` do not skip bodiless declarations: a Swift protocol
  requirement gets an `aws-lambda` node, 54 of Safe's 155 and 61 of
  morpho-blue's entry edges are interface members. Skip them. Spring/JAX-RS API
  interfaces (`PetsApi.getPet`) need the reverse: also enter in-tree overrides
  of an abstract match. Reproduced (fixtures, safe-smart-account, morpho-blue).
- **Bases split across declarations** (Swift, C#, Kotlin, Dart, JS/TS;
  extraction). Members of a Swift extension, a C# partial part with no base list
  (EF `*.Designer.cs`, Blazor code-behind) or a Kotlin `object : Base {}`
  literal (it gets the enclosing class's bases) match nothing, a later
  `extension X: Sendable` replaces `X: XCTestCase` in `by_owner`, and a JS/TS
  base through a default or renamed import breaks the walk (45 heroku `run()`).
  Union bases per type; resolve bases through import records. Reproduced
  (fixtures).
- **Generic and aliased bases** (Python, TS, Kotlin, Scala, Swift, C++;
  extraction). `is_type_name` drops `Base[int | str]`, multi-line `Base[...]`
  (18 proxmoxve coordinators) and TS `Base<A | B>`; a `typealias` base hides
  roots (two mihon Paging sources), extending the listed Kotlin `typealias`
  item. Reproduced (fixtures).
- **Constructor rule shape** (Kotlin, Java, C#, Scala; matcher). No rule can
  root the constructor of a matched class: Kotlin Activity/Service `init {}`,
  WorkManager `Worker`, Vaadin `@Route` views, Kotest specs, Spring stereotype
  beans, the JPA no-arg constructor, FluentValidation validators, AutoMapper
  profiles, Akka.NET `ReceiveActor` all stay medium findings. Add a constructor
  token for `base_types` and `attributes` rows. Reproduced (fixtures).
- **Method-name vocabulary** (Elixir, Erlang, ObjC, Go, Perl, Haskell, C;
  matcher). Names carry no arity (Elixir `update/2`, `id/1`; eunit takes arity-1
  CT cases; ObjC `testHelperWithValue:` is a test); Go falls back to
  case-insensitive names (`mod.rs:604`: `serveHTTP`, `marshalJSON` helpers
  become entries, bubbletea `Init` hits `init`) while Moose `DESTROY` must not
  take `destroy`; patterns allow one leading or trailing `*` (Haskell `get*R`,
  `sqlite3_*_init`); `signature_methods_for` reads only `Type name` parameters
  (Zig `b: *std.Build`, Go, Rust, Kotlin, Swift, Scala, TS). Add arity,
  per-language case rules, infix patterns and name-first parameters. Reproduced
  (fixtures).
- **Detection engine** (all; matcher). `detect_calls` compares bare names and
  counts in every file (`Deno.serve` is not expressible; any helper named
  `describe` turns on `js-test-globals`: 23 false rows); `detect_paths` sees
  only parsed files of the rule's language and needs a `/` before the marker
  (`serverless.yml`, `.mocharc.yml`, `*.Tests.ps1`, `.t.sol`, Lua `lua/`); a JS
  harness config never activates its TS twin; `import_matches` is case-sensitive
  in case-insensitive languages (VHDL `library Ieee`, Fortran `USE MPI_F08`,
  PowerShell, CMake); TS `import type` counts as evidence. Allow qualified
  `detect_calls`, count Test-rule calls only in test files, match walked paths
  and suffixes, share evidence between twins, fold case. Reproduced (fixtures).
- **C-family header evidence** (C, C++, ObjC; matcher). Detection reads only a
  file's own includes, so a `.m` or test `.cpp` including a project header that
  includes `<XCTest/XCTest.h>` or gtest is no harness file (Hammerspoon's XCTest
  suite; dart-flutter `shell/common`: 32 findings). Count system includes of the
  quoted-include closure (`c_include_closures`), widen the test-file arm
  (`_tests.`, `_unittests.`, `.cxx`), then drop `cpp-test-macros`'s
  repo-specific `doctest_compatibility.h` prefix. Reproduced (fixtures,
  Hammerspoon).
- **Client-only and sibling detect prefixes** (table). False "seen, no rules"
  lines: JVM `com.amazonaws.services.lambda` (use `.runtime`; thingsboard), Ruby
  `aws-sdk-*` gems (mastodon), Scala `org.http4s`/`akka.http.scaladsl` clients,
  `akka.actor` on typed-only files, Haskell `Servant.Client`, Clojure
  `ring`/`org.httpkit` (use `ring.adapter`, `org.httpkit.server`), JS
  `socket.io-client`, `@graphql-tools/schema`, `https://deno.land` (std only),
  Go `gorilla/websocket`/`handlers` (photoprism, kubepi), C# bare
  `Microsoft.AspNetCore` in two rules and `Grpc.Net.Client`, PHP
  `HttpFoundation` in Laravel, `Illuminate` standalone, `Cake` (chronos),
  `functions.php`, Rust `actix-web` (never matches) and any `worker` crate, R
  `source()` paths read as packages, `c-runtime` on `stdio.h`/`stdlib.h` (29
  apps). Narrow each. Reproduced (fixtures; corpus grep).
- **Includes through variables** (PowerShell, Bash, R; resolver). `.
  $PSScriptRoot/../src/x.ps1`, a `. $i.FullName` loader loop, `source
  "$DIR/x.sh"`, `. "$d_api"` and `source(file.path(…))` resolve nothing, so
  cross-file calls stay `candidates-in-other-files` (acme.sh: 0 cross-file
  edges). Resolve `$PSScriptRoot`-relative paths; fall back to a unique by-name
  binding at `medium` in a file with no literal include. Reproduced (fixtures,
  bash-acme).
- **Top-level cross-file calls credit nothing** (C#, Python, F#, OCaml, Perl,
  Lua, Zig, R, Bash, Scala; resolver). A module-level call into another file is
  `no-candidate-in-file`, and `toplevel:invocation` reads only
  `no-enclosing-callable`, so the callee is dead: eShop `Program.cs`
  `MapCatalogApi`, `from helpers import make; make(2)`, Expecto `testList` and
  Giraffe `webApp` values (FsAutoComplete: 468 test callables), `use Calc
  qw(add)` in a `.t`, Lua `require` in a spec, Zig `util.sub()` in
  `test`/`comptime`, `source()`d R and Bash, ScalaTest class-body `test("x")
  {}`. `--module-scope-calls` fixes only F#, Python and Scala. Root such sites
  by name or resolve them; the listed CMake `include()` item is one case.
  Reproduced (fixtures).
- **`bin/` is a built-in deny dir** (Rust, Dart, OCaml, Perl, Ruby; walker).
  Cargo `src/bin` (app-axum-cratesio's server, rust-bat's CLI, the gotham
  router), Dart `bin/`, dune `bin/` (ocaml.org's `main.ml`) and script dirs
  vanish. Deny `bin` only when it holds no registered source. Reproduced
  (fixtures).
- **Builtin root tables bypass rules** (dead code). `CONVENTIONAL_METHODS` roots
  `describe` in every language (no runtime calls it); `LIFECYCLE` roots every
  Java/Kotlin `run`, `call`, `onCreate`…, `initialize`, and Erlang/Elixir
  `init`, `handle_*` with no base or behaviour check, while Erlang lacks `stop`,
  `prep_stop`, `format_status`, `callback_mode`. Drop `describe`; move the rest
  into base- or behaviour-scoped rows, then corpus A/B. Reproduced (fixtures).
- **`main` rules print false gaps** (ObjC, Dart, Haskell, F#, Zig; table).
  `objc-main`, `dart-main`, `haskell-main`, `fsharp-entrypoint` and `zig-main`
  call `main` too common though `UNIVERSAL_ENTRY` roots it, and their detect
  lists (`dart`, `System`, `System.Exit`, `std`) make every tree PARTIAL. Give
  them `methods = ["main"]` (node false) or the real marker (`EntryPoint`,
  `wWinMain`, `EfiMain`), or drop them. Reproduced (fixtures).
- **Foreign-export markers** (Go, Swift, Haskell, Fortran, F#, Zig, Rust,
  Erlang, Lean; extraction). Exports read dead because the marker is not
  recorded: cgo `//export`/`//go:wasmexport` (tigerbeetle), `@_cdecl`, `foreign
  export`, `bind(c)` bodies, `UnmanagedCallersOnly`, Zig `export fn` (27 in
  libxev, 119 in ghostty; record the visibility, add a `visibility_entries_for`
  row) and `@export`, Rust `#[unsafe(no_mangle)]` and `unsafe(export_name)`
  (`attribute_key` cuts at the first `(`), `-on_load(F/0)`, Lean `@[export]`.
  Tag them with `FFI_EXPORT_ATTRS`. Reproduced (fixtures).
- **Cross-language FFI links** (`ffi::link_ffi`; C/C++ with R, OCaml, Lean,
  Elixir/Erlang, VHDL, SystemVerilog). No edge for R `.Call("_pkg_f")` (ranger
  `rangerCpp`), OCaml `external … = "stub"` (unison: 65 stubs dead), Lean
  `@[extern "f"]` (LeanCopilot 7), `nif_error` stubs to
  `ERL_NIF_INIT`/`#[rustler::nif]`, VHDL `attribute foreign … "VHPIDIRECT c"`
  (microwatt 8), SV `import "DPI-C"` (ibex 18). Add each as JNI and P/Invoke
  are. Reproduced (fixtures).
- **Native-extension init rows** (C, C++, ObjC; table). Loaders call `R_init_*`
  (ranger, vroom), `PyInit_*`, Ruby `Init_*` (puma), `luaopen_*` (Hammerspoon:
  105 dead; a `luaopen_*` row gives 106 roots), `JNI_OnLoad`,
  `RedisModule_OnLoad`, `sqlite3_*_init`, `ERL_NIF_INIT` `load`/`upgrade` by
  name; none is a rule. One row family, detected by the host header. Prototyped.
- **Attribute capture gaps** (extraction). Not recorded: C/C++/ObjC `[[…]]`,
  `__attribute__((constructor/destructor))` (then root them under
  `c-runtime`/`cpp-runtime`), `__declspec`, `IBAction`; every Swift attribute
  (`@Test`, `@IBAction`, `@main`, `@objc`); Dart annotations but `@override`
  (`@pragma('vm:entry-point')`, `@Route.get`, `@riverpod`); PHP class attributes
  and fully qualified `#[\A\B(…)]` (`attribute_verb` splits on `.`/`:` only);
  Java/Kotlin class annotations as containers (Kotlin pins them on the class
  callable and mints route-prefix entries; needed for class-level
  `@Path`/`@RequestMapping` prefixes); C# attributes on fields, properties and
  parameters (`ContextMenuItem`, `ValueSource`, `ParamsSource`; Java JMH
  `@Param` and CDI `@Observes` too); Perl `:Path(…)` sub attributes (already in
  `signature_hint`); Clojure `^:export`; Lean keys (`attribute_key("@[export]")`
  is `[export`); F# `let [<A>] f`; PowerShell `[PoshBot.BotCommand()]`.
  Annotation import aliases are ignored (Kotlin, TS, Scala; Groovy is listed)
  and .NET `TestAttribute` keeps its suffix. VHDL/Verilog kind tags count as
  attributes (add to `KEYWORD_MARKERS`; extends the listed Verilog `task` item).
  Reproduced (fixtures).
- **C-family value references** (C, C++, ObjC; extraction). Registrar arguments
  bypass `c_values`: `&h` and casts are dropped (`signal(SIGTERM, &h)`;
  erlang-otp `qsort(…, &compare)` reports `compare` dead) and locals are not
  pruned (`qsort(items, count, …)` roots a function `count()`); `(DL_FUNC) &f`
  parses as `T & f`, so every R `R_CallMethodDef` and `(PyCFunction) &f` table
  reads dead (ranger 124 -> 72 with a fix); ObjC message arguments (Hammerspoon:
  112 of 221 dead C functions) and ALL-CAPS macro arguments (`G_CALLBACK`,
  `RUN_TEST`, `EXPORT_SYMBOL`, `BENCHMARK`) are no values. Land the first two
  before any new C registrar verb. Reproduced (fixtures, erlang-otp, ranger).
- **`value_refs` capability undeclared** (C, C++, ObjC, Swift, Dart, Scala, Lua,
  Zig, Erlang, Groovy, Clojure, OCaml, Haskell, F#; caps). `caps.rs:153` infers
  support from whether a `Reference` edge landed, so findings carry
  `language-lacks-value-references` in a tree without one. Set the flag.
  Reproduced (fixture).
- **`super` into an out-of-tree base** (Dart, TS/JS, Swift, ObjC; resolver).
  `super.dispose()` binds another file's override by name at medium (immich
  `reassemble` both ways; ObjC `[super setUp]` reaches an unrelated test class).
  Extend `super-base-out-of-graph` beyond Python, Kotlin, Java and C#.
  Reproduced (fixtures, immich).
- **Twin rows drift** (JVM, .NET, JS/TS; table plus a test). Scala lacks 43 and
  Groovy 42 of the shared Java/Kotlin ids (`junit`, `testng`, `jmh`, `guice`,
  `jsr330-inject`, `servlet`, `spring`, `micronaut`…; add groovy to
  `IMPORT_SCOPED_ATTRIBUTES` with them); `java-serialization` has no Kotlin,
  Scala or Groovy row (`signature_methods_for` keys `java`); `gradle-plugin` is
  Groovy-only; Kotlin `spring-batch` and the Kotlin Android service rows lost
  Java's bases and methods; the Gradle task annotation lists differ in all three
  languages; F# lacks `aspnet-mvc`, `dotnet-hosting`, `benchmarkdotnet`,
  `mstest`, `xunit`, `orleans`, `avalonia`, `signalr`, `aws-lambda`; `polka` has
  no TS row. Copy the rows and add a test comparing twin rows. Reproduced
  (prototype rows).
- **Coverage disclosure** (coverage output). A language whose rules never detect
  counts as covered (`bash-dispatch`: acme.sh prints nothing); frameworks with
  no rule print nothing (Qt, Unreal, ROS 2, gRPC C++, Drogon, Boost.Test, AVA,
  bun, QUnit, Kotest, IHP, Lapis); "detected, no entry" fires on client imports
  (`FileProvider`, `AppWidgetManager`) and on matches another rule evicted
  (django, aws-lambda); a detected file that failed to parse loses entries
  silently (two Dancer2 hooks drop 17 of 23 routes; a PowerShell backtick
  continuation drops `Set-TargetResource`). Count never-detected languages as
  uncovered, add detect-only rows, require a listed base before the gap, count
  evicted matches, note parse errors. Reproduced (fixtures, bash-acme,
  app-perlmaven-dancer2).
- **Rule tests** (all). `detect_prefixes.rs:153` writes only each rule's first
  prefix in one import form, so later prefixes are never fire-tested (`typer`,
  `lightning`, `@nestjs/core`, `Mojo::Base`, `com.amazonaws.lambda`…). No
  end-to-end test touches a rule in Elixir, Erlang, Perl, R, Julia, PowerShell,
  Zig, Lua, Clojure, OCaml, Bash, CMake, Nix, Starlark, ObjC or Dart; most rules
  elsewhere are untested (Python web, hono/polka/restify, nestjs, Java/Kotlin
  server, C# lifecycle and server, Go routers, actix/rocket, sinatra/grape,
  wordpress, Swift `xctest`, C/C++ callbacks, Spock fixtures), and no negative
  test pins a name-only or generic-base rule. Test every prefix and form, then
  one fixture per rule shape plus a negative case, seeded from the audit
  fixtures. Code only.
- **APPS claims carry no language** (corpus tooling). A Swift twin satisfies the
  ObjC `uikit-*` and `xctest` claims, and Kotlin, Scala, Groovy and F# twins are
  never checked (`pair_gaps`). Let a claim name its language; per-language claim
  fixes are in the sections below. Code only.
- **Stale gap text and docs** (extends the listed stale-gap item). Gap text now
  false: `pytest`, `nicegui`, `panel`; `koa`, `hapi`, `nuxt`,
  `cloudflare-workers`, `react-hooks`, `svelte`; `java-runtime`;
  `kotlinx-coroutines`, `kotlin-runtime`, `android-compose`; `wpf-winforms`,
  `maui`, `avalonia-xaml`, `xunit`, `nunit`, `blazor-lifecycle`; `cobra`,
  `urfave-cli`, `go-signal`, `machinery`; `rspec`, `rake`, `roda`, `padrino`;
  `laravel-events`, `yii`, `slim`, `drupal`, `magento`, `swoole`; `play-mvc`,
  `play-module`, `akka-*`, `spark`, `specs2`, `scalatest`; `vapor`, `combine`,
  `swiftui`; `c-runtime`, `cpp-runtime`; `erlang-otp-lifecycle`,
  `erlang-application`, `cowboy`; `flutter`, `flame`; `giraffe`, `suave`,
  `saturn`; `yesod`; `ring`, `http-kit`; `dream`, `opium`, `alcotest`,
  `cmdliner`; `catalyst`, `plack`, `test-more`; `plumber`, `r-s3-dispatch`;
  `genie` (credits Oxygen's `@get`); PowerShell `azure-functions`; `zig-export`
  (`callconv(.c)`); `kong`, `openresty`; `bash-dispatch`; `cmake`; `bazel`;
  `vhdl-toplevel`; `fortran-program`. README: Cloudflare/Deno "default export",
  the coverage example (bash, a false wordpress line), CUDA under shape F
  without `.cu`, Elixir import list without `use`; `app.use(fn)` comments at
  `registrar.rs:228` and `frameworks.rs:1128`. dev_docs: `rules-part-*.md` line
  numbers run ~230 lines behind, `rules-part-3.md:338` (py-argparse); add matrix
  section 10 (Kotlin, Swift, C++, Dart, OCaml, Bash rows) to the listed re-score
  item. Code only, checked on fixtures.
- **Agent-section items with more evidence here** (land once). Minted closures
  keep their context: JS `stream.on`, file-scope `process.on` and
  `parentPort.on` closures orphan their callees. Base types the plugins lose:
  member-less bases (`include Sidekiq::Worker` gives diaspora 0 sidekiq entries,
  Python `class Base(View): pass`, Swift `EmptyBase: XCTestCase`, Dart
  `BaseState`, C# Ombi roots 3 of ~30 Quartz jobs; seed `by_owner` from
  `FileFacts::classes`, generalising the listed Dart item). Curried registrars:
  `typer.run(main)`, Slack `app.action(x)(fn)`. Constructed handler objects: Go
  `RegisterActivity(&Acts{})`, `RegisterXServer(s, &srv{})`, robfig `AddJob`,
  buffalo `Resource`; C# `MapHub<T>()`, `MapGrpcService<T>()`,
  `UseMiddleware<T>()`, `UseStartup<T>()`. One verb in two rules: symfony with
  symfony-messenger `AsMessageHandler`, bullmq with node-event-emitter `on`,
  JAX-RS `@GET` plus `@Path`. Class-level markers naming members: PHP class
  attributes, Java/Kotlin class annotations. Path and config entry points: the
  Next.js family.

### Python

- **`mock.patch` is an HTTP verb.** `@patch(...)`/`@mock.patch(...)` tests
  become fastapi PATCH routes (Netflix dispatch 24; revel-backend 1,172 test
  false positives), and every rule with a `patch` attribute is exposed. Skip
  decorators imported from `unittest.mock`. Reproduced (fixture).
- **fastapi `add_route` fires on any call.** jupyterhub's `Proxy.add_route(...,
  service.name)` mints a fastapi entry for `Spawner.name` in a file without
  fastapi. FastAPI uses `add_api_route`; drop `add_route` (falcon and robyn
  carry it too) or gate it per file. Reproduced (fixture,
  app-jupyterhub-tornado).
- **django registrars and `urls.py` leak.** `atexit.register(f)` and
  `registry.register(fn)` become django routes, a project's own `path("x", fn)`
  too, and a `urls.py` without django turns the rule on. Limit `register` to DRF
  routers, `path`/`re_path`/`url` to files importing
  `django.urls`/`django.conf.urls`, and require a django import beside
  `urls.py`. Reproduced (fixture).
- **A decorator entry evicts the `path()` route.** `dedup` keeps the strongest
  shape per target across rules, so `@login_required` drops `path("acct/",
  views.account)` and its route; coverage then prints aws-lambda under "seen, no
  rules" when powertools already claimed `lambda_handler`. Keep or merge the
  route. Reproduced (fixtures).
- **Queue route labels take any literal.** `attribute_string_arg`
  (`roots.rs:318`) takes the first literal anywhere: huey
  `periodic_task(crontab(minute="0"))` keys as `"0"`, rq `enqueue(f,
  "http://…")` by its payload (three targets on one node), Airflow
  `@task.virtualenv(requirements=[…])` by a requirement. Take a first positional
  literal only, or no route for Queue/Schedule rules. Reproduced (fixture).
- **`from aiohttp import web` records `aiohttp`.** `parse_import` keeps the
  package, so an `aiohttp.web` prefix never fires and 8 corpus apps detect
  aiohttp from client use only. Record one path per imported name, or detect on
  `web.Application(`/`RouteTableDef(`. Reproduced (fixture).
- **Web rule rows.** fastapi `on_event` plus
  `add_event_handler`/`add_exception_handler`; flask/quart
  `teardown_appcontext`, `context_processor`,
  `template_filter`/`_global`/`_test`, `url_value_preprocessor`, `url_defaults`,
  `shell_context_processor`, the Blueprint `app_*`/`before_app_request` set,
  `register_error_handler`, `View.dispatch_request`, `@app.cli.command`, quart
  `while_serving`/`teardown_websocket` (13 findings); falcon `on_get_<suffix>`
  responders and
  `process_startup`/`_shutdown`/`_request_ws`/`_resource_ws`/`*_async`; robyn
  `@app.websocket` with `on_connect`/`on_close`. Reproduced (fixtures).
- **django CBV/DRF hooks and `require_*`.** `get_queryset`, `get_context_data`,
  `form_valid`, `get_success_url`, `dispatch`, `get_serializer_class`,
  `perform_create`/`_update`/`_destroy`, `require_GET`/`_POST`/`_safe` and DRF
  `action` are dead at medium; `handler404 = "app.views.f"` strings too (not a
  row). Reproduced (fixture).
- **Cloud rule rows.** azure-functions `generic_`, `kafka_`, `sql_`, `warm_up_`,
  dapr and assistant triggers, Durable
  `orchestration_`/`activity_`/`entity_trigger` (detect
  `azure.durable_functions`); firebase-functions lists 11 of ~40 decorators
  (`on_document_*_with_auth_context`, `on_value_written`/`_deleted`,
  `on_object_*`, `before_user_*`, `on_task_dispatched`, `on_alert_published`,
  `on_config_updated`, `on_custom_event_published`; JS/TS twin below);
  powertools `resolver`/`batch_resolver`/`async_batch_resolver` and
  `idempotent`/`idempotent_function`; dash `layout(**kw)` in a `register_page`
  module (needs the path/owner gate). Reproduced (fixtures).
- **Plain boto3 Lambdas are undetected.** Detect is
  powertools/aws_lambda_typing/awslambdaric, pinned by
  `a_lambda_handler_outside_a_lambda_project_is_not_an_entry`; reading the
  `handler:` key of `serverless.yml`/SAM templates is a new file type (minor),
  and serves JS/TS `handler` names too. Reproduced (fixture).
- **celery signals and Task hooks.** `@app.on_after_configure.connect` has verb
  `connect`, so the listed `on_after_configure` never fires
  (`setup_periodic_tasks` dead; app-hypothesis-pyramid `h/tasks/celery.py:160`);
  `@worker_ready.connect`-style receivers and
  `on_failure`/`on_success`/`on_retry`/`after_return`/`before_start` are dead
  too. Use `*.<signal>.connect` patterns per Celery signal (never bare
  `connect`) and add the hooks. Reproduced (fixture, 7 findings cleared).
- **Library rule rows.** `atexit.register` as an attribute (not bare
  `register`); `Thread`/`Process` bases with `run`; sqlalchemy `@validates`,
  `@reconstructor` and a qualified `event.listen` registrar; django-admin
  `handle_label`/`handle_app_config`; torch `autograd.Function`
  `forward`/`backward`/`setup_context`, `extra_repr` and Lightning hooks,
  `LightningDataModule`/`Callback` bases (bare `Function` also matches sympy:
  check the corpus); scrapy `Rule(callback="parse_item")` with `string_targets`
  and pipeline/middleware methods; dramatiq `Middleware` hooks and
  `GenericActor.perform`; huey `context_task`; apscheduler `add_listener`; a
  qualified `typer.run` registrar. Reproduced (fixtures).
- **kivy ignores KivyMD.** `MDApp`/`MDScreen`/`MDTextField` subclasses match no
  base, so every KivyMD `build`/`on_start` is dead; `insert_text`,
  `build_config`, `build_settings` and `register_event_type` events are missing.
  Add `kivymd` to detect, the MD and `*Behavior` bases and the methods.
  Reproduced (fixture; app-pywallet-kivy `textfields.py:13`; MYScrcpy 256 -> 145
  findings).
- **marimo over-roots.** `@app.function`/`@app.class_definition` mark importable
  definitions; only `@app.cell` runs, so an unused helper is never reported.
  Keep `cell` (`function` is also `@tf.function`). Reproduced (fixture).
- **pytest and unittest gaps.** `pytest_*` hooks in a plugin module are
  production findings (add the pattern); a plain-assert suite with no `import
  pytest` gets no harness (add `detect_paths: ["conftest.py"]` or detect by test
  role); `setUpModule`, `tearDownModule`, `load_tests`, `setUpTestData` and the
  Django/DRF bases `SimpleTestCase`, `TransactionTestCase`, `APITestCase` are
  missing from `py-unittest`. Reproduced (fixtures).
- **`py-setuptools-entrypoints` is silent for pyproject-only packages.** Decode
  `name = mod:func` strings in `setup()` dicts (patch); pyproject/setup.cfg
  parsing is a new file type (minor). Reproduced (fixture).
- **`py-context-manager` duplicates the dunder roots.** `roots.rs:1035` roots
  every Python dunder, so the rule changes nothing but prints "detected, no
  entry" in 20 apps importing `contextlib`. Retire it. Code only.
- **Detect-only rules a row can enumerate.** tornado
  `RequestHandler`/`WebSocketHandler` verbs and hooks (jupyterhub 520 -> 436
  findings; route tuples stay unread); sanic, pyramid, starlette, bottle
  decorators, `add_route`/`add_view`/`Route` registrars with
  `endpoint=`/`callback=`, `HTTPMethodView`/`HTTPEndpoint`/`BaseHTTPMiddleware`
  bases (pyramid `includeme`, `@view_defaults` stay open); gradio, nicegui,
  panel, streamlit keyword callbacks (sdwebui-gradio 27 findings; `@ui.page`);
  airflow `dag`/`task`/`task.*`/`task_group`, `PythonOperator`
  `python_callable=`, `BaseOperator.execute`, prefect `flow`/`task`, dagster
  `asset`/`op`/`job`/`schedule`/`sensor` (basedosdados 319 -> 155,
  oldata-dagster 187 -> 136). Generic verbs land after per-file gating.
  Reproduced (fixtures, corpus).
- **APPS claims.** pytest fires unclaimed on many real apps;
  aws-lambda-powertools, python aws-lambda, aws-cdk, firebase-functions, dash,
  litestar, scrapy, apscheduler, py-atexit and py-setuptools-entrypoints rest on
  framework repos or official samples; no Python app claims azure-functions or
  airflow, or uses aiohttp as a server, streamlit, starlette or bottle. Code
  only.

New coverage (ranked):

- **P2 pluggy** (public; marker row `hookimpl`) 1.33B dl/month; llm 20 entries.
- **P2 modal** (queue; row) `@app.function`/`cls`,
  `@modal.method`/`enter`/`fastapi_endpoint`; `.remote()` is unbound, so all
  read dead. Fixture only.
- **P2 ray** (queue; row plus class decorators as containers) `@ray.remote`
  functions dead; actors need the container (Robot `@library` too).
- **P2 luigi** (schedule; row) Task `run`/`requires`/`output`/`complete`.
- **P2 aiogram 3, python-telegram-bot** (network; rows) `@router.message`,
  `dp.message.register`; handler constructors as registrars (already live;
  entries only). remnawave-bedolaga 1,078, chatgpt_telegram_bot 17.
- **P2 python-socketio, Django Channels, FastStream** (network/queue; rows)
  `@sio.event`/`on`, Namespace `on_*` (JS socket-io twin); Channels consumer
  bases; `@broker.subscriber`. metube 1, socialhome 5.
- **P2 sublime-text, nox, robot, asv, python-fire** (rows) `*Command.run`,
  `EventListener.on_*`, `plugin_loaded`; `@nox.session`; `@keyword`; asv
  `time_*`/`mem_*`/`peakmem_*`/`track_*`; `Fire(Cls)` methods (matcher change).
- **P2 pyuvm** (test; row; `pyuvm`) `pyuvm.test`, `uvm_*` bases, `*_phase`,
  `body`, `write`; 403k/month. Fixture only.
- **P3 locust, metaflow, kfp, hypothesis stateful, arcade/pyglet, wx, injector,
  kedro, Twisted, Chalice, taskiq, arq, schedule, Hatchet, Telethon, CherryPy,
  pika/kombu/Pub/Sub callbacks** (rows) small gain or already live.

Corpus additions:

- qutebrowser/qutebrowser (11.8k; qt, pytest-bdd; 598 medium findings)
- tconbeer/harlequin (6.5k; textual, py-atexit; textual silent)
- dougiteixeira/proxmoxve (1.0k; home-assistant; 127 medium)
- ucupumar/ucupaint (2.3k; blender-addon; 1,224 medium)
- Cog-Creators/Red-DiscordBot (5.8k; discord.py; click mislabels ~600)
- simonw/llm (12.8k; pluggy, pydantic, click)
- jrnl-org/jrnl (7.4k; pytest-bdd)
- OCA/project (431; odoo addon collection; 135 findings)
- SeldonIO/MLServer (904; grpc-python)
- TracecatHQ/tracecat (3.9k; temporal; MCP side in the agent section)
- python-discord/bot (1.6k; discord.py; 779 dead)
- father-bot/chatgpt_telegram_bot (5.6k; python-telegram-bot)
- BEDOLAGA-DEV/remnawave-bedolaga-telegram-bot (1.2k; aiogram)
- letsrevel/revel-backend (122; django-ninja)
- alexta69/metube (15.3k; aiohttp server, socketio)
- fpganinja/taxi (1.0k; cocotb, HDL toplevels; 514 Verilog findings)
- apfaudio/eurorack-pmod (243; cocotb; 7 tests unrooted)
- me2sy/MYScrcpy (114; KivyMD)

### JavaScript and TypeScript

- **Name-only `methods` match any callable** (`cloudflare-workers`, `deno-http`,
  `aws-lambda`). Every `fetch`/`scheduled`/`queue`/`email`/`trace`/`tail`
  becomes a network entry (`User.tail`, `Mailer.queue`), and aws-lambda takes a
  Next.js `export default function handler`. Bind only on the default-exported
  object or a `DurableObject`/`WorkerEntrypoint`/`WorkflowEntrypoint` subclass,
  whose `alarm`, `webSocketMessage`/`Close`/`Error`, `connect`, RPC methods and
  `run` are dead today. Reproduced (fixtures, prototype).
- **Bare `on` and `process` claim every emitter** (`hono`, `bullmq`,
  `node-event-emitter`). `process.on('SIGTERM', f)` becomes `network::hono::on`
  and `queue::bullmq::on` (38 on app-calcom-nextjs), and browser
  `$(document).on`/`socket.on` become lifecycle entries; with node-event-emitter
  the handler gets a second node. Drop `on` from bullmq, accept hono `on` only
  with an HTTP-method string. Bull's own `queue.process(fn)` and `process(5,
  fn)` are dropped as ambiguous: corroborate by a `new Queue(…)` receiver, a
  numeric first argument or a lone function. Reproduced (fixtures).
- **Route objects and controller tuples.** hapi `server.route({ method, path,
  handler })` (and arrays) and fastify `route({ url, handler })` capture the
  handler but no route, so the gate drops them: read `path`/`url`. AdonisJS
  `router.get('/x', [Controller, 'index'])` leaves every action dead because
  `class_method_pair` pairs only PHP `C::class`. Reproduced (fixtures).
- **Object-literal handlers are not callables.** Arrow-valued properties make no
  callable and method shorthand is extracted under its bare key
  (`'tasks.insert'` with quotes; `author` without `Query.`), so
  `Meteor.methods`, Apollo/graphql-yoga resolver maps, Feathers services,
  `bun-serve`, `exposeInMainWorld('api', { openFile })` and Redux `createSlice`
  reducers read dead. Mint key-path-qualified callables and match an object
  passed to a registrar. Overlaps the agent section's Options-object handlers.
  Reproduced (fixtures).
- **`.bind` and named function expressions.** `app.get(p, ctl.m.bind(ctl))`
  mints no entry and `m` reads dead (Palmr: 96 controller methods with a fastify
  row); `new CronJob(e, function hourly(){})` leaves `hourly` dead. Reproduced
  (fixtures).
- **Cloud rule rows.** azure-functions v4
  `get`/`post`/`put`/`patch`/`deleteRequest`, `generic`, `sql`, `mySql`,
  `warmup`, `webPubSub`, `connectorTrigger`, `mcpTool`/`Resource`/`Prompt`,
  `app.hook` hooks (after per-file gating: `get`/`post` widen false entries);
  firebase v2 `onCallGenkit`, `onTaskDispatched`, `before*` blocking functions,
  `onDocument*WithAuthContext`, `onValueWritten`/`Deleted`, `onObject*`,
  `onCustomEventPublished`, `onConfigUpdated`, `onTestMatrixCompleted`, alerts
  (Python twin above). Reproduced (prototypes, upstream `src/app.ts`).
- **Deno and import specifiers.** `Deno.serve(handler)` with no std `http`
  import is undetected (needs the qualified `detect_calls` form);
  `npm:express@4`, `jsr:@hono/hono` and `https://deno.land/x/hono@v3/…` never
  match a prefix (`prefix_keys`, `mod.rs:510`): normalise JS/TS specifiers once.
  Reproduced (fixtures).
- **Worker module paths.** `resolve_module_path` (`mod.rs:1765`) takes a
  relative literal only, so `new Worker(path.join(__dirname, 'w.js'))`, `new
  URL('./w.mjs', import.meta.url)`, Piscina `filename: path.resolve(…)` and Bull
  `queue.process(__dirname + '/p.js')` enter nothing; a worker whose only body
  is `parentPort.on('message', …)` yields no entry (`match_self_modules` needs a
  Function surface). Read the last string literal. Reproduced (fixtures).
- **React.** Class `render` is never rooted: add a base-typed row (`Component`,
  `PureComponent`, `React.*`) with `render` and the hooks, node false. JSX
  element names (`<UserCard />`) are no value refs, so every function component
  is never-referenced: record capitalised and member tags. `react-hooks` can
  enumerate `useEffect`, `useLayoutEffect`, `useCallback`, `useMemo`,
  `useReducer`, `useSyncExternalStore`, `useImperativeHandle` callbacks.
  Reproduced (fixtures, prototype).
- **Vue and Svelte.** Options-API members (`setup`, `mounted`, …) are callables
  in `.js`/`.ts` and composition registrars `onMounted`, `watch`, `watchEffect`
  (Svelte `onMount`, `onDestroy`) enter callbacks, but `setup`/`created` need
  per-file gating and the two must ship together (rooting closures alone makes
  callees of the unrooted `mounted` new findings); Svelte 5 `*.svelte.js` rune
  modules import nothing and are undetected; `data()` and template methods need
  `.vue` parsing (minor). Reproduced (prototype).
- **electron main process.** Only ipc, `app` and `contextBridge` are covered:
  add `webContents.on`, `win.on`, `globalShortcut.register`, `protocol.handle`,
  `autoUpdater.on`, `powerMonitor.on`, `nativeTheme.on`, `screen.on`, Menu
  `click:`, `app.whenReady().then(fn)`; the single kind is `queue`, also for
  `app.on('ready')`. Reproduced (fixture; app-insights-feathers
  `mainWindow.on`).
- **react-native listeners.** No rows for
  `AppState`/`Linking`/`BackHandler.addEventListener`, `Keyboard.addListener`,
  expo `TaskManager.defineTask`; `registerComponent(name, () => App)` enters a
  closure with no edge to `App`. Reproduced (fixture).
- **NestJS.** Missing method decorators `Search`, `QueryMethod`, WebDAV
  (`Propfind`, `Proppatch`, `Mkcol`, `Copy`, `Move`, `Lock`, `Unlock`),
  `GrpcMethod`, `GrpcStreamMethod`, `GrpcStreamCall` and `@nestjs/graphql`
  `Query`/`Mutation`/`Subscription`/`ResolveField`/`ResolveReference`
  (hoppscotch: 234 entries, 342 -> 183; parameter `@Query()` is not recorded, so
  safe); lifecycle hooks `onModuleInit`, `onApplicationBootstrap`,
  `onModuleDestroy`, `beforeApplicationShutdown`, `onApplicationShutdown` as a
  methods-only row (base types disable methods-only matching, `mod.rs:1074`;
  ghostfolio 11 dead `onModuleInit`), plus guard/interceptor/pipe/filter/gateway
  hooks; new rows `nestjs-events` (`OnEvent`; immich 84 sites), `nestjs-bull`
  (`Process`, `OnQueue*`, `OnGlobalQueue*`, `OnWorkerEvent`,
  `WorkerHost.process`), `nestjs-cqrs`
  (`ICommandHandler`/`IEventHandler`/`IQueryHandler` `execute`/`handle`,
  `Saga`). TS only. Reproduced (fixtures, ghostfolio, calcom).
- **Angular.** The `angular` row has base types, so `ngOnInit` and kin root only
  with `implements` (ghostfolio: 9 dead): split the eight `ng*` names into a
  methods-only row. Class decorators (`@Component`, `@Injectable`, `@Pipe`,
  `@Controller`) reach no container, so DI-built constructors and the
  `effect(…)` closures they register read dead: add `container_attributes_for`
  rows for Angular and Nest. Template bindings (`(click)="save()"`, `host:`) are
  unread and the rules have empty gaps: add gap text, read inline
  `template:`/`host:` (patch); sibling `.html` is minor. TS only. Reproduced
  (fixtures).
- **Detect-only rules a row can enumerate.** koa (express verbs plus `del`,
  `param`; outline 526 entries), fastify (verbs, `addHook`, `register`,
  `setErrorHandler`), hapi `ext`, oak (app-edrys-oak 14 routes), socket-io `on`,
  trpc `.query`/`.mutation` (generator subscriptions do not bind:
  `CLOSURE_KINDS`), meteor `publish`/`startup`/`connectHandlers.use`; hono
  `openapi` (`@hono/zod-openapi`), loopback `@operation` and `LifeCycleObserver`
  `start`/`stop`. Ship after per-file gating (`query`, `on`, `register`).
  Reproduced (prototypes).
- **File-convention frameworks.** Next.js `getServerSideProps`,
  `getStaticProps`, `getStaticPaths`, `generateStaticParams`,
  `generateMetadata`, `generateViewport` are safe names now (calcom: ~99 medium
  findings); route handlers `GET`/`POST` and `middleware` need
  `methods_case_sensitive` and the path gate (a name-only prototype took
  `Cache.get`); same for remix, sveltekit, astro. Nuxt: a
  `defineEventHandler`/`eventHandler` registrar binds; add `h3` to detect.
  Parsing `.svelte`, `.vue`, `.astro` is minor. Reproduced (fixtures).
- **Test harness shapes.** Parameterised and modifier chains (`test.each(t)(…)`,
  `describe.each`, `it.for`, `test.skipIf(c)(…)`, `test.failing`, `test.fails`,
  `test.fail`, `onTestFinished`, node:test `t.test`) fail `registrar::admits`
  (it takes the last callee segment, `each([..])`): strip argument lists per
  segment, add the verbs, admit `<param>.test`. Playwright `test.extend`
  fixtures and specs importing `test` from `./fixtures` (then labelled jest)
  need a one-hop import follow; `HarnessChoice` should prefer the nearest
  config. Config-named modules (`globalSetup`, `globalTeardown`, `setupFiles`)
  and Mocha `mochaHooks`, `mochaGlobalSetup`, `mochaGlobalTeardown` read dead.
  Test files miss `.cy.*` outside `e2e/`, `.bench.*`, `.e2e-spec.ts` (Nest's
  default), `.test-d.ts`, Playwright `*.setup.ts` and `import { test as setup
  }`. Reproduced (fixtures).
- **AVA, bun:test, node-tap, QUnit, Deno.test have no rule.** A tree tested only
  with one shows `recognised (none)` and no gap. Add rows; `QUnit.test`,
  `t.test`, `Deno.test` need a receiver allowance in `admits`. Reproduced
  (fixtures).
- **APPS claims.** app-edrys-oak should claim `~oak`, app-calcom-nextjs
  `~nextjs`; the `express` claims of ghostfolio and cal.com are met only by
  false entries; app-mtgnode-sails claims nothing. polka, restify and
  gcp-functions enumerate only on their own repos; hono, firebase, azure on
  samples; `bullmq` JS, `node-test` and the electron JS row lack a real app; no
  TS app for restify, azure/gcp functions, koa, fastify, hapi or remix; jest,
  mocha, vitest and react claims sit on framework or library repos; nothing
  claims vue or svelte. Code only.

New coverage (ranked):

- **P2 Tagged-template value refs** (extraction) `${this._onClick}` in `html`
  templates records nothing: Lit, lit-html, FAST.
- **P2 discord.js, Telegraf/grammY, Slack Bolt JS** (network; rows plus route
  gate) command `execute`/`autocomplete`, `on`/`once` (evobot 31);
  `command`/`hears`/`on`/`action` (opencode-telegram-bot 285 entries, ~49 real;
  `on`/`use` need a filter string); Bolt verbs need a string (OpenPanel: 136
  false entries from a client import).
- **P2 Mongoose middleware** (lifecycle; row plus assignment matcher)
  `pre`/`post` closures; `schema.methods.x = fn`, `statics`. LibreChat 317
  entries.
- **P2 Node schedulers and job queues** (schedule/queue; rows) node-cron, cron
  `CronJob`, node-schedule, croner, bree, agenda, pg-boss `work`, bee-queue,
  `@nestjs/schedule` `addCronJob`; `cron.schedule` on a default import and
  `CronJob.from({ onTick })` did not match a prototype. Shelf 4 entries.
- **P2 Kafka/AMQP/SQS/Pub/Sub/NATS consumers** (queue; options-key matcher)
  `eachMessage`/`eachBatch`/`handleMessage`; verb rows are noise (uptime-kuma
  172 false); amqplib `consume("q", fn)` fits today.
- **P2 Temporal, Trigger.dev, Inngest** (queue; rows plus a matcher) Temporal
  `workflowsPath` exports and the `activities` namespace (3 dead on a fixture);
  `task`/`schemaTask` (calcom has 6 files), `createFunction`.
- **P2 commander, yargs** (cli; rows) `action`, `hook`; yargs `command`,
  `commandDir`, `exports.handler`; 786M/392M per week.
- **P2 Web extensions, Redux Toolkit, Capacitor** (lifecycle)
  `chrome.*.addListener` files import nothing (detect on the callee head;
  manifest.json is minor; darkreader 35); `createSlice` reducers need the
  object-handler fix; `App.addListener('backButton', fn)`.
- **P2 Stencil, Stimulus, Cocos Creator, Ionic** (lifecycle, node false; rows)
  `Watch`/`Listen`/`Method`; `connect`/`*TargetConnected` (actions via
  `data-action` are minor); `onLoad`/`start`/`update`; `ionView*`.
- **P2 Cucumber.js** (test; row) `Given`/`When`/`Then`/`Before`/`After`; step
  files are not test-classified.
- **P2 gRPC-js, Connect** (network; object-map extraction) `addService(def, { m
  })`, `router.service(S, { m() {} })`.
- **P3 GNOME Shell extensions, MikroORM, Objection, n8n nodes, Node-RED,
  tapable, Storybook, Cypress commands, Apps Script, json-rpc-2.0 `addMethod`,
  probot `app.on`, Encore `api()`, Moleculer, Vercel/Netlify/SST handler
  files.**

Corpus additions:

- heroku/cli (887; oclif; 343 `run`-family findings dead)
- outline/outline (41.4k; koa, sequelize-typescript, socket.io, bullmq)
- workadventure/workadventure (6.0k; Phaser; svelte seen in 348 files)
- blacksmithgu/obsidian-dataview (9.5k; Obsidian)
- danny-avila/LibreChat (47.5k; Mongoose; 68 s at `--jobs 1`)
- darkreader/darkreader (22.6k; web extensions)
- eritislami/evobot (1.9k; discord.js; last commit 2024-06)
- paperwm/PaperWM (4.4k; GNOME Shell)
- Openpanel-dev/openpanel (7.3k; Elysia, kafkajs, trpc)
- C4illin/ConvertX (19.8k; Elysia)
- retracedhq/retraced (450; tsoa)
- kyantech/Palmr (2.4k; fastify, `.bind` controllers)
- louislam/uptime-kuma (94.8k; socket.io JS, croner)
- grinev/opencode-telegram-bot (1.4k; grammY)
- Shelf-nu/shelf.nu (3.1k; pg-boss, remix)

### Java

- **`swing` roots any listener-named method.** 44 `methods` and no `base_types`,
  so every `drop`, `valueChanged`, `stateChanged`, `dragOver` roots once any
  file imports `javax.swing` (`javax.swing.undo` counts; JabRef's two `swing`
  entries are JavaFX `dragOver`). Move the listener interfaces and adapters into
  `base_types` and add models, renderers and `SwingWorker` (`getRowCount`,
  `getValueAt`, `getElementAt`, `get*CellRendererComponent`, `doInBackground`,
  `done`, `process`). Java and Kotlin. Reproduced (fixture).
- **`java-concurrent` misses executor callbacks.**
  `RecursiveTask`/`RecursiveAction`/`CountedCompleter.compute`,
  `ThreadFactory.newThread`, `uncaughtException`, `rejectedExecution`; the
  `java.lang.Thread` prefix only matches a redundant import. Java and Kotlin.
  Reproduced (fixture).
- **Android rows miss common callbacks.** `android-activity`: base
  `ReactActivity`, `onCreateOptionsMenu`, `onPrepareOptionsMenu`,
  `onOptionsItemSelected`, context menus, `onSupportNavigateUp`, `onPostCreate`,
  `onWindowFocusChanged`, `onKeyDown`/`Up`; `android-fragment`:
  `onCreatePreferences`, `onHiddenChanged`, `onActivityResult`,
  `onRequestPermissionsResult`; `android-service`: `TileService`
  `onClick`/`onStartListening`, `onInterrupt`, `onNotificationPosted` (Wear
  tiles and complications too: home-assistant 27). Generic names only behind
  base types. Java and Kotlin. Reproduced (fixtures).
- **No rows for adapters, view listeners, ViewModel, FCM.** Lifecycle rows (node
  false): `RecyclerView.Adapter` (qualified), `BaseAdapter`,
  `View.OnClickListener`, `TextWatcher`, `ViewModel.onCleared`,
  `DefaultLifecycleObserver`/`onStateChanged`, `ActivityLifecycleCallbacks`,
  `FirebaseMessagingService.onMessageReceived`/`onNewToken`. Java and Kotlin;
  prototypes: home-assistant 8 lifecycle and 4 FCM roots, mihon 7,
  app-wordpress-android 66 and 2. Reproduced (fixture, prototypes).
- **`spring-context` misses container callbacks.** Bases `ApplicationListener`
  (openboxes: 11 dead; Groovy too), `CommandLineRunner`/`ApplicationRunner`,
  `InitializingBean`, `DisposableBean`, `SmartLifecycle`, `BeanPostProcessor`,
  `SmartInitializingSingleton`, `*Aware`; attributes `Autowired` and Modulith
  `ApplicationModuleListener` (a `high` finding today); stereotype beans
  (`@Component`, `@Service`, `@RestController`) as root marks for their
  constructors. Java, Kotlin, Groovy. Reproduced (fixture).
- **`guice`, `dagger`, `jackson`, Gradle rows.** guice
  `AbstractModule`/`PrivateModule`/`ServletModule`
  `configure`/`configureServlets`; dagger `AssistedInject`; jackson `JsonKey`
  and `StdSerializer`, `JsonSerializer`, `StdDeserializer`, `JsonDeserializer`,
  `KeyDeserializer` (3.x `ValueSerializer`) with databind detect prefixes;
  Gradle `@Option`/`@OptionValues`,
  `WorkAction`/`TransformAction`/`ValueSource`. Java and Kotlin. Reproduced
  (fixtures).
- **Test rules miss documented shapes.** junit
  `BeforeParameterizedClassInvocation`/`AfterParameterizedClassInvocation`
  (JUnit 5.13, `high` today), `DataPoints`/`DataPoint`, `suite`; JUnit 5
  extension interfaces (`BeforeEachCallback`, `ParameterResolver`,
  `TestExecutionExceptionHandler`) and JUnit 4 `TestRule`, `MethodRule`,
  `ExternalResource` as base rows (trim the `junit-params` gap); testng listener
  bases (`ITestListener`, `ISuiteListener`, `IInvokedMethodListener`,
  `IRetryAnalyzer`, `IReporter`, `IAnnotationTransformer`); cucumber
  `Default*Transformer`. Java and Kotlin. Reproduced (fixtures).
- **Persistence and service loading.** `AttributeConverter` sits in
  `hibernate-listeners` behind `org.hibernate`; move it to `jpa-lifecycle`.
  `java-serialization` lacks `Externalizable.readExternal`/`writeExternal` and
  `@Serial`. `java-serviceloader` should read `module-info.java` `provides I
  with Impl;` (patch); `META-INF/services` is minor. Reproduced (fixtures).
- **FXML handlers** (minor). `onAction="#onLoad"` in a `.fxml` names a public
  controller method that is reported; needs an FXML reader like C#'s
  `markup_kinds_for`. Java and Kotlin. Reproduced (fixture).
- **Declarative HTTP clients become server entries** (`spring`, `micronaut`,
  `jakarta-rs`). Bodiless methods of `@FeignClient`, Micronaut `@Client`,
  `@RegisterRestClient` and `HttpExchange` interfaces get network entries. Skip
  matches inside a container carrying a client marker (needs class annotations),
  not every abstract member: generated `PetsApi` interfaces are servers. Java
  and Kotlin. Reproduced (fixtures).
- **`@Async` is a schedule entry** (`spring-jobs`). It moves an in-tree call
  onto an executor; rooting it hides uncalled `@Async` methods. Drop it. Java
  and Kotlin. Reproduced (fixture).
- **Spring messaging and GraphQL.** `@MessageMapping` never fires in a
  STOMP/RSocket controller importing only `org.springframework.messaging` (add
  its `handler.annotation`, `simp.annotation`, `rsocket.annotation` packages
  plus `SubscribeMapping`, `ConnectMapping`, `MessageExceptionHandler`);
  `@StreamListener` lacks its `org.springframework.cloud.stream` prefix; add
  `@SqsListener`, `@PulsarListener`, `@DltHandler`, `MessageListener.onMessage`;
  Spring for GraphQL (`QueryMapping`, `MutationMapping`, `SubscriptionMapping`,
  `SchemaMapping`, `BatchMapping`, `GraphQlExceptionHandler`) has no row. Java
  and Kotlin. Reproduced (prototypes).
- **Non-Spring server annotations.** Quarkus (`io.quarkus.scheduler`
  `@Scheduled`, `@ConsumeEvent`, `@Route`, `@RouteFilter`, `@Funq`), Micronaut
  (`@Scheduled`, `@EventListener`, `@Error`, `@RequestFilter`/`@ResponseFilter`,
  Kafka `@KafkaListener`/`@Topic`, RabbitMQ `@Queue`, WebSocket
  `@OnOpen`/`@OnMessage`/`@OnClose`/`@OnError`), EJB
  `Schedule`/`Schedules`/`Timeout`, MicroProfile `@Incoming`/`@Outgoing`,
  Jakarta WebSocket `@ServerEndpoint` hooks, JAX-RS
  `ExceptionMapper.toResponse`, request/response filters and interceptors,
  Spring `SchedulingConfigurer.configureTasks`. None is detected today (dead at
  `high`). Java and Kotlin. Reproduced (fixtures).
- **Container callbacks** (`servlet`, `spring`). `init`/`destroy`,
  `ServletContainerInitializer`/`WebApplicationInitializer.onStartup`, attribute
  listeners, `HandlerInterceptor` `preHandle`/`postHandle`/`afterCompletion`,
  actuator `HealthIndicator.health`/`doHealthCheck` (kapua 3). Generic names
  after the private-member fix. Java and Kotlin. Reproduced (fixture).
- **`akka` bases and Pekko.** Add `AbstractActorWithTimers`,
  `AbstractActorWithStash`, `AbstractFSM`, `AbstractPersistentActor`,
  `EventSourcedBehavior`, `DurableStateBehavior` (`createReceiveRecover`,
  `commandHandler`, `eventHandler`, `emptyState`) and the
  `org.apache.pekko.actor` prefix. Java and Kotlin. Reproduced (fixtures).
- **Class-level route prefixes and double nodes** (`jakarta-rs`, `spring`,
  `micronaut`). `@GET` with `@Path("/x")` mints two nodes and a class-level
  `@Path`/`@RequestMapping` never joins the route. Fold `Path` into the verb and
  read the class prefix once class annotations are recorded. Java and Kotlin.
  Reproduced (fixture).
- **Detect-only JVM rules a row can enumerate.** `spring-integration`
  (`ServiceActivator`, `Transformer`, `Splitter`, `Aggregator`, `Router`,
  `Filter`, `InboundChannelAdapter`, `BridgeFrom`/`To`), `camel`
  (`RouteBuilder.configure`, `Processor.process`; bambufarm 4), `dropwizard`
  (`Application`, `Managed`, `HealthCheck`, `Command`; marquez 10), `vaadin`
  (`ClientCallable`, router observers), `kafka-streams` (ksql 26), `flink`,
  `vertx` (`AbstractVerticle`/`VerticleBase` `start`/`stop`; no `handler`
  registrar), `struts` (`ActionSupport.execute`, `@Action`). Generic names after
  import scoping and the private-member fix. Java and Kotlin. Reproduced
  (prototypes).
- **APPS claims.** JabRef's `swing` claim rests on JavaFX methods (freeplane
  fires 1,159); thingsboard's `~aws-lambda` is the SDK client; `vertx`,
  `kafka-streams` (ksql) and `grpc-java` (thingsboard, druid) are used but
  unclaimed; `swing`, the Gradle rules, `caliper`, `moshi`, `jmh` and
  `java-serialization` rest on libraries or the framework's repo; `micronaut`,
  `akka`, `spring-batch` and Java `aws-lambda` on tutorials or samples; no Java
  app for azure/gcp functions or `android-worker`. Code only.

New coverage (ranked):

- **P2 Jakarta CDI observers** (extraction) `@Observes`/`@Disposes` sit on
  parameters; `@Produces` is a row. geopulse: 16 reported (7 `high`).
- **P2 Jenkins plugins, Fabric** (lifecycle; rows)
  `@DataBoundConstructor`/`Setter`, Descriptor `doCheck*`/`doFill*`
  (configuration-as-code 107 entries); `ModInitializer.onInitialize` and twins
  (Kotlin SkyHanni 1).
- **P2 Beam, Hadoop, Camunda 8, Axon, ElasticJob, PowerJob** (queue/schedule;
  rows) DoFn `@ProcessElement`/`@Setup`/`@StartBundle`, `PTransform.expand`
  (Scala Scio twin); `Mapper.map`, `Reducer.reduce` (gobblin 11); `@JobWorker`;
  `@CommandHandler`/`@EventHandler`/`@QueryHandler`; `SimpleJob.execute`,
  `BasicProcessor.process`.
- **P2 Jazzer, picocli, Thrift, Dubbo** `fuzzerTestOneInput`/`fuzzerInitialize`,
  `@FuzzTest`; `@Command`/`@Option` methods (classes need containers);
  `Iface`/`AsyncIface` after the private-member fix; `@DubboService` public
  methods (container).
- **P2 Javalin, Helidon SE** (network; rows after method-reference capture).
- **P3 Spring Shell, jqwik, Velocity/Sponge listeners, Byte Buddy `@Advice` and
  `premain`/`agentmain`, Conductor, Telegram bots, Armeria, JobRunr
  `@Recurring`, Play `conf/routes` (minor).**

Corpus additions:

- mcMMO-Dev/mcMMO (1.1k; bukkit; 1,679 findings)
- CaffeineMC/sodium (5.9k; mixin, fabric; 92 `high`)
- Creators-of-Create/Create (4.6k; forge events, mixin; 58 `high`)
- 00-Evan/shattered-pixel-dungeon (6.9k; libgdx)
- Together-Java/TJ-Bot (123; jda; 981 findings)
- jagrosh/MusicBot (5.8k; jda; last push 2024-11)
- tess1o/geopulse (1.5k; CDI observers, Quarkus)
- halirutan/IntelliJ-Key-Promoter-X (3.1k; IntelliJ platform, Java)
- jenkinsci/configuration-as-code-plugin (2.8k; jenkins plugin, jmh)
- freeplane/freeplane (4.5k; swing; replaces the JabRef claim)
- jreleaser/jreleaser (1.3k; picocli)
- YunaiV/ruoyi-vue-pro (40.6k; quartz, bpmn, rocketmq, spring websocket)
- apache/gobblin (2.3k; temporal, hadoop, quartz)
- kestra-io/kestra (30.3k; real micronaut app, grpc-java; also in the agent
  section)
- ZhongFuCheng3y/austin (6.1k; xxl-job, rocketmq)

### Kotlin

- **`compose` roots every `@Composable`.** An unused composable and its callees
  are never reported, though `setContent { }` already gives `onCreate ->
  Screen`. Narrow to the `@Preview` family plus an
  `AbstractComposeView`/`Content` base row, then corpus A/B. Reproduced
  (fixture).
- **Constructor annotations are never read.** `class R @Inject constructor(…)`
  roots nothing: the Constructor record takes only the class modifiers
  (`kotlin.rs:176-215`) and a `secondary_constructor` is no callable. Merge
  `primary_constructor` modifiers and extract secondary constructors. Rules
  `jsr330-inject`, `dagger`, `guice`, `jackson` (`@JsonCreator`);
  app-wordpress-android has 1,299 jsr330 files and 0 entries. Reproduced
  (fixture; the Java twin roots it).
- **`operator fun` members have no caller.** `getValue` behind `by`, `invoke`
  behind `p()`, `plus` behind `+` are medium findings. Bind the syntax to the
  operator, or record an `operator` keyword marker and root it in
  `kotlin-runtime`, exempt from `IMPORT_SCOPED_ATTRIBUTES`. Reproduced
  (fixture).
- **Kotest and Spek are silent.** Add a detect-only `kotest` row (`io.kotest`,
  `org.spekframework`) saying the spec constructor holds the tests; after the
  constructor rule shape, base types over `FunSpec`, `StringSpec`, … root it as
  a test entry (ort: 342 spec files, 0 Kotlin test entries). Reproduced
  (fixture).
- **Ktor modules named in `application.conf`.** With `EngineMain`, Ktor calls
  the `fun Application.x()` modules under `ktor.application.modules`, so every
  module and route below reads dead (`ktor` is detect-only). Patch: root
  top-level `Application.*` extensions, read from `signature_hint` as
  `type_hints::kotlin_extension` does; reading the config is minor.
  app-kotlinconf-ktor: 71 findings, 23 with the modules rooted. Reproduced.
- **Glance widgets.** `android-broadcastreceiver` detects
  `androidx.glance.appwidget` but no row names `GlanceAppWidget`
  `provideGlance`/`providePreview`; add them once the Kotlin row has bases.
  app-kori-ktorclient: 4 dead, 2 with a prototype. Reproduced.
- **APPS claims.** `caliper`, `cucumber`, `guice`, `hibernate-listeners`, `jmh`,
  `jpa-*`, `jsr250-lifecycle`, `spring-context` and `testng` never fire in
  Kotlin; `akka`, `jakarta-rs`, `micronaut`, `servlet`, `spring-batch`,
  `spring-jobs`, `spring-messaging`, `spring-integration`, `storm` and the cloud
  rows are never exercised; `micronaut` rests on a guide, `akka` on samples;
  `kotlin-okio` is a library. Still wanted: a Kotlin Micronaut/Quarkus app and a
  Kafka/Rabbit one. Code only.

New coverage (ranked):

- **P2 Clikt** (cli; row) `CliktCommand.run` is live through builtin names but
  mints no node. ort: 65 entries.
- **P2 KSP, detekt** (lifecycle; rows) `SymbolProcessorProvider.create`,
  `SymbolProcessor.process`/`finish` (home-assistant 5, SkyHanni 5); detekt Rule
  `visit*`, `RuleSetProvider.instance` (SkyHanni 33, ort 5).
- **P3 Vert.x `CoroutineVerticle`, Koin annotations, Kobweb `@Api`/`@InitApi`,
  http4k, Javalin Kotlin DSL, Kord** (rows; the last three need registrar and
  lambda capture).

Corpus additions:

- YiiGuxing/TranslationPlugin (11.9k; IntelliJ platform; 1,418 findings)
- minecraft-dev/MinecraftDev (1.8k; IntelliJ platform)
- home-assistant/android (4.1k; FCM, room3, lifecycle, KSerializer, KSP, tiles)
- mihonapp/mihon (25.6k; paging, lifecycle, typealias base)
- oss-review-toolkit/ort (2.1k; clikt, kotest)
- lavalink-devs/Lavalink (1.9k; first real Kotlin Spring Boot app)
- gotson/komga (Kotlin Spring Boot server; `spring` 260, `spring-jobs` 4)
- kernitus/BukkitOldCombatMechanics (224; bukkit, Java and Kotlin)
- zachbr/Dis4IRC (122; jda)
- SkyHanniStudios/SkyHanni (564; fabric, detekt, KSP)
- KotatsuApp/kotatsu-syncserver (107; ktor `application.conf` module)

### C\#

- **`mediatr` names one application's types.** `base_types` lists
  `IntegrationEventHandler`/`IIntegrationEventHandler` (dotnet/eShop's own bus)
  and `methods` lists `HandleAsync`, which no MediatR interface declares
  (`rules.rs:11679-11682`, since 0.5.0), so an unrelated `Handle` is rooted.
  Remove all three; check eShop's bus handlers stay live through
  `inheritance_fanout`. Reproduced (fixture).
- **`global using` and implicit usings hide frameworks.** `record_using` keeps
  the `global` keyword in the path (the listed `global using static` item is one
  case), so eShop's `global using MediatR;` leaves `mediatr` undetected and 17
  `Handle` methods dead, xunit, mstest and benchmarkdotnet likewise (`mstest`
  carries a literal `global using` prefix as a workaround, `rules.rs:8114`);
  SDK-Web implicit usings leave a template `Program.cs` with none. Strip
  `global`, apply it project-wide, add `detect_calls`
  `CreateBuilder`/`CreateSlimBuilder`; csproj `<Using>` and SDK implicit usings
  are minor. Land before adding csharp to the import-scoped set. Reproduced
  (fixtures, app-eshop-aspnet).
- **Harness extension attributes and fixtures.** Not matched: `[AvaloniaTest]`,
  `[AvaloniaFact]`/`[AvaloniaTheory]`, `[StaFact]`/`[WpfFact]`/`[UIFact]`,
  `[SkippableFact]`, Unity `[UnityTest]`/`[UnitySetUp]`/`[UnityTearDown]`,
  MSTest `[UITestMethod]` (add `*Fact`, `*Theory`, `*TestMethod`); xUnit
  `IAsyncLifetime` and fixture/test-class constructors, NUnit
  `[DatapointSource]`/`[Datapoints]`/`[TestFixtureSource]`, MSTest
  `DynamicData(nameof(M), …, DynamicDataDisplayName = nameof(F))` (add to
  `member_sources_for`). Reproduced (fixtures; app-avalonia-samples).
- **Unity callbacks** (`unity`, `unity-editor`). `AssetPostprocessor`
  `OnPreprocess*`/`OnPostprocess*` (app-unity-boat, `high`), EditorWindow
  `CreateGUI`/`OnHierarchyChange`/`OnSelectionChange`/`OnInspectorUpdate`,
  `CreateInspectorGUI`, `CreatePropertyGUI`, `OnJointBreak(2D)`,
  `OnCanvasGroupChanged`, `[SettingsProvider]`, `[InitializeOnEnterPlayMode]`,
  `[Shortcut]`/`[ClutchShortcut]`. String-named calls (`InvokeRepeating("M",
  …)`, `Invoke`, `SendMessage`, `BroadcastMessage`, `StartCoroutine("M")`) are
  skipped because registrar capture takes the first string of a multi-argument
  call as the route (`registrar.rs:519`): add a "first string is the handler"
  flag or a `dynuse` entry (app-unity-boat `LoadingScreen.SetLoad`). Reproduced
  (fixtures).
- **App-model rows.** `dotnet-hosting` `IHostedLifecycleService` with
  `Starting`/`Started`/`Stopping`/`StoppedAsync`; `ef-core`
  `ModelSnapshot.BuildModel`; `avalonia` `ReactiveWindow`/`ReactiveUserControl`;
  `godot` roots callbacks on 52 listed bases only (`CanvasLayer`,
  `MeshInstance3D`, Godot 3 `Spatial` miss): add a methods-only companion for
  the underscore names; `wpf-winforms` and `maui` can enumerate `OnStartup`,
  `OnClosed`, `OnPaint`, `OnLoad`, `IValueConverter.Convert`, `OnAppearing`,
  `OnStart`, `OnSleep`, `CreateWindow`; `csharp-runtime` can root finalizers
  with `methods = ["~*"]`. Reproduced (prototypes).
- **Serialization and MVVM rows.** `dotnet-serialization` covers only
  `[OnDeserialized]`: add `IJsonOn*`, `JsonConverter<T>` `Read`/`Write`,
  Newtonsoft `ReadJson`/`WriteJson`/`CanConvert`, `IXmlSerializable`,
  `ShouldSerialize*`, `[JsonConstructor]`, detect
  `System.Text.Json`/`System.Xml.Serialization`; `mvvm-toolkit` roots only
  `[RelayCommand]`: add `IRecipient<T>.Receive`, `ObservableRecipient`
  `OnActivated`/`OnDeactivated` and the `[ObservableProperty]` partial
  `On<Name>Changed`/`Changing` hooks. Reproduced (prototypes).
- **Markup and code-behind.** WinUI/UWP `Click="{x:Bind Go}"` fails
  `is_identifier` (`markup.rs:99`) and needs no `(sender, args)` signature; a
  `.razor.cs` beside a `.razor` with no `@inherits` should imply
  `ComponentBase`; Razor `@code { }`/`@functions` blocks are not parsed (eShop:
  24 of 40 `.razor` files; minor), so give `blazor-lifecycle` a gap meanwhile.
  Reproduced (fixtures, coverage).
- **Static constructors are dead.** `static T()` gets the instance constructors'
  name and kind, so no rule can root it (the hook of Unity `[InitializeOnLoad]`
  classes too). Mark it and root it under `csharp-runtime`. Reproduced
  (fixture).
- **ASP.NET pipeline members.** MVC filters (`IActionFilter` …
  `IAsyncResourceFilter`, `*FilterAttribute`), `IEndpointFilter`, `IMiddleware`
  and `ViewComponent` need a base row (Ombi `WizardActionFilter` dead);
  convention middleware (`Invoke(HttpContext)` via `UseMiddleware<T>`) and
  `Startup.Configure*` need type-argument capture; `aspnet-minimal` lacks
  `MapFallback` and `Map` (gate `Map` on a route: AutoMapper's `mapper.Map`).
  Reproduced (prototypes).
- **Serverless and messaging rows.** aws-lambda: method-level
  `[LambdaSerializer]` (serverless-examples 3 handlers;
  `LambdaBootstrapBuilder.Create`, `APIGatewayProxyFunction.Init` stay open);
  azure-functions Durable `TaskActivity`/`TaskOrchestrator` `RunAsync` and
  `IFunctionsWorkerMiddleware.Invoke` (detect `Microsoft.DurableTask`);
  masstransit `IJobConsumer`, `IActivity`, `IExecuteActivity`,
  `ICompensateActivity`; quartz listeners (`IJobListener`, `ITriggerListener`,
  `ISchedulerListener`), and `Execute(string)` on an `IJob` class is rooted
  beside `Execute(IJobExecutionContext)` (pin with `signature_methods_for`);
  hangfire filters `IServerFilter`, `IClientFilter`, `IElectStateFilter`,
  `IApplyStateFilter`, `IBackgroundProcess(Async)`; `godot-rpc` Godot 3
  `Remote`, `RemoteSync`, `Master*`, `Puppet*`; `dotnet-ffi-export` whole key
  `DNNE.Export`. Reproduced (prototypes).
- **`wcf` can enumerate.** `attributes = ["OperationContract"]` roots the
  contracts and `inheritance_fanout` reaches the implementation under
  `--dead-code`; note that client proxies carry it too. orleans-samples
  `framework/wcf` 0 -> 20. F# too. Reproduced.
- **APPS claims.** Only Ombi and aiproxy are real server apps: `masstransit`,
  `orleans`, `wcf`, `azure-functions`, `blazor-jsinterop`, `dotnet-ffi-export`,
  `godot-rpc`, `aws-lambda`, `aspnet-minimal`, `grpc-dotnet` and every UI rule
  (`unity`, `avalonia`, `monogame`, `godot`, `razor`, `blazor-lifecycle`) rest
  on samples; `xunit` has no real app, `mediatr` only its repo,
  `dotnet-serialization`, `benchmarkdotnet` and `xunit-data` libraries; no C#
  `gcp-functions` app. Code only.

New coverage (ranked):

- **P2 WinUI 3/UWP** (lifecycle; row) `OnLaunched`, `OnNavigatedTo`…; Files 49
  entries. Lands with the `{x:Bind}` fix.
- **P2 GraphQL.NET, Hot Chocolate** (network) `ObjectGraphType` constructors
  hold the resolvers (Squidex 56);
  `[QueryType]`/`[MutationType]`/`[ExtendObjectType]` need class containers,
  `ObjectType<T>.Configure` is a row.
- **P2 Dapr .NET, Wolverine, Coravel** (queue/schedule) `[Topic]`, `Actor`,
  `Workflow`/`WorkflowActivity.RunAsync` (OpenMU 31); `*Handler`/`*Consumer`
  owner-name matcher plus `[WolverineHandler]`; `IInvocable.Invoke`,
  `IListener<T>.HandleAsync` (dasblog-core 1).
- **P2 Discord bots** (network; row)
  `Discord.Interactions`/`Discord.Commands`/`DSharpPlus` `[SlashCommand]`,
  `[Command]` (generic: gate on detect). SysBot.NET 81.
- **P2 PowerShell cmdlets, Autofac/Ninject, Revit, Prism, AutoMapper, BepInEx,
  Xamarin.Forms, Marten, Carter, ServiceStack, TUnit** (rows) `PSCmdlet`
  `Begin`/`Process`/`EndProcessing` (Posh-SSH 21); `Module.Load`,
  `IStartable.Start` (Orchard 23); `IExternalCommand`/`Application` (RevitLookup
  13); `IModule`, `INavigationAware`, `IDialogAware`; `Profile` constructors;
  `BaseUnityPlugin` (read `high` today) and `BasePlugin.Load`; MAUI hooks;
  projection `Create`/`Apply`; `ICarterModule.AddRoutes`; `Service`
  `Any`/`Get`/`Post`; TUnit `[Test]`/`[Before]`/`[After]`/`[MethodDataSource]`
  after NUnit `Test` stops claiming it.
- **P3 Stride, CliFx, Cocona, MEF, Excel-DNA, Temporal .NET, Elsa, MagicOnion,
  Brighter, KafkaFlow, Proto.Actor, SlimMessageBus, protobuf-net.Grpc,
  Telegram.Bot** (rows).

Corpus additions:

- rwmt/Multiplayer (684; Harmony; 760 `high` findings)
- OrchardCMS/Orchard (2.4k; Reqnroll, Autofac, nunit-runner)
- OpenKH/OpenKh (466; McMaster, monogame, xaml, xunit)
- mivano/azure-cost-cli (1.1k; Spectre.Console.Cli)
- files-community/Files (46.8k; Roslyn, WinUI, xaml, mstest, mvvm-toolkit)
- darkoperator/Posh-SSH (1.1k; cmdlets)
- lookup-foundation/RevitLookup (1.5k; Revit, mvvm-toolkit)
- aptabase/aptabase (1.9k; FluentMigrator)
- Revolutionary-Games/Thrive (3.9k; godot, godot-signal, benchmarkdotnet; 1.8 GB
  checkout, above the clone cap)
- PixiEditor/PixiEditor (8.2k; avalonia, xaml, xunit)
- MUnique/OpenMU (1.3k; Dapr, blazor-jsinterop)
- NexusForever/NexusForever (685; Rebus)
- Reaparr/Reaparr (703; FastEndpoints, signalr)
- SkillsFundingAgency/das-commitments (UK government service; NServiceBus,
  mediatr)
- IoTSharp/IoTSharp (1.4k; CAP, aspnet-mvc)
- neo-project/neo (3.5k; Akka.NET, mstest)
- Squidex/squidex (2.5k; GraphQL.NET, xunit, aspnet-mvc)
- colinin/abp-next-admin (1.0k; ABP)
- kwsch/SysBot.NET (578; Discord.Net)
- poppastring/dasblog-core (483; Coravel, aspnet-mvc, xunit)

### Go

- **go-testing roots `Test*` outside `_test.go`.** `match_methods`
  (`mod.rs:1068`) admits a test name in any file importing `testing`, so
  `testutil.TestServer(t)` in `server.go` is never reported; `go test` reads
  only `_test.go`. Require the test role (the unit test at `mod.rs:2570` pins
  today's behaviour). Bare `Test`/`Benchmark`/`Fuzz` are valid test names that
  `name_pattern_matches` and `is_test_name` refuse (`roots.rs:1536` asserts the
  wrong rule), and bare `Example` is a root but no entry. Reproduced (fixtures).
- **Package-function registrars bind any receiver.** `s.Start(work)` on a local
  `Server` becomes an aws-lambda-go entry; robfig `Schedule` and gcp-functions
  `HTTP` share it. Accept these only through an import alias of the detect
  prefix. Reproduced (fixture).
- **Router setup closures and handler factories become entries.** martini
  `m.Group("/books", func(r martini.Router){…})` enters the setup closure;
  `mux.Handle("x", NewProcessor())` (asynq, net-http) enters the constructor,
  not `ProcessTask`. Skip a closure taking a Router parameter; prefer the
  factory's result type. Reproduced (fixtures).
- **Router and job rows.** gin `NoRoute`, `NoMethod`, `Match`, `Group`; echo
  `CONNECT`, `TRACE`, `RouteNotFound`, `Match`, `Pre`, `Group`; fiber `Connect`,
  `Trace`, `Group`; chi `Connect`, `Trace`, `NotFound`, `MethodNotAllowed`,
  `MethodFunc`, `With` (check `With` for false hits); temporal
  `RegisterWorkflowWithOptions`, `RegisterActivityWithOptions`,
  `ExecuteLocalActivity`, `ExecuteChildWorkflow`, `SetQueryHandler`,
  `SetUpdateHandler` (and `…WithOptions`); gcp-functions
  `funcframework.Register*Function*`; go-encoding `AppendText`/`AppendBinary`
  (also absent from `GO_STDLIB_INTERFACES`), `UnmarshalTOML`,
  `MarshalBSON`/`UnmarshalBSON` with their prefixes. Reproduced (prototypes).
- **robfig `Schedule(spec, cron.FuncJob(f))` is not entered.** The wrapper
  descent in `collect_value_refs` (`registrar.rs:836`) requires a route string,
  which `Schedule` never has; let it run for non-ambiguous verbs. Reproduced
  (fixture).
- **`base_types` never match in Go.** `BaseTypeIndex::build` (`mod.rs:919`)
  reads `DefRecord.base_types`, which Go methods never carry; embedding is only
  a `ClassDecl` (`go.rs:762`). Key Go methods by receiver type through the
  `ClassDecl` bases. Blocks beego, revel, iris MVC, grpc-go/connect-go
  `Unimplemented*` embeds and gqlgen (photoview: 71 resolvers reachable only
  from dead code). Reproduced (fixture).
- **Struct-field function values carry no context.** `RunE: run`, `cmd.Run = f`,
  `Handler: h`, `e.HTTPErrorHandler = f` are bare value refs, so no row can name
  them; record the composite literal's type and key, or the assigned selector.
  Unblocks cobra, urfave-cli, `http.Server.Handler`, echo/fiber error handlers
  (overlaps the agent section's capture item). Reproduced (fixtures).
- **iris and machinery can enumerate.** An iris row (`Get` … `Any`, `Handle`,
  `HandleFunc`, `Party`, `Done`, `OnErrorCode`) gave 199 entries on
  app-kubepi-iris; machinery `RegisterTask`/`RegisterPeriodicTask` bind,
  `RegisterTasks(map…)` needs `literal_value` descent; buffalo `Resource` needs
  the handed-type item. Reproduced.
- **APPS claims.** app-gin-photoprism's chi and gorilla-mux claims and
  app-kubepi-iris's gorilla-mux claim rest on false detections; aws-lambda-go,
  temporal, hertz, fiber and asynq are claimed only by framework or examples
  repos; go-testing fires on photoprism, memos and fiber-recipes but is claimed
  only by examples; grpc-go, urfave-cli and grpc-gateway have no claim; add
  hcl-terraform-aws `terraform-plugin-framework`, app-homebox-chi `ent-schema`,
  app-tigerbeetle-zig `cgo-export` once those rows land. Code only.

New coverage (ranked):

- **P2 Interface-satisfaction matcher** (new matcher) root a type's methods when
  its method set covers a listed interface (`go_iface` shape) in importing
  packages: ebitengine `Update`/`Draw`/`Layout` (neko 3 of 3 dead; name rows
  give 45 entries on airplanes, 3 real), go-flags `Commander.Execute`, fyne
  renderers, river `Work`.
- **P2 kong, mage, wails** (cli/lifecycle) `cmd:""` tagged fields and
  `Run`/`Before*`/`After*` (crossplane 6; a name row adds 12 false); exported
  functions of `//go:build mage` files; values bound in `Bind:
  []interface{}{app}`.
- **P2 ginkgo, bun hooks, go-nsq, gocron, huma, gcp-pubsub, slack socketmode,
  gqlgen** (rows) `Describe`/`It`/`BeforeEach` closures;
  `BeforeAppendModel`/`AfterScanRow`; `HandleMessage`; v2 `NewTask` (v1 `Do`
  collides with `sync.Once`); `huma.Get`/`Register`; `sub.Receive(ctx, fn)`;
  `HandleEvents`/`HandleSlashCommand`; resolvers behind `ResolverRoot`.
- **P2 connect-go, twirp, go-zero, gogf, kitex** (network) type-handed services
  and struct-field routes; count them when those items land.
- **P3 uber-fx/dig, godog, cloudevents, gofr, Encore `//encore:api`, telebot,
  Dapr, Cadence, Macaron.**

Corpus additions:

- crossplane/crossplane (12.3k; controller-runtime, kong, prometheus)
- dlvhdr/gh-dash (12.8k; bubbletea v2, cobra)
- crgimenes/neko (648; ebitengine)
- m110/airplanes (323; ebitengine false-positive check)
- oliver006/redis_exporter (3.7k; prometheus collectors, net-http)
- openmeterio/openmeter (2.5k; watermill, gocron, sarama)
- choria-io/go-choria (107; nats)
- automuteus/automuteus (935; discordgo)
- kubesphere/kubesphere (17.1k; go-restful)
- nsqio/nsq (25.8k; httprouter, go-nsq)
- PeerDB-io/peerdb (3.3k; real temporal app; 67 entries)
- JioTV-Go/jiotv_go (763; real fiber app)
- awslabs/ssosync (676; real aws-lambda-go app)
- frain-dev/convoy (2.9k; asynq; regression app for per-file gating)
- photoview/photoview (6.6k; gqlgen, embedded bases)

### Rust

- **actix-web roots `main` as a network entry.** Its `main` attribute mints
  `network::actix-web::crate::main` and, by last segment, claims a
  `#[tokio::main]` elsewhere; `main` is already a root, so drop it. Reproduced
  (fixture).
- **rtic claims Embassy tasks.** Marker-only `rtic` matches
  `#[embassy_executor::task]` by last segment, so each Embassy task gets a
  second entry (RS-Key: 19 false). Match only the bare
  `#[task]`/`#[init]`/`#[idle]` RTIC writes. Reproduced (fixture, RS-Key).
- **`#[graphql(skip)]` members become entries.** `match_container_attributes`
  enters every member of an `#[Object]` impl; skip `graphql(skip)`
  (async-graphql) and `graphql(ignore)` (juniper). Reproduced (fixture).
- **rocket misses `#[launch]`, fairings and guards.** `#[launch] fn rocket()` is
  dead at `high`; `Fairing` (`info`, `on_ignite`, `on_liftoff`, `on_request`,
  `on_response`, `on_shutdown`) and
  `FromRequest`/`FromData`/`FromParam`/`FromSegments`/`FromFormField` methods
  are medium findings. Add `launch` and a node-false base row. Reproduced
  (prototype).
- **ffi-export misses module and NIF markers.** `#[pymodule]`/`pymodule_init`
  (py-robyn's `robyn`), rustler `#[nif]`/`rustler::nif` (supavisor: 4 of 4 dead
  at `high`; logflare native: 174 findings -> 2), neon `export`/`main`, mlua
  `lua_module` are dead at `high`; `proc_macro`, `proc_macro_derive`,
  `proc_macro_attribute` medium; `ctor`/`dtor` need a lifecycle marker row. The
  rustler route must not be the `schedule = "DirtyCpu"` string. Reproduced
  (fixtures, py-robyn).
- **Rust detection reads only `use`.** create-tauri-app v2 writes
  `#[tauri::command]` and `tauri::Builder` with no `use tauri`, so tauri is
  undetected; `#[shuttle_runtime::main]` likewise; HAL apps write `use
  rp_pico::entry` or `#[cortex_m_rt::exception]`, so embedded-rt
  `#[interrupt]`/`#[exception]` handlers are dead at `high`. Count the first
  segment of a scoped attribute or path as evidence, and move `interrupt`,
  `exception`, `pre_init` to a marker-only row. Reproduced (fixtures).
- **Rule rows.** cloudflare-workers (rust) Router `get`/`post`/…/`on` and
  `_async` forms plus base `DurableObject` (`new`, `fetch`, `alarm`,
  `websocket_*`; `#[durable_object]` sits on a struct); actix-actor `stopping`,
  `finished`, `restarting`, `service_started` and
  `Supervised`/`SystemService`/`ArbiterService`; rust-std-traits
  `Mul`/`Div`/`Rem`/`Bit*`/`Shl`/`ShrAssign`, `Hasher`, `BuildHasher`,
  `Termination`, `io::Seek`, `io::BufRead`, `Sum`, `Product` with
  `IMPLICIT_RUST_TRAITS`; axum `fallback`, `fallback_service`, `route_service`,
  `nest_service`, `middleware::from_fn(_with_state)`; actix-web `configure`,
  `from_fn`, `#[trace]`, `#[connect]`. Reproduced (prototypes).
- **Rust test harness shapes.** test-strategy `#[proptest]` is rooted but gets
  no cargo-test entry; rstest `#[fixture]` hits the language-agnostic `fixture`
  in `FRAMEWORK_ATTRS` and becomes a production root; iai-callgrind
  `#[library_benchmark]` is neither. Add to cargo-test and cargo-bench.
  Reproduced (fixtures).
- **clap `default_value_t = f()`** extends the listed `value_parser` item: a
  call in a clap field attribute is dead at `high` too, so the fix must read
  calls as well as paths. Reproduced (fixture).
- **tonic matches only the qualified spelling.** `container_attributes_for`
  `tonic::async_trait` misses `#[async_trait] impl Greeter for S` (qdrant: 98
  gRPC findings, 41 medium); also fold `_` in the descriptor linker. Code only.
- **Detect-only rules a row can enumerate.** poem `#[handler]`, `#[oai(…)]` and
  axum-style registrars; iron `get`/`post`/…/`any`,
  `Before`/`After`/`AroundMiddleware`, `Handler` (keep detect `iron`, not
  `router`; wificonnect's two routes and `AfterMiddleware::catch`); gotham `to`,
  `to_async`, `to_async_borrowing`, `to_new_handler`; tide after receiver-chain
  routes. Reproduced (prototypes, app-concordium-gotham).
- **APPS claims.** salvo, async-graphql, juniper, actix-actor, shuttle, rmcp,
  rtic, embedded-rt, lambda-runtime and cargo-bench rest on framework or
  examples repos; poem is only `~` via shuttle-examples; tonic and tokio-cron
  have no claim. Code only.

New coverage (ranked):

- **P2 sea-orm, loco-rs, apalis, kameo, ractor** (lifecycle/queue; rows)
  `ActiveModelBehavior` hooks, `MigrationTrait` `up`/`down` (svix 10);
  `BackgroundWorker`, `Task`, `Hooks`, `Initializer` (connect-killer 28, 44 dead
  gone); `build_fn` (stump 1); actor
  `handle`/`on_start`/`pre_start`/`post_stop`.
- **P2 criterion and iai groups** (test; item-macro registrar) capture
  identifiers in `criterion_group!(g, f1, f2)` and `library_benchmark_group!`.
- **P2 linkme, inventory, nih-plug, Zed extensions, cucumber-rs**
  `#[distributed_slice(S)]` (dead at `high`), `inventory::submit!` handlers;
  `Plugin::process`/`initialize`/`reset`/`params`, `zed::Extension`
  (superposition 3); `#[given]`/`#[when]`/`#[then]`.
- **P2 RPC trait services, warp** (network; matchers) fan a trait attribute out
  to impl methods (jsonrpsee `<Trait>Server`, tarpc, restate; helios `RpcServer`
  impls stay medium); warp `and_then`/`map`/`then` gated to warp files (reacher
  8 of 8; tree-wide `.map(UserId::new)` was false).
- **P3 relm4, ggez, extism-pdk `#[plugin_fn]`, googletest `#[gtest]`, spin
  `#[http_component]`, matrix-sdk, socketioxide, `proptest!`/`fuzz_target!`
  bodies** (no callable today).

Corpus additions:

- ShadowBlip/OpenGamepadUI (1.0k; gdext; 126 `high`)
- Dreamtowards/Ethertum (561; bevy, fn values; 24 `high`)
- juspay/superposition (442; leptos, Zed extension; 92 `high`)
- OlaProeis/Ferrite (1.9k; eframe, fn values; 1,281 findings)
- TheMaxMur/RS-Key (722; kani, real embedded-rt firmware)
- svix/svix-webhooks (3.5k; sea-orm hooks)
- infinition/waveshare-watch-rs (390; embedded-rt on ESP32)
- afternoon/microgroove (21; the only real RTIC app found)
- OpenGamingCollective/asusctl (698; zbus)
- unixporn/robbb (144; poise, serenity)
- Headline/discord-compiler-bot (446; serenity)
- kozalosev/DickGrowerBot (146; teloxide)
- a16z/helios (2.2k; jsonrpsee)
- reacherhq/check-if-email-exists (10.5k; warp, lambda-runtime)
- stumpapp/stump (2.8k; apalis, async-graphql, axum, tauri)
- valeriansaliou/vigil (2.0k; actix-web, rmcp)
- qdrant/qdrant (35.7k; tonic)
- MoreTore/connect-killer (46; loco-rs)
- supabase/supavisor (2.3k; rustler NIFs in an Elixir app)

### Ruby

- **Rails route verbs leak.** `scope` also matches ActiveRecord `scope
  :published, -> { … }` (66 of 186 rails entries on lobsters are model scopes);
  `namespace`/`resources`/`resource`/`scope` blocks and Rake `namespace :x do`
  mint entries and bind their symbol by simple name (`by_simple`,
  `mod.rs:1608`); option values (`as: :tag`, `:format => "rss"`) bind
  `ApplicationHelper#tag` and `TimeSeries#format`. Gate route verbs to
  `config/routes*`, let container verbs neither mint nor bind (`resources :x`
  may still reach `XController` actions), and read only the `"path" => "c#a"`
  pair and `to:`/`controller:`/`action:`. Reproduced (fixtures, lobsters).
- **Hanami routes claimed by `rails`.** `config/routes.rb` fires on Hanami 2 and
  `to: "books.delete"` decodes through the Lambda `module.function` arm
  (`mod.rs:2105`) onto any `Books#delete`: all 16 rails entries on
  app-terminus-hanami are wrong. Require rails/action_controller evidence
  besides the path and limit the dotted decode to rules that need it. Reproduced
  (fixture, terminus).
- **ActiveJob jobs labelled `sidekiq`.** `ApplicationJob`/`ActiveJob::Base` and
  `app/jobs/` sit in the sidekiq row, so GoodJob and Solid Queue apps report
  `queue::sidekiq::X::perform`. Split an `activejob` row. Reproduced (fixture).
- **Gemfile evidence.** Bundler apps never `require 'grape'`, so barong's Grape
  is undetected and its routes go to rails; treat `gem "x"` in a Gemfile and
  `add_dependency` in a `.gemspec` (both parsed as Ruby) as detect hits outside
  the import table. Fixes grape, sidekiq, padrino, puma, rake. Reproduced
  (fixture).
- **Rule rows.** minitest declarative `test "x" do`/`setup do`/`teardown do` and
  base `ActionDispatch::SystemTestCase`; sinatra `before`, `after`, `not_found`,
  `error` (and a padrino row); `helper_method` in `rails-callbacks`; an
  `actionmailer` row (`ApplicationMailer`/`ActionMailer::Base`, `app/mailers/`,
  previews, node false); the no-argument block forms need DSL block callables.
  Reproduced (prototypes).
- **Detect-only rules a row can enumerate.** hanami `Hanami::Action`
  `handle`/`call` with `app/actions/` (terminus 130 entries, after the rails
  fix); rake `task`/`multitask` with `Rakefile`/`lib/tasks/` (route from a
  leading symbol or hash key: `task :clean`, `task report: :env`, also Padrino
  `get :index`); rspec `it`/`specify`/`let`/`subject`/`before`/`after`/`around`
  with a `harness_globals_rank` entry against minitest; roda
  `r.get`/`r.post`/`r.root`, `hash_branch`, `route`; padrino blocks (minted
  today but labelled rails). Reproduced (prototypes).
- **APPS claims.** grape rests on app-grape-swagger (a Grape plugin) while
  app-barong-grape cannot claim it until Gemfile detection; rails on
  app-terminus-hanami is wrong entries only; diaspora's `~sidekiq` hides the
  member-less base gap; no Ruby app for gcp-functions; rspec, rake, puma and
  rack have no claim. Code only.

New coverage (ranked):

- **P2 ERB templates** (minor) lift `<% %>` code into Ruby as the `.ipynb`
  extractor does; dead helpers: mastodon 156, casa 102, rubygems.org 71, huginn
  51 (EEx, Jinja, JSP alike).
- **P2 gRPC Ruby and gruf** (network; row after qualified bases) generated
  `X::Service` subclasses. ubicloud csi: 42 entries.
- **P2 Queue consumers and schedulers** (rows) Shoryuken, SuckerPunch, Faktory
  `perform`, Sneakers `work`, Hutch `process`, Que `run`, Resque `self.perform`,
  Karafka `consume`, Racecar `process`; rufus-scheduler and clockwork
  `every`/`cron`/`interval` (huginn 9 blocks).
- **P2 Ruby file names** (plugin table, patch) Fastfile, Dangerfile, Podfile,
  Vagrantfile, Capfile, Guardfile, Thorfile, `.thor`, `.jbuilder` are skipped;
  then a fastlane `lane` rule (223M downloads).
- **P2 Cucumber Ruby, dry-cli, Sequel** (rows) `Given`/`When`/`Then`/`Before` in
  `features/step_definitions/`; `Dry::CLI::Command#call`; `Sequel::Model`
  `before_*`/`after_*`/`around_*`/`validate` (ubicloud).
- **P3 Liquid tags, maintenance_tasks, Avo, RuboCop cops, discordrb, Lita, Gosu,
  AASM, GLI** (rows).

Corpus additions:

- rubygems/rubygems.org (2.4k; migrations, Pundit, Phlex, validators)
- rubyforgood/casa (384; migrations, Pundit, ViewComponent, rspec, rake)
- chaskiq/chaskiq (3.6k; graphql-ruby, Action Policy, actioncable)
- basecamp/kamal (14.8k; Thor; every command reported dead)
- ubicloud/ubicloud (12.4k; roda `hash_branch`, gRPC, Sequel, puma)
- huginn/huginn (50.6k; migrations, rufus-scheduler, Liquid tags)
- gollum/gollum (14.4k; a real Sinatra app)

### PHP

- **`laravel` ambiguous verbs mint false routes.** `get`/`view`/`redirect` pass
  with one string, so `$request->get('status')`, `Cache::get('x')`, `view('x')`
  and `redirect('x')` become entries (monica's feature tests bind five routes to
  `AddressBookSubscription::password`). Drop `view`/`redirect`, qualify the
  verbs (`Route::get`), never decode the route string; `codeigniter` has the
  same single-string hazard. Reproduced (fixture, app-monica-laravel).
- **symfony mints two entries per handler.** `AsMessageHandler` is in both
  `symfony` and `symfony-messenger`, so a handler gets network and queue nodes,
  and `AsCommand`/`Command::execute` are labelled network. Drop it from
  `symfony` and move console into a `cli` row that also detects standalone
  `Symfony\Component\Console` apps (attribute `AsCommand`, base `Command` with
  `execute`/`__invoke`/`configure`/`interact`/`complete`; shlink: 33 entries).
  Reproduced (fixture, prototype).
- **codeigniter enters every member.** Empty `methods` plus no PHP visibility
  (`php.rs` `record_def`) makes privates and `BaseController::initController`
  entries; `BaseCommand::run`, `FilterInterface` `before`/`after`,
  `$routes->cli()` and `'Ctrl::m/$1'` targets (`decode_string_target` rejects
  `/$1`) are missed; CodeIgniter 3 needs `application/config/routes.php` and
  `CI_Controller`. Reproduced (fixtures).
- **Registrar arguments.** `X::class` is captured as a reference named `class`
  (`last_segment`), so `Route::get('/u', ShowProfile::class)`,
  `Route::resource('p', C::class)` and Slim handlers bind nothing (also add
  `__invoke` to `laravel.methods`); `string_within` on a keyed array returns the
  key, so `register_rest_route(ns, r, ['callback' => 'f'])`, `['uses' => …]` and
  `[$this, 'm']` in options are lost. Emit the class name; read only the value
  of a `=>` element. Reproduced (fixtures).
- **PHP magic methods are never roots.** `is_dunder` (`roots.rs:1126`) needs a
  trailing `__`, so `__construct`, `__destruct`, `__toString`, `__call`, `__get`
  of DI-built classes are reported. Add a PHP magic-method list. Reproduced
  (fixture).
- **Rule rows.** symfony `AsEventListener`, `AsPeriodicTask`, `AsCronTask`,
  `AsTwigFilter`/`AsTwigFunction`, `LiveAction`/`LiveListener`,
  `EventSubscriberInterface.getSubscribedEvents`; laravel queued jobs
  `ShouldQueue` with
  `handle`/`failed`/`middleware`/`retryUntil`/`backoff`/`uniqueId`/`tags` (also
  queued listeners); laravel-lifecycle `JsonResource`/`ResourceCollection`
  `toArray`/`with`/`withResponse`, Model
  `scope*`/`*Attribute`/`casts`/`boot`/`booted`/`newFactory`, Factory
  `definition`/`configure`; wordpress
  `add_menu_page`/`add_submenu_page`/`add_options_page`, `add_meta_box`,
  settings API, `register_deactivation_hook`/`register_uninstall_hook` (the
  handler string is taken as the route), `register_block_type`,
  `wp_add_dashboard_widget`, bases
  `WP_Widget`/`WP_REST_Controller`/`WP_List_Table`; Bref
  `handleEventBridge`/`handleDynamoDb`/`handleKinesis`,
  `KafkaHandler`/`WebsocketHandler`, `RequestHandlerInterface` for the non-Bref
  `PsrRequestHandler`; gcp-functions `FunctionsFramework::typed`; phpunit vendor
  bases (Codeception `Unit`, Drupal
  `UnitTestCase`/`KernelTestBase`/`BrowserTestBase`, API Platform `ApiTestCase`;
  humhub 113 classes, open_social 71). Reproduced (prototypes).
- **Drupal files are skipped** (minor). `.module`, `.install`, `.theme`,
  `.profile`, `.engine` and `.inc` with `<?php` are `unknown-extension`, so
  every procedural hook is outside the graph. Reproduced (fixture).
- **Detect-only rules a row can enumerate.** drupal method-level `#[Hook]`
  (open_social 33) and `FormBase`/`BlockBase`/`QueueWorkerBase`; yii
  `Controller`/`Action` `action*`/`run` (humhub 411); slim verbs with string
  targets (grocy 172; `'C:m'` needs a `:` decode); cakephp controller, command
  and table bases (passbolt); magento `*ActionInterface`/`ObserverInterface`
  `execute`; swoole `on`/`tick`/`after`. Reproduced (prototypes).
- **APPS claims.** `symfony-messenger` fires only on symfony/symfony,
  `codeigniter` only on the empty appstarter, `magento` only on an extension; no
  Bref or Functions Framework app; `laravel-events` is unclaimed on monica; the
  `~wordpress`, `~cakephp` and `~phpunit` claims encode false detections; add
  `hyperf` to app-mineadmin-swoole once the row lands. Code only.

New coverage (ranked):

- **P2 laminas-mvc/mezzio, typo3 extbase, ratchet, drush** (rows)
  `AbstractActionController` `*Action`; `ActionController`
  `*Action`/`initialize*Action`; `MessageComponentInterface` hooks;
  `#[CLI\Command]`/`#[CLI\Hook]`.
- **P2 Temporal PHP** (queue; row; class attributes needed) `WorkflowMethod`,
  `SignalMethod`, `QueryMethod`, `UpdateMethod`, `ActivityMethod`.
- **P2 codeception, behat** (test) `*Cest` classes need a class-name suffix
  matcher (humhub uses it); `#[Given]`/`#[When]`/`#[Then]`/`#[BeforeScenario]`.
- **P2 Docblock annotations** (extraction) `@Route`, `@ORM\PrePersist`, `@test`,
  `@Given` are invisible; recording `@Tag(...)` above a method lets existing
  rows match.
- **P3 phpbench, joomla, php-amqplib/enqueue, spiral, phalcon, prestashop,
  moodle, mediawiki, lighthouse, phpstan/psalm rules.**

Corpus additions:

- coollabsio/coolify (66.4k; livewire, pest, real laravel)
- pelican/panel (2.4k; filament, livewire, pest)
- julien-nc/cospend-nc (329; nextcloud app)
- Part-DB/Part-DB-server (1.9k; api-platform, doctrine, real symfony)
- kimai/kimai (5.2k; real symfony and phpunit app)

### Scala

- **Effect-app `run` is unrooted.** cats-effect `IOApp`, `IOApp.Simple`,
  `ResourceApp` and ZIO `ZIOAppDefault`/`ZIOApp` start at `def run`, called by a
  `main` outside the tree, so `run` and all it reaches read dead. New cli rows
  (detect `cats.effect`/`zio`, those bases, `run`); feral `IOLambda` is not in
  `aws-lambda` either. watchlistarr: candidates 98 -> 43. Reproduced (fixtures).
- **Extractor `unapply` is dead.** `case … / UserVar(id)` in an http4s route
  (any pattern) records no reference to `unapply`/`unapplySeq`. Root them, or
  record a pattern-site reference. Reproduced (fixture).
- **Imports are not split.** `record_import` (`scala.rs:456`) keeps the whole
  text, so `import play.api.{ApplicationLoader, …}` misses class-level prefixes,
  `import a.B, c.D` loses `c.D`, and `_root_.akka…` and `play.api.{mvc => m}`
  match nothing. Emit one `ImportRecord` per selector and strip `_root_.` (the
  resolver's import tables gain too). Reproduced (fixtures).
- **Detect-only rules a row can enumerate.** `akka-scala` classic actors
  (`Actor`, `AbstractActor`, `PersistentActor`, `Timers`, `FSM`, `Stash` with
  `receive`, `preStart`, `postStop`, `preRestart`, `postRestart`,
  `receiveCommand`, `receiveRecover`, `persistenceId`, `supervisorStrategy`;
  riff-raff 8; a `val` override is still missed); `play-mvc` controller bases
  (riff-raff ~111 entries: 80 of 83 routes plus ~20 helpers, so record Scala
  `private`/`protected`), `Filter`/`EssentialFilter`/`HttpErrorHandler` and DI
  `Module.bindings`/`ApplicationLoader.load`/`AbstractModule.configure`
  (`conf/routes` is minor); specs2 `def is`, ScalaTest
  `beforeAll`/`afterAll`/`beforeEach`/`afterEach`/`withFixture`, Spark
  `SparkListener`/`StreamingQueryListener` `on*` (keep `map`, `run` off).
  Reproduced (fixtures, app-riffraff-play).
- **APPS claims.** No row claims `akka-http`, `akka-scala` or `akka-typed`;
  `scalatest` and `play-mvc` are claimed only by groovy-gradle;
  app-riffraff-play, app-jobserver-spark, app-watchlistarr-http4s and
  app-facto-specs2 detect them unclaimed; the Scala serverless rows have no app.
  Code only.

New coverage (ranked):

- **P2 Finagle, Finatra, TwitterServer** (network; row) Service/Filter `apply`,
  `configureHttp`/`configureThrift`, `setup`/`warmup`/`modules`. the-algorithm:
  151 entries, +368 with a Scala guice row.
- **P2 MUnit, Scalatra, Tapir, ZIO HTTP** (rows; bodies need class-body calls)
  hooks only today (steward +1); `ScalatraBootstrap` `init`/`destroy` (gitbucket
  2; 349 class-body routes wait).
- **P2 Beam/Scio, Akka/Pekko Streams GraphStage, sbt AutoPlugin** (rows)
  `@ProcessElement`…; `createLogic`, `onPush`, `onPull` (Java twin with
  `AbstractInHandler`); `trigger`/`requires`/`projectSettings` (nussknacker 12).
- **P3 Cask (`@cask.*` with no import), weaver-test, Scalafix rules,
  ScalaFX/Scala Swing, Temporal, Mill files (minor).**

Corpus additions:

- webjars/webjars (387; zio-test, zio-app, zio-http)
- scalacenter/scastie (440; `@main`, Scala.js, GraphStage, sbt plugin)
- TouK/nussknacker (746; flink, slick, sbt plugin)
- gitbucket/gitbucket (9.4k; slick, scalatra; 1,293 Scala findings)
- twitter/the-algorithm (74.1k; finagle, guice, beam; last push 2025-09)
- scala-steward-org/scala-steward (1.2k; munit, IOApp, http4s)

### Swift

- **`aws-lambda` misses runtime 2.x.** The README writes the handler as a
  trailing closure to `LambdaRuntime { … }` (or `LambdaRuntime(body: f)`), but
  the only registrar is `Lambda.run`. Add the bare verb `LambdaRuntime`, and
  move `handle` to a base-typed row (`LambdaHandler`, `SimpleLambdaHandler`,
  `StreamingLambdaHandler`) once extension conformances reach the type.
  Reproduced (prototype).
- **UIKit, AppKit and WatchKit hooks.** Dead today: `scene(_:willConnectTo:)`,
  `windowScene`, `stateRestorationActivity`, `viewIsAppearing`,
  `prepare(for:sender:)`, `didMove(toParent:)`, `draw(_:)`,
  `mouseDown`/`keyDown` and kin, `windowDidLoad`, `windowWillClose`,
  `numberOfRows`, `textField`, `userNotificationCenter`, `peripheral`,
  `mapView`, `userContentController`; CodeEdit adds `outlineView` (20),
  `controlTextDidChange`, `makeWindowControllers`. `uikit-lifecycle` lists the
  ObjC `awakeWithContext`; Swift's is `awake`. Generic names go in base-typed
  rows (UIView, NSView, UIViewController, NSResponder, WKInterfaceController,
  NSDocument). Reproduced (fixtures).
- **`xctest` and Quick.** `invokeTest` and `record(_:)` are runner-invoked (the
  gap says so) but `CalcTests.invokeTest` is a finding; add them (ObjC lacks
  `invokeTest` too) and a `quick` row (`QuickSpec`, `AsyncSpec`; `spec`).
  Reproduced (fixture).
- **Rows unblocked by attribute capture.** `swift-testing` (`Test`, `Suite`),
  `@IBAction`/`@IBSegueAction`, `@main` and `@_cdecl` rows wait on recording
  Swift attributes; listed in matrix 10 and 11 only. Reproduced (fixture).
- **`vapor` can enumerate.** A base row (`RouteCollection`, `AsyncMiddleware`,
  `Middleware`, `LifecycleHandler`; `boot`, `respond`, `willBoot`, `didBoot`,
  `shutdown` and async forms) removes 16 false findings on alarik. Route verbs
  need a gate first: naive `get`/`put`/`delete` minted ~22 wrong entries (cache
  `put(app:…)`), and `todos.get(use: index)` is refused by the route gate;
  accept a `use:` argument or trailing closure, never a capitalised receiver
  (Feather: 298 such calls, 275 handlers dead). Reproduced (prototypes,
  app-alarik-vapor).
- **`combine` and `swiftui` hook rows.** combine
  `Publisher`/`Subscriber`/`Subscription` `receive`/`request`/`cancel`, node
  false; swiftui `NSViewRepresentable`, `UIViewRepresentable`, `ViewModifier`,
  `ButtonStyle`, `PreferenceKey`, `Layout`, `DropDelegate` with `makeNSView`,
  `updateNSView`, `makeCoordinator`, `body`, `makeBody`, `reduce`,
  `sizeThatFits` (CodeEdit ~70 findings). `var body` is a callable only under
  `--property-edges` and `property_reads.rs:191-200` gives it no bases, so
  View/App/Scene rows cannot match it. Reproduced (prototypes).
- **Stdlib protocol witnesses are dead.** `==`, `hash(into:)`, `encode(to:)` on
  `Hashable`/`Codable` types (CodeEdit: 17, 12, 12). Add a Swift analogue of
  `rust-std-traits`; the stdlib is never imported, so settle the detect gate
  first. Reproduced (fixture).
- **APPS claims.** No Swift Lambda app; `xctest` is claimed only by
  groovy-gradle while app-codeedit-combine and app-ora-swifttesting fire it
  unclaimed; `swiftui` only as `~swiftui` on app-rn-expensify, not on CodeEdit
  (343 files); swift-alamofire (a library) and rust-bat (syntax fixtures) carry
  the UIKit claims. Code only.

New coverage (ranked):

- **P2 SwiftNIO** (network; row; `NIO`, `NIOCore`) `channelRead`,
  `channelActive`, `errorCaught`, `decode`; Java Netty twin. container: 17 dead
  gone.
- **P2 gRPC Swift** (network; rows) v2 `registerMethods` (container withheld
  chain 1,501 -> 1,214); v1 `*AsyncProvider` needs base patterns.
- **P2 swift-openapi-generator, service-lifecycle, Hummingbird** (network; rows
  after extension bases) `APIProtocol` handlers in extensions, excluding the
  generated `Client` (228 of 291 prototype entries on Feather were client);
  `Service.run`; `RouterMiddleware.handle`, `ResponseGenerator.response`
  (Feather 15).
- **P2 SpriteKit/GameplayKit/SceneKit** (lifecycle; row; ObjC twin) `didMove`,
  `update`, `didBegin`, `GKState.didEnter`; fixture 7 -> 0; no real app found.
- **P2 Composable Architecture** (lifecycle) `@Reducer` adds the conformance and
  the entry is `var body`; only legacy `: Reducer` with `reduce(into:)` works as
  a row.
- **P3 GRDB record hooks, SwiftPM and macro plugins.**

Corpus additions:

- apple/container (52.1k; argument-parser, SwiftNIO, grpc-swift 2,
  swift-testing)
- Dimillian/IceCubesApp (7.1k; WidgetKit, AppIntents, a real xctest/swiftui
  claimant)
- SwiftPackageIndex/SwiftPackageIndex-Server (701; vapor, fluent, commands)
- FeatherCMS/feather (610; hummingbird, swift-openapi, service-lifecycle)

### Objective-C

- **`xctest` takes `test…:` methods with arguments.** XCTest runs only
  argument-less `test…` methods, so `testHelperWithValue:` gets an entry;
  `setUpWithError:`/`tearDownWithError:` are missing (the Swift row has them).
  Reject a `test*` match whose selector has a `:`. Reproduced (fixture).
- **UIKit and Cocoa delegate rows.** `uikit-appdelegate` lacks the UIScene set
  (`scene`, `sceneDidDisconnect`, `sceneDid*`/`sceneWill*`, `windowScene`,
  `stateRestorationActivityForScene`); `uikit-lifecycle` lacks
  `viewIsAppearing`, `viewSafeAreaInsetsDidChange`, `prefersStatusBarHidden`,
  `preferredStatusBarStyle`, `supportedInterfaceOrientations`,
  `canBecomeFirstResponder`, `canPerformAction`, `keyCommands`,
  `canBecomeKeyWindow`, `windowDidLoad`; `cocoa-delegate` lists the Swift label
  `numberOfSections` but not `numberOfSectionsInCollectionView`,
  `numberOfRowsInTableView`, `scrollViewDidEndDecelerating`,
  `scrollViewWillBeginDragging`, `textFieldDidBeginEditing`/`DidEnd…`,
  `textField`, `textView`, `userNotificationCenter`, `pushRegistry`,
  `mailComposeController`, `documentPicker`, `netServiceBrowser`,
  `speechSynthesizer`. Short keywords are also getters: require a `:` in the
  signature. Prototype: Monal 1,123 -> 986, Hammerspoon 3,759 -> 3,532.
  Reproduced.
- **NSObject runtime methods.** `+load`, `description`, `debugDescription`,
  `hash`, `isEqual:`, `awakeAfterUsingCoder:`, `keyPathsForValuesAffecting*`,
  `automaticallyNotifiesObservers*` have no caller in source (only `initialize`
  is rooted), and `dealloc` sits in `uikit-lifecycle`, so a Foundation-only tool
  reports it. Add a Foundation row (`load` only as a `+` class method) and move
  `dealloc` there. Hammerspoon: `load` 37, `description` 22 dead. Reproduced
  (fixtures).
- **`@import UIKit;` records no import.** `objc.rs` reads only
  `preproc_include`/`preproc_import` and skips `module_import`, so a
  Clang-modules tree (the Xcode default) detects no rule. Record the module
  path. Reproduced (fixture).
- **AppKit apps miss two rules.** `objc-selector` detects only Foundation/UIKit
  and `objc-main` lacks AppKit; add `<AppKit`/`AppKit`, `<Cocoa`/`Cocoa` and
  `WebKit`. Reproduced (fixture).
- **No class bases or adopted protocols.** No `ClassDecl` for `@interface X :
  Base <P>`, so `xctest` cannot use `base_types: ["XCTestCase"]`, every
  `cocoa-delegate` keyword roots tree-wide and `super` binds by name. Record
  bases and protocols from `.m` interfaces and class extensions (headers once
  they reach the plugin). Code only (`objc.rs` `class_implementation`).
- **APPS claims.** The corpus has no real ObjC application: objc-afnetworking is
  a library plus its Example app, and a Swift twin satisfies every ObjC claim.
  Code only.

New coverage (ranked):

- **P2 App extension principals** (lifecycle; row gated per framework)
  `didReceiveNotificationRequest`, `startTunnel…`, `didSelectPost`. Monal 5.
- **P2 Cordova and Flutter plugins** (ffi) actions are any
  `-(void)x:(CDVInvokedUrlCommand*)` (wildcard `signature_methods_for` with ObjC
  parameters); `registerWithRegistrar`, `handleMethodCall`,
  `onListen`/`onCancelWithArguments`.
- **P2 CallKit, Foundation subclass hooks** `providerDidReset` now, `provider:`
  delegates after bases (Monal 7 dead); NSURLProtocol, NSOperation
  `isExecuting`/`isFinished`, NSValueTransformer.
- **P3 Metal/SpriteKit, Mantle/YYModel/MJExtension/JSONModel hooks.**

Corpus additions:

- Hammerspoon/hammerspoon (16.5k; AppKit, Lua C API, XCTest; ~159 MB)
- Sequel-Ace/Sequel-Ace (7.6k; AppKit documents, xctest)
- Automattic/simplenote-ios (2.1k; Core Data)
- monal-im/Monal (690; UIKit, app extensions, XCTest; ~204 MB)
- RocketChat/Rocket.Chat.ReactNative (2.4k; React Native ObjC; 389 MB)

### C++

- **`cpp-callbacks` verbs match member calls.** `bus.signal(3, on_event)`
  root-marks `on_event`, and `screen->on_exit()` alone detects the rule (false
  line on app-smoothieware-cpp): `detect_calls` and capture ignore the receiver
  (`mod.rs:248-256`). Accept these verbs only bare or `std::`/`::`-qualified;
  `c-callbacks` too. Reproduced (fixture).
- **gtest and gMock gaps.** Add `GTEST_TEST`/`GTEST_TEST_F`, rows for
  `testing::Environment` `SetUp`/`TearDown` and
  `TestEventListener`/`EmptyTestEventListener` `On*` (case-sensitive); `class F
  : public Test` after `using ::testing::Test` misses the `testing::Test` base.
  `ACTION`/`ACTION_P…` parse as functions named for the macro and
  `MOCK_METHOD(…)` as `Mock::MOCK_METHOD`: add them (`MOCK_*` node false);
  `MATCHER` needs a block callable. Reproduced (fixtures).
- **`aws-lambda` (cpp) binds no lambda handler.** `auto h =
  [&c](invocation_request const& r) {…}; run_handler(h);` (the S3 example) mints
  nothing; the lambda's calls go to `main`, so dead code is unaffected. Enter
  the enclosing function or say so. Reproduced (fixture).
- **CUDA and HIP sources are never parsed** (minor). No plugin claims
  `.cu`/`.cuh`/`.hip`, so app-cuda-samples skips 1,004 files and mints 2 entries
  (the same 40 `.cu` files renamed `.cpp` mint 50). Claim them in the C++
  plugin; the C `cuda` row can never fire (C records no attributes): drop it or
  give `c.rs` the `cuda_qualifiers` scan. Known in dev_docs `cpp.md:72`.
  Reproduced.
- **`T x;` makes no constructor edge.** `auto c = C()` and `new D()` reach the
  constructor, but `CMakeSetupDialog dialog;` (cmake-gui) leaves it and every
  slot it connects dead. Emit a typed construction for a class-typed
  declaration, `T x(args);` too. Reproduced (fixture, cmake-kitware
  `Source/QtDialog`).
- **Catch2 and doctest can enumerate.** `TEST_CASE("…") {…}` yields no callable
  and its calls stay `no-candidate-in-file`; synthesize a `handler_at_<line>`
  for a namespace-scope `ident("…", …)` followed by a block in detected files,
  then give `cpp-test-macros` methods. Reproduced (fixture).
- **APPS claims.** `cuda` rests on NVIDIA samples and a CMake HIP test fixture;
  `libfuzzer`/cpp only on nlohmann-json (a library); `aws-lambda`/cpp never
  runs; `cpp-callbacks` is claimed only by the samples repo while
  app-waybar-catch2 fires it unclaimed; groovy-gradle fires `gtest` only from
  doc snippets. Code only.

New coverage (ranked):

- **P2 Unreal Engine** (lifecycle; after the two extraction items) `BeginPlay`,
  `Tick`, `OnRep_*`, `UFUNCTION` members. Bomber 4 entries today.
- **P2 Class-naming registrars** (new matcher) `PLUGINLIB_EXPORT_CLASS`,
  `RCLCPP_COMPONENTS_REGISTER_NODE`, `QTEST_MAIN`, `GDREGISTER_CLASS` name a
  class whose constructor and overrides run (navigation2 keeps `configure` 47,
  `activate` 41 dead with a plain row).
- **P2 wxWidgets, Drogon, godot-cpp, pybind11/nanobind** (rows) `wxApp`
  `OnInit`/`OnExit` and `EVT_*` tables (treesheets 765 -> 583);
  HttpController/WebSocketController members (paroli 111 -> 88; `METHOD_ADD`
  needs the macro item); `_bind_methods`, `_ready`, `_process`;
  `PYBIND11_MODULE`/`NB_MODULE`/`BOOST_PYTHON_MODULE` bodies.
- **P2 Fuzz and benchmark harnesses** (test; rows) libprotobuf-mutator
  `DEFINE_PROTO_FUZZER` (and `_BINARY_`/`_TEXT_`), FuzzTest `FUZZ_TEST(Suite,
  Fn)`, Google Benchmark `BENCHMARK(BM_x)` and `->Arg(8)` (not
  `BENCHMARK_TEMPLATE`; `BENCHMARK_F`, `->Setup(fn)` untried), CppUTest `TEST(G,
  n)`/`IGNORE_TEST`/`TEST_GROUP` (C firmware too). Prototyped.
- **P3 openFrameworks, Crow, cpp-httplib, oatpp** (route lambdas already
  attribute to the enclosing function: entries only).

Corpus additions:

- flameshot-org/flameshot (31.7k; Qt Widgets; 452 findings)
- keepassxreboot/keepassxc (30.1k; QtTest, Qt; 2,776 findings)
- PowerDNS/pdns (4.5k; Boost.Test, libfuzzer, cpp-callbacks)
- ros-navigation/navigation2 (5.1k; ROS 2, BehaviorTree.CPP, pluginlib, ament
  CMake extras)
- tensorflow/serving (6.4k; gRPC C++, gtest)
- asb2m10/dexed (3.7k; JUCE)
- aardappel/treesheets (3.2k; wxWidgets)
- meshtastic/firmware (8.6k; Arduino; 5,034 findings)
- JanSeliv/Bomber (392; Unreal Engine)
- marty1885/paroli (136; Drogon)

### C

- **Test harness rows.** Criterion's `Test(suite, name) { .. }` yields no
  callable, so the code under test reads dead: add a detect-only `criterion` row
  until block callables land. Unity `RUN_TEST(fn)` is invisible (ALL-CAPS
  macro); it matters only for names without Ceedling's `test_` prefix: a `unity`
  row (registrars `RUN_TEST`, `RUN_TEST_CASE`; methods `setUp`, `tearDown`).
  Reproduced (fixture).
- **No Linux `EXPORT_SYMBOL` rule.** A function exported only to other modules
  is reported dead. A `public` row, node false (detect `linux/module.h`,
  `linux/export.h`; registrars `EXPORT_SYMBOL`, `_GPL`, `_NS`, `_NS_GPL`) roots
  it once macro arguments are read; `module_init` and `file_operations`
  initialisers already root theirs. Reproduced (prototype).
- **APPS claims.** `cuda` rests on NVIDIA's samples and CMake test data,
  `libfuzzer` on c-jq (fine) and nlohmann-json (a library), `~c-runtime` only on
  non-C apps with stray C files while c-jq and c-redis detect it unclaimed. Add
  a real CUDA app once `.cu` is read and a GTK app with the GLib row. Code only.

New coverage (ranked):

- **P2 PostgreSQL extensions** (ffi; new matcher; `fmgr.h`) root functions whose
  parameter list is `PG_FUNCTION_ARGS`: pgvector has 99 of 105 SQL functions
  dead; `PG_FUNCTION_INFO_V1(f)` parses as a declaration.
- **P2 Flipper Zero apps** (lifecycle; minor: `application.fam`)
  `entry_point="x"`: 137 of 161 entry points dead; a suffix row cuts 8,129 ->
  5,928 but roots ~14 helpers.
- **P2 eBPF/libbpf, in-tree registration macros** (extraction) record
  `SEC("...")` before a definition as an attribute (pwru: 67 findings over 32
  programs; `BPF_PROG` is misnamed); read an ALL-CAPS macro's argument as a
  value when its in-tree `#define` puts it in an initializer (zmk: 25 of 27
  `ZMK_LISTENER` callbacks dead).
- **P2 PHP, Ruby, MicroPython extensions** (ffi; rows)
  `PHP_FE`/`ZEND_FE`/`PHP_ME`, `PHP_MINIT`/`PHP_MINFO` (phpredis 1,483 -> 1,421;
  unresolved sites rose 7,038 -> 11,990 in the prototype); `rb_define_method*`
  (puma +27 entries; one `#ifdef` twin became dead); `MP_DEFINE_CONST_FUN_OBJ_*`
  arguments.
- **P3 Windows `DllMain`/`DriverEntry`, SDL3 callbacks, `GST_PLUGIN_DEFINE`,
  `NAPI_MODULE`, LVGL, Mongoose, libevent, libuv** (rows; value refs already
  keep most targets live).

Corpus additions:

- zmkfirmware/zmk (4.3k; Zephyr, CMake Zephyr detection)
- geany/geany (3.8k; GLib/GTK; 8,130 findings)
- sle118/squeezelite-esp32 (2.3k; ESP-IDF, FreeRTOS)
- flipperdevices/flipperzero-firmware (17.0k; Flipper apps, FreeRTOS)
- phpredis/phpredis (10.2k; PHP extension)
- pgvector/pgvector (23.8k; PostgreSQL extension)
- asg017/sqlite-vec (8.3k; SQLite extension, libfuzzer)
- RedisBloom/RedisBloom (1.8k; Redis module)
- cilium/pwru (3.9k; eBPF)
- puma/puma (7.9k; Ruby C extension)
- tbamud/tbamud (149; Unity tests, c-callbacks)
- bitcraze/crazyflie-firmware (1.6k; CMSIS handlers)

### Elixir

- **Phoenix route arguments bind unrelated defs.** `live "/p", PostLive.Index,
  :index` binds any unique `index` and `as: :site` binds `site` (all 9 `live`
  entries on Plausible are wrong); a piped `Map.put("k", x.y)` reads as a route
  (6 on Plausible). `registrar_arg` should skip the live-action slot and keyword
  options, and `record_registrar` should refuse a receiver-qualified verb on the
  right of `|>`. Reproduced (fixtures, Plausible).
- **Phoenix routes to non-unique actions get no entry.** The plugin emits
  `PostController::index` pairs but `phoenix` has `string_targets: false`, so
  only globally unique actions bind (about a third of Plausible's). Set it after
  the fix above. Reproduced (prototype).
- **`resources`, `forward` and `to:` are not expanded.** `resources "/c",
  CommentController` roots no action and `forward "/api", to: Mod` or `post
  "/x", to: Mod` no `init`/`call`. Expand per verb (seven REST actions narrowed
  by `only:`/`except:`; plug `init` plus `call`). Rails `resources :users` is
  the Ruby twin. Reproduced (fixture).
- **Missing callbacks.** `render/1`, LiveComponent `update/2`, `update_many/1`,
  `on_mount/4`, Socket `id/1` once methods carry arity; function plugs (`plug
  :name`) leave `name/2` dead: read the atom as a value ref to the module's own
  `name/2`. Reproduced (fixtures).
- **`use M` never credits `M.__using__/1`.** Every in-tree `__using__` and the
  web module's `router/0`, `controller/0`, `live_view/0` read dead (Plausible:
  25). Emit a call to `M.__using__`, plus `M.atom/0` for `use M, :atom`; the
  same closure is what per-file gating needs. Reproduced (fixture).
- **Import and marker records.** `use X, opts` records the whole argument list,
  so `use GenServer, restart: :transient`, `use Phoenix.VerifiedRoutes, …` and
  `use Oban.Worker, queue: …` detect nothing (also in the agent section);
  `@behaviour X` without `use` detects nothing and `@impl` roots nothing
  (Plausible: 74 findings under `@impl`); `@spec f(...)` reads as a top-level
  call that roots every `f` (one `@spec change` keeps 218 migration `change`
  defs live; also in the agent section). Record the first child of `use`,
  `@behaviour` as an import, `@impl` as an attribute, and skip `@spec`.
  Reproduced (fixtures, Plausible).
- **`defimpl` bodies of outside protocols are dead.** `defimpl
  Jason.Encoder`/`String.Chars` defs are unscoped (a top-level `defimpl` gives
  the bare `encode`). Scope as `<Protocol>.<For>` and root defs of protocols the
  tree does not declare, as `clojure:external-contract` does. Reproduced
  (fixture).
- **HEEx components.** `<.button>` in `~H` sigils is not read and no rule
  detects `use Phoenix.Component`. Scan `~H` for `<.name`/`<Mod.name` (patch),
  add a detect-only row; `.html.heex` files are minor. Reproduced (fixture).
- **APPS claims.** `plug-router` is claimed only by elixir-phoenix (the
  framework repo). Code only.

New coverage (ranked):

- **P2 gen_statem/GenStateMachine** (lifecycle; row with arity) `handle_event/4`
  collides with LiveView `handle_event/3` (teslamate: 59 of 157 outside).
- **P2 PromEx, WebSockex** (lifecycle; rows)
  `event_metrics`/`polling_metrics`/`manual_metrics`, dashboards (realtime 26
  dead); `handle_frame`/`handle_connect`/`handle_disconnect` (teslamate 2).
- **P2 Absinthe, WebSock, elixir-grpc** (network) `resolve &M.f/3` is refused by
  the leading-string gate (glific: 444 resolve lines, 0 entries); callbacks of
  the module passed to `WebSockAdapter.upgrade` (lanyard); public defs of a `use
  GRPC.Server` module (logflare 3, live only by accident).
- **P3 Tortoise MQTT, Swoosh adapters, LiveDashboard pages, Ash, Commanded,
  Quantum, Membrane.**

Corpus additions:

- Phineas/lanyard (1.5k; plug-router, the first real-app evidence)
- hexpm/hexpm (1.2k; Oban, Ecto.Migration, PromEx, Phoenix.Component)
- Logflare/logflare (1.0k; Broadway, GenStage, Rustler, gRPC, Ecto.Type)
- supabase/realtime (7.7k; real phoenix-channel and phoenix-socket)
- teslamate-org/teslamate (9.3k; GenStateMachine, Tortoise, WebSockex)
- sequinstream/sequin (2.2k; Oban, GenStage, Broadway; last push 2026-02)
- glific/glific (223; Absinthe, Oban)

### Erlang

- **Scope OTP callbacks by `-behaviour`.** `erlang-gen-server` roots every
  `handle_call`/`handle_event` once any file declares `gen_server` (OTP
  `mnesia_loader:handle_event/3`). The plugin records `-behaviour` per file:
  stamp each module's definitions with those atoms as `base_types` (also a
  `gen_server:start_link(…, ?MODULE, …)` call: `timer.erl` has no behaviour
  line), then let `erlang-otp-lifecycle` and `erlang-application` enumerate
  `init`, `terminate`, `code_change`, `format_status`, `callback_mode`,
  `handle_event`, `start`, `stop`, `prep_stop`, `config_change`, `start_phase`
  (vernemq: 13 `stop/1` findings). Elixir `use GenServer`/`@behaviour` the same.
  Reproduced (prototype, 12 of 12 callbacks).
- **Common Test.** `group/1` and the `ct_hooks` callbacks (`id`,
  `pre_*`/`post_*`, `on_tc_fail`, `on_tc_skip`, `post_all`, `post_groups`) are
  missing, though `ct_hooks` is a detect prefix; cases named in `all/0` and
  `groups/0` (skip `{group, G}`) are never entered: read those literal lists as
  value refs to the local `Name/1`. Reproduced (fixtures).
- **State functions and in-tree behaviours.** In `state_functions` mode the
  runtime calls `Module:State/3` for the atom in `{ok, S, D}`/`{next_state, S,
  …}` (`gen_fsm`: `S/2`, `S/3`): record those atoms as value refs in modules
  declaring the behaviour (vernemq: 14 reported). `-callback` is not read, so
  implementers called through a variable (vernemq `routes/0`, rebar3 provider
  `do/1`) read dead: root or dynamically link same name/arity functions of
  modules declaring an in-tree behaviour. Elixir `@callback` likewise.
  Reproduced (fixtures).
- **Missing rows.** `logger_handler`/`logger_formatter` (`adding_handler`,
  `changing_config`, `filter_config`, `log`, `removing_handler`, `check_config`,
  `format`; scoped per module); PropEr (`prop_*`, zero-arity, in modules
  including `proper.hrl`); cowboy `cowboy_rest` names,
  `websocket_init`/`_handle`/`_info`, `webtransport_*` as a node-false methods
  row (generic `init/2`, `info/3`, `execute/2` after module scoping). Reproduced
  (prototypes).
- **cowboy detection.** Guide-shaped handlers have no `-behaviour`, and
  `cowboy_loop`, `cowboy_stream`, `cowboy_middleware`, `cowboy_sub_protocol`,
  `cowboy_webtransport` are not prefixes; add them and `detect_calls`
  `cowboy:start_clear`/`start_tls`, `cowboy_router:compile`. Reproduced
  (fixtures).
- **APPS claims.** `common-test`, `~erlang-application` and
  `~erlang-otp-lifecycle` are claimed only by erlang-otp; app-vernemq-cowboy
  fires `common-test` and `eunit`, app-rebar3-eunit fires `common-test` and
  detects both OTP rules: claim them. Code only.

New coverage (ranked):

- **P2 Module/function argument pairs** (extraction) `f(…, ?MODULE, fun_atom,
  …)` naming a local exported function. ejabberd: 168 of 189
  `ejabberd_hooks:add` and 34 of 35 IQ handlers dead.
- **P2 Term files** (minor) `{M,F,A}` in `.app.src`/`sys.config` and `{mod, {M,
  _}}`. vernemq: 26 of 34 plugin hooks dead.
- **P2 minirest, elli, brod** (rows) `api_spec`/`paths`/`schema`/`namespace` and
  `'operationId' => f` atoms (emqx: 169 of 196 handlers dead; 22,251 -> 21,551);
  `handle/2`, `handle_event/3` after module scoping (helium/router 524 -> 515);
  `handle_message`, `assignments_received`/`_revoked`, `get_committed_offset`
  with `detect_calls` `brod_group_subscriber_v2:start_link`.
- **P3 `trails_handler`, nova, yaws appmods, webmachine, zotonic observers, OTP
  `cli`.**

Corpus additions:

- emqx/emqx (16.9k; minirest, brod, `?MODULE` values; real-app common-test,
  eunit, gen-server)
- processone/ejabberd (6.8k; MF argument pairs, in-tree `gen_mod` callbacks)
- helium/router (69; elli; a real LoRaWAN router)

### Groovy

- **Spock `string-named` enters production methods.** Once spock is detected,
  any `def "…"()` in `src/main` becomes a test case; the attribute path in
  `match_attributes` has no test-file or `Specification` gate. Require one.
  Reproduced (fixture).
- **Spock features with identifier names are missed.** A method with a top-level
  `given:`/`when:`/`then:`/`expect:`/`where:` block is a feature (`void
  subtracts() { expect: … }`); mark it in `groovy.rs` and add the mark to
  `spock` (groovy-gradle: 60 such methods). Reproduced (fixture).
- **Groovy `gradle-task` prints a false gap.** It is detect-only and its
  annotations live under `gradle-task-annotations`, so every Gradle repo prints
  "seen, no rules gradle-task" while `@TaskAction` enumerates. Fold the list
  into `gradle-task` (one list for Java, Kotlin, Groovy) and update the claims.
  Reproduced (fixture).
- **`grails` detection and rows.** Add `grails.transaction` (41 files in
  streama), `grails.test`, `grails.async`, `grails.plugin.*` and `detect_paths =
  ["grails-app/"]` (domain classes import nothing); GORM hooks
  `beforeInsert`/`afterUpdate`/`beforeValidate`… as a node-false row; controller
  actions and `*Interceptor` `before`/`after` need the owner-suffix path gate.
  Reproduced (prototype; corpus grep).
- **Jenkins shared-library steps.** `vars/<name>.groovy` `def call(…)` is
  invoked by file name; a `detect_paths = ["vars/"]` + `methods = ["call"]`
  prototype removed all 179 dead `call` findings on app-saplib-jenkins (446 to
  109 findings) but also rooted a `call` under `template/`, so it needs the path
  gate. Reproduced.
- **`Jenkinsfile` as a Groovy file** (minor). Left out because the old grammar
  parsed too few (pinned in `detect.rs` tests); measure the dekobon parse rate
  first. 178k GitHub hits. Unverified.
- **APPS claims.** groovy-gradle (gradle/gradle) is the only evidence for
  `gradle-plugin`, `gradle-task` and `gradle-task-annotations`;
  app-saplib-jenkins is a shared library, not a pipeline consuming one. Code
  only.

New coverage (ranked):

- **P2 Geb specs** (test; spock row change) bases stop at `Specification`, so
  `setup`/`cleanup` of `GebReportingSpec`/`GebSpec` subclasses are dead
  (openboxes 5).
- **P2 Nextflow `.nf`** (minor; new parse mechanism) `process`/`workflow` blocks
  and `include { A } from './m'`. nf-core/rnaseq: 10 of 928 files analyzed.
- **P3 AST transformations, Ratpack handlers, Spock extensions** (rows; `visit`,
  `Handler.handle`, `IGlobalExtension`/`IAnnotationDrivenExtension`).

Corpus additions:

- openboxes/openboxes (938; grails, GORM, quartz, spock, geb; 1,266 dead
  controller findings)
- InovelliUSA/Hubitat (105; Hubitat drivers)
- tonesto7/echo-speaks (114; Hubitat app)
- jenkins-infra/pipeline-library (311; Jenkins vars)
- firstBitMarksistskaya/jenkins-lib (217; Jenkins vars and src, groovy junit)
- nf-core/rnaseq (1.4k; Nextflow candidate)

### Dart

- **Base clauses lose bases.** A clause starting `with`, a `mixin … on B` clause
  and `dart format`'s multi-line `with` record no base (`dart.rs`/`attrs.rs`
  `split_type_list`), so every base-type rule misses the class (PiliPlus
  `AudioController::onInit`). Scala shares the splitter (unverified). Reproduced
  (fixtures).
- **`@pragma('vm:entry-point')` is dead.** Functions the embedder calls by name
  (`overlayMain`, background handlers) are medium findings; after Dart
  annotation capture, root `pragma('vm:entry-point')` only. Reproduced
  (fixture).
- **`flutter` and `flame` can enumerate.** `dart.rs` records
  `extends`/`with`/`implements`; a row on `State`, `StatelessWidget`,
  `StatefulWidget`, `CustomPainter` with the lifecycle names gave 683 entries on
  immich mobile (roots 267 -> 950, reported 3,055 -> 2,697) and left a
  non-widget `build()` alone. Split rows per base family (`paint`, `load`), add
  `activate` and Flame's `onHotReload`, node false; flame through bonfire
  (`SimpleEnemy`, `SimplePlayer`, `GameDecoration`) needs a `bonfire` row.
  Reproduced (fixtures, immich).
- **Small table fixes.** Drop the nonexistent `package:shelf_io` prefix from
  `shelf`; then shelf `get` … `mount`, `add`, `addHandler`, `serve` and
  flutter-test `test`, `testWidgets`, `group`, `setUp*`, `tearDown*`
  (flame_test) once Dart registrar capture lands (dartpad registers 15 routes).
  Reproduced (fixture).
- **APPS claims.** app-immich-nestjs (432 flutter files) claims neither
  `~flutter` nor `~flutter-test`; the only `~flame` claimant is a bonfire app.
  Code only.

New coverage (ranked):

- **P2 serverpod** (network; row) `Endpoint` bases and `FutureCall.invoke`; also
  claims `_private` helpers until Dart visibility comes from the leading `_`
  (discord_open 22).
- **P2 Jaspr, bloc, go_router, shelf_router** (rows) components (site-www 167
  entries, 305 -> 189); bloc/hydrated_bloc overrides (thunder 5); `GoRouteData`
  `build`/`redirect`/`onExit`; `@Route.get` (pub-dev 69, after annotation
  capture).
- **P2 `toJson()` for `dart:convert`** (`signature_methods_for`, no parameters)
  `jsonEncode` calls it: SkyBridge 253, pub-dev 100, lichess 21 findings.
- **P3 Hive `TypeAdapter`, auto_route guards, flutter_modular, stacked,
  benchmark_harness, mason hooks, relic/alfred routers.**

Corpus additions:

- thunder-app/thunder (1.0k; equatable, bloc, flutter)
- bggRGjQaUbCoE/PiliPlus (21.4k; GetX, base clauses)
- lichess-org/mobile (2.7k; riverpod, flutter)
- leoafarias/fvm (5.5k; package:args)
- videah/SkyBridge (602; dart_frog; last push 2024-05)
- Coffiie/discord_open (130; serverpod)
- dart-lang/pub-dev (948; the pub.dev site; shelf, shelf_router)
- dart-lang/site-www (1.1k; the dart.dev site; jaspr)

### F\#

- **Type attributes land on the primary constructor.** `push_constructor`
  (`fsharp.rs:398`) gives `type C(…) =` the attributes written above the type,
  so a rule listing `Route` enters the constructor. Latent until an F#
  `aspnet-mvc` row exists; record them as container attributes. Reproduced
  (prototype).
- **xUnit and FsCheck tests are never entered.** `fsharp-nunit` lists
  `Fact`/`Theory` but detects only NUnit/FsUnit, so an `open Xunit` file mints
  nothing and prints nothing. Add `fsharp-xunit` (detect `Xunit`,
  `FsCheck.Xunit`; `Fact`, `Theory`, `Property`) and drop `Fact`/`Theory` from
  `fsharp-nunit`; add `member_sources_for` arms for
  `TestCaseSource`/`ValueSource` and `MemberData`/`ClassData`. Reproduced
  (fixtures).
- **`fsharp-entrypoint` and expecto.** Add `attributes = ["EntryPoint"]`
  (FsAutoComplete `Program.fs:4` `entry` is dead) and narrow the
  `System`/`Argu`/`CommandLine` detect list; `[<Tests>]` sits on value bindings,
  which are no callables, so expecto cannot enumerate until attributed values
  become roots (`Tests`, `FTests`, `PTests`). Reproduced (fixtures).
- **APPS claims.** `fsharp-nunit` is claimed only by app-orleans-samples but
  fires unclaimed in fsharp-paket; `fsharp-entrypoint` only by samples; F#
  `azure-functions` has no app; app-cardmgmt-giraffe is a self-described
  example. Code only.

New coverage (ranked):

- **P2 avalonia with FuncUI, signalr, akka.net** (rows after bases)
  `HostWindow`/`HostControl` (Rocksmith2014.NET 5); `inherit Hub<T>()`; actors
  as the C# row.
- **P2 FsCheck, Hedgehog** (test; row; C# twin) `[<Property>]`/`[<Properties>]`,
  FsCheck.NUnit, Hedgehog.Xunit; 19.6M downloads.
- **P2 Bolero, WebSharper** (network; rows)
  `ProgramComponent`/`RemoteHandler`/`Component` (shader-minifier: 4 files
  detected, 0 entries); `Rpc`, `SPAEntryPoint`.
- **P2 Falco, Oxpecker, Elmish, aspnet-minimal** (network; detect-only rows now)
  enumerate after curried registrar capture; `.MapGet(p, Func<_>(fun …))` needs
  lambda callables (akka-dotnet-bank 75 calls).
- **P3 TickSpec, Fabulous, Fable.Remoting, FAKE, Quartz and MassTransit twins.**

Corpus additions:

- ScottArbeit/Grace (589; orleans, giraffe, fsharp-nunit)
- laurentlb/shader-minifier (587; bolero, elmish, generic bases)
- iminashi/Rocksmith2014.NET (116; avalonia, expecto)
- nojaf/fantomas-tools (35; aws-lambda, suave, elmish)
- danne931/akka-dotnet-bank (19; akka, aspnet-minimal, signalr, expecto)

### Haskell

- **Argument-less bindings read as values are dead.** WS26b records every
  non-lambda `x = …` as a data member and `value_refs::prune` drops values
  naming one (`value_refs.rs:184`), so scotty `routes`, servant `server = a :<|>
  b`, `app = serve api server` and tasty `unitTests` lose their only evidence.
  Credit them in the liveness pass without a `Via::Reference` edge (~114k edges
  otherwise). Check OCaml too. Reproduced (fixture, app-realworld-scotty).
- **tasty and hspec.** tasty lists only `tests`: add tasty-discover's `unit_*`,
  `prop_*`, `scprop_*`, `hprop_*`, `test_*`, `spec_*`, `tasty_*` (elm-format 2
  -> 16). Hand-wired hspec suites (`main = hspec $ do M.tests`, `tests :: Spec`;
  app-concordium-gotham: 25 modules, 0 entries) need a top-level signature's
  result type for rules to match (`Spec`, `SpecWith`, `TestTree`; the same
  serves wai `Application`, servant `Server API`, scotty `ScottyM ()`).
  Reproduced (prototype).
- **Instance methods carry no class.** `instance C T where` heads are never read
  (`haskell.rs:205`), so Yesod's `instance Yesod App` callbacks (`approot`,
  `makeLogger`) and `ToJSON`/`Show` methods are medium findings with no
  `implements-external-contract` cap. Record the class; give yesod `base_types`.
  Reproduced (fixture, app-stackage-yesod).
- **servant handler sets.** Every operand of a `:<|>` chain is a handler and
  already a value ref (`haskell.rs:262`); tag it with context `:<|>` to enter
  handlers without routes (after the data-member fix). Code only.
- **APPS claims.** hspec and `~haskell-main` are claimed by hspec's own repo;
  the only scotty evidence is a RealWorld demo. Code only.

New coverage (ranked):

- **P2 Quasi-quote splice references** (extraction; `.hamlet` files minor) calls
  in `[hsx|{f x}|]` and hamlet `#{}`/`^{}`: predimarkt 329 of 661 findings.
- **P2 sydtest, Servant generic records** `spec` (smos 142); `NamedRoutes` field
  values (share-api: 14 of 134 handlers dead).
- **P3 Happstack, Spock, Snap, HUnit, criterion/gauge/tasty-bench detect-only
  rows; GHC plugins; Template Haskell `'f` quotes (distributed-process
  `remotable`).**

Corpus additions:

- koalaman/shellcheck (40.4k; QuickCheck `prop_*`)
- simonmichael/hledger (4.8k; yesod)
- avh4/elm-format (1.3k; tasty-discover)
- NorfairKing/smos (312; sydtest, yesod, servant)
- haskell/hackage-server (470; Happstack)
- unisonweb/share-api (31; servant generic records)
- avitkauskas/predimarkt (7; IHP, the best public IHP app found)

### Clojure

- **`compojure` verbs through any alias mint entries.** `record_call` keeps only
  the operator's last segment (`clojure.rs:722-738`), so `(http/post url …)`,
  `(fs/delete p)` and `(lib/options x)` look like route-less Compojure verbs
  (metabase: 18 of 32 entries have no route). Keep the written operator in
  `context` and admit a route-less capture only when the verb is referred from,
  or its alias names, a `compojure.*` namespace; check Elixir and Perl, which
  also cut to the last segment. Reproduced (fixture).
- **Route shapes.** `(GET ["/user/:id" :id #"[0-9]+"] [id] body)` has no leading
  string, so no route or handler (`clojure.rs:772-775`): read the first string
  of a leading vector (clojars-web: 23 missed); slot 1 is always the request
  binding when a route exists (`clojure.rs:782`), so a user rule's `(myroute
  "/x" handler)` and `(testing "x" (f))` lose their target: make the slot a
  property of the Compojure verbs; Pedestal terse routes in a set literal `#{…}`
  are no values (`value_form`, `clojure.rs:1198-1215`). Reproduced (fixtures).
- **`-main`, `:gen-class` and `with-let`.** `UNIVERSAL_ENTRY` is `["main"]` and
  `LANG_ENTRY` has no Clojure row, so `(defn -main …)` and `-greet` in a
  `:gen-class` namespace are dead; root `-main` and `-name` defns there. Reagent
  `(r/with-let [..] [page])` walks no values after the vector: add `with-let` to
  `CLOJURE_BINDING_FORMS` (rems `rems.app/page`). Reproduced (fixtures).
- **ring and http-kit can enumerate.** `registrars: ["run-jetty",
  "run-undertow"]` and `["run-server"]` bind a named handler today; still
  missing: the `#'app` var form (only `sym_lit` is captured, `clojure.rs:841`),
  `(wrap-x h)` chains, channel callbacks. Reproduced (prototype).
- **`deftest` and `defspec` are no definitions.** `is_defn_form`
  (`clojure.rs:213-235`) lacks them, so their bodies are top-level calls and
  every function they call is a production `TopLevelInvocation` root, not
  test-only. Record them as test callables, give `clojure-test` a matcher, add
  `use-fixtures` as a registrar. Reproduced (fixture).
- **reitit and pedestal route data.** Handlers stay live through map and vector
  values, but no entries or labels exist: read `["path" {:get h}]`/`["path" :get
  h]` passed to `reitit.ring/router`, `reitit.http/router`, `expand-routes`,
  `table-routes` as registrar records. Reproduced (fixture).
- **APPS claims.** clojure-compojure is the framework's own repo and most of
  app-metabase-compojure's entries are the false route-less ones. Code only.

New coverage (ranked):

- **P2 secretary** (network; row) `(defroute "/x" [] body)`: 47 of 47 in rems.
- **P2 Framework def-macros** (extraction; one table of namespace and macro to
  kind) mount `defstate` (rems 19), pathom3 `defresolver`/`defmutation`, fulcro
  `defmutation`, trapperkeeper `defservice`, quartzite `defjob`, liberator
  `defresource`; test macros midje `fact`/`facts`, expectations `defexpect`.
- **P2 `shadow-cljs.edn` `:init-fn`/`:main`** (minor).
- **P3 sente, lacinia, chime, langohr, jackdaw, cljfx, electric.**

Corpus additions:

- repath-studio/repath-studio (175; re-frame, cljs exports)
- CSCfi/rems (68; a production service; re-frame, secretary, mount, compojure)
- clojars/clojars-web (484; real compojure app; 34 routed entries)
- puppetlabs/puppetserver (298; trapperkeeper, clojure-test)

### OCaml

- **Data bindings and `let … in` locals become callables.** A parameterless
  `let` is both a `DefRecord` and a `data_member` (`ocaml.rs:230-235`), so
  values naming it are pruned and it reads dead (Cmdliner `count`, `msg`,
  `cmd`); `let doc = … in` locals are callables too, and a bare `info` in `Arg.(
  … )` gets a `?` edge to `cmd`'s local. Keep non-function lets as data members
  only and skip `let … in` locals. Reproduced (fixture).
- **`Mod.f` into a workspace module never resolves.** `Util.helper` lands in the
  external bucket even with `open Util`; only bare names after `open` bind, so
  cross-file route handlers (ocaml.org: 84 `Handler.<x>` named in `router.ml`
  are dead) and `Router.router ()` read dead. Map a capitalised head to
  `m.ml`/`m.mli` as `open A.B` does (`cross_file.rs:2301`), for calls and values
  (known in dev_docs `ocaml.md` §04, §16; Haskell's twin is listed). Reproduced
  (fixtures).
- **ppx_inline_test and ppx_expect.** `let%expect_test "n"` and `let%test` make
  no callable and `let%test_unit _ =` makes a dead callable `_` (ocaml-dune: 119
  files). Tag `let%test*` bindings and add a test rule. Reproduced (fixture).
- **Detection.** `alcotest` misses runners written against `Alcotest_lwt`,
  `Alcotest_async`, `Alcotest_mirage` (ocaml-ci, zpool); add a detect-only
  `sihl` row (zpool routes with `Sihl.Web`; its `~opium` comes only from request
  helpers; upstream quiet since 2024-05). Reproduced (fixture).
- **APPS claims.** app-ocamlorg-dream and app-ocamlci-alcotest detect dream,
  alcotest and cmdliner but claim none, though CHANGELOG says cmdliner evidence
  moved there; app-opam-cmdliner vendors it as `OpamCmdliner`; no corpus Opium
  app registers `App.get` routes. Code only.

New coverage (ranked):

- **P2 cohttp server, `Callback.register`, QCheck, OUnit2** `Server.make
  ~callback` (learn-ocaml `callback` dead); C reaches `f` through
  `caml_named_value("n")` (unison 47 registrations); `Test.make ~name gen prop`;
  `"n" >:: f` operators need a new matcher (0install ~80 sites).
- **P3 Crowbar, lablgtk3 `#connect#clicked ~callback:f` (unison `uigtk3.ml`: 653
  findings), js_of_ocaml `Js.export`, Eliom `.eliom` files (minor).**

Corpus additions:

- bcpierce00/unison (5.6k; C stubs, `Callback.register`, lablgtk3)
- janestreet/magic-trace (6.4k; Jane Street Command, ppx_expect)
- mirage/qubes-mirage-firewall (241; MirageOS)
- robur-coop/albatross (172; cmdliner outside `bin/`, alcotest, C stubs)
- 0install/0install (586; OUnit2, lablgtk)
- ocaml-sf/learn-ocaml (335; cohttp server, js_of_ocaml, cmdliner)
- ComputerAidedLL/click-and-collect (27; real Opium `App.get`/`App.post` routes)

### Perl

- **Parse errors swallow routes.** In app-perlmaven-dancer2 `lib/Perl/Maven.pm`
  fails inside two `hook` bodies and only 6 of 23 routes become entries; modern
  Mojolicious headers (`use Mojo::Base -base;`, `-strict`, `use
  Mojolicious::Lite -signatures;`, `sub ($c) {...}`) all fail to parse with
  tree-sitter-perl 1.1.2 (78 of 237 files in Ravada). Blank `-flag` words and
  anonymous-sub signatures through `parse_source`, add a route-line rescan when
  `parse_status` is error, and report the inputs upstream. Reproduced (fixtures,
  corpus).
- **Route shapes.** Both DSLs spell DELETE `del` (dancer2's `delete` matches no
  keyword; mojolicious `route` was removed in 9.0); paren-less routes with three
  or more arguments (`get '/x' => {id => undef} => sub`, Dancer2 named routes)
  mint nothing and a named route is labelled by its name; `get qr{^/media/(.+)}
  => sub`, `get ['/a','/b'] => sub` and `any ['get','post'] => '/x'` are skipped
  (`registrar_route`, `perl.rs:486`). Ravada `rvd_front` and perlmaven lose
  routes. Reproduced (fixtures).
- **Mojolicious controllers.** `->to('ctrl#action')` is captured but
  `string_targets` is false and `decode_string_target` (`mod.rs:2082`) decodes
  `#` for Ruby only, so every action and `startup` is dead: add a Perl arm
  (`foo-bar#baz` -> `Foo::Bar::baz`), set `string_targets`, add a `startup`
  lifecycle row; `->to(controller => …, action => …)` needs key pairing.
  Reproduced (prototype).
- **Definitions carry no `base_types`.** Inheritance is recorded only as
  `perl-isa` imports, so no Perl base row can fire: Plack::Middleware
  `call`/`prepare_app`, Mojolicious::Plugin `register`, Catalyst
  `begin`/`end`/`auto` read dead. Stamp each sub with its package's bases.
  Reproduced (fixture).
- **`catalyst` can enumerate sub attributes.** `perl.rs:316` hardcodes empty
  attributes, but `:Local`, `:Path(...)`, `:Chained(...)` survive in
  `signature_hint`, even in files that fail to parse (116 of 128 controller subs
  in app-metacpan-catalyst); parse them and add attribute rows (`Path`, `Local`,
  `Global`, `Chained`, `Regex`, `LocalRegex`). Keep detection tree-wide:
  controllers extend an app base. Reproduced (fixture, corpus).
- **`test-more` and `plack`.** Detect `Test::Most` (all 8 `.t` files in
  perlmaven), `Test2::V0`, `Test2::Tools`, `Test::Class`, and add `registrars =
  ["subtest"]` (metacpan-catalyst: 57 subtest lines); a `mount` registrar
  enumerates `mount '/legacy' => \&app` in `.pl`/`.pm`, but `app.psgi` is
  skipped (parsing `.psgi` is minor). Reproduced (prototypes).
- **APPS claims.** `plack` and `test-more` are claimed only by the framework
  repos: add `~plack,~test-more` to app-metacpan-catalyst, `~test-more` to
  app-ravada-mojolicious and `~plack` to app-perlmaven-dancer2. Code only.

New coverage (ranked):

- **P2 App::Cmd, mod_perl, Plack::Component, Mojolicious commands, Test::Class**
  `execute`, `opt_spec`, `validate_args`, `abstract`, `usage_desc` (Dist-Zilla
  52 entries, 260 -> 186); `handler` (bugzilla 2); `call`, `prepare_app`,
  `to_app`; `run` of command subclasses (openQA 6); `test_*`, `test_startup` (`:
  Test` attributes need extraction).
- **P3 TheSchwartz, Gearman, Dist::Zilla roles, CGI::Application `run_modes` (a
  hash-of-handlers registrar mode also serves POE and Irssi).**

Corpus additions:

- os-autoinst/openQA (381; mojolicious full app, minion, moose)
- metacpan/metacpan-api (313; moose, catalyst, plack, test-more)
- irssi/scripts.irssi.org (503; irssi scripts)
- rjbs/Dist-Zilla (189; App::Cmd, moose)
- bugzilla/bugzilla (867; mod_perl, TheSchwartz, Perl lifecycle)

### R

- **`r-package-hooks` is gated on 15 common packages.** Package code imports
  through NAMESPACE, never `library()`, so `R/zzz.R` `.onLoad` gets no entry,
  while any script loading `data.table` reports a gap. Empty `detect` so the
  fixed-symbol branch (`mod.rs:323`) fires on the definition; drop defunct
  `.First.lib`. Julia `julia-module-init` (`__init__`) has the same gate.
  Reproduced (prototype).
- **Package files never bind each other.** Cross-file binding needs a
  `library`/`require`/`source` in the calling file, so the `R/*.R` of one
  package get no cross-file edges (`app_server` -> `mod_table_server`
  unresolved). Drop the import gate for R or scope bare calls to the DESCRIPTION
  root. Noted only in dev_docs `r.md:330`. Reproduced (fixture).
- **Definitions missed or misnamed.** `"[.cls" <- function` is no callable; ``
  `+.gg` `` keeps its backticks (a `+.*` pattern misses it; mermaid shows a
  half-escaped label); `R6Class`/`setRefClass` methods and top-level anonymous
  functions (plumber handlers, `server.R`) are not callables. Strip quotes and
  backticks; mint `Type.m` and `handler_at_<line>`. Reproduced (fixture).
- **Detection forms.** `requireNamespace("x")`, `pacman::p_load(x)`,
  `box::use(x[...])` (rhino), bare `pkg::fn()` and roxygen
  `@import`/`@importFrom` detect nothing; add builtin `detect_calls`
  (`shinyApp`, `moduleServer`, `pr`, `plumb`, `test_that`) and the `plumber2`
  prefix (same `#*` blocks plus `api() |> api_get(path, handler)`);
  NAMESPACE/DESCRIPTION reading is minor. Reproduced (fixtures).
- **`shiny` and `r-s3-dispatch` can enumerate.** A `server`/`app_server` methods
  row roots the server, whose reactive bodies are already attributed
  (parameter-name matching would need a new matcher); `print.*`, `format.*`,
  `summary.*`, `as.data.frame.*`, `as.list.*`, grid
  `widthDetails.*`/`grobWidth.*` rooted 17 genuine methods in r-ggplot2 (avoid
  `is.*`); package generics need a `UseMethod("g")` -> `g.*` fan-out. Reproduced
  (prototypes).
- **APPS claims.** The one Shiny app (tweetconf) uses legacy
  `server.R`/`callModule`; no golem, rhino, `moduleServer` or plumber2 app backs
  the fixes. Code only.

New coverage (ranked):

- **P2 R value references** (extraction) `lapply`, `purrr::map`/`partial`,
  `do.call`, `future_lapply`, `later::later` arguments; CDC `save_figure` via
  `purrr::partial` reads never-referenced.
- **P2 Package exports** (public; extraction) roxygen `#' @export` shares the
  plumber comment mechanism; NAMESPACE `export()` is minor; also covers OpenCPU.
- **P3 RUnit (`inst/unitTests/runit*.R` is no test file), tinytest (needs the
  top-level fix), `box` modules, RestRserve, ambiorix, fiery, httpuv.**

Corpus additions:

- CDCgov/wastewater-informed-covid-forecasting (51; targets, testthat)
- imbs-hl/ranger (817; Rcpp tables, `R_init`, `.Call`)
- ashbaldry/designer (153; golem shiny app via NAMESPACE)

### Julia

- **Callable-struct methods are not callables** (`flux`). `function (m::T)(x)`
  and `(m::T)(x) = …` record nothing (`julia.rs:85-92`, `:139`), so a forward
  pass's callees are production `toplevel:invocation` roots never linked to the
  layer. Record the method under the type with a call-operator marker so step 7
  can bind `model(x)`. Reproduced (fixture).
- **`julia-base-dispatch` sees few Base methods.** `Base.length(b::Bag) = …` is
  dropped (the `assignment` arm, `julia.rs:136-159`, takes only an `identifier`
  head) and `function Base.:(==)(a, b)` is named `Base.:`; `import Base: show`
  plus a bare `function show(…)` never roots; only 15 Base names are listed
  (`isless`, `firstindex`, `promote_rule`, `showerror`, `IteratorSize` and
  operators are dead), and `methods = ["Base.*"]` rooted 9 of 9 but is excluded
  from the self-evidence path (`mod.rs:322-333`). Take names from the head node,
  match bare defs in files importing them from Base, allow the pattern.
  Reproduced (fixtures, prototype).
- **Imports and includes.** `using X: a`, `import X: a`, `import X as Y` and
  `using ..X` record nothing (`record_import`, `julia.rs:220-253`, skips
  `selected_import`, `import_alias`, `import_path`), so `using Genie: route`
  detects nothing; `include("x.jl")` is not followed, so included files lose the
  module prefix and calls through `using M` stay `candidates-in-other-files`
  (matrix 16 lists it). Reproduced (fixtures).
- **Detection prefixes.** `julia-module-init` and `julia-base-dispatch` detect
  only on stdlib modules (a Genie-only package gets neither); `genie` misses
  `GenieFramework`, `Stipple`, `StippleUI`; `julia-test` misses `SafeTestsets`.
  Reproduced (fixtures).
- **`flux` protocol extensions can enumerate.** `ChainRulesCore.rrule`,
  `Functors.functor`,
  `LuxCore.initialparameters`/`initialstates`/`parameterlength`/`statelength`,
  `Optimisers.apply!`/`init`, `Adapt.adapt_structure` (node false; detect
  `LuxCore`, `ChainRulesCore`, `Functors`); 4 of 4 in a prototype; a generic
  Julia row may fit better. Reproduced.
- **APPS claims.** `julia-base-dispatch` enumerates only in Flux.jl and Genie.jl
  (also the only `julia-test` claimants); app-alphapem-genie detects `genie` and
  `julia-test` unclaimed, app-crnn-flux detects `flux` unclaimed; Oxygen and
  Bukdu have no app. Code only.

New coverage (ranked):

- **P2 Comonicon, `(@main)`** (cli) record a wrapping `@main`/`@cast` macro as
  an attribute (also Oxygen `@get "/p" function h`; AirspeedVelocity 3); name
  `function (@main)(args)` `main`.
- **P3 Gtk4.jl `signal_connect`, BenchmarkTools suites, JSONRPC.jl dispatch.**

Corpus additions:

- fonsp/Pluto.jl (5.4k; HTTP.jl, base dispatch, module init)
- JuliaRegistries/Registrator.jl (228; HTTP.jl)
- JuliaPluto/PlutoSliderServer.jl (158; HTTP.jl)
- JuliaLang/www.julialang.org (386; Franklin)
- MilesCranmer/AirspeedVelocity.jl (150; Comonicon)

### PowerShell

- **`azure-functions`: `run.ps1` alone is no function.** `detect_paths:
  ["run.ps1"]` fires on any launcher (the only corpus hit, app-orleans-samples,
  is a `docker build` wrapper). Require a sibling `function.json` or
  `detect_calls: ["Push-OutputBinding"]`. Reproduced (fixture).
- **Scope-qualified function names.** `function script:Get-X` and `global:X`
  keep the prefix in the simple name, so a call to `Get-X` binds nothing
  (chocolatey-packages 746, CIPP-API 82, winutil 62 such definitions); the agent
  section notes the same. Reproduced (fixture).
- **Import extraction.** `first_command_argument` (`powershell.rs:324`) reads
  only barewords, so `Import-Module 'Pester'` records nothing, `Import-Module A,
  B` only `A`, and `#Requires -Modules X` is unread (app-importexcel-pester 15
  files); `import_matches` is case-sensitive. Reproduced (fixtures).
- **Detection by the DSL.** pester `detect_calls: ["Describe", "Invoke-Pester"]`
  (NetworkingDsc: 62 `*.Tests.ps1`, pester never fires; winutil 742 `It`
  blocks); `powershell-dsc` `detect_paths: ["DSCResources/", "DscResources/"]`
  and `detect_calls: ["Import-DscResource"]`; `powershell-module` `detect_calls:
  ["Export-ModuleMember"]` (its prefixes fire only in build scripts). Autoloaded
  Pode, InvokeBuild and psake need `Start-PodeServer`, `Invoke-Build`,
  `Invoke-psake` calls and suffix paths (`*.build.ps1`). Reproduced
  (prototypes).
- **DSC resources.** `[DscResource()]` classes' `Get`/`Set`/`Test` and the
  DscResource.Base hooks (`GetCurrentState`, `Modify`, `AssertProperties`,
  `NormalizeProperties`) get no entry: hooks are a methods row (3 entries);
  `Get`/`Set`/`Test` need class attributes or `ClassDecl` bases. A backtick
  continuation before `)` in `@( … )` drops `Set-`/`Test-TargetResource` from
  NetworkingDsc `DSC_Route.psm1`; a same-length rewrite in `parse_source`
  recovers all five (29 `.psm1` carry the idiom). `Configuration X { }` is no
  callable: say so. Reproduced (fixture, corpus file).
- **Module exports.** No command arguments are captured, so `Export-ModuleMember
  -Function 'X'` and `.psd1` `FunctionsToExport = @('A')` are invisible
  (importexcel: 63 of 71 exports dead; dbatools 694 of 737). Capture the
  strings, then a registrar row roots each; wildcards stay in the gap.
  Reproduced (prototype).
- **APPS claims.** No corpus app has PowerShell Azure Functions
  (`function.json`) or `[DscResource()]` classes. Code only.

New coverage (ranked):

- **P2 Pode, InvokeBuild, psake** (network/cli; detect-only rows now,
  ScriptBlock callables to enumerate) routes, schedules, timers as `-ScriptBlock
  { }` (OpenGSMGateway 3 routes, 3 schedules); `task Name deps { }` (PSReadLine
  10 tasks).
- **P2 Azure Functions `function.json`** (minor) `scriptFile` plus `entryPoint`
  name an exported function (CIPP-API: 5 dead, no `run.ps1`); one reader also
  serves the Python v1 and JS v3 models.
- **P3 Engine callbacks (`IArgumentCompleter.CompleteArgument`,
  `IValidateSetValuesGenerator.GetValidValues`), PoshBot, PowerShell
  Universal.**

Corpus additions:

- KelvinTegelaar/CIPP-API (307; Azure Functions, module)
- chocolatey-community/chocolatey-packages (477; Chocolatey AU)
- ChrisTitusTech/winutil (66.7k; pester)
- dataplat/dbatools (2.9k; module exports, a Functions runner)
- PowerShell/PSReadLine (4.4k; InvokeBuild)
- demogorgonz/OpenGSMGateway (Pode)

### Zig

- **Builtin arguments are dropped.** The `builtin_function` arm returns without
  walking its children (`zig.rs:348-386`), so `@export(&f, …)` records no value
  (ghostty: 207 sites) and a call wrapped in any builtin, `@intCast(compute())`,
  has no edge at all. Walk builtin arguments; read `@export`'s first argument as
  a value. Reproduced (fixtures).
- **`build.zig` `build` is dead in every Zig repo** (`zig-build`). A `methods =
  ["build"]` row also enters `Builder.build` in `src/pool.zig`; gate it to
  `detect_paths` files, or use `signature_methods_for` `build(Build)` once
  name-first parameters parse. zls and libxev report it. Reproduced (prototype).
- **Container methods carry no owner.** `zig.rs:230-241` writes `qualified_name:
  name` and `FreeFunction` for every fn, so `Point.format` is `format` and no
  rule can tell a method from a free function (mermaid shows many duplicate bare
  names). Qualify members by container path and use `InherentMethod`. Reproduced
  (fixture).
- **std-invoked hooks are dead.** `format(self, *std.Io.Writer)` (called for
  `{f}`), `jsonStringify`, `jsonParse`, `jsonParseFromValue` (zls 4 `format`
  findings; ghostty 41 `pub fn format(`). Add a node-false `zig-std-hooks` row
  limited to container members (after the owner fix), the Zig analogue of
  `rust-std-traits`. Reproduced (fixture, zls).
- **Alternative entry points.** `LANG_ENTRY` has only `_start` for Zig
  (`roots.rs:29`); add `wWinMain`, `WinMain`, `EfiMain` (C/C++ `wWinMain` is the
  listed item); the legacy root-file `pub fn panic` (Zig 0.13 and older) is a
  finding too. Reproduced (fixtures).
- **Every Zig rule detects on `std`.** zls (65 files, no `export fn`) lists all
  four as seen. Detect `zig-build` by `build.zig`, `zig-export` by recorded
  `export` visibility, `zig-test` by a test declaration or `_test.zig`.
  Reproduced (coverage).
- **APPS claims.** `zig-export` is claimed only by app-libxev-zig, a library;
  claim `~zig-export` on app-ghostty-zig (119 `export fn`, 207 `@export(`). Code
  only.

New coverage (ranked):

- **P2 jetzig** (network; path-gated methods or the signature form) views
  `index`/`get`/`post`… under `src/app/views` are dead; no real app found.
- **P3 ziggy-pydust (needs visibility), mach systems (enum literals), zap
  (archived 2026), zigler `~Z` sigils (minor).**

Corpus additions:

- monome/maiden (56; httpz)
- ochi-team/ochi (68; httpz `dispatch`)
- nullclaw/nullclaw (8.2k; websocket, zig-build, zig-test)
- lightpanda-io/browser (42.5k; `export fn`, zig-build, zig-test)

### Lua

- **Host-global receivers bind in-tree functions by name.** `ngx.say`,
  `kong.log.err`, `love.graphics.draw`, `vim.notify` bind a same-named local or
  table member at medium (`ngx.on_abort(on_abort)` binds its own callback): ~240
  such edges in app-apisix-openresty, only 5 right. Add a builtin host-global
  table (`ngx`, `ndk`, `kong`, `love`, `vim`) refused as external unless the
  tree assigns into it, as `heads.rs` does. Reproduced (fixture, apisix).
- **String-keyed and returned table fields are no callables.** `return { ["/x"]
  = { GET = function(self) … end } }` (Kong Admin API `kong/api/routes`, plugin
  `api.lua`, Lapis `respond_to`) records nothing: `field` needs an identifier
  key and an assigned table. Name them by key path, then a kong row for verb
  keys under a path gate. Reproduced (fixture).
- **`pcall(require, "mod")` is no import.** The protected form (42 files in
  apisix and lua-kong) neither detects openresty/kong nor resolves calls through
  the module; treat `pcall`/`xpcall` of `require` with a literal as a require.
  Reproduced (fixture).
- **kong can enumerate phase methods with a gate.** Since WS8 `function
  H:access` has simple name `access`, so a methods row fires (circuit-breaker: 4
  correct entries), but without the path/owner gate it roots `_M.log` in helpers
  and 71 of 236 lua-kong entries outside `kong/plugins/*/handler.lua`. Add the
  row (`init_worker`, `configure`, `certificate`, `rewrite`, `access`,
  `response`, `header_filter`, `body_filter`, `log`, `preread`, `ws_*`) with the
  gate on members of the table `handler.lua` returns (`module_returns`). Kong
  PDK in Go, JS and Python has no rule. Reproduced (prototype).
- **APPS claims.** kong's only claimant, app-circuitbreaker-kong, is one 5-file
  plugin rock, not a deployment; find an application carrying several custom
  plugins. Code only.

New coverage (ranked):

- **P2 mpv scripts, Garry's Mod, Factorio, WoW Ace3** (rows after registrar
  capture) `register_event`, `add_key_binding`, `observe_property` (uosc: 382 of
  571 callables reported); `hook.Add`, `concommand.Add`, `net.Receive`;
  `script.on_event`/`on_init`, `commands.add_command`;
  `OnInitialize`/`OnEnable`, `RegisterEvent("X")` string targets (ElvUI 2
  entries).
- **P2 Defold, Nmap NSE, Roblox Luau** (minor: `.script`/`.gui_script` gated on
  `game.project`, `.nse`, `.luau`) defold-games: 433 files, 1,035 callbacks
  invisible.
- **P3 pandoc filters, Lapis `app:match`/`get`/`post`, `respond_to{GET=}`.**

Corpus additions:

- stevearc/oil.nvim (7.0k; Neovim plugin)
- MT-CTF/capturetheflag (102; Luanti)
- koreader/koreader (31.3k; busted)
- tomasklaen/uosc (3.5k; mpv script)
- a327ex/SNKRX (2.0k; LÖVE; shipped game, idle since 2022)
- benjames-171/defold-games (473; Defold)
- tukui-org/ElvUI (718; WoW Ace3)

### Solidity

- **Foundry tests are entered as attack surface.** `match_visibility` enters
  every public function of `*.t.sol`/`test/` contracts: 229 of morpho-blue's 365
  entry edges. Add a `foundry` test rule (detect `forge-std`, `ds-test`;
  `test*`, `invariant*`, `statefulFuzz*`, `table*`, `setUp`, `afterInvariant`)
  and skip test-role files in `match_visibility` (a harness entry never evicts a
  framework one). liquity/bold: 837 cases, stargate 516. Reproduced (prototype).
- **`fallback()` and `receive()` are no callables.**
  `fallback_receive_definition` (and 0.4's unnamed `function () payable`) is
  never recorded, so the two chain-invoked entries get none (Safe has 8). Record
  them as `C.fallback`/`C.receive`, external. Reproduced (fixture).
- **Modifier invocations are no calls.** `function f() external onlyOwner`
  records nothing, so every modifier and its helpers are dead (morpho-blue
  `Morpho.onlyOwner`). Record each `modifier_invocation` as a bare call.
  Reproduced (fixture).
- **Constructors and pre-0.5 visibility.** Every constructor not reached by `new
  C()` is a finding with a cascade (morpho-blue 4): root `constructor` (node
  false) and record the pre-0.4.22 contract-named constructor as one; an empty
  visibility on a contract, library or interface function is public before 0.5.
  Reproduced (fixtures).
- **OpenZeppelin hook overrides.** `_update`, `_authorizeUpgrade`,
  `_beforeTokenTransfer`, `_msgSender` and ERC4626/Governor hooks are called by
  an out-of-tree base: add a node-false row detected by
  `@openzeppelin/contracts[-upgradeable]` and `openzeppelin-contracts` (kleros:
  24 entries), and read `override_specifier` into `attrs::with_override_marker`.
  Reproduced (prototype).
- **Relative imports into `lib/` never match.** `import
  "../lib/forge-std/src/Test.sol"` (morpho-blue: 7 files) is kept verbatim and
  `prefix_keys` cuts only from the start; also record the key after a `lib/` or
  `node_modules/` segment (metamorpho: the foundry prototype finds 0).
  Reproduced (fixtures).
- **APPS claims.** `solidity-public` is claimed by solidity-openzeppelin (a
  library) and rust-bat (one syntax-test file); no real Solidity app is in the
  corpus. Code only.

New coverage (ranked):

- **P2 Uniswap v4 hooks, LayerZero OApp, ERC-4337 accounts** (network; rows)
  BaseHook `_beforeSwap` … `_afterDonate` (flaunchgg 14 of 14, 73 -> 43);
  `_lzReceive`, `_lzReceiveSimulate`, OFT `_debit`/`_credit` (stargate-v2);
  `_validateSignature`, `_validateNonce`, `_validatePaymasterUserOp`, `_postOp`.
- **P2 Hardhat tasks** (cli; TS and JS row; `hardhat/config`, not bare
  `hardhat`) `task`, `subtask`, `setAction`. stargate-v2 95, kleros 7.
- **P3 Axelar `_execute`/`_executeWithToken`, Hyperlane `_handle`,
  Echidna/Medusa/Halmos `echidna_*`/`property_*`/`optimize_*`/`check_*` (already
  public).**

Corpus additions:

- morpho-org/morpho-blue (Foundry; ~13 MB)
- safe-global/safe-smart-account (Hardhat; `fallback`/`receive`; ~11 MB)
- kleros/kleros-v2 (83; OpenZeppelin hooks, Chainlink VRF, Hardhat tasks)
- Frankencoin-ZCHF/Frankencoin (33; Chainlink CCIP)
- flayerlabs/flaunchgg-contracts (67; Uniswap v4 hooks; 252 MB with `lib/`)
- stargate-protocol/stargate-v2 (63; LayerZero, Hardhat tasks)
- liquity/bold (140; Foundry, real `solidity-public` app)
- morpho-org/metamorpho (198; relative `../lib/` imports)

### Bash

- **Calls inside `$(...)` in arguments and declarations.** `walk` returns after
  `record_command` and on every `declaration_command` (`bash.rs:60-69`), so
  `echo "$(f)"`, `local x=$(f)` and `export P="$(f)"` lose `f` (plain `v=$(f)`
  works). Walk argument and value children for substitutions only; the largest
  source of false Bash findings (go-fzf `__fzf_select__`). Reproduced (fixture,
  go-fzf).
- **`trap`, `complete -F` and `bind -x` handlers are never roots.** `trap` is
  dropped before any record (`bash.rs:164-198`) and no argument is captured, so
  a registrar row matches nothing. Record the handler word (trap's first action
  word, the word after `-F`/`-C`, the name after `:`) and add a node-false
  `bash-builtins` rule; zsh `compdef` alike. 64 `trap fn SIG` lines in 34 corpus
  repos. Reproduced (fixture, go-fzf).
- **Affix-built command names** (dynuse). acme invokes hooks as
  `cmd="${root}_add"; $cmd …`; 435 of 471 findings are such hooks. A Bash dynuse
  taking the literal affixes of the string assigned to a `$var` command word
  could screen same-affix functions without naming an app. Reproduced
  (bash-acme).
- **APPS claims.** bash-acme claims nothing and is the only Bash origin. Code
  only.

New coverage (ranked):

- **P2 argc, interactive registrations, APKBUILD and ebuild** `# @cmd` comments
  as attributes (llm-functions: 39 of 51 findings); alias values, `export -f`,
  `PROMPT_COMMAND`, `preexec_functions+=`, `__git_complete`, zsh
  `add-zsh-hook`/`zle -N` (forgit 29, bashhub 12), `command_not_found_handle`;
  `src_*`/`pkg_*` phases (minor: file types).
- **P3 bashunit (`*.test.sh`), bash_unit, OpenRC, ShellSpec.**

Corpus additions:

- mikefarah/yq (16.3k; shUnit2; 204 Bash findings)
- pyenv/pyenv (45.2k; bats, `complete -F`, `trap`)
- dokku/dokku (32.2k; bats; variable-path sources)
- CachyOS/CachyOS-PKGBUILDS (401; makepkg)
- wfxr/forgit (5.1k; alias registrations, bashunit)
- sigoden/llm-functions (786; argc)
- rcaloras/bashhub-client (1.3k; bash-preexec hooks)

### CMake

- **Callbacks named as arguments.** `cmake_language(CALL f)`,
  `cmake_language(DEFER … CALL f)`, `variable_watch(V f)` and
  `cmake_language(SET_DEPENDENCY_PROVIDER f …)` record only the built-in, so `f`
  gets no edge, root or doubt. In `plugins/cmake.rs` read `CALL f` (after
  `DEFER` options) as a call and record the watch/provider argument as a value
  or root mark. Reproduced (fixture).
- **Command names are case-insensitive.** `function(My_Helper)` called as
  `my_helper()`, or a top-level `DO_WORK()`, goes to the external bucket. Fold
  CMake names as Fortran does (`scope.rs:52`), in resolution and root matching.
  Reproduced (fixture).
- **`find_package` file conventions.** `find_package(Foo)` never tries
  `FindFoo.cmake`, `FooConfig.cmake` or `foo-config.cmake`
  (`cross_file.rs:2255-2263`), so a module's function binds every same-named
  copy at medium. Add those paths. Reproduced (fixture).
- **`cmake` detection misses most projects.** Only 32 `include`/`find_package`
  names detect it, so `find_package(Qt6|Python|GTest|CUDAToolkit)` trees get no
  disclosure, though the gap claims the top-level file is detected by name: add
  `detect_paths: &["CMakeLists.txt"]` and a test. Reproduced (prototype).
- **APPS claims.** No row names `cmake`; lua-neovim and app-ghostty-zig detect
  it and can take `~cmake`; the main CMake checkout is Kitware's own repo. Code
  only.

New coverage (ranked):

- **P2 ament_cmake and catkin extras** (public; extraction and resolver)
  `CONFIG_EXTRAS`/`CFG_EXTRAS` files are the package's CMake API (navigation2:
  `nav2_package` 42 sites, `nav2_add_test` 34, dead); a detection row first.
- **P2 ESP-IDF `project_include.cmake`** (public; path-keyed module-surface
  matcher, generalising `self_module_markers_for`) every component's file is
  included project-wide (fujinet `littlefs_create_partition_image` dead;
  xiaozhi-esp32 gets no cmake line).
- **P3 Zephyr `find_package(Zephyr)` (zmk is silent; the C row is above), vcpkg
  `portfile.cmake` (disclosure only).**

Corpus additions:

- zrythm/zrythm (3.2k; `DEFER CALL`, CPM)
- FujiNetWIFI/fujinet-firmware (300; vendored modules, ESP-IDF)
- 78/xiaozhi-esp32 (33.8k; ESP-IDF detection)

### Nix

- **Curried applications are recorded twice.** `walk` calls `record_call` on
  every `apply_expression` and `record_call` descends the chain, so `add 1 2`
  yields two refs and `import ./h.nix { inherit pkgs; }` adds a second import
  whose path is the attrset text (`nix.rs:71-90`, `:156-192`); the bogus imports
  feed detection and the "file has imports" gates. Record at the outermost
  apply, take the import target from the innermost. Reproduced (fixture: 2
  imports counted as 4).
- **Import-free flakes are undetected.** A `flake.nix` with `inputs`/`outputs`
  and no `import` (the usual form), `import inputs.nixpkgs {}`, `import
  (fetchTarball …) {}` and `imports = [ <home-manager/nixos> ]` are not even
  "seen"; `flake-utils` never fires (it is an input). Add `detect_paths:
  ["flake.nix"]` and read `imports = [ … ]` elements as import records.
  Reproduced (fixtures).
- **`nix` can enumerate flake outputs and module callbacks.** cgg already
  extracts `outputs = {…}: …`, one-attrpath outputs (`overlays.default = final:
  prev: …`, `nixosModules.x`) and mkOption `apply = x: …`; a methods row
  (`outputs`, `overlay`, `overlays.*`, `nixosModule`, `nixosModules.*`,
  `homeManagerModules.*`, `homeModules.*`, `darwinModules.*`, `flakeModules.*`,
  `apply`, mkOptionType `check`/`merge`) gave 39 real entries on
  nix-home-manager. Nested `overlays = { default = …; }` is not matched; update
  the gap. Reproduced (prototype).
- **APPS claims.** `~nix` is claimed only by app-jabref-javafx and
  haskell-pandoc (incidental packaging); nix-home-manager is the framework's own
  repo. Code only.

New coverage (ranked):

- **P2 Module system** (extraction) a file whose top level is a lambda (NixOS,
  home-manager, nix-darwin modules, `callPackage` packages, overlays) yields no
  callable (mitchellh/nixos-config: 21 files, 2 callables): mint a file callable
  and read `imports`, `callPackage ./p.nix`, `import ./m` as module-path
  entries.
- **P3 hercules-ci `herculesCI`, colmena `makeHive` hosts (after nested names
  and the module item).**

Corpus additions:

- nix-community/lanzaboote (1.9k; NixOS tests)
- zhaofengli/attic (2.2k; flake-parts)
- mitchellh/nixos-config (3.1k; module system)
- srid/nixos-config (589; flake-parts)
- dolphin-emu/sadm (50; Dolphin's live infrastructure; colmena)

### Starlark

- **No value references or registrar captures.** `StarlarkWalker::record_call`
  ignores `ExtractCtx` (`starlark.rs:30`), so `rule(implementation = _impl)`
  (also `aspect`, `repository_rule`, `module_extension`, `transition`) and
  `struct(basename = _basename)` exports leave each function never-referenced
  (kythe: 36 of 87 findings are `*_impl`; pixie 28 of 84). Call
  `registrar::capture`/`capture_value_refs` as `python.rs:296-325` does, and add
  Starlark to `FILE_SCOPED_VALUE_REF_LANGS` (`roots.rs:1368`): `_` names cannot
  be loaded. Reproduced (fixture).
- **`bazel` can enumerate once capture lands.** Registrars `rule`, `aspect`,
  `repository_rule`, `module_extension`, `transition`, `exec_transition`,
  `macro`, `subrule`, `materializer_rule`, `provider` (`init=`); the same row in
  Python recovers `rule(implementation = _impl)`. skylib
  `unittest.make`/`analysistest.make`/`loadingtest.make` and rules_testing
  `analysis_test(impl = …)` then need a test rule with receiver-qualified
  registrars (never bare `make`). Reproduced (prototype).
- **`bazel` detection.** Repos with `MODULE.bazel`/`WORKSPACE` and only local
  labels, or only `@rules_kotlin`, `@io_bazel_rules_kotlin`, `@rules_apple`,
  `@rules_swift`, `@rules_scala`, `@rules_android`, `@aspect_rules_ts`,
  `@aspect_rules_py`, `@hedron_compile_commands`, are never detected; add
  `detect_paths` `MODULE.bazel`, `WORKSPACE`, `WORKSPACE.bazel`, `BUILD.bazel`,
  `REPO.bazel` (not bare `BUILD`: Buck1, Pants, Please) and the prefixes;
  canonical `@@rules_python+//…` labels miss (`+` is no cut in `prefix_keys`).
  Reproduced (prototype).
- **Tiltfile, `copy.bara.sky` and Buck2 files are skipped.** All Starlark, all
  `unknown-extension` (Buck2 `TARGETS`/`PACKAGE` too). Adding the names is patch
  by the WS25 precedent (`BUILD` in 0.11.1); then a `buck2` row (`@prelude//`,
  `rule(impl = _impl)`, `anon_rule`, `bxl_main`; wows-toolkit 307 `*_impl`
  findings). Reproduced (fixtures).
- **APPS claims.** lua-kong detects `bazel` without claiming `~bazel`;
  app-kythe-bazel stays the evidence app. Code only.

New coverage (ranked):

- **P2 Pixlet/Tidbyt** (lifecycle; row; `render.star`, `schema.star`)
  `get_schema` roots 779 of 1,095 findings on tidbyt/community; `handler=`
  arguments need capture.
- **P2 Kurtosis** (cli; row; `detect_calls` `import_module`) `run`:
  ethereum-package candidates 378 -> 147; `import_module()` is no import record.
- **P2 Tilt** (cli; with the file-name fix; `ext://`) 7.1k Tiltfiles on GitHub;
  app-passbolt-cakephp has one.
- **P3 Drone `.drone.star` (entry node only), Copybara transforms, envd,
  lucicfg, ytt (a detect-only gap row).**

Corpus additions:

- pixie-io/pixie (6.6k; bazel)
- tidbyt/community (429; Pixlet)
- ethpandaops/ethereum-package (489; Kurtosis)
- landaire/wows-toolkit (99; Buck2; vendored `prelude/` dominates)
- xenia-project/xenia (9.9k; Drone)
- kubernetes-sigs/cluster-api (4.4k; Tilt)

### Fortran

- **`--externals` reads `bind(c)` exports as C-ABI imports.** The line scan of
  every `bind(c` line (`externals.rs:871-891`) counts procedures defined in the
  tree as `<c-abi>` sites leaving it at `high`. Only interface bodies are
  foreign declarations; read `language_binding` from the tree. Reproduced
  (fixture).
- **PROGRAM units are never roots or entries** (`fortran-program`). Programs,
  subroutines and functions are recorded alike (`fortran.rs:146-186`), so only a
  program named `main` is rooted (app-fpm-fortran: 47 PROGRAM units dead; a tree
  with no `program main` has no roots). Tag `program` nodes with a keyword
  marker kept out of attribute counting and let the rule enumerate it and
  replace its import list. Reproduced (fixture, app-fpm-fortran).
- **FINAL, type-bound procedures and procedure values.** `final ::` procedures
  run on deallocation; `procedure :: area => circle_area` targets are reached
  only via `c%area()`, which `record_call` drops without a record; `call
  integrate(square, a)` and `p => square` leave `square` dead (no Fortran value
  refs). Root FINAL targets, credit binding targets, record `x%m()` with a
  receiver, record argument identifiers and `=>` right-hand sides pruned through
  `locals::collect_fortran`. Reproduced (fixtures).
- **test-drive.** `new_unittest("n", proc)` inside collectors passed to
  `new_testsuite` gives cases not named `test_*` no entry; a test rule (detect
  `testdrive`, `mctc_env_testing`; registrars `new_unittest`, `new_testsuite`)
  needs the value-ref item (app-fpm-fortran: 314 lines). pFUnit `.pf` is minor.
  Reproduced (fixture).
- **Module prefix leaks and `.ftn`.** `extract_module` never resets
  `self.module`, so `program main_prog` after a module is `shapes::main_prog`
  (documented in dev_docs); `.ftn`/`.FTN` fixed-form files are
  `unknown-extension`. Reproduced (fixtures).
- **Detection of `include 'mpif.h'`.** Fortran `include` is no import record, so
  MPI-via-include trees go undetected; record it (case folding is
  cross-cutting). Reproduced (fixture).

New coverage (ranked):

- **P2 CCPP physics schemes** (lifecycle; extraction) record `\section
  arg_table_<name>` comments as an attribute; a suffix prototype on ccpp-physics
  roots 216 (174 true), clearing 183.
- **P2 pFUnit, f2py `.pyf`** (minor: file types) CTSM: 75 `.pf` files, 621
  `@Test`; adflow `adflow.pyf`: 95 of 120 procedures dead.
- **P2 PETSc callbacks** (lifecycle; C/C++ row, Fortran value refs)
  `SNESSetFunction`, `KSPMonitorSet`, `MatShellSetOperation`, `PCShellSetApply`:
  adflow 9 of 9 dead.
- **P3 MATLAB MEX `mexFunction` (Fortran and C/C++), gtk-fortran `c_funloc()`
  callbacks.**

Corpus additions:

- ESCOMP/CTSM (362; ESMF/NUOPC, pFUnit)
- mom-ocean/MOM6 (226; ESMF/NUOPC, PROGRAM units)
- NCAR/ccpp-physics (79; CCPP schemes)
- ammarkh95/ABAQUS_PDALAC (233; Abaqus, fixed form)
- mdolab/adflow (341; PETSc, f2py)
- dftd4/dftd4 (232; `bind(c)` API, test-drive)
- grimme-lab/xtb (890; PROGRAM units, test-drive)

### VHDL

- **No calls, instantiations or processes are extracted.** `vhdl.rs` records
  entities, architectures, subprograms and imports; the one reference is
  architecture -> entity, and the header's `function_call_expression` is a kind
  the grammar lacks (neorv32: 212 callables, 0% reach). Record
  `procedure_call_statement` and `concurrent_procedure_call_statement` callees,
  `component_instantiation_statement` targets (`entity work.x` last segment or
  the component name), a callable per `process_statement`, `name(args)` calls
  only where a VHDL function carries the name, and add `vhdl` to
  `CASE_INSENSITIVE_LANGS`. Patch; blocks every `vhdl-toplevel` follow-up.
  Verilog tasks and functions alike. Reproduced (fixture: 11 callables, 4 edges,
  0 sites).
- **Roots once processes exist.** Tag processes `process` and root them through
  a node-false `attributes` row on `vhdl-toplevel`; configuration declarations
  and `attribute foreign` need extraction; the top entity (named by no
  instantiation) needs an engine root plus an entity -> architecture edge.
  Verilog `always`/`initial` and the top module the same. Code only.

New coverage (ranked):

- **P2 OSVVM** (test; row plus extraction) `Tb*`, `tb_*`, `*_tb` harness
  entities (spi-to-axi-bridge 2, OpenJLS 33); test cases are TestCtrl
  architectures bound by `configuration X of Tb is … use entity
  work.TestCtrl(arch)`, so extract `configuration_declaration`. An `attributes
  ["architecture"]` row fires tree-wide (153 of 186 OpenJLS entries were RTL).
- **P2 UVVM** (test; row; `uvvm_util`, `uvvm_vvc_framework`) the same patterns;
  no real app over 30 stars found outside UVVM's repos.
- **P3 FuseSoC `.core` `toplevel:` (minor; useful after instantiation edges),
  VUnit `run.py` `pre_config`/`post_check` callbacks.**

Corpus additions:

- antonblanchard/microwatt (730; VUnit, VHPIDIRECT, FuseSoC)
- OpenResearchInstitute/dvb_fpga (152; VUnit, OSVVM packages)
- airhdl/spi-to-axi-bridge (56; OSVVM TestCtrl and configurations)
- isentropic-fpga/OpenJLS (16; OSVVM testbenches)

### Interface languages (Protobuf, GraphQL, OpenAPI, AsyncAPI, Smithy)

New coverage (ranked):

- **P2 oapi-codegen, lighthouse, ariadne** (network) PascalCase(`operationId`)
  methods on the handed-in server type; resolvers named by schema
  `@field(resolver:)` or `Queries\X::__invoke`; `@query.field("x")` (verb
  `field`; Misago).
- **P3 smithy-rs** (network; linker) handlers are already live by value ref;
  only the `ns#Op` edge is missing. Apollo/graphql-yoga resolver-map keys:
  entries only.
- GraphQL and gRPC server rows are listed per language: Python graphene,
  strawberry, grpc-python; JS/TS nestjs-graphql, TypeGraphQL; Java and Kotlin
  spring-graphql, DGS, grpc-java, grpc-kotlin; C# grpc-dotnet, Hot Chocolate; Go
  gqlgen, grpc-go; Ruby graphql-ruby, gruf; Elixir Absinthe, elixir-grpc; Rust
  tonic; Swift grpc-swift; C++ gRPC.

Corpus additions:

- kubeflow/katib (1.7k; grpc-python, grpc-go, descriptor linker)
- hoppscotch/hoppscotch (81.0k; nestjs-graphql; 224 resolver methods dead)
- lldap/lldap (6.6k; juniper, replacing examples-only evidence)

### Assembly, HCL, Kivy, Lean and Verilog

New coverage (ranked):

- **P2 Assembly function macros** (extraction; builtin table)
  `SYM_FUNC_START(x)`, `ENTRY(x)`, LK `FUNCTION(x)`, x86inc `cglobal x` define
  no callable, so their bodies' calls vanish.
- **P2 HDL top and testbench modules** (lifecycle; matcher; Verilog and VHDL)
  root never-instantiated portless modules (a fixture `tb_top` reads dead);
  cocotb `toplevel="x"`/`hdl_toplevel=` in a runner names the DUT (taxi: 135 of
  293 dead Verilog names); Verilator `Vtop`/`top->eval()` is subsumed.
- **P2 UVM** (test; extraction then a row; `uvm_pkg`) SystemVerilog classes have
  no owner, bases or imports; then
  `uvm_test`/`env`/`agent`/`driver`/`monitor`/`scoreboard`/`sequence` bases with
  `*_phase`, `body`, `write` (ibex: 66 classes, 12 `build_phase` dead).
- **P2 Lean tactic-database attributes** (lifecycle; node-false row) `@[simp]`
  (batteries 676), `@[ext]`, `@[norm_cast]`, `instance`.
- **P2 KV in `Builder.load_string`** (minor) 3 of 7 corpus Kivy apps, 40 files;
  only a `BLIND_SPOTS` line today.
- **P3 Lake files** (`lakefile.lean` yields 0 callables: `lean_exe`, `script`,
  `@[test_driver]`; the TOML form is minor), Lean `elab`/`macro`/`syntax`
  declarations (158 in batteries), KV bindings as entry nodes (roots today),
  linker-script `ENTRY()` (minor; 14 of 16 name `_start`).

Corpus additions:

- riscv-software-src/opensbi (1.6k; assembly extraction)
- lowRISC/ibex (2.2k; UVM, DPI-C)
- lean-dojo/LeanCopilot (1.4k; Lean attributes and FFI)
- nomeata/loogle (162; Lean `@[extern]`; `cliMain` dead though `main` calls it)

## AI agent and tool-calling frameworks (2026-11-25)

Twelve agents studied agent, LLM tool-calling and MCP frameworks in every
language cgg reads: fixtures per registration shape, prototype
`[[framework]]` rows, and real applications run against 0.13.3 (sources
ingested into nkb). New rows and fixes to matchers or plugins are patch
level; the `tool` entry kind and the new matchers marked minor go in 0.14.0.

### Engine and extraction

Tools, graph nodes, steps, hooks, guardrails and runtime entry points are
called by the framework, never by the app. Six shapes cover every
framework seen: a marker (A: `@mcp.tool`, `@Tool`, `#[tool]`,
`[KernelFunction]`), a registration call taking a function value or
closure (B/C: `tool({ execute })`, `s.AddTool(t, h)`, `tools=[f]`), a
base type (D: `BaseTool._run`, `impl Tool`, `ServerHandler`), a string
naming a member (E: `spec_functions`, `this.schedule(1, "m")`), a module
path convention (F: xmcp `src/tools/*.ts`) and a class-level marker naming
members (`@component`, `#[AsTool(method:)]`, `#[derive(Tool)]`). Functions
passed as values already stay live through value references in most
languages; what is missing is the entry, and what reads dead is A, D and
method-shorthand B. The changes below serve every framework at once.

- **P2 Curried registrars** (python first, then js/ts; matcher, patch).
  `a.register_for_llm(d)(f)`, `mcp.tool()(f)`, `app.route("/x")(view)`:
  the outer call has no callee name, so nothing captures `f` (Agent-E:
  10 tools, 0 entries). Capture the outer arguments under the inner verb.
- **P2 Constructed handler objects** (java, kotlin, c#, scala, ts,
  python, go; matcher, patch). `Wrap(A)` expands to the rule's methods
  but `Wrap(B())`, `new X()`, `&x{}` do not (A2A Go `NewHandler(&x{})`,
  Vertex templates, Spring AI `FunctionToolCallback`, MCP Java
  `SyncToolSpecification`). Expand a construction argument like a type,
  and in dead code let a class constructed in live code keep its
  external-contract overrides live.
- **P2 Registrar capture in more plugins** (kotlin, scala, dart,
  powershell, r, php; extraction, patch). Kotlin and Scala never call
  `registrar::capture`, so every registrar row there is inert; R and
  Julia record no value refs at all (ellmer `tool(impl)` dead, also
  `lapply`); PHP decodes no `[X::class, 'm']`, `[$this, 'm']` or
  closures; Rust does not peel `Arc::new`/`Box::new`/`Box::pin`; Go skips
  `composite_literal` fields (`ServerTool{Handler: h}`, ADK config
  callbacks, cobra `RunE`).
- **P2 Class-level markers naming members** (python, php, c#, swift,
  rust; new matcher, minor). Haystack `@component` (Python records no
  class decorators), `#[AsTool(..., method:)]`, `[GenerateJsonSchema]`
  on an interface, Swift attribute macros (none recorded; also blocks
  swift-testing), swiftide `#[derive(Tool)]` calling the snake_case
  method. Extend `container_attributes_for` with a member filter.
- **P2 String-named members** (python, ts, ruby; matcher, patch).
  LlamaIndex `spec_functions = ["m"]`, Langroid `request = "m"`,
  langchainrb `define_function :m`, and a string after the route
  (`this.schedule(60, "cleanup")`: 4 of 8 targets dead on Mail-0/Zero).
- **P2 Framework annotations cap dead code at medium** (java, kotlin,
  c#; dead code, patch). A package-private `@Tool` or `@McpTool` from a
  detected import reads `high` dead; an unrecognised annotation from a
  detected framework should cap at `medium`.
- **P3 Gap text** (all languages; rules and output, patch). Gap text
  prints only for rules with no matchers. Add detection-only rows for
  raw provider SDKs (`openai`, `anthropic`, `google.genai`, boto3
  Bedrock action groups) with gap text such as: "Tool calls come back as
  JSON naming a function and the app dispatches them, usually through a
  dict or a match on the name. cgg keeps functions stored in a table
  live through value references but mints no entry; treat every
  function in the app's tool table as a model-invoked root." Let a rule
  with matchers also print a partial gap (typed-literal guardrails,
  `spec_functions`) in the coverage table.
- **P3 No new agent-to-tool edge** (all languages; output, patch). The
  registration site's enclosing callable already reaches the tool as a
  `?ref` edge under `--reference-edges` (fixture `py-kw`: `build -> search`).
  Record that callable as `registered_by` on the JSON entry so tools
  group by agent; do not add a default edge.
- **P3 Wrapper propagation** (go, ts, python; new matcher, minor). A
  helper whose function parameter reaches a registrar argument makes its
  call sites registrations: github-mcp-server `NewTool[In,Out]` (~100
  tools), mcp-grafana `MustTool` (143) stop at the SDK seam today.
- **P3 Path and config entry points** (ts, python; new matchers,
  minor). Default exports of `src/tools/*.ts` (xmcp, Raycast; shared
  with the Next.js/SvelteKit/Nuxt/Remix/Astro gaps) and `langgraph.json`
  `graphs`/`auth.path` `file.py:attr` entries.
- **P3 One verb in two rules** (all languages; dedup, patch).
  `ToolNode` in both a LangChain and a LangGraph row mints two entries
  for one target; dedup entries per site and target.

### Python

New coverage (ranked):

- **P2 Google ADK** (B + D, rows; `google.adk`). `Agent(tools=,
  before_model_callback=)` wants keyword slots; `_run_async_impl` dead.
- **P2 Microsoft Agent Framework** (A + B + D, rows; `agent_framework`).
  `@ai_function`/`@tool`, `Executor` + `@handler`, `@executor`,
  middleware `process`.
- **P2 AG2 / AutoGen 0.2** (A + B, rows + curried registrars;
  `autogen`). `register_function`, `register_for_llm`, `register_reply`.
- **P2 AutoGen 0.4+** (A + B + D, rows; `autogen_agentchat`,
  `autogen_core`). `@message_handler`/`@event`/`@rpc`, `FunctionTool`.
- **P2 Haystack** (class marker; `haystack`). `@component` `run`,
  `run_async`, `warm_up`; tools are a plain row.
- **P2 Semantic Kernel** (A + B + D, rows; `semantic_kernel`).
  `@kernel_function`, `@kernel.filter`, `add_filter`.
- **P2 smolagents, Agno, browser-use** (A + D, rows; `smolagents`,
  `agno`, `browser_use`). `@tool`, `Tool.forward/setup`, `Toolkit`,
  `@tools.action`.
- **P2 Powertools Bedrock agents** (A, matcher on the existing rows;
  `aws_lambda_powertools`). Add `tool` to the powertools resolvers.
- **P3 Agent Squad, Azure AI Agents, Vertex AI Agent Engine** (B + D;
  `agent_squad`, `azure.ai.agents`, `vertexai`). Vertex templates need
  construction expansion.
- **P3 Burr, PocketFlow, Griptape, Langroid, Mirascope, Atomic Agents,
  Letta, Marvin/ControlFlow** (rows). `@action`, `Node.prep/exec/post`,
  `@activity`, `ToolMessage.handle`, `@marvin.fn`; Atomic Agents tools
  are called explicitly, so only context providers need a row.

Fixes:

- **P2 Bound methods through untyped receivers.** `add_node("p",
  agents["e"].plan)` binds only a unique name: 7 of 11 gpt-researcher
  nodes stay unbound.
- **P3 Keyword values credit by name.** A top-level `on_handoff=
  on_handoff` credits the same-named `MyAgentHooks.on_handoff`.

### JavaScript and TypeScript

New coverage (ranked):

- **P2 LangGraph.js** (B/C, row; `@langchain/langgraph`). `addNode`,
  `addConditionalEdges`, `task`, `entrypoint`, `createReactAgent`.
- **P2 FastMCP TS, tmcp, mcp-use, mcp-framework** (B + D, rows;
  `fastmcp`, `tmcp`, `mcp-use`, `mcp-framework`). `addTool({ execute })`,
  `server.tool({ name }, h)`, `MCPTool.execute`.
- **P2 CopilotKit** (B, row + registrar alias; `@copilotkit/*`).
  `useCopilotAction`, `useFrontendTool`, `CopilotRuntime({ actions })`.
- **P2 Google ADK TS, Genkit, Strands TS, AgentCore TS** (B/C, rows;
  `@google/adk`, `genkit`, `@strands-agents/sdk`, `bedrock-agentcore`).
  `FunctionTool({ execute })`, `defineTool`/`defineFlow`,
  `tool({ callback })`, `addHook`, `invocationHandler`.
- **P2 VoltAgent, Inngest AgentKit, MS 365 Agents, LlamaIndex.TS, A2A JS**
  (B/C + D, rows; `@voltagent/core`, `@inngest/agent-kit`,
  `@microsoft/agents-hosting`, `llamaindex`, `@a2a-js/sdk`).
  LlamaIndex `wf.handle([ev], fn)` needs the ambiguous-verb fix.
- **P3 BeeAI, Ax, xmcp, Raycast AI tools** (rows; path matcher for the
  last two; `beeai-framework`, `@ax-llm/ax`, `xmcp`, `@raycast/api`).

Fixes:

- **P2 `.d.ts` ambient declarations in dead code.** 5,231 of 5,295
  findings on mcp-server-cloudflare are in `worker-configuration.d.ts`.
- **P3 zod `.describe()`** makes `js-test-globals` report "seen, no
  rules" in files with no tests.

### Java

New coverage (ranked):

- **P2 Quarkus MCP server** (A, row; `io.quarkiverse.mcp.server`).
  `@Tool`/`@Prompt`/`@Resource`/`@Complete*`; EDDI 14 -> 2.
- **P2 Google ADK Java** (A + D + new extraction; `com.google.adk`).
  `FunctionTool.create(X.class, "m")` needs class-literal and string
  capture in Java registrars.
- **P2 Embabel, AgentScope** (A, rows; `com.embabel.agent`,
  `io.agentscope`). GOAP `@Action`/`@AchievesGoal`/`@Condition`,
  `toolkit.registerTool(obj)` `@Tool`s.
- **P3 MCP Java SDK, langgraph4j, Spring AI Alibaba Graph** (D, rows;
  `io.modelcontextprotocol`, `org.bsc.langgraph4j`,
  `com.alibaba.cloud.ai.graph`). Lambdas are live; handler objects need
  the constructed-object change.
- **P3 Semantic Kernel Java, Spring AI AgentCore, A2A Java, Solon AI,
  Agents-Flex, Tools4AI** (A/D, rows). `@DefineKernelFunction`,
  `@AgentCoreInvocation`, `AgentExecutor`, `@ToolMapping`, `@ToolDef`.

Fixes:

- **P2 Java registrar arguments.** `object_creation_expression`,
  `class_literal` and method references are not captured, so builder
  registrations (`.tools(new X())`) cannot be rows.

### Kotlin

New coverage (ranked):

- **P2 MCP Kotlin SDK** (B/C, row after Kotlin registrar capture;
  `io.modelcontextprotocol.kotlin.sdk.server`). `addTool`/`addPrompt`/
  `addResource`; handlers live, no entries (crosspaste, burp-ai-agent).
- **P2 Embabel, Google ADK Kotlin, A2A Kotlin** (A + D, rows;
  `com.embabel.agent`, `com.google.adk.kt`, `io.a2a`). KSP-generated
  `generatedTools()` leaves `@Tool` with no caller.

Fixes:

- **P2 `Foo()::m`.** A bound reference through a construction stays
  unresolved (`candidates-in-other-files`).
- **P3 Stale `kotlin-runtime` gap text** (`rules.rs:8104`) says Kotlin
  records no value references.

### Scala

New coverage (ranked):

- **P2 LangChain4j twin** (A, row; `dev.langchain4j`).
- **P3 sttp-ai** (A, row; `sttp.ai.core.agent`). `@description` trait
  methods via `AgentTools.derive[T]`.

Fixes:

- **P3 Scala 3 `@main`.** `@main def server()` is never rooted
  (`UNIVERSAL_ENTRY` holds only `main`); seen in chimp MCP servers.

### C# and F#

New coverage (ranked):

- **P2 Microsoft.Extensions.AI** (B + D, rows + ambiguous-verb fix;
  `Microsoft.Extensions.AI`). `AIFunctionFactory.Create(f)`,
  `AIFunction.InvokeCoreAsync`.
- **P2 Microsoft 365 Agents SDK** (A + B, rows;
  `Microsoft.Agents.Builder`). `[MessageRoute]` and `*Route`, `OnActivity`.
- **P3 A2A .NET, CSharpToJsonSchema, AutoGen .NET, LlmTornado,
  BotSharp** (A/D, rows; class marker for `[GenerateJsonSchema]`;
  AutoGen `[Function]` collides with Azure Functions).

Fixes:

- **P2 Collection expressions.** `tools: [GetWeather]` elements are not
  value refs; a private tool passed only there is dead at `high`.
- **P3 Top-level statements are no callable.** `Program.cs` wiring has
  no source node in the graph.

### Go

New coverage (ranked):

- **P2 fantasy** (B/C, row; `charm.land/fantasy`). `NewAgentTool`;
  crush 38 entries.
- **P2 LangChainGo** (Go method sets; `github.com/tmc/langchaingo`).
  `tools.Tool` Name/Description/Call and `callbacks.Handler` need a
  framework interface table checked through `go_iface`.
- **P2 Genkit Go, ADK Go** (B/C, rows; `github.com/firebase/genkit/go`,
  `google.golang.org/adk`). `DefineTool`/`DefineFlow`,
  `functiontool.New` (needs a built-in path verb).
- **P3 Small Go tool APIs** (rows). anthropic-sdk-go toolrunner,
  trpc-agent-go, mcp-golang, go-mcp, openai-agents-go, langgraphgo.

Fixes:

- **P2 Composite-literal typing.** `x := &T{}` leaves `x` untyped:
  `mcp.Use(y.B)` drops and `x.A()` fans out.

### Rust

New coverage (ranked):

- **P2 ADK-Rust** (A + D + B, rows; `adk_rust`, `adk_core`, `adk_tool`,
  `adk_agent`, `adk_graph`). `#[tool]`, `impl Tool { execute }`,
  `impl Agent { run }`.
- **P2 graph-flow (rs-graph-llm)** (D, row; `graph_flow`). `impl Task
  { run }`; qbit 8 recovered.
- **P2 Swiftide** (A + D + derive matcher; `swiftide`). kwaak 7 dead.
- **P2 mistral.rs, rust-mcp-sdk, pmcp** (B/D, rows; `mistralrs`,
  `rust_mcp_sdk`, `pmcp`). `with_tool_callback` needs `Arc::new` peeling.
- **P3 AutoAgents, langchain-rust, ollama-rs, swarms-rs, turbomcp**
  (A/D, rows). Each fixture -> 0. llm-chain (last release 2023): skip.

Fixes:

- **P3 rtic `task`.** The marker-only rtic rule fires on adk-graph
  `#[task]`; gate it on `#[rtic::app]` or the dependency.

### Ruby

New coverage (ranked):

- **P2 fast-mcp, ai-agents, ActiveAgent, langchainrb** (D/E, rows;
  `fast_mcp`, `agents`). `Tool#call`, `Resource#content`, `perform`,
  `define_function :m`.
- **P3 Raix** (B, row; `raix`). Node only; the block is already live.

Fixes:

- **P2 `class << self`.** Methods there lose the class's bases.

### PHP

New coverage (ranked):

- **P2 Neuron AI, php-mcp/server** (A/B/D, rows; `NeuronAI`,
  `PhpMcp\Server`). `withTool([C::class, 'm'])` mints a node, no edge.
- **P3 LLPhant** (new matcher; `LLPhant`). `new FunctionInfo('m', $obj)`.

Fixes:

- **P3 Trait methods** used by tool classes are not credited by base
  matching (relaticle).

### Elixir

New coverage (ranked):

- **P2 Anubis/Hermes MCP, Jido** (D, rows; `Anubis`, `Hermes`, `Jido`).
  `execute/2`, `read/2`, `run/2`, `on_*`.

Fixes:

- **P2 Top-level roots.** `@spec run(...)` roots all 83 `run/2` in
  loomkin; an ExUnit `test` body roots 24 `execute/2` in sanbase2.
- **P2 MFA tuples.** `{Mod, :fun}` (ReqLLM `callback:`) are no value refs.

### Swift

New coverage (ranked):

- **P2 SwiftMCP, Swarm** (A; needs Swift attribute capture).
- **P3 MCP Swift SDK** (B/C, row; `MCP`). Entries only (iMCP).

### Dart

Fixes:

New coverage (ranked):

- **P3 langchain_dart** (D, row; `package:langchain`). `invokeInternal`.

### R, Julia, PowerShell, C++

- **P2 R ellmer / mcptools** (value refs in R; `ellmer`). side 9 of 9
  tool impls dead; no rule needed once refs exist.
- **P3 Julia PromptingTools.jl** (value refs in Julia).
- **P3 PowerShell PSAI / PSMCP.** Strip `Global:`/`script:` from
  function names, then read `Register-Tool F` as a string target.
- **P3 cpp-mcp** (B, row; `mcp_server.h`). Entries only.
- Not worth a rule: Lua, Clojure, Perl, Zig, Haskell, OCaml, Erlang
  (no SDK over 60 stars, or already reached).

### Corpus additions

The corpus holds no agent application today. Counts are 0.13.3 findings.
Before adding: confirm that the vendor-owned repos (cloudflare/,
google-gemini/, github/, charmbracelet/crush, awslabs/loom) are real
applications rather than samples, and check the large ones (deer-flow,
gemini-cli, kestra) against the 400 MB clone limit.

- bytedance/deer-flow (85,772; LangChain + LangGraph; 1,836 findings)
- stanford-oval/storm (31,948; DSPy; 198, 31 `forward` dead)
- assafelovic/gpt-researcher (30,895; LangGraph; 334, no entries)
- droidrun/mobilerun (10,112; LlamaIndex Workflows; 204, 32 steps dead)
- EmergenceAI/Agent-E (1,257; AG2; 67, 0 tool entries)
- 0x4m4/hexstrike-ai (13,885; FastMCP; 175, 143 tools dead)
- strnad/CrewAI-Studio (1,371; CrewAI; 177, 6 `_run` dead)
- divar-ir/ai-doc-gen (772; Pydantic AI; own `agents/` trips detect)
- Beever-AI/beever-atlas (455; Google ADK + FastMCP; 1,011 findings)
- FradSer/mcp-server-mas-sequential-thinking (307; Agno + FastMCP; 56)
- ralliesai/tenk (134; OpenAI Agents SDK; tools live, no entries)
- awslabs/loom (204; Strands + AgentCore; 4 of 16 dead)
- gensecaihq/Wazuh-Autopilot (68; Strands; 14 of 25 hooks dead)
- Zen7-Labs/Zen7-Payment-Agent (177; A2A Python; executor dead)
- google-gemini/gemini-cli (109,080; A2A JS; executor live in tests)
- miurla/morphic (9,357; Vercel AI SDK; 256, 5 `execute` dead)
- elie222/inbox-zero (13,115; AI SDK + MCP; 3,051, 0 tool entries)
- op7418/CodePilot (6,521; Claude Agent SDK + AI SDK; 1,141)
- upstash/context7 (65,415; MCP TS SDK; 161, 0 MCP entries)
- datagallery-ai/dataagent (748; Mastra; 2,440, 0 tool entries)
- RussellCanfield/wingman-ai-vscode-extension (265; LangGraph.js; 529)
- Mail-0/Zero (10,900; Cloudflare Agents; 1,186, 8 schedules dead)
- cloudflare/mcp-server-cloudflare (4,497; MCP SDK + McpAgent; 5,295,
  5,231 in `.d.ts`)
- brightdata/brightdata-mcp (2,738; FastMCP TS; 0 FastMCP entries)
- microsoft/vscode-pull-request-github (2,646; VS Code LM tools; 959)
- ALwrity/ALwrity (1,230; CopilotKit; 4,168, 0 CopilotKit entries)
- kestra-io/kestra (30,337; LangChain4j + MCP SDK; 29 `@Tool` dead)
- apache/hertzbeat (7,476; Spring AI; 189 `@Tool` dead)
- ageerle/ruoyi-ai (5,842; LangChain4j agentic; 31 dead)
- labsai/EDDI (393; Quarkus LangChain4j + MCP; 14 tools dead)
- block/trailblaze (335; Koog + MCP SDK; 19 `@Tool` dead)
- maikotrindade/mobile-tester-agent (112; Koog; 24 `@Tool` dead)
- askimo-ai/askimo (570; LangChain4j Kotlin; 24 `@Tool` dead)
- JetBrains/klibs-io (109; Spring AI MCP Kotlin; `ToolCallback` dead)
- CrossPaste/crosspaste-desktop (2,733; MCP Kotlin SDK; no entries)
- six2dez/burp-ai-agent (1,600; MCP Kotlin SDK on Ktor; no entries)
- daniel3303/Equibles (248; MCP C# SDK; 6 tools + 35 helpers dead)
- clawdotnet/openclaw.net (539; MCP, SK, MAF; 34 of 56 tools dead)
- foxminchan/BookWorm (506; MAF workflows; 14 executors dead)
- sbroenne/mcp-server-excel (961; MCP C# SDK; 2 of 3 tools dead)
- junkai-li/NetCoreKevin (558; Microsoft.Extensions.AI; ~36 dead)
- github/github-mcp-server (34.7k; MCP go-sdk; 121, 0 entries)
- charmbracelet/crush (29.2k; fantasy + go-sdk; 584, 0 entries)
- k8sgpt-ai/k8sgpt (8.4k; mcp-go; 34, handlers ref-only)
- grafana/mcp-grafana (3.7k; MCP go-sdk; 104, 0 entries)
- dromara/mayfly-go (2.4k; Eino; 1,923, 6 Eino-caused)
- isaacphi/mcp-language-server (1.6k; mcp-go; 137, 0 entries)
- infinilabs/coco-server (151; LangChainGo; 1,187, 5 tool methods)
- sopaco/deepwiki-rs (3,179; Rig + rmcp; 214, 23 tool/helper)
- Jakedismo/codegraph-rust (921; Rig + rmcp; 867, 24 Tool members)
- BeaconBay/ck (1,772; rmcp `ServerHandler`; 163, 3 handlers)
- evilsocket/legba (1,954; rmcp macros; 83, description as route)
- qbit-ai/qbit (127; Rig + graph-flow + rmcp; 1,079, 37 misattributed)
- bosun-ai/kwaak (338; Swiftide + rmcp; 67, 7 derive(Tool) methods)
- rubyevents/rubyevents (571; RubyLLM; 19 `execute` + 20 helpers dead)
- CanineHQ/canine (2,950; MCP Ruby SDK; 35 `call` + 5 `template`)
- relaticle/relaticle (1,927; Laravel MCP + AI; 214 tool methods)
- escapeboy/agent-fleet-o (84; Laravel MCP/AI + Prism; 1,836 findings)
- santiment/sanbase2 (94; Anubis MCP; 21 callbacks dead)
- pass-agent/loomkin (183; Jido; `run/2` masked by `@spec`)
- sozercan/kaset (2,649; Foundation Models; 2 `Tool.call` dead)
- leancodepl/marionette_mcp (514; mcp_dart; arrow-closure tool bodies)
- simonpcouch/side (47; ellmer; 9 of 9 tool impls dead)
- mattt/iMCP (2,002; MCP Swift SDK; 0 handlers dead, no entries)

## Process

- Re-score the parity matrix (`dev_docs/matrix/` sections 02, 05, 06, 07,
  11, 12 and 14) against the post-0.11.2 code.
- Tidy the stale rule `gap` prose for go-testing, eunit, common-test,
  xctest, fsharp-nunit and cpp-test-macros (suppressed by
  `has_matchers`, so never printed).
