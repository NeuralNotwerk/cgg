# Changelog

All notable changes to `cgg` are documented here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/); this project is
pre-1.0, so the resolver's edge set may grow between releases (it only
ever grows in default mode — see *Compatibility* below).

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
