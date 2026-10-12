# cgg fix plan v2 — verified against independent ground truth

Date: 2026-10-10. Supersedes v1. cgg 0.10.1 @ `17b89f3`, corpus
`/storage/neurosymbolic_kb`.

Method change: v1 was written from one code-walk pair plus my own
artifact mining. v2 was rebuilt after four fresh-eyes verification agents
re-derived every mechanism from primary sources, hunted 10
should-capture + 10 near-miss instances per fix family, and produced
collision/test-churn maps. Where a verifier's claim was load-bearing I
re-verified it myself against source/artifacts (all spot-checks listed
below passed). Several v1 claims are reversed below; the ledger is §5.

Ground-truth artifacts note (affects all measurement): the `external`
name-screen (`cgg-core/src/external.rs:159-163`) records some unresolved
refs **audit-only** — they never appear in the graph JSON. Verified:
`path_re/hex_re/num_re/diag_re` exist in `*.audit.json` and are absent
from `graph.unresolved`. All capture counts must read the audit file.

## 0. Headline correction

The false-High rate on this corpus is **~89 of 105 (range 85–90), not
~70** — a verifier adjudicated all 105 rows (25-row sample agreed at
84%). Corrected breakdown:

| class | rows | status |
| --- | --- | --- |
| serde attribute-string names | 63 | false — F5 |
| trait declarations dispatched via dyn/generics | 12 | false — F2/F6 |
| framework-consumed impl (`AppError::into_response`) | 1 | false — F8 |
| test-only (`DreamLlm::complete_synthesis`) | 1 | false — already categorized |
| resolver blind spots (turbofish ×3, value-position ×6, receiver-shape ×3) | 12 | false — F12/F13/F4+F11 |
| actually dead | 16 | true — incl. `Engine::run_one` (`#[allow(dead_code)]`, zero callers, double-confirmed) and `DreamCommunity::summarize` (held as `Arc<dyn …>` but **zero `.summarize(` call sites** — holding ≠ calling) |

## 1. Invariants (unchanged unless marked)

1. A string that names a callable never manufactures an edge (only
   lowers confidence).
2. Typed-receiver misses never degrade to name matching; external-head
   receivers get full-path matching only. **New corollary from
   verification: any hint normalization must preserve `::` paths** —
   stripping them disables `is_external_rust_head` and reopens the
   measured 797-edge fan-out regression (`cross_file.rs:2372-2377`).
3. Fan-out cap is decided on the narrowed set before the visibility
   filter (`cross_file.rs:3036-3046`; test `:4132`).
4. Post-reconciliation unresolved entries mean genuinely unresolved;
   screened things ride their own channel (`external`, `reflection`
   precedents).
5. Evidence caps, never adds. **Verified trap:** the only High
   promotion is conjunctive — `capped == Medium && !has_doubt` promotes
   (`evidence.rs:216-223`). A new capping evidence that does *not*
   carry `Polarity::Lowers` gets promoted straight back to High.
   New Evidence variants must also register in `slug()` (compile
   gate) and the polarity-partition test.
6. Determinism at any `--jobs`; build any new index/set sorted — the
   `dispatch.rs:62-68` comment documents exactly the bug class a
   trait-impl-set map reintroduces.
7. **Verified scope fact:** framework `detect()` is evaluated **once per
   workspace run**, keyed `(rule.id, rule.language)`
   (`frameworks/mod.rs:281-305`); matchers filter by language only.
   One axum-importing crate arms the axum rule for all 23 crates;
   `base_type_matches` compares last-path-segment case-insensitive
   (`mod.rs:751-765`). Root-only channels cannot add false DEAD findings;
   `node: true` rules mint entry **edges** and a wrong match mints a
   wrong liveness assertion.
8. **Verified rollout effect:** demotions are not report-only — `marked`
   sets the `unreferenced` node flag (`cgg/src/lib.rs:1677-1680`), so
   graph JSON, `--fail-on-dead` scope, and py/node parity artifacts
   shift with any band change.

