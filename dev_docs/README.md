# dev_docs

Internal reference docs for cgg's language parity work. These are not user documentation: the user-facing docs are the repo `README.md` and `skills/`. Every claim here cites code (`file:line`) or a fixture run, as of 2026-10-17 (0.11.0 plus the `remediate/cross-language` commits).

**Start here: [PARITY_MATRIX.md](PARITY_MATRIX.md).** It has the 46-language × 16-category overview, the top 30 parity gaps ranked by reach × cheapness, and coverage notes.

## Layout

| Path | Contents |
| --- | --- |
| [PARITY_MATRIX.md](PARITY_MATRIX.md) | Combined overview matrix, top 30 gaps, legend, coverage notes |
| [languages/](languages/) | 46 language catalogues, one per registered plugin (`rust.md`, `python.md`, …). Each has sections 01–16 matching the categories below, plus a summary scorecard |
| [frameworks/](frameworks/) | `rules-part-1.md` to `rules-part-4.md`: every `RuleSpec` in `crates/cgg-core/src/frameworks/rules.rs` (475 specs), with its triggers, matching semantics and enumerating or gap-only status |
| [matrix/](matrix/) | 16 category files. Each has a per-language capability matrix, a ranked **Parity gaps** list (with S/M/L effort), doc corrections and the fixtures run |

## Categories (`matrix/`)

1. [Detection & parsing](matrix/01-detection-parsing.md)
2. [Callables extracted](matrix/02-callables-extracted.md)
3. [Call-site forms](matrix/03-call-site-forms.md)
4. [Imports & module resolution](matrix/04-imports-module-resolution.md)
5. [Type inference & receiver typing](matrix/05-type-inference-receiver-typing.md)
6. [Inheritance, dispatch & overloading](matrix/06-inheritance-dispatch-overloading.md)
7. [Scoping & shadowing](matrix/07-scoping-shadowing.md)
8. [Macros, preprocessor & codegen](matrix/08-macros-preprocessor-codegen.md)
9. [FFI & cross-language links](matrix/09-ffi-cross-language-links.md)
10. [Frameworks & entry points](matrix/10-frameworks-entry-points.md)
11. [Test harness entries](matrix/11-test-harness-entries.md)
12. [Dead-code analysis](matrix/12-dead-code-analysis.md)
13. [Builtins & stdlib tables](matrix/13-builtins-stdlib-tables.md)
14. [Confidence & certainty](matrix/14-confidence-certainty.md)
15. [Dynamic use & reflection](matrix/15-dynamic-use-reflection.md)
16. [Known gaps & limitations](matrix/16-known-gaps-limitations.md)

## Legend (shared by every matrix)

✅ full · 🟡 partial · ⬜ none (the concept exists and cgg does nothing with it) · — n/a · ❔/? unverified

## Keeping it current

These docs are a snapshot. When a fix closes a gap:

- update the cell and the gap entry in the category file;
- update the language catalogue section;
- regenerate the overview row in `PARITY_MATRIX.md` using the scoring rule described there.

Nothing in `scripts/docs-check.py` reads this folder, so nothing will flag it when it goes stale.
