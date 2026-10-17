# 10. Frameworks & entry points

This category covers how cgg finds the places where a framework, runtime or harness calls into user code and no call site for that exists in the tree. Examples are HTTP routes, queue consumers, cron jobs, CLI commands, test cases, lifecycle overrides, DI providers, exported FFI symbols and markup-bound UI handlers. The whole mechanism is `cgg_resolve::frameworks::detect` (`crates/cgg-resolve/src/frameworks/mod.rs:58`). It runs over the 475 built-in `RuleSpec`s in `crates/cgg-core/src/frameworks/rules.rs:90-11626`, plus user rules. Each `RuleSpec` has exactly **one** `language` (`rules.rs:28-31`), and a rule only ever matches callables of that language (`mod.rs:82-111`, `match_attributes` `mod.rs:597`). A rule first has to be **detected** (import prefix, file path, call name, or a marker on the definition). Then it **enumerates** entries through one or more matcher shapes:

- A: attribute/decorator;
- B: registrar call with a named handler;
- C: registrar call with an inline closure;
- E: string target;
- F: worker self-module;
- base-type chain;
- method name;
- the helper tables (member sources, signature methods, container attributes, markup kinds, visibility, observer fields).

A rule with `node: true` mints a `<framework-entry>::<kind>::<id>::…` node with `entry` edges. A rule with `node: false` only root-marks for the dead-code pass. Test-kind rules mint one harness node per framework (`<framework-entry>::test::<id>`). A detected rule with no matchers emits no entries; it only adds a "seen, no rules" line, carrying its `gap` text, to the coverage table (`mod.rs:388-416`).

Sources: section 10 of all 46 `dev_docs/languages/<lang>.md` files, their scorecard row 10, and `dev_docs/frameworks/rules-part-{1..4}.md`. I checked these against the code. I re-derived all rule counts from `rules.rs` with a parser script (`<scratch>/parity/matrix-10/rules.py`, modes `summary`, `kinds`, `tests`, `bykind`, `methonly`, `diff`), and I ran the fixtures listed under "Fixtures run for this matrix" below.

## Legend

| mark | meaning |
| --- | --- |
| ✅ | full: the capability exists for the language, and the main frameworks of that domain use it (evidence in the language doc or below) |
| 🟡 | partial: some support (some rules enumerate, or root-mark only, or an important form is missed) |
| ⬜ | none: the language has the concept and cgg has no working support (no rule, or only a detect-only gap rule) |
| — | n/a: the concept does not exist in the language, or is not meaningful for it |

"Rules" is `enumerating / total` rules with that `language`. "Enumerating" means `has_matchers()` (`crates/cgg-core/src/frameworks.rs:301-321`): attributes, registrars, base types, methods, observer types, or a helper-table entry. Helper tables count, so `xunit-data`, `xaml`, `razor`, `godot-signal`, `java-serialization`, `async-graphql`, `juniper` and `solidity-public` count as enumerating.

Mechanism columns (how an entry is found):

| column | meaning | where in code |
| --- | --- | --- |
| Imp | detection by import prefix (`detect`) | `mod.rs:200-282` (`prefix_index`, `prefix_keys`) |
| Path/Call | detection by file path (`detect_paths`) or by call name (`detect_calls`) | `mod.rs:284-290`; `detect_calls_for` `rules.rs:11867` |
| Attr | shape A: attribute, decorator or annotation on the definition | `match_attributes` `mod.rs:590-629`; key/verb split `deadcode/roots.rs:297-350` |
| Reg | shape B: registrar call naming a handler (built-in registrar rules plus plugin argument capture) | `match_registrars` `mod.rs:1394`; `registrar::capture` `cgg-lang/src/plugins/registrar.rs:424-436` |
| Inline | shape C: closure passed to a registrar, synthesized as `handler_at_<line>` | `registrar::inline_closures` `registrar.rs:258`; plugins that mint it: go.rs:245, javascript.rs:1296 (TS shares `JsWalker`, typescript.rs:12), ruby.rs:436, perl.rs:662, elixir.rs:658, clojure.rs:437 |
| Str | shape E: a string or symbol argument names the handler (`string_targets`) | `decode_string_target` `mod.rs:1907` |
| Base | base-type/interface contract followed up the inheritance chain | `match_base_types` `mod.rs:869-925`, `find_in_base_chain` |
| Name | method-name convention with no base type (implicit interfaces, behaviours, hooks) | `match_methods` `mod.rs:996-1043` |
| MemSrc | attribute arguments name the member to root (`member_sources_for`) | `rules.rs:11719-11768`, `match_member_sources` `mod.rs:1192` |
| Sig | exact name + parameter list (`signature_methods_for`) | `rules.rs:11835-11865`, `mod.rs:1309` |
| Cont | an attribute on a container roots every member (`container_attributes_for`) | `rules.rs:11770-11797`, `mod.rs:633-695` |
| Markup | handlers bound in sibling markup (`markup_kinds_for`) | `rules.rs:11812-11819`, `mod.rs:699` |
| Vis | a visibility keyword is the entry (`visibility_entries_for`) | `rules.rs:11686-11691`, `match_visibility` `mod.rs:1655` |
| Obs | observer fields: a `FooProperty` field roots `on_foo` (`observer_types`) | `match_observer_callbacks` `mod.rs:1694` |
| SelfMod | shape F: the file identifies itself as a worker module (`self_module_markers_for`) | `rules.rs:11666-11673`, `mod.rs:1775` |

Domain columns (which kinds of framework are covered):

