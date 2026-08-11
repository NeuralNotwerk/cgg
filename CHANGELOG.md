# Changelog

All notable changes to `cgg` are documented here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/); this project is
pre-1.0, so the resolver's edge set may grow between releases (it only
ever grows in default mode — see *Compatibility* below).

## [Unreleased]

### Performance — the 0.6.0 regression, diagnosed

No fix yet. What follows corrects the cause 0.6.0 guessed at, so the next
attempt does not start from the wrong place.

**It is lost parallelism, not extra work.** Measured on `c-jq` with
`/usr/bin/time -v`, medians of 7, binaries alternated:

| `--jobs` | 0.5.0 wall | 0.6.x wall | 0.5.0 CPU | 0.6.x CPU |
| --- | --- | --- | --- | --- |
| 1 | 290 ms | **270 ms** | 300 ms | **260 ms** |
| 2 | 200 ms | 250 ms | 300 ms | 290 ms |
| 4 | 170 ms | 230 ms | 330 ms | 330 ms |
| 8 | 120 ms | 160 ms | 370 ms | 410 ms |

At `--jobs 4` the CPU time is **identical** and the wall time is 35%
worse. At `--jobs 1`, 0.6.x is *faster* on both. The work got cheaper; the
scheduling got worse.

**0.6.0 blamed `ExtractCtx`. That was wrong.** Threading the extraction
switches replaced an atomic load with a field read, which is why
single-threaded got faster. It cannot explain a regression that only
appears with more than one worker and costs no extra CPU.

**The cause is `ThreadPool::install(whole_pipeline)`**, which runs the
driver *on* a pool worker instead of on the calling thread. 0.5.0 drove
from the main thread because its pool was global. Rebuilding 0.6.1 with
`build_global` recovers most of it — +8-12% against 0.5.0 rather than
+33-35% — but that is exactly the global pool whose one-shot
initialisation made `--jobs` silently ignored after the first call, so it
cannot simply be reverted.

Scaling confirms the shape: `install` at `2N` threads matches
`build_global` at `N`. Achieved speedup at `--jobs 8` is 3.17x for 0.5.0
against 2.56x for `install`.

**A tried and rejected fix:** keeping the driver on the calling thread and
wrapping each of the five parallel regions in `pool.install` individually.
That is *worse* — +39-50% against 0.5.0 — because the repeated handoffs
cost more than the single one they replace. Recorded so nobody spends the
afternoon on it twice.

**Workaround today:** pass roughly double the threads. `--jobs 16` on this
64-core box performs like 0.5.0's `--jobs 8`.

Note for anyone re-measuring: the whole-corpus figure is +4-6.8%, not
+33%. `c-jq` is the worst case, chosen here because a large effect is
easier to attribute. Machine load moves these numbers several points, so
compare on an idle box.

## [0.6.1] - 2026-08-11

**First release published to crates.io.** `cargo install cgg` works, and
`cgg` is usable as a library dependency. 0.6.0 shipped only as a git tag.

### Packaging

- **Published to PyPI as `cgg-callgraphgenerator`** — `pip install
  cgg-callgraphgenerator`, then `import cgg`. PyPI's `cgg` belongs to an
  unrelated GGUF tool with 49 releases, so the short name was never
  available; the long form spells out what CGG stands for. That package
  also ships a top-level `cgg` module, so **the two must not share an
  environment** — both write to `site-packages/cgg/` and pip will not stop
  you. Documented in both READMEs.

  The wheel is `manylinux_2_17` + `abi3-cp39`: built in maturin's CentOS 7
  container, because PyPI rejects plain `linux_x86_64` wheels and a wheel
  built on a modern box links a glibc newer than most users have. One
  wheel covers every CPython from 3.9 up. 10.1 MB.
  `scripts/publish-python.sh` does the build, the clean-venv test and the
  upload.

- All six library/binary crates now carry the metadata a registry listing
  needs — `repository`, `homepage`, `keywords`, `categories`,
  `rust-version` — inherited from the workspace so the listing cannot
  drift from it, plus a per-crate `README.md`. The root README lives
  outside every package directory, so without one each crates.io page
  would have rendered blank.
- The five internal library crates say so on their own front page: they
  are published only so `cgg` can depend on them by version, and their
  APIs change freely between minors to serve that one consumer.
- `cgg-py` and `cgg-ffi` stay `publish = false`. The artifact anyone
  wants from those is a wheel and a shared library, not a crate.


### Added

- **`crates/cgg-ffi` — a C ABI.** One shared library (`libcgg.so`) or
  static archive (`libcgg.a`) serves C, .NET, Java, Go, Ruby and anything
  else with an FFI, so a binding is source rather than another native
  artifact per language per platform.

  Six functions. Options cross as a JSON document and results as strings,
  which is the load-bearing decision: adding a cgg flag adds a JSON key,
  not an entry point, so **the ABI does not change when cgg gains a
  feature** and no wrapper needs rebuilding. It is affordable because
  rendering is nearly free next to analysis — 6.5 ms for `to_json()` and
  1.3 ms for `to_mermaid()` against 137.7 ms of analysis on cgg's own
  tree. `cgg_analyze` returns an opaque handle rather than a rendered
  string, so mermaid *and* JSON *and* the metrics cost one analysis.

  Output is byte-identical to the CLI across all four formats, asserted by
  test. Statically linked, a C program depends on nothing but libc and
  libgcc — the CLI's self-contained promise survives the boundary.

  Unknown option keys are an error rather than ignored: C callers
  hand-write the JSON, so a silently-dropped `"hopz"` is the failure this
  boundary is most exposed to.

- **The Rust library needs only one dependency now.** `cgg` re-exports
  `OutputFormat`, `Result`/`Error` and the graph types, so a consumer no
  longer has to name `cgg-format` and `anyhow` in its own manifest just to
  call the API — which also stopped their versions from drifting from the
  ones cgg was built against.

- `RunOptions` derives `Serialize`/`Deserialize`, with `#[serde(default,
  deny_unknown_fields)]`, which is what the JSON options boundary is built
  on.

### Fixed

- **`cgg::analyze` no longer leaks memory per call.** 0.6.0 leaked ~161
  bytes every time it was called. `cgg_resolve::type_hints` had a
  `leak_str` helper that `Box::leak`ed a copy of every parameter name and
  type, with a comment calling that "acceptable because we're in a
  short-lived analysis pass" — true while the pipeline was private to a
  binary that analyzed once and exited, and false the moment 0.6.0 made it
  a library, a Python module and a C ABI callable in a loop. A host
  process analyzing on a loop grew without bound.

  The leak existed only to dodge a borrow conflict: the strings are slices
  of `def.signature_hint`, and `facts` is mutably borrowed later in the
  same function. They now go into an owned `Vec` that the function drops.
  **The allocation count is unchanged** — the leaked version called
  `to_string()` too — so this costs nothing; the strings are simply freed.

  Verified under valgrind: 0 bytes definitely lost on `cgg-walk`,
  `cgg-format`, a single 467-line file and all of `./crates`, where 0.6.0
  lost 161 bytes in 28 blocks. Graph output is byte-identical on
  `./crates`, `c-jq`, `go-fzf` and `python-flask`, and still deterministic
  across `--jobs` 1–32.

  Diagnosis note, because it is the reason 0.6.0 shipped with this:
  **`mimalloc` hides leaks from valgrind entirely.** With the global
  allocator in place valgrind reported no leak summary at all; the 161
  bytes only appeared once it was removed. The CLI was never affected — it
  analyzes once and exits.

- `docs-check.py` check 9: `Box::leak`, `.leak()` and `mem::forget` in the
  pipeline crates must be listed in `ALLOWED_LEAKS` with a reason. A leak
  justified by "the process is about to exit" has to be re-checked when
  that stops being true, and nothing was re-checking. The one remaining
  entry is `profile.rs`, which is bounded by `&'static str` span literals
  and compiled out of release builds.

### Performance

Flat. `scripts/perf-compare.sh` against the previous commit, median of 7
over the standard 9-repo set: **2258 ms → 2260 ms, +0.1%** — far inside
the ~1–1.5% noise floor. Expected: the leak fix changes where the strings
are freed, not how many are allocated.

The +4–6.8% regression 0.6.0 introduced against 0.5.0 is unchanged by
this and is still open. (Its cause was later re-measured — it is the
per-call pool's `install`, not the `ExtractCtx` threading. See the
Unreleased entry.)

## [0.6.0] - 2026-08-11

The pipeline is a library, and there is a Python module on top of it.

```python
import cgg
g = cgg.analyze("./src")
print(g.to_mermaid())
```

The 1,035-line `run()` inside `main.rs` was private to a bin-only crate,
so nothing outside the binary could invoke it. `crates/cgg` now has a
`[lib]` target alongside its `[[bin]]`, and `crates/cgg-py` is a PyO3
extension module over it. Both front ends call `cgg::analyze`, so the
resolver ordering that CLAUDE.md calls load-bearing exists in exactly one
place.

**The CLI is unchanged.** Graph output is byte-identical to 0.5.0 across
all four formats, and so is the stdout/stderr interleaving, the exit
codes, and every advisory's `-q` gating. The binary links no libpython
and grew 48 KB.

### Added

- `cgg::analyze(&RunOptions) -> RunOutcome`, performing no I/O beyond
  reading source — no writes, no stdout/stderr, no `process::exit`.
  Everything a run writes comes back as an ordered `Vec<Emission>`.
- `crates/cgg-py`: `import cgg`. Every option that changes the graph is a
  keyword argument, with one rename — `entry_nodes=True` rather than
  `--no-entry-nodes`, same default. Four renderers, `.callables`,
  `.edges`, `.files`, `.metrics`, `.notices`, `.jobs`, `to_dict()`,
  `callable()`, `callers_of()`, `callees_of()`. `abi3-py39`, so one wheel
  per platform serves every CPython >= 3.9. Build with
  `scripts/build-python.sh`.
- `docs-check.py` checks 7 and 8: the self-analysis showcase filter must
  agree across every file that names it and must still produce a graph
  spanning three or more crates, and every `RunOptions` field must be
  reachable from `cgg-py` or listed as deliberately deferred.

### Fixed