## 2. Fixes

### F1 — async impl methods: self-field facts + workspace-aware rewrite guard
- `rust.rs:1330-1334` filter gains `AsyncFunction` (async replaces the
  impl variant at `:536-544`; verified 199 async-in-impl callables on
  this corpus, all currently fact-less).
- **Guard (verified necessary):** type_hints Pass-3 self-branch
  (`:526-536`) must rewrite only when the simplified type contains
  `::` with a workspace crate-root head, or is a bare name ∈ workspace
  owner names. Otherwise skip — leaving the lowercase hint for the
  duck path.
- Verified capture set: 254 single-hop `self.field.m()` sites in async
  impl methods; 10 pure captures (unresolved→typed; e.g.
  `idle_watcher.rs:158,212,215` `self.nkb_config.get`,
  `self.lock.store/snapshot`; `gpu.rs:250,263,329`
  `self.inner.append/search/len`; `extract/soft_fail.rs:166`),
  88 upgrades (duck→typed), **5 verified regressions without the
  guard** (~8 medium edges: `Notify::notified` ×8 sites incl.
  async_trait ×44 corpus uses; generic-field `self.vec_index: V` at
  `ingest/lib.rs:925,1111`; `tokio::sync::watch::Sender` head at
  `llm_client/lib.rs:5713`). All five are non-workspace types or
  generic params ⇒ the guard above extinguishes them.
- v1 reversal: "fixes zero nkb rows" was wrong — it was judged only
  against the trait-declaration High rows; the graph-recall effect is
  ~98 sites and some liveness truth via `MultiIndex::*` etc.
- Related latent inconsistency (note, not fixed here):
  `intra_file::is_method_variant` excludes `AsyncFunction` while
  `dispatch::is_override_candidate` includes it — opposite answers.

### F1b — plumb `def_variant` into dead-code (sub-fix, enabler)
`DeadCodeFinding.def_variant` is declared but **never populated** —
all three construction sites write `String::new()`
(`deadcode/config.rs:270`, `cgg-core/deadcode.rs:899`,
`deadcode/mod.rs:443`). F1's report-accuracy claim and F2's
default-method refinement both need it. Two-line plumbing.

### F2 — implemented trait declaration: dispatch-surface evidence (HEADLINER)
- New Evidence `ImplementedTraitSurface`, `Polarity::Lowers`, cap
  Medium. Predicate, tightened by verification to a **node-field**
  rule (never edges — fan-out edges exist only in dead-mode runs):
  callable is a workspace-trait declaration method `T::m`, ≥1 callable
  has `trait_impl_target == T` — read from the field **or** parsed
  from the `<X as T>::m` wrapper qualified-name form (verified: some
  wrapper-q members carry an empty `trait_impl_target` field, e.g.
  `config::<ConfigSource<T> as Default>::default`) — **and** dispatch
  left evidence for *this method*: ≥1 same-simple-name unresolved
  site, or ≥1 impl of `T::m` with a non-dynamic incoming edge.
- Verified capture set: 13 of the 14 High trait-declaration rows
  (all 8 originally claimed + `DreamLlm::{complete_synthesis,
  synthesis_capacity}`, `DreamExtractor::extract`, both
  `*ProgressReporter::update`).
- **`DreamCommunity::summarize` deliberately excluded:** trait object
  held, zero `.summarize(` sites anywhere (verified) → predicate
  declines → stays High, which is *correct dead*. This case is the
  regression test that keeps v1's unconditional cap out.
- Near-misses handled: `LlmClient` (impls, zero dyn usage) — zero
  sites ⇒ predicate declines, stays reportable (v1's unconditional cap
  would have buried it); two traits sharing a method simple-name
  (`update` on two traits) — predicate is trait-name-keyed, immune;
  test-module impls (`store_tests::<StubIndex as VectorIndex>::score_set`,
  `test_role=None`) — vote via impl presence, harmless; external-trait
  bare-name collision (`trait Default` declared locally + std impls
  in graph) — latent, none today, predicate requires the trait be
  **declared in-graph** so an external trait never triggers it.