| column | meaning |
| --- | --- |
| Web | HTTP/RPC server routes and handlers (`TrustKind::Network`), excluding the cloud-function rules |
| Cloud | serverless function runtimes (aws-lambda*, azure-functions, gcp-functions, firebase-functions, cloudflare-workers, aws-cdk, deno-http, lambda-runtime, shuttle) |
| Queue | message consumers, actors, job workers (`TrustKind::Queue`) |
| Sched | cron/scheduled jobs (`TrustKind::Schedule`) |
| CLI | command-line dispatch (`TrustKind::Cli`, or a CLI command matcher inside a network rule) |
| Test | test/benchmark harness that enumerates its cases (`TrustKind::Test` with matchers) |
| Life/DI | lifecycle overrides, DI providers, ORM hooks, UI-toolkit callbacks, runtime callbacks (`TrustKind::Lifecycle`/`Ui`) |
| FFI | exported-symbol entry rule (`TrustKind::Ffi`) |

## The matrix

| lang | Rules | Imp | Path/Call | Attr | Reg | Inline | Str | Base | Name | MemSrc | Sig | Cont | Markup | Vis | Obs | SelfMod | Web | Cloud | Queue | Sched | CLI | Test | Life/DI | FFI | notes (most important gap) |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| rust | 19/30 | ✅ | ⬜ | ✅ | ✅ | ⬜ | — | 🟡 | ⬜ | — | — | ✅ | — | — | — | — | ✅ | ✅ | 🟡 | ⬜ | ⬜ | ✅ | ✅ | ✅ | A route closure (`get(\|\| async {…})`) mints no entry (fixture `rs`); warp/poem/tide/tonic/clap/tokio-cron/criterion are gap-only |
| python | 34/49 | 🟡 | ✅ | ✅ | ✅ | ⬜ | ✅ | ✅ | ✅ | — | — | ⬜ | 🟡 | — | ✅ | — | ✅ | ✅ | ✅ | 🟡 | ✅ | ✅ | ✅ | — | tornado/aiohttp/sanic/starlette/bottle/pyramid and airflow/prefect/dagster are gap-only; `import a, b` defeats detection |
| kivy | 0/0 | — | — | — | — | — | — | — | — | — | — | — | 🟡 | — | — | — | — | — | — | — | — | — | 🟡 | — | No kivy-language rule; KV callables are dead-code roots (`kivy:kv-binding`, roots.rs:733), never entry nodes; served by the python `kivy` rule |
| javascript | 16/37 | ✅ | ⬜ | ⬜ | ✅ | ✅ | ⬜ | ⬜ | 🟡 | — | — | — | ⬜ | — | — | ✅ | 🟡 | ✅ | ✅ | ⬜ | ⬜ | ⬜ | 🟡 | — | jest/vitest/playwright are gap-only, so no JS test case is an entry; next/koa/fastify/socket-io are gaps |
| typescript | 22/44 | ✅ | ⬜ | ✅ | ✅ | ✅ | 🟡 | 🟡 | 🟡 | — | — | ⬜ | ⬜ | — | — | ✅ | 🟡 | ✅ | ✅ | 🟡 | ⬜ | ⬜ | 🟡 | — | Same test-harness hole as JS; nextjs/fastify/koa/trpc/vue gaps; no `polka` twin |
| go | 15/29 | ✅ | ⬜ | — | ✅ | ✅ | — | — | ✅ | — | — | — | — | — | — | — | ✅ | ✅ | ✅ | ✅ | ⬜ | ✅ | 🟡 | ⬜ | cobra/urfave-cli/grpc-go are gap-only; implicit `String`/`Error` interfaces are not rooted |
| java | 47/60 | ✅ | ⬜ | ✅ | ⬜ | ⬜ | ⬜ | ✅ | 🟡 | ✅ | ✅ | ⬜ | ⬜ | — | — | — | ✅ | ✅ | ✅ | ✅ | ⬜ | ✅ | ✅ | — | No registrar rule (Javalin/Spark lambdas), although java.rs:372 captures arguments; vertx/dropwizard/struts/grpc gaps |
| kotlin | 19/30 | ✅ | ⬜ | ✅ | ⬜ | ⬜ | — | 🟡 | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | — | — | — | 🟡 | ✅ | 🟡 | ⬜ | ⬜ | ✅ | 🟡 | — | 23 enumerating Java rule ids have no Kotlin row; Android Activity/Fragment/Service/Application are gap-only although bases are recorded |
| c | 2/3 | ✅ | ✅ | ⬜ | 🟡 | — | — | — | ⬜ | — | — | — | — | — | — | — | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | 🟡 | ⬜ | Only `c-callbacks` root marks work (the C `cuda` rule cannot fire); constructor attrs, sigaction, `module_init` unseen |
| cpp | 3/6 | ✅ | ✅ | 🟡 | 🟡 | ⬜ | — | ⬜ | ⬜ | — | — | — | ⬜ | — | — | — | ⬜ | 🟡 | ⬜ | ⬜ | ⬜ | ⬜ | 🟡 | ⬜ | gtest/doctest/Catch2 are gap-only; no Qt, Unreal, Crow/Drogon or libFuzzer rules |
| csharp | 32/41 | ✅ | ⬜ | ✅ | 🟡 | ⬜ | — | ✅ | 🟡 | ✅ | ⬜ | ⬜ | ✅ | — | — | — | ✅ | ✅ | 🟡 | ✅ | ⬜ | ✅ | ✅ | ✅ | `aspnet-mvc` enters private controller methods; `MapPost(…, () => …)` lambdas mint no entry (fixture `csl`) |
| bash | 0/1 | 🟡 | — | — | ⬜ | — | — | — | ⬜ | — | — | — | — | — | — | — | — | — | — | — | ⬜ | ⬜ | ⬜ | — | `bash-dispatch` is detect-only (literal `source` paths); `case "$1"` dispatch, `trap` and `main "$@"` unseen |
| ruby | 9/17 | ✅ | ✅ | — | ✅ | ✅ | ✅ | ✅ | 🟡 | — | — | — | — | — | — | — | 🟡 | ✅ | ✅ | ⬜ | ⬜ | 🟡 | 🟡 | — | `resources :users` is not expanded to the seven actions; rspec is gap-only; detection is tree-wide, so frameworks cross-claim routes |
| swift | 5/9 | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | — | 🟡 | ✅ | — | — | ⬜ | ⬜ | — | — | — | ⬜ | 🟡 | ⬜ | ⬜ | ⬜ | 🟡 | 🟡 | ⬜ | The plugin records no attributes (`@Test`, `@main`, `@IBAction` invisible) and captures no registrar args (aws-lambda `run` is dead); vapor gap |
| lua | 0/2 | ✅ | ⬜ | — | ⬜ | ⬜ | — | — | ⬜ | — | — | — | — | — | — | — | ⬜ | — | — | — | — | ⬜ | ⬜ | — | kong/openresty are detect-only; no LÖVE, Neovim or busted rules |
| php | 9/17 | ✅ | ✅ | 🟡 | ✅ | ⬜ | ✅ | ✅ | ⬜ | ⬜ | — | ⬜ | — | — | — | — | 🟡 | ✅ | 🟡 | ⬜ | 🟡 | ✅ | 🟡 | — | Misses `\`-qualified, comma-grouped, class-level and docblock attributes; slim/cakephp/yii/drupal/magento are gaps |
| dart | 0/5 | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | — | ⬜ | ⬜ | — | — | — | — | — | — | — | ⬜ | ⬜ | — | — | ⬜ | ⬜ | ⬜ | ⬜ | Zero enumerating rules: flutter/flame/shelf/flutter-test are all detect-only, and the plugin has no attributes or registrar capture |
| scala | 3/12 | ✅ | ⬜ | 🟡 | ⬜ | ⬜ | — | 🟡 | ⬜ | — | ⬜ | ⬜ | — | — | — | — | ⬜ | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | — | Only the 3 cloud rules bind; play/akka-http/http4s/akka/scalatest/specs2 are gaps; no Spring or JUnit row for Scala |
| hcl | 0/0 | ⬜ | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | ⬜ | — | No rule at all (`no_markers`); Terraform root-module conventions not modelled |
| zig | 0/4 | ✅ | — | — | — | — | — | — | ⬜ | — | — | — | — | — | — | — | — | — | — | — | ⬜ | ⬜ | ⬜ | ⬜ | All 4 rules are detect-only; `test` blocks are not callables; `export fn` is not recorded |
| objc | 4/6 | 🟡 | ⬜ | ⬜ | ⬜ | ⬜ | — | ⬜ | ✅ | — | — | — | ⬜ | — | — | — | — | — | — | — | ⬜ | ✅ | 🟡 | — | Name-only matching (no base types); imports seen only in `.h`/`.pch` do not activate rules |
| r | 1/5 | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | — | — | ✅ | — | — | — | — | — | — | — | ⬜ | — | — | — | — | ⬜ | 🟡 | — | Only `r-package-hooks` enumerates; shiny/plumber/testthat/S3 dispatch are detect-only |
| groovy | 5/9 | ✅ | ⬜ | ✅ | ⬜ | ⬜ | — | 🟡 | ⬜ | ⬜ | ⬜ | ⬜ | — | — | — | — | ⬜ | ✅ | — | — | ⬜ | ⬜ | 🟡 | — | No JUnit/Spring/Spock enumeration for Groovy; grails/jenkins/gradle-task are detect-only |
| julia | 2/5 | 🟡 | ⬜ | ⬜ | ⬜ | ⬜ | — | — | ✅ | — | — | — | — | — | — | — | ⬜ | — | — | — | — | ⬜ | 🟡 | — | genie/julia-test/flux are detect-only; `import Base: show` is not an import record |
| perl | 2/5 | ✅ | ⬜ | ⬜ | ✅ | ✅ | ⬜ | ⬜ | ⬜ | — | — | — | — | — | — | — | 🟡 | — | ⬜ | — | ⬜ | ⬜ | ⬜ | — | No string targets (`->to('users#logout')`); every route is claimed by both mojolicious and dancer2; catalyst/plack are gaps |
| elixir | 9/9 | ✅ | ⬜ | ⬜ | ✅ | ✅ | ✅ | — | ✅ | — | — | — | — | — | — | — | 🟡 | ⬜ | ⬜ | ⬜ | ✅ | ⬜ | ✅ | — | No ExUnit rule at all (not even detection), unlike Erlang's eunit/common-test; `resources`/`live`/`forward` not expanded; no Oban |
| erlang | 3/6 | ✅ | ⬜ | — | ⬜ | ⬜ | — | — | ✅ | — | — | — | — | — | — | — | ⬜ | — | 🟡 | — | ⬜ | ✅ | 🟡 | — | cowboy dispatch tables are not decoded; otp-lifecycle/application are detect-only |
| fortran | 0/1 | ✅ | — | — | — | — | — | — | ⬜ | — | — | — | — | — | — | — | — | — | — | — | ⬜ | ⬜ | — | ⬜ | `fortran-program` is detect-only; PROGRAM units and `bind(c)` procedures are not entries |
| clojure | 1/6 | ✅ | ⬜ | ⬜ | ✅ | ✅ | ⬜ | — | ⬜ | — | — | — | — | — | — | — | 🟡 | ⬜ | ⬜ | — | ⬜ | ⬜ | ⬜ | — | Only compojure enumerates; ring/reitit/pedestal/http-kit/clojure-test are gaps |
| haskell | 2/7 | ✅ | ⬜ | — | ⬜ | ⬜ | — | — | 🟡 | — | — | — | — | — | — | — | ⬜ | ⬜ | — | — | ⬜ | 🟡 | — | ⬜ | servant/yesod/scotty/wai are detect-only; the plugin captures no registrar args |
| ocaml | 0/4 | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | — | — | ⬜ | — | — | — | — | — | — | — | ⬜ | — | — | — | ⬜ | ⬜ | — | ⬜ | All 4 rules (dream/opium/alcotest/cmdliner) are detect-only; `ExtractCtx` is ignored (ocaml.rs:27) |
| powershell | 1/4 | ✅ | ✅ | ⬜ | ⬜ | ⬜ | — | — | ✅ | — | — | — | — | — | — | — | ⬜ | ⬜ | — | — | ⬜ | ⬜ | 🟡 | — | Only `powershell-dsc` enumerates; Pester, `run.ps1` Azure handlers and module exports are gaps |
| solidity | 1/1 | — | — | — | — | — | — | ⬜ | ⬜ | — | — | — | — | ✅ | — | — | — | — | — | — | — | ⬜ | ⬜ | — | `public`/`external` are entries; OpenZeppelin hook conventions, `fallback`/`receive` and Foundry tests are not modelled |
| fsharp | 2/7 | ✅ | ⬜ | ✅ | ⬜ | ⬜ | — | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | — | — | — | ⬜ | 🟡 | ⬜ | ⬜ | ⬜ | 🟡 | ⬜ | ⬜ | 27 enumerating C# rule ids have no F# row (aws-lambda, gcp, hosting, …); giraffe/saturn/suave/expecto are gaps |
| starlark | 0/1 | ✅ | — | — | ⬜ | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | ⬜ | — | `bazel` is detect-only; `rule(implementation=…)`, `aspect`, `repository_rule` are not registrars |
| cmake | 0/1 | ✅ | — | — | ⬜ | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | ⬜ | — | `cmake` is detect-only; the entry is the file, plus `cmake_language(DEFER CALL)` targets |
| nix | 0/1 | ✅ | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | ⬜ | — | `nix` is detect-only; its `"."` prefix fires on every relative import; `outputs`/`callPackage` are not modelled |
| verilog | 0/0 | ⬜ | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | ⬜ | ⬜ | — | No rule (it is the test exemplar for "no rules"); top-level modules and testbenches are not entries |
| vhdl | 0/1 | ✅ | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | ⬜ | ⬜ | — | `vhdl-toplevel` is detect-only (uninstantiated entity, testbench, processes) |
| asm | 0/0 | ⬜ | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | ⬜ | — | No rule; interrupt vectors are not modelled (only `_start` via the dead-code `LANG_ENTRY`) |
| smithy | 0/0 | ⬜ | — | — | — | — | — | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | — | — | — | — | No rule; `@http` operations are the API surface but never entries |
| proto | 0/0 | ⬜ | — | — | — | — | — | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | — | — | — | — | No rule; rpcs are not entries, and host gRPC rules are all gap-only |
| graphql | 0/0 | ⬜ | — | — | — | — | — | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | — | — | — | — | No rule; schema root fields are not entries (host-side async-graphql/juniper work, yoga/apollo are gaps) |
| openapi | 0/0 | ⬜ | — | — | — | — | — | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | — | — | — | — | No rule; `operationId` → handler rooting (Connexion) is not done |
| asyncapi | 0/0 | ⬜ | — | — | — | — | — | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | — | — | — | — | No rule; channels/operations are not entries and do not root host handlers |
| lean | 0/0 | ⬜ | — | ⬜ | ⬜ | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | ⬜ | ⬜ | ⬜ | — | No rule; `@[init]`/`initialize`, elaborator attributes, Lake `lean_exe` and `#eval` are not modelled |

