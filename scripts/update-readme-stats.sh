#!/usr/bin/env bash
# scripts/update-readme-stats.sh — manual README refresh for the surfaces
# the pre-commit hook does NOT touch.
#
# When to run (manual only):
#   - After `scripts/benchmark.sh` has cloned/updated the repos under
#     $CGG_BENCH_DIR — this is what produces the per-project numbers in
#     the README benchmark table.
#   - When the self-analysis mermaid graph (the big one under
#     "## Self-analysis", filtered to `cgg::analyze_in_pool`) needs to
#     reflect resolver/plugin changes.
#   - As a follow-up after large refactors that change the benchmark
#     numbers materially.
#
# What it patches:
#   1. Self-stats line  → delegated to scripts/update-readme-stats.py
#                         (same marker-based path the pre-commit hook uses)
#   2. Self-analysis mermaid graph → scripts/patch-readme-stats.py
#   3. Benchmark table             → scripts/patch-readme-stats.py
#
# What it does NOT patch (handled elsewhere):
#   - cgg-walk / cgg-lang mermaid blocks → pre-commit hook
#     (scripts/update-readme-graphs.py)
#   - Doc-consistency checks            → pre-commit hook
#     (scripts/docs-check.py)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CGG="${ROOT}/target/release/cgg"
README="${ROOT}/README.md"
REPOS_DIR="${CGG_BENCH_DIR:-/storage/cgg-test_repos}"

if [ ! -f "$CGG" ]; then
    echo "Build cgg first: cargo build --release -p cgg" >&2
    exit 1
fi

# --- Generate data files ---
echo "Generating self-analysis..."
$CGG "$ROOT/crates" -t mermaid -o /dev/null --metrics /dev/null 2>/tmp/cgg_self_stats.txt
$CGG "$ROOT/crates" -t mermaid --filter 'cgg::analyze_in_pool$' -n 1 -o /tmp/cgg_self_graph.mmd 2>/dev/null

# Self-stats line is patched separately via marker-based updater so
# this workflow stays consistent with the pre-commit hook.
python3 "${ROOT}/scripts/update-readme-stats.py" < /tmp/cgg_self_stats.txt

echo "Generating benchmark data..."
: > /tmp/cgg_bench_table.md
printf "| Project | Language | Callables | Edges | Cross-file | Time |\n" >> /tmp/cgg_bench_table.md
printf "| ------- | -------- | --------- | ----- | ---------- | ---- |\n" >> /tmp/cgg_bench_table.md

# display|clone dir|--lang|subdir
#
# INVARIANT: one entry per language plugin. This list must cover exactly
# the same (clone dir, lang, subdir) triples as `REPOS=( … )` in
# scripts/benchmark.sh — scripts/docs-check.py enforces it. It shipped
# five languages short (smithy, proto, graphql, openapi, asyncapi), so
# the README benchmark table silently claimed 44-language support while
# measuring 39 of them.
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
    "PowerShellGet|powershell-psget|powershell|"
    "openzeppelin-contracts|solidity-openzeppelin|solidity|contracts"
    "Paket|fsharp-paket|fsharp|src"
    "bazel-skylib|starlark-skylib|starlark|lib"
    "CMake/Modules|cmake-kitware|cmake|Modules"
    "home-manager|nix-home-manager|nix|modules"
    "picorv32|verilog-picorv32|verilog|"
    "UVVM|vhdl-uvvm|vhdl|uvvm_util/src"
    "xv6|asm-xv6|asm|"
    "xv6 (c+asm)|asm-xv6|c,asm|"
    "smithy/protocol-tests|smithy-protocol-tests|smithy|smithy-aws-protocol-tests/model"
    "grpc-proto|proto-grpc|proto|"
    "graphql-schema|graphql-github|graphql|"
    "OpenAPI-Specification|openapi-spec|openapi|examples"
    "asyncapi/spec|asyncapi-spec|asyncapi|examples"
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
    /tmp/cgg_self_graph.mmd \
    /tmp/cgg_bench_table.md

echo "Done."
