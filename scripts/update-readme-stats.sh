#!/usr/bin/env bash
# scripts/update-readme-stats.sh — Update README.md with current benchmark and self-analysis data.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CGG="${ROOT}/target/release/cgg"
README="${ROOT}/README.md"
REPOS_DIR="${CGG_BENCH_DIR:-/storage/tmp}"

if [ ! -f "$CGG" ]; then
    echo "Build cgg first: cargo build --release -p cgg" >&2
    exit 1
fi

# --- Generate data files ---
echo "Generating self-analysis..."
$CGG "$ROOT/crates" -t mermaid -o /dev/null --metrics /dev/null 2>/tmp/cgg_self_stats.txt
$CGG "$ROOT/crates" -t mermaid --filter 'cgg::run$' -n 1 -o /tmp/cgg_self_graph.mmd 2>/dev/null

echo "Generating benchmark data..."
> /tmp/cgg_bench_table.md
printf "| Project | Language | Callables | Edges | Cross-file | Time |\n" >> /tmp/cgg_bench_table.md
printf "|---------|----------|-----------|-------|------------|------|\n" >> /tmp/cgg_bench_table.md

declare -a ENTRIES=(
    "ripgrep|rust-ripgrep|rust|crates"
    "flask|python-flask|python|src"
    "express|js-express|javascript|lib"
    "zod|ts-zod|typescript|src"
    "fzf|go-fzf|go|src"
    "gson|java-gson|java|gson/src/main"
    "okio|kotlin-okio|kotlin|okio/src"
    "jq|c-jq|c|src"
    "nlohmann/json|cpp-nlohmann-json|cpp|include"
    "serilog|csharp-serilog|csharp|src"
    "acme.sh|bash-acme|bash|"
    "jekyll|ruby-jekyll|ruby|lib"
    "laravel|php-laravel|php|src"
    "AFNetworking|objc-afnetworking|objc|AFNetworking"
    "ggplot2|r-ggplot2|r|R"
    "Alamofire|swift-alamofire|swift|Source"
    "kong|lua-kong|lua|kong"
    "flame|dart-flame|dart|packages/flame/lib"
    "play|scala-play|scala|core/play/src/main"
    "terraform-vpc|hcl-vpc|hcl|"
    "http.zig|zig-http|zig|src"
    "gradle|groovy-gradle|groovy|subprojects/core/src"
    "Flux.jl|julia-flux|julia|src"
    "mojolicious|perl-mojolicious|perl|lib"
    "phoenix|elixir-phoenix|elixir|lib"
    "otp/stdlib|erlang-otp|erlang|lib/stdlib/src"
    "stdlib|fortran-stdlib|fortran|src"
    "ring|clojure-ring|clojure|ring-core/src"
    "pandoc|haskell-pandoc|haskell|src"
    "dune|ocaml-dune|ocaml|src"
)

for entry in "${ENTRIES[@]}"; do
    IFS='|' read -r display name lang subdir <<< "$entry"
    dir="$REPOS_DIR/$name"
    [ ! -d "$dir" ] && continue
    scan="$dir"
    [ -n "$subdir" ] && [ -d "$dir/$subdir" ] && scan="$dir/$subdir"
    out=$($CGG "$scan" --lang "$lang" -t mermaid -o /dev/null --metrics /dev/null 2>&1 || true)
    calls=$(echo "$out" | grep -oP '\d+ callables' | grep -oP '\d+')
    edges=$(echo "$out" | grep -oP '\d+ edges' | grep -oP '\d+')
    cf=$(echo "$out" | grep -oP '\d+ cross-file' | grep -oP '\d+')
    time_ms=$(echo "$out" | grep -oP '[\d.]+ ms' | grep -oP '[\d.]+')
    calls=${calls:-0}; edges=${edges:-0}; cf=${cf:-0}
    cf_pct="—"
    [ "$edges" -gt 0 ] && cf_pct="$(echo "scale=0; $cf * 100 / $edges" | bc)%"
    time_int=$(printf "%.0f" "${time_ms:-0}")
    printf "| %s | %s | %'d | %'d | %s | %sms |\n" \
        "$display" "$lang" "$calls" "$edges" "$cf_pct" "$time_int" >> /tmp/cgg_bench_table.md
done

# --- Patch README ---
echo "Patching README.md..."
python3 "${ROOT}/scripts/patch-readme-stats.py" "$README" \
    /tmp/cgg_self_stats.txt \
    /tmp/cgg_self_graph.mmd \
    /tmp/cgg_bench_table.md

echo "Done."
