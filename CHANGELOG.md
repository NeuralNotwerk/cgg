# Changelog

All notable changes to `cgg` are documented here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/); this project is
pre-1.0, so the resolver's edge set may grow between releases (it only
ever grows in default mode — see *Compatibility* below).

## [0.4.0] - 2026-08-05

Dead-code reporting. cgg already computed the thing a dead-code finder
needs — a resolved call graph — but had no way to ask "what does nothing
call?". `--dead-code` answers that, annotating the normal graph output
rather than replacing it.

The report is **best effort by construction**: cgg reports what it could
not find a caller for, which is not the same as proving no caller
exists. Every output surface says so, every finding carries the evidence
both for and against it, and `--why-live` inverts the question so the
reasoning can be checked in the opposite direction. cgg never modifies
code and takes no position on what should be done about a finding.

### Added

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

### Fixed

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
