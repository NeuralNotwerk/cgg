#!/usr/bin/env bash
# scripts/benchmark.sh — Clone/update test repos and calculate support stats.
#
# Usage: ./scripts/benchmark.sh [--update] [--lang LANG]
#
# When to run (manual only — never invoked by the pre-commit hook):
#   - After adding a new language plugin (also add a REPOS entry below).
#   - To refresh the README benchmark table's per-language numbers.
#   - To validate a resolver change against real-world projects.
#
# Clones each repo into $CGG_BENCH_DIR (default /storage/cgg-test_repos). First run
# is multi-minute and network-bound; subsequent runs reuse the clones.
# Does NOT patch README.md — follow up with
# `scripts/update-readme-stats.sh` to regenerate the README table.
set -uo pipefail

REPOS_DIR="${CGG_BENCH_DIR:-/storage/cgg-test_repos}"

# --- Time budget -------------------------------------------------------
#
# A corpus run must never be able to hang. Most of these repos finish in
# a few seconds; one pathological input should cost a bounded amount and
# be *reported*, not silently eat an afternoon. Learned the hard way:
# `erlang-otp` ran for 3h40m inside an unguarded sweep before anyone
# noticed, and the released binary hangs on it too.
#
#   CGG_REPO_TIMEOUT  per-repo wall cap   (default 60s)
#   CGG_TOTAL_BUDGET  whole-run wall cap  (default 1800s = 30 min)
#
# A repo that trips the per-repo cap is recorded as `TIMEOUT` and the run
# continues. When the total budget is gone the run stops and says how far
# it got — a partial corpus that announces itself beats a complete one
# that never arrives.
CGG_REPO_TIMEOUT="${CGG_REPO_TIMEOUT:-60}"
CGG_TOTAL_BUDGET="${CGG_TOTAL_BUDGET:-1800}"
BENCH_STARTED=$(date +%s)
TIMED_OUT_REPOS=()

# Wall seconds consumed so far.
budget_spent() { echo $(( $(date +%s) - BENCH_STARTED )); }

# True while there is budget left to start more work.
budget_left() {
    local spent; spent=$(budget_spent)
    [ "$spent" -lt "$CGG_TOTAL_BUDGET" ]
}

# Run cgg under the per-repo cap. Returns 124 on timeout, like timeout(1).
run_cgg() { timeout "$CGG_REPO_TIMEOUT" "$@"; }
CGG="${CGG_BIN:-$(dirname "$0")/../target/release/cgg}"

# Ensure cgg is built
if [ ! -f "$CGG" ]; then
    echo "Building cgg..."
    cargo build --release -p cgg --manifest-path "$(dirname "$0")/../Cargo.toml"
fi

# Repository definitions: name|url|lang|src_subdir|ctags_lang|ctags_kinds
REPOS=(
    "rust-ripgrep|https://github.com/BurntSushi/ripgrep.git|rust|crates|Rust|fPM"
    "python-flask|https://github.com/pallets/flask.git|python|src|Python|fm"
    "js-express|https://github.com/expressjs/express.git|javascript|lib|JavaScript|fmG"
    "ts-zod|https://github.com/colinhacks/zod.git|typescript|src|TypeScript|fmG"
    "go-fzf|https://github.com/junegunn/fzf.git|go|src|Go|f"
    "java-gson|https://github.com/google/gson.git|java|gson/src/main|Java|m"
    "kotlin-okio|https://github.com/square/okio.git|kotlin|okio/src|Kotlin|m"
    "c-jq|https://github.com/jqlang/jq.git|c|src|C|fp"
    "cpp-nlohmann-json|https://github.com/nlohmann/json.git|cpp|include|C++|f"
    "csharp-serilog|https://github.com/serilog/serilog.git|csharp|src|C#|m"
    "bash-acme|https://github.com/acmesh-official/acme.sh.git|bash||Sh|f"
    "ruby-jekyll|https://github.com/jekyll/jekyll.git|ruby|lib|Ruby|fm"
    "php-laravel|https://github.com/laravel/framework.git|php|src|PHP|f"
    "objc-afnetworking|https://github.com/AFNetworking/AFNetworking.git|objc|AFNetworking|ObjectiveC|m"
    "r-ggplot2|https://github.com/tidyverse/ggplot2.git|r|R|R|f"
    "swift-alamofire|https://github.com/Alamofire/Alamofire.git|swift|Source|Swift|f"
    "lua-kong|https://github.com/Kong/kong.git|lua|kong|Lua|f"
    "dart-flame|https://github.com/flame-engine/flame.git|dart|packages/flame/lib|Dart|f"
    "scala-play|https://github.com/playframework/playframework.git|scala|core/play/src/main|Scala|fm"
    "hcl-vpc|https://github.com/terraform-aws-modules/terraform-aws-vpc.git|hcl||HCL|f"
    "zig-http|https://github.com/karlseguin/http.zig.git|zig|src|Zig|f"
    "groovy-gradle|https://github.com/gradle/gradle.git|groovy|subprojects/core/src|Groovy|f"
    "julia-flux|https://github.com/FluxML/Flux.jl.git|julia|src||f"
    "perl-mojolicious|https://github.com/mojolicious/mojo.git|perl|lib|Perl|f"
    "elixir-phoenix|https://github.com/phoenixframework/phoenix.git|elixir|lib|Elixir|f"
    "erlang-otp|https://github.com/erlang/otp.git|erlang|lib/stdlib/src|Erlang|f"
    "fortran-stdlib|https://github.com/fortran-lang/stdlib.git|fortran|src|Fortran|f"
    "clojure-ring|https://github.com/ring-clojure/ring.git|clojure|ring-core/src||f"
    "haskell-pandoc|https://github.com/jgm/pandoc.git|haskell|src|Haskell|f"
    "ocaml-dune|https://github.com/ocaml/dune.git|ocaml|src|OCaml|f"
    "powershell-psget|https://github.com/PowerShell/PowerShellGet.git|powershell|||"
    "solidity-openzeppelin|https://github.com/OpenZeppelin/openzeppelin-contracts.git|solidity|contracts||"
    "fsharp-paket|https://github.com/fsprojects/Paket.git|fsharp|src||"
    "lean-batteries|https://github.com/leanprover-community/batteries.git|lean|Batteries|Lean|f"
    "starlark-skylib|https://github.com/bazelbuild/bazel-skylib.git|starlark|lib||"
    "cmake-kitware|https://github.com/Kitware/CMake.git|cmake|Modules||"
    "nix-home-manager|https://github.com/nix-community/home-manager.git|nix|modules||"
    "verilog-picorv32|https://github.com/YosysHQ/picorv32.git|verilog|||"
    "vhdl-uvvm|https://github.com/UVVM/UVVM.git|vhdl|uvvm_util/src||"
    "asm-xv6|https://github.com/mit-pdos/xv6-public.git|asm|||"
    "asm-xv6-mixed|https://github.com/mit-pdos/xv6-public.git|c,asm|||"
    "kivy-carvera|https://github.com/Carvera-Community/Carvera_Controller.git|kivy|.||"
    "smithy-protocol-tests|https://github.com/smithy-lang/smithy.git|smithy|smithy-aws-protocol-tests/model||"
    "proto-grpc|https://github.com/grpc/grpc-proto.git|proto|||"
    "graphql-github|https://github.com/octokit/graphql-schema.git|graphql|||"
    "openapi-spec|https://github.com/OAI/OpenAPI-Specification.git|openapi|examples||"
    "asyncapi-spec|https://github.com/asyncapi/spec.git|asyncapi|examples||"
)

