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