### Evidence for each column, by language

What follows is the evidence behind each mark that is not plain from the language doc, plus the mechanism counts from `rules.py summary`. These count rules per language with each matcher field non-empty, comment lines stripped: attr / reg / base / meth / str-target / detect_paths / root-mark-only.

| lang | attr | reg | base | meth | str | paths | node:false | evidence notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| java | 27 | 0 | 22 | 23 | 0 | 0 | 31 | member sources `junit-sources` (rules.rs:11721); signature `java-serialization` (rules.rs:11837) |
| python | 21 | 15 | 8 | 13 | 3 | 1 | 17 | observer types only on `kivy` (14 `*Property`); string targets: rq, apscheduler, aws-cdk |
| typescript | 5 | 15 | 2 | 6 | 1 | 0 | 3 | string target only `aws-cdk` (rules.rs:4284); self-module `worker-threads` (rules.rs:11668) |
| csharp | 13 | 1 | 15 | 13 | 0 | 0 | 15 | member sources nunit-runner/xunit-data (rules.rs:11738-11761); markup xaml/razor/tscn (rules.rs:11815-11817) |
| javascript | 0 | 15 | 0 | 4 | 0 | 0 | 1 | no attribute rule; react is methods-only (14 names, no `render`) |
| kotlin | 12 | 0 | 3 | 8 | 0 | 0 | 14 | bases used only by aws-lambda, gcp-functions and spring-batch |
| rust | 14 | 5 | 2 | 2 | 0 | 0 | 4 | container attributes ffi-export/async-graphql/juniper/rmcp (rules.rs:11770-11797) |
| go | 0 | 13 | 0 | 4 | 0 | 0 | 3 | registrar verbs match case-insensitively (HTTP_VERBS comment, rules.rs:60-66) |
| ruby | 0 | 5 | 4 | 4 | 2 | 4 | 4 | string targets rails, rails-callbacks |
| php | 3 | 4 | 7 | 6 | 4 | 3 | 4 | `detect_calls` on wordpress (rules.rs:1110) |
| scala | 1 | 0 | 2 | 2 | 0 | 0 | 0 | — |
| elixir | 0 | 2 | 0 | 7 | 0 | 0 | 2 | atom/alias args become `Owner::action` string refs (elixir.rs:413-669) although `string_targets` is false |
| groovy | 2 | 0 | 3 | 3 | 0 | 0 | 2 | — |
| swift | 0 | 1 | 1 | 5 | 0 | 0 | 5 | the `run` registrar is never captured: swift.rs has no `registrar::` call (grep) |
| fsharp | 2 | 0 | 0 | 0 | 0 | 0 | 0 | — |
| haskell | 0 | 0 | 0 | 2 | 0 | 0 | 0 | — |
| cpp | 1 | 2 | 0 | 0 | 0 | 0 | 2 | `detect_calls` on cpp-callbacks (rules.rs:8019) |
| c | 1 | 1 | 0 | 0 | 0 | 0 | 2 | `detect_calls` on c-callbacks (rules.rs:7981) |
| objc | 0 | 0 | 0 | 4 | 0 | 0 | 5 | — |
| erlang | 0 | 0 | 0 | 3 | 0 | 0 | 2 | — |
| clojure | 0 | 1 | 0 | 0 | 0 | 0 | 0 | — |
| perl | 0 | 2 | 0 | 0 | 0 | 0 | 0 | — |
| r, julia, powershell | 0 | 0 | 0 | 1 / 2 / 1 | 0 | 0 / 0 / 1 | 0 / 1 / 1 | powershell `detect_paths: ["run.ps1"]` (rules.rs:4625) |
| dart, ocaml, zig, lua, bash, cmake, nix, starlark, fortran, vhdl | 0 | 0 | 0 | 0 | 0 | 0 | — | detect-only only |
| solidity | 0 | 0 | 0 | 0 | 0 | 0 | 0 | visibility `solidity-public` (rules.rs:11688) |

