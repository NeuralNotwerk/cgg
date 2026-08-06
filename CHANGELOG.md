# Changelog

All notable changes to `cgg` are documented here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/); this project is
pre-1.0, so the resolver's edge set may grow between releases (it only
ever grows in default mode — see *Compatibility* below).

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

#### Fixed

- **`#include` resolution was nondeterministic.** `collect_include_defs`
  picked its target with `HashMap::values().find(...)`; Rust seeds its
  hasher per process, so when several files matched an include suffix —
  routine in C/C++, where many directories hold a `common.h` — the
  winner varied run to run. Measured on `cpp-spdlog`: the same binary on
  the same input produced 1460/1463/1466/1469 edges across 10 runs. Now
  prefers the exactly-resolved path, then the lowest `FileId`.
- **Invalid `--filter` / `--exclude-*` patterns are now a hard error.**
  A bad regex was silently mapped to match-everything, while
  `apply_exclusions` silently dropped it — two opposite silent failures
  for the same mistake.

#### Changed

- **`--stack-graphs` has no effect** and its help text now says so. The
  integration was removed in the tree-sitter 0.26 upgrade (upstream
  pins tree-sitter 0.24); the orchestration around the resulting stub
  still ran on every invocation, deep-copying the graph, the facts and
  every file's source bytes into a thread before blocking on a
  60-second timeout. Removing it, and the retained source-byte corpus
  it kept alive, made ordinary runs measurably faster.
- Dead-code-only extraction is gated behind the mode, so a run without
  `--dead-code` does not pay for it.

#### Compatibility

Default output is unchanged except for the two edge-count effects noted
above (Rust macro-argument calls, C/C++ `#include` determinism), both of
which only ever *add* or *stabilise* edges. `--stack-graphs` is still
accepted. `--include-tests`, previously parsed and never read, now has
real semantics.

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
|---|---|---|
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
|---|---|---|
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

#### Fixed

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

#### Compatibility

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
  + rustls — binary stays self-contained, no system OpenSSL.)
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
