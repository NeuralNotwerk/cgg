# Cross-language remediation tracker

Goal (user, 2026-10-12): remediate ALL findings in FINDINGS.md. Loop every
2h (session cron) until all are remediated.

User rules:
- Work sequentially: ONE subagent at a time, never parallel (no resource conflicts).
- Each language and/or major-finding remediation gets a subset of perf
  metrics measured BEFORE modifications and AFTER (paired, tools/perf_subset.sh).
- Do NOT chase individual function names.
- Do NOT chase deviations under 1 second.
- NEVER make content specific to any function/framework name outside of
  frameworks and builtins (framework rule tables, stdlib/builtin tables are
  the only place names may appear).

Repo conventions: branch `remediate/cross-language`. USER RULE (2026-10-12): after EVERY fix set,
commit AND push to remotes `origin` and `backup` — the box's NVMe is failing. Also run
`tools/push_state.sh "<msg>"` after every fix set and every tracker update: it snapshots this
directory (minus binaries / full graph outputs) onto orphan branch `remediation-state` on both
remotes. To resume on another box: clone, `git checkout remediate/cross-language`, and
`git show remediation-state:TRACKER.md`.
One commit per workstream after review, staging only the workstream's
files — NEVER stage the user's own uncommitted CLAUDE.md / scripts/release.sh /
scripts/upgrade-local.sh edits. Commit with `CGG_INSTALL_DIR=""` so an
unreleased build is not installed onto PATH. CLAUDE.md doc changes a
workstream needs are queued in "Pending CLAUDE.md edits" below, not made.

Tools (target/remediation/tools): graph_ab.py (full-corpus graph +
dead-code A/B, null-tested), perf_subset.sh (paired latency, alone on
the box). Baselines: target/remediation/bins/. Results: ab/, perf/, logs/.

## Workstreams (in execution order)