- Default methods with bodies (4 of the 14: `after_write`,
  `ontology_requirements`, `pool_size`, `per_endpoint_concurrency`):
  two are borderline (sole production impl overrides; default runs
  only in test stubs). v1 accepts Medium for all four; refinement
  knob (needs F1b): cap a default body only if some non-test impl
  lacks an override.
- Ship with: `--why-live` dispatch-proof line (it proves over edges
  only — post-F2 the report would say "doubtful" while `--why-live`
  says "no path"; the contradiction is real, `mod.rs:686+`).
- Build the trait→impls map sorted (invariant 6).

### F3 — plausible-filter loosening: **REJECTED by default** (kept as fallback)
Computed, not estimated: the relaxation demotes 17/105; 12 are already
carried by F2; the remaining 5 (`spawn`, `pool_for_url`,
`spill_partner_pool`, `is_zero`, `into_response`×2) demote only on
**coincidental** name matches (56 `tokio::spawn` sites; std
`Duration::is_zero` — whose real liveness is the F5 attribute-string
path; axum wrapper `Json(…).into_response()` sites) and every one is
correctly fixed by F4+F11/F5/F8 anyway. Without a stdlib-name screen
on the *sites*, the doubt pool adds 122 std `insert` and 54
`Sha256::update` coincidences — right verdicts, wrong reasons, and the
Medium band stops meaning anything. Keep the exact-equality rule
(`evidence.rs:136-144`); revisit only if Phases 1–4 leave a named gap.
Verified structural note: across all 105 High rows there are exactly 0
sites with empty/self-ish/owner-equal hints — the filter never fired
because such a site would already have blocked High.

### F4 — receiver-hint normalization (tightened by the collision map)
- Shared `normalize_receiver_hint` in `cgg-resolve::names`, used by
  type_hints Pass-3 (`:526-528` exact-key lookups that multi-line
  hints miss today), cross_file step gates, and `evidence.rs` — the
  verified collision map lists ~45 hint-shape inspection sites
  (e.g. `cross_file.rs:2643` is the **only** trim inside
  `try_resolve_ref`; internal newlines survive everything else;
  `evidence.rs:139-151` compares raw). Audit keeps raw text verbatim.
- Plugin-side extraction (`rust.rs:1216-1238`): struct-literal
  receivers → emit the `type` field path **with `::` preserved**
  (invariant 2); unwrap `await`/parens recursively (depth ≤4);
  collapse whitespace/newlines. Verified corpus hint text keeps
  source indentation (`"self\n            .vec_index"`), 112 newline
  hints, 384 `self*` compound hints.
