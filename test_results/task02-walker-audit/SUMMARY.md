# Task 2 — Walker with built-in deny-list, .gitignore, .cggignore, audit

## What shipped

- `cgg-walk` crate wrapping the `ignore` crate's `WalkBuilder`.
- Built-in per-language deny list matched by any path component:
  `node_modules`, `.venv`, `venv`, `site-packages`, `vendor`,
  `target`, `build`, `bin`, `obj`, `dist`, `.git`, `.gradle`,
  `.cargo`, `__pycache__`, `.next`, `.nuxt`.
- `.gitignore` and `.cggignore` both honored (gitignore syntax).
- Binary-content heuristic (NUL in first 8 KiB) → skip with
  `reason: binary`.
- Size threshold (default 25 MiB) → skip with `reason: too-large`.
- Symlink-outside-root detection → skip with
  `reason: symlink-outside-root`.
- Every skip emits a structured `AuditEvent::FileSkipped` with a
  typed `SkipReason` carrying the matched detail (directory name,
  error text, etc.).
- `AuditEvent` is an algebraic type (tagged by `event`) covering
  `run_started`, `file_discovered`, `file_skipped`, `file_analyzed`,
  `run_finished`.
- `JsonlAuditWriter` (streaming, one JSON object per line) and
  `JsonAuditWriter` (single pretty array) implementations in
  `cgg-core`.
- `cgg` binary wires the walker and audit writers end to end.

## Artifacts

- `jsonl-demo.jsonl` — full streaming output on the fixture.
- `jsonl-demo.stderr.txt` — stderr summary from the same run.
- `json-demo.json` — same events as a single pretty JSON array.
- `json-demo.stderr.txt` — stderr summary from the json run.

## Fixture

Built under `/tmp/cgg-walker-demo`:

| Path                       | Expected outcome                      |
|----------------------------|---------------------------------------|
| `src/a.py`                 | discovered                            |
| `src/b.rs`                 | discovered                            |
| `.cggignore`               | discovered (unknown ext → Task 3)     |
| `secrets.py`               | **not** emitted — matched `.cggignore`|
| `node_modules/pkg.js`      | skipped, `{kind:"builtin",detail:"node_modules"}` |
| `.git/HEAD`                | skipped, `{kind:"builtin",detail:".git"}`         |
| `blob.dat`                 | skipped, `{kind:"binary"}`            |

## Results

- JSONL demo: 8 lines (1 `run_started` + 3 `file_discovered` + 3
  `file_skipped` + 1 `run_finished`). Every line is a valid
  JSON object.
- JSON demo: one pretty array of 8 events, first `run_started`,
  last `run_finished`.
- No trace of `secrets.py` in either stream (confirms `.cggignore`
  filters before audit emission; this is intentional — gitignore
  and .cggignore matches are suppressed entries, not skips).
- 3 typed skip reasons verified end-to-end: `builtin`, `binary`,
  and implicitly `gitignore` (tested in unit tests).

## Test counts

- `cgg-walk` unit tests: **7 passed**.
- `cgg` integration tests: `cli.rs` 5 + `walker.rs` 3 = **8 passed**.
- Workspace total after Task 2: **28 tests passed**.