Three bugs that a binary running one analysis per process could not
reach, and a second caller can:

- **`--jobs` was ignored after the first analysis in a process.**
  `build_global()`'s error was dropped with `let _ =`, and the global
  pool can only be set once, so later calls silently reused the first
  call's thread count. Now a per-call pool entered with
  `ThreadPool::install`, with `RunOutcome::jobs` reporting
  `rayon::current_num_threads()` read inside it.
- **One project's framework rules suppressed the next project's.**
  `set_extra_registrar_verbs` wrote to a `OnceLock` where only the first
  call took effect, so the second project analyzed in a process could
  lose its entry points. The switch now travels in a per-run
  `cgg_lang::ExtractCtx`; `DEADCODE_SIGNALS`, `EXTRA_REGISTRAR_VERBS` and
  `HAS_EXTRA_VERBS` are gone.
- **`--write-roots` called `std::process::exit(0)` inside the pipeline.**
  Harmless in a binary; linked into CPython it would terminate the
  interpreter with no traceback. It returns the baseline now.

### Performance

**cgg is roughly 4–5% slower on the standard 9-repo comparison set.**
Measured with `scripts/perf-compare.sh` against 0.5.0, median of 7:

| repo | 0.5.0 | unreleased | delta |
| --- | --- | --- | --- |
| rust-ripgrep | 236 ms | 238 ms | +0.8% |
| python-flask | 129 ms | 130 ms | +0.8% |
| js-express | 83 ms | 83 ms | +0.0% |
| go-fzf | 219 ms | 226 ms | +3.2% |
| c-jq | 145 ms | 182 ms | +25.5% |
| cpp-spdlog | 308 ms | 308 ms | +0.0% |
| csharp-serilog | 115 ms | 120 ms | +4.3% |
| swift-alamofire | 238 ms | 242 ms | +1.7% |
| cpp-nlohmann-json | 724 ms | 756 ms | +4.4% |
| **TOTAL** | **2197 ms** | **2285 ms** | **+4.0%** |

Three runs of the full set gave totals of **+4.0%, +5.5% and +6.8%**, with
`c-jq` at +25.5%, +21.5% and +25.9%. The direction is reproducible and
well above the ~1–1.5% noise floor, so this is a real regression, not
jitter. The spread tracks machine load, which was 1.0–2.8 across the
three runs — `perf-compare.sh` warns that a loaded machine invalidates
the comparison, so treat +4% as the floor and re-run on an idle box
before tagging.

The graph is byte-identical to 0.5.0 on every repo in the table, so the
cost buys nothing in output — it is the price of the correctness fixes
above. `parse` CPU rises ~7% while wall time rises more, so the loss is mostly
parallel efficiency rather than extra work.

> **This entry's attribution was wrong.** It blamed the `ExtractCtx`
> threading on a bisect run against a loaded machine. Re-measured on a
> quieter one, the cause is `ThreadPool::install` running the driver on a
> pool worker, and `ExtractCtx` is if anything a small *win*. See the
> Unreleased entry for the corrected analysis.

**Recovering this is open follow-up work.** It is a deliberate trade for
now: the globals it replaced made a second analysis in one process return
wrong answers, and correctness at 4% is a better default than speed that
silently lies.

### Compatibility

**No change to the CLI.** Every flag, every output format, every exit
code and the stdout/stderr interleaving are what 0.5.0 produced, verified
byte-for-byte on `./crates` and on the 9-repo comparison corpus. The
default graph does not grow: this release adds no resolver and no rule.

**`cgg-lang` has a breaking API change.** `LanguagePlugin::extract` takes
`&ExtractCtx` as its first argument, and `set_deadcode_signals`,
`deadcode_signals`, `set_extra_registrar_verbs` and `is_registrar_verb`
are gone from the crate root — `ExtractCtx::is_registrar_verb` replaces
the last. All 44 in-tree plugins are updated. An out-of-tree plugin must
add the parameter; `ExtractCtx::plain()` is the no-switches context that
plugin tests use.

**The Rust library API is new, and pre-1.0.** `cgg::analyze`,
`RunOptions`, `RunOutcome`, `Emission` and `cgg::emit` are public as of
this release and may change in any 0.x minor — `RunOptions` in
particular gains a field whenever a graph-affecting flag is added, and it
is deliberately *not* `#[non_exhaustive]`, because `From<&Cli>`
destructures it with no `..` rest so that a new flag fails to compile
until it is routed. Pin an exact minor if you depend on it.

## [0.5.0] - 2026-08-07

Two releases in one. The framework rule table went from 51 rules to 394
across 36 languages, and then the tool was taught to use the machine it
runs on. **Whole-corpus latency fell from 3,202s to 205s.**

### Performance

0.4.2 → 0.5.0, measured on the **shipped default worker count** — which
is what you actually get, not a tuned best case:

| repo | 0.4.2 | 0.5.0 default | 0.5.0 `--jobs 32` |
| --- | --- | --- | --- |
| app-druid-jaxrs | 143.7 s | **18.6 s** (7.7×) | 8.0 s (18×) |
| c-redis | 41.4 s | **7.9 s** (5.2×) | 2.6 s (15.7×) |
| rust-ripgrep | 0.53 s | **0.27 s** (2.0×) | 0.40 s |
| app-django-netbox | 4.07 s | **3.58 s** | 3.74 s |
| python-flask | 0.18 s | **0.13 s** | 0.16 s |

**Two numbers, on purpose.** The default is deliberately conservative —
half the physical cores, capped at 8 — so cgg is a good guest on a shared
host. On a large tree more workers genuinely help, and `--jobs 32`
roughly doubles the default again. Small repositories are *faster* at
the default, where thread-spawn cost dominates.

The corpus-wide figure below was measured before that cap existed, at one
worker per logical CPU. It is the ceiling the parallelism reaches, not
what an untuned run produces:

**Whole corpus: 3,202 s → 205 s (−93.6%)** over 103 repositories, 100 of
them comparable. 73 of 100 repositories are more than 5% faster. **None
is more than 10% slower in absolute terms** — the 9 that show a
percentage regression are
all sub-200ms runs where a few milliseconds of process startup dominates.

**`zig-zig` could not be analysed at all before this release.** It
exceeded a 1,800-second timeout on 0.4.2 and now completes in 498s,
producing 344,807 callables. That is a capability change, not a speed
one, and it is why the raw corpus node and finding totals jump: excluding
that one repository, nodes move +8,107 and dead-code findings move
**−9,146**. `dart-flutter` and `erlang-otp` still exceed 1,800s on both
releases and are excluded from every total here.

Standard 9-repo comparison set:

| repo | latency | nodes | edges | entry | dead |
| --- | --- | --- | --- | --- | --- |
| rust-ripgrep | 399→249 ms | 2,906 | 7,106 | 0 | 1,429→1,420 |
| python-flask | 158→118 ms | 1,732→1,736 | 1,744→1,752 | 444→452 | 174→131 |
| js-express | 102→75 ms | 546→548 | 393 | 0 | 65→66 |
| go-fzf | 263→229 ms | 1,615 | 10,291 | 0 | 131 |
| c-jq | 123→130 ms | 1,119 | 21,724 | 0 | 425→424 |
| cpp-spdlog | 350→315 ms | 1,357 | 11,412 | 0 | 809 |
| csharp-serilog | 118→121 ms | 1,689 | 1,864 | 0 | 663→658 |
| swift-alamofire | 414→313 ms | 2,538 | 6,737 | 0 | 946→826 |
| cpp-nlohmann-json | 911→788 ms | 5,567 | 7,075 | 0 | 2,109→2,105 |
| **TOTAL** | **2,839→2,339 ms** | **19,069→19,075** | **68,346→68,354** | **444→452** | **6,751→6,570** |

Reproduce with `scripts/compare-release.py OLD_BIN NEW_BIN`. Note that
`--jobs 1` is the setting for numbers being published; the default runs
repositories concurrently, which is sound for an A/B delta but not for an
absolute latency claim.

### Parallelism

cgg parallelised exactly one phase before this release — the per-file
parse loop — and everything after it ran on one core. On Druid that was
110% CPU on a 64-core machine for 150 seconds.

Five phases now run in parallel, each verified to produce a
**byte-identical graph** at any thread count:

- **Cross-file resolution.** The single biggest win, and the reason Druid
  went from 150s to 8s. A 637-line per-file loop that read shared indexes
  and wrote only its own output.
- **Intra-file linking and type propagation.** Per-file and independent.
- **Framework matching**, fanned out across rules once the per-language
  indexes are hoisted out of the loop.
- **Audit serialisation** — 569ms of a Druid run spent on one core
  serialising JSON. Output is byte-identical to `to_writer_pretty`.

**The allocator was the ceiling.** Thread scaling stopped paying after
four cores, and the profiler showed why: the *same work* cost 6.8s of CPU
at `--jobs 4` and 10.6s at `--jobs 64` — 56% more CPU to produce
identical output. That is the system allocator serialising under
extraction's load, which allocates a `String` per name, per reference,
per qualified path, on every worker at once. **cgg now uses `mimalloc`
as its global allocator**; on its own that took Druid from 138s to 106s.

Two ordinary inefficiencies turned up in the same pass and are worth
naming because neither needed parallelism to fix: `known_refs` was
rebuilt **once per file** from identical data (1,273 files × ~10,000
names on netbox), and each file's audit record was located by linear
scan, making that step O(files²).

### Added: `--profile`

`RunMetrics::phases` records four coarse buckets. That stopped being
enough once "link" grew a type propagator, a cross-file resolver, an FFI
linker, a descriptor linker, a framework engine with six matchers and a
dead-code pass — a 25% regression inside that bucket is invisible from
outside it.

`--profile` prints a per-span breakdown. It is **compiled out of release
builds**: `span()` is `#[cfg]`-reduced to a constant `None`, so there is
no atomic load, no clock read and no branch to argue about. The numbers
this project publishes are measured on release binaries, and a profiler
that *could* perturb them is a profiler that makes those numbers
arguable. Debug builds collect by default. For release-speed numbers with
attribution, build with `RUSTFLAGS="-C debug-assertions=on"`.

