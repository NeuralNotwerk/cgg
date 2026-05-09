# Task 5 — Intra-file linker, enclosing_callable, confidence scoring

## What shipped

- **`cgg_resolve::intra_file::link_file`** — ports codescope's
  `get_containing_def_for_ref` with byte-based smallest-enclosing-range
  semantics. Scans every `RefRecord` in a `FileFacts`:
  * Locates the smallest definition whose byte range contains the
    reference's `site_byte`; that becomes the edge's `src`.
  * Matches the reference's `simple_name` against all definitions in
    the same file.
    * **1 candidate** → `CallEdge` with `confidence=High`,
      `via=Direct`, `resolver="intra-file"`.
    * **0 candidates** → `AuditUnresolvedCall { reason: "no-candidate-in-scope" }`.
    * **≥2 candidates** → `AuditUnresolvedCall { reason: "ambiguous-in-file" }`
      (Task 6 collapses these via scope graphs).
  * **Cycles preserved** — recursion (`fn f() { f(); }`) and mutual
    recursion produce real edges; there is no cycle removal anywhere.
- **Minimal mermaid writer** (`cgg_format::mermaid::MermaidFormatter`)
  implementing `GraphFormatter`. Emits `flowchart LR` with `C<id>`
  node identifiers and the qualified name as the label; escapes `<`,
  `>`, and `"`. Empty graphs render a placeholder `Empty` node so the
  file is still mermaid-valid. Task 9 adds per-language subgraphs and
  edge styles for `via` / `confidence`.
- **`cgg` binary** now runs the full pipeline:
  `walk → detect → parse → extract → build Graph → intra-file link →
   emit graph + audit`. Audit destination rules:
  * `--metrics FILE` → audit to FILE.
  * `-t json` and no `--metrics` → audit embedded (primary output).
  * other formats → sidecar `<output>.audit.json`.
- `confidence_histogram`, per-language `edges` and `unresolved`
  counters, and `phases.link_ms` are now rolled up in the run metrics.

## Artifacts

- `out.mmd` — mermaid graph for the demo fixture.
- `out.jsonl` — full JSONL audit with per-file records.
- `run.stderr.txt` — one-line run summary from stderr.
- `cargo-test.txt` — full workspace test run (76 passed).

## Demo

Fixture `/tmp/cgg-intrafile/`:

- `calc.rs` — five free functions:
  `add`, `mul` (calls `add`), `dot` (calls `add` and `mul`),
  `fact` (recursive), `noop` (calls `absent_fn` — intentionally
  absent so we can see the unresolved path).
- `svc.py` — two classes:
  `class Service: handle → parse + render; class Other: handle`.

Mermaid output (`out.mmd`):

```
flowchart LR
  C0["svc.Service.handle"]
  C1["svc.Service.parse"]
  C2["svc.Service.render"]
  C3["svc.Other.handle"]
  C4["crate::add"]
  C5["crate::mul"]
  C6["crate::dot"]
  C7["crate::fact"]
  C8["crate::noop"]
  C0 --> C1
  C0 --> C2
  C5 --> C4
  C6 --> C4
  C6 --> C5
  C7 --> C7
```

Run summary from stderr:

```
cgg: 2 files discovered, 2 analyzed, 0 skipped;
     9 callables, 6 edges, 3 unresolved (3.0 ms).
     [Task 5: intra-file linker wired]
```

Key observations:

- **`C7 --> C7` self-edge is preserved** — the `fact` recursion
  cycle appears as a real edge in the graph, exactly as required.
- **`svc.Other.handle` is correctly left unconnected** — it's never
  called in the file. (Had someone called `.handle()` on an unknown
  receiver, the linker would mark it ambiguous because both
  `Service.handle` and `Other.handle` share the simple name.)
- **Three unresolved calls** captured in audit:
  * `str` (Python builtin) in `svc.Service.render`
  * `len` (Rust method on slices) in `crate::dot`
  * `absent_fn` in `crate::noop`
- **Confidence histogram** = `{high: 6, medium: 0, low: 0}` — all
  intra-file matches were single-candidate hits.

## Test counts

- `cgg-resolve` intra-file unit tests: **6 passed**
  * `single_match_emits_edge`
  * `zero_candidates_unresolved_no_candidate`
  * `ambiguous_name_flags_unresolved`
  * `smallest_enclosing_wins`
  * `self_call_is_preserved_as_edge`
  * `ref_outside_any_def_is_unresolved`
- `cgg-format` mermaid unit tests: **3 passed**
  * `renders_nodes_and_edge`, `empty_graph_is_still_valid`,
    `angle_brackets_escaped`.
- `cgg` binary integration (`link.rs`): **5 passed**
  * `rust_intra_file_edges_emit_mermaid`
  * `recursion_preserves_self_edge`
  * `python_intra_file_method_to_method`
  * `unresolved_reference_shows_up_in_audit`
  * `metrics_count_edges_and_confidence`
- Workspace total after Task 5: **76 tests passed**.