Registrar argument capture (Reg/Inline columns) depends on the plugin as well as the rule:

- Calls `registrar::capture` or gates on `ExtractCtx::is_registrar_verb`: clojure.rs:279, cpp.rs:287, c.rs:147, csharp.rs:279, elixir.rs:453, go.rs:203, java.rs:372, javascript.rs:1255 (TS too), perl.rs (`arguments_of`/`is_closure`, 460/662), php.rs:390, python.rs:292, ruby.rs:411.
- Rust has its own `refs_from_args` / `registrar::route_of` (rust.rs:2285-2300), with no verb gate.
- **No** registrar capture at all, by grep for `registrar::`: kotlin, scala, swift, groovy, dart, lua, julia, haskell, ocaml, fsharp, r, powershell, objc, erlang, bash, zig, fortran, and every config/HDL/IDL language. For these languages a user `--registrar` verb also does nothing.

Attribute recording (Attr column). These plugins write no source attributes: swift.rs:114/179, objc.rs:102/209, erlang.rs:213, julia.rs:215, lua.rs:227, haskell.rs:201, ocaml.rs:234, r.rs:115, powershell.rs:157, perl.rs:318, zig.rs:201, clojure.rs:240 and elixir.rs:344 all have `attributes: Vec::new()`. Dart records only the synthetic `abstract` (dart.rs:119). So no attribute rule can fire for them. For languages with attribute syntax this is marked ⬜: Swift `@Test`, Dart `@pragma`, PowerShell `[CmdletBinding()]`, Perl `:Path`, Elixir `@impl`, OCaml `[@@…]`, Clojure metadata, Julia `@get` macros, Lean `@[init]`. For languages without it, it is marked —.