Spans accumulate into per-thread-cached atomics rather than a locked map.
The first version used a global mutex and reported 263% CPU on a run the
plain binary did at 212% — it was measuring the contention it caused.

### Framework coverage: 45 → 343 frameworks, 12 → 36 languages

The rule table went from 51 to 394 `(id, language)` rules.

**The failure this fixes is silence.** An unrecognised framework
previously produced *no coverage line at all* — a real aiohttp app with
two routes reported `recognised (none)` and `seen, no rules (none)`, so
the disclosure pointed the reader at an empty list. Most of the new rules
are deliberately **detect-only**: they enumerate nothing and carry a
`gap` string naming the concrete construct to inspect by hand. A rule
that enumerates badly is worse than one that declines and says why.

Six detection gaps that were defects are closed, each traced to a
specific real-world idiom:

| framework | what was missed |
| --- | --- |
| `actix-actor` | `rust.rs` never populated `base_types`, so no base-type rule could fire on Rust at all |
| `chi` | handlers wrapped in `chain.ToHandlerFunc(...)` |
| `sinatra` | `get "/x" do … end` — Ruby hangs the block off the call's `block` field, never the argument list |
| `worker-threads` | worker modules that identify *themselves* rather than being named at a literal spawn path |
| `spring-messaging` | Spring AMQP puts `@RabbitListener` on the class and `@RabbitHandler` on the method |
| `nestjs-schedule` | nothing — the rule was correct; the corpus app scheduled via `SchedulerRegistry` |

New trust boundaries: **`TrustKind::Public`** for Solidity, where the
language *is* the framework — every `public`/`external` function is
callable by any address on the chain. 1,495 entries on OpenZeppelin, and
dead-code false positives there fell 37%. **`TrustKind::Ffi`** is now
produced, for `#[no_mangle]`/PyO3/wasm-bindgen exports.

**Descriptor → implementation linking** (`Via::Descriptor`). cgg parses
`.proto` and the languages that implement it, so it can now link
`service Greeter { rpc SayHello }` to the Go method serving it — an edge
neither file references. `.proto` rpcs became callables to make this
possible. The match requires the implementing type to *name the service*:
method name alone matches `Get` everywhere, and a missing edge is a gap
while a wrong edge is a lie about where control goes.

### Fixed: two nondeterminism defects, one of them shipped in 0.4.2

> **Read this section even if you skip the rest.** cgg's central claim is
> that the same input yields the same graph. Two code paths broke that,
> and **one of them is in the released 0.4.2 binary**. Neither produced a
> wrong edge *set* — only a varying edge or node *order* — which is
> precisely why they survived: a spot check passes, and only a byte-diff
> of two runs shows it.

- **`--dead-code` produced a different graph on every run, on any
  codebase with traits.** `--dead-code` force-enables
  `--dynamic-dispatch`, and `dispatch::fanout` iterated a `HashMap` to
  emit its declaration→implementation edges. Rust's `RandomState`
  reseeds per process, so the fan-out edges came out in a different order
  each time. Four runs over cgg's own source produced four different
  graphs. **This is present in 0.4.2 and every release that had
  `--dynamic-dispatch`.** If you diffed dead-code output between runs and
  saw churn, this was why. Fixed by sorting the keys.
- **`-n 0 --max-paths N` produced a different graph on every run when the
  cap truncated.** Entry points were walked in `HashMap` order, so *which*
  paths survived the cap varied. Node counts swung 7–9 across four runs of
  the same command. Without truncation the result was identical either
  way, which is why it hid. Fixed by sorting the entry set.

Both are now covered by `crates/cgg/tests/determinism.rs`, and both tests
were verified to *fail* against the unfixed code — a regression test that
has never seen the bug it guards is a guess.

**The determinism test that shipped alongside the parallelism work did
not catch either of these**, because its fixture had no trait with
multiple implementations and never triggered path truncation. That is the
more useful lesson than the bugs themselves: a green determinism suite is
only evidence for the shapes it actually exercises.

### Fixed: four plugin bugs that silently disabled whole languages

> **Read this section.** Each of these made a language's framework
> detection impossible, with no error and no warning — the coverage table
> simply reported nothing and looked correct doing it.

- **Lua recorded every `require` as the literal string `"("`.**
  `arguments.child(0)` returns the `(` token; it needed `named_child(0)`.
  Kong now detects across 744 files where it previously detected none,
  and Lua cross-file resolution gains real edges.
- **C and C++ ignored system includes entirely.** Only quoted includes
  were recorded, so `#include <gtest/gtest.h>` produced nothing and no
  C/C++ rule keyed on a system header could fire. System includes are now
  recorded under a distinct `system-include` kind, so the cross-file
  resolver still correctly ignores them.
- **Erlang recorded only `-import(...)`.** `-behaviour(gen_server)` — the
  only way Erlang declares an OTP contract — was invisible.
- **`FrameworkRule::has_matchers()` omitted `methods`.** Any methods-only
  rule (the structural-typing escape hatch that exists precisely for Go
  interfaces and Elixir OTP behaviours) filed itself under "seen, no
  rules" no matter how many entries it minted — a rule reporting a gap it
  did not have. Elixir went 0 → 359 entries on a real Phoenix app.

Registrar capture was added to the Elixir, Perl and Clojure plugins,
which had none: Phoenix's router now enumerates 151 routes on Plausible,
and Mojolicious 283 on its own tree. Lua was assessed and deliberately
**not** changed — its two rules are declared-gap rules, so capture there
would have had no consumer.

### Added: verification that the table cannot rot

- **`tests/detect_prefixes.rs`** synthesises, for every rule, a file
  importing that rule's own first `detect` prefix, and asserts the rule
  fires. A rule whose prefix does not match how the language actually
  writes that import is worse than no rule, because the coverage table
  then implies the framework was considered and found absent. This test
  found the Lua, C/C++ and Erlang bugs above.
- **`tests/determinism.rs`** asserts the graph is identical at 1, 2, 8
  and 32 threads. It compares *structure*, not bytes: the JSON and audit
  documents embed per-run timings, so a naive hash comparison reports
  nondeterminism that is not there.
- **`scripts/sync-app-manifest.py`** derives the corpus manifest from
  measurement rather than by hand, and **`APPS_UNVERIFIED`** in
  `benchmark.sh` states out loud which rules no real application
  exercises, with a reason each.

### Changed

#### Dependency disclosure: `mimalloc`

**cgg takes a new dependency in this release** — the first since
`b2afced` removed the update check and every network dependency. Stated
plainly because a dependency added for speed is still a dependency:

| | |
| --- | --- |
| crate | `mimalloc` 0.1.52 → `libmimalloc-sys` 0.1.49 |
| licence | **MIT** (both), already on `deny.toml`'s permissive allow-list |
| what it is | Microsoft's general-purpose allocator, set as cgg's `#[global_allocator]` |
| ships C | yes — `libmimalloc-sys` bundles mimalloc's C source and compiles it at build time via `cc` |
| new transitive deps | none. `cc` 1.2.62 was already in `Cargo.lock` |
| generation | **mimalloc v3.3.2** — upstream's recommended line, not the v2 "stable" line. Selected by leaving the `v2` feature off; every number above was measured on v3. Take `features = ["v2"]` for the conservative choice |
| features | none. `override` is **off**, which matters: mimalloc serves only Rust's `Global`, so the 44 tree-sitter C parsers that handle untrusted input keep glibc's hardened allocator |

**It does not change cgg's build requirements.** 45 crates in the graph
already required `cc` — every tree-sitter grammar, plus
`crates/cgg-lang/build.rs` compiling a vendored `parser.c` for the Smithy
grammar — and `skills/cgg-install/SKILL.md` already documents the C
toolchain check. A from-source install without a C compiler was already
broken before 0.5.0. Cost measured: +4.3s on a clean release build, and a
416 KB static library. Nothing changes at runtime; cgg still makes no
network requests.

**One advisory exists and does not apply.** RUSTSEC-2022-0094
(`unsound`, bad alignment) is patched in mimalloc >= 0.1.39; the pin is
0.1.52. `cargo deny check` passes all four checks — advisories, bans,
licenses, sources.

What it buys: thread scaling stopped paying past four cores because the
system allocator serialised under extraction's allocation load. mimalloc
alone took Druid from 138s to 106s, and is a substantial part of the
overall 15× speedup.

If you would rather not ship it, removing the `#[global_allocator]`
attribute in `crates/cgg/src/main.rs` and the two `Cargo.toml` entries
restores the previous allocator; everything else in this release stands
without it.

#### Other changes

- `--profile` is a new flag (see above).
- `scripts/compare-release.py` and `scripts/sync-app-manifest.py` are new
  release tooling.

### Compatibility

The default graph grows, as it only ever does: +8,107 nodes and +20,581
edges across the corpus, excluding the repository that previously timed
out. Dead-code findings *fall* by 9,146 — the new entry points give
previously-unreferenced handlers a caller.

## [0.4.2] - 2026-08-06

One real application per framework, and every detection gap that was a
defect closed. 0.4.1 checked the documentation against the code; this
release checks the *framework rules* against real applications, which
found six gaps and two counting bugs that no fixture had exercised.

### Performance

0.4.1 → 0.4.2: latency **flat**.

Measured against **0.4.0**, not 0.4.1 — 0.4.1 was never committed
separately, so it is not a ref that can be checked out and built. 0.4.1
was itself flat against 0.4.0, so the 0.4.1 → 0.4.2 delta is flat too.
Two full runs, because a single run's per-repo numbers did not reproduce:

| repo | 0.4.0 | 0.4.2 run 1 | 0.4.2 run 2 |
| --- | --- | --- | --- |
| rust-ripgrep | 429 / 421 ms | 430 ms | 426 ms |
| python-flask | 158 / 161 ms | 166 ms | 162 ms |
| js-express | 107 / 100 ms | 99 ms | 101 ms |
| go-fzf | 273 / 274 ms | 291 ms | 283 ms |
| c-jq | 155 / 149 ms | 149 ms | 152 ms |
| cpp-spdlog | 308 / 303 ms | 305 ms | 318 ms |
| csharp-serilog | 137 / 136 ms | 140 ms | 139 ms |
| swift-alamofire | 420 / 427 ms | 429 ms | 427 ms |
| cpp-nlohmann-json | 943 / 939 ms | 908 ms | 920 ms |
| **TOTAL** | **2,930 / 2,910 ms** | **2,917 ms (−0.4%)** | **2,928 ms (+0.6%)** |