# Framework application corpus: name|url|frameworks
#
# One real application per framework rule in
# crates/cgg-core/src/frameworks/rules.rs. These are applications that
# *use* a framework, never the framework's own repository — a router's
# own test suite proves the grammar parses, not that cgg recognises the
# hand-off shape as an application writes it.
#
# scripts/framework-coverage.py reads this array and fails if a declared
# framework does not fire, or if a rule in rules.rs has no entry here.
#
# A `~` prefix means "cgg detects this framework on this app but
# enumerates no entry points from it" — the framework must still land in
# the coverage table's `seen, no rules` section, which is what keeps a
# gap visible instead of silently reporting zero. Two reasons a rule sits
# here, both real and both worth tracking:
#
#   ~nextjs, ~blazor       the rule ships a `gap:` string: routes live in
#                          file-system layout / `.razor` markup, neither
#                          of which cgg parses.
#   ~chi, ~sinatra, …      the rule has matchers, but the application
#                          writes the idiom in a form they miss (handlers
#                          wrapped in `chain.ToHandlerFunc(...)`, Sinatra
#                          `get "/x" do … end` blocks, `new Worker(v)`
#                          with a variable path). These are the ones to
#                          fix; the marker is a to-do, not an excuse.
APPS=(
    "app-fastapi-dispatch|https://github.com/Netflix/dispatch.git|click,fastapi,pydantic,py-signal,~py-context-manager,~py-threading,~py-unittest,~react"
    "app-django-netbox|https://github.com/netbox-community/netbox.git|django,django-admin,py-unittest,rq,~py-argparse,~py-context-manager,~py-threading"
    "app-flaskbb-flask|https://github.com/flaskbb/flaskbb.git|celery,click,flask,~py-context-manager"
    "app-saleor-celery|https://github.com/saleor/saleor.git|celery,django,django-admin,py-context-manager,~py-threading,~py-unittest"
    "app-black-click|https://github.com/psf/black.git|click,py-signal,py-unittest,~py-argparse,~py-context-manager,~py-threading"
    "app-torch-ultralytics|https://github.com/ultralytics/ultralytics.git|flask,py-context-manager,py-threading,rust-std-traits,torch,~py-argparse,~py-signal,~py-unittest"
    "app-ghost-express|https://github.com/TryGhost/Ghost.git|express,node-event-emitter,react,worker-threads"
    "app-ghostfolio-nestjs|https://github.com/ghostfolio/ghostfolio.git|angular,angular-host,angular-router,bullmq,express,nestjs,nestjs-schedule"
    "app-immich-nestjs|https://github.com/immich-app/immich.git|android-broadcastreceiver,android-worker,bullmq,cocoa-delegate,compose,express,fastapi,nestjs,node-event-emitter,py-context-manager,torch,uikit-appdelegate,worker-threads,~android-activity,~android-application,~java-closeable,~nestjs-schedule,~py-argparse,~py-signal,~py-threading,~py-unittest,~react,~uikit-lifecycle"
    "app-calcom-nextjs|https://github.com/calcom/cal.com.git|bullmq,express,nestjs,node-event-emitter,react,wordpress,worker-threads"
    "app-spring-mall|https://github.com/macrozheng/mall.git|junit,servlet,spring,spring-jobs,spring-messaging,~java-closeable,~java-concurrent"
    "app-thingsboard-concurrent|https://github.com/thingsboard/thingsboard.git|angular,angular-host,angular-router,express,hibernate-listeners,java-closeable,java-concurrent,junit,servlet,spring,spring-jobs,~jpa-lifecycle"
    "app-akka-samples|https://github.com/akka/akka-samples.git|akka,junit,~java-closeable,~java-concurrent"
    "app-druid-jaxrs|https://github.com/apache/druid.git|jakarta-rs,java-closeable,java-concurrent,junit,react,servlet,~py-argparse"
    "app-micronaut-graalapp|https://github.com/micronaut-guides/micronaut-creating-first-graal-app.git|junit,micronaut,~java-closeable"
    "app-gin-photoprism|https://github.com/photoprism/photoprism.git|chi,gin,go-encoding,gorilla-mux,net-http,robfig-cron"
    "app-memos-echo|https://github.com/usememos/memos.git|echo,react,~go-encoding,~net-http"
    "app-fiber-recipes|https://github.com/gofiber/recipes.git|asynq,fiber,hono,net-http,~go-encoding,~react"
    "app-homebox-chi|https://github.com/sysadminsmedia/homebox.git|chi,go-encoding,py-unittest,~net-http"
    "app-temporal-samples|https://github.com/temporalio/samples-go.git|net-http,temporal,~go-encoding"
    "app-eshop-aspnet|https://github.com/dotnet/eShop.git|aspnet-minimal,aspnet-mvc,csharp-disposable,dotnet-hosting,ef-core,mstest,mvvm-toolkit,~blazor-lifecycle"
    "app-masstransit-sample|https://github.com/MassTransit/Sample-Twitch.git|aspnet-mvc,dotnet-hosting,masstransit,~aspnet-minimal,~csharp-disposable"
    "app-ombi-quartz|https://github.com/Ombi-app/Ombi.git|angular,angular-router,aspnet-mvc,csharp-disposable,ef-core,quartz,signalr,~angular-host,~aspnet-minimal,~dotnet-hosting"
    "app-axum-cratesio|https://github.com/rust-lang/crates.io.git|axum,rust-std-traits,~py-argparse"
    "app-lemmy-actix|https://github.com/LemmyNet/lemmy.git|actix-web,rust-std-traits"
    "app-actix-examples|https://github.com/actix/examples.git|actix-actor,actix-web,rust-std-traits,~py-argparse,~py-context-manager,~py-signal"
    "app-vaultwarden-rocket|https://github.com/dani-garcia/vaultwarden.git|rocket,rust-std-traits,~node-event-emitter,~py-argparse"
    "app-rails-mastodon|https://github.com/mastodon/mastodon.git|express,rails,rails-callbacks,react,sidekiq,~actioncable"
    "app-resque-sinatra|https://github.com/resque/resque.git|minitest,rails,sinatra,~actioncable,~rails-callbacks"
    "app-grape-swagger|https://github.com/ruby-grape/grape-swagger.git|grape,~rails-callbacks"
    "app-monica-laravel|https://github.com/monicahq/monica.git|laravel,laravel-lifecycle,phpunit,~symfony"
    "app-symfony-demo|https://github.com/symfony/demo.git|phpunit,symfony"
    "app-wordpress|https://github.com/WordPress/WordPress.git|wordpress"
    "app-codeigniter-starter|https://github.com/codeigniter4/appstarter.git|codeigniter"
    "app-cuda-samples|https://github.com/NVIDIA/cuda-samples.git|cuda,~py-argparse,~py-context-manager,~py-threading,~torch"
    "solidity-openzeppelin|https://github.com/OpenZeppelin/openzeppelin-contracts.git|solidity-public"
    "app-pydantic-core|https://github.com/pydantic/pydantic-core.git|ffi-export,py-unittest,rust-std-traits"
    "app-nextcloud-android|https://github.com/nextcloud/android.git|android-activity,android-application,android-broadcastreceiver,android-contentprovider,android-fragment,android-service,android-service-bind,android-service-start,android-worker,compose,java-concurrent,junit,~java-closeable,~py-argparse"
    "app-plausible-phoenix|https://github.com/plausible/analytics.git|express,mix-task,otp,otp-application,phoenix,phoenix-liveview,plug,react,~phoenix-channel"
    "app-hono-examples|https://github.com/honojs/examples.git|hono,~react"
    "app-unity-gamekit|https://github.com/Unity-Technologies/EndlessRunnerSampleGame.git|csharp-disposable,unity"
    # AWS Lambda spans six languages and two registration mechanisms, so
    # it takes three applications to exercise. cdk-examples is the one
    # that matters most: it is the only app in this list where the entry
    # points are named by a *string in the infrastructure code* rather
    # than by anything in the handler's own file.
    "app-powertools-lambda|https://github.com/aws-powertools/powertools-lambda-python.git|aws-lambda,aws-lambda-powertools,aws-cdk"
    "app-awslambda-go|https://github.com/aws/aws-lambda-go.git|aws-lambda-go,net-http"
    "app-cdk-examples|https://github.com/aws-samples/aws-cdk-examples.git|aws-cdk,aws-lambda,aws-lambda-go,junit"
    # The other clouds. Every one of these was detect-only or absent
    # before 0.6.8, so each entry is the application that proves its
    # rule enumerates rather than merely disclosing a gap.
    "app-gcp-functions|https://github.com/GoogleCloudPlatform/functions-framework-nodejs.git|gcp-functions,express"
    "app-firebase-samples|https://github.com/firebase/functions-samples.git|firebase-functions,express,flask"
    "app-azure-functions-js|https://github.com/Azure-Samples/functions-quickstart-javascript-azd.git|azure-functions"
    "app-azure-functions-dotnet|https://github.com/Azure-Samples/functions-quickstart-dotnet-azd.git|azure-functions,~aspnet-minimal"
    "app-cloudflare-workers|https://github.com/cloudflare/workers-rs.git|cloudflare-workers,axum,ffi-export"
    "app-deno-std|https://github.com/denoland/std.git|deno-http,node-event-emitter,ffi-export"
    "app-quotesbot-scrapy|https://github.com/scrapy/quotesbot.git|scrapy"
    "app-dash-sample-apps|https://github.com/plotly/dash-sample-apps.git|dash,~flask,~py-argparse,~py-threading,~torch"
    "app-shopping-loopback|https://github.com/loopbackio/loopback4-example-shopping.git|express,loopback"
    "app-hertz-examples|https://github.com/cloudwego/hertz-examples.git|hertz,~go-encoding,~go-runtime,~go-signal,~go-testing,~net-http"
    "app-litestar-fullstack|https://github.com/litestar-org/litestar-fullstack.git|click,litestar,~py-argparse,~py-context-manager,~py-threading,~py-unittest,~pydantic,~pytest,~react,~react-hooks,~vitest"
    "app-gs-batch-processing|https://github.com/spring-guides/gs-batch-processing.git|junit,spring-batch,~spring"
    "app-metabase-compojure|https://github.com/metabase/metabase.git|angular,compojure,express,node-event-emitter,react,~angular-host,~angular-router,~bun-serve,~clojure-test,~nextjs,~react-hooks,~ring,~vitest"
    "kivy-carvera|https://github.com/Carvera-Community/Carvera_Controller.git|kivy,py-threading,uikit-lifecycle"
    "app-salvo-examples|https://github.com/salvo-rs/salvo.git|react,rust-std-traits,salvo,~criterion,~react-hooks,~rust-runtime,~serde"
    "app-avalonia-samples|https://github.com/AvaloniaUI/Avalonia.Samples.git|avalonia,csharp-disposable,mvvm-toolkit,~avalonia-xaml,~csharp-runtime,~fsharp-entrypoint,~nunit,~xunit"
    "app-monogame-samples|https://github.com/MonoGame/MonoGame.Samples.git|csharp-disposable,monogame,~csharp-runtime"
    "app-godot-demos|https://github.com/godotengine/godot-demo-projects.git|godot,~csharp-disposable,~csharp-runtime,~godot-signal"
    "app-jabref-javafx|https://github.com/JabRef/jabref.git|jakarta-rs,java-closeable,java-concurrent,javafx,junit,swing,~java-runtime,~java-serviceloader,~junit-params,~kotlin-runtime,~nix,~py-argparse"
    "app-shuttle-examples|https://github.com/shuttle-hq/shuttle-examples.git|actix-web,axum,rocket,rust-std-traits,salvo,shuttle,~nextjs,~poem,~react,~react-hooks,~rust-runtime,~serde,~warp"
    "app-blazor-samples|https://github.com/dotnet/blazor-samples.git|aspnet-minimal,aspnet-mvc,blazor-jsinterop,blazor-lifecycle,csharp-disposable,ef-core,signalr,~blazor,~csharp-runtime,~dotnet-hosting,~maui,~react,~react-hooks"
    "app-orleans-samples|https://github.com/dotnet/samples.git|android-activity,aspnet-mvc,csharp-disposable,dotnet-hosting,mstest,orleans,uikit-appdelegate,uikit-lifecycle,~aspnet-minimal,~azure-functions,~blazor,~blazor-lifecycle,~c-runtime,~cocoa-delegate,~cpp-runtime,~csharp-runtime,~fsharp-entrypoint,~fsharp-nunit,~nunit,~objc-main,~objc-selector,~signalr,~wcf,~wpf-winforms,~xunit"
    "app-rn-expensify|https://github.com/Expensify/App.git|android-broadcastreceiver,android-jobservice,android-service-bind,android-service-start,react-native,uikit-appdelegate,uikit-lifecycle,~android-activity,~android-application,~android-service,~bun-serve,~c-runtime,~cocoa-delegate,~java-closeable,~java-concurrent,~java-runtime,~jest,~kotlin-runtime,~objc-main,~objc-selector,~react,~react-hooks,~swiftui"
    "app-serverless-examples|https://github.com/serverless/examples.git|aws-lambda,aws-lambda-go,express,flask,lambda-runtime,mediatr,nestjs,node-event-emitter,sinatra,~apollo-server,~csharp-disposable,~csharp-runtime,~go-encoding,~go-runtime,~go-testing,~java-closeable,~java-runtime,~net-http,~playwright,~serde,~xunit"
    "app-storm-starter|https://github.com/apache/storm.git|express,jakarta-rs,java-closeable,java-concurrent,junit,py-unittest,servlet,storm,~c-runtime,~clojure-test,~java-runtime,~java-serviceloader,~junit-params,~py-argparse,~py-context-manager"
    "app-symfony-messenger|https://github.com/symfony/symfony.git|phpunit,symfony,symfony-messenger,~wordpress"
    "app-wordpress-android|https://github.com/wordpress-mobile/WordPress-Android.git|android-broadcastreceiver,android-fragment,android-jobservice,android-service,android-service-bind,android-service-start,android-worker,compose,junit,~android-activity,~android-application,~android-compose,~android-contentprovider,~java-closeable,~java-concurrent,~java-runtime,~junit-params,~kotlin-runtime,~kotlinx-coroutines"
    "csharp-mediatr|https://github.com/jbogard/MediatR.git|mediatr,~csharp-disposable,~csharp-runtime,~xunit"
    "elixir-phoenix|https://github.com/phoenixframework/phoenix.git|mix-task,otp,otp-application,phoenix,phoenix-channel,phoenix-socket,plug,plug-router,~jest"
    "erlang-otp|https://github.com/erlang/otp.git|erlang-gen-server,~c-runtime,~common-test,~cpp-runtime,~erlang-application,~erlang-otp-lifecycle,~java-closeable,~java-runtime"
    "groovy-gradle|https://github.com/gradle/gradle.git|android-activity,compose,gradle-plugin,java-closeable,java-concurrent,junit,servlet,~android-compose,~android-fragment,~c-runtime,~cpp-runtime,~gradle-task,~gtest,~java-runtime,~java-serviceloader,~junit-params,~kotlin-runtime,~kotlinx-coroutines,~play-mvc,~scalatest,~spock,~spring,~xctest"
    "haskell-hspec|https://github.com/hspec/hspec.git|hspec,~haskell-main"
    "haskell-pandoc|https://github.com/jgm/pandoc.git|tasty,~haskell-main,~nix,~servant,~wai"
    "js-polka|https://github.com/lukeed/polka.git|express,polka,react,~apollo-server,~fastify,~nextjs,~nuxt,~react-hooks,~socket-io"
    "js-restify|https://github.com/restify/node-restify.git|node-event-emitter,restify,~socket-io"
    "julia-flux|https://github.com/FluxML/Flux.jl.git|julia-base-dispatch,~flux,~julia-module-init,~julia-test"
    "julia-genie|https://github.com/GenieFramework/Genie.jl.git|julia-base-dispatch,julia-module-init,~genie,~julia-test"
    "perl-dancer2|https://github.com/PerlDancer/Dancer2.git|dancer2,~plack,~test-more"
    "perl-mojolicious|https://github.com/mojolicious/mojo.git|mojolicious,~test-more"
    "py-apscheduler|https://github.com/agronholm/apscheduler.git|apscheduler,fastapi,flask,py-context-manager,~py-atexit,~py-threading,~py-unittest,~pytest,~starlette"
    "py-dramatiq|https://github.com/Bogdanp/dramatiq.git|apscheduler,dramatiq,py-signal,py-threading,~celery,~py-argparse,~py-atexit,~py-context-manager,~py-setuptools-entrypoints,~py-unittest,~pytest"
    "py-falcon|https://github.com/falconry/falcon.git|django,falcon,flask,py-context-manager,py-threading,py-unittest,~bottle,~django-admin,~py-argparse,~py-atexit,~py-setuptools-entrypoints,~py-signal,~pytest"
    "py-huey|https://github.com/coleifer/huey.git|django,django-admin,flask,huey,py-context-manager,py-signal,py-threading,py-unittest,~py-atexit,~py-setuptools-entrypoints"
    "py-quart|https://github.com/pallets/quart.git|click,flask,py-context-manager,py-signal,quart,~py-threading,~py-unittest,~pytest"
    "py-robyn|https://github.com/sparckles/Robyn.git|actix-actor,ffi-export,py-signal,py-threading,robyn,rust-std-traits,~actix-web,~nextjs,~py-argparse,~py-unittest,~pydantic,~pytest,~react,~react-hooks,~rust-runtime,~serde"
    "r-ggplot2|https://github.com/tidyverse/ggplot2.git|r-package-hooks,~r-s3-dispatch,~testthat"
    "rust-ntex|https://github.com/ntex-rs/ntex.git|ntex,rust-std-traits,~rust-runtime,~serde"
    "rust-rtic|https://github.com/rtic-rs/rtic.git|embedded-rt,rust-std-traits,~clap,~rust-runtime"
    # Added 2026-09-23 for detect-only rules that no application exercised.
    # One real application per rule, never the framework itself; each was
    # confirmed to be detected by cgg before it was listed here.
    "app-adocasts-adonisjs|https://github.com/adocasts/adocasts.git|~adonisjs"
    "app-casdoor-beego|https://github.com/casdoor/casdoor.git|~beego"
    "app-golangflow-buffalo|https://github.com/bscott/golangflow.git|~buffalo"
    "app-passbolt-cakephp|https://github.com/passbolt/passbolt_api.git|~cakephp"
    "app-kapua-camel|https://github.com/eclipse-kapua/kapua.git|~camel"
    "app-metacpan-catalyst|https://github.com/metacpan/metacpan-web.git|~catalyst"
    "app-ghcli-cobra|https://github.com/cli/cli.git|~cobra"
    "app-codeedit-combine|https://github.com/CodeEditApp/CodeEdit.git|~combine"
    "app-annif-connexion|https://github.com/NatLibFi/Annif.git|~connexion"
    "app-vernemq-cowboy|https://github.com/vernemq/vernemq.git|~cowboy"
    "app-oldata-dagster|https://github.com/mitodl/ol-data-platform.git|~dagster"
    "app-marquez-dropwizard|https://github.com/MarquezProject/marquez.git|~dropwizard"
    "app-opensocial-drupal|https://github.com/goalgorilla/open_social.git|~drupal"
    "app-marktext-electron|https://github.com/marktext/marktext.git|~electron"
    "app-fsautocomplete-expecto|https://github.com/ionide/FsAutoComplete.git|~expecto"
    "app-betting-expressgateway|https://github.com/johndavedecano/betting-api-starter.git|~express-gateway"
    "app-insights-feathers|https://github.com/mariusandra/insights.git|~feathers"
    "app-darkness-flame|https://github.com/RafaelBarbosatec/darkness_dungeon.git|~flame"
    "app-flinkplatform-flink|https://github.com/zhp8341/flink-streaming-platform-web.git|~flink"
    "app-cardmgmt-giraffe|https://github.com/atsapura/CardManagement.git|~giraffe"
    "app-micromdm-gokit|https://github.com/micromdm/micromdm.git|~go-kit"
    "app-arduinoagent-goa|https://github.com/arduino/arduino-create-agent.git|~goa"
    "app-concordium-gotham|https://github.com/Concordium/concordium-node.git|~gotham"
    "app-sdwebui-gradio|https://github.com/AUTOMATIC1111/stable-diffusion-webui.git|~gradio"
    "app-streama-grails|https://github.com/streamaserver/streama.git|~grails"
    "app-toiletmap-yoga|https://github.com/public-convenience-ltd/toiletmap.git|~graphql-yoga"
    "app-terminus-hanami|https://github.com/usetrmnl/terminus.git|~hanami"
    "app-aiproxy-hangfire|https://github.com/unfish/AI_Proxy_United.git|~hangfire"
    "app-pa11y-hapi|https://github.com/pa11y/pa11y-webservice.git|~hapi"
    "app-riemann-httpkit|https://github.com/riemann/riemann.git|~http-kit"
    "app-watchlistarr-http4s|https://github.com/nylonee/watchlistarr.git|~http4s"
    "app-kubepi-iris|https://github.com/1Panel-dev/KubePi.git|~iris"
    "app-wificonnect-iron|https://github.com/balena-os/wifi-connect.git|~iron"
    "app-saplib-jenkins|https://github.com/SAP/jenkins-library.git|~jenkins-pipeline"
    "app-dataverse-jsf|https://github.com/IQSS/dataverse.git|~jsf"
    "app-ksql-kafkastreams|https://github.com/confluentinc/ksql.git|~kafka-streams"
    "app-yapi-koa|https://github.com/YMFE/yapi.git|~koa"
    "app-circuitbreaker-kong|https://github.com/dream11/kong-circuit-breaker.git|~kong"
    "app-moon-kratos|https://github.com/aide-family/moon.git|~kratos"
    "app-kotlinconf-ktor|https://github.com/JetBrains/kotlinconf-app.git|~ktor"
    "app-kori-ktorclient|https://github.com/YangDai2003/Kori.git|~ktor-client"
    "app-ferry-machinery|https://github.com/lanyulei/ferry.git|~machinery"
    "app-elasticsuite-magento|https://github.com/Smile-SA/elasticsuite.git|~magento"
    "app-4minitz-meteor|https://github.com/4minitz/4minitz.git|~meteor"
    "app-wiregui-nicegui|https://github.com/bartei/wiregui.git|~nicegui"
    "app-apisix-openresty|https://github.com/apache/apisix.git|~openresty"
    "app-zpool-opium|https://github.com/uzh/z-pool-tool.git|~opium"
    "app-xrviz-panel|https://github.com/intake/xrviz.git|~panel"
    "app-cljdoc-pedestal|https://github.com/cljdoc/cljdoc.git|~pedestal"
    "app-importexcel-pester|https://github.com/dfinke/ImportExcel.git|~pester,~powershell-module"
    "app-riffraff-play|https://github.com/guardian/riff-raff.git|~play-module"
    "app-arcos-plumber|https://github.com/wpinvestigative/arcos-api.git|~plumber"
    "app-basedosdados-prefect|https://github.com/basedosdados/pipelines.git|~prefect"
    "app-hypothesis-pyramid|https://github.com/hypothesis/h.git|~pyramid"
    "app-lipas-reitit|https://github.com/lipas-liikuntapaikat/lipas.git|~reitit"
    "app-wishlist-remix|https://github.com/Hujjat/wishlist-inspire-app.git|~remix"
    "app-mediom-revel|https://github.com/huacnlee/mediom.git|~revel"
    "app-shlink-roadrunner|https://github.com/shlinkio/shlink.git|~roadrunner"
    "app-html2rss-roda|https://github.com/html2rss/html2rss-web.git|~roda"
    "app-owllook-sanic|https://github.com/howie6879/owllook.git|~sanic"
    "app-swate-saturn|https://github.com/nfdi4plants/Swate.git|~saturn"
    "app-realworld-scotty|https://github.com/eckyputrady/haskell-scotty-realworld-example-app.git|~scotty"
    "app-dartpad-shelf|https://github.com/dart-lang/dart-pad.git|~shelf"
    "app-tweetconf-shiny|https://github.com/gadenbuie/tweet-conf-dash.git|~shiny"
    "app-diaspora-sidekiq|https://github.com/diaspora/diaspora.git|~sidekiq-lifecycle"
    "app-grocy-slim|https://github.com/grocy/grocy.git|~slim"
    "app-jobserver-spark|https://github.com/spark-jobserver/spark-jobserver.git|~spark"
    "app-facto-specs2|https://github.com/nymanjens/facto.git|~specs2"
    "app-bamboobsc-struts|https://github.com/billchen198318/bamboobsc.git|~struts"
    "app-fssnip-suave|https://github.com/fssnippets/fssnip-website.git|~suave"
    "app-ora-swifttesting|https://github.com/the-ora/browser.git|~swift-testing"
    "app-mineadmin-swoole|https://github.com/mineadmin/MineAdmin.git|~swoole"
    "app-clashverge-tauri|https://github.com/clash-verge-rev/clash-verge-rev.git|~tauri"
    "app-realworld-tide|https://github.com/colinbankier/realworld-tide.git|~tide"
    "app-jupyterhub-tornado|https://github.com/jupyterhub/jupyterhub.git|~tornado"
    "app-bambufarm-vaadin|https://github.com/TFyre/bambu-farm.git|~vaadin"
    "app-alarik-vapor|https://github.com/achtungsoftware/alarik.git|~vapor"
    "app-stackage-yesod|https://github.com/commercialhaskell/stackage-server.git|~yesod"
    "app-humhub-yii|https://github.com/humhub/humhub.git|~yii"
    "app-ghostty-zig|https://github.com/ghostty-org/ghostty.git|~zig-build"
    "app-libxev-zig|https://github.com/mitchellh/libxev.git|~zig-export"
    "app-zls-zig|https://github.com/zigtools/zls.git|~zig-main"
    "app-tigerbeetle-zig|https://github.com/tigerbeetle/tigerbeetle.git|~zig-test"
    "app-neorv32-vhdl|https://github.com/stnolting/neorv32.git|~vhdl-toplevel"
    "app-fpm-fortran|https://github.com/fortran-lang/fpm.git|~fortran-program"
    "app-waybar-catch2|https://github.com/Alexays/Waybar.git|~cpp-test-macros"
    "app-kythe-bazel|https://github.com/kythe/kythe.git|~bazel"
    # Every corpus directory is an APPS entry, so framework coverage and the
    # manifest sync measure all of them. Claims are measured, not hand-written
    # (scripts/sync-app-manifest.py).
    "app-carvera-firmware-cpp|https://github.com/Carvera-Community/Carvera_Community_Firmware.git|py-signal,py-threading,~py-argparse"
    "app-itunesweb-padrino|https://github.com/sshaw/itunes_store_transporter_web.git|sinatra,~sidekiq"
    "app-katrain-kivy|https://github.com/sanderland/katrain.git|kivy,py-signal,py-threading,~py-unittest"
    "app-lobsters-puma|https://github.com/lobsters/lobsters.git|rails,rails-callbacks,sidekiq,~actioncable"
    "app-mimic-kivy|https://github.com/ISS-Mimic/Mimic.git|kivy,py-signal,py-threading,~py-argparse"
    "app-mtgnode-sails|https://github.com/Yomguithereal/mtgnode.git|"
    "app-networkingdsc-dsc|https://github.com/dsccommunity/NetworkingDsc.git|"
    "app-ocamlci-alcotest|https://github.com/ocurrent/ocaml-ci.git|"
    "app-ocamlorg-dream|https://github.com/ocaml/ocaml.org.git|"
    "app-opam-cmdliner|https://github.com/ocaml/opam.git|"
    "app-pywallet-kivy|https://github.com/AndreMiras/PyWallet.git|kivy,py-unittest,~py-threading"
    "app-quack-oak|https://github.com/raaymax/quack.git|android-broadcastreceiver,android-service-bind,android-service-start,junit,node-event-emitter,~android-activity,~android-service,~react"
    "app-rebar3-eunit|https://github.com/erlang/rebar3.git|erlang-gen-server"
    "app-smoothie-kivy|https://github.com/wolfmanjm/kivy-smoothie-host.git|kivy,py-signal,py-threading,~py-argparse"
    "app-smoothieware-cpp|https://github.com/Smoothieware/Smoothieware.git|py-signal,py-threading,~py-argparse"
    "app-snlquest-kivy|https://github.com/sandialabs/snl-quest.git|kivy,py-threading,~py-context-manager"
    "app-unity-boat|https://github.com/Unity-Technologies/BoatAttack.git|csharp-disposable,unity"
    "app-zenplayer-kivy|https://github.com/Zen-CODE/zenplayer.git|flask,kivy,py-threading,pydantic"
    "asm-xv6|https://github.com/mit-pdos/xv6-public.git|"
    "asyncapi-spec|https://github.com/asyncapi/spec.git|"
    "bash-acme|https://github.com/acmesh-official/acme.sh.git|"
    "c-jq|https://github.com/jqlang/jq.git|"
    "c-redis|https://github.com/redis/redis.git|py-threading,~py-argparse,~py-signal"
    "clojure-compojure|https://github.com/weavejester/compojure.git|compojure"
    "clojure-ring|https://github.com/ring-clojure/ring.git|"
    "cloudflare-worker-examples|https://github.com/cloudflare/worker-template.git|"
    "cmake-kitware|https://github.com/Kitware/CMake.git|android-activity,cocoa-delegate,cuda,ffi-export,py-threading,py-unittest,uikit-appdelegate,uikit-lifecycle,~csharp-disposable,~java-closeable,~py-argparse,~rust-std-traits"
    "cpp-nlohmann-json|https://github.com/nlohmann/json.git|py-threading,~py-argparse,~py-context-manager"
    "cpp-spdlog|https://github.com/gabime/spdlog.git|"
    "csharp-newtonsoft|https://github.com/JamesNK/Newtonsoft.Json.git|csharp-disposable"
    "csharp-serilog|https://github.com/serilog/serilog.git|csharp-disposable"
    "dart-flame|https://github.com/flame-engine/flame.git|~py-signal"
    "dart-flutter|https://github.com/flutter/flutter.git|android-activity,android-fragment,cocoa-delegate,compose,java-closeable,java-concurrent,junit,py-unittest,uikit-appdelegate,uikit-lifecycle,~android-application,~android-broadcastreceiver,~android-contentprovider,~android-service,~android-service-bind,~android-service-start,~py-argparse,~py-signal,~py-threading"
    "fortran-stdlib|https://github.com/fortran-lang/stdlib.git|~py-argparse"
    "fsharp-paket|https://github.com/fsprojects/Paket.git|csharp-disposable"
    "go-caddy|https://github.com/caddyserver/caddy.git|chi,go-encoding,net-http"
    "go-fzf|https://github.com/junegunn/fzf.git|minitest,~go-encoding"
    "go-hertz|https://github.com/cloudwego/hertz.git|go-encoding,hertz,net-http"
    "go-martini|https://github.com/go-martini/martini.git|net-http"
    "graphql-github|https://github.com/octokit/graphql-schema.git|"
    "hcl-terraform-aws|https://github.com/hashicorp/terraform-provider-aws.git|go-encoding,~net-http,~py-argparse"
    "hcl-vpc|https://github.com/terraform-aws-modules/terraform-aws-vpc.git|"
    "java-gson|https://github.com/google/gson.git|java-closeable,junit,~java-concurrent"
    "java-okhttp|https://github.com/square/okhttp.git|java-closeable,junit,~android-activity,~android-application,~java-concurrent"
    "java-spring-batch|https://github.com/spring-projects/spring-batch.git|java-concurrent,junit,spring-batch,spring-jobs,~java-closeable,~jpa-lifecycle,~spring-messaging"
    "js-express|https://github.com/expressjs/express.git|"
    "js-lodash|https://github.com/lodash/lodash.git|"
    "js-loopback|https://github.com/loopbackio/loopback-next.git|express,loopback,node-event-emitter"
    "kivy-kivymd|https://github.com/kivymd/KivyMD.git|kivy,~py-argparse"
    "kotlin-okio|https://github.com/square/okio.git|junit,~java-closeable,~java-concurrent"
    "lua-kong|https://github.com/Kong/kong.git|rust-std-traits,~py-argparse"
    "lua-neovim|https://github.com/neovim/neovim.git|"
    "nix-home-manager|https://github.com/nix-community/home-manager.git|~py-argparse"
    "objc-afnetworking|https://github.com/AFNetworking/AFNetworking.git|cocoa-delegate,uikit-appdelegate,uikit-lifecycle"
    "ocaml-dune|https://github.com/ocaml/dune.git|"
    "openapi-spec|https://github.com/OAI/OpenAPI-Specification.git|"
    "php-laravel|https://github.com/laravel/framework.git|laravel,laravel-lifecycle,phpunit,symfony,~wordpress"
    "powershell-psget|https://github.com/PowerShell/PowerShellGet.git|"
    "proto-grpc|https://github.com/grpc/grpc-proto.git|"
    "py-dash|https://github.com/plotly/dash.git|celery,dash,fastapi,flask,py-context-manager,py-signal,py-threading,py-unittest,quart,react,~py-argparse,~pydantic"
    "py-litestar|https://github.com/litestar-org/litestar.git|click,litestar,py-context-manager,pydantic,~py-argparse,~py-threading,~py-unittest"
    "py-scrapy|https://github.com/scrapy/scrapy.git|py-context-manager,py-signal,py-threading,py-unittest,scrapy,~py-argparse"
    "python-flask|https://github.com/pallets/flask.git|celery,click,flask,py-context-manager"
    "python-httpie|https://github.com/httpie/cli.git|py-context-manager,py-threading,~py-argparse,~py-unittest"
    "ruby-jekyll|https://github.com/jekyll/jekyll.git|minitest"
    "rust-bat|https://github.com/sharkdp/bat.git|rust-std-traits,uikit-lifecycle,~cocoa-delegate,~java-closeable,~py-argparse,~py-context-manager,~py-threading,~react,~uikit-appdelegate"
    "rust-ripgrep|https://github.com/BurntSushi/ripgrep.git|rust-std-traits,~py-argparse,~py-threading"
    "scala-play|https://github.com/playframework/playframework.git|java-closeable,junit,~hibernate-listeners,~java-concurrent,~jpa-lifecycle"
    "scala-spark|https://github.com/apache/spark.git|java-closeable,java-concurrent,junit,py-context-manager,py-signal,py-threading,py-unittest,r-package-hooks,servlet,torch,~jakarta-rs,~py-argparse"
    "smithy-protocol-tests|https://github.com/smithy-lang/smithy.git|java-closeable,java-concurrent,junit,py-unittest,~py-argparse,~py-context-manager"
    "starlark-skylib|https://github.com/bazelbuild/bazel-skylib.git|"
    "swift-alamofire|https://github.com/Alamofire/Alamofire.git|cocoa-delegate,uikit-appdelegate,uikit-lifecycle"
    "ts-typeorm|https://github.com/typeorm/typeorm.git|~react"
    "ts-zod|https://github.com/colinhacks/zod.git|~react"
    "verilog-picorv32|https://github.com/YosysHQ/picorv32.git|"
    "vhdl-uvvm|https://github.com/UVVM/UVVM.git|~py-argparse"
    "zig-http|https://github.com/karlseguin/http.zig.git|"
    "zig-zig|https://github.com/ziglang/zig.git|uikit-appdelegate,~cocoa-delegate,~uikit-lifecycle"
    # Real applications for frameworks whose only evidence was the
    # framework's own repository (2026-09-23). The framework repos stay:
    # removing them would break historical comparisons.
    "app-alphapem-genie|https://github.com/gassraphael/AlphaPEM.git|julia-module-init,~julia-base-dispatch"
    "app-amurex-robyn|https://github.com/thepersonalaicompany/amurex-backend.git|robyn,~py-context-manager,~py-threading"
    "app-barong-grape|https://github.com/openware/barong.git|rails,rails-callbacks,sidekiq,~actioncable,~minitest"
    "app-bewcloud-deno|https://github.com/bewcloud/bewcloud.git|deno-http"
    "app-brightsky-huey|https://github.com/jdemaeyer/brightsky.git|click,fastapi,huey,py-context-manager,pydantic,~falcon,~py-threading,~py-unittest"
    "app-bugbug-gcpfunctions|https://github.com/mozilla/bugbug.git|fastapi,flask,gcp-functions,py-context-manager,py-signal,py-threading,pydantic,~py-argparse,~py-unittest,~react,~rq"
    "app-caninclude-polka|https://github.com/CyberLight/caninclude-v2.git|~polka"
    "app-crnn-flux|https://github.com/DENG-MIT/CRNN.git|~julia-base-dispatch,~julia-module-init"
    "app-dailynotes-quart|https://github.com/djedi/DailyNotes.git|quart"
    "app-domainwatchdog-symfony|https://github.com/maelgangloff/domain-watchdog.git|phpunit,symfony,~react,~symfony-messenger"
    "app-edrys-oak|https://github.com/edrys-org/edrys.git|~deno-http"
    "app-hadolint-hspec|https://github.com/hadolint/hadolint.git|hspec"
    "app-libchecker-gradle|https://github.com/LibChecker/LibChecker.git|android-broadcastreceiver,android-service-bind,android-service-start,java-closeable,junit,py-unittest,~android-activity,~android-application,~android-contentprovider,~android-fragment,~android-service,~py-argparse"
    "app-nanocl-ntex|https://github.com/next-hat/nanocl.git|ntex,rust-std-traits"
    "app-newscrawl-storm|https://github.com/commoncrawl/news-crawl.git|junit,storm,~java-closeable"
    "app-oncall-falcon|https://github.com/linkedin/oncall.git|falcon"
    "app-papercups-phoenix|https://github.com/papercups-io/papercups.git|mix-task,otp,otp-application,phoenix,phoenix-channel,phoenix-socket,plug,react"
    "app-perlmaven-dancer2|https://github.com/szabgab/Perl-Maven.git|dancer2"
    "app-pixlserv-martini|https://github.com/MilanMisak/pixlserv.git|martini,~go-encoding,~net-http"
    "app-ravada-mojolicious|https://github.com/UPC/ravada.git|mojolicious"
    "app-reggie-restify|https://github.com/mbrevoort/node-reggie.git|restify"
    "app-socialhome-dramatiq|https://github.com/jaywink/socialhome.git|django,django-admin,dramatiq,py-unittest,~apscheduler"
    "app-workerstunnel-cloudflare|https://github.com/zhu327/workers-tunnel.git|cloudflare-workers,~rust-std-traits"
)