## Parity gaps

Ranked by how much entry surface is lost compared with a comparable language that has the support, and how often the language appears in real trees. Effort: S = add or copy `RuleSpec`/helper rows, M = plugin extraction work plus rules, L = a new input format or a new matcher shape.

1. **Kotlin lacks 23 enumerating Java rules, and its Android lifecycle rules are gap-only.**
   - `rules.py diff java kotlin` lists the missing ids: akka, caliper, gradle-model-rules, gradle-services, hibernate-listeners, jackson, jakarta-rs, java-closeable, java-concurrent, javafx, jpa-mapping, jsr250-lifecycle, jsr330-inject, junit-sources, micronaut, moshi, servlet, spring-context, spring-jobs, spring-messaging, storm, swing, testng.
   - So `@Scheduled`, `@KafkaListener`, `@Bean`, `@EventListener`, `@PostConstruct`, JAX-RS and Micronaut controllers enter nothing in Kotlin.
   - `android-activity`, `android-fragment`, `android-service`, `android-contentprovider` and `android-application` (rules.rs:3822-3913) have no matchers. Their gap text says the plugin does not record supertypes, but kotlin.rs:174 (`kotlin_bases`) does. Fixture `kt`: `class MainAct : AppCompatActivity()` gives "seen, no rules android-activity" with that stale text, and `onCreate` gets no framework root.
   - The Kotlin `spring` rule also lacks the Java `MessageMapping`/`ExceptionHandler`/`InitBinder`/`ModelAttribute` attributes (kotlin.md §10).
   - Effort **S** (copy rows; the plugin already records attributes and bases).
2. **JavaScript/TypeScript have no working test harness.**
   - `vitest`, `jest` and `playwright` are gap-only in both languages (`rules.py tests`; javascript.md §10, typescript.md §10).
   - By contrast, Python (`py-unittest`, `pytest`), Go (`go-testing`), Rust (`cargo-test`), Java (`junit`, …) and C# (`xunit`, `mstest`, …) mint harness nodes.
   - The JS plugin already synthesizes inline closures for registrars (javascript.rs:1296). `it`/`test`/`describe` are kept out of the verb gate on purpose (registrar.rs:191-205 comment; registrar_verbs doc rules.rs:11897-11904).
   - Effort **M** (a test-harness shape that names the closure by its description string without enabling it for ordinary callbacks).
3. **Elixir has no ExUnit rule at all, while Erlang has `eunit` and `common-test`.**
   - `grep -c ExUnit rules.rs` = 0. `rules.py diff erlang elixir` gives common-test, eunit and erlang-gen-server; Elixir's `otp` stands in for the last one.
   - The Elixir plugin already synthesizes `handler_at_<line>` for `get "/x" do … end` (elixir.rs:658), so `test "name" do … end` is reachable with the same machinery.
   - Oban (`perform/1`) and Ecto have no rule either (elixir.md §10, fixture f2).
   - Effort **M**.
