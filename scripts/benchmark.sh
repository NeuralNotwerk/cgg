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
    "ts-zod|https://github.com/colinhacks/zod.git|typescript|packages/zod/src|TypeScript|fmG"
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
    "openapi-spec|https://github.com/OAI/OpenAPI-Specification.git|openapi|_archive_/schemas||"
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
    "app-fastapi-dispatch|https://github.com/Netflix/dispatch.git|click,fastapi,py-signal,pydantic,slack-bolt,~py-context-manager,~py-threading,~py-unittest,~react"
    "app-django-netbox|https://github.com/netbox-community/netbox.git|django,django-admin,django-hooks,django-lifecycle,drf-serializers,py-unittest,rq,strawberry,~py-argparse,~py-context-manager,~py-threading"
    "app-flaskbb-flask|https://github.com/flaskbb/flaskbb.git|celery,click,flask,~py-context-manager,sqlalchemy-events"
    "app-saleor-celery|https://github.com/saleor/saleor.git|celery,django,django-admin,django-hooks,django-lifecycle,graphene,py-context-manager,pydantic,~py-threading,~py-unittest"
    "app-black-click|https://github.com/psf/black.git|click,py-signal,py-unittest,~py-argparse,~py-context-manager,~py-threading"
    "app-torch-ultralytics|https://github.com/ultralytics/ultralytics.git|flask,py-context-manager,py-threading,rust-std-traits,torch,~py-argparse,~py-signal,~py-unittest"
    "app-ghost-express|https://github.com/TryGhost/Ghost.git|express,mocha,node-event-emitter,node-test,playwright,react,vitest,worker-threads"
    "app-ghostfolio-nestjs|https://github.com/ghostfolio/ghostfolio.git|angular,angular-host,angular-router,bullmq,express,jest,nestjs,nestjs-schedule"
    "app-immich-nestjs|https://github.com/immich-app/immich.git|android-activity,android-application,android-broadcastreceiver,android-worker,bullmq,cocoa-delegate,compose,express,fastapi,nestjs,node-event-emitter,playwright,py-context-manager,torch,uikit-appdelegate,vitest,~java-closeable,~nestjs-schedule,~py-argparse,~py-signal,~py-threading,~py-unittest,~pydantic,~react,~uikit-lifecycle"
    "app-calcom-nextjs|https://github.com/calcom/cal.com.git|bullmq,express,nestjs,node-event-emitter,playwright,react,vitest,wordpress,worker-threads"
    "app-spring-mall|https://github.com/macrozheng/mall.git|junit,servlet,spring,spring-jobs,spring-messaging,~java-closeable,~java-concurrent"
    "app-thingsboard-concurrent|https://github.com/thingsboard/thingsboard.git|angular,angular-host,angular-router,express,hibernate-listeners,java-closeable,java-concurrent,junit,servlet,spring,spring-jobs,~aws-lambda,~jpa-lifecycle,jsr250-lifecycle,spring-context,testng,jackson,netty,spring-websocket,grpc-java"
    "app-akka-samples|https://github.com/akka/akka-samples.git|akka,junit,~java-closeable,~java-concurrent"
    "app-druid-jaxrs|https://github.com/apache/druid.git|jakarta-rs,java-closeable,java-concurrent,junit,react,servlet,~py-argparse,guice,jackson,junit-sources,grpc-java"
    "app-micronaut-graalapp|https://github.com/micronaut-guides/micronaut-creating-first-graal-app.git|junit,micronaut,~java-closeable"
    "app-gin-photoprism|https://github.com/photoprism/photoprism.git|chi,gin,go-encoding,gorm,net-http"
    "app-memos-echo|https://github.com/usememos/memos.git|echo,net-http,react,~go-encoding"
    "app-fiber-recipes|https://github.com/gofiber/recipes.git|asynq,fiber,hono,net-http,cloudflare-workers,~go-encoding,~react"
    "app-homebox-chi|https://github.com/sysadminsmedia/homebox.git|chi,go-encoding,py-unittest,~net-http,ent-schema"
    "app-temporal-samples|https://github.com/temporalio/samples-go.git|net-http,temporal,~go-encoding"
    "app-eshop-aspnet|https://github.com/dotnet/eShop.git|aspnet-minimal,aspnet-mvc,csharp-disposable,dotnet-hosting,ef-core,mstest,mvvm-toolkit,~blazor-lifecycle,fluentvalidation,grpc-dotnet,microsoft-extensions-ai"
    "app-masstransit-sample|https://github.com/MassTransit/Sample-Twitch.git|aspnet-mvc,dotnet-hosting,masstransit,~aspnet-minimal,~csharp-disposable"
    "app-ombi-quartz|https://github.com/Ombi-app/Ombi.git|angular,angular-router,aspnet-mvc,csharp-disposable,ef-core,quartz,signalr,~angular-host,~aspnet-minimal,~dotnet-hosting,nunit-runner"
    "app-axum-cratesio|https://github.com/rust-lang/crates.io.git|axum,rust-std-traits,~py-argparse"
    "app-lemmy-actix|https://github.com/LemmyNet/lemmy.git|actix-web,rust-std-traits"
    "app-actix-examples|https://github.com/actix/examples.git|actix-actor,actix-web,rust-std-traits,~py-argparse,~py-context-manager,~py-signal,async-graphql,juniper,cargo-test"
    "app-vaultwarden-rocket|https://github.com/dani-garcia/vaultwarden.git|rocket,rust-std-traits,~node-event-emitter,~py-argparse"
    "app-rails-mastodon|https://github.com/mastodon/mastodon.git|active-model-serializers,active-model-validators,ar-migration,express,pundit,rails,rails-callbacks,react,sidekiq,thor,~actioncable,~aws-lambda"
    "app-resque-sinatra|https://github.com/resque/resque.git|minitest,rails,sinatra,~actioncable,~rails-callbacks"
    "app-grape-swagger|https://github.com/ruby-grape/grape-swagger.git|grape,rails-callbacks"
    "app-monica-laravel|https://github.com/monicahq/monica.git|laravel,laravel-lifecycle,phpunit,~symfony"
    "app-symfony-demo|https://github.com/symfony/demo.git|phpunit,symfony"
    "app-wordpress|https://github.com/WordPress/WordPress.git|wordpress"
    "app-codeigniter-starter|https://github.com/codeigniter4/appstarter.git|codeigniter"
    "app-cuda-samples|https://github.com/NVIDIA/cuda-samples.git|cuda,~py-argparse,~py-context-manager,~py-threading,~torch,cpp-callbacks"
    "solidity-openzeppelin|https://github.com/OpenZeppelin/openzeppelin-contracts.git|solidity-public"
    "app-pydantic-core|https://github.com/pydantic/pydantic-core.git|ffi-export,py-unittest,pydantic,rust-std-traits"
    "app-nextcloud-android|https://github.com/nextcloud/android.git|android-activity,android-application,android-broadcastreceiver,android-contentprovider,android-fragment,android-service,android-service-bind,android-service-start,android-worker,compose,java-concurrent,junit,~java-closeable,~py-argparse,dagger,eventbus,jsr330-inject"
    "app-plausible-phoenix|https://github.com/plausible/analytics.git|ecto-migration,ecto-type,express,mix-task,oban,otp,otp-application,phoenix,phoenix-liveview,plug,react,~phoenix-channel"
    "app-hono-examples|https://github.com/honojs/examples.git|hono,cloudflare-workers,~react"
    "app-unity-gamekit|https://github.com/Unity-Technologies/EndlessRunnerSampleGame.git|csharp-disposable,unity"
    # AWS Lambda spans six languages and two registration mechanisms, so
    # it takes three applications to exercise. cdk-examples is the one
    # that matters most: it is the only app in this list where the entry
    # points are named by a *string in the infrastructure code* rather
    # than by anything in the handler's own file.
    "app-powertools-lambda|https://github.com/aws-powertools/powertools-lambda-python.git|aws-cdk,aws-lambda-powertools,py-context-manager,py-threading,pydantic,~py-unittest"
    "app-awslambda-go|https://github.com/aws/aws-lambda-go.git|aws-lambda-go,go-encoding,net-http"
    "app-cdk-examples|https://github.com/aws-samples/aws-cdk-examples.git|aws-cdk,aws-lambda,aws-lambda-go,express,flask,junit,worker-threads,~csharp-disposable,~go-encoding,~java-closeable,~py-argparse"
    # The other clouds. Every one of these was detect-only or absent
    # before 0.6.8, so each entry is the application that proves its
    # rule enumerates rather than merely disclosing a gap.
    "app-gcp-functions|https://github.com/GoogleCloudPlatform/functions-framework-nodejs.git|express,gcp-functions"
    "app-firebase-samples|https://github.com/firebase/functions-samples.git|express,firebase-functions,flask,~py-argparse"
    "app-azure-functions-js|https://github.com/Azure-Samples/functions-quickstart-javascript-azd.git|azure-functions"
    "app-azure-functions-dotnet|https://github.com/Azure-Samples/functions-quickstart-dotnet-azd.git|azure-functions,~aspnet-minimal,~aspnet-mvc,~csharp-disposable,~dotnet-hosting"
    "app-cloudflare-workers|https://github.com/cloudflare/workers-rs.git|axum,cloudflare-workers,ffi-export,rust-std-traits"
    "app-deno-std|https://github.com/denoland/std.git|deno-http,ffi-export,node-event-emitter,~rust-std-traits,~worker-threads"
    "app-quotesbot-scrapy|https://github.com/scrapy/quotesbot.git|scrapy"
    "app-dash-sample-apps|https://github.com/plotly/dash-sample-apps.git|dash,~flask,~py-argparse,~py-threading,~torch"
    "app-shopping-loopback|https://github.com/loopbackio/loopback4-example-shopping.git|express,loopback"
    "app-hertz-examples|https://github.com/cloudwego/hertz-examples.git|hertz,net-http,~go-encoding,~go-runtime,~go-signal,go-testing"
    "app-litestar-fullstack|https://github.com/litestar-org/litestar-fullstack.git|click,litestar,~py-argparse,~py-context-manager,~py-threading,~py-unittest,~pydantic,~react,pytest,~react-hooks,vitest"
    "app-gs-batch-processing|https://github.com/spring-guides/gs-batch-processing.git|junit,spring-batch,~spring"
    "app-metabase-compojure|https://github.com/metabase/metabase.git|angular,compojure,express,node-event-emitter,react,~angular-host,~angular-router,~bun-serve,~clojure-test,~nextjs,~react-hooks,~ring,vitest"
    "kivy-carvera|https://github.com/Carvera-Community/Carvera_Controller.git|kivy,py-threading,uikit-lifecycle,~cocoa-delegate,~py-argparse,~py-unittest,~uikit-appdelegate"
    "app-salvo-examples|https://github.com/salvo-rs/salvo.git|react,rust-std-traits,salvo,~criterion,~react-hooks,~rust-runtime,~serde"
    "app-avalonia-samples|https://github.com/AvaloniaUI/Avalonia.Samples.git|avalonia,csharp-disposable,mvvm-toolkit,~avalonia-xaml,~csharp-runtime,~fsharp-entrypoint,~nunit,xunit"
    "app-monogame-samples|https://github.com/MonoGame/MonoGame.Samples.git|csharp-disposable,monogame,~csharp-runtime"
    "app-godot-demos|https://github.com/godotengine/godot-demo-projects.git|godot,~csharp-disposable,~csharp-runtime,godot-signal,godot-rpc"
    "app-jabref-javafx|https://github.com/JabRef/jabref.git|jakarta-rs,java-closeable,java-concurrent,javafx,junit,swing,~py-argparse,~java-runtime,~java-serviceloader,~junit-params,~kotlin-runtime,~nix,junit-sources,eventbus"
    "app-shuttle-examples|https://github.com/shuttle-hq/shuttle-examples.git|actix-web,axum,rocket,rust-std-traits,salvo,shuttle,~react,~nextjs,~poem,~react-hooks,~rust-runtime,~serde,~warp,rmcp"
    "app-blazor-samples|https://github.com/dotnet/blazor-samples.git|aspnet-minimal,aspnet-mvc,blazor-jsinterop,blazor-lifecycle,csharp-disposable,ef-core,signalr,~dotnet-hosting,~react,~blazor,~csharp-runtime,~maui,~react-hooks,dotnet-ffi-export,razor"
    "app-orleans-samples|https://github.com/dotnet/samples.git|android-activity,aspnet-mvc,csharp-disposable,dotnet-hosting,mstest,orleans,uikit-appdelegate,uikit-lifecycle,~aspnet-minimal,~azure-functions,~blazor-lifecycle,~cocoa-delegate,~signalr,~blazor,~c-runtime,~cpp-runtime,~csharp-runtime,~fsharp-entrypoint,fsharp-nunit,~nunit,~objc-main,~objc-selector,~wcf,~wpf-winforms,xunit,xaml"
    "app-rn-expensify|https://github.com/Expensify/App.git|android-broadcastreceiver,android-jobservice,android-service-bind,android-service-start,react-native,uikit-appdelegate,uikit-lifecycle,~android-activity,android-application,android-service,~cocoa-delegate,~java-closeable,java-concurrent,~react,~bun-serve,~java-runtime,jest,~kotlin-runtime,~objc-main,~objc-selector,~react-hooks,~swiftui"
    "app-serverless-examples|https://github.com/serverless/examples.git|aws-lambda,aws-lambda-go,express,flask,lambda-runtime,mediatr,nestjs,node-event-emitter,sinatra,~csharp-disposable,~go-encoding,~java-closeable,~net-http,~apollo-server,~csharp-runtime,~go-runtime,go-testing,~java-runtime,~playwright,~serde,xunit"
    "app-storm-starter|https://github.com/apache/storm.git|express,jakarta-rs,java-closeable,java-concurrent,junit,py-unittest,servlet,storm,~py-argparse,~py-context-manager,~c-runtime,~clojure-test,~java-runtime,~java-serviceloader,~junit-params,java-serialization"
    "app-symfony-messenger|https://github.com/symfony/symfony.git|phpunit,symfony,symfony-messenger,~wordpress"
    "app-wordpress-android|https://github.com/wordpress-mobile/WordPress-Android.git|android-broadcastreceiver,android-fragment,android-jobservice,android-service,android-service-bind,android-service-start,android-worker,compose,junit,android-activity,android-application,android-contentprovider,~java-closeable,java-concurrent,~android-compose,~java-runtime,~junit-params,~kotlin-runtime,~kotlinx-coroutines,dagger,eventbus,jsr330-inject,room"
    "csharp-mediatr|https://github.com/jbogard/MediatR.git|mediatr,~csharp-disposable,~csharp-runtime,xunit"
    "elixir-phoenix|https://github.com/phoenixframework/phoenix.git|mix-task,otp,otp-application,phoenix,phoenix-channel,phoenix-socket,plug,plug-router,jest"
    "erlang-otp|https://github.com/erlang/otp.git|erlang-gen-server,~java-closeable,~c-runtime,common-test,~cpp-runtime,~erlang-application,~erlang-otp-lifecycle,~java-runtime"
    "groovy-gradle|https://github.com/gradle/gradle.git|android-activity,compose,gradle-plugin,java-closeable,java-concurrent,junit,servlet,~android-fragment,~spring,~android-compose,~c-runtime,~cpp-runtime,gradle-task,gtest,~java-runtime,~java-serviceloader,~junit-params,~kotlin-runtime,~kotlinx-coroutines,~play-mvc,~scalatest,spock,xctest,gradle-model-rules,java-serialization,testng,gradle-services,gradle-task-annotations"
    "haskell-hspec|https://github.com/hspec/hspec.git|hspec,~haskell-main"
    "haskell-pandoc|https://github.com/jgm/pandoc.git|tasty,~haskell-main,~nix,~servant,~wai"
    "js-polka|https://github.com/lukeed/polka.git|express,polka,react,~apollo-server,~fastify,~nextjs,~nuxt,~react-hooks,~socket-io"
    "js-restify|https://github.com/restify/node-restify.git|js-test-globals,node-event-emitter,restify,~socket-io"
    "julia-flux|https://github.com/FluxML/Flux.jl.git|julia-base-dispatch,~julia-module-init,~flux,~julia-test"
    "julia-genie|https://github.com/GenieFramework/Genie.jl.git|genie,julia-base-dispatch,julia-module-init,~http-jl,~julia-test"
    "perl-dancer2|https://github.com/PerlDancer/Dancer2.git|dancer2,~plack,~test-more"
    "perl-mojolicious|https://github.com/mojolicious/mojo.git|mojolicious,~test-more"
    "py-apscheduler|https://github.com/agronholm/apscheduler.git|apscheduler,fastapi,flask,py-context-manager,~py-threading,~py-unittest,~py-atexit,pytest,~starlette"
    "py-dramatiq|https://github.com/Bogdanp/dramatiq.git|apscheduler,dramatiq,py-signal,py-threading,~celery,~py-argparse,~py-context-manager,~py-unittest,py-atexit,~py-setuptools-entrypoints,pytest"
    "py-falcon|https://github.com/falconry/falcon.git|django,falcon,flask,py-context-manager,py-threading,py-unittest,~django-admin,~py-argparse,~py-signal,~bottle,py-atexit,~py-setuptools-entrypoints,pytest"
    "py-huey|https://github.com/coleifer/huey.git|django,django-admin,flask,huey,py-context-manager,py-signal,py-threading,py-unittest,py-atexit,~py-setuptools-entrypoints"
    "py-quart|https://github.com/pallets/quart.git|click,flask,py-context-manager,py-signal,quart,~py-threading,~py-unittest,pytest"
    "py-robyn|https://github.com/sparckles/Robyn.git|actix-actor,ffi-export,py-signal,py-threading,robyn,rust-std-traits,~actix-web,~py-argparse,~py-unittest,~pydantic,~react,~nextjs,pytest,~react-hooks,~rust-runtime,~serde"
    "r-ggplot2|https://github.com/tidyverse/ggplot2.git|r-package-hooks,~r-s3-dispatch,~testthat"
    "rust-ntex|https://github.com/ntex-rs/ntex.git|ntex,rust-std-traits,~rust-runtime,~serde,cargo-test,cargo-bench"
    "rust-rtic|https://github.com/rtic-rs/rtic.git|embedded-rt,rust-std-traits,~clap,~rust-runtime,rtic"
    # Added 2026-09-23 for detect-only rules that no application exercised.
    # One real application per rule, never the framework itself; each was
    # confirmed to be detected by cgg before it was listed here.
    "app-adocasts-adonisjs|https://github.com/adocasts/adocasts.git|~adonisjs"
    "app-casdoor-beego|https://github.com/casdoor/casdoor.git|net-http,react,robfig-cron,~beego"
    "app-golangflow-buffalo|https://github.com/bscott/golangflow.git|~go-encoding,~buffalo"
    "app-passbolt-cakephp|https://github.com/passbolt/passbolt_api.git|phpunit,~cakephp"
    "app-kapua-camel|https://github.com/eclipse-kapua/kapua.git|jakarta-rs,java-closeable,java-concurrent,jpa-lifecycle,junit,servlet,~spring,~camel,cucumber"
    "app-metacpan-catalyst|https://github.com/metacpan/metacpan-web.git|~node-event-emitter,~catalyst"
    "app-ghcli-cobra|https://github.com/cli/cli.git|go-encoding,net-http,~gorilla-mux,~cobra"
    "app-codeedit-combine|https://github.com/CodeEditApp/CodeEdit.git|appkit,cocoa-delegate,uikit-appdelegate,uikit-lifecycle,~combine"
    "app-annif-connexion|https://github.com/NatLibFi/Annif.git|click,flask,torch,~py-context-manager,~py-threading,~py-unittest,~connexion"
    "app-vernemq-cowboy|https://github.com/vernemq/vernemq.git|erlang-gen-server,~cowboy"
    "app-oldata-dagster|https://github.com/mitodl/ol-data-platform.git|py-context-manager,py-threading,pydantic,~py-argparse,~py-unittest,~dagster,marimo"
    "app-marquez-dropwizard|https://github.com/MarquezProject/marquez.git|express,jakarta-rs,java-closeable,java-concurrent,junit,servlet,~py-unittest,~react,~dropwizard"
    "app-opensocial-drupal|https://github.com/goalgorilla/open_social.git|phpunit,symfony,~drupal"
    "app-marktext-electron|https://github.com/marktext/marktext.git|electron,node-event-emitter,~react"
    "app-fsautocomplete-expecto|https://github.com/ionide/FsAutoComplete.git|~expecto"
    "app-betting-expressgateway|https://github.com/johndavedecano/betting-api-starter.git|express,~express-gateway"
    "app-insights-feathers|https://github.com/mariusandra/insights.git|electron,express,react,~feathers"
    "app-darkness-flame|https://github.com/RafaelBarbosatec/darkness_dungeon.git|uikit-appdelegate,uikit-lifecycle,~android-activity,~cocoa-delegate,~flame"
    "app-flinkplatform-flink|https://github.com/zhp8341/flink-streaming-platform-web.git|java-concurrent,junit,spring,spring-jobs,~java-closeable,~servlet,~flink,quartz"
    "app-cardmgmt-giraffe|https://github.com/atsapura/CardManagement.git|~giraffe"
    "app-micromdm-gokit|https://github.com/micromdm/micromdm.git|go-encoding,gorilla-mux,net-http,~go-kit"
    "app-arduinoagent-goa|https://github.com/arduino/arduino-create-agent.git|gin,net-http,~cocoa-delegate,~go-encoding,~py-signal,~uikit-appdelegate,~uikit-lifecycle,~goa"
    "app-concordium-gotham|https://github.com/Concordium/concordium-node.git|ffi-export,rust-std-traits,~hspec,~gotham"
    "app-sdwebui-gradio|https://github.com/AUTOMATIC1111/stable-diffusion-webui.git|fastapi,py-context-manager,py-signal,py-threading,torch,~py-argparse,~pydantic,~gradio"
    "app-streama-grails|https://github.com/streamaserver/streama.git|~grails"
    "app-toiletmap-yoga|https://github.com/public-convenience-ltd/toiletmap.git|~react,~graphql-yoga"
    "app-terminus-hanami|https://github.com/usetrmnl/terminus.git|rails,sidekiq,~hanami"
    "app-aiproxy-hangfire|https://github.com/unfish/AI_Proxy_United.git|aspnet-mvc,ef-core,~aspnet-minimal,~csharp-disposable,~rust-std-traits,~hangfire"
    "app-pa11y-hapi|https://github.com/pa11y/pa11y-webservice.git|~hapi"
    "app-riemann-httpkit|https://github.com/riemann/riemann.git|~http-kit"
    "app-watchlistarr-http4s|https://github.com/nylonee/watchlistarr.git|~http4s"
    "app-kubepi-iris|https://github.com/1Panel-dev/KubePi.git|angular,gorilla-mux,net-http,~angular-host,~angular-router,~go-encoding,~iris"
    "app-wificonnect-iron|https://github.com/balena-os/wifi-connect.git|rust-std-traits,~react,~iron"
    "app-saplib-jenkins|https://github.com/SAP/jenkins-library.git|go-encoding,gorilla-mux,junit,net-http,servlet,spring,~java-closeable,~java-concurrent,~jenkins-pipeline"
    "app-dataverse-jsf|https://github.com/IQSS/dataverse.git|jakarta-rs,java-closeable,java-concurrent,jpa-lifecycle,junit,servlet,~r-package-hooks,~jsf"
    "app-ksql-kafkastreams|https://github.com/confluentinc/ksql.git|java-closeable,java-concurrent,junit,~jakarta-rs,~py-argparse,~kafka-streams"
    "app-yapi-koa|https://github.com/YMFE/yapi.git|react,~koa"
    "app-circuitbreaker-kong|https://github.com/dream11/kong-circuit-breaker.git|busted,kong"
    "app-moon-kratos|https://github.com/aide-family/moon.git|go-encoding,~gorilla-mux,~net-http,~robfig-cron,~kratos"
    "app-kotlinconf-ktor|https://github.com/JetBrains/kotlinconf-app.git|android-broadcastreceiver,compose,junit,android-activity,android-application,~android-fragment,~ktor"
    "app-kori-ktorclient|https://github.com/YangDai2003/Kori.git|compose,junit,uikit-appdelegate,android-activity,android-application,~android-broadcastreceiver,~android-contentprovider,~cocoa-delegate,~uikit-lifecycle,~ktor-client"
    "app-ferry-machinery|https://github.com/lanyulei/ferry.git|gin,go-encoding,~net-http,~machinery"
    "app-elasticsuite-magento|https://github.com/Smile-SA/elasticsuite.git|phpunit,~magento"
    "app-4minitz-meteor|https://github.com/4minitz/4minitz.git|js-test-globals,~meteor"
    "app-wiregui-nicegui|https://github.com/bartei/wiregui.git|fastapi,py-context-manager,py-signal,~py-unittest,~pydantic,~nicegui"
    "app-apisix-openresty|https://github.com/apache/apisix.git|net-http,py-threading,~go-encoding,~java-closeable,~java-concurrent,~openresty"
    "app-zpool-opium|https://github.com/uzh/z-pool-tool.git|~opium"
    "app-xrviz-panel|https://github.com/intake/xrviz.git|~py-context-manager,~panel"
    "app-cljdoc-pedestal|https://github.com/cljdoc/cljdoc.git|~java-closeable,~pedestal"
    "app-importexcel-pester|https://github.com/dfinke/ImportExcel.git|~pester,~powershell-module"
    "app-riffraff-play|https://github.com/guardian/riff-raff.git|~play-module"
    "app-arcos-plumber|https://github.com/wpinvestigative/arcos-api.git|~r-package-hooks,~plumber"
    "app-basedosdados-prefect|https://github.com/basedosdados/pipelines.git|py-context-manager,pydantic,~py-argparse,~py-threading,~py-unittest,~r-package-hooks,~prefect"
    "app-hypothesis-pyramid|https://github.com/hypothesis/h.git|celery,click,~py-argparse,~py-context-manager,~py-unittest,~pyramid"
    "app-lipas-reitit|https://github.com/lipas-liikuntapaikat/lipas.git|~reitit,re-frame,cljs-export,shadow-cljs-hooks"
    "app-wishlist-remix|https://github.com/Hujjat/wishlist-inspire-app.git|~node-event-emitter,~react,~remix"
    "app-mediom-revel|https://github.com/huacnlee/mediom.git|~go-encoding,~net-http,~revel"
    "app-shlink-roadrunner|https://github.com/shlinkio/shlink.git|phpunit,~wordpress,~roadrunner"
    "app-html2rss-roda|https://github.com/html2rss/html2rss-web.git|~roda"
    "app-owllook-sanic|https://github.com/howie6879/owllook.git|~sanic"
    "app-swate-saturn|https://github.com/nfdi4plants/Swate.git|~electron,~react,~saturn"
    "app-realworld-scotty|https://github.com/eckyputrady/haskell-scotty-realworld-example-app.git|hspec,~scotty"
    "app-dartpad-shelf|https://github.com/dart-lang/dart-pad.git|uikit-appdelegate,uikit-lifecycle,~cocoa-delegate,~shelf"
    "app-tweetconf-shiny|https://github.com/gadenbuie/tweet-conf-dash.git|~shiny"
    "app-diaspora-sidekiq|https://github.com/diaspora/diaspora.git|rails,rails-callbacks,sidekiq,~actioncable,~sidekiq-lifecycle"
    "app-grocy-slim|https://github.com/grocy/grocy.git|~slim"
    "app-jobserver-spark|https://github.com/spark-jobserver/spark-jobserver.git|py-unittest,~java-closeable,~spark"
    "app-facto-specs2|https://github.com/nymanjens/facto.git|~specs2"
    "app-bamboobsc-struts|https://github.com/billchen198318/bamboobsc.git|jakarta-rs,java-concurrent,junit,servlet,swing,~hibernate-listeners,~java-closeable,~jpa-lifecycle,~spring,~spring-jobs,~struts,jpa-mapping"
    "app-fssnip-suave|https://github.com/fssnippets/fssnip-website.git|~suave"
    "app-ora-swifttesting|https://github.com/the-ora/browser.git|cocoa-delegate,uikit-appdelegate,uikit-lifecycle,~py-argparse,~swift-testing"
    "app-mineadmin-swoole|https://github.com/mineadmin/MineAdmin.git|hyperf,phpunit,~swoole"
    "app-clashverge-tauri|https://github.com/clash-verge-rev/clash-verge-rev.git|react,rust-std-traits,tauri"
    "app-realworld-tide|https://github.com/colinbankier/realworld-tide.git|rust-std-traits,~tide"
    "app-jupyterhub-tornado|https://github.com/jupyterhub/jupyterhub.git|fastapi,flask,py-signal,pydantic,~py-argparse,~py-context-manager,~py-threading,~py-unittest,~react,~tornado"
    "app-bambufarm-vaadin|https://github.com/TFyre/bambu-farm.git|jakarta-rs,~java-closeable,~java-concurrent,~servlet,~vaadin"
    "app-alarik-vapor|https://github.com/achtungsoftware/alarik.git|vapor,~cocoa-delegate"
    "app-stackage-yesod|https://github.com/commercialhaskell/stackage-server.git|yesod"
    "app-humhub-yii|https://github.com/humhub/humhub.git|node-event-emitter,~phpunit,~yii"
    "app-ghostty-zig|https://github.com/ghostty-org/ghostty.git|uikit-appdelegate,uikit-lifecycle,~cocoa-delegate,~py-argparse,~py-context-manager,~zig-build"
    "app-libxev-zig|https://github.com/mitchellh/libxev.git|~zig-export"
    "app-zls-zig|https://github.com/zigtools/zls.git|~zig-main"
    "app-tigerbeetle-zig|https://github.com/tigerbeetle/tigerbeetle.git|csharp-disposable,java-closeable,java-concurrent,junit,minitest,mstest,py-threading,rust-std-traits,~go-encoding,~zig-test"
    "app-neorv32-vhdl|https://github.com/stnolting/neorv32.git|~vhdl-toplevel"
    "app-fpm-fortran|https://github.com/fortran-lang/fpm.git|~fortran-program"
    "app-leancopilot-lean|https://github.com/lean-dojo/LeanCopilot.git|fastapi,lean-ffi,~cpp-runtime,~pydantic,~torch"
    "app-loogle-lean|https://github.com/nomeata/loogle.git|lean-attributes,lean-ffi,py-threading,py-unittest,~c-runtime,~py-argparse,~py-signal"
    "app-waybar-catch2|https://github.com/Alexays/Waybar.git|py-signal,~py-argparse,~cpp-test-macros"
    "app-kythe-bazel|https://github.com/kythe/kythe.git|gtest,gtest-fixture,go-encoding,java-closeable,junit,net-http,~cocoa-delegate,~java-concurrent,~py-argparse,~py-context-manager,~bazel"
    # 0.14.0 framework audit, native languages (B5).
    "app-opengamepadui-gdext|https://github.com/ShadowBlip/OpenGamepadUI.git|gdext,rust-std-traits,~rust-runtime,~zbus"
    "app-ethertum-bevy|https://github.com/Dreamtowards/Ethertum.git|bevy,cargo-test,rust-std-traits,~rust-runtime,~serde"
    "app-superposition-leptos|https://github.com/juspay/superposition.git|actix-web,cargo-test,express,ffi-export,flake-parts,hspec,java-closeable,java-concurrent,js-test-globals,junit,playwright,py-threading,rust-std-traits,rust-ui-components,~bun-serve,~c-runtime,~clap,~criterion,~go-encoding,~go-runtime,~gradle-task,~haskell-main,~java-runtime,~java-serialization,~kotlin-runtime,~kotlinx-coroutines,~py-context-manager,~py-setuptools-entrypoints,~react,~react-hooks,~rust-runtime,~rust-server-fn,~serde"
    "app-rskey-kani|https://github.com/TheMaxMur/RS-Key.git|cargo-test,embedded-rt,kani,node-test,py-argparse,py-context-manager,py-threading,py-unittest,pytest,rtic,rust-std-traits,~c-runtime,~click,~cloudflare-workers,~criterion,~nix,~py-atexit,~py-signal,~rust-runtime,~serde"
    "app-ferrite-eframe|https://github.com/OlaProeis/Ferrite.git|cargo-test,eframe,rust-std-traits,~clap,~cloudflare-workers,~nix,~py-argparse,~rust-runtime,~serde"
    "app-asusctl-zbus|https://github.com/OpenGamingCollective/asusctl.git|cargo-test,rust-std-traits,zbus,~rust-runtime,~serde"
    "app-robbb-serenity|https://github.com/unixporn/robbb.git|poise,rust-std-traits,serenity,~bash-dispatch,~nix,~rust-runtime,~serde"
    "app-dickgrowerbot-teloxide|https://github.com/kozalosev/DickGrowerBot.git|cargo-test,rust-std-traits,teloxide,~axum,~rust-runtime,~serde,~tonic"
    "app-xfr-hyper|https://github.com/lance0/xfr.git|cargo-test,hyper,rust-std-traits,~clap,~criterion,~nix,~rust-runtime,~serde"
    "app-flameshot-qt|https://github.com/flameshot-org/flameshot.git|flake-parts,qt-widgets,~cmake,~cpp-runtime"
    "app-keepassxc-qttest|https://github.com/keepassxreboot/keepassxc.git|cmake-codecoverage,cocoa-delegate,py-signal,qt-widgets,qtest,~c-runtime,~cmake,~cpp-callbacks,~cpp-runtime,~go-encoding,~go-runtime,~objc-main,~objc-selector,~py-argparse,~uikit-appdelegate,~uikit-lifecycle"
    "app-pdns-boosttest|https://github.com/PowerDNS/pdns.git|boost-test,cargo-test,cpp-callbacks,libfuzzer,py-threading,py-unittest,pytest,rust-std-traits,~c-runtime,~clap,~cpp-runtime,~cpp-test-macros,~hyper,~js-test-globals,~py-argparse,~py-setuptools-entrypoints,~rust-runtime,~serde"
    "app-navigation2-ros2|https://github.com/ros-navigation/navigation2.git|behaviortree-cpp,behaviortree-register,gtest,gtest-fixture,py-threading,py-unittest,pytest,qt-widgets,ros2-rclcpp,~c-runtime,~cmake,~cpp-runtime,~py-argparse,~py-setuptools-entrypoints,~py-signal"
    "app-dexed-juce|https://github.com/asb2m10/dexed.git|juce,juce-plugin,~cpp-runtime"
    "app-tfserving-grpc|https://github.com/tensorflow/serving.git|cpp-callbacks,grpc-cpp,gtest,gtest-fixture,~bazel,~cpp-runtime,~py-argparse,~py-atexit,~py-setuptools-entrypoints,~py-threading"
    "app-meshtastic-arduino|https://github.com/meshtastic/firmware.git|arduino,c-callbacks,cmsis-vectors,cpp-callbacks,esp-idf,freertos,libfuzzer,py-argparse,~c-runtime,~cpp-runtime,~py-threading"
    "app-geany-glib|https://github.com/geany/geany.git|c-callbacks,glib-signals,uikit-lifecycle,~c-runtime,~cocoa-delegate,~cpp-runtime,~csharp-disposable,~csharp-runtime,~django,~go-runtime,~java-closeable,~java-runtime,~jsr250-lifecycle,~julia-base-dispatch,~julia-module-init,~objc-main,~py-argparse,~rust-runtime,~rust-std-traits,~uikit-appdelegate,~vhdl-toplevel"
    "app-zmk-zephyr|https://github.com/zmkfirmware/zmk.git|c-callbacks,zephyr,~c-runtime,~react,~react-hooks"
    "app-redisbloom-redismodule|https://github.com/RedisBloom/RedisBloom.git|c-callbacks,py-unittest,redis-module,~bash-dispatch,~c-runtime,~py-argparse,~py-context-manager,~py-signal"
    "app-sqlitevec-sqlite|https://github.com/asg017/sqlite-vec.git|c-callbacks,cargo-test,libfuzzer,pytest,sqlite-ext,~bun-serve,~c-runtime,~criterion,~go-runtime,~py-argparse,~py-context-manager,~py-unittest,~vue"
    "app-squeezelite-espidf|https://github.com/sle118/squeezelite-esp32.git|c-callbacks,cpp-callbacks,esp-idf,freertos,gtest,js-test-globals,libfuzzer,py-argparse,py-context-manager,py-threading,~c-runtime,~cmake,~cpp-runtime,~cpp-test-macros,~gtest-fixture,~py-setuptools-entrypoints,~rake,~zephyr"
    "app-crazyflie-cmsis|https://github.com/bitcraze/crazyflie-firmware.git|c-callbacks,cmsis-vectors,ffi-export,freertos,glib-signals,py-threading,pytest,qt-widgets,~c-runtime,~cpp-runtime,~py-argparse,~py-setuptools-entrypoints,~rake"
    "app-hoverboard-stm32hal|https://github.com/EFeru/hoverboard-firmware-hack-FOC.git|cmsis-vectors,stm32-hal,~c-runtime"
    "app-maiden-httpz|https://github.com/monome/maiden.git|jest,react,vitest,zig-httpz,~react-hooks,~zig-build,~zig-export,~zig-main,~zig-test,~zig-websocket"
    "app-nullclaw-websocket|https://github.com/nullclaw/nullclaw.git|py-threading,zig-websocket,~zig-build,~zig-export,~zig-main,~zig-test"
    "app-zrythm-cpm|https://github.com/zrythm/zrythm.git|boost-test,c-callbacks,cpm-cmake,cpp-callbacks,csharp-disposable,gtest,gtest-fixture,juce,py-threading,pytest,qt-widgets,~c-runtime,~cmake,~cpp-runtime,~csharp-runtime,~java-closeable,~java-runtime,~juce-plugin,~py-argparse,~qtest"
    "app-fujinet-cmake|https://github.com/FujiNetWIFI/fujinet-firmware.git|arduino,c-callbacks,cmake-codecoverage,cmake-package-init,cpp-callbacks,csharp-disposable,esp-idf,freertos,libfuzzer,nunit-runner,py-threading,py-unittest,~bash-dispatch,~c-runtime,~cmake,~cpp-runtime,~cpp-test-macros,~csharp-runtime,~nunit,~py-argparse,~py-signal,~rake,~zephyr"
    "app-attic-flakeparts|https://github.com/zhaofengli/attic.git|axum,cargo-test,flake-parts,rust-std-traits,~clap,~criterion,~nix,~rust-runtime,~serde"
    "app-lanzaboote-nixostest|https://github.com/nix-community/lanzaboote.git|cargo-test,nixos-test,rust-std-traits,~clap,~nix,~rust-runtime,~serde"
    "app-pdalac-abaqus|https://github.com/ammarkh95/ABAQUS_PDALAC.git|abaqus-user-subroutine"
    "app-mom6-esmf|https://github.com/mom-ocean/MOM6.git|esmf-nuopc,~fortran-program,~py-argparse"
    "app-microwatt-vunit|https://github.com/antonblanchard/microwatt.git|c-callbacks,cpp-callbacks,ffi-export,vunit,~c-runtime,~cpp-runtime,~py-argparse,~py-signal,~rust-runtime,~rust-std-traits,~vhdl-toplevel"
    "app-dvbfpga-vunit|https://github.com/OpenResearchInstitute/dvb_fpga.git|vunit,~py-argparse,~py-threading,~vhdl-toplevel"
    # 0.14.0 Phase C: Rust AI-agent and tool-calling frameworks (B5).
    "app-deepwiki-rig|https://github.com/sopaco/deepwiki-rs.git|cargo-test,rig,rust-std-traits,~clap,~rig-hooks,~rmcp,~rmcp-handlers,~rust-runtime,~serde"
    "app-codegraph-rig|https://github.com/Jakedismo/codegraph-rust.git|axum,cargo-test,py-unittest,rig,rmcp,rust-std-traits,~clap,~criterion,~py-argparse,~py-context-manager,~rig-hooks,~rmcp-handlers,~rust-runtime,~serde"
    "app-ck-rmcp|https://github.com/BeaconBay/ck.git|cargo-test,net-http,node-test,py-unittest,rmcp,rmcp-handlers,rust-std-traits,~clap,~go-runtime,~py-argparse,~py-context-manager,~rust-runtime,~serde"
    "app-qbit-rig|https://github.com/qbit-ai/qbit.git|cargo-test,playwright,react,rmcp-handlers,rust-std-traits,tauri,vitest,~clap,~py-argparse,~react-hooks,~rig,~rig-hooks,~rmcp,~rust-runtime,~serde"
    "app-spacebot-rig|https://github.com/spacedriveapp/spacebot.git|axum,cargo-test,js-test-globals,react,rig,rig-hooks,rmcp-handlers,rust-std-traits,serenity,tauri,~appkit,~clap,~cloudflare-workers,~cocoa-delegate,~nextjs,~nix,~react-hooks,~rmcp,~rust-runtime,~serde,~teloxide,~tokio-cron,~uikit-appdelegate,~uikit-lifecycle"
    "app-orbfirmware-zephyr|https://github.com/worldcoin/orb-firmware.git|py-threading,zephyr,ztest,~c-runtime,~cmake,~nix,~py-argparse"
    # Every corpus directory is an APPS entry, so framework coverage and the
    # manifest sync measure all of them. Claims are measured, not hand-written
    # (scripts/sync-app-manifest.py).
    "app-carvera-firmware-cpp|https://github.com/Carvera-Community/Carvera_Community_Firmware.git|py-signal,py-threading,~py-argparse"
    "app-itunesweb-padrino|https://github.com/sshaw/itunes_store_transporter_web.git|sinatra,~sidekiq"
    "app-katrain-kivy|https://github.com/sanderland/katrain.git|kivy,py-signal,py-threading,~py-unittest"
    "app-lobsters-puma|https://github.com/lobsters/lobsters.git|rails,rails-callbacks,sidekiq,~actioncable"
    "app-mimic-kivy|https://github.com/ISS-Mimic/Mimic.git|kivy,py-signal,py-threading,~py-argparse"
    "app-mtgnode-sails|https://github.com/Yomguithereal/mtgnode.git|"
    "app-networkingdsc-dsc|https://github.com/dsccommunity/NetworkingDsc.git|powershell-dsc"
    "app-ocamlci-alcotest|https://github.com/ocurrent/ocaml-ci.git|"
    "app-ocamlorg-dream|https://github.com/ocaml/ocaml.org.git|"
    "app-opam-cmdliner|https://github.com/ocaml/opam.git|"
    "app-pywallet-kivy|https://github.com/AndreMiras/PyWallet.git|kivy,py-unittest,~py-threading"
    "app-quack-oak|https://github.com/raaymax/quack.git|android-broadcastreceiver,android-service-bind,android-service-start,junit,node-event-emitter,~android-activity,android-service,~react"
    "app-rebar3-eunit|https://github.com/erlang/rebar3.git|erlang-gen-server,eunit,rebar3-provider"
    "app-smoothie-kivy|https://github.com/wolfmanjm/kivy-smoothie-host.git|kivy,py-signal,py-threading,~py-argparse"
    "app-smoothieware-cpp|https://github.com/Smoothieware/Smoothieware.git|py-signal,py-threading,~py-argparse"
    "app-snlquest-kivy|https://github.com/sandialabs/snl-quest.git|kivy,py-threading,~py-context-manager"
    "app-unity-boat|https://github.com/Unity-Technologies/BoatAttack.git|csharp-disposable,unity,unity-editor"
    "app-zenplayer-kivy|https://github.com/Zen-CODE/zenplayer.git|flask,kivy,py-threading,pydantic"
    "asm-xv6|https://github.com/mit-pdos/xv6-public.git|"
    "asyncapi-spec|https://github.com/asyncapi/spec.git|"
    "bash-acme|https://github.com/acmesh-official/acme.sh.git|"
    "c-jq|https://github.com/jqlang/jq.git|libfuzzer"
    "c-redis|https://github.com/redis/redis.git|py-threading,~py-argparse,~py-signal,c-callbacks"
    "clojure-compojure|https://github.com/weavejester/compojure.git|compojure"
    "clojure-ring|https://github.com/ring-clojure/ring.git|"
    "cloudflare-worker-examples|https://github.com/cloudflare/worker-template.git|"
    "cmake-kitware|https://github.com/Kitware/CMake.git|android-activity,cocoa-delegate,cuda,ffi-export,py-threading,py-unittest,uikit-appdelegate,uikit-lifecycle,~csharp-disposable,~java-closeable,~py-argparse,~rust-std-traits"
    "cpp-nlohmann-json|https://github.com/nlohmann/json.git|libfuzzer,py-threading,~py-argparse,~py-context-manager"
    "cpp-spdlog|https://github.com/gabime/spdlog.git|"
    "csharp-newtonsoft|https://github.com/JamesNK/Newtonsoft.Json.git|csharp-disposable,dotnet-serialization,nunit-runner,benchmarkdotnet"
    "csharp-serilog|https://github.com/serilog/serilog.git|csharp-disposable,xunit-data"
    "dart-flame|https://github.com/flame-engine/flame.git|~py-signal"
    "dart-flutter|https://github.com/flutter/flutter.git|android-activity,android-fragment,cocoa-delegate,compose,java-closeable,java-concurrent,junit,py-unittest,uikit-appdelegate,uikit-lifecycle,android-application,android-broadcastreceiver,~android-contentprovider,~android-service,~android-service-bind,~android-service-start,~py-argparse,~py-signal,~py-threading"
    "fortran-stdlib|https://github.com/fortran-lang/stdlib.git|~py-argparse"
    "fsharp-paket|https://github.com/fsprojects/Paket.git|csharp-disposable"
    "go-caddy|https://github.com/caddyserver/caddy.git|go-encoding,net-http"
    "go-fzf|https://github.com/junegunn/fzf.git|minitest,~go-encoding"
    "go-hertz|https://github.com/cloudwego/hertz.git|go-encoding,hertz,net-http"
    "go-martini|https://github.com/go-martini/martini.git|net-http"
    "graphql-github|https://github.com/octokit/graphql-schema.git|"
    "hcl-terraform-aws|https://github.com/hashicorp/terraform-provider-aws.git|go-encoding,~net-http,~py-argparse,terraform-plugin-framework"
    "hcl-vpc|https://github.com/terraform-aws-modules/terraform-aws-vpc.git|"
    "java-gson|https://github.com/google/gson.git|java-closeable,junit,java-concurrent,caliper"
    "java-okhttp|https://github.com/square/okhttp.git|java-closeable,junit,android-activity,android-application,java-concurrent,moshi"
    "java-spring-batch|https://github.com/spring-projects/spring-batch.git|java-concurrent,junit,spring-batch,spring-jobs,~java-closeable,~jpa-lifecycle,~spring-messaging"
    "js-express|https://github.com/expressjs/express.git|"
    "js-lodash|https://github.com/lodash/lodash.git|"
    "js-loopback|https://github.com/loopbackio/loopback-next.git|express,loopback,mocha,node-event-emitter"
    "kivy-kivymd|https://github.com/kivymd/KivyMD.git|kivy,~py-argparse"
    "kotlin-okio|https://github.com/square/okio.git|junit,java-closeable,java-concurrent"
    "lua-kong|https://github.com/Kong/kong.git|busted,kong,rust-std-traits,~py-argparse"
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
    "rust-bat|https://github.com/sharkdp/bat.git|rust-std-traits,uikit-lifecycle,~cocoa-delegate,~java-closeable,~py-argparse,~py-context-manager,~py-threading,~react,~uikit-appdelegate,solidity-public"
    "rust-ripgrep|https://github.com/BurntSushi/ripgrep.git|rust-std-traits,~py-argparse,~py-threading"
    "scala-play|https://github.com/playframework/playframework.git|java-closeable,junit,~hibernate-listeners,~java-concurrent,~jpa-lifecycle"
    "scala-spark|https://github.com/apache/spark.git|java-closeable,java-concurrent,junit,py-context-manager,py-signal,py-threading,py-unittest,r-package-hooks,servlet,torch,~jakarta-rs,~py-argparse"
    "smithy-protocol-tests|https://github.com/smithy-lang/smithy.git|java-closeable,java-concurrent,junit,py-unittest,~py-argparse,~py-context-manager,jmh"
    "starlark-skylib|https://github.com/bazelbuild/bazel-skylib.git|"
    "swift-alamofire|https://github.com/Alamofire/Alamofire.git|cocoa-delegate,uikit-appdelegate,uikit-lifecycle"
    "ts-typeorm|https://github.com/typeorm/typeorm.git|mocha,node-event-emitter,~react"
    "ts-zod|https://github.com/colinhacks/zod.git|vitest,~react"
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
    "app-domainwatchdog-symfony|https://github.com/maelgangloff/domain-watchdog.git|api-platform,doctrine-lifecycle,phpunit,symfony,~react,~symfony-messenger"
    "app-edrys-oak|https://github.com/edrys-org/edrys.git|~deno-http"
    "app-hadolint-hspec|https://github.com/hadolint/hadolint.git|hspec"
    "app-libchecker-gradle|https://github.com/LibChecker/LibChecker.git|android-broadcastreceiver,android-service-bind,android-service-start,java-closeable,junit,py-unittest,android-activity,android-application,~android-contentprovider,android-fragment,android-service,~py-argparse"
    "app-nanocl-ntex|https://github.com/next-hat/nanocl.git|ntex,rust-std-traits"
    "app-newscrawl-storm|https://github.com/commoncrawl/news-crawl.git|junit,storm,~java-closeable"
    "app-oncall-falcon|https://github.com/linkedin/oncall.git|falcon"
    "app-papercups-phoenix|https://github.com/papercups-io/papercups.git|ecto-migration,mix-task,oban,otp,otp-application,phoenix,phoenix-channel,phoenix-socket,plug,react"
    "app-perlmaven-dancer2|https://github.com/szabgab/Perl-Maven.git|dancer2"
    # 0.14.0 scripting hosts (B6): real applications of each new row.
    "app-chocolateypackages-au|https://github.com/chocolatey-community/chocolatey-packages.git|chocolatey-au"
    "app-julialang-franklin|https://github.com/JuliaLang/www.julialang.org.git|franklin,playwright,~js-test-globals,~julia-base-dispatch,~julia-module-init,~react,~react-hooks"
    "app-openqa-minion|https://github.com/os-autoinst/openQA.git|click,minion,moose-lifecycle,mojolicious,py-argparse,~bash-dispatch,~test-more"
    "app-pluto-httpjl|https://github.com/fonsp/Pluto.jl.git|http-jl,jest,julia-base-dispatch,julia-module-init,~julia-test"
    "app-scriptsirssi-irssi|https://github.com/irssi/scripts.irssi.org.git|irssi"
    "app-wastewater-targets|https://github.com/CDCgov/wastewater-informed-covid-forecasting.git|~py-argparse,~r-package-hooks,~r-s3-dispatch,~targets,~testthat"
    "app-pixlserv-martini|https://github.com/MilanMisak/pixlserv.git|martini,~go-encoding,~net-http"
    "app-ravada-mojolicious|https://github.com/UPC/ravada.git|mojolicious"
    "app-reggie-restify|https://github.com/mbrevoort/node-reggie.git|restify"
    "app-socialhome-dramatiq|https://github.com/jaywink/socialhome.git|django,django-admin,dramatiq,py-unittest,~apscheduler"
    "app-workerstunnel-cloudflare|https://github.com/zhu327/workers-tunnel.git|cloudflare-workers,~rust-std-traits"
    # 0.14.0 Python application frameworks (framework audit 2026-11-25).
    "app-ucupaint-blender|https://github.com/ucupumar/ucupaint.git|blender-addon,blender-addon-register,py-threading,py-unittest"
    "app-qutebrowser-qt|https://github.com/qutebrowser/qutebrowser.git|flask,py-context-manager,py-signal,py-threading,py-unittest,pytest,pytest-bdd,qt,~py-argparse,~py-atexit,~py-setuptools-entrypoints"
    "app-harlequin-textual|https://github.com/tconbeer/harlequin.git|click,py-atexit,py-context-manager,py-signal,py-threading,pytest,textual,~py-argparse,~py-unittest"
    "app-proxmoxve-homeassistant|https://github.com/dougiteixeira/proxmoxve.git|home-assistant,home-assistant-entity,pytest,~py-context-manager,~py-threading,~py-unittest"
    "app-oca-project-odoo|https://github.com/OCA/project.git|odoo,odoo-model,~py-unittest"
    "app-pgmigrate-behave|https://github.com/yandex/pgmigrate.git|behave,~py-argparse,~py-context-manager,~py-setuptools-entrypoints,~py-threading"
    "app-eurorack-pmod-cocotb|https://github.com/apfaudio/eurorack-pmod.git|cocotb,~py-argparse"
    "app-tracecat-temporal|https://github.com/TracecatHQ/tracecat.git|click,fastapi,js-test-globals,node-event-emitter,playwright,py-context-manager,py-signal,py-threading,pydantic,pytest,react,sqlalchemy-events,temporal-python"
    "app-mlserver-grpc|https://github.com/SeldonIO/MLServer.git|click,fastapi,grpc-python,py-context-manager,py-signal,pydantic,pytest,torch,~aiohttp,~py-argparse,~py-threading,~py-unittest"
    "app-pydis-bot-discord|https://github.com/python-discord/bot.git|discord-py,discord-py-extension,py-context-manager,py-unittest,pydantic,~aiohttp,~py-signal"
    "app-revel-backend-ninja|https://github.com/letsrevel/revel-backend.git|celery,django,django-admin,django-hooks,django-lifecycle,django-ninja,py-context-manager,py-signal,py-threading,py-unittest,pydantic,pytest,~py-argparse"
    # 0.14.0 Python AI-agent and tool-calling SDKs (agent research 2026-11-25).
    "app-hexstrike-mcp|https://github.com/0x4m4/hexstrike-ai.git|flask,mcp-python,py-threading,~aiohttp,~py-argparse,~py-signal"
    "app-storm-dspy|https://github.com/stanford-oval/storm.git|dspy,~dspy-tools,~py-argparse,~py-context-manager,~py-setuptools-entrypoints,~py-threading,~streamlit"
    "app-compounding-eng-dspy|https://github.com/Strategic-Automation/dspy-compounding-engineering.git|click,dspy,dspy-tools,mcp-python,py-threading,py-unittest,pytest,~py-argparse,~py-context-manager,~pydantic"
    "app-deerflow-langchain|https://github.com/bytedance/deer-flow.git|discord-py,discord-py-extension,fastapi,js-test-globals,langchain-python,langchain-python-middleware,langgraph-python,langgraph-python-tools,mcp-python,node-event-emitter,node-test,playwright,py-argparse,py-atexit,py-context-manager,py-signal,py-threading,py-unittest,pydantic,pytest,react,sqlalchemy-events,textual,~nextjs,~react,~react-hooks"
    "app-gpt-researcher-langgraph|https://github.com/assafelovic/gpt-researcher.git|express,fastapi,langchain-python,langgraph-python,node-test,py-context-manager,py-unittest,pytest,~aiohttp,~click,~langchain-python-middleware,~langgraph-python-tools,~nextjs,~py-argparse,~py-threading,~pydantic,~react,~react-hooks"
    "app-mobilerun-llamaindex|https://github.com/droidrun/mobilerun.git|click,llamaindex-workflows,py-context-manager,py-threading,py-unittest,pytest,~llamaindex-tools,~mcp-python,~py-setuptools-entrypoints,~pydantic"
    "app-pygpt-llamaindex|https://github.com/szczyglis-dev/py-gpt.git|llamaindex-tools,llamaindex-workflows,node-test,openai-agents-python,py-context-manager,py-signal,py-threading,pydantic,pytest,qt,~langchain-python,~langchain-python-middleware,~mcp-python,~openai-agents-python-hooks,~py-argparse,~py-unittest,~sqlalchemy-events,~torch"
    "app-gmail-automation-crewai|https://github.com/tonykipkemboi/crewai-gmail-automation.git|crewai-project,crewai-tools,~crewai-flow,~pydantic"
    "app-jobapplication-crewai|https://github.com/rafaelcalassara/JobApplicationFlow.git|crewai-flow,crewai-project,crewai-tools,~gradio"
    "app-crewai-studio-crewai|https://github.com/strnad/CrewAI-Studio.git|crewai-tools,py-threading,pytest,~crewai-flow,~crewai-project,~langchain-python,~pydantic,~sqlalchemy-events,~streamlit"
    "app-wazuh-autopilot-strands|https://github.com/gensecaihq/Wazuh-Autopilot.git|fastapi,mcp-python,py-threading,pytest,sqlalchemy-events,strands-python,strands-python-hooks,~py-context-manager,~pydantic,~react,~react-hooks"
    "app-loom-agentcore|https://github.com/awslabs/loom.git|bedrock-agentcore-python,fastapi,node-test,py-context-manager,py-threading,py-unittest,pydantic,pytest,react,sqlalchemy-events,strands-python,strands-python-hooks,~a2a-python,~mcp-python,~py-argparse,~react-hooks"
    "app-hr-breaker-pydanticai|https://github.com/btseytlin/hr-breaker.git|click,fastapi,py-context-manager,py-threading,pydantic,pydantic-ai,pydantic-ai-hooks,pytest,~py-unittest"
    "app-ai-doc-gen-pydanticai|https://github.com/divar-ir/ai-doc-gen.git|pydantic,pydantic-ai,~py-argparse,~pydantic-ai-hooks"
    "app-agentic-devops-openai|https://github.com/agenticsorg/devops.git|openai-agents-python,openai-agents-python-hooks,py-unittest,pytest,vitest,~py-argparse,~py-setuptools-entrypoints,~pydantic,~react,~react-hooks"
    "app-tenk-openai|https://github.com/ralliesai/tenk.git|openai-agents-python,py-threading,~openai-agents-python-hooks"
    "app-nanoclaw-claude|https://github.com/ApeCodeAI/nanoclaw-py.git|apscheduler,claude-agent-sdk-python,claude-agent-sdk-python-hooks,py-unittest"
    "app-zen7-payment-a2a|https://github.com/Zen7-Labs/Zen7-Payment-Agent.git|a2a-python,click,fastapi,mcp-python,~azure-functions,~py-context-manager,~pydantic,~sqlalchemy-events"
    # 0.14.0 framework audit: JS/TS, Swift, Objective-C and Dart application
    # framework rows (measured claims).
    "app-heroku-oclif|https://github.com/heroku/cli.git|mocha,node-event-emitter,oclif,~js-test-globals"
    "app-convertx-elysia|https://github.com/C4illin/ConvertX.git|elysia,js-test-globals,~bun-serve"
    "app-retraced-tsoa|https://github.com/retracedhq/retraced.git|express,mocha,tsoa"
    "app-wiredoor-routingcontrollers|https://github.com/wiredoor/wiredoor.git|express,jest,routing-controllers,~typeorm-listeners,~typeorm-subscribers,~vue"
    "app-standardnotes-inversify|https://github.com/standardnotes/server.git|express,inversify-express-utils,js-test-globals,node-event-emitter,~jest,~typeorm-listeners,~typeorm-subscribers"
    "app-waifuvault-tsed|https://github.com/waifuvault/WaifuVault.git|express,fiber,go-testing,node-event-emitter,tsed,vitest,~go-encoding,~go-runtime,~go-signal,~net-http,~nextjs,~react,~react-hooks,~socket-io,~typeorm-listeners,~typeorm-subscribers,~worker-threads"
    "app-certd-midway|https://github.com/certd/certd.git|jest,midway,mocha,node-event-emitter,node-test,vitest,~bash-dispatch,~go-encoding,~go-runtime,~java-closeable,~java-runtime,~java-serialization,~koa,~net-http,~typeorm-listeners,~typeorm-subscribers,~vue"
    "app-cromwell-typegraphql|https://github.com/CromwellCMS/Cromwell.git|jest,nestjs,react,type-graphql,~apollo-server,~fastify,~nextjs,~react-hooks,~typeorm-listeners,~typeorm-subscribers,~worker-threads"
    "app-outline-sequelize|https://github.com/outline/outline.git|bullmq,express,node-event-emitter,react,sequelize-typescript,vitest,~koa,~react-hooks,~socket-io"
    "app-kresus-typeorm|https://github.com/kresusapp/kresus.git|js-test-globals,node-event-emitter,react,typeorm-subscribers,~express,~py-argparse,~react-hooks,~typeorm-listeners"
    "app-shoot-typeorm|https://github.com/MaddyUnderStars/shoot.git|bullmq,express,node-event-emitter,typeorm-listeners,vitest,~typeorm-subscribers"
    "app-workadventure-phaser|https://github.com/workadventure/workadventure.git|electron,express,node-event-emitter,node-test,phaser,playwright,vitest,~svelte"
    "app-dataview-obsidian|https://github.com/blacksmithgu/obsidian-dataview.git|js-test-globals,node-event-emitter,obsidian,~jest"
    "app-homeassistant-lit|https://github.com/home-assistant/frontend.git|lit,node-event-emitter,playwright,vitest,~worker-threads"
    "app-container-argparser|https://github.com/apple/container.git|swift-argument-parser,~cocoa-delegate,~swift-testing"
    "app-spi-vapor|https://github.com/SwiftPackageIndex/SwiftPackageIndex-Server.git|fluent,vapor,vapor-commands,~cocoa-delegate,~swift-testing"
    "app-icecubes-widgetkit|https://github.com/Dimillian/IceCubesApp.git|appintents,apple-app-extension,cocoa-delegate,uikit-appdelegate,uikit-lifecycle,widgetkit,xctest,~combine,~swift-testing,~swiftui"
    "app-rocketchat-reactnative|https://github.com/RocketChat/Rocket.Chat.ReactNative.git|apple-app-extension,cocoa-delegate,react-native,uikit-appdelegate,uikit-lifecycle,~combine,~core-data,~jest,~objc-main,~objc-selector,~react,~react-hooks,~swiftui"
    "app-sequelace-appkit|https://github.com/Sequel-Ace/Sequel-Ace.git|appkit,cocoa-delegate,minitest,uikit-appdelegate,uikit-lifecycle,xctest,~apple-app-extension,~c-runtime,~combine,~objc-main,~objc-selector,~rake,~swiftui"
    "app-simplenote-coredata|https://github.com/Automattic/simplenote-ios.git|apple-app-extension,cocoa-delegate,core-data,uikit-appdelegate,uikit-lifecycle,widgetkit,xctest,~c-runtime,~objc-main,~objc-selector,~rake,~swiftui"
    "app-thunder-equatable|https://github.com/thunder-app/thunder.git|android-activity,equatable,uikit-appdelegate,xctest,~apple-app-extension,~cocoa-delegate,~dart-main,~flutter,~flutter-test,~js-test-globals,~py-argparse,~uikit-lifecycle"
    "app-piliplus-getx|https://github.com/bggRGjQaUbCoE/PiliPlus.git|android-contentprovider,getx,uikit-appdelegate,uikit-lifecycle,~android-activity,~appkit,~cmake,~cocoa-delegate,~cpp-runtime,~dart-main,~flutter,~flutter-test,~java-closeable,~java-runtime,~kotlin-runtime"
    "app-lichess-riverpod|https://github.com/lichess-org/mobile.git|android-activity,android-broadcastreceiver,riverpod,uikit-appdelegate,uikit-lifecycle,widgetkit,~appintents,~apple-app-extension,~cmake,~cocoa-delegate,~dart-main,~flutter,~flutter-test,~java-closeable,~java-concurrent,~kotlin-runtime,~swiftui"
    "app-skybridge-dartfrog|https://github.com/videah/SkyBridge.git|dart-frog,~dart-main"
    "app-fvm-args|https://github.com/leoafarias/fvm.git|dart-args,py-unittest,~dart-main,~flutter-test,~nextjs,~py-argparse,~py-signal,~react,~react-hooks"
    "app-vernissage-vaporqueues|https://github.com/VernissageApp/VernissageServer.git|fluent,vapor,vapor-queues,~cocoa-delegate,~swift-testing,~vapor-commands"
    "app-mcmmo-bukkit|https://github.com/mcMMO-Dev/mcMMO.git|bukkit,java-closeable,java-concurrent,java-serialization,junit,junit-sources"
    "app-sodium-mixin|https://github.com/CaffeineMC/sodium.git|java-closeable,java-concurrent,mixin,swing"
    "app-create-neoforge|https://github.com/Creators-of-Create/Create.git|forge-events,java-closeable,mixin"
    "app-shatteredpd-libgdx|https://github.com/00-Evan/shattered-pixel-dungeon.git|android-activity,libgdx"
    "app-tjbot-jda|https://github.com/Together-Java/TJ-Bot.git|jackson,java-closeable,jda,junit,junit-sources"
    "app-ruoyivuepro-flowable|https://github.com/YunaiV/ruoyi-vue-pro.git|bpmn-delegates,fault-tolerance,jackson,java-closeable,java-concurrent,jsr250-lifecycle,junit,junit-sources,py-unittest,quartz,rocketmq,servlet,spring,spring-context,spring-jobs,spring-messaging,spring-websocket"
    "app-austin-xxljob|https://github.com/ZhongFuCheng3y/austin.git|eventbus,flink,java-concurrent,jsr250-lifecycle,junit,rocketmq,spring,spring-context,spring-messaging,xxl-job"
    "app-gobblin-temporal|https://github.com/apache/gobblin.git|click,eventbus,gradle-plugin,gradle-task-annotations,guice,jackson,java-closeable,java-concurrent,java-serialization,jmh,jsr330-inject,junit,quartz,temporal,testng"
    "app-cassandre-dgs|https://github.com/cassandre-tech/cassandre-trading-bot.git|dgs,jsr250-lifecycle,junit,spring-context,spring-jobs"
    "app-translationplugin-intellij|https://github.com/YiiGuxing/TranslationPlugin.git|intellij-platform,java-closeable,java-concurrent,junit,swing"
    "app-mihon-android|https://github.com/mihonapp/mihon.git|android-activity,android-application,android-broadcastreceiver,android-service,android-service-bind,android-service-start,android-worker,androidx-paging,compose,gradle-task,java-closeable,junit,junit-sources,kotlinx-serialization"
    "app-nussknacker-flink|https://github.com/TouK/nussknacker.git|express,flink,java-serialization,slick"
    "app-webjars-zio|https://github.com/webjars/webjars.git|zio-test"
    "app-gitbucket-slick|https://github.com/gitbucket/gitbucket.git|slick"
    "app-scastie-scalajs|https://github.com/scalacenter/scastie.git|scala-main,scalajs-export"
    "app-openboxes-grails|https://github.com/openboxes/openboxes.git|grails-quartz,js-test-globals,node-event-emitter,react,spock"
    "app-inovelli-hubitat|https://github.com/InovelliUSA/Hubitat.git|hubitat"
    "app-repath-reframe|https://github.com/repath-studio/repath-studio.git|cljs-export,electron,junit,re-frame,shadow-cljs-hooks,uikit-appdelegate"
    "app-scavenger-grpc|https://github.com/naver/scavenger.git|apscheduler,django,grpc-kotlin,java-closeable,java-concurrent,jmh,junit-sources,py-threading,py-unittest,spring,spring-context"
    "app-utaformatix-kotlinjs|https://github.com/sdercolin/utaformatix3.git|junit,kotlin-js-export,kotlinx-serialization"
    "app-multiplayer-harmony|https://github.com/rwmt/Multiplayer.git|csharp-disposable,harmony,unity,~csharp-runtime,~dotnet-serialization,~unity-editor"
    "app-orchard-reqnroll|https://github.com/OrchardCMS/Orchard.git|csharp-disposable,nunit-runner,reqnroll,~c-runtime,~csharp-runtime,~dotnet-serialization,~js-test-globals,~nunit,~wcf"
    "app-openkh-mcmaster|https://github.com/OpenKH/OpenKh.git|csharp-disposable,mcmaster-cli,monogame,xaml,xunit,xunit-data,~cpp-runtime,~csharp-runtime,~dotnet-serialization,~mvvm-toolkit,~wpf-winforms"
    "app-azurecostcli-spectre|https://github.com/mivano/azure-cost-cli.git|csharp-disposable,spectre-cli,xunit,~csharp-runtime,~xunit-data"
    "app-files-roslyn|https://github.com/files-community/Files.git|csharp-disposable,dotnet-ffi-export,mstest,mvvm-toolkit,roslyn,xaml,~csharp-runtime"
    "app-aptabase-fluentmigrator|https://github.com/aptabase/aptabase.git|aspnet-mvc,csharp-disposable,dotnet-hosting,fluentmigrator,vitest,xunit,~aspnet-minimal,~csharp-runtime,~react,~react-hooks,~xunit-data"
    "app-nexusforever-rebus|https://github.com/NexusForever/NexusForever.git|csharp-disposable,dotnet-hosting,ef-core,quartz,rebus,~aspnet-minimal,~aspnet-mvc,~csharp-runtime,~dotnet-serialization,~roslyn"
    "app-dascommitments-nservicebus|https://github.com/SkillsFundingAgency/das-commitments.git|aspnet-mvc,csharp-disposable,ef-core,fluentvalidation,mediatr,nservicebus,nunit-runner,~aspnet-minimal,~azure-functions,~csharp-runtime,~dotnet-hosting,~dotnet-serialization,~nunit"
    "app-iotsharp-cap|https://github.com/IoTSharp/IoTSharp.git|aspnet-mvc,benchmarkdotnet,csharp-disposable,dotnet-hosting,dotnetcore-cap,ef-core,playwright,quartz,xunit,xunit-data,~aspnet-minimal,~csharp-runtime,~dotnet-serialization,~py-argparse,~react,~react-hooks,~roslyn,~vue"
    "app-reaparr-fastendpoints|https://github.com/Reaparr/Reaparr.git|csharp-disposable,dotnet-hosting,ef-core,fastendpoints,fluentvalidation,quartz,signalr,spectre-cli,vitest,~aspnet-minimal,~aspnet-mvc,~csharp-runtime,~dotnet-serialization,~nuxt,~vue"
    "app-abpnextadmin-abp|https://github.com/colinin/abp-next-admin.git|abp,abp-appservice,aspnet-mvc,csharp-disposable,dotnet-hosting,ef-core,fluentvalidation,mediatr,playwright,quartz,razor,vitest,xaml,xunit,~android-activity,~aspnet-minimal,~blazor,~blazor-lifecycle,~cmake,~cpp-runtime,~csharp-runtime,~dart-main,~dotnet-serialization,~dotnetcore-cap,~flutter,~flutter-test,~grpc-dotnet,~hangfire,~jest,~js-test-globals,~koa,~maui,~powershell-module,~react,~react-hooks,~signalr,~vue,~xunit-data"
    "app-neo-akkanet|https://github.com/neo-project/neo.git|akka-net,benchmarkdotnet,csharp-disposable,mstest,~csharp-runtime,~dotnet-serialization"
    "app-grace-orleans|https://github.com/ScottArbeit/Grace.git|cargo-test,csharp-disposable,fsharp-nunit,node-test,nunit-runner,orleans,~aspnet-minimal,~aspnet-mvc,~csharp-runtime,~fsharp-entrypoint,~giraffe,~nunit"
    "app-fantomastools-lambda|https://github.com/nojaf/fantomas-tools.git|aws-lambda,~fsharp-entrypoint,~suave,~vitest"
    "app-crossplane-controllerruntime|https://github.com/crossplane/crossplane.git|controller-runtime,go-testing,prometheus-collector,~go-encoding,~go-runtime,~grpc-go,~net-http,~nix,~robfig-cron"
    "app-ghdash-bubbletea|https://github.com/dlvhdr/gh-dash.git|bubbletea,go-encoding,go-testing,~astro,~cobra,~go-runtime,~net-http"
    "app-openmeter-watermill|https://github.com/openmeterio/openmeter.git|chi,controller-runtime,ent-schema,go-encoding,go-testing,net-http,node-test,py-context-manager,py-unittest,sarama,testify-suite,vitest,watermill,~cobra,~go-runtime,~go-signal,~grpc-go,~nix,~node-event-emitter,~prometheus-collector,~py-argparse,~react,~react-hooks,~robfig-cron,~urfave-cli"
    "app-gochoria-nats|https://github.com/choria-io/go-choria.git|go-encoding,go-testing,nats,net-http,~go-runtime,~go-signal,~prometheus-collector,~robfig-cron"
    "app-automuteus-discordgo|https://github.com/automuteus/automuteus.git|bubbletea,discordgo,gin,go-testing,gorilla-mux,net-http,~go-encoding,~go-runtime,~go-signal,~prometheus-collector"
    "app-nsq-httprouter|https://github.com/nsqio/nsq.git|go-encoding,go-testing,httprouter,net-http,~go-runtime,~go-signal,~tornado"
    "app-kubesphere-gorestful|https://github.com/kubesphere/kubesphere.git|controller-runtime,go-encoding,go-restful,go-testing,net-http,robfig-cron,~cobra,~go-runtime,~go-signal,~gorilla-mux,~prometheus-collector,~py-argparse"
    # 0.14.0: applications for the Lua and Bash host rules (2026-11-25).
    "app-cachyos-makepkg|https://github.com/CachyOS/CachyOS-PKGBUILDS.git|makepkg,mkinitcpio,~py-argparse"
    "app-capturetheflag-luanti|https://github.com/MT-CTF/capturetheflag.git|luanti"
    "app-dokku-bats|https://github.com/dokku/dokku.git|bats,express,flask,go-encoding,go-testing,net-http,servlet,sinatra,~bash-dispatch,~compojure,~django,~go-runtime,~go-signal,~grpc-go,~java-closeable,~java-runtime,~java-serialization,~py-argparse,~ring,~robfig-cron"
    "app-koreader-busted|https://github.com/koreader/koreader.git|busted,~nix,~py-argparse,~py-context-manager"
    "app-kong-oidc-kong|https://github.com/nokia/kong-oidc.git|busted,kong,~openresty"
    "app-oil-neovim|https://github.com/stevearc/oil.nvim.git|busted,neovim-plugin,~py-argparse"
    "app-pyenv-bats|https://github.com/pyenv/pyenv.git|bats,~bash-dispatch,~c-runtime,~py-argparse"
    "app-snkrx-love2d|https://github.com/a327ex/SNKRX.git|love2d"
    "app-yq-shunit2|https://github.com/mikefarah/yq.git|go-encoding,go-testing,shunit2,~cobra,~go-runtime"
    # 0.14.0: applications for the Ruby and PHP rules (2026-11-25).
    "app-rubygems-rails|https://github.com/rubygems/rubygems.org.git|active-model-validators,ar-migration,minitest,pundit,rails,rails-callbacks,sidekiq,view-component"
    "app-chaskiq-graphql|https://github.com/chaskiq/chaskiq.git|actioncable,active-model-validators,ar-migration,graphql-ruby,js-test-globals,pundit,rails,rails-callbacks,react,sidekiq"
    "app-kamal-thor|https://github.com/basecamp/kamal.git|minitest,thor"
    "app-casa-pundit|https://github.com/rubyforgood/casa.git|active-model-validators,ar-migration,jest,pundit,rails,rails-callbacks,view-component"
    "app-coolify-livewire|https://github.com/coollabsio/coolify.git|laravel,laravel-lifecycle,livewire,node-test,pest,phpunit,playwright"
    "app-pelican-filament|https://github.com/pelican/panel.git|filament,laravel,laravel-lifecycle,livewire,pest,phpunit"
    "app-cospend-nextcloud|https://github.com/julien-nc/cospend-nc.git|nextcloud,nextcloud-lifecycle,phpunit"
    "app-partdb-apiplatform|https://github.com/Part-DB/Part-DB-server.git|api-platform,doctrine-lifecycle,phpunit,symfony"
    # 0.14.0 phase C: Ruby and PHP AI tool frameworks (2026-11-25).
    "app-rubyevents-rubyllm|https://github.com/rubyevents/rubyevents.git|ar-migration,minitest,rails,rails-callbacks,ruby-llm,sidekiq,thor,view-component,~actioncable,~active-model-validators,~puma"
    "app-canine-mcpruby|https://github.com/CanineHQ/canine.git|actioncable,ar-migration,mcp-ruby,pundit,py-argparse,py-threading,py-unittest,rails,rails-callbacks,sidekiq,view-component,~active-model-validators,~puma,~py-context-manager,~py-signal,~rspec"
    "app-relaticle-laravelmcp|https://github.com/relaticle/relaticle.git|filament,laravel,laravel-ai,laravel-ai-agent,laravel-lifecycle,laravel-mcp,livewire,pest,phpunit,~bash-dispatch,~js-test-globals,~laravel-events,~py-argparse,~symfony"
    "app-agentfleet-prism|https://github.com/escapeboy/agent-fleet-o.git|express,laravel,laravel-ai,laravel-ai-agent,laravel-lifecycle,laravel-mcp,livewire,phpunit,prism-php,py-signal,~go-runtime,~js-test-globals,~laravel-events,~py-argparse,~py-threading,~symfony"
    "app-solidinvoice-mcpphp|https://github.com/SolidInvoice/SolidInvoice.git|api-platform,doctrine-lifecycle,go-testing,laravel,makepkg,mcp-php,phpunit,symfony,symfony-messenger,~cobra,~go-encoding,~go-runtime,~react,~react-hooks,~symfony-ai"
    # 0.14.0 framework rows: Elixir, Haskell, OCaml and Solidity (2026-11-25).
    "app-sequin-oban|https://github.com/sequinstream/sequin.git|broadway,ecto-migration,ecto-type,express,genstage,go-testing,js-test-globals,mix-task,oban,otp,otp-application,phoenix,phoenix-liveview,plug,~go-encoding,~go-runtime,~go-signal,~gorilla-mux,~net-http,~phoenix-channel,~svelte"
    "app-hexpm-ecto|https://github.com/hexpm/hexpm.git|broadway,ecto-migration,ecto-type,mix-task,oban,otp,otp-application,phoenix,phoenix-liveview,plug"
    "app-shellcheck-quickcheck|https://github.com/koalaman/shellcheck.git|quickcheck,~haskell-main"
    "app-hackage-hedgehog|https://github.com/haskell/hackage-server.git|hedgehog,quickcheck,tasty,~c-runtime,~haskell-main,~nix"
    "app-predimarkt-ihp|https://github.com/avitkauskas/predimarkt.git|ihp,~hspec,~nix,~quickcheck,~wai"
    "app-qubesfirewall-mirage|https://github.com/mirage/qubes-mirage-firewall.git|mirage,~alcotest,~cmdliner"
    "app-frankencoin-chainlink|https://github.com/Frankencoin-ZCHF/Frankencoin.git|chainlink,js-test-globals,solidity-public"
    # 0.14.0: a real Python Lambda application (AWS Solutions). Powertools
    # itself declares `aws_lambda_powertools` in-tree, so it no longer detects
    # aws-lambda; this app carries that claim.
    "app-instancescheduler-lambda|https://github.com/aws-solutions/instance-scheduler-on-aws.git|aws-lambda,aws-lambda-powertools,jest,py-argparse,py-context-manager,py-threading,pytest,~aws-cdk,~py-unittest,~pydantic"
    "app-hertzbeat-springai|https://github.com/apache/hertzbeat.git|axum,cargo-test,grpc-java,jackson,java-closeable,java-concurrent,jsr250-lifecycle,junit,junit-sources,netty,node-test,playwright,py-unittest,react,rmcp,servlet,spring,spring-ai,spring-ai-hooks,spring-context,spring-jobs,vitest"
    "app-ruoyiai-langchain4j|https://github.com/ageerle/ruoyi-ai.git|jackson,java-closeable,jsr250-lifecycle,junit,langchain4j,langchain4j-agentic,langchain4j-hooks,py-unittest,servlet,spring,spring-context,spring-jobs,spring-websocket"
    "app-eddi-quarkus|https://github.com/labsai/EDDI.git|jackson,jakarta-rs,java-closeable,java-concurrent,jsr250-lifecycle,jsr330-inject,junit,junit-sources,langchain4j,langchain4j-hooks,node-test,playwright,py-argparse,py-threading,react,vitest"
    "app-kestra-langchain4j|https://github.com/kestra-io/kestra.git|grpc-java,jackson,java-closeable,java-concurrent,jmh,jsr250-lifecycle,jsr330-inject,junit,junit-sources,langchain4j,langchain4j-hooks,micronaut,playwright"
    "app-askimo-langchain4j|https://github.com/askimo-ai/askimo.git|compose,java-closeable,junit,junit-sources,kotlinx-serialization,langchain4j,langchain4j-agentic,langchain4j-hooks,swing"
    "app-klibsio-springai|https://github.com/JetBrains/klibs-io.git|compose,gradle-task,jsr250-lifecycle,junit,junit-sources,kotlinx-serialization,playwright,servlet,spring,spring-ai,spring-context,spring-jobs,vitest"
    "app-trailblaze-koog|https://github.com/block/trailblaze.git|android-activity,android-broadcastreceiver,android-contentprovider,android-fragment,android-service,appkit,cocoa-delegate,compose,electron,gradle-task,java-closeable,java-concurrent,js-test-globals,junit,koog,kotlin-js-export,kotlinx-serialization,playwright,swing,uikit-appdelegate"
    "app-mobiletester-koog|https://github.com/maikotrindade/mobile-tester-agent.git|junit,koog,vitest"
    # 0.14.0 agent audit: JS/TS and Swift AI agent and tool-calling rows
    # (measured claims).
    "app-context7-mcp|https://github.com/upstash/context7.git|express,hono,mcp-handler,mcp-sdk,node-event-emitter,vercel-ai,vitest,~js-test-globals,~react,~react-hooks,~vercel-ai-hooks"
    "app-zero-cfagents|https://github.com/Mail-0/Zero.git|cloudflare-agents-lifecycle,cloudflare-workers,cloudflare-workers-rpc,hono,mcp-sdk,playwright,vercel-ai,vitest,~cloudflare-agents,~react,~react-hooks,~trpc,~vercel-ai-hooks"
    "app-coworkers-cfagents|https://github.com/fayazara/coworkers.git|cloudflare-agents,cloudflare-agents-lifecycle,cloudflare-workers,cloudflare-workers-rpc,mcp-sdk,vercel-ai,vitest,~react,~react-hooks,~vercel-ai-hooks"
    "app-morphic-vercelai|https://github.com/miurla/morphic.git|node-event-emitter,vercel-ai,vitest,~nextjs,~react,~react-hooks,~vercel-ai-hooks"
    "app-inboxzero-vercelai|https://github.com/elie222/inbox-zero.git|bullmq,electron,mcp-sdk,node-event-emitter,node-test,playwright,react,vercel-ai,vercel-ai-hooks,vitest,~nextjs,~react-hooks"
    "app-dataagent-mastra|https://github.com/datagallery-ai/dataagent.git|click,fastapi,mastra,mcp-sdk,node-event-emitter,node-test,pytest,vitest,~mastra-agent,~mastra-workflows,~nextjs,~py-argparse,~py-unittest,~pydantic,~react,~react-hooks,~sqlalchemy-events"
    "app-openreels-mastra|https://github.com/tsensei/OpenReels.git|bullmq,mastra-workflows,node-event-emitter,vitest,~fastify,~mastra,~mastra-agent,~nextjs,~react,~react-hooks,~vercel-ai,~vercel-ai-hooks"
    "app-wingman-langchain|https://github.com/RussellCanfield/wingman-ai-vscode-extension.git|langchain-js,vitest,vscode-extension,~langchain-js-callbacks,~react,~react-hooks,~vscode-lm-tools"
    "app-refly-langchain|https://github.com/refly-ai/refly.git|bullmq,langchain-js,langchain-js-callbacks,nestjs,nestjs-schedule,node-event-emitter,react,vitest,~express,~jest,~mcp-sdk,~py-argparse,~react-hooks"
    "app-wordpecker-openaiagents|https://github.com/baturyilmaz/wordpecker-app.git|express,openai-agents,~jest,~js-test-globals,~openai-agents-handoffs,~openai-agents-hooks,~react,~react-hooks"
    "app-codepilot-claudeagent|https://github.com/op7418/CodePilot.git|claude-agent-sdk,electron,mcp-sdk,node-event-emitter,node-test,playwright,react,vercel-ai,vercel-ai-hooks,~js-test-globals,~nextjs,~react-hooks,~worker-threads"
    "app-vscodeprgithub-vscode|https://github.com/microsoft/vscode-pull-request-github.git|mocha,node-event-emitter,react,vscode-extension,vscode-lm-tools,~react-hooks"
    "app-kaset-foundationmodels|https://github.com/sozercan/kaset.git|appintents,appkit,cocoa-delegate,foundation-models,py-context-manager,py-signal,py-threading,py-unittest,uikit-appdelegate,uikit-lifecycle,xctest,~apple-app-extension,~py-argparse,~swift-testing,~swiftui"
    "app-equibles-mcp|https://github.com/daniel3303/Equibles.git|aspnet-mvc,benchmarkdotnet,csharp-disposable,dotnet-hosting,ef-core,masstransit,mcp-csharp-sdk,xunit,xunit-data,~aspnet-minimal,~csharp-runtime,~dotnet-serialization"
    "app-openclaw-semantickernel|https://github.com/clawdotnet/openclaw.net.git|aspnet-minimal,avalonia,csharp-disposable,dotnet-hosting,mcp-csharp-sdk,microsoft-agent-framework,microsoft-extensions-ai,mvvm-toolkit,node-event-emitter,py-signal,py-unittest,semantic-kernel,xaml,xunit,xunit-data,~aspnet-mvc,~avalonia-xaml,~blazor,~blazor-jsinterop,~blazor-lifecycle,~csharp-runtime,~go-encoding,~go-runtime,~net-http,~py-argparse,~py-threading,~semantic-kernel-hooks"
    "app-bookworm-agentframework|https://github.com/foxminchan/BookWorm.git|azure-functions,csharp-disposable,ef-core,fluentvalidation,grpc-dotnet,mcp-csharp-sdk,microsoft-agent-framework,microsoft-agent-framework-workflows,microsoft-extensions-ai,quartz,vitest,~aspnet-minimal,~aspnet-mvc,~csharp-runtime,~dotnet-hosting,~dotnet-serialization,~nextjs,~node-event-emitter,~playwright,~react,~react-hooks"
    "app-netcorekevin-meai|https://github.com/junkai-li/NetCoreKevin.git|aspnet-mvc,csharp-disposable,dotnetcore-cap,ef-core,mediatr,microsoft-extensions-ai,signalr,xunit,~aspnet-minimal,~csharp-runtime,~dotnet-hosting,~dotnet-serialization,~hangfire,~mcp-csharp-sdk,~microsoft-agent-framework,~vue,~xunit-data"
    "app-blazorgpt-semantickernel|https://github.com/magols/BlazorGPT.git|aspnet-mvc,blazor-lifecycle,csharp-disposable,ef-core,razor,semantic-kernel,semantic-kernel-hooks,~aspnet-minimal,~blazor,~blazor-jsinterop,~csharp-runtime,~js-test-globals"
    "app-githubmcp-gosdk|https://github.com/github/github-mcp-server.git|chi,go-encoding,go-testing,mcp-go-sdk,mcp-go-sdk-middleware,net-http,node-test,~cobra,~go-runtime,~go-signal,~react,~react-hooks"
    "app-k8sgpt-mcpgo|https://github.com/k8sgpt-ai/k8sgpt.git|go-testing,mcp-go,net-http,~cobra,~controller-runtime,~go-encoding,~go-runtime,~go-signal,~grpc-gateway,~grpc-go,~mcp-go-hooks,~prometheus-collector,~robfig-cron"
    "app-terraformmcp-mcpgo|https://github.com/hashicorp/terraform-mcp-server.git|go-testing,mcp-go-hooks,mcp-go-sdk,net-http,~cobra,~go-encoding,~go-runtime,~go-signal,~mcp-go,~mcp-go-sdk-middleware"
    "app-mayfly-eino|https://github.com/dromara/mayfly-go.git|eino,go-encoding,go-testing,gorm,net-http,vitest,~cobra,~eino-graph,~eino-hooks,~gin,~go-runtime,~go-signal,~gorilla-mux,~grpc-go,~mcp-go,~mcp-go-hooks,~robfig-cron,~vue"
    "app-cozestudio-eino|https://github.com/coze-dev/coze-studio.git|eino,eino-graph,eino-hooks,go-encoding,go-testing,gorm,hertz,nats,node-event-emitter,react,sarama,testify-suite,vitest,~go-runtime,~go-signal,~net-http,~react-hooks"
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
    "ansys-user-material|ANSYS USERMAT sources found on GitHub are single-file course exercises, not applications"
    "lsdyna-user-material|no public LS-DYNA user-material application found; the UMAT4x names are fixed by the solver"
    "apisix-plugin|no third-party APISIX plugin repository with real use was found (searches for custom-plugin repos over 30 stars returned none); measured on APISIX itself (app-apisix-openresty): 317 phase-function entries"
    "mastra-agent|no application found passing a function as a Mastra Agent's instructions/model/tools (dataagent and OpenReels use strings; GitHub code search, 0.14.0 agent audit); verified by crates/cgg/tests/frameworks_apps_b.rs"
    "openai-agents-handoffs|no application found using handoff({ onHandoff }) from @openai/agents (wordpecker, clueless and witsy checked; the 78 package.json hits are mostly SDK examples); verified by crates/cgg/tests/frameworks_apps_b.rs"
    "openai-agents-hooks|no application found calling defineInputGuardrail/defineOutputGuardrail or giving an Agent a function as instructions (same search as openai-agents-handoffs); verified by crates/cgg/tests/frameworks_apps_b.rs"
    "symfony-ai|no application using Symfony AI's #[AsTool] was found: a GitHub code search (2026-11-25) returned only symfony/ai, its demo, tool and MCP bundles and a fork; detected (no tools) on app-solidinvoice-mcpphp; the row is verified by crates/cgg/tests/frameworks_ruby_php.rs"
    "oxygen|no real Oxygen.jl application found: a GitHub search for apps using Oxygen returned only examples and projects with one star or fewer (0.14.0 audit); the row is verified by crates/cgg/tests/frameworks_perl_r_julia_ps.rs"
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
