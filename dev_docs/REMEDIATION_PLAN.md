# Parity remediation plan (top 30 gaps)

Written 2026-10-17 from [PARITY_MATRIX.md](PARITY_MATRIX.md) §3, against
`remediate/cross-language` at 4bba8c4 (0.11.0 + tooling). Gap numbers below
are that table's ranks. Workstreams continue the numbering in
`target/remediation/TRACKER.md` (WS1–WS18 are done).

## Ordering principle

cgg's primary consumer is a coding agent reading mermaid. A wrong edge
marked `high` is worse than a missing edge, because the reader trusts it
and nothing in the output says otherwise. So the order is:

1. **Wrong `high` edges** (WS19): the confidence the output claims must
   be true before anything is added.
2. **Dead-code false positives** (WS20): findings an agent would act on
   by deleting live code.
3. **Missing edges in mainstream languages** (WS21–WS23), cheapest first.
4. **Framework and harness parity** (WS24), then detection and builtins
   (WS25).
5. **Per-plugin M-sized parity** (WS26–WS28): value references, locals
   specs and arity narrowing, one language at a time by corpus reach.
6. **Docs** (WS29): apply the matrices' "Doc corrections", then re-run
   the affected category agents to re-score.

## Code check of the claims (2026-10-17)

Before writing this plan I checked the top claims against the code and,
where cheap, a fixture (`<scratch>/spot/`):

| Gap | Status | What was seen |
| --- | --- | --- |
| 1 | **reproduced** | `service = mk(); service.run()` (mk returns an int) → `Service.run` with no `?` marker. The single-candidate arm `intra_file.rs:~615` emits `Confidence::High` unconditionally. |
| 2 | **reproduced** | `other()` calling `helper()` binds `outer.helper` (nested in another function) at high. The narrowing block has `&& candidates.len() > 1` (`intra_file.rs:~367`). |
| 3 | confirmed in code | `let is_param = fs == ls;` (`intra_file.rs:~297`): a closure's parameter scope is never the function's. |
| 4 | confirmed in code, **not reproduced** | `build_return_type_map` (`type_hints.rs:272`) keys on `def.simple_name` only (no language, no file), `or_insert` = first wins. A Python+Kotlin `make()` fixture did not trigger a wrong typing, so step 0 of WS19 is a reproducing fixture. |
| 6 | **reproduced** | async `_ahook` is reported `⟨unreferenced⟩`, sync `_shook` is not. `deadcode/mod.rs:505` uses `nd.kind != CallableKind::Function`. |
| 10 | **reproduced, worse than documented** | `func (s *Server) Zap() { s.zip() }` with a second `zip` on another type gives **no edge at all** (unresolved), not a fan-out. |
| 11 | confirmed in code | `FileFacts::unreachable` is filled by 8 plugins. Only the per-language flag (`LanguageSignals::unreachable`, `lib.rs:1935`) is read, and no resolver or dead-code code reads the regions. |
| 15 | confirmed in code | `rust_test_role` (`rust.rs:3069`) matches test attributes only. Nothing classifies non-test items inside `#[cfg(test)] mod`. |

The other gaps come from the category files with their own evidence. Each
workstream starts by reproducing its gaps as failing tests. A gap that
does not reproduce is recorded as such and dropped, not "fixed".

## Process (every workstream)

These are the standing rules from `target/remediation/PROTOCOL.md` and
memory, restated so each WS is self-contained:

