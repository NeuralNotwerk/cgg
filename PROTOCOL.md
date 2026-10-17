# Per-workstream protocol (every remediation subagent follows this)

You are the ONLY agent working right now. Do not spawn subagents. Do not run
anything concurrently with a perf measurement.

Read first: /storage/cgg/CLAUDE.md (architecture, gates), TRACKER.md (rules,
your workstream row), FINDINGS.md (the findings you are fixing, with code
pointers — line numbers drift, re-locate by symbol).

## Hard rules (from the user)
1. Never write content specific to an individual function/method/class name
   outside framework rule tables (cgg-core/src/frameworks/rules.rs and the
   like) and builtin/stdlib tables (cgg-core stdlib lookup tables). Fixes
   are structural: grammar node kinds, import tables, scopes, types.
   Test fixtures may of course use any names.
2. Do not chase individual function names in the corpus diff — classify
   changes by PATTERN (node shape, receiver kind, import kind), never
   "fix" one name.
3. Do not chase latency deviations under 1 second.
4. Do not edit /storage/cgg/CLAUDE.md, scripts/release.sh or
   scripts/upgrade-local.sh. If CLAUDE.md prose becomes wrong because of
   your change, write the needed edit into your final report under
   "CLAUDE.md edits needed"; the orchestrator applies it.
5. Do not commit, push, tag, or switch branches. The orchestrator reviews
   and commits. Leave your changes in the working tree.
6. Keep the default graph deterministic at any --jobs (build new indexes
   sorted / in input order). Never manufacture an edge from a string.
   Prefer refusing a wrong edge over guessing; a dropped site must stay
   recorded as unresolved with a reason (never silent).
7. NEVER run a command that needs interactive approval: no `rm -rf`, no
   `nohup`, no `( … &)` subshells, no `git` writes. In a background agent
   such a call hangs forever (WS1 lost ~3h to this). To redo an output
   dir, write to a NEW name (ab/<WS>-v2, -v3 …) instead of deleting. For
   anything longer than ~9 min (the A/B), use the Bash tool's
   `run_in_background: true` and wait for its completion notice.

## Steps
1. BEFORE binary: target/remediation/bins/<WS>-before already exists (the
   orchestrator copies it before you start). Do not rebuild it.
2. Reproduce each finding with the fixtures in target/remediation/fixtures
   (or new minimal ones under target/remediation/fixtures/<WS>/) using the
   BEFORE binary. Keep a positive control per pattern.
3. Implement. Add unit tests next to the code you change (plugin
   `#[cfg(test)]` modules, cross_file.rs / deadcode tests) covering the
   fixed pattern AND a near-miss that must not change. Existing guard tests
   that start failing are the fix working only if you can explain why —
   otherwise they are a regression.
4. Gates, all must pass:
   `cargo test --workspace`
   `cargo clippy --workspace --all-targets -- -D warnings`
   `cargo fmt --all --check`  (run `cargo fmt --all` to fix)
   `cargo build --release -p cgg` then `python3 scripts/docs-check.py`
5. Correctness A/B over the WHOLE corpus (never a subset):
   `python3 target/remediation/tools/graph_ab.py target/remediation/bins/<WS>-before target/release/cgg target/remediation/ab/<WS> --parallel 6`
   (≈15–25 min). Read report.txt, by_language.tsv. Classify EVERY dropped
   and added edge and every dead-high change into pattern classes with
   counts (script it over dropped.jsonl/added.jsonl/changed.jsonl/
   dead_changed.jsonl — the `line` field is the call-site source text).
   Spot-check ≥5 per class by reading source. Any class that is a
   regression (a correct edge lost, a wrong edge gained, a correct dead
   finding hidden) must be fixed or explicitly justified. Re-run the A/B
   after any further change. Excluded (timed-out) repos are named.
6. Perf, BEFORE vs AFTER, over the WHOLE corpus (rule since 2026-10-16:
   the 9-repo subset hid 0.11.0's +2.7 s regression for three days). With
   nothing else running (after the A/B finishes), in the background:
   `python3 scripts/compare-release.py target/remediation/bins/<WS>-before target/release/cgg --jobs 1 --out target/remediation/perf/<WS>.json > target/remediation/perf/<WS>.log 2>&1`
   (~10 min). Report total, median per-repo delta, and every repo moving
   ≥1 s. A deviation ≥1 s is investigated (re-run that repo alone; if it
   persists, find the cause and fix it or justify it). Under 1 s: report,
   do not chase.
7. Add a CHANGELOG.md entry under `## [Unreleased]` (create the section
   above the latest released version if absent): what was wrong, what changed, which
   languages, and a *Compatibility* note if default-graph edges drop.
   Keep it factual; numbers from step 5.
8. Final report (≤ 700 words): findings fixed (by FINDINGS id), files
   changed, tests added, gate results, A/B summary per language with the
   pattern classification and spot-check verdicts, perf table summary,
   anything NOT fixed and why, "CLAUDE.md edits needed".
