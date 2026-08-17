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
    "starlark-skylib|https://github.com/bazelbuild/bazel-skylib.git|starlark|lib||"
    "cmake-kitware|https://github.com/Kitware/CMake.git|cmake|Modules||"
    "nix-home-manager|https://github.com/nix-community/home-manager.git|nix|modules||"
    "verilog-picorv32|https://github.com/YosysHQ/picorv32.git|verilog|||"
    "vhdl-uvvm|https://github.com/UVVM/UVVM.git|vhdl|uvvm_util/src||"
    "asm-xv6|https://github.com/mit-pdos/xv6-public.git|asm|||"
    "asm-xv6-mixed|https://github.com/mit-pdos/xv6-public.git|c,asm|||"
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
    "app-ghostfolio-nestjs|https://github.com/ghostfolio/ghostfolio.git|angular,angular-host,angular-router,bullmq,nestjs,nestjs-schedule,~express"
    "app-immich-nestjs|https://github.com/immich-app/immich.git|android-broadcastreceiver,android-worker,bullmq,cocoa-delegate,compose,express,fastapi,nestjs,node-event-emitter,py-context-manager,torch,uikit-appdelegate,~android-activity,~android-application,~java-closeable,~nestjs-schedule,~py-argparse,~py-signal,~py-threading,~py-unittest,~react,~uikit-lifecycle,~worker-threads"
    "app-calcom-nextjs|https://github.com/calcom/cal.com.git|bullmq,express,nestjs,node-event-emitter,react,wordpress,worker-threads"
    "app-spring-mall|https://github.com/macrozheng/mall.git|junit,servlet,spring,spring-jobs,spring-messaging,~java-closeable,~java-concurrent"
    "app-thingsboard-concurrent|https://github.com/thingsboard/thingsboard.git|angular,angular-host,angular-router,express,java-closeable,java-concurrent,junit,servlet,spring,spring-jobs,~hibernate-listeners,~jpa-lifecycle"
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
    "app-openzeppelin-solidity|https://github.com/OpenZeppelin/openzeppelin-contracts.git|solidity-public"
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
    "android-jobservice|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "apscheduler|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "avalonia|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "blazor-jsinterop|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "compojure|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "dancer2|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "dash|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "dramatiq|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "embedded-rt|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "erlang-gen-server|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "falcon|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "godot|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "gradle-plugin|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "hertz|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "hspec|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "huey|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "javafx|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "julia-base-dispatch|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "julia-module-init|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "lambda-runtime|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "litestar|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "loopback|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "martini|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "mediatr|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "mojolicious|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "monogame|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "ntex|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "orleans|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "phoenix-socket|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "plug-router|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "polka|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "powershell-dsc|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "quart|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "r-package-hooks|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "react-native|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "restify|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "robyn|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "salvo|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "scrapy|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "shuttle|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "spring-batch|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "storm|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "swing|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "symfony-messenger|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
    "tasty|no APPS application exercises this rule; tests/detect_prefixes.rs proves its detect prefix can fire, nothing in APPS proves it enumerates — see the language-corpus note above"
)

# Clone or update repos
clone_repos() {
    echo "Cloning/updating repos in $REPOS_DIR..."
    mkdir -p "$REPOS_DIR"
    for entry in "${REPOS[@]}"; do
        IFS='|' read -r name url _ _ _ _ <<< "$entry"
        dir="$REPOS_DIR/$name"
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

        dir="$REPOS_DIR/$name"
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
