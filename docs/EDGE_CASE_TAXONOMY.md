# cgg edge-case taxonomy — verified misses on the nkb corpus

Date: 2026-10-10. cgg 0.10.1, commit `17b89f3`. Corpus: `/storage/neurosymbolic_kb`
(a 23-crate Rust workspace + vendored JS assets; 314 files, 6070 callables,
16904 edges, 23887 unresolved call sites, dead-code: 105 high / 785 withheld).

Method: every finding below was verified against the corpus source by
caller-grep triangulation (an independent reading of which functions are
actually wired) and the audit's own `unresolved` list; this section was
subsequently re-adjudicated row-by-row by four independent verification
agents (FIX_PLAN.md §5 ledger). Corrected headline: **~89 of the 105
high-confidence dead-code findings are false** (range 85–90; the
earlier ~70 understated), and the class breakdown is: **63** serde
attribute-string names, **12** trait declarations dispatched via
dyn/generics, **12** resolver blind spots (turbofish callee-drop ×3,
value-position drop ×6, receiver-shape ×3), **1** framework-consumed
impl, **1** test-only — **16 actually dead**. cgg never *fabricates* a
call site on Rust here — its Rust misses are precision over-engineered
into recall holes.

## Categories with ≥5 instances (each deserves exploration)

### C1 — Attribute-string references counted as nothing (60+ verified false-High)

`#[serde(default = "default_hops")]` names `default_hops` in a string, so
cgg emits no edge — by design ("a string literal that names a callable is
never a call", `cross_file.rs:1370-1373`). The bug is downstream: the
`continue` happens *before* any unresolved-site or screened-site is recorded,
so the dead-code evidence index (`deadcode/evidence.rs:89-97`) finds no
same-name site and **raises** `NoUnresolvedSiteWithName` — the exact string
`no-unresolved-site-with-name` then corroborates ~60 findings to High
(`default_hops`, `default_true`, `default_k`… across 21 server files;
verified live at e.g. `server/src/graph_routes.rs:402`).

Fix: string/attr refs should feed the existing
`Evidence::NameCollidesWithScreenedSite { screen: "attribute-string" }`
(`cgg-core/src/deadcode.rs:238`) — the machinery for a weak lowering
evidence already exists; STRING_REF just never reaches it.

The blind spot itself is already acknowledged in-tree: the serde rule's
gap text (`cgg-core/src/frameworks/rules.rs:7377-7393`) says "cgg records
no reference at all from a field attribute, so every one of those helper
functions … reads as dead while being called on every
(de)serialization". The extraction gap is known; the open bug is only
the bookkeeping — the dead-code evidence layer reads the resulting
*absence* of a site as corroborating evidence
(`no-unresolved-site-with-name`) and promotes, instead of treating it as
missing information.

### C2 — Dispatch-surface trait declarations flagged never-referenced (13 verified false-High; 14th row true-dead)

The plausible-receiver filter (`deadcode/evidence.rs:136-144`) keeps an
unresolved site as doubt only when its `receiver_hint` is empty, in
`{self,Self,this,cls,super}`, or **string-equal to the owner**. Every hint a
real Rust caller produces fails all three:

| flagged callable | observed hint at the caller | truth |
|---|---|---|
| `VectorIndex::insert` (`vector_index/src/lib.rs:26`) | `self.vec_index`, `index`, `s.vec_index` | called everywhere via `Box<dyn VectorIndex>` |
| `VectorIndex::score_set` (`:34`) | `self\n.vec_index` (newline raw text) | same |
| `ImportSource::{describes,next_batch,after_write,ontology_requirements}` (`server/src/import_routes.rs:168-202`) | `src`, `x`, `Arc` | the whole import-pack subsystem |
| `EmbeddingClient::{pool_size,per_endpoint_concurrency}` (`embed/src/lib.rs:28,33`) | field names | trait surface |

~~`Engine::run_one`~~ (`rules/src/engine.rs:405`) was in this table; it
is a **true positive**: a *private* `fn` carrying the author's own
`#[allow(dead_code)]`, so cross-crate callers are impossible — the
manager/dream/admin_jobs `run_one` sites belong to
`admin_jobs::CascadeRunner::run_one`, a different method whose edge
resolves fine. Found during the FIX_PLAN.md execution pass. Note every
surviving row above is a **trait declaration** (mix of 1-line abstract
sigs and default-method bodies), not an impl — the false Highs are the
dispatch surface itself, in-degree 0 because the call sites feeding it
failed elsewhere. Row-by-row re-adjudication found **six further**
declaration rows in the same shape (`DreamLlm::{complete_synthesis,
synthesis_capacity}`, `DreamCommunity::summarize`,
`DreamExtractor::extract`, `DreamProgressReporter::update`,
`ProgressReporter::update`) — and excluded one:
`DreamCommunity::summarize` is **truly dead** despite its trait being
held as `Arc<dyn DreamCommunity>` — there are zero `.summarize(` call
sites anywhere. Holding a trait object ≠ calling its methods; the
fix's predicate must require dispatch *evidence* (same-name unresolved
sites or non-dynamic impl references), not mere impl existence.

