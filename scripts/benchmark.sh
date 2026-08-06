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
    "app-fastapi-dispatch|https://github.com/Netflix/dispatch.git|fastapi"
    "app-django-netbox|https://github.com/netbox-community/netbox.git|django,django-admin"
    "app-flaskbb-flask|https://github.com/flaskbb/flaskbb.git|flask"
    "app-saleor-celery|https://github.com/saleor/saleor.git|celery"
    "app-black-click|https://github.com/psf/black.git|click"
    "app-torch-ultralytics|https://github.com/ultralytics/ultralytics.git|torch"
    "app-ghost-express|https://github.com/TryGhost/Ghost.git|express,worker-threads"
    "app-ghostfolio-nestjs|https://github.com/ghostfolio/ghostfolio.git|nestjs-schedule"
    "app-immich-nestjs|https://github.com/immich-app/immich.git|nestjs,bullmq,~worker-threads"
    "app-calcom-nextjs|https://github.com/calcom/cal.com.git|~nextjs"
    "app-spring-mall|https://github.com/macrozheng/mall.git|spring,spring-jobs,spring-messaging"
    "app-thingsboard-concurrent|https://github.com/thingsboard/thingsboard.git|java-concurrent"
    "app-akka-samples|https://github.com/akka/akka-samples.git|akka"
    "app-druid-jaxrs|https://github.com/apache/druid.git|jakarta-rs"
    "app-micronaut-graalapp|https://github.com/micronaut-guides/micronaut-creating-first-graal-app.git|micronaut"
    "app-gin-photoprism|https://github.com/photoprism/photoprism.git|gin,chi,net-http"
    "app-memos-echo|https://github.com/usememos/memos.git|echo"
    "app-fiber-recipes|https://github.com/gofiber/recipes.git|fiber"
    "app-homebox-chi|https://github.com/sysadminsmedia/homebox.git|chi"
    "app-temporal-samples|https://github.com/temporalio/samples-go.git|temporal"
    "app-eshop-aspnet|https://github.com/dotnet/eShop.git|aspnet-minimal,aspnet-mvc,dotnet-hosting,~blazor"
    "app-masstransit-sample|https://github.com/MassTransit/Sample-Twitch.git|masstransit"
    "app-ombi-quartz|https://github.com/Ombi-app/Ombi.git|quartz"
    "app-axum-cratesio|https://github.com/rust-lang/crates.io.git|axum"
    "app-lemmy-actix|https://github.com/LemmyNet/lemmy.git|actix-web"
    "app-actix-examples|https://github.com/actix/examples.git|actix-actor"
    "app-vaultwarden-rocket|https://github.com/dani-garcia/vaultwarden.git|rocket"
    "app-rails-mastodon|https://github.com/mastodon/mastodon.git|rails,sidekiq"
    "app-resque-sinatra|https://github.com/resque/resque.git|sinatra"
    "app-grape-swagger|https://github.com/ruby-grape/grape-swagger.git|grape"
    "app-monica-laravel|https://github.com/monicahq/monica.git|laravel"
    "app-symfony-demo|https://github.com/symfony/demo.git|symfony"
    "app-wordpress|https://github.com/WordPress/WordPress.git|wordpress"
    "app-codeigniter-starter|https://github.com/codeigniter4/appstarter.git|codeigniter"
    "app-cuda-samples|https://github.com/NVIDIA/cuda-samples.git|cuda"
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
        out=$("$CGG" "$scan_path" --lang "$lang" -t mermaid -o /dev/null --metrics /tmp/cgg_bench_metrics.json 2>&1 || true)
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