# Framework rules with NO application in the APPS manifest above.
#
# The gate demands an app per enumerating rule, and the honest answer is
# sometimes "there isn't one". Listing a rule here is that statement,
# made out loud. Silence would have been the alternative, and silence is
# the failure mode this whole subsystem exists to avoid.
#
# Two things this list does NOT say. It used to imply both.
#
# 1. It does not mean a fixture verifies the rule. There is no per-rule
#    fixture for any of these. crates/cgg/tests/frameworks.rs tests the
#    six hand-off *shapes*, not individual rules, and
#    crates/cgg/tests/detect_prefixes.rs tests detection only — that a
#    rule's first `detect` prefix can fire — explicitly not enumeration.
#
# 2. It does not mean the rule has never fired on real code. Eight of
#    them enumerate against the *language* corpus (REPOS above), which
#    scripts/framework-coverage.py never reads because those repos are
#    libraries and framework sources rather than applications. Measured
#    on 0.5.0 with `cgg <repo> --framework-coverage`:
#
#      mojolicious          283 entries  perl-mojolicious
#      erlang-gen-server    101 entries  erlang-otp (lib/stdlib/src)
#      tasty                 65 entries  haskell-pandoc
#      plug-router           49 entries  elixir-phoenix
#      julia-base-dispatch   35 entries  julia-flux
#      phoenix-socket        19 entries  elixir-phoenix
#      r-package-hooks        2 entries  r-ggplot2
#      lambda-runtime         1 entry    app-serverless-examples (a clone
#                                        that is in neither manifest)
#      gradle-plugin        detected, enumerated nothing   groovy-gradle
#      julia-module-init    detected, enumerated nothing   julia-flux
#      mediatr              detected, enumerated nothing   app-serverless-examples
#
#    The other 34 were not detected anywhere in $CGG_BENCH_DIR at all.
#    Promoting those eight to APPS entries is the real fix; until then
#    this note is the evidence, and `tasty` in particular has a genuine
#    application behind it — pandoc is an application, it is just filed
#    under REPOS because it is also the Haskell language corpus.
#
# Format: id|why no application was found
APPS_UNVERIFIED=(
    "powershell-dsc|application found, not detected. dsccommunity/NetworkingDsc (app-networkingdsc-dsc) is a MOF-based DSC resource module, but every resource imports its helpers through a computed path, Import-Module -Name (Join-Path -Path modulePath -ChildPath 'DscResource.Common'); cgg reads only a literal module name, so the computed path detects nothing. Needs computed Import-Module handling, not a repo"
)