1. **One subagent at a time.** It writes failing tests first (fixture per
   gap, per affected language), then the fix. It never runs `rm`, `nohup`,
   background jobs or git writes, and never edits CLAUDE.md (queued in the
   tracker's "Pending CLAUDE.md edits").
2. **No names in code** outside the framework tables (`rules.rs`,
   `deadcode/roots.rs`) and builtin tables (`stdlib`, `rust_types`,
   `DYN_*`). Scan every diff for string literals.
3. **Full-corpus graph A/B** with `tools/graph_ab.py` (all ~283 repos,
   null-tested), reported by confidence with `tools/conf_summary.py`.
   The orchestrator reviews a sample of every added/dropped bucket by
   reading source.
4. **Full-corpus perf per fix set** (new rule, 2026-10-16):
   `scripts/perf-compare.sh` against the WS's before-binary, all repos,
   `--jobs 1`. Ignore deltas under 1 s; anything over is explained
   before commit. (In 0.11.0 the 9-repo subset hid a +2.7 s regression
   for three days.)
5. Commit with `CGG_INSTALL_DIR=""` (md5 `~/.local/bin/cgg` before and
   after), push to origin and backup, then `tools/push_state.sh`, then
   update the TRACKER row.

**Release:** WS19 and WS20 change many edges and dead findings, but
they are bug fixes, so they ship as a patch (**0.11.x**) with a
Compatibility note. Only a wholly new feature warrants 0.12.0.

---

## WS19 — Confidence honesty (gaps 1, 2, 3, 4, 9) · effort S–M

**Why first:** every item here is a `high` edge that can be wrong. All of
them are in the shared resolver, so one change fixes many languages.

| Gap | Change | Where |
| --- | --- | --- |
| 1 | In the single-candidate intra-file arm, emit `Medium` when the site's receiver type is `weak_receiver_type` (mirror `cross_file.rs:~2724`). Also audit every other `Confidence::High` literal in `intra_file.rs` for the same flag. | `cgg-resolve/src/intra_file.rs` |
| 2 | Run lexical narrowing for one candidate too. A lone candidate nested in a function that does **not** enclose the site is not lexically visible. Keep the binding (a returned closure bound to a same-named local is real, see the comment at `:~292`) but cap it at `Medium`. Drop it only when the local/param shadowing check already says shadowed. | `intra_file.rs` narrowing block |
| 3 | Record closure parameters as parameters. `locals.rs` knows the binding came from a parameter list; carry an `is_param` bit on the local instead of inferring it from `fs == ls`. Covers `(x) =>`, `lambda x:`, `\|x\|`, `fun x ->` in every locals language. | `cgg-lang/src/locals.rs`, `FileFacts::locals`, `intra_file.rs:~297` |
| 4 | Step 0: write a fixture that reproduces a wrong typing from the bare-name key (two same-named functions with different declared return types, same language, different files; then cross-language). Fix: key the map by `(language, simple_name)`, prefer the caller's file, and type only when every same-language definition agrees (the `settle_closure_args` rule). | `cgg-resolve/src/type_hints.rs:272` |
| 9 | Include-style imports (`source`, `require_relative`/`load`, R `source()`, PowerShell dot-source, Solidity `import`, Starlark `load`, CMake `include`) bind through the file the include names, as `c_include_copies` does for C. If the included file cannot be determined, several same-named copies are a fan-out at `medium`, never `high`. | `cross_file.rs` (generalise `c_include_copies`), `modules.rs` |

**Acceptance:**

- **Fixtures:** every gap has a per-language fixture test in `crates/cgg-resolve/tests/`. Gaps 2 and 3 cover all 13 lexically scoped languages; gap 9 covers the 7 named languages.
- **A/B:** high → medium moves only, plus the drops for gap 9. No edges added at high.
- **Sampled review:** the orchestrator samples 50 moved edges across at least 10 languages and finds at least 90% were really uncertain.
- **Dead code:** dead-code findings may *rise* in confidence only where an edge that was keeping a callable live was wrong. Read every newly-high dead finding in a sample of 20.

**Risk:** gap 2's cap may demote correct bindings of closures returned and called through a same-named local. The `is_param`/local check keeps the common case; measure it in the A/B before tuning.

## WS20 — Dead-code false positives (gaps 5, 6, 11, 15, 21, 23) · effort S each

| Gap | Change | Where |
| --- | --- | --- |
| 6 | Decide "is a method" by owner (the definition has an owning class), not by `kind != Function`. Python async methods then get the external-contract cap. | `cgg-resolve/src/deadcode/mod.rs:505` |
| 5 | Record override markers: the Kotlin/Swift/Scala `override` modifier, TS `override`, Dart `@override`, C++ `override`/`final` specifiers. Normalise PHP `#[\Override]` so it matches `OVERRIDE_MARKERS` (leading `\`, attribute-group brackets). | the 7 plugins' attribute capture; `OVERRIDE_MARKERS` |
| 11 | Consume `FileFacts::unreachable`. A call site inside an unreachable region does not keep its target live in the dead-code pass, and its graph edge is capped at `medium`. | `lib.rs` (carry the regions), `deadcode/mod.rs`, edge confidence pass |
| 15 | Items inside a `#[cfg(test)]` module (and `#[cfg(test)]` items) get `TestRole::Helper`, so they are test roots, not production dead code. The `testfile.rs:69-71` comment then becomes true. | `cgg-lang/src/plugins/rust.rs` near `rust_test_role` |
| 21 | Make gtest enumerating: a `gtest` rule in `rules.rs` over the `TEST`/`TEST_F`/`TEST_P` macro-generated callables in files that include gtest, plus `SetUp`/`TearDown` on `::testing::Test` subclasses. Root `LLVMFuzzerTestOneInput` as a fuzz harness (`kind: test`). The names live in the framework table only. | `cgg-core/src/frameworks/rules.rs`, C++ macro tagging |
| 23 | Test-file arms for perl `t/*.t`, groovy `*Spec`, lua `_spec`, powershell `*.Tests.ps1`, solidity `.t.sol`, objc/haskell/r/clojure/hcl/bash/zig/julia conventions. Add JS/TS `.test.mts`/`.spec.cjs`. Tighten the generic `is_test_name` so PowerShell `Test-*` verbs are not tests, and require Go `Test` + non-lowercase (`Testing` is not a test). | `cgg-lang` testfile module, per-language arms |

**Acceptance:**

- **Fixtures:** each gap has a fixture with an expected dead-code report.
- **A/B:** dead findings only fall or move high→medium, except gap 23's PowerShell/Go tightening, which may raise some. Read every raised finding in a 20-sample.
- **Carry-over:** the WS3/WS4 carry-over checks in TRACKER (Java external-contract overrides) still pass.

## WS21 — Constructor and super edges (gaps 12, 13) · effort S–M

- **Gap 12:** emit a construction (`new T()` in C#, `T()` in Swift, `T.new` in Ruby) as a call site to the type, and add `constructor_names` rows that map it onto `T.T`/`init`/`initialize`. Start with C#, Swift and Ruby (S), then PHP, Scala, Groovy, Solidity, PowerShell, ObjC and F#.
- **Gap 13:** record delegation sites:
  - Java `this(…)`/`super(…)`;
  - JS/TS `super()`;
  - C# `: base()`/`: this()`;
  - C++ member-initializer base calls;
  - Solidity base-constructor modifiers;
  - Ruby bare `super`.

  Each resolves through the existing base chain (`resolve_via_bases`), so no new resolution logic is needed.
- **Acceptance:**
  - **Edges:** every new edge goes to a constructor (`init`/`initialize`) or a base member.
  - **Review:** read a sample of 30 new high edges.
  - **Dead code:** constructors that were reported dead and are now reached drop out of the report. Count them.

## WS22 — Callable extraction parity (gaps 7, 8, 24) · effort S–M

- **Gap 7:** tag bodiless interface and abstract members `abstract`, as TS, Swift, Scala and Dart already do:
  - Java/C#/Kotlin/Groovy interface methods without a body, and `abstract` methods;
  - Go interface method specs;
  - PHP `abstract`/interface methods;
  - C++ pure virtual (`= 0`);
  - ObjC `@protocol` methods;
  - Solidity interface functions;
  - Fortran `interface` blocks;
  - Elixir `@callback`;
  - Rust trait methods without a default body.

  Duck fan-out and intra-file preference already handle the tag. Acceptance: fan-out sets shrink and nothing new is refused; check that no set empties, since the narrowing is one-sided.
- **Gap 8:** JS/TS class-field arrow and function expressions (`handler = () => {…}`, `handler = function () {…}`) become methods of the class, reusing `try_record_named_fn`. Acceptance: the `no-enclosing-callable` count falls in React and Angular repos, and `this.handler()` binds.
- **Gap 24:** add a `call_expression` arm to objc.rs that records plain C calls, shared with the C plugin's arm where possible. Acceptance: C helpers defined in `.m` files stop being reported dead in the ObjC corpus repos.

## WS23 — Receiver typing (gaps 10, 26, 28, 16) · effort S–M · depends on WS19 gap 4

- **Gap 10:** type Go method receivers (`func (s *Server)`), and same-package parameter types written without a package qualifier. Reuse the Go parameter reader, not the type-first reader that parses `q *Server` back to front. Acceptance: the reproduced fixture binds `s.zip()` to `Server.zip` at high.
- **Gap 26:** read declared return types for Java, C#, Dart, Groovy, C, C++ and ObjC, so `var t = make(); t.work()` and `getTool().work()` bind. This needs WS19 gap 4's corrected map key. Fix or remove the false comment at `type_hints.rs:~2650`.
- **Gap 28:** treat JS/TS locals as a typing source: `const t = new Svc()`, `const t: Svc = …`, and `let`/`var` likewise. Use the scoped `FileFacts::locals` spans, not file-wide last-write-wins. Acceptance: TS medium fan-out falls in the corpus; check by reading a sample of new highs.
- **Gap 16:** Swift and Groovy typed cross-file overload lookups expand to the file's overload set. Either add them to `OVERLOAD_ARITY_LANGS`, if they record arity (see WS28), or expand without narrowing at medium.

## WS24 — Framework and harness parity (gaps 14, 22, 30) · effort S–M (gap 30 M–L)

- **Gap 14:**
  - Add `kotlin` to the 23 enumerating Java rules whose triggers are annotations or bases the Kotlin plugin already records (`@Scheduled`, `@KafkaListener`, `@Bean`, `@PostConstruct`, JAX-RS, Micronaut, `junit-sources` `@MethodSource`, …).
  - Give the Android lifecycle rules Kotlin matchers, and fix the stale gap text.
  - Add a Kotlin fixture per rule.
  - Name a Kotlin corpus app in `benchmark.sh` APPS, or list the rule in `APPS_UNVERIFIED`, so docs-check passes.
- **Gap 22:**
  - Add `member_access_expression` to C# `IDENT_KINDS`, so `app.MapGet("/a", Handlers.List)` registers.
  - Thread `ExtractCtx` through swift.rs, so registrar verbs (`aws-lambda` `run`) can match.
- **Gap 30:** JS/TS test harnesses, in two steps.
  - *(a)* Turn `jest`/`vitest`/`mocha`/`node:test`/`playwright` from gap-only into enumerating rules over named functions in test files, plus the files' top-level helpers as test roots.
  - *(b)* Make `it(…)`/`test(…)` callbacks callables, which is the closure-based test-framework follow-up already logged for Jest/Vitest, RSpec, Dart and kotest. Do (b) after WS22 gap 8, which adds the same machinery.

## WS25 — Builtins, reflection and detection (gaps 17, 18, 19, 20) · effort S

- **Gap 17:** `module_is_stdlib` splits qualified heads on `::`, `/` and `\` as well as `.`, so `std::fs::read_to_string`, `clojure.string/join` and `\strlen` reach the stdlib tables.
- **Gap 18:** add the Java and C# reflective calls to the dynuse screen tables. These names belong to the builtin tables, so they are allowed there.
- **Gap 19:** match a shebang on the interpreter's basename as a whole token, including after `env`, never as a substring of the line. Map `pwsh`/`powershell` to PowerShell and `ts-node`/`tsx`/`deno` to TypeScript. Make sure the `zsh` needle works.
- **Gap 20:** add filename detection next to extensions:
  - CMake: `CMakeLists.txt`;
  - Starlark: `BUILD`, `BUILD.bazel`, `WORKSPACE`, `BUCK`;
  - Ruby: `Gemfile`, `Rakefile`, `config.ru`;
  - Groovy: `Jenkinsfile`.

  These are file-format names in the plugin's detection table, like extensions, not code names.
- **Acceptance:**
  - Detection changes are A/B'd on files analysed: a newly detected file must parse without errors in at least 95% of cases.
  - The stdlib split moves `<external>` exit nodes to `<stdlib>` only under `--include-*`; the default graph does not change.

## WS26 — Function values as references (gap 25) · effort M per plugin

This is the largest dead-code false-positive class. The shared helpers exist (`registrar.rs:517, 563`). Wire each plugin's value-position arguments (`map(fn)`, `setTimeout(cb)`, `::topHelper`, `sort.Slice(x, less)`) into `RefRecord::value_ref`.

- **Order:** by corpus reach: JS/TS → Go → Kotlin → C/C++ → Ruby → PHP → Swift → Dart → Scala → Lua → the rest.
- **One language per commit,** each with its own A/B.
- **Acceptance per language:**
  - dead findings fall;
  - `--reference-edges` gains edges;
  - the default graph is unchanged, because reference edges are dropped by default.

## WS27 — Locals specs (gap 27) · effort M per language

Add `locals.rs` specs so parameters and locals shadow, giving `shadowed-by-local` and no wrong `high`. Order: Go → C# → C → C++, then Groovy, R, Julia, Haskell, Clojure, Nix, Starlark, Solidity, F#, Fortran and Perl. Watch perf in each A/B: WS14's locals walk was where 0.11.0's regression came from (fixed in 711bd39).

## WS28 — Arity narrowing (gap 29, plus 14 #4) · effort M

Order: C# → Kotlin → Swift → Groovy → TS → others. For each language:

- the plugin sets `RefRecord::arity`;
- `accepts_arity` learns that language's `params`/`vararg`, defaults and argument labels;
- the language joins `OVERLOAD_ARITY_LANGS`.

Separately (S): let cross-file Java/Scala overloads narrowed to one by arity reach `high`, as the `CertainCtx` text already promises.

## WS29 — Docs and re-score · effort S

- Apply the "Doc corrections" sections of `matrix/*.md` to CLAUDE.md/README; the orchestrator does this, never a subagent. Example: CLAUDE.md says a guessed receiver type "is never `high`", which until WS19 was true cross-file only.
- Re-run the category agents for 02, 05, 06, 07, 11, 12 and 14 against the new tree and regenerate `PARITY_MATRIX.md`, so the gain is measured, not asserted.
- Resolve the ~80 unverified cells while doing so.

---

## Summary

| WS | Gaps | Effort | Main effect |
| --- | --- | --- | --- |
| WS19 | 1, 2, 3, 4, 9 | S–M | wrong `high` → `medium` or correct; ~30 languages touched through shared code |
| WS20 | 5, 6, 11, 15, 21, 23 | S each | dead-code false positives removed or capped |
| WS21 | 12, 13 | S–M | constructor and `super` edges in 10+ languages |
| WS22 | 7, 8, 24 | S–M | abstract tagging, JS/TS class-field handlers, ObjC C calls |
| WS23 | 10, 26, 28, 16 | S–M | receiver typing for Go, JVM/.NET returns, JS/TS locals |
| WS24 | 14, 22, 30 | S–M (30: M–L) | Kotlin/Java framework parity, C#/Swift registrars, JS/TS harnesses |
| WS25 | 17, 18, 19, 20 | S | stdlib split, reflection screen, shebang and filename detection |
| WS26 | 25 | M × language | function values as references (largest dead-code FP class) |
| WS27 | 27 | M × language | parameters/locals shadow in Go, C#, C, C++ … |
| WS28 | 29 | M | arity narrowing for C#, Kotlin, Swift, Groovy, TS |
| WS29 | — | S | doc corrections, re-score the matrix |

**Suggested release cuts:**

All of WS19–WS29 are bug fixes (wrong or missing edges, false dead
findings), so they ship as 0.11.x patches with Compatibility notes:

- **0.11.1:** WS19–WS22. Confidence and dead-code correctness, plus the cheap edge additions.
- **0.11.2:** WS23–WS25.
- **WS26–WS28:** ship per language as they land, as further patches.
