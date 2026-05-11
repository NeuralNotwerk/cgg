#!/usr/bin/env bash
# scripts/benchmark.sh — Clone/update test repos and calculate support stats.
# Usage: ./scripts/benchmark.sh [--update] [--lang LANG]
set -euo pipefail

REPOS_DIR="${CGG_BENCH_DIR:-/storage/tmp}"
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
    "lua-kong|https://github.com/Kong/kong.git|lua||Lua|f"
    "dart-flame|https://github.com/flame-engine/flame.git|dart||Dart|f"
    "scala-play|https://github.com/playframework/playframework.git|scala||Scala|fm"
    "hcl-vpc|https://github.com/terraform-aws-modules/terraform-aws-vpc.git|hcl||HCL|f"
    "zig-http|https://github.com/karlseguin/http.zig.git|zig||Zig|f"
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
                grep -v "Anonymous\|__anon\|anonFunc" | \
                awk -F'\t' '{print $1}' | \
                grep -v "^[A-Z_]*$" | \
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
        if [ "$ct" -gt 0 ]; then
            ratio=$(echo "scale=0; $cg * 100 / $ct" | bc)
            if [ "$ratio" -ge 90 ]; then tier="✅ Fully"; ((fully++)) || true
            elif [ "$ratio" -ge 75 ]; then tier="◐ Partial"; ((partial++)) || true
            elif [ "$ratio" -ge 50 ]; then tier="⚠ Best Effort"; ((best++)) || true
            else tier="❌ Deficient"; ((deficient++)) || true; fi
            ratio="${ratio}%"
        else
            # No ctags baseline — mark as OK if we found callables
            if [ "$cg" -gt 0 ]; then tier="✅ Fully"; ((fully++)) || true; fi
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
    --help)   echo "Usage: $0 [--update] [--lang LANG]"; exit 0 ;;
    *)        clone_repos; run_benchmark ;;
esac