**−0.4% and +0.6% — a 1.0% spread against a 1.0–1.5% noise floor, so
flat.** Per-repo deltas are *not* reported as real: they did not
reproduce between runs. `cpp-spdlog` swung −1.0% → +5.0% and `go-fzf`
+6.6% → +3.3%, on code paths this release does not touch. Two baseline
columns are shown for the same reason — the baseline binary itself
measured 2,930 ms and 2,910 ms on identical code.

Graph output on the same 9-repo set, 0.4.0 → 0.4.2, one methodology
(whole repo, default mode; `dead` from `--dead-code`, high-confidence
plus withheld):

| repo | nodes | edges | entry | dead |
| --- | --- | --- | --- | --- |
| rust-ripgrep | 2,906 | 7,100 | 0 | 1,429 |
| python-flask | 1,698→1,732 | 1,734→1,738 | 238→272 | 174 |
| js-express | 546 | 393 | 0 | 65 |
| go-fzf | 1,615 | 9,991 | 0 | 131 |
| c-jq | 1,119 | 5,463 | 0 | 425 |
| cpp-spdlog | 1,357 | 1,464 | 0 | 809 |
| csharp-serilog | 1,689 | 1,864 | 0 | 663 |
| swift-alamofire | 2,538 | 6,437 | 0 | 946 |
| cpp-nlohmann-json | 5,567 | 6,738 | 0 | 2,109 |
| **TOTAL** | **19,035→19,069** | **41,184→41,188** | **238→272** | **6,751** |

Only Flask moves, and only by the entry nodes the collapse fix
un-merged. Dead-code findings are unchanged everywhere — this release
adds entry points, it does not change what counts as unreferenced.

> **These numbers are not comparable to the 0.4.1 table below.** That one
> was gathered per-repo through `scripts/benchmark.sh`, which scans a
> configured *subdirectory* (`js-express` → `lib/`); this one scans whole
> repositories, as `scripts/perf-compare.sh` does. Hence js-express 285
> vs 546 nodes for the same code. Neither is wrong; they answer different
> questions, and mixing them in one column would be.

Reproduce with `scripts/perf-compare.sh` and
`scripts/framework-coverage.py`.

### Framework detection

The corpus went from 38 to **43 of 45 frameworks enumerating entry
points**. The two that remain are architectural limits cgg already
declares — not misses, and not silent.

> **Two counting bugs are disclosed below.** Both overstated or
> understated what cgg found, in the one table a reader consults to size
> an attack surface. Neither was caught by a fixture — both needed a real
> application.

#### One application per framework

`scripts/benchmark.sh` gains an `APPS=( … )` corpus: 35 applications that
*use* a framework, never the framework's own repository. A router's test
suite proves the grammar parses; it does not prove cgg recognises the
hand-off as an application writes it. Tracked two ways:

- **`scripts/docs-check.py`** — new gate, pure text, runs in pre-commit.
  Fails when a rule in `rules.rs` has no application, or an application
  claims a framework with no rule.
- **`scripts/framework-coverage.py`** (`benchmark.sh --apps`) — measures
  against the corpus and fails when a declared framework does not fire.
  Reports registrations and *distinct entry nodes* as separate columns,
  because they are not the same number and only reporting the first is
  how the collapse below stayed invisible.

The corpus, measured end to end. `registrations` is how many
hand-offs the resolver matched; `entry nodes` is how many distinct
nodes they became. The two differ when a framework carries no route
string to key a node on — see the collapse fix below.

| application | nodes | edges | entry nodes | registrations | time |
| --- | --- | --- | --- | --- | --- |
| fastapi-dispatch | 4,662 | 4,264 | 249 | 356 | 775 ms |
| django-netbox | 10,416 | 11,155 | 281 | 281 | 3,896 ms |
| flaskbb-flask | 2,890 | 5,119 | 159 | 164 | 381 ms |
| saleor-celery | 23,721 | 67,661 | 112 | 153 | 13,047 ms |
| black-click | 2,607 | 2,070 | 5 | 7 | 818 ms |
| torch-ultralytics | 3,284 | 5,356 | 1 | 160 | 739 ms |
| ghost-express | 20,320 | 47,954 | 272 | 356 | 8,245 ms |
| ghostfolio-nestjs | 1,908 | 3,252 | 106 | 124 | 416 ms |
| immich-nestjs | 9,351 | 16,955 | 273 | 403 | 12,298 ms |
| calcom-nextjs | 19,160 | 48,715 | 152 | 218 | 9,199 ms |
| spring-mall | 14,264 | 2,896 | 112 | 252 | 1,410 ms |
| thingsboard-concurrent | 49,316 | 151,454 | 587 | 640 | 39,080 ms |
| akka-samples | 526 | 457 | 9 | 9 | 91 ms |
| druid-jaxrs | 92,192 | 395,116 | 531 | 653 | 136,612 ms |
| micronaut-graalapp | 15 | 2 | 1 | 1 | 28 ms |
| gin-photoprism | 11,473 | 63,759 | 44 | 44 | 9,396 ms |
| memos-echo | 6,678 | 11,155 | 6 | 6 | 1,167 ms |
| fiber-recipes | 1,551 | 2,646 | 49 | 67 | 305 ms |
| homebox-chi | 10,181 | 10,282 | 98 | 103 | 2,710 ms |
| temporal-samples | 1,026 | 1,578 | 167 | 171 | 185 ms |
| eshop-aspnet | 2,568 | 873 | 57 | 69 | 266 ms |
| masstransit-sample | 217 | 21 | 13 | 18 | 51 ms |
| ombi-quartz | 14,962 | 7,055 | 429 | 513 | 2,224 ms |
| axum-cratesio | 3,451 | 12,568 | 57 | 70 | 1,223 ms |
| lemmy-actix | 2,330 | 10,046 | 135 | 207 | 749 ms |
| actix-examples | 776 | 599 | 189 | 242 | 126 ms |
| vaultwarden-rocket | 2,613 | 7,160 | 302 | 310 | 639 ms |
| rails-mastodon | 11,698 | 11,961 | 323 | 335 | 11,244 ms |
| resque-sinatra | 789 | 755 | 49 | 49 | 156 ms |
| grape-swagger | 668 | 536 | 142 | 183 | 134 ms |
| monica-laravel | 13,172 | 12,989 | 314 | 342 | 5,978 ms |
| symfony-demo | 241 | 24 | 13 | 14 | 52 ms |
| wordpress | 26,906 | 39,036 | 604 | 1,479 | 6,745 ms |
| codeigniter-starter | 26 | 7 | 2 | 2 | 40 ms |
| cuda-samples | 6,403 | 5,894 | 1 | 1 | 1,076 ms |
| **TOTAL** | **372,361** | **961,370** | **5,844** | **8,002** | **271.5 s** |

Two applications dominate that total: **Druid at 137 s** (92k nodes,
395k edges) and **thingsboard at 39 s**. They are the largest trees cgg
has been run against and are included deliberately — a framework corpus
that only holds small apps would not exercise the resolver at scale.
Excluding them the other 33 finish in 95 s combined.

A `~` prefix marks a framework cgg detects but cannot enumerate. It must
still appear in the coverage table's "seen, no rules" section — the
marker asserts the gap is *disclosed*, not that it is absent — and a `~`
that starts enumerating is flagged as stale.

#### Detection gaps closed

Each was a distinct real-world idiom no fixture had exercised:

| framework | what was missed | entries |
| --- | --- | --- |
| `actix-actor` | **`rust.rs` never populated `base_types`** — every base-type rule was dead on Rust | 0 → 30 |
| `chi` | handlers wrapped in `chain.ToHandlerFunc(...)` | 0 → 103 |
| `sinatra` | `get "/x" do … end` — Ruby hangs the block off the call's `block` field, never the argument list | 0 → 24 |
| `worker-threads` | worker modules that identify *themselves* (`import worker_threads` + `parentPort`) rather than being named at a spawn site | 0 → 7 |
| `nestjs-schedule` | nothing — the rule was correct; the corpus app scheduled via `SchedulerRegistry` and had no `@Cron` | 0 → 5 |
| `spring-messaging` | Spring AMQP puts `@RabbitListener` on the class and `@RabbitHandler` on the *method* | 0 → 1 |

The wrapper case is worth stating precisely, because the code declined to
handle it on purpose. `collect_value_refs` skipped nested calls, reasoning
that the walker visits every call on its own turn. It does — but a
wrapper's own turn *bails*, because its verb is not a registrar verb, so
nothing captured it. Descending is therefore not the double work the
comment feared. Only the innermost callee is the handler, so the recursion
emits a call's own name only when it found nothing deeper: that is what
separates `ToHandlerFunc` from `ctrl.Handle`.

`nextjs` and `blazor` remain detected-but-not-enumerated, each with a
`gap:` string saying why: Next.js routes come from file-system layout,
and `.razor` components carry `@page` in markup cgg does not parse.

#### Fixed: entry nodes collapsed onto shared names

**A routeless entry took only the last segment of its handler's qualified
name.** Every Django view named `get` merged into one
`<framework-entry>::network::django::get` node. NetBox reported **10 entry
nodes for 150 registrations** — and the coverage table said "128 entries",
so the disclosure was honest about the framework and wrong about what
could be queried. Filtering the documented attack-surface query returned 8
nodes for ~128 endpoints, and the fan-out from each was the union of
unrelated handlers.

Now the whole qualified name. NetBox reports **281**; WordPress 604,
Mastodon 323, Ghost 272. Frameworks that carry a route string
(`@app.route("/users")`) were never affected.

#### Fixed: per-language entry counts were summed, not split

`into_coverage` keyed entry counts on `(framework, "")` — an empty
language — so a framework with a rule per language reported the
**combined** total on every row. Ghost printed
`express (network, 349 entries)` twice for one set of 349, inviting the
reader to sum it to 698. Now split correctly: **301 JavaScript + 48
TypeScript**. The coverage table also names the language whenever two rows
would otherwise be indistinguishable.

