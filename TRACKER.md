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
| WS1 | External module/package heads never name-matched to workspace (all langs w/ imports); no high promotion for external alias | A1 | in progress (agent launched 2026-10-12 01:00) | | |
| WS2 | Java/C# visibility (annotation args, implicit-public interface members, protected evidence); TS visibility capture | B1, C5-TS | pending | | |
| WS3 | Param-list splitting respects generics; Kotlin receiver construction; Python multi-line self receiver | A2, A3, A7 | pending | | |
| WS4 | Module-qualified calls bind by module, not bare name (JS/TS namespace/require, Python/Go decoys); bare call never binds to a method (PHP, Perl) | A4, A5, A6 | pending | | |
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
(none yet)

## Log
- 2026-10-12: tracker created; baseline bins/cgg-0.10.2 (= 5b4bdec); harness null test 0 diffs (5 repos); sensitivity 0.10.0->0.10.2 reproduces nkb 105->17 high.
- 2026-10-12 01:00: WS1 started; before = bins/WS1-before (0.10.2). Cron c84238a3 every 2h at :17.
- 2026-10-12 01:10: user: commit+push after every fix set (failing NVMe). Pushed remediate/cross-language (=5b4bdec) and remediation-state to origin+backup.
- 2026-10-12 01:20: user approved committing their own CLAUDE.md/release.sh/upgrade-local.sh edits to main + push. Hook-skipping commit was denied; do it WITH the hook (worktree target/main-wt, already applied) between WS1 and WS2 so the build does not skew WS1 perf. Backup in user-edits/.
- 2026-10-12 02:50: WS1 agent stalled ~1h on a nohup-backgrounded A/B launch that never executed (v1 A/B done: 160 repos changed, all drops, 0 adds). Stopped and resumed with context; told to use Bash run_in_background instead of nohup.
- 2026-10-12 04:50: WS1 agent stalled again (02:47) on 'rm -rf … && graph_ab' — calls needing approval hang forever in a background agent. Orchestrator runs the v2 A/B itself (ab/WS1-v2); agent to be resumed for classification/perf/report with 'no rm -rf, no nohup'.
- 2026-10-12 05:10: WS1 v2 A/B (orchestrator): 283 repos, excl hcl-terraform-aws + zig-zig (timeout old AND new — zig near 60s cap, check later), dropped java 54684 kotlin 8422 py 8096 go 5987 ts 5890 js 3462 cpp 1242 cs 1201; added 31 (cpp). Orchestrator spot-checks: drops correct (external heads, static imports, FQN stdlib, C# namespace visibility); 36 new dead-high trace to correct drops (remaining = interface decls → WS2/WS5). REGRESSION: 31 added cpp-nlohmann std::to_string/std::swap → workspace 'std::string to_string' high. Agent resumed to fix + v3 A/B + perf + changelog.
- 2026-10-12 06:47: tick — WS1 agent active, on iteration v7 (A/B ab/WS1-v7 running, gates green: 1095 tests, clippy, fmt, docs-check). perf/WS1.md exists.