4. **F# lacks the C# .NET rules.**
   - `rules.py diff csharp fsharp`: aws-lambda, gcp-functions, aspnet-mvc, aspnet-minimal, dotnet-hosting, masstransit, quartz, mediatr, ef-core, signalr, orleans, dotnet-ffi-export, dotnet-serialization, mstest, benchmarkdotnet and others (27 ids). `fsharp-nunit` stands in for xunit/nunit-runner.
   - F# records attributes (fsharp.rs:168-173 per fsharp.md) and runs on the same SDKs. Only `azure-functions` is shared.
   - F# also has no `member_sources_for` row (`TestCaseSource` not rooted).
   - Effort **S** for attribute/base rows, **M** for Giraffe/Saturn routing (operator/CE position, which needs plugin work).
5. **Scala and Groovy lack the JVM rules Java has.**
   - Scala: 43 ids missing, Groovy: 42 (`rules.py diff java scala|groovy`), including `spring`, `junit`, `jakarta-rs`, `jsr250-lifecycle`, `spring-context` and `akka`.
   - Scala `akka-scala`/`akka-typed`/`play-mvc` are gap-only, although the plugin records bases and annotations (scala.md §10; stale gap prose at rules.rs:5512, 5571).
   - Groovy records annotations and bases (fixture f9 in groovy.md: `@TaskAction` and `Plugin<Project>` matched), but Spock/Grails are gap-only.
   - Effort **S** (rows) for Spring/JUnit/Akka-classic; **M** for Spock feature methods.
6. **Inline route closures are entries in JS/TS, Go, Ruby, Perl, Elixir and Clojure, but not in Rust, C#, PHP, Python, Java or Kotlin.**
   - Fixture `rs`: `.route("/b", get(|| async { helper() }))` mints only `axum::get('/a')`.
   - Fixture `csl`: `app.MapPost("/b", () => Helper())` mints only `mapget('/a')`.
   - PHP `Route::get('/closure', function () {…})` (php.md §10, fixture p2) and Python lambdas (python.rs has no `handler_at`) behave the same way.
   - Effort **M** per plugin (reuse `registrar::inline_closures`).
7. **Swift: no attributes and no registrar capture, unlike Kotlin/Java/C#.**
   - swift.rs:114/179 writes `attributes: Vec::new()`, so `swift-testing` (`@Test`/`@Suite`), `@main`, `@IBAction`/`@objc` and Vapor route closures are invisible.
   - `aws-lambda`'s `registrars: ["run"]` is dead because swift.rs never calls `registrar::` (swift.md §10, fixture f9: `legacy`/`work` dead).
   - ObjC is similar: name-only rules with no base types, and imports seen only in `.h` are ignored (objc.md §10, fixture f1).
   - Effort **M**.
8. **Web frameworks are detect-only in nine languages whose peers enumerate routes.**
   - Kotlin `ktor`, Swift `vapor`, Scala `play-mvc`/`akka-http`/`http4s`, Haskell `servant`/`scotty`/`yesod`, OCaml `dream`/`opium`, F# `giraffe`/`saturn`/`suave`, Lua `openresty`/`kong`, Julia `genie`, R `plumber`/`shiny`, Erlang `cowboy`, Dart `shelf`.
   - Compare JS (express, hono), Go (gin, chi, …), Ruby (sinatra) and Clojure (compojure).
   - The common blocker is gap 6 or the missing registrar capture listed under "Registrar argument capture" above.
   - The Yesod gap needs a `get*R` pattern. `name_pattern_matches` supports only one of prefix or suffix (mod.rs:570-584), so `get*R` compares as an exact string.
   - Effort **M-L**.