## [0.4.1] - 2026-08-06

A documentation audit that turned into a correctness release. Every
factual claim in `README.md` and the bundled skills was checked against
a deterministic symbolic check — `cgg` itself, `cargo test`,
`cgg --help`, `rg`, the benchmark corpus. Most claims held. The ones
that did not split into two kinds, and both kinds are fixed here: places
where the documentation described something better than the code did,
and places where the code was quietly wrong.

### Performance

0.4.0 → 0.4.1: latency **flat** (within noise).

Maintenance release. Every metric identical to 0.4.0 — latency, nodes, edges,
entry points and findings all unchanged, as intended.

| repo | latency | nodes | edges | entry | dead |
| --- | --- | --- | --- | --- | --- |
| rust-ripgrep | 433→429 ms | 2,906 | 7,100 | 0 | 1,429 |
| python-flask | 161→157 ms | 1,460 | 1,734 | 238 | 174 |
| js-express | 103→105 ms | 285 | 393 | 0 | 65 |
| go-fzf | 287→284 ms | 1,615 | 9,991 | 0 | 131 |
| c-jq | 143→147 ms | 1,119 | 5,463 | 0 | 425 |
| cpp-spdlog | 310→325 ms | 1,357 | 1,464 | 0 | 809 |
| csharp-serilog | 143 ms | 1,689 | 1,864 | 0 | 663 |
| swift-alamofire | 422→436 ms | 2,522 | 6,437 | 0 | 946 |
| cpp-nlohmann-json | 946→937 ms | 5,567 | 6,738 | 0 | 2,109 |
| **TOTAL** | **2,948→2,963 ms** | **18,520** | **41,184** | **238** | **6,751** |

Measured across a 9-repo, 9-language comparison set. All four releases
built from source and measured together on one machine, interleaved per
repo with a discard warmup and rotated ordering. Reproduce with
`scripts/perf-compare.sh`.

**Latency noise floor is ~1.0–1.5% on the total** — two identical runs
of the same commits differ by that much, so smaller deltas are reported
as flat. Node/edge/entry/finding counts are exact and deterministic.

Zeros mean the feature did not exist in that release, not that it found
nothing. `a→b` marks a value that changed; a single value did not.

### Fixed

> **Read this section even if you skip the rest.** The first entry
> silently fabricated half of Elixir's edges. Like the `#include`
> nondeterminism fixed in 0.4.0, it produced a plausible wrong answer
> rather than an error — so nothing signalled that the graph was wrong.

- **Elixir: a definition head was recorded as a call to itself.**
  `def run(x) do … end` parses its head, `run(x)`, as a nested `call`
  node, and the walker recorded it like any other call site. Each
  phantom reference then resolved either to the function itself (a
  self-loop) or to a same-named function in another module (a bogus
  cross-file edge) — and marked its own function reachable, hiding it
  from `--dead-code`. On phoenix this was **1,404 of 2,858 edges**.
  Removing them changes no real edge: every removed edge sat exactly at
  a `def`/`defp`/`defmacro` head offset inside its own source callable,
  and no edge was added. Suppression is keyed on the head's start
  offset, so default arguments, guards, body calls and genuine
  recursion are all untouched. The benchmark row moves from 3,431 edges
  to 1,723.
- **`--max-paths` truncation was silent.** `-n 0` stopped enumerating
  at the cap and said nothing, so a capped path set was
  indistinguishable from a complete one — the caller asked for every
  route through a callable and got a prefix. Hitting the cap now prints
  a note on stderr and records a `paths_truncated` audit event. The
  event fires only when the cap actually turned away work that had been
  reached, not merely when the count landed on the limit.
- **`--dead-code` with no `-o` produced no report.** The sidecar path is
  derived from `-o`, so with the graph on stdout the report was dropped
  and only a one-line summary survived — while the documented
  invocation was `cgg ./src --dead-code`. The text report now goes to
  stderr in that case. JSON has no stderr fallback (interleaving it with
  the run summary would parse as nothing); it names the flag to pass
  instead.
- **The text report was written to a `.json` file.** The sidecar
  extension now follows `--dead-code-format`: `<output>.deadcode.txt`
  for `text`, `<output>.deadcode.json` for `json`.
- **`--write-roots` silently emitted a graph.** It lives inside the
  dead-code pass, so without `--dead-code` it fell through and printed
  ordinary mermaid — a no-op wearing the costume of a baseline. It now
  implies `--dead-code`, as `--why-live` already did.
- **`--ignore-attributes` named the wrong languages.** The "matched
  nothing" note and the `--help` text both said attribute capture was
  "python, rust" long after seven more plugins had learned it. The note
  now reads the list off the plugin registry, so it cannot rot again.
- **README self-analysis graph could not be regenerated.** It sat
  outside the `cgg:begin`/`cgg:end` markers, so the pre-commit hook
  never touched it and it had drifted to stale node ids. It is now a
  `raw:self` marker block patched on every commit.
- **The README graph generator silently dropped edges.** `clean()`
  matched edges with `" --> " in line`, which is false for cgg's own
  collapsed form `A -->|3x| B`. Every multi-site edge vanished from the
  README graphs with no error — the nodes stayed and only the arrow went
  missing. Fixed, with a `--self-test` the hook now runs.

### Changed

- **The cache feature is removed.** cgg never had an on-disk cache: the
  flags were declared in the initial commit as part of a planned task
  list and no implementation ever landed. What shipped was a hollow
  shell — `--cache DIR` and `--no-cache` parsed and were never read, an
  unused `bincode` workspace dependency, a `RESOLVER_FORMAT_VERSION`
  constant existing only to salt cache keys, a `.cgg-cache/` gitignore
  entry, and a `CacheMetrics` block emitted in **every** audit file as
  `{"hits":0,"misses":0,"bytes_read":0,"bytes_written":0}` — which reads
  as "the cache ran and got no hits", not "there is no cache". All of it
  is gone.

  The `cgg` skill had also been advising agents to leave the cache on
  because it "makes re-runs near-instant".

  **Breaking:** `--cache` and `--no-cache` are no longer accepted, so a
  command line passing either now exits 2. Unlike `--stack-graphs` and
  `--no-update-check` — kept as inert flags because they once had
  behavior a script might depend on suppressing — these never did
  anything, so nothing can depend on their effect.

  **Breaking (audit schema):** `metrics.cache` is no longer present in
  the audit document.
- `--include-tests` help text no longer claims to be "a reserved future
  knob, honored as a no-op". It has been live since 0.4.0: it widens the
  dead-code *report*, not the analysis.
- `--roots` help text now describes the discovery order it has actually
  used since 0.4.0 — analyzed paths first, then the working directory.

### Documentation

- **Three bundled skills, not two.** `skills/cgg-frameworks/SKILL.md`
  shipped in 0.4.0 and was never listed in the README;
  `install-skill.sh` had been installing it all along.
- **The audit jq recipe in the `cgg` skill did not run.** The audit
  document is a JSON array of events, so `.unresolved[]` errored with
  `Cannot index array with string`. Replaced with working queries, plus
  one that buckets unresolved sites by `reason.stage`.
- **The `cgg` skill listed six languages as lacking cross-file
  resolution that have it** (Bash, Clojure, Elixir, Erlang, Fortran,
  Julia), contradicting the README's own language table and the
  benchmark numbers. Three genuinely yield none: HCL, Verilog/SV and
  Assembly. Verilog is the subtle one — it parses `` `include ``, so it
  looks resolved, but task/function calls are never captured and so
  nothing ever crosses a file; the README table has always marked its
  cross-file column `—`, and the benchmark measures picorv32 at 0%.
- **`cgg-frameworks` contradicted itself on attribute capture**, saying
  nine languages in Step 2 and "Rust and Python only" in the limitations
  section, and pointed at `crates/cgg-core/src/frameworks_rules.rs`,
  which does not exist (it is `frameworks/rules.rs`). Its verification
  recipe also assumed the old `.deadcode.json` naming.
- Benchmark table: added the five interface/descriptor languages
  (smithy, proto, graphql, openapi, asyncapi) that had plugins and
  benchmark entries but no row, and corrected `xv6 (c+asm)` from 2,087
  to 2,092 edges. All 45 rows now reproduce exactly.
- License section enumerated seven licenses; the dependency tree
  actually uses Zlib (`foldhash`) and Unicode-3.0 (`unicode-ident`) too.
  It now points at `deny.toml` as the authority.
- Verilog's language-table row explains that `` `include `` yields no
  edges because task/function *calls* are not captured — only module
  instantiation is.
- **The `## CLI` usage synopsis never listed `--since`.** The flag
  shipped with a table row and its own worked example, but the usage
  block above them was never updated. `docs-check.py` had only ever
  validated the flag *table*, so nothing noticed.
- **The license section pointed at the wrong artifact.** It claimed
  `MIT OR Apache-2.0 OR LGPL-2.1-or-later` was "the only copyleft
  identifier anywhere in `Cargo.lock`" — but lockfiles record no license
  fields at all, so that check could never have run. The claim itself
  holds (`r-efi` is the sole crate offering an LGPL disjunct across all
  176 packages); it now names the dependency tree, which is where the
  evidence actually lives.
- `docs-check.py` grew a sixth check so the synopsis cannot drift again:
  every flag in `cgg --help` that still does something must appear in
  the usage block, and a flag named there that no longer exists fails
  the commit. Deprecated no-ops (`--stack-graphs`, `--no-update-check`)
  are exempt — they identify themselves with "No effect" in their help
  text, and the synopsis is the wrong place to advertise a flag that
  does nothing.

## [0.4.0] - 2026-08-06

Two features that ask opposite questions of the same graph.

**Dead-code reporting** asks what nothing calls. **Framework entry
points** answer why so much of the apparent answer was wrong: cgg
resolves calls it can see in source, and frameworks invoke user code by
means that are not calls. A route handler rendered as a node with
in-degree zero — which is not merely an incomplete graph but a false
claim that nothing calls it, and which then cascaded into a dead-code
finding for the handler *and* for every private helper reachable only
from it.

They ship together because neither is honest without the other. Both are
**best effort by construction**, both state their evidence, and both say
plainly what they could not see.