| # | Workstream | Findings | Status | Commit | Notes |
|---|---|---|---|---|---|
| WS1 | External module/package heads never name-matched to workspace (all langs w/ imports); no high promotion for external alias | A1 | DONE 2026-10-12 | 02780c3 (+433e266 hook fix, 8507bf7 docs) | A/B ab/WS1-v7: -146,535 / +208 edges, 36 new dead-high (all wrong callers); perf/WS1-v7.md -0.9%, all <1s. Residuals: Kotlin receiver-less runBlocking/File(x).readText → WS3; import shadowed by local; webpack/babel-only JS aliases; chained off imported fn (sql`…`.execute()); gtest-style template owner fan-out; C++ qn folds return type (`std::string to_string`) → WS8 |
| WS2 | Java/C# visibility (annotation args, implicit-public interface members, protected evidence); TS visibility capture | B1, C5-TS | DONE 2026-10-12 | b019a00 | A/B ab/WS2-v1: graph identical; 19,317 dead high→medium (Java 16,770: interface 8,445, annotation-hidden public 4,186, protected cap 4,076; C# 2,547), none raised; perf/WS2.md −0.2%, all <1s. Decision: protected → existing PublicVisibility evidence (token `protected`), caps medium. Residuals: kotlin_vis text-cut (safe direction) → WS3; Java record_method static check text-based → WS8; TS abstract_class_declaration pushes no scope → WS8 |
| WS3 | Param-list splitting respects generics; Kotlin receiver construction; Python multi-line self receiver | A2, A3, A7 (+ WS1/WS2 Kotlin residuals) | DONE 2026-10-12 | bc64d1b (+22795af docs) | A/B ab/WS3-v9: −43,449 (2,632 high) / +24,121 (2,411 high) in 123 repos; Kotlin −37,725/+12,895 (dropped = wrong bare/self/super edges; added high = extensions, constructed receivers; added medium duck = mixed, replaces wrong HIGH bare binds). super via base chain in all langs w/ bases. perf/WS3.md −0.7%, all <250ms. ACCEPTANCE CARRY-OVER: 68 Java dead findings newly HIGH (26 @PreDestroy/@PostConstruct → WS7, 17 external-contract overrides → WS5, 3 method refs → WS6, 1 serialization → WS7, ~21 untyped-receiver-only) + 3 Rust Rocket `Fairing::info` impls (external-trait impls → WS13) — those WS must show these drop out of HIGH. Residuals: Kotlin this.x/other.x duck (no ctor-property typing); C# extension methods / Solidity `using L for T` lost on now-typed receivers (~12 edges) → WS10; nullable Foo? → WS11; C++ `ns::Foo x` params caught by name:Type parser → WS8; Java/C# super into external base still fans out → WS5; one-line Kotlin `object X { fun f() }` loses scope → WS8; Rust `Data<…>` collides with workspace `Data` → WS13 |
| WS4 | Module-qualified calls bind by module, not bare name (JS/TS namespace/require, Python/Go decoys); bare call never binds to a method (PHP, Perl) | A4, A5, A6 | in progress | | |
| WS5 | Interface dispatch beyond Rust: impl->decl link from base types, F2/F6 analogs, override evidence, variable/generic receivers as doubt, decl nodes for TS/Swift/Scala/Dart | B2 | pending | | |
| WS6 | Value-position references + reflection evidence (Java method refs, C# method groups/events/nameof, Python return/yield/decorator/default/chained), honest value_refs capability | B3 | pending | | |
| WS7 | Framework/builtin roots: test data-source attributes (JUnit/TestNG/NUnit/xUnit), Java serialization hooks, lifecycle/event annotations, C/C++ callback gaps | B4 | pending | | |
| WS8 | Extraction bugs: Dart method spans, Scala receiver split, C pointer-returning fns, C++ ctor-arg locals, C++ .h virtuals, Julia self-edge, C macro w/o `;`, C# inherited via construction, Ruby paren-less calls, PHP $this->m(), Lua M.f, Elixir &f/1, Python assigned lambda | C5, A8, A9, A10 | pending | | |
| WS9 | Explicit type-argument callees (C++, Go, Dart, C#, Swift, Scala) | C1, B5 | pending | | |
| WS10 | Receiver shapes: paren/cast/await, construction receivers, null-conditional (C#, Dart, PHP) | C2 | pending | | |
| WS11 | Optional/union/wrapper annotation typing (Python, TS, Kotlin, Swift) | C3 | pending | | |
| WS12 | Macro analogs: C/C++ function-defining macros; Elixir/Clojure quoted defs | C4, A11 | pending | | |
| WS13 | Rust leftovers: macro invocation sites credited; `Type::f()` constructor reading | C6 | pending | | |

## Pending CLAUDE.md edits
(none — WS1's were applied in 8507bf7; orchestrator applies future ones itself, user's edits are committed)

## Log
- 2026-10-12: tracker created; baseline bins/cgg-0.10.2 (= 5b4bdec); harness null test 0 diffs (5 repos); sensitivity 0.10.0->0.10.2 reproduces nkb 105->17 high.
- 2026-10-12 01:00: WS1 started; before = bins/WS1-before (0.10.2). Cron c84238a3 every 2h at :17.
- 2026-10-12 01:10: user: commit+push after every fix set (failing NVMe). Pushed remediate/cross-language (=5b4bdec) and remediation-state to origin+backup.
- 2026-10-12 01:20: user approved committing their own CLAUDE.md/release.sh/upgrade-local.sh edits to main + push. Hook-skipping commit was denied; do it WITH the hook (worktree target/main-wt, already applied) between WS1 and WS2 so the build does not skew WS1 perf. Backup in user-edits/.
- 2026-10-12 02:50: WS1 agent stalled ~1h on a nohup-backgrounded A/B launch that never executed (v1 A/B done: 160 repos changed, all drops, 0 adds). Stopped and resumed with context; told to use Bash run_in_background instead of nohup.
- 2026-10-12 04:50: WS1 agent stalled again (02:47) on 'rm -rf … && graph_ab' — calls needing approval hang forever in a background agent. Orchestrator runs the v2 A/B itself (ab/WS1-v2); agent to be resumed for classification/perf/report with 'no rm -rf, no nohup'.
- 2026-10-12 05:10: WS1 v2 A/B (orchestrator): 283 repos, excl hcl-terraform-aws + zig-zig (timeout old AND new — zig near 60s cap, check later), dropped java 54684 kotlin 8422 py 8096 go 5987 ts 5890 js 3462 cpp 1242 cs 1201; added 31 (cpp). Orchestrator spot-checks: drops correct (external heads, static imports, FQN stdlib, C# namespace visibility); 36 new dead-high trace to correct drops (remaining = interface decls → WS2/WS5). REGRESSION: 31 added cpp-nlohmann std::to_string/std::swap → workspace 'std::string to_string' high. Agent resumed to fix + v3 A/B + perf + changelog.
- 2026-10-12 06:47: tick — WS1 agent active, on iteration v7 (A/B ab/WS1-v7 running, gates green: 1095 tests, clippy, fmt, docs-check). perf/WS1.md exists.
- 2026-10-12 07:05: WS1 COMMITTED 02780c3 and pushed (origin+backup). Orchestrator re-verified the terraform class (58,597 Go drops): tests import external terraform-plugin-testing/knownvalue, the workspace one is aliased tfknownvalue → all wrong edges.
- 2026-10-12 07:05: hook bug found: `${CGG_INSTALL_DIR:-…}` ignored the documented CGG_INSTALL_DIR="" opt-out and installed the WS1 build to ~/.local/bin/cgg for ~1 min. Restored the 0.10.2 release (cmp-verified; glitch copy untouched). Fixed in 433e266 (`-` not `:-`), verified by the next commit.
- 2026-10-12 07:25: user's CLAUDE.md/release.sh/upgrade-local.sh committed to main WITH hook (dcb9876), pushed origin+backup; merged into remediate/cross-language (a905e94, README self-stats conflict → ours); worktree target/main-wt removed. WS1 CLAUDE.md notes applied (8507bf7). Branch pushed.
- 2026-10-12 07:30: WS2 started; before = bins/WS2-before (= 8507bf7 build). PROTOCOL rule 7 added (no approval-needing commands in agents).
- 2026-10-12 08:25: WS2 COMMITTED b019a00, pushed origin+backup (orchestrator verified: 0 graph diffs, 19,317 high→medium only, samples match classes). WS3 started; before = bins/WS3-before (= b019a00 build).
- 2026-10-12 08:47: tick — WS3 agent active; v1 A/B done, v2 A/B running (ab/WS3-v2).
- 2026-10-12 10:47: tick — WS3 agent active, iterating (A/B v9 running). Diff 16 files +2006/-154 (type_hints 695, kotlin 465, cross_file 353, cpp 186); other plugins only gain a mechanical `arrow: false` field (C++ `->` receivers).
- 2026-10-12 11:40: WS3 COMMITTED bc64d1b + docs 22795af, pushed origin+backup (orchestrator verified Kotlin: dropped high = super/literal self-edges; added high = real extension calls; added medium duck mixed). WS4 started; before = bins/WS4-before (= 22795af build).