- Explicitly **out of scope v1** (verified near-miss classes):
  tuple-index receivers (`line.1.len()` — unwrapping `.N` mis-attributes
  owners); tuple-struct receivers (`Json(InferenceResponse{…})
  .into_response()` → true callee is axum's external impl);
  `r#match` raw identifiers (word-scans mis-fire); block receivers
  (`{ … }.with_purpose()` — needs a last-expression transform);
  builder chains (`Client::builder()…build()` — today correctly
  suppressed *because* the head stays external; do not type-bind
  them).
- Verified capture set (struct-literal/await/paren class): 8 real
  instances (`main.rs:903` CascadeRunner — **also needs F11**;
  `expand.rs:2336,2356` `ExpandRequest{..}.clamped()`;
  `research/lib.rs:294,336,433,1046` `reporter.lock().await.update` —
  these land only once F10 makes `Arc<Mutex<dyn T>>` type-resolvable;
  `main.rs:1040` paren class).
- **CascadeRunner truth (v1 reversal):** the proximate loss is step-4c
  `is_external_rust_head` firing on the *in-workspace module* head
  `admin_jobs` (`main.rs:13` is `mod admin_jobs;`, no `use`) — the
  correct callee is among the 3 recorded candidates and never gets a
  turn. Second layer: `spawn` ∈ stdlib manifest
  (`stdlib/rust.txt:278`), so duck was always suppressed. Hint
  normalization alone does not fix this site; F11 does.

### F5 — serde attribute-strings → screened-site evidence
- Verified mechanism: attributes attach **only to callables**
  (`collect_attributes` called at exactly one site, `rust.rs:567`);
  `record_struct_fields` reads name+type only — field attribute
  siblings are never read. Zero graph callables carry a serde
  attribute. Corpus inventory: 109 string-valued serde-family attr
  occurrences, 60 distinct targets; ~50 map to High-band callables —
  the 63-row dominant class.
- Fix: new facts channel (per-file `attr_string_refs:
  Vec<(key, path_string)>`, keys allowlisted to `default`,
  `deserialize_with`, `serialize_with`, `getter`, `with`,
  `skip_serializing_if` — the allowlist is pre-approved by cgg's own
  gap text, `rules.rs:7390`); evidence consumes it exactly like the
  `dyn_uses` → `reflection` hook (`mod.rs:171-179`) emitting
  `NameCollidesWithScreenedSite { screen: "attribute-string" }`.
  No edges, no unresolved entries.
- Tightened by verification: resolve the string **file/module-relative
  and owner-qualified**, never by simple name — `fn default_true` is
  defined **8 times** (each named by its own module's attr), and
  `skip_serializing_if = "Vec::is_empty"` must resolve to std, not to
  the live `MultiIndex::is_empty`. Provenance-precise capping also
  keeps one module's attr from capping another module's dead twin.
- `is_zero` proves the class end-to-end: dead-listed, zero unresolved
  sites, live only via `skip_serializing_if = "is_zero"`.
- Flood audit: `example = "` 0, `#[doc =` 0, `#[schema(` 0,
  `description = "` only in code assignments — allowlist retained as
  policy; this corpus wouldn't flood but the guard is free.
- `--dead-code` report currently never hints `--reference-edges` or
  attribute strings for these rows; the why-line gains the screen slug.

### F6 — homogeneous trait candidate sets bind to the declaration
- Verified: the 361 `FanoutCapExceeded` sites **are** the dispatch call
  sites (`describes` at `import_routes.rs:4573` etc.); raising
  `--fanout-cap` would bind sites to *impls* and still leave
  declarations in-degree-0 — only this narrowing and F2 move the
  declaration itself.
- Tightened predicate (all four verifiers converge): fires only when
  (a) the set is homogeneous on one workspace trait `T` — reading
  `trait_impl_target` **or** wrapper-qn `<X as T>::m` (verified
  members with the marker in qn but empty field), (b) the declaration
  `T::m` exists in the graph, (c) test-module members are excluded by
  **name-shape regex** `\w*_?tests$` — the `::tests::` substring and
  `test_role` both miss `store_tests`-style modules (verified
  `test_role=None` test impl inside the `score_set` set), one-sided
  rule, (d) inherent-vs-trait-impl on the same owner are
  distinguished by the ` as ` wrapper marker — they share the
  normalized `by_owner_method` key (`ActivityTracker::in_flight`
  case) and an owner-only vote would conflate them, (e) synthetic
  `closure_at_*/async_at_*` names never participate in the vote,
  (f) everything else unchanged — std-trait sets (`Default` ×86,
  `Display` ×7, `From<…>` ×5, `IntoResponse` ×2 …, verified to have
  **no declaration node**) fall back to the cap path.
- Latent note: binding to a declaration with a default body would
  point edges at the default body; latent (no live fan-out sites for
  those names today).
- Cap-before-visibility ordering untouched (narrowing joins the
  existing one-sided set at the same stage); test `:4132` unaffected.

### F7 — unique-name fallback: **opt-in** (`--guess-unique-names`), re-scoped with real numbers
- Verified population: 2593 `candidates==1` sites; ~2200 are stdlib
  names (`iter` 1259, `join` 524, `map_err` 169, …) that the
  stdlib-name guard already refuses — reuse of that guard is
  mandatory, not stylistic. Residue ≈390, judged ~80–90% correct:
  68 have a test-module sole candidate, 123 carry external-looking
  heads. `spawn_periodic` is the one TRUE case where the external
  head would also block the typed bind.
- **Normalization-interaction trap (verified):** the judged-false
  survivors (`Router::…merge`, `tracing_subscriber::…init`,
  `http::…body`) are suppressed today *only because* their raw
  multi-line hints contain `::`, which trips the external-head guard.
  After F4 normalizes hints, that accident disappears — so F7's head
  check must test crate-root membership (the
  `is_external_rust_head` head set) **even without `::`**, and exclude
  test-module candidates (name-shape regex).
- Opt-in keeps the default graph byte-identical (repo posture for all
  graph-changing knobs); Low confidence; resolver tag
  `cross-file:name-unique`; docs-check 8 Python-parity entry required;
  `macro_arg_ref_retries` tests are the nearest-policy precedents.

### F8 — framework-consumed trait impls: **REVISED (naive version rejected)**
- Measured precision of v1's proposal on this corpus: **1/2.**
  `AppError::into_response` is genuinely live (182 bare
  `Result<_, AppError>` handler returns);
  `CreateDreamResponse::into_response` is genuinely **dead** — every
  return wraps it in `Json(…)`, so axum calls Json's impl and never
  the type's (verified at `dream_route.rs:143,251`). Rooting it is a
  false root; its High finding is correct. That Json-wrapped shape is
  the mandatory regression test for any version of this fix.
- Rejected specifics: the axum rule is `node: true` → base_types would
  mint `<framework-entry>` **nodes + edges**, and detection is
  workspace-wide (invariant 7), so one axum crate arms impl-matching
  across all 23 — with `base_type_matches` comparing last segments
  case-insensitively, a future local `trait Service` in any crate
  would be rooted as an axum contract. Capture population here is
  only 2 rows (derive-only serde: zero `impl Visitor/Serialize`
  callables exist; no FromRef/utoipa/tower-Service/clap-manual
  population — verified).
- Revised form: a **`node: false`** rule (the `rust-std-traits`
  pattern → roots-only via the `framework_roots` channel; no edges),
  detect-gated to axum, base_types `["IntoResponse"]` only (skip
  `Handler`/`Service` — collision vectors with zero captures here),
  shipped with the Json-wrap regression test. Precision on this
  corpus stays 1/2 — the honest framing is "one FP class cleared, one
  TP occasionally suppressed"; ship as a clearly-flagged tradeoff or
  hold. **Recommendation: hold behind the flag decision** — 1-row net
  gain does not clearly pay the root-inflation cost; revisit when a
  corpus with hand-written Serialize/Visitor impls (where F8's real
  population lives) is in scope.
- Verified nuance: the 22 unresolved `into_response` sites *do* exist
  but evidence screens them as owner-contradicting
  (`evidence.rs:145-165`) — the why-line stays self-consistent; F8 is
  the only lever that moves these rows correctly, F2 does not touch
  impl-side rows at all (disjoint target sets from the 12
  declaration rows).

### F9 — `macro_rules!` expansion: scoped project, danger reclassified
- Verified inventory: exactly 3 macros — `new_id!` (item template,
  fits, 7 invocations), `lazy_re!` (item template, fits, 14), `sym!`
  (expression position — excluded by design). The 3 macro *definitions*
  are already callables (`variant FreeFunction`, attrs `["macro"]`);
  bodies skipped verbatim at `rust.rs:404-408`.
- Verified captures: **146 unresolved callers of `new_id!`-generated
  API** (from 104, as_str 35, new 7) + 6 `lazy_re!` callers — which
  are **invisible in the graph JSON** (external name-screen,
  audit-only; verified) — success measurement must read
  `*.audit.json`.
- **Primary danger reclassified from resolution to reachability:**
  expanding `new_id!` synthesizes `Display::fmt` / `Default::default`
  / `From::from` impls, and `IMPLICIT_RUST_TRAITS`
  (`deadcode/roots.rs:34-90`) roots *any* impl of those names →
  ~21 silent new roots, reachability expands with no cap, and dead
  code turns live through synthesized impls. Mitigation is part of
  the design: synthesized impls get the `synthesized_from_macro` tag
  and are **excluded from implicit-trait rooting** (they have no
  hand-written body to be a root for) while `is_implicitly_invokable`
  caps them Medium.
- Other verified hazards to design around: two `from` bodies in one
  template collapse under `by_qn` (Owner::from ×2 → dedupe rule
  needed); `by_simple(from)` = 5 == cap today, +14 synthesized `from`
  would make one existing medium edge vanish (duck-pressure,
  measurable); `env!`/`include_str!`/`concat!`/`join!`/
  `embed_migrations!` (crates.io macros) must be excluded by
  **definition-in-tree**, not name; recursion: none in corpus;
  token-tree receiver reconstruction **drops `self`** through
  `select!`/`info!` chains (`idle_watcher.rs:173,243` → hint
  `signal`) — sub-fix 9c restores the prefix and converts 2 more
  sites.
- Phase 9a (measurement: pattern-fit rate across the benchmark corpus)
  still gates 9b.

### F10 — wrapper-aware type rewrite (NEW; bug-grade)
- Verified mechanism: `let rule_scheduler = Arc::new(nkb_rules::
  Scheduler::new(…))` → constructor table types the variable `Arc`
  (`type_hints.rs:628-647` overwrites `receiver_hint` in place) →
  `rule_scheduler.spawn_periodic()` loses its lowercase
  duck-resolvable hint (1 candidate, ≤ cap — resolvable today *before*
  the rewrite) to an uppercase std wrapper head → step-5 gate denies,
  4c denies, unresolved with `candidates: 1`. The tool destroyed its
  own resolvable site.
- Fix, both sides: constructor side — when the outer constructor is a
  smart-pointer/wrapper (`Arc/Box/Rc/Mutex/RwLock/ArcSwap/Cell/
  RefCell`), descend into the first argument for an inner
  `Type::new(…)`/`Type::default()` (prefer the fully-qualified path
  when present: `nkb_rules::Scheduler` → step-4 full-path hit);
  otherwise **skip the rewrite** (leave the lowercase hint).
  Annotation side — `parse_colon_param` cuts at the first `<`, so
  `x: Arc<Mutex<dyn T>>` types to `Arc`; unwrap wrapper chains to the
  inner named type (`Arc<T>→T`, `Mutex<T>→T` via guard-deref
  semantics), depth ≤3. Verified payoff includes the whole
  `reporter.lock().await.update` class (F4's await captures only land
  once this exists) and the `pool_for_url`/`spill_partner_pool`
  hint-`Arc` rows.
- Regression direction is recall-positive: it can only replace
  never-resolvable wrapper heads; guarded by the same workspace-owner
  test as F1.

### F11 — in-workspace module heads are not external (NEW)
- Verified: `is_external_rust_head` (`cross_file.rs:3175-3202`) fires
  on any head absent from crate roots + import aliases;
  `main.rs:903`'s `admin_jobs::CascadeRunner{…}.spawn()` — `mod
  admin_jobs;` with no `use` — is refused at step 4c **before the
  candidate set is consulted**, and the correct callee is sitting in
  that set. Any same-crate `module_path::Type::m()` call from a file
  lacking a `use` for that module hits this.
- Fix: collect module declarations into facts (`mod_item` is already
  walked/scope-pushed at `rust.rs:328`; add the name to a
  `facts.mod_decls` set per crate), and have `is_external_rust_head`
  answer false when the head matches an in-workspace module path
  (longest-prefix for `a::b::` chains). Owners for step 4 then
  normalize via the existing last-segment path.
- Guardrails (must stay green — these define the guard's purpose):
  `rust_external_crate_head_produces_no_edge` (`:4307`),
  `rust_unknown_nested_head_still_produces_no_edge` (`:4975`),
  `rust_external_head_owner_match_requires_full_path` (`:5570`).
  Bare-name owner collisions across modules (`ConfigSource` declared
  in both `nkb_common::config` and `nkb_dream::config`, verified) mean
  a module-head hit should prefer the full-path index
  (`by_full_owner_method`) over the bare key when the head resolved
  as a module path.

### F12 — turbofish callees: drop must become a reference (NEW; bug-grade)
- Verified: `ref_from_call` (`rust.rs:1203-1237`) handles
  `identifier`/`field_expression`/`scoped_identifier` callees and hits
  `_ => return None` for `generic_function` (the node kind of
  `f::<T>(…)`; the kind string appears nowhere in the plugin) → the
  call produces **no reference and no unresolved record**. Verified
  end-to-end: `call_llm_typed::<…>` at `passes.rs:146/292/390` with
  **zero** unresolved mentions anywhere — the why-line's
  "no-unresolved-site-with-name" is literally true and entirely
  misleading. ≥3 false-High rows (`call_llm_typed`,
  `call_explore_llm`, `read_and_parse`); also means my original
  feature analysis wrongly called `call_llm_typed` dead.
- Fix: unwrap the callee's `function` field and emit the reference
  (name + receiver as for the un-turbofished form); add the class to
  `caps.rs BLIND_SPOTS` (it is currently absent from the disclosed
  table). `turbofish_receiver_is_dropped` (rust.rs:2326) concerns the
  *receiver* position and must stay as-is.

### F13 — value-position callees: screened evidence, never edges (NEW)
- Verified: `.map(iso_day)`, `and_then(to_instant)`,
  `query_map(…, edge_row)` bind via `Via::Reference` and are pruned
  before dead-code unless `--reference-edges` — 6 false-High rows
  (including `iso_day` ×2, which my first pass wrongly called dead).
- Fix honoring the opt-in posture: do not un-prune by default; record
  the value-position site as evidence (new screened-style channel
  fed pre-prune, `Polarity::Lowers`, cap Medium — same posture as
  `external`/`reflection`). `reference_edges_confer_liveness`
  (mod.rs:956) stays the opt-in edge semantics; the report's why-line
  can then name the mechanism instead of contradicting itself.

## 3. Sequencing (v2)

- **Phase 0** — diagnostics script over `nkb_graph.json` **+
  `nkb_graph.json.audit.json`** (external-screen hiding, §0 note):
  hint-shape × stage tabulation; replay of F2's tightened predicate
  over the 105 rows (expected: 13 demotions, `summarize` retained);
  F10 pass replay (expected: `spawn_periodic`, `pool_for_url`,
  `spill_partner_pool` resolve typed or duck).
- **Phase 1 (bug-grade, low blast radius)** — F12, F10, F11, F1b.
- **Phase 2 (dead-code truth)** — F2 (+ `--why-live` line), F13.
- **Phase 3 (resolver)** — F4 + F1 together (shared normalizer;
  order: F4 normalize → F1/F10 typed rewrites → cross_file steps),
  then F6.
- **Phase 4** — F5 (facts channel + evidence + file-relative
  resolution); F8 decision point (hold-recommended).
- **Phase 5** — F7 behind the opt-in flag (after F4 has shrunk its
  guess population).
- **Phase 6** — F9: 9a fit-rate measurement across the benchmark
  corpus (incl. macro-embedded-self sub-fix 9c budget), go/no-go for
  9b with the root-exclusion design mandatory in 9b itself.

## 4. Gates and expected churn (verified test names)

Every phase: `cargo test --workspace`;
`determinism.rs` (incl. `dynamic_dispatch_fanout_is_ordered`,
dead-code-identical-at-every-thread-count `:271`); py/node parity
(`test_analyze.py` structural parity incl. dead-mode,
`analyze.test.js`); `docs-check.py` (F7 flag ⇒ flag table + Python
keyword; F5 ⇒ attributes prose counts; F12 ⇒ BLIND_SPOTS).

Named guards per family (churn here = the fix working; review each):
`cross_file.rs`: `fanout_cap_is_decided_before_the_visibility_filter_
not_after` (:4132), the 4c guards (:4307/:4975/:5570/:4898),
`rust_bare_self_receiver_is_never_cross_file_resolved` (:4604),
stdlib/From guards (:4702/:5402/:5478), macro-retry guards
(:5238/:5311/:5700 — F9-sensitive); `resolve.rs`:906
`chained_call_dropped_at_the_cap…` (asserts the literal stage string);
`evidence.rs`: `promotion_to_high_is_conjunctive`,
`any_doubt_blocks_promotion`, `polarity_partitions_the_evidence_enum`
(F2 must register polarity + slug), `unresolved_name_collision_caps…`;
`mod.rs`: visibility High-guards (:1030/:1104), `why_live_*` (:1179–
:1222 — F2 adds a proof line), `reference_edges_confer_liveness`
(:956); `roots.rs`: `implicit_rust_traits_are_roots` (:786 — F9's
root-exclusion must keep user traits non-rooted while excluding
synthesized std-trait impls), `serde_visitor_methods_are_runtime_
invoked` (:858); `rust.rs`: `visibility_and_async_captured` (:2114 —
F1-compatible), macro token-tree tests (:2227/:2261/:2298/:2326 —
F9/F4-sensitive); `frameworks/mod.rs`: `string_targets_are_opt_in_
per_rule`, `bucket_d_lifecycle_rules_do_not_mint_nodes`, gap-coverage
tests (F8 churn); `type_hints.rs`: `self_field_rewrites_to_the_declared_
type` (:1433), locals-scoping tests (:1227/:1269/:1418) — F1 guard
must keep them.

Rollout notes: any demotion flips `unreferenced` node flags → JSON and
`--fail-on-dead` scope shrink; F9 adds callables/roots (README stats
regenerate via hooks); F7 default-off keeps everything byte-identical
unless requested.

## 5. v1 → v2 ledger

| v1 claim | verdict | v2 state |
| --- | --- | --- |
| "~70/105 High false" | REFUTED | ~89/105 (63 serde / 12 trait-decl / 12 resolver-gap / 2 framework+test / 16 dead) |
| C2 trait-decl rows = 8 | CORRECTED | 14 claimed → 13 captured (summarize is true-dead; holding a `dyn` field ≠ calling it) |
| async self-field fix "repairs zero nkb rows" | REFUTED | 10 captures + 88 upgrades + 5 guarded regressions |
| CascadeRunner culprit = struct-literal hint | CORRECTED | step-4c module-head over-refusal (F11) + stdlib `spawn`; normalization alone insufficient |
| spawn_periodic `Arc` hint = receiver text | CORRECTED | constructor-rewrite artifact (F10); resolvable site destroyed by typing pass |
| F3 relaxation as deferred hardening | REJECTED by default | computed 17→12-overlap→5-coincidental; every gap covered elsewhere |
| F6 same-trait uncap | REVISED | bind the declaration; predicate requires in-graph declaration + wrapper-qn parse + test-shape regex |
| F7 population 2480 safe guesses | RE-SCOPED | 2593 → ~390 residue, 80–90%; guards mandatory; opt-in |
| F8 axum base_types | REVISED / hold | measured precision 1/2 (Json-wrap true positive); roots-only node:false form or hold |
| F9 danger = resolution noise | RECLASSIFIED | implicit-trait root inflation; root-exclusion ships with 9b |
| turbofish / value-position classes | MISSED | F12/F13 — new; turbofish has zero audit trace, absent from BLIND_SPOTS |
| `Engine::run_one` true-dead reclassification | CONFIRMED twice | stays |

Nothing is categorically wrong; two fixes were re-scoped (F3→rejected
by default, F9→project), one held (F8), two bug-grade classes were
missing entirely (F12/F13), and the headline number moved +19.