**Also fixes a correctness bug that silently affected every C/C++ graph
cgg has ever produced** — `#include` resolution was nondeterministic, so
C/C++ edge counts varied run to run. See *Fixed* below.

### Performance 0.4.0

0.3.0 → 0.4.0: latency **flat** (within noise).

**+2,159 edges (+5.5%) with no node change and no latency cost.** Five
repos moved, for three distinct reasons:

- **ripgrep +1,520** — Rust macro-argument call extraction. Calls inside
  `format!`/`writeln!`/`vec!` produced no edge at all before, because
  tree-sitter leaves macro bodies as unstructured token trees.
- **flask +440, express +180, alamofire +24** — of flask's gain, 238 are
  the entry-node edges themselves; the rest is new extraction reaching
  call sites it previously missed.
- **spdlog: a range replaced by a value.** The table shows 1,469 → 1,464,
  but that −5 is not a decrease — 0.3.0 has no stable edge count on this
  repo. Ten runs of the *same* 0.3.0 binary on the *same* input give
  1460 ×3, 1463 ×4, 1466 ×1, 1469 ×2; the collection run simply drew
  1,469. Ten runs of 0.4.0 give 1464 every time. `collect_include_defs`
  resolved each `#include` by taking the first `HashMap` iteration
  match, and Rust reseeds its hasher per process, so when several files
  matched an include suffix — routine in C/C++, where many directories
  carry their own `common.h` — the winner varied per run and a different
  header meant a different set of imported definitions. 0.4.0 prefers
  the exactly-resolved path, then the lowest `FileId`.

  This is also why the 0.2.0 and 0.3.0 totals in these tables are ±6
  edges run-to-run, and why their spdlog rows should be read as
  "1460–1469", not as the single number shown.

Entry points and dead-code reporting appear here for the first time.
Entry nodes are ON by default, so this is new default work absorbed at
no measurable latency cost, offset by removing the inert stack-graphs
orchestration.

| repo | latency | nodes | edges | entry | dead |
| --- | --- | --- | --- | --- | --- |
| rust-ripgrep | 442→433 ms | 2,906 | 5,580→7,100 | 0 | 0→1,429 |
| python-flask | 137→161 ms | 1,460 | 1,294→1,734 | 0→238 | 0→174 |
| js-express | 101→103 ms | 285 | 213→393 | 0 | 0→65 |
| go-fzf | 315→287 ms | 1,615 | 9,991 | 0 | 0→131 |
| c-jq | 177→143 ms | 1,119 | 5,463 | 0 | 0→425 |
| cpp-spdlog | 307→310 ms | 1,357 | 1,469→1,464 | 0 | 0→809 |
| csharp-serilog | 142→143 ms | 1,689 | 1,864 | 0 | 0→663 |
| swift-alamofire | 450→422 ms | 2,522 | 6,413→6,437 | 0 | 0→946 |
| cpp-nlohmann-json | 932→946 ms | 5,567 | 6,738 | 0 | 0→2,109 |
| **TOTAL** | **3,003→2,948 ms** | **18,520** | **39,025→41,184** | **0→238** | **0→6,751** |

Measured across a 9-repo, 9-language comparison set. All four releases
built from source and measured together on one machine, interleaved per
repo with a discard warmup and rotated ordering. Reproduce with
`scripts/perf-compare.sh`.

**Latency noise floor is ~1.0–1.5% on the total** — two identical runs
of the same commits differ by that much, so smaller deltas are reported
as flat. Node/edge/entry/finding counts are exact and deterministic.

Zeros mean the feature did not exist in that release, not that it found
nothing. `a→b` marks a value that changed; a single value did not.

### Dead-code reporting

cgg already computed the thing a dead-code finder
needs — a resolved call graph — but had no way to ask "what does nothing
call?". `--dead-code` answers that, annotating the normal graph output
rather than replacing it.

The report is **best effort by construction**: cgg reports what it could
not find a caller for, which is not the same as proving no caller
exists. Every output surface says so, every finding carries the evidence
both for and against it, and `--why-live` inverts the question so the
reasoning can be checked in the opposite direction. cgg never modifies
code and takes no position on what should be done about a finding.

#### Added

- **`--dead-code`.** Marks callables nothing appears to reference as
  `unreferenced` in whatever `-t` selects — mermaid label + `classDef`,
  dot dashed node + tooltip, a graphml `<data>` key, a json field. The
  detailed report (evidence, roots, per-language capability table) goes
  to a `<output>.deadcode.json` sidecar, the same convention the audit
  already used.
- **`--why-live PATTERN`.** Prints the shortest path from a root proving
  a callable is live, preferring high-confidence direct edges and
  non-test roots. Answers "why do you think this is used?" and, when no
  path exists, says so as a derivation rather than an assertion.
- **`cgg-deadcode.toml`.** `roots` entries are entry points and confer
  liveness transitively; `[[allow]]` entries are reviewed findings that
  are suppressed *without* being made live, so accepting one hides it
  and nothing else. Parsed with `deny_unknown_fields`, so a typo is a
  hard error rather than a silently ignored line. `--write-roots`
  generates a baseline; `--roots FILE` pins it.
- **Supporting flags:** `--dead-code-format`, `--dead-code-confidence`,
  `--dead-code-report`, `--ignore-names`, `--ignore-attributes`,
  `--fail-on-dead` (exit 3, opt-in).
- **Calls inside Rust macro arguments are now extracted.** tree-sitter
  leaves macro bodies as unstructured token trees, so a real call like
  `writeln!(out, "{}", xml_escape(s))` produced no edge. Rust edge
  counts rise ~12-27% depending on macro density; no other language is
  affected.
- **New extraction signals:** normalized `Vis` for 7 languages (was 2),
  `TestRole` and test-file classification, `ExportRecord` (Rust
  `pub use`, Python `__all__`), `DynUse` reflection hints
  (suppression-only, never an edge), and `UnreachableRegion` for
  statements after an unconditional terminator across 6 language
  families.
- **`LanguagePlugin::signals()`** — a per-plugin manifest of which
  optional signals it actually extracts, so a report can distinguish
  "this definition genuinely has no attributes" from "cgg never looked".

#### Removed

- **The update check, and with it every network call cgg makes.**
  `update_check.rs` made one `GET` to `api.github.com` per day to read a
  release tag. Its dependency, `minreq`, carried the entire HTTP/TLS
  stack — `rustls`, `rustls-webpki`, `webpki-roots` — and with it three
  RustSec advisories (RUSTSEC-2026-0098/0099/0104).

  Clearing those advisories in place meant `minreq` 2 → 3, which pulls
  `aws-lc-rs`/`aws-lc-sys` and a build-time C toolchain — a poor trade
  for a feature whole exploit surface was "someone lies to you about the
  latest version number". Removing the feature clears them outright and
  makes *offline* a property of the code rather than a default that can
  be flipped: the workspace now contains zero network call sites.

  `--no-update-check` is still accepted and does nothing, so existing
  command lines keep working. To keep an installed binary current, use
  `cargo install-update -a` (from the `cargo-update` crate) or re-run
  `cargo install --git`.

### Framework entry points

cgg resolves calls it can see in source; frameworks invoke user code by
means that are not calls. That did not merely leave the graph
incomplete — it made it **wrong**: a route handler rendered as a node
with in-degree zero, which is a claim ("nothing calls this") and a false
one.

`<framework-entry>` nodes fix that, mirroring the exit nodes
`--include-external` already minted for control leaving the tree. They
are **on by default**, deliberately unlike the exit-node flags: an exit
node tells you nothing you did not already know from reading the call,
while an entry node tells you something the source cannot state at all.

Entry nodes are an **inference, not an observation** — nothing in your
source says the call happens — so coverage is disclosed rather than
implied. Every run prints which frameworks were recognised, which were
seen and not understood, and which languages have no rules at all.

#### Added

- **`<framework-entry>` nodes.** One per entry point with real identity
  — a route, a queue, a command — carrying a trust-boundary kind in the
  qualified name (`<framework-entry>::network::flask::route("/users")`).
  Edges are `Via::FrameworkEntry(framework)` at `Confidence::Low`,
  tagged `entry` in mermaid, bold purple in dot, and `framework-entry`
  in a new graphml edge attribute.
- **Trust-boundary kinds** — `network`, `queue`, `schedule`, `cli`,
  `ffi`, `lifecycle`, `test` — filterable because they are part of the
  name: `cgg ./src --filter '<framework-entry>::network::' -n 3`
  enumerates attack surface and its blast radius in one query. Only
  `network` is asserted to carry untrusted input; `queue` depends on who
  can enqueue, which cgg cannot see.
- **Framework rules for 40+ frameworks** across python, javascript,
  typescript, java, kotlin, go, ruby, php, csharp, rust and cpp,
  covering all six hand-off shapes: attribute markers (Flask, FastAPI,
  Spring, Jakarta/Quarkus, Micronaut, NestJS, ASP.NET MVC, Symfony,
  Rocket, Actix, Celery, Click), value refs (Express, Gin, Echo, Fiber,
  Chi, net/http, Axum, Django `urls.py`, Temporal), inline closures,
  base types (PyTorch, Quartz, MassTransit, Sidekiq, Akka,
  `BackgroundService`, `Runnable`), string targets (Rails
  `'photos#index'`, Laravel's `@` string *and* `[C::class,'m']` array,
  WordPress hooks) and module paths (`worker_threads`, piscina).
- **A coverage table on every run.** Three sections, stated separately:
  what was recognised (with entry counts), what was *seen and not
  enumerated* (with the reason), and which languages have no rules.
  A framework that is recognised but matched nothing is reported as a
  gap too, because "flask (network, 0 entries)" reads as "this app has
  no routes". Also emitted as an `AuditEvent::FrameworkCoverage`, with
  `FRAMEWORK_ENTRY_DISCLAIMER` copied in by the engine so no formatter
  can drop it.
- **`[[framework]]` blocks in `cgg-deadcode.toml`,** so the gap list is
  actionable: a framework cgg does not ship rules for can be covered
  locally without waiting for a release.
- **`--no-entry-nodes`** to opt out, and **`--framework-coverage`** to
  print the table even when nothing was recognised.