9. **Dart has zero enumerating rules.** flutter, flame, dart-main, flutter-test and shelf are all detect-only (rules.rs:5402-5498). The plugin records no annotations and captures no registrar arguments, so even `State.initState`/`build` overrides are unrooted. Kotlin's `compose` and Swift's `uikit-lifecycle` root the equivalents. Effort **M** (a base-type rule on `State`/`StatelessWidget`/`FlameGame` needs only bases, which is unverified for dart.rs; the closure-based tests need gap 2's mechanism).
10. **Member sources exist only for Java and C#.** `member_sources_for` has rows only for (`junit-sources`, java), (`nunit-runner`, csharp) and (`xunit-data`, csharp) (rules.rs:11719-11768). These are missing:
    - Kotlin JUnit 5 `@MethodSource`;
    - F# `TestCaseSource`;
    - PHPUnit `#[DataProvider('cases')]` (php.md §10, fixture p9: `AttrOnly::cases` dead).

    Effort **S**.
11. **Signature methods exist only for Java.** `signature_methods_for` has only (`java-serialization`, java) (rules.rs:11835-11865). Two near-equivalents are missing:
    - Kotlin/Scala/Groovy classes implementing `Serializable` use the same JVM hooks.
    - C# `(SerializationInfo, StreamingContext)` constructors are listed in the `csharp-runtime` gap.

    Effort **S**.
12. **Container attributes exist only for Rust.** `container_attributes_for` covers Rust `impl` blocks only (rules.rs:11770-11797), and only the Rust plugin fills `FileFacts::attributed_containers`. These container forms root nothing:
    - PHP class-level `#[AsMessageHandler]`/`#[Route]`: not recorded, because attributes are collected only on function `DefRecord`s (php.md §10, php.rs:241; fixture p8 `MH::__invoke` dead).
    - Python class decorators (`@strawberry.type`): unverified whether the plugin records them.
    - C#/Java/TypeScript class-level GraphQL or controller attributes.

    Effort **M**.
13. **Markup bindings exist only for C#.** `markup_kinds_for` is xaml/razor/tscn (rules.rs:11815-11817), and Kivy KV is linked separately by `ffi.rs:268` (`link_kivy_python`) as dead-code roots, not entries. These have nothing:
    - Android layout `android:onClick` (Java/Kotlin);
    - JavaFX `.fxml` `onAction="#h"`;
    - Apple storyboards/xibs (Swift/ObjC);
    - Angular templates and Vue/Svelte SFCs (not parsed; javascript.md §10).

    Effort **L** (new input formats).
14. **The JS/TS rule sets are asymmetric.**
    - `aws-cdk` (string target `handler: "module.fn"`, rules.rs:4284), `nestjs`, `nestjs-schedule`, `loopback` and the Angular rules exist only for TypeScript.
    - `polka` exists only for JavaScript (`rules.py diff`). CDK stacks written in JS are common.
    - JS also has no Schedule rule (no node-cron) and no CLI rule (no commander/yargs).
    - Effort **S**.
15. **CLI dispatch enumerates only in Python (click/typer, django-admin, argparse) and Elixir (mix-task), plus PHP symfony/laravel commands.**
    - Rust `clap`, Go `cobra`/`urfave-cli`, Ruby `rake`, Haskell `haskell-main`, OCaml `cmdliner`, F# `fsharp-entrypoint`, Zig `zig-main`/`zig-build`, Fortran, Bash and PowerShell modules are all gap-only (`rules.py bykind Cli`).
    - There is no Java picocli, no JS commander/yargs and no C# System.CommandLine rule (grep: none).
    - Effort **M** (most dispatch through struct fields or closures).
16. **Schedule coverage is uneven.**
    - Java has `spring-jobs`, C# `quartz`, Go `robfig-cron`, Python `apscheduler` and TS `nestjs-schedule`.
    - Kotlin has no `spring-jobs` row (gap 1). Rust `tokio-cron` is a gap. JS, Ruby (sidekiq-cron/whenever) and PHP (Laravel scheduler) have no rule.
    - Effort **S** for Kotlin, **M** elsewhere.
17. **Go implicit interfaces: only `ServeHTTP` (net-http), `ProcessTask` (asynq) and the encoding family are rooted.** `String`, `Error`, `Len`/`Less`/`Swap`, `Scan`/`Value` are not (go.md §10, fixture f5 `T.String` dead). Compare Rust `rust-std-traits` (Display, Iterator, …) and C# `csharp-disposable`. Effort **S** (the `go-runtime` gap says names are too common; a base-free method rule would need the receiver-type check Rust gets from trait impls).
18. **String targets are missing where the framework uses them.**
    - Ruby (rails `'c#a'`), PHP (`'C@m'`, `[C::class,'m']`) and Python (rq, apscheduler) decode string targets.
    - Perl Mojolicious `->to('users#logout')` / `->to(controller => …)` does not (perl.md §10, fixture f5), and neither does Elixir `resources "/u", UserController` (no action atoms written) or Rails `resources :users`.
    - Effort **S** for Perl (set `string_targets` and teach `decode_string_target` the `#` form for perl), **M** for resource expansion.
19. **Lifecycle rules for C, C++ and the systems languages.**
    - Root marks exist only for `c-callbacks`/`cpp-callbacks` and `cuda`.
    - These have no rule: `__attribute__((constructor))` (fixture f9 in cpp.md: `init_ctor` dead), Qt `connect`/slots (cpp.md fixture f8: `onClick` lost), Linux `module_init`/`EXPORT_SYMBOL`, libFuzzer `LLVMFuzzerTestOneInput`, Arduino `setup`/`loop`, Google Test `TEST_F` bodies.
    - The C `cuda` rule (rules.rs:1399) can never fire, because the C plugin records no attributes (c.md §10). The C++ one works through `cuda_qualifiers` (cpp.rs:2713).
    - Effort **M**.
20. **FFI export rules exist for Rust and C# only.**
    - Zig `export fn`/`callconv(.c)` is gap-only (`zig-export`, rules.rs:6612-6628; the Zig plugin writes no visibility).
    - These have no rule at all: C/C++ `__declspec(dllexport)`, Go cgo `//export`, Swift `@_cdecl`, Fortran `bind(c)`, Haskell `foreign export`, F# `UnmanagedCallersOnly` (which C# has in `dotnet-ffi-export`).
    - JNI/PyO3/napi edges are linked by `ffi::link_ffi`, not by these rules.
    - Effort **S-M**.
21. **The interface languages root nothing.**
    - OpenAPI `operationId`, AsyncAPI operations, proto rpcs, GraphQL root fields and Smithy `@http` operations are never entries.
    - No host rule reads the spec to root its handler. Examples: Connexion (rules.rs:8151-8165 gap), grpc-go/grpc-java/grpc-dotnet/tonic (all gap-only), graphql-yoga/apollo (gap-only).
    - Effort **L** (a cross-language descriptor→handler rule shape).
22. **Detection is tree-wide per (rule, language), so frameworks bleed into each other.**
    - `Detection::is_active` is keyed only by `(id, language)` (mod.rs:362-365), and every matcher except Test-kind method rules (which also require a test file or a harness import, mod.rs:1010-1021) runs over every callable of the language.
    - This causes the documented duplicate entries: Ruby sinatra+grape (ruby.md fixture `fw`), Perl mojolicious+dancer2 (perl.md f5), Elixir phoenix+plug-router (elixir.md f2), and PowerShell DSC in an unimported file (powershell.md f7).
    - The same per-tree activation is what lets Java's base-chain matching root a class that imports nothing (java.md f9). Per-file gating would break that, so the fix needs care.
    - Affects every language with ≥2 registrar rules. Effort **M**.
23. **Base types go unused where they are recorded.**
    - ObjC rules have `base_types: NONE` (objc.md §10), so any `tableView`/`application` method in a UIKit-importing file is rooted, whatever its class.
    - Swift `uikit-lifecycle` is the same (fixture f9 `NotAVC.viewDidLoad` rooted).
    - Perl records `@ISA`/`parent` (perl.md §10) but no rule uses it (Catalyst controllers).
    - Effort **S** where bases are recorded.

## Doc corrections

Corrections to `dev_docs/languages/*.md`. The language docs themselves are not edited here.

1. **kotlin.md scorecard row 10** says "12 gap-only". `rules.rs` has **11** Kotlin rules without matchers: ktor, android-activity, android-fragment, android-service, android-contentprovider, android-application, kotlinx-coroutines, kotlin-runtime, spring-integration, android-compose, ktor-client. Evidence: `rules.py kotlin` output, and 19 enumerating + 11 = 30.
2. **kivy.md §10** says the `kivy` rule has "50 lifecycle and observer names". **python.md §10** says "bases App/Widget/… (34)" and "on_* (45)". The rule (rules.rs:290-414) has **33** `base_types`, **44** `methods`, **14** `observer_types` and 4 registrars, counted from the quoted literals in the `id: "kivy"` block. kivy.md's "33 widget/app bases" is correct; python.md's 34 and both method counts are wrong.
3. **php.md §10** says the fully-qualified-attribute miss is "unverified in code, but observed". It is verified in code:
   - `attribute_verb` splits the key only on `.` and `:` (`crates/cgg-resolve/src/deadcode/roots.rs:347-350`), and `attribute_key` strips only `#[`/`[`/`@`/`]` and the argument list (roots.rs:297-306). So `\Symfony\…\Route` compares as the whole backslash path.
   - Fixture `php` (this matrix): `#[Route('/a')]` and `#[\Symfony\Component\Routing\Attribute\Route('/fq')]` → `recognised symfony (network, 1 entry)`, only `Route('/a')`.
4. **ruby.md §10** says cross-claiming is "per project" with the mechanism inferred. **elixir.md §10** says "the exact detection scoping is unverified". Both are now verified:
   - Activation is `Detection::is_active(id, language)` over `hits` keyed by `(id, language)` (mod.rs:356-365), checked in `detect` (mod.rs:82, 106).
   - `match_registrars`, `match_attributes` and `match_base_types` do not consult `file_carries`. Only `match_methods` does, and only for `TrustKind::Test` (mod.rs:1010-1021).
   - Activation is therefore tree-wide per language. The Perl and PowerShell docs already describe it that way.
5. **csharp.md §10** states, from fixture f2, that `aspnet-mvc` enters private controller methods. The cause is confirmed in code: `match_base_types` applies no visibility filter, and when `rule.methods` is empty every member of a matching class is `named` (mod.rs:895-896). This affects every base-type rule without `methods` (aspnet-mvc, codeigniter, actioncable, signalr, orleans, …), not only aspnet-mvc.
6. **typescript.md §10 "Parity gaps against JavaScript"** lists `express-gateway`, `meteor`, `polka` and `sails` as having no TypeScript twin. Of these, only `polka` enumerates in JavaScript. The other three are gap-only in JS too (javascript.md table), so the only enumerating loss is `polka` (`rules.py diff javascript typescript` → `['polka']`).
7. **c.md / cpp.md** are consistent. One clarification: cpp.md lists `cpp-callbacks` with "ten" verbs and c.md lists `c-callbacks` with nine. That is correct (`call_once` is C++-only, rules.rs:8019-8045), and C has no C-side equivalent of `run_handler` (`rules.py diff cpp c` → aws-lambda, cpp-callbacks).
8. **kotlin.md §10 "Missing compared with Java"** names only `junit-sources` and `java-serialization` among Java rules. The full enumerating-rule difference is 23 ids (gap 1 above), including `spring-jobs`, `spring-messaging`, `spring-context`, `jsr250-lifecycle`, `jakarta-rs`, `micronaut`, `servlet` and `testng`, which the doc does not mention.
9. **rust.md §10** says registrar verbs do not use the `ExtractCtx` gate. That is accurate: rust.rs has only `registrar::route_of` (rust.rs:2300), no `capture` and no `is_registrar_verb`. Also note, as the doc does not say, that a closure handler in `get(|| …)` mints no entry (fixture `rs`).
10. **graphql.md / openapi.md / asyncapi.md / lean.md** say rules exist for "36 languages". This is correct: `languages_with_rules()` (rules.rs:11887-11893) over SPECS gives 36 distinct languages. The 10 without any rule are kivy, hcl, verilog, asm, smithy, proto, graphql, openapi, asyncapi and lean.
11. **swift.md §10** marks the `xctest` gap text as stale and **objc.md** marks its own `xctest` gap text as stale. Both are correct. The strings at rules.rs:5141 and rules.rs:5381 still say "no prefix matcher", but the rules carry `test*` and the `has_matchers()` path suppresses the gap (frameworks.rs:301-321, mod.rs:395-404).

## Fixtures run for this matrix

All of them are under `<scratch>/parity/matrix-10/` and were run with `target/release/cgg`.

| fixture | input | result |
| --- | --- | --- |
| `kt/Main.kt` | `import androidx.appcompat.app.AppCompatActivity`; `class MainAct : AppCompatActivity() { override fun onCreate… }` | `seen, no rules android-activity` with the stale "does not record class supertypes" gap; no framework entry or root for `onCreate` |
| `php/c.php` | `#[Route('/a')]` and `#[\Symfony\Component\Routing\Attribute\Route('/fq')]` | `recognised symfony (network, 1 entry)`: only `Route('/a')` |
| `rs/src/main.rs` | axum `.route("/a", get(named)).route("/b", get(\|\| async { helper() }))` | one entry `axum::get('/a')`; the closure route has none |
| `csl/P.cs` | `app.MapGet("/a", Named); app.MapPost("/b", () => Helper());` | one entry `aspnet-minimal::mapget('/a')`; the lambda route has none |
| `rules.py` | parser over `rules.rs` SPECS | totals: 475 rules, 36 languages; per-language enumerating/total in the Rules column |