`self.vec_index` ≠ `self`; `engine` ≠ `Engine` (case); `Arc` is a wrapper,
not a type contradiction. The sites exist with matching names — the report
still says "no unresolved site with name" because the plausible filter
discarded them as "contradictory" when they are merely un-type-propagated.

Mechanism, verified against `cgg-lang/src/plugins/rust.rs` (node kinds
checked against tree-sitter-rust-0.24.2 `node-types.json`):

- Receiver hints are the **raw verbatim source text** of the
  `field_expression.value` node (`rust.rs:1218-1225`) — method calls in
  this grammar are `field_expression` (there is no
  `method_call_expression`).
- A `self.<field>` rewrite into the declared field type does exist, via
  `emit_self_field_local_types` — but that pass filters definitions on
  `InherentMethod | TraitMethod` (`rust.rs:1326-1335`), and an
  `async fn` in an impl is classified `AsyncFunction` instead of either
  (`rust.rs:536-544`: async *replaces* the scope-derived variant). So
  **every async impl method — including all `#[async_trait]` methods —
  gets zero `self.<field>` type facts**, and its hints stay raw. In an
  axum/async_trait corpus like nkb that is most of the impl surface.
- When the pass does fire it is one level deep, and generic field types
  (`vec_index: V` in `Memory<V>`) survive `simplify_rust_type` as `V` —
  no owner by that name exists, hint stays raw. (`Box<dyn T>`/`Arc<dyn T>`
  fields do simplify to `T` and work — `rust.rs:1378-1413`.)
- Type propagation then skips any hint that starts uppercase or contains
  `.` as "already a type path" (`type_hints.rs:~555`) — no normalization
  of raw text is ever attempted.
- Downstream, both the resolver's typed path and the dead-code plausible
  filter compare that raw text to the owner name by string equality
  (`deadcode/evidence.rs:136-144`).

Fix: (a) accept hints with a `self.`/`Self::` prefix; (b) case-insensitive,
path-suffix-tolerant compare; (c) a hint that names no known workspace type
is *unknown*, not a contradiction; (d) unwrap `Arc/RwLock/Box` wrapper
words. Also normalize hints like `self\n.vec_index` to one line.
Upstream there is a genuine extractor bug — `async fn` in an impl is
classified `AsyncFunction` (`rust.rs:536-544`), which is filtered out of
`emit_self_field_local_types` — but the FIX_PLAN.md execution pass
corrected an earlier over-claim here: this corpus's receivers are
trait-object loop vars (`src`, `x`), `State`-field chains
(`s.vec_index`), and *generic*-typed self fields (`vec_index: V` →
simplifies to `V`, which names no owner), so the async fix repairs zero
of the rows above. It is a correctness fix for *other* corpora. The
remedy for these rows is binding the call site to the trait declaration
when the fan-out candidate set is homogeneous in `trait_impl_target`
(FIX_PLAN.md F6), plus a dead-code cap that treats an implemented trait
declaration as a dispatch surface (F2).

### C3 — Chained receivers across lines yield no usable hint (2552 unresolved sites; 1 false-High)