# One checkout per repository. A row that measures a repository another
# row already clones reads that row's directory instead of cloning it a
# second time under its own name — a duplicate directory is measured twice
# by every script that walks the corpus (compare-release, perf-compare,
# determinism-sweep), which inflated corpus totals. xv6 is benchmarked
# twice on purpose (asm alone, then C+asm), from one clone.
declare -A REPO_DIR=(
    [asm-xv6-mixed]=asm-xv6
)
repo_dir() { printf '%s/%s' "$REPOS_DIR" "${REPO_DIR[$1]:-$1}"; }

# Clone or update repos
clone_repos() {
    echo "Cloning/updating repos in $REPOS_DIR..."
    mkdir -p "$REPOS_DIR"
    for entry in "${REPOS[@]}"; do
        IFS='|' read -r name url _ _ _ _ <<< "$entry"
        dir="$(repo_dir "$name")"
        if [ -d "$dir" ]; then
            if [ "${1:-}" = "--update" ]; then
                echo "  Updating $name..."
                git -C "$dir" pull --ff-only 2>/dev/null || true
            fi
        else
            echo "  Cloning $name..."
            git clone --depth 1 "$url" "$dir" 2>/dev/null || echo "  FAILED: $name"
        fi
    done
    echo ""
}

# Run benchmark
run_benchmark() {
    local filter_lang="${1:-}"

    printf "%-20s │ %5s │ %6s │ %6s │ %6s │ %5s │ %8s │ %s\n" \
        "Project" "Lang" "ctags" "cgg" "Ratio" "CF%" "Time" "Tier"
    printf "%-20s─┼─%5s─┼─%6s─┼─%6s─┼─%6s─┼─%5s─┼─%8s─┼─%s\n" \
        "────────────────────" "─────" "──────" "──────" "──────" "─────" "────────" "──────────"

    local total=0 fully=0 partial=0 best=0 deficient=0

    for entry in "${REPOS[@]}"; do
        IFS='|' read -r name url lang src_dir ctags_lang ctags_kinds <<< "$entry"
        [ -n "$filter_lang" ] && [ "$lang" != "$filter_lang" ] && continue

        dir="$(repo_dir "$name")"
        [ ! -d "$dir" ] && continue

        local scan_path="$dir"
        [ -n "$src_dir" ] && [ -d "$dir/$src_dir" ] && scan_path="$dir/$src_dir"

        # ctags count (excluding anonymous, test files, enum constants for Java)
        local ct=0
        if command -v ctags &>/dev/null && [ -n "$ctags_lang" ]; then
            ct=$(ctags -R --languages="$ctags_lang" --kinds-${ctags_lang}="$ctags_kinds" \
                --exclude='test*' --exclude='*_test*' --exclude='spec' --exclude='vendor' \
                --exclude='node_modules' -f - "$scan_path" 2>/dev/null | \
                grep -av "Anonymous\|__anon\|anonFunc" | \
                awk -F'\t' '{print $1}' | \
                grep -av "^[A-Z_]*$" | \
                sort -u | wc -l)
        fi

        # cgg run
        local out
        if ! budget_left; then
            echo "  BUDGET EXHAUSTED after $(budget_spent)s — stopping before $name" >&2
            break
        fi
        out=$(run_cgg "$CGG" "$scan_path" --lang "$lang" -t mermaid -o /dev/null --metrics /tmp/cgg_bench_metrics.json 2>&1 || true)
        if [ -z "$out" ] || printf '%s' "$out" | grep -q '^$'; then :; fi
        # `timeout` kills the child at the cap; the metrics file is then
        # stale or absent, so the repo is recorded as a timeout rather
        # than silently contributing a wrong number.
        if ! printf '%s' "$out" | grep -q 'callables'; then
            echo "  TIMEOUT (>${CGG_REPO_TIMEOUT}s) or no output: $name" >&2
            TIMED_OUT_REPOS+=("$name")
            continue
        fi
        local cg=$(echo "$out" | grep -oP '\d+ callables' | grep -oP '\d+')
        local edges=$(echo "$out" | grep -oP '\d+ edges' | grep -oP '\d+')
        local cf=$(echo "$out" | grep -oP '\d+ cross-file' | grep -oP '\d+')
        local time_ms=$(echo "$out" | grep -oP '[\d.]+ ms' | grep -oP '[\d.]+')
        cg=${cg:-0}; edges=${edges:-0}; cf=${cf:-0}

        # Calculate ratio
        local ratio="n/a" tier="—"
        if [ "${ct:-0}" -gt 0 ] 2>/dev/null; then
            ratio=$(echo "scale=0; $cg * 100 / $ct" | bc)
            if [ "$ratio" -ge 90 ]; then tier="✅ Fully"; ((fully++)) || true
            elif [ "$ratio" -ge 75 ]; then tier="◐ Partial"; ((partial++)) || true
            elif [ "$ratio" -ge 50 ]; then tier="⚠ Best Effort"; ((best++)) || true
            else tier="❌ Deficient"; ((deficient++)) || true; fi
            ratio="${ratio}%"
        else
            # No ctags baseline — mark as OK if we found callables
            if [ "${cg:-0}" -gt 0 ] 2>/dev/null; then tier="✅ Fully"; ((fully++)) || true; fi
            ratio="—"
        fi

        # Cross-file percentage
        local cf_pct="—"
        [ "$edges" -gt 0 ] && cf_pct="$(echo "scale=0; $cf * 100 / $edges" | bc)%"

        printf "%-20s │ %5s │ %6s │ %6s │ %6s │ %5s │ %7sms │ %s\n" \
            "$name" "$lang" "$ct" "$cg" "$ratio" "$cf_pct" "${time_ms:-?}" "$tier"
        ((total++)) || true
    done

    echo ""
    echo "Summary: $total languages tested"
    echo "  ✅ Fully Supported (≥90%): $fully"
    echo "  ◐ Partially Supported (75-89%): $partial"
    echo "  ⚠ Best Effort (50-74%): $best"
    echo "  ❌ Deficient (<50%): $deficient"
}

# Anything that timed out is named at the end, where it cannot be missed.
# Silence here would put us back where we started: a number that looks
# complete and is not.
report_timeouts() {
    local spent; spent=$(budget_spent)
    echo
    echo "corpus run finished in ${spent}s of a ${CGG_TOTAL_BUDGET}s budget"
    if [ ${#TIMED_OUT_REPOS[@]} -gt 0 ]; then
        echo "TIMED OUT (>${CGG_REPO_TIMEOUT}s each), excluded from the table:"
        printf '  %s\n' "${TIMED_OUT_REPOS[@]}"
        echo "These are cgg bugs or genuinely huge inputs — investigate, do not raise the cap and look away."
    fi
}
trap report_timeouts EXIT

# Main
case "${1:-}" in
    --update) clone_repos --update; run_benchmark "${2:-}" ;;
    --lang)   run_benchmark "${2:-}" ;;
    --apps)   exec "$(dirname "$0")/framework-coverage.py" --clone ;;
    --help)
        echo "Usage: $0 [--update] [--lang LANG] [--apps]"
        echo "  --apps   framework coverage over the APPS corpus"
        echo "           (delegates to scripts/framework-coverage.py)"
        exit 0 ;;
    *)        clone_repos; run_benchmark ;;
esac