- **CUDA kernels are entry points.** `tree-sitter-cpp` parses
  `saxpy<<<a,b>>>(x)` as nested comparison operators, so the launch
  produces no edge and the kernel plus every `__device__` helper read as
  dead. Treating `__global__` as a root qualifier fixes the cascade
  without fighting the grammar.

#### Extraction

- **Attribute capture** for java, csharp, typescript, javascript, php,
  kotlin and cpp (previously rust and python only). Stored **verbatim**,
  because `python.rs` refines a `DefVariant` from raw decorator text and
  `--ignore-attributes` matches what the user actually wrote. This also
  raises those languages' dead-code confidence ceiling.
- **Value-reference capture** for python, javascript, typescript, go,
  java, csharp, php and ruby (previously rust only), with two long-
  standing gaps closed: `intra_file` could only bind a value ref within
  one file, and a value ref resolved across files was tagged
  `Via::Direct` — claiming a call site that does not exist and escaping
  the `--reference-edges` flag meant to gate it.
- **Base-type capture** (`DefRecord::base_types`) for python, java,
  csharp, javascript, typescript, php and ruby, including Ruby's
  `include Sidekiq::Job` mixins. This is the principled replacement for
  the hardcoded `LIFECYCLE` name list.
- **PHP import capture** (`use`/`namespace`) and **PHP static calls**
  (`C::m()`), neither of which was extracted before. PHP's graph on the
  Laravel corpus goes from **329 edges to 16,355** (0 → 15,408
  cross-file); the run costs ~70% more wall time as a result, which is
  the price of a language whose call graph was previously ~1% resolved.
- **TypeScript signal manifest.** `TypeScriptPlugin` reused `JsWalker`
  but declared no signals and skipped the unreachable/reflection passes,
  so the dead-code capability table said cgg had never looked. Both
  fixed.
- `RefRecord` gains `context` and `route`; `DefRecord` gains
  `base_types`; `CallableNode` gains `framework_entry`. All additive and
  serde-defaulted.

#### Verified against real applications

Seven applications *using* each framework — not the frameworks' own
repositories, which never import themselves and exercise no rule:

| app | framework | entries found |
| --- | --- | --- |
| NetBox | Django | 128 network · 22 cli |
| Netflix Dispatch | FastAPI | 318 network · 38 cli |
| Mastodon | Rails + Sidekiq | 199 network · 109 queue |
| macrozheng/mall | Spring Boot | 250 network · 1 schedule |
| PhotoPrism | Gin + Chi | 44 network |
| crates.io | Axum | 70 network |
| Ultralytics | PyTorch | 159 lifecycle (root-marked, no nodes) |

Both payoffs move in the right direction on those applications, which
is the test a phase has to pass to earn its place — entry nodes up,
dead-code findings down:

| app | findings without entry nodes | with |
| --- | --- | --- |
| Ultralytics | 1,400 | 1,169 (−17%) |
| Netflix Dispatch | 2,564 | 2,133 (−17%) |

That exercise found five defects that fixtures had not:

- **A UTF-8 panic aborted the entire run.** `detect.rs` sliced a file's
  head at byte 2048 without checking the char boundary, so any file
  whose first 2 KiB contain non-Latin text crashed the process. Mastodon
  ships ~90 such translation catalogues. `type_hints.rs` had the same
  bug on `ty[..1]` for a non-ASCII identifier.
- **Rust value refs lost their route.** The registration context was
  emitted as a *second* record sharing the first's `(name, site_byte)`,
  and the context-less one won — so every axum route resolved anonymous.
- **Ambiguous verbs matched ordinary code.** `crate_ids.get(id)` and
  `session.get("user_id")` became "routes" in an axum project. A match on
  a verb like `get`/`add`/`use` now needs corroboration: an identity, or
  a receiver-less call (axum's `get(handler)` is a free function; a map
  lookup is not).
- **String routing applied everywhere.** Decoding a string into a
  handler name is now opt-in per rule (`string_targets`), set only for
  the four frameworks that route that way.
- **A marker-only rule detected everywhere.** CUDA has no import to gate
  on, so it counted as "detected" in every repository containing a C++
  file and was reported as a coverage gap in all of them.

Three coverage gaps closed as a direct result:

- **Inherited framework contracts.** A real application never inherits
  the framework base directly — NetBox writes `class
  CircuitListView(generic.ObjectListView)` and only three levels up does
  anything name Django's `View`. Base-type matching now walks the
  inheritance chain (depth-capped, cycle-guarded): Django 65 → 128
  entries, PyTorch 143 → 159, Sidekiq 96 → 109.
- **`utoipa::path`.** The `utoipa-axum` pattern registers handlers
  through `.routes(routes!(a, b, c))`, a proc-macro whose token tree
  cgg cannot read — but every one of those handlers carries its method
  and path in a `#[utoipa::path]` attribute. crates.io went 7 → 70.
- **Sidekiq workers carry no import.** Rails autoloads, so
  `app/workers/*.rb` names `Sidekiq::Worker` without requiring it; the
  convention directory is the only marker. Mastodon 0 → 109.

### Fixed

> **Read this section even if you skip the rest.** The first entry
> changed results *silently, on every run, for multiple releases*. A bug
> that returns a plausible wrong answer is worse than one that crashes:
> nothing prompts you to go looking, and any number you published in the
> meantime was wrong without saying so.

**`#include` resolution was nondeterministic — this silently affected
every C/C++ graph cgg has ever produced.** `collect_include_defs` picked
its target with `HashMap::values().find(...)`, and Rust seeds its hasher
per process, so when several files matched an include suffix — routine
in C/C++, where many directories carry their own `common.h` — the winner
varied run to run, and a different header meant a different set of
imported definitions.

Measured on `cpp-spdlog`: the same binary on the same input produced
1460 ×3, 1463 ×4, 1466 ×1, 1469 ×2 across ten runs. It now prefers the
exactly-resolved path, then the lowest `FileId`, and gives 1464 every
time.

Consequences worth knowing:

- Any C/C++ edge count published before 0.4.0 — including this
  project's own README benchmark table — was one draw from a range, not
  a fixed value.
- Determinism is a headline claim in the README. It did not hold for
  C/C++, and no test covered it. `dead_code_output_is_byte_stable` and
  the `edge_order_invariance` unit tests now do.
- The bug predates 0.3.0; it is fixed here rather than in a patch
  release because it was found while building the dead-code engine,
  whose whole model assumes a stable graph.

- **Invalid `--filter` / `--exclude-*` patterns are now a hard error.**
  A bad regex was silently mapped to match-everything, while
  `apply_exclusions` silently dropped it — two opposite silent failures
  for the same mistake.
- **Config discovery was working-directory-relative,** so
  `cgg /path/to/project` from anywhere else silently ignored that
  project's `cgg-deadcode.toml`. Discovery now searches upward from each
  analyzed path first.
- **`cross_file` de-duplicated edges with an O(edges) scan per resolved
  reference.** Invisible while PHP resolved almost nothing; ~4s of a
  Laravel run once it started resolving properly. Now indexed.
- GraphML dropped the edge `via` tag entirely, so a consumer could not
  tell an inferred edge from a resolved call.
- **Haskell definitions were never qualified by their module.**
  `extract_module` looked for a `module_name` node that
  `tree-sitter-haskell` 0.23 does not have (the kind is `module`, and
  the keyword is an anonymous token of the same name), so every Haskell
  callable came out as a bare `work` rather than `Data.Thing.work` and
  same-named functions in different modules were indistinguishable.
  Silent, because an unqualified name is still a perfectly good name.
  Haskell now joins with `.`, matching how modules are written and
  imported; on pandoc this resolves ~250 previously-unresolved calls.

### Changed

- **`--stack-graphs` has no effect** and its help text now says so. The
  integration was removed in the tree-sitter 0.26 upgrade (upstream
  pins tree-sitter 0.24); the orchestration around the resulting stub
  still ran on every invocation, deep-copying the graph, the facts and
  every file's source bytes into a thread before blocking on a
  60-second timeout. Removing it, and the retained source-byte corpus
  it kept alive, made ordinary runs measurably faster.
- Dead-code-only extraction is gated behind the mode, so a run without
  `--dead-code` does not pay for it.

### Compatibility

Default output is unchanged except for the two edge-count effects noted
above (Rust macro-argument calls, C/C++ `#include` determinism), both of
which only ever *add* or *stabilise* edges. `--stack-graphs` is still
accepted. `--include-tests`, previously parsed and never read, now has
real semantics.

**The default graph grows.** Entry nodes are on by default, so node and
edge counts move for every language with framework rules. This follows
the project's standing rule that the default graph only ever grows in
default mode; `--no-entry-nodes` restores the previous shape exactly.

Adding `Via::FrameworkEntry` is a compile error in exactly the two
`match` arms that classify edges for output, so no formatter can
silently ignore it.

## [0.3.0] - 2026-06-30

Five interface/descriptor languages, taking cgg from 39 to **44**
languages. These map an API model's shape graph onto the call-graph
model, so a descriptor renders as a topology of
service → operation → message/structure → field-type edges. Purely
additive: no existing language's graph changes.

### Performance

0.2.0 → 0.3.0: latency **flat** (within noise).

Five interface/descriptor languages added. Graph unchanged on this set (+3
edges) because those languages are not present in it.

| repo | latency | nodes | edges | entry | dead |
| --- | --- | --- | --- | --- | --- |
| rust-ripgrep | 432→442 ms | 2,906 | 5,580 | 0 | 0 |
| python-flask | 149→137 ms | 1,460 | 1,294 | 0 | 0 |
| js-express | 95→101 ms | 285 | 213 | 0 | 0 |
| go-fzf | 317→315 ms | 1,615 | 9,991 | 0 | 0 |
| c-jq | 170→177 ms | 1,119 | 5,463 | 0 | 0 |
| cpp-spdlog | 310→307 ms | 1,357 | 1,463→1,469 | 0 | 0 |
| csharp-serilog | 147→142 ms | 1,689 | 1,864 | 0 | 0 |
| swift-alamofire | 452→450 ms | 2,522 | 6,413 | 0 | 0 |
| cpp-nlohmann-json | 934→932 ms | 5,567 | 6,738 | 0 | 0 |
| **TOTAL** | **3,006→3,003 ms** | **18,520** | **39,019→39,025** | **0** | **0** |