`admin_jobs::CascadeRunner { .. }\n    .spawn();` (`server/src/main.rs:903`)
— receiver is a multi-line struct literal and the hint is its **verbatim
raw text** (`admin_jobs::CascadeRunner {\n defer_gate: … }`) — not empty,
just unmatchable. (tree-sitter-rust has no `method_call_expression`:
method calls are `field_expression`, whose `value` is `_expression`, so
struct literals / parens / awaits reach the hint builder as full
expressions — verified against `node-types.json`. Chained calls leak
`self\n.vec_index` the same way, via the inner chain node's text.)
Verification corrected the culprit: the resolution dies at step-4c's
external-head refuse — `main.rs:13` is `mod admin_jobs;` with no
`use`, so `is_external_rust_head` classifies the in-workspace module
head as external and returns `None` **before the candidate set is
consulted**, and the correct callee (`CascadeRunner::spawn`) is
sitting in the 3 recorded candidates
(`candidates-in-other-files`). Second layer: `spawn` is in the Rust
stdlib manifest (`stdlib/rust.txt:278`), so duck fan-out would have
been suppressed for it regardless. → the call never becomes an edge
and `CascadeRunner::spawn` reads D001 dead. Corpus-wide: 7746 source lines
start with `.method(`; 2552 of those sites are unresolved.

Fix: for `Expr{..}.m()` emit the struct name as the hint; a newline-bearing
hint means raw-text receiver text leaked and should be normalized.

### C4 — `macro_rules!` generated APIs are invisible (21 orphan sites, ~20 generated methods)

`common/src/id.rs:4-50` — `new_id!(DocId,"doc")` ×7 generates
`from_str`/`Display` per newtype; **zero** generated callables are in the
graph, and the seven invocation sites land in
`no-enclosing-callable` (file-scope macros have no enclosing callable —
correct behavior, wrong outcome: the callees are credited to nobody).
Same for `ingest/src/issue_trackers.rs:727` `lazy_re!` (×14 orphan sites;
callers of `path_re()` etc. resolve to nothing). `vec_gpu/src/lib.rs:150`
`sym!`. This is cgg's biggest Rust recall hole: an entire generated API
surface (newtype `from_str`, static regex getters) has neither callables
nor call edges, so deadness and reachability of its callers is unknowable.

Mechanism (verified in `plugins/rust.rs`): the walker records the macro
definition as a callable, then returns at `rust.rs:404-407` with the
in-tree comment *"Don't walk into macro body (token trees aren't
useful)"* — true for `vec!`/`format!`-class expression macros, wrong for
item-generating macros like these. Invocations leak *refs* out of their
token trees (`refs_from_token_tree`, receivers reconstructed from flat
token adjacency) but never new *callables*, so the generated
`DocId::from_str` surface has nothing to bind to.

Fix: synthesize callables from expansion sites (parse the
`macro_definition` body template, apply the invocation's args — `new_id!`
is 1 pattern × 7 instantiations); for orphan sites, attribute
file-scope macro invocations to the *synthesized* callable instead of
dropping them.

### C5 — Single-candidate name-only refusal (2480 safe guesses declined)

`candidates-in-other-files` is the dominant unresolved bucket (17812 Rust
sites). The policy is a precision choice (no false edges from name matching
— correct for `iter`/`join`/`new`), but it is applied uniformly: `spawn_periodic`
at `server/src/main.rs:433` (`rule_scheduler.spawn_periodic()`) has
`candidates: 1` — a **globally unique name** — and the edge is still refused.
Verified correction to the mechanism: the recorded hint `Arc` is a
**tool artifact**, not receiver text — `rule_scheduler =
Arc::new(nkb_rules::Scheduler::new(…))` makes the constructor-typing
pass overwrite the resolvable lowercase hint with the *wrapper* type
name (uppercase → duck gate denied, std wrapper → typed dead-end);
the site was duck-resolvable (1 candidate ≤ cap) until the tool's own
typing pass destroyed it (FIX_PLAN.md F10).
Real population, replacing the earlier "2480 safe guesses": of 2593
single-candidate sites, **~2200 are stdlib names** (`iter` 1259,
`join` 524, `map_err` 169, …) whose suppression is correct — the
genuine guess population is ~390, measured ~80–90% right, with 68
test-module-candidate and 123 external-head sites to exclude. Dead-code verdicts mostly survive (mitigation caps them
to medium), but any consumer of the *graph* (reachability, "is feature X
exposed?") systematically undercounts; our first pass over this corpus
produced 443 false "not exposed" gaps for exactly this reason.

Fix: emit a Low-confidence edge when the name is globally unique in the
language (`name-unique` as the safety condition), or add
`--guess-unique-names`.

### C6 — Fanout cap truncates declared trait sets (360 Rust sites)

`score_set` exceeds `--fanout-cap 5` with exactly **6** real `VectorIndex`
backends; `describes`/`next_batch` with 12 `ImportSource` impls; `run` with
33 (subcommand dispatch). The cap counts same-name duck candidates and
same-trait impls alike — but a trait's impl set is *declared*, not guessed
(`trait_impl_target`), the same reasoning that exempts C++ vtables from the
cap (`cross_file.rs` `cpp_virtual_dispatch`). Note the trap: `--dead-code`
turns on `--dynamic-dispatch` and the fanout still applies to the
call-site→declaration step, so the edges it just enabled are dropped again.

Fix: when all candidates for a name are impls of one trait, exempt from the
cap (or count them as one trait-declaration candidate).

### C11 — Function values passed as arguments disappear without a trace (6 verified false-High)

`.map(iso_day)`, `.and_then(to_instant)`, `query_map(…, edge_row)` —
a bare callable name in *argument position* binds via `Via::Reference`,
which the pipeline prunes before dead-code unless `--reference-edges`
(CLAUDE.md phase 3.5, "never change the default graph"). The callee is
therefore live in reality and `NeverReferenced` in the report —
`iso_day` ×2 (`issue_trackers.rs:1239/1243`,
`release_source.rs:474`), `to_instant`/`to_ms` (`host_throttle.rs:287,288,544,545`),
`edge_row`/`ent_row` (`feedback_routes.rs:584/587/538/541`). The
High-band report never hints that `--reference-edges` would resolve
the contradiction.

Fix: keep the opt-in posture for *edges*, but record the value-position
site as a screened-style **evidence** channel (`Polarity::Lowers`, cap
Medium — the `external`/`reflection` precedent), so the finding stays
reportable and honest without the default graph changing.

## Categories below 5 on this corpus (noted, not deep-dived)

- **C7 Framework generic consumption (1 false + 1 true):** verified split —
  `<AppError as IntoResponse>::into_response` is a **false** High (182 bare
  `Result<_, AppError>` handler returns; axum's machinery invokes it, caller
  out-of-tree by construction), while `<CreateDreamResponse as IntoResponse>::into_response`
  is **truly dead** (every return site wraps it in `Json(…)`, so axum calls
  Json's impl, never the type's — `dream_route.rs:143/251`). A naive
  "root impls of framework traits" fix therefore scores 1/2 precision here,
  and framework detection is workspace-wide per rule (one axum-importing
  crate arms the matcher for all 23), so the root-only variant of this fix
  ships with the Json-wrapped shape as its regression test — or not at all
  (FIX_PLAN.md F8: hold-recommended, revisit on corpora with hand-written
  Serialize/Visitor impls).
- **C10 Turbofish callee drop (3, but bug-grade):** `f::<T>()` calls record
  **no reference and no unresolved record** — `ref_from_call` handles
  `identifier`/`field_expression`/`scoped_identifier` callees and discards
  the `generic_function` callee kind silently (`rust.rs:1203-1237`; the kind
  appears nowhere in the plugin). Verified end-to-end: `call_llm_typed::<…>`
  call sites exist (`passes.rs:146/292/390`) with **zero** unresolved mentions
  — the why-line's "no-unresolved-site-with-name" is literally true and
  wholly misleading. Absent from cgg's own `BLIND_SPOTS` table. Below the 5-row
  bar but flagged as a bug, not a policy miss.
- **C8 Minified-JS noise:** vendored `cytoscape.min.js`/`alpine.min.js` account
  for 669/691 `no-enclosing-callable` sites and most one-letter-name unresolved
  entries. `--skip-minified` exists; use it on web corpora.
- **C9 Duplicate helper names:** `default_hops` defined in two files, both
  false-High via C1 — name-keyed screens should be owner-aware.

## What cgg got right (guardrails not to regress)

- 0 fabricated call sites on this corpus's Rust; the 205 `dispatch:fanout`
  dynamic edges checked are accurate (trait-declaration → impls, cross-crate),
  and `--dead-code` force-enables that fan-out (`lib.rs:242`, confirmed by
  `dispatch.rs`'s own comment). The C2-class dead-code misses are
  *declarations* at in-degree 0 — the impls they dispatch to already have
  correct dynamic edges.
- Spawn boundaries are handled: `spawn_call_for` + `emit_spawned_closure`
  name spawned closures as callables and emit the spawner edge — witness:
  `CascadeRunner::spawn::async_at_234 → CascadeRunner::run_one` resolves
  in this very graph. "spawn" is not a miss category.
- Test-scope exclusion: 839 only-used-by-tests correctly withheld from high.
- Example binaries' `main` as roots; framework-entry roots for axum.
- The STRING_REF design itself — no false edge from strings — is right;
  only the confidence bookkeeping (C1) is wrong.
- Spawn handling is better than it looks: spawned closures become named
  callables with spawner edges (`async_at_*` in the graph), and
  `--dead-code` force-enables the (accurate) trait-dispatch fan-out.
- Measurement caveat for any fix validation: the `external` name screen
  records some unresolved refs **audit-only**
  (`cgg-core/src/external.rs:159-163`) — verified, the `lazy_re!`-generated
  callers appear in `*.audit.json` and are absent from
  `graph.unresolved`. Capture counts read from the graph JSON
  understate; read the audit sidecar.

## Reproduce

```sh
cgg <corpus> -t json -o /tmp/g.json --dead-code --dead-code-format text \
    --dead-code-report /tmp/dc.txt --framework-coverage
# taxonomy mining: /tmp/g.json `unresolved[].reason.stage` × receiver_hint;
# false-High audit: diff /tmp/dc.txt high list against caller-grep of each name.
```