**Caveat on this table:** cgg's `#include` resolution is nondeterministic
in this release (fixed in 0.4.0). `cpp-spdlog`'s edge count varies
1460–1469 across runs of this same binary, so its row — and the totals —
are one draw from a range, not a fixed value.

Measured across a 9-repo, 9-language comparison set. All four releases
built from source and measured together on one machine, interleaved per
repo with a discard warmup and rotated ordering. Reproduce with
`scripts/perf-compare.sh`.

**Latency noise floor is ~1.0–1.5% on the total** — two identical runs
of the same commits differ by that much, so smaller deltas are reported
as flat. Node/edge/entry/finding counts are exact and deterministic.

Zeros mean the feature did not exist in that release, not that it found
nothing. `a→b` marks a value that changed; a single value did not.

### Added

- **Smithy, Protobuf, GraphQL, OpenAPI/Swagger, and AsyncAPI plugins.**
  - Smithy: `service → operation → structure → shape-member` edges;
    traits and prelude primitives skipped. The published
    `tree-sitter-smithy` crate pins an incompatible `tree-sitter 0.20`,
    so its generated `parser.c` is **vendored** under
    `crates/cgg-lang/vendor/smithy/` (MIT, see `PROVENANCE.md`),
    compiled by a new `crates/cgg-lang/build.rs`, and bound through
    `tree_sitter_language::LanguageFn`.
  - Protobuf: message field types + gRPC `service` rpc →
    request/response message edges.
  - GraphQL: SDL `type → field-type`, `implements`, and `union` member
    edges; built-in scalars skipped.
  - OpenAPI/Swagger and AsyncAPI: YAML **or** JSON (both parsed with the
    YAML grammar), content-detected by their root `openapi:` /
    `swagger:` / `asyncapi:` key via a new `cgg-lang::detect` rule, so
    ordinary `.yaml`/`.json` config/data files are untouched.
    Operation → schema and schema → schema (`$ref`) edges; AsyncAPI adds
    channel/message edges.
- **Cross-file resolution for descriptor languages.** References in
  Smithy/Protobuf/GraphQL/OpenAPI/AsyncAPI resolve by global simple-name
  within the model (bounded to ≤4 candidates) — see
  `cgg-resolve::cross_file`.

### Changed / Improved

- **Per-language stdlib filter audit.** 21 stdlib lists (bash, c, cpp,
  clojure, dart, elixir, erlang, go, groovy, haskell, hcl, javascript,
  kotlin, lua, objc, perl, php, python, ruby, typescript, zig) tuned
  against real-world `external`-bucket noise. Eight remain seeded from
  language references only (csharp, fortran, java, julia, ocaml, r,
  scala, swift).
- Docs synced to the code: README language table/count (44), embedded
  mermaid graphs, self-stats, and the Limitations / Potential-future-
  improvements sections; `skills/cgg/SKILL.md`; `CLAUDE.md`; and the
  `scripts/benchmark.sh` targets for the five new languages.

### Compatibility / migration

- **To keep the previous behavior: do nothing.** The five new languages
  only add graphs for file types that previously produced none. No
  existing language's nodes or edges change. `.yaml`/`.json` files are
  analyzed only when their root key marks them as an OpenAPI/AsyncAPI
  document.

## [0.2.0] - 2026-06-18

A resolver-precision pass (the `necessary_fixes.md` program) plus four
opt-in output modes. Verified against a 38-language real-world corpus:
**the default graph is a strict superset of the previous one — 0 nodes
and 0 edges lost in any language** (checked at per-call-site,
overload-distinguishing granularity), and faster.

### Performance

Baseline for the series; 0.1.0 predates this CHANGELOG and was not
measured. Dead-code reporting and framework entry points did not exist.

| repo | latency | nodes | edges | entry | dead |
| --- | --- | --- | --- | --- | --- |
| rust-ripgrep | 432 ms | 2,906 | 5,580 | 0 | 0 |
| python-flask | 149 ms | 1,460 | 1,294 | 0 | 0 |
| js-express | 95 ms | 285 | 213 | 0 | 0 |
| go-fzf | 317 ms | 1,615 | 9,991 | 0 | 0 |
| c-jq | 170 ms | 1,119 | 5,463 | 0 | 0 |
| cpp-spdlog | 310 ms | 1,357 | 1,463 | 0 | 0 |
| csharp-serilog | 147 ms | 1,689 | 1,864 | 0 | 0 |
| swift-alamofire | 452 ms | 2,522 | 6,413 | 0 | 0 |
| cpp-nlohmann-json | 934 ms | 5,567 | 6,738 | 0 | 0 |
| **TOTAL** | **3006 ms** | **18,520** | **39,019** | **0** | **0** |

**Caveat on this table:** cgg's `#include` resolution is nondeterministic
in this release (fixed in 0.4.0). `cpp-spdlog`'s edge count varies
1460–1469 across runs of this same binary, so its row — and the totals —
are one draw from a range, not a fixed value.

Measured across a 9-repo, 9-language comparison set. All four releases
built from source and measured together on one machine, interleaved per
repo with a discard warmup and rotated ordering. Reproduce with
`scripts/perf-compare.sh`.

**Latency noise floor is ~1.0–1.5% on the total** — two identical runs
of the same commits differ by that much, so smaller deltas are reported
as flat. Node/edge/entry/finding counts are exact and deterministic.

Zeros mean the feature did not exist in that release, not that it found
nothing. `a→b` marks a value that changed; a single value did not.

### Added

- **Update check.** A best-effort, **opt-out**, once-a-day "newer
  release available?" notice. It runs on a background thread that
  overlaps the analysis, prints a single line to stderr only in an
  interactive terminal, and caches its result in
  `$XDG_CACHE_HOME/cgg/update-check.json` (so the network is hit at most
  once per 24h). It is cgg's *only* network access, never affects the
  graph/output/exit-code, and is disabled by `--no-update-check`,
  `--quiet`, a non-interactive invocation, or `CGG_NO_UPDATE_CHECK` /
  `DO_NOT_TRACK` / `CI`. (Adds cgg's first network dependency, `minreq`
  - rustls — binary stays self-contained, no system OpenSSL.)
- **`--include-external` / `--include-stdlib`** — surface calls into
  third-party / standard-library code as deduplicated leaf "exit nodes"
  (one node per `(language, receiver, name)` symbol; every call site
  collapses onto it with multiplicity). Edges tagged `ext` / `std`.
- **`--dynamic-dispatch`** — for interface/trait dispatch, emit fan-out
  edges from each method *declaration* to every concrete *implementation*
  (one low-confidence edge per impl). The exact call-site → declaration
  edge is always emitted; this flag adds the over-approximated dispatch.
  Edges tagged `dyn`. (Plugin capture wired for Rust; resolver/format
  machinery is language-agnostic.)
- **`--reference-edges`** — when a function is passed *by name* as a
  value (`register(handler)`), emit a reference edge distinct from a
  call edge, repairing the "registered handler looks like dead code"
  distortion. Edges tagged `ref`. (Rust.)
- New `Via` edge kinds (`External`, `Stdlib`, `Reference`) and
  `CallableNode` fields (`synthetic`, `trait_impl_target`), rendered as
  label tags in mermaid (`ext`/`std`/`dyn`/`ref`), edge styles in dot,
  and serialized in json/graphml.
- **Structured unresolved-call audit** — each unresolved record now
  names the resolution *stage* that rejected it
  (`no-candidate-in-file`, `ambiguous-in-file`, `no-candidate-cross-file`,
  …) plus the evidence it had (candidate counts, which name-screen was
  applied). The unresolved population is now sliceable by category for
  regression tracking.

### Changed / Improved (default mode — no flags needed)

- **Toolchain.** Moved to the Rust **2024 edition**; minimum supported
  Rust is now **1.85** (was 1.80). No API changes — a one-line
  match-ergonomics adjustment was the only code impact.
- **Cache format.** `RESOLVER_FORMAT_VERSION` bumped to `2` so stale
  `.cgg-cache` entries from 0.1.x are re-extracted (the new
  function-as-value records and edge kinds need a fresh pass).
- **Owner-qualified disambiguation.** Same-name candidates
  (`Parser::new` vs `Cursor::new`, and `Self::new` inside an impl) are
  now disambiguated by the call's owner qualifier instead of being
  abandoned as ambiguous.
- **Cross-file receiver resolution.** Method calls on a receiver of
  known type now resolve through an `(owner type, method)` index —
  including through import aliases (`use a::b::Engine as Motor`) and
  multi-segment receiver paths. This also made resolution **faster**:
  the index replaces a per-call-site O(callables) scan with an O(1)
  lookup (≈ −40% wall time on method-heavy Kotlin, −33% on Rust).
- **Standard-library name-collision ordering.** A project method whose
  name collides with stdlib vocabulary (`EntityId::len`) is no longer
  siphoned into the stdlib bucket — owner ownership is checked first.

### Fixed

- The summary line's `cross-file` count used a formula that predated
  edge deduplication; it now counts actual inter-file edges of the whole
  analysis and stays consistent with the `edges` total even under
  `--filter`/`-n`.
- A latent subtract-overflow panic in the summary computation
  (surfaced by the new synthetic edges).

### Compatibility / migration

- **To keep the previous behavior: do nothing.** All new behavior is
  either a strictly-additive precision improvement or gated behind an
  opt-in flag. With no new flags, the default graph contains every node
  and edge the previous version produced (verified across 38 languages),
  plus newly-resolved direct edges — nothing is removed or retargeted at
  the unique-edge level.
- **To get the new structural views:** add the opt-in flags above. They
  only *add* tagged edges/nodes. Downstream consumers can include or
  exclude them by the mermaid label tags (`ext`/`std`/`dyn`/`ref`) or by
  the `via` / `confidence` fields in json/graphml.
- **Audit consumers:** the unresolved `reason` field is now a structured
  object (`{"stage": …}`); the deserializer still accepts the old
  free-form string, so existing tooling that only reads other fields is
  unaffected.
