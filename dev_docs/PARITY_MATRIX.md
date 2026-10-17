# cgg language parity matrix

This document combines the sixteen per-category parity matrices into one view. It answers three questions:

1. How evenly does cgg support each of its 46 languages across all sixteen feature categories?
2. Which parity gaps are worth closing first?
3. What did the catalogue not cover?

It was compiled on 2026-10-17 from the `remediate/cross-language` tree (0.11.0 plus later commits). Every rating and gap below comes from a category file under [`matrix/`](matrix/). Each of those files cites the code (`file:line`) and the fixtures behind every cell. Read the category file before acting on a gap: this page is a summary and does not replace it.

## 1. Purpose and how the docs are organised

`dev_docs/` holds three layers. Each layer is built from the one before it.

| Layer | Files | What it holds |
| --- | --- | --- |
| Language catalogues | [`languages/*.md`](languages/) (46 files, one per registered plugin) | For each language, sections 01–16: what the plugin extracts and how the resolver treats it, with code citations and fixture results, then a summary scorecard. |
| Framework catalogue | [`frameworks/rules-part-1.md`](frameworks/rules-part-1.md) · [`part-2`](frameworks/rules-part-2.md) · [`part-3`](frameworks/rules-part-3.md) · [`part-4`](frameworks/rules-part-4.md) | Every `RuleSpec` in `crates/cgg-core/src/frameworks/rules.rs` (475 specs), with its triggers, its matching semantics and whether it enumerates entries or only detects the framework. |
| Category matrices | [`matrix/01…16-*.md`](matrix/) | One file per feature category. Each has a per-language × per-capability matrix, a ranked **Parity gaps** list, doc corrections and the fixtures run. |

The sixteen categories, and the column labels used in the overview below:

| # | Category | File |
| --- | --- | --- |
| 01 | Detection & parsing | [matrix/01-detection-parsing.md](matrix/01-detection-parsing.md) |
| 02 | Callables extracted | [matrix/02-callables-extracted.md](matrix/02-callables-extracted.md) |
| 03 | Call-site forms | [matrix/03-call-site-forms.md](matrix/03-call-site-forms.md) |
| 04 | Imports & module resolution | [matrix/04-imports-module-resolution.md](matrix/04-imports-module-resolution.md) |
| 05 | Type inference & receiver typing | [matrix/05-type-inference-receiver-typing.md](matrix/05-type-inference-receiver-typing.md) |
| 06 | Inheritance, dispatch & overloading | [matrix/06-inheritance-dispatch-overloading.md](matrix/06-inheritance-dispatch-overloading.md) |
| 07 | Scoping & shadowing | [matrix/07-scoping-shadowing.md](matrix/07-scoping-shadowing.md) |
| 08 | Macros, preprocessor & codegen | [matrix/08-macros-preprocessor-codegen.md](matrix/08-macros-preprocessor-codegen.md) |
| 09 | FFI & cross-language links | [matrix/09-ffi-cross-language-links.md](matrix/09-ffi-cross-language-links.md) |
| 10 | Frameworks & entry points | [matrix/10-frameworks-entry-points.md](matrix/10-frameworks-entry-points.md) |
| 11 | Test harness entries | [matrix/11-test-harness-entries.md](matrix/11-test-harness-entries.md) |
| 12 | Dead-code analysis | [matrix/12-dead-code-analysis.md](matrix/12-dead-code-analysis.md) |
| 13 | Builtins & stdlib tables | [matrix/13-builtins-stdlib-tables.md](matrix/13-builtins-stdlib-tables.md) |
| 14 | Confidence & certainty | [matrix/14-confidence-certainty.md](matrix/14-confidence-certainty.md) |
| 15 | Dynamic use & reflection | [matrix/15-dynamic-use-reflection.md](matrix/15-dynamic-use-reflection.md) |
| 16 | Known gaps & limitations | [matrix/16-known-gaps-limitations.md](matrix/16-known-gaps-limitations.md) |

### Legend

All sixteen category files share the same cell marks:

| Mark | Meaning |
| --- | --- |
| ✅ | full: the capability exists and covers the ecosystem's common cases |
| 🟡 | partial: it exists, but misses a common case, loses data or has a verified wrong-result case (often a false `high`) |
| ⬜ | none: the language has the concept and cgg does nothing with it |
| — | n/a: the concept does not exist in the language, or does not matter for it |
| ❔ / ? | unverified: the language doc is silent and no fixture was run (categories 03, 05, 06, 14 and 16 only) |

**How a category-level rating is derived.** None of the category files has a single summary column. Each has between 9 and 24 capability columns. The overview cell is therefore computed from that language's row in the category file:

- **score** = (✅ + ½ × 🟡) ÷ (✅ + 🟡 + ⬜). n/a and unverified cells are left out. The cell shows the score as a number from 0 to 100.
- **mark**: ✅ if the score is 85 or more, 🟡 from 35 to 84, ⬜ below 35, and — if every cell in the row is n/a.
- Cells that carry no mark are ignored, for example category 10's `Rules 19/30` count or category 12's `A·high` class.
- In category 16, ✅ means "no known gap of this class", so the scale runs the same way as in the other categories.

The score counts columns and does not weight them, so read it as a guide to parity rather than as a quality metric. A language with few applicable columns can score well on very little. Category 14 is the clearest example: it has ten columns, and many of them are n/a for the scripting languages. When a cell matters, open the category file.

## 2. Overview matrix

One row per registered language, in the order the category files use. The column numbers are the category numbers above. The **mean** column is the unweighted mean of that row's non-n/a scores. The last row is each category's mean across languages.

| language | 01 | 02 | 03 | 04 | 05 | 06 | 07 | 08 | 09 | 10 | 11 | 12 | 13 | 14 | 15 | 16 | mean |
| --- | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: |
| [rust](languages/rust.md) | 🟡 62 | 🟡 75 | 🟡 73 | 🟡 83 | 🟡 64 | ✅ 94 | 🟡 50 | 🟡 62 | 🟡 50 | 🟡 62 | 🟡 79 | ✅ 91 | 🟡 73 | ✅ 94 | 🟡 75 | 🟡 79 | 🟡 73 |
| [python](languages/python.md) | 🟡 67 | 🟡 72 | 🟡 82 | 🟡 83 | 🟡 39 | 🟡 69 | 🟡 44 | ⬜ 0 | 🟡 50 | 🟡 81 | 🟡 73 | ✅ 88 | ✅ 95 | 🟡 75 | ⬜ 31 | 🟡 72 | 🟡 64 |
| [kivy](languages/kivy.md) | ✅ 88 | 🟡 62 | 🟡 50 | ⬜ 5 | 🟡 50 | ⬜ 0 | — | ⬜ 0 | ✅ 100 | 🟡 50 | — | 🟡 57 | ⬜ 20 | 🟡 50 | ⬜ 12 | 🟡 42 | 🟡 42 |
| [javascript](languages/javascript.md) | 🟡 67 | 🟡 65 | 🟡 69 | 🟡 83 | ⬜ 17 | ⬜ 33 | 🟡 57 | ⬜ 0 | 🟡 40 | 🟡 44 | ⬜ 28 | 🟡 53 | ✅ 94 | 🟡 69 | ⬜ 21 | 🟡 58 | 🟡 50 |
| [typescript](languages/typescript.md) | 🟡 65 | 🟡 66 | 🟡 71 | 🟡 83 | ⬜ 27 | 🟡 42 | 🟡 50 | ⬜ 0 | 🟡 40 | 🟡 56 | ⬜ 28 | 🟡 53 | ✅ 90 | 🟡 50 | ⬜ 21 | 🟡 50 | 🟡 50 |
| [go](languages/go.md) | 🟡 70 | 🟡 73 | 🟡 72 | ✅ 95 | ⬜ 31 | ⬜ 17 | ⬜ 31 | ⬜ 0 | ⬜ 19 | 🟡 73 | ✅ 89 | 🟡 50 | ✅ 86 | 🟡 56 | ⬜ 25 | 🟡 50 | 🟡 52 |
| [java](languages/java.md) | 🟡 65 | 🟡 67 | 🟡 82 | 🟡 68 | 🟡 42 | ✅ 89 | ⬜ 0 | ⬜ 0 | 🟡 36 | 🟡 61 | 🟡 77 | 🟡 81 | 🟡 62 | ✅ 85 | ⬜ 25 | 🟡 81 | 🟡 58 |
| [kotlin](languages/kotlin.md) | 🟡 59 | 🟡 56 | 🟡 58 | 🟡 62 | 🟡 47 | 🟡 57 | 🟡 64 | ⬜ 0 | ⬜ 29 | 🟡 39 | 🟡 46 | 🟡 41 | 🟡 61 | 🟡 69 | ⬜ 0 | 🟡 55 | 🟡 46 |
| [c](languages/c.md) | 🟡 75 | 🟡 64 | 🟡 57 | 🟡 62 | ⬜ 11 | ⬜ 0 | ⬜ 8 | 🟡 67 | 🟡 44 | ⬜ 23 | ⬜ 31 | 🟡 38 | ✅ 100 | 🟡 58 | ⬜ 25 | 🟡 41 | 🟡 44 |
| [cpp](languages/cpp.md) | 🟡 70 | 🟡 60 | 🟡 68 | 🟡 50 | 🟡 46 | 🟡 61 | ⬜ 6 | 🟡 69 | 🟡 45 | ⬜ 25 | ⬜ 25 | 🟡 41 | 🟡 59 | 🟡 72 | ⬜ 17 | 🟡 41 | 🟡 47 |
| [csharp](languages/csharp.md) | 🟡 67 | 🟡 47 | 🟡 59 | 🟡 71 | 🟡 43 | 🟡 77 | ⬜ 7 | ⬜ 33 | 🟡 57 | 🟡 66 | 🟡 68 | 🟡 74 | 🟡 67 | 🟡 70 | 🟡 36 | 🟡 61 | 🟡 56 |
| [bash](languages/bash.md) | 🟡 45 | 🟡 50 | ⬜ 33 | ⬜ 20 | — | — | ⬜ 0 | ⬜ 0 | ⬜ 0 | ⬜ 8 | ⬜ 19 | ⬜ 30 | ✅ 100 | 🟡 50 | ⬜ 0 | ⬜ 17 | ⬜ 27 |
| [ruby](languages/ruby.md) | 🟡 59 | 🟡 50 | 🟡 56 | ⬜ 22 | ⬜ 14 | 🟡 50 | ⬜ 33 | ⬜ 0 | ⬜ 25 | 🟡 71 | 🟡 73 | ⬜ 33 | ✅ 92 | 🟡 56 | ⬜ 14 | ⬜ 32 | 🟡 42 |
| [swift](languages/swift.md) | 🟡 46 | 🟡 47 | 🟡 42 | ⬜ 15 | ⬜ 25 | 🟡 47 | 🟡 64 | ⬜ 12 | ⬜ 25 | ⬜ 24 | 🟡 46 | ⬜ 21 | 🟡 56 | 🟡 50 | ⬜ 0 | ⬜ 30 | ⬜ 34 |
| [lua](languages/lua.md) | 🟡 55 | 🟡 55 | 🟡 44 | 🟡 62 | ⬜ 0 | ⬜ 0 | 🟡 71 | ⬜ 0 | ⬜ 30 | ⬜ 12 | ⬜ 17 | ⬜ 18 | 🟡 70 | 🟡 58 | ⬜ 0 | ⬜ 32 | ⬜ 33 |
| [php](languages/php.md) | 🟡 58 | 🟡 37 | 🟡 57 | 🟡 64 | ⬜ 32 | 🟡 41 | 🟡 75 | ⬜ 0 | ⬜ 30 | 🟡 56 | 🟡 58 | 🟡 41 | 🟡 68 | 🟡 62 | ⬜ 12 | 🟡 47 | 🟡 46 |
| [dart](languages/dart.md) | 🟡 64 | 🟡 41 | 🟡 53 | ✅ 96 | ⬜ 27 | 🟡 62 | 🟡 57 | ⬜ 25 | 🟡 38 | ⬜ 8 | ⬜ 32 | ⬜ 19 | 🟡 70 | 🟡 81 | ⬜ 0 | 🟡 39 | 🟡 44 |
| [scala](languages/scala.md) | 🟡 54 | 🟡 50 | 🟡 66 | 🟡 54 | 🟡 37 | 🟡 59 | 🟡 56 | ⬜ 8 | 🟡 40 | ⬜ 19 | ⬜ 31 | ⬜ 24 | 🟡 78 | 🟡 56 | ⬜ 0 | 🟡 39 | 🟡 42 |
| [hcl](languages/hcl.md) | 🟡 44 | ⬜ 25 | 🟡 62 | ⬜ 0 | — | — | ⬜ 0 | ⬜ 25 | ⬜ 0 | ⬜ 0 | ⬜ 17 | ⬜ 20 | 🟡 83 | ⬜ 30 | ⬜ 0 | ⬜ 9 | ⬜ 22 |
| [zig](languages/zig.md) | 🟡 50 | 🟡 39 | 🟡 62 | 🟡 82 | 🟡 69 | ⬜ 0 | 🟡 50 | ⬜ 25 | ⬜ 20 | ⬜ 17 | ⬜ 29 | ⬜ 19 | ✅ 86 | ✅ 88 | ⬜ 0 | 🟡 44 | 🟡 42 |
| [objc](languages/objc.md) | ⬜ 32 | 🟡 42 | ⬜ 25 | 🟡 44 | ⬜ 0 | ⬜ 5 | ⬜ 0 | ⬜ 4 | 🟡 36 | ⬜ 27 | 🟡 36 | ⬜ 23 | 🟡 64 | 🟡 43 | ⬜ 0 | ⬜ 17 | ⬜ 25 |
| [r](languages/r.md) | 🟡 46 | 🟡 36 | ⬜ 33 | ⬜ 12 | ⬜ 17 | ⬜ 0 | ⬜ 0 | ⬜ 25 | ⬜ 0 | ⬜ 28 | ⬜ 22 | ⬜ 25 | 🟡 58 | 🟡 50 | ⬜ 0 | ⬜ 11 | ⬜ 23 |
| [groovy](languages/groovy.md) | 🟡 40 | 🟡 53 | 🟡 37 | ⬜ 25 | ⬜ 12 | 🟡 38 | ⬜ 0 | ⬜ 0 | 🟡 38 | ⬜ 27 | ⬜ 23 | ⬜ 31 | 🟡 61 | 🟡 56 | ⬜ 0 | ⬜ 21 | ⬜ 29 |
| [julia](languages/julia.md) | 🟡 64 | 🟡 39 | ⬜ 27 | ⬜ 12 | ⬜ 0 | ⬜ 14 | ⬜ 7 | ⬜ 12 | ⬜ 25 | ⬜ 22 | ⬜ 25 | ⬜ 30 | 🟡 67 | 🟡 42 | ⬜ 0 | ⬜ 14 | ⬜ 25 |
| [perl](languages/perl.md) | 🟡 40 | 🟡 38 | 🟡 68 | ⬜ 27 | ⬜ 25 | 🟡 50 | ⬜ 10 | ⬜ 0 | 🟡 38 | ⬜ 27 | ⬜ 12 | ⬜ 23 | 🟡 57 | 🟡 71 | ⬜ 0 | ⬜ 14 | ⬜ 31 |
| [elixir](languages/elixir.md) | 🟡 65 | 🟡 46 | 🟡 68 | 🟡 62 | ⬜ 33 | ⬜ 29 | ⬜ 17 | 🟡 42 | 🟡 40 | 🟡 54 | ⬜ 28 | ⬜ 26 | 🟡 75 | 🟡 57 | ⬜ 0 | 🟡 45 | 🟡 43 |
| [erlang](languages/erlang.md) | 🟡 46 | 🟡 36 | 🟡 50 | 🟡 55 | ✅ 100 | 🟡 38 | ⬜ 25 | ⬜ 19 | ⬜ 20 | 🟡 40 | 🟡 67 | ⬜ 27 | ✅ 100 | 🟡 64 | ⬜ 0 | ⬜ 32 | 🟡 45 |
| [fortran](languages/fortran.md) | 🟡 39 | ⬜ 32 | ⬜ 8 | ⬜ 23 | ⬜ 0 | ⬜ 0 | ⬜ 0 | ⬜ 4 | ⬜ 10 | ⬜ 20 | ⬜ 19 | ⬜ 8 | 🟡 62 | 🟡 50 | ⬜ 0 | ⬜ 7 | ⬜ 18 |
| [clojure](languages/clojure.md) | 🟡 59 | 🟡 36 | 🟡 39 | 🟡 36 | ⬜ 0 | ⬜ 30 | ⬜ 0 | 🟡 42 | ⬜ 25 | ⬜ 27 | ⬜ 25 | ⬜ 15 | 🟡 50 | 🟡 50 | ⬜ 0 | ⬜ 24 | ⬜ 29 |
| [haskell](languages/haskell.md) | 🟡 38 | 🟡 37 | ⬜ 8 | ⬜ 25 | — | ⬜ 12 | ⬜ 10 | ⬜ 8 | ⬜ 25 | ⬜ 20 | 🟡 56 | ⬜ 12 | 🟡 70 | 🟡 57 | ⬜ 0 | ⬜ 18 | ⬜ 26 |
| [ocaml](languages/ocaml.md) | 🟡 38 | ⬜ 23 | ⬜ 12 | ⬜ 23 | ⬜ 0 | ⬜ 0 | 🟡 83 | ⬜ 0 | ⬜ 12 | ⬜ 10 | ⬜ 22 | ⬜ 30 | 🟡 75 | 🟡 58 | ⬜ 0 | ⬜ 13 | ⬜ 25 |
| [powershell](languages/powershell.md) | 🟡 50 | 🟡 50 | 🟡 35 | ⬜ 14 | ⬜ 11 | ⬜ 0 | ⬜ 0 | ⬜ 0 | ⬜ 0 | ⬜ 32 | ⬜ 25 | ⬜ 20 | ⬜ 0 | 🟡 50 | ⬜ 0 | ⬜ 8 | ⬜ 18 |
| [solidity](languages/solidity.md) | 🟡 61 | 🟡 68 | ⬜ 32 | ⬜ 15 | ⬜ 11 | ⬜ 25 | ⬜ 0 | ⬜ 0 | ⬜ 0 | ⬜ 20 | ⬜ 17 | ⬜ 29 | ⬜ 0 | 🟡 44 | ⬜ 0 | ⬜ 25 | ⬜ 22 |
| [fsharp](languages/fsharp.md) | 🟡 46 | ⬜ 32 | ⬜ 31 | ⬜ 14 | ⬜ 4 | ⬜ 14 | ⬜ 25 | ⬜ 25 | ⬜ 25 | ⬜ 16 | 🟡 46 | ⬜ 17 | ⬜ 0 | 🟡 44 | ⬜ 0 | ⬜ 12 | ⬜ 22 |
| [starlark](languages/starlark.md) | 🟡 56 | ⬜ 28 | 🟡 43 | ⬜ 22 | — | — | ⬜ 0 | 🟡 50 | ⬜ 0 | ⬜ 33 | ⬜ 25 | ⬜ 18 | ⬜ 0 | 🟡 42 | ⬜ 0 | ⬜ 4 | ⬜ 23 |
| [cmake](languages/cmake.md) | 🟡 56 | 🟡 40 | ⬜ 17 | ⬜ 25 | — | ⬜ 0 | ⬜ 0 | 🟡 36 | ⬜ 0 | ⬜ 33 | ⬜ 25 | ⬜ 31 | ⬜ 0 | 🟡 50 | ⬜ 0 | ⬜ 0 | ⬜ 21 |
| [nix](languages/nix.md) | 🟡 71 | 🟡 43 | 🟡 50 | ⬜ 5 | — | ⬜ 0 | ⬜ 0 | ⬜ 0 | ⬜ 0 | 🟡 50 | ⬜ 21 | 🟡 43 | ⬜ 0 | 🟡 50 | ⬜ 0 | ⬜ 11 | ⬜ 23 |
| [verilog](languages/verilog.md) | 🟡 56 | ⬜ 25 | ⬜ 8 | ⬜ 0 | ⬜ 0 | ⬜ 0 | ⬜ 25 | ⬜ 8 | ⬜ 0 | ⬜ 0 | ⬜ 21 | ⬜ 6 | ⬜ 0 | 🟡 80 | ⬜ 0 | ⬜ 14 | ⬜ 15 |
| [vhdl](languages/vhdl.md) | 🟡 71 | 🟡 35 | ⬜ 0 | ⬜ 20 | — | ⬜ 0 | ⬜ 25 | ⬜ 0 | ⬜ 0 | ⬜ 33 | ⬜ 21 | ⬜ 7 | ⬜ 0 | 🟡 80 | ⬜ 0 | ⬜ 21 | ⬜ 21 |
| [asm](languages/asm.md) | 🟡 50 | ⬜ 30 | ⬜ 12 | ⬜ 0 | — | — | ⬜ 0 | ⬜ 4 | 🟡 50 | ⬜ 0 | ✅ 100 | ⬜ 32 | ⬜ 0 | 🟡 80 | ⬜ 0 | ⬜ 25 | ⬜ 27 |
| [smithy](languages/smithy.md) | 🟡 62 | ⬜ 33 | 🟡 75 | ⬜ 6 | — | 🟡 50 | — | — | 🟡 50 | ⬜ 0 | ⬜ 33 | ⬜ 0 | 🟡 83 | 🟡 80 | — | ⬜ 31 | 🟡 42 |
| [proto](languages/proto.md) | 🟡 64 | 🟡 67 | 🟡 75 | ⬜ 12 | — | ⬜ 0 | ⬜ 0 | 🟡 50 | 🟡 50 | ⬜ 0 | — | ⬜ 0 | ⬜ 0 | 🟡 80 | — | 🟡 36 | ⬜ 33 |
| [graphql](languages/graphql.md) | 🟡 44 | 🟡 38 | ✅ 100 | ⬜ 17 | — | ⬜ 25 | — | ⬜ 0 | ⬜ 0 | ⬜ 0 | — | ⬜ 0 | 🟡 50 | 🟡 80 | — | ⬜ 21 | ⬜ 31 |
| [openapi](languages/openapi.md) | 🟡 62 | 🟡 40 | 🟡 75 | ⬜ 6 | — | 🟡 50 | — | ⬜ 0 | ⬜ 0 | ⬜ 0 | — | ✅ 100 | — | 🟡 70 | ⬜ 0 | ⬜ 33 | 🟡 36 |
| [asyncapi](languages/asyncapi.md) | 🟡 62 | 🟡 40 | 🟡 50 | ⬜ 6 | — | ⬜ 25 | — | ⬜ 0 | ⬜ 0 | ⬜ 0 | — | ⬜ 0 | — | 🟡 70 | — | ⬜ 17 | ⬜ 25 |
| [lean](languages/lean.md) | 🟡 50 | 🟡 53 | 🟡 45 | ⬜ 25 | ⬜ 5 | ⬜ 10 | 🟡 50 | ⬜ 0 | ⬜ 0 | ⬜ 0 | ⬜ 15 | ⬜ 25 | ⬜ 12 | 🟡 57 | ⬜ 0 | ⬜ 22 | ⬜ 23 |
| **all languages** | **56** | **47** | **49** | **37** | **25** | **29** | **24** | **15** | **25** | **29** | **38** | **32** | **54** | **62** | **7** | **31** | |

**What the matrix says.**

- **Nobody is at parity.** Only 25 of the 736 language × category cells are ✅. 314 are 🟡, 363 are ⬜ and 34 are n/a. Rust (mean 73) and Python (64) are the only languages above 60. Java (58), C# (56), Go (52), JS and TS (50 each) follow. The 26 languages with a mean below 35 are mostly ⬜: in 25 of them, ⬜ is the most common mark.
- **The weakest categories are cross-cutting.** These are 15 dynamic use (mean 7: the reflection screen covers 8 languages), 08 macros (15), 07 scoping (24: only 11 languages have a locals spec), 05 typing (25) and 09 FFI (25). These are also where most false `high` edges come from (gaps 1–3 and 10 below).
- **Imports (04) are strong for the mainstream languages and near zero for the tail.** Go and Dart are ✅, and Rust, Python, JS and TS score 83. HCL, Verilog, asm and the descriptor languages score near zero.
- **Bands.** The hardware, build and config languages (Verilog, VHDL, CMake, HCL, Starlark, Nix, PowerShell, Fortran) all have a mean of 23 or less. They are mostly ⬜ outside detection and confidence, which is consistent with their low corpus share. The parity work that pays off most is between peers: Kotlin against Java, C# against Java, Swift against Kotlin, and JS/TS against Python. The top gaps below are chosen for that reason.

## 3. Top parity gaps

These are the 30 most valuable gaps across all 16 categories, deduplicated. Where several category files list the same root cause, it appears once, with every source given.

**Ranking.** (reach × cheapness): reach is the real-world user base of the affected languages (JS/TS, Python, Java, C#, Go, Kotlin, C/C++ rank highest). Cheapness comes from the effort the category file gives (S = hours to a day, M = a day to a few days, L = a week or more). Within a tier, gaps that produce **wrong `high` edges or false `high` dead-code findings** rank above gaps that only lose edges. The `high` band is the one cgg promises is safe.

The "Evidence" column gives the category file and its gap number, as `05 #1` = [matrix/05](matrix/05-type-inference-receiver-typing.md#parity-gaps), gap 1.

| Rank | Gap | Category | Languages | Evidence | Effort |
| --: | --- | --- | --- | --- | :-: |
| 1 | **Intra-file `high` ignores `weak_receiver_type`.** A receiver typed only by the variable-name heuristic (`service = mk(); service.run()`) binds a same-file `Service.run` at **high**, even when the variable holds an int or another class. `cross_file.rs:2724` honours the weak flag, but `intra_file.rs:629-640` emits `High` unconditionally. | 05, 14 | rust, python, javascript, typescript, go, java, kotlin, cpp, csharp, ruby, swift, lua, dart, scala, objc, groovy, solidity, fsharp, lean (19) | [05 #1](matrix/05-type-inference-receiver-typing.md#parity-gaps), [14 #1](matrix/14-confidence-certainty.md#parity-gaps) | S |
| 2 | **Lexical narrowing skips a site that has only one candidate.** A lone nested function is bound from *any* function in the file at **high**, because the narrowing block requires `candidates.len() > 1` (`intra_file.rs:367`). | 07 | the 13 `lexically_scoped` languages: JS/TS, Python, Rust, Kotlin, Swift, Scala, Dart, Haskell, Julia, Lua, OCaml, F# | [07 #1](matrix/07-scoping-shadowing.md#parity-gaps) | S |
| 3 | **Lambda and closure parameters are not treated as parameters.** `is_param = fs == ls` (`intra_file.rs:296`) is false for `(x) =>`, `lambda x:`, `\|x\|` and `fun f ->`, so a closure parameter named like a nested function does not shadow it. | 07 | python, javascript, typescript, rust, ocaml (and every locals language) | [07 #2](matrix/07-scoping-shadowing.md#parity-gaps) | S |
| 4 | **The return-type map is keyed by bare name, across all languages, and the first definition wins.** `build_return_type_map` (`type_hints.rs:272-287`) has no language or file in its key. Two `make()` functions give whichever was seen first, and the result types every Strategy 4 local. Found by code reading; no fixture was run. | 05 | python, typescript, kotlin, swift, scala, go, rust (all Strategy 4 users) | [05 #16](matrix/05-type-inference-receiver-typing.md#parity-gaps) | S |
| 5 | **Override markers are not recorded, so `implements-external-contract` never caps.** An override of an Android, Flutter, UIKit or React base is reported like ordinary dead code. Java, C# and Groovy work. PHP's `#[\Override]` is recorded but never matches. | 06, 12, 16 | kotlin, swift, scala, typescript, dart, cpp, php | [12 #3](matrix/12-dead-code-analysis.md#parity-gaps), [06 #8](matrix/06-inheritance-dispatch-overloading.md#parity-gaps), [16 #9](matrix/16-known-gaps-limitations.md#parity-gaps) | S |
| 6 | **Python `async def` methods skip the external-contract cap**, because `deadcode/mod.rs:505` checks `kind != Function`, and async methods are kind `function`. Result: a false **high** dead finding (fixture f1: `_ahook` high, `_shook` medium). | 12, 02 | python | [12 #7](matrix/12-dead-code-analysis.md#parity-gaps) | S |
| 7 | **Interface and abstract members are not tagged `abstract`**, so duck fan-out keeps the bodiless declaration next to the implementations. TS, Swift, Scala, Dart and Python tag them. Bodiless detection already exists in most of these plugins. | 02, 06 | java, csharp, kotlin, go, php, groovy, cpp, objc, solidity, fortran, elixir, rust (trait decls) | [02 #3](matrix/02-callables-extracted.md#parity-gaps), [06 #7](matrix/06-inheritance-dispatch-overloading.md#parity-gaps) | S |
| 8 | **Class-field arrow functions are not callables in JS/TS.** `handler = () => { this.go() }` (the dominant React and Angular handler idiom) gets no node, and its calls are `no-enclosing-callable`. `try_record_named_fn` already handles `const f = () =>`. | 02 | javascript, typescript | [02 #1](matrix/02-callables-extracted.md#parity-gaps) | S–M |
| 9 | **Include-style imports bind an arbitrary same-named copy at `high`.** In fixture `wc`, the path-sorted last copy won in every language tested, regardless of the import. C-family `c_include_copies` already scopes this correctly. | 04, 14 | bash, ruby, r, powershell, solidity, starlark, cmake (perl, lua, nix likely) | [04 #1](matrix/04-imports-module-resolution.md#parity-gaps), [14 #2](matrix/14-confidence-certainty.md#parity-gaps) | S–M |
| 10 | **Go method receivers and same-package parameters are untyped.** In `func (s *Server) Zap() { s.zip() }`, `s` has no type, because the type-first parser reads `q *Server` back to front. Package-qualified parameters are typed. | 05 | go | [05 #3](matrix/05-type-inference-receiver-typing.md#parity-gaps) | S |
| 11 | **Unreachable regions are computed and never consumed.** Eight plugins fill `FileFacts::unreachable`, and nothing reads it. A call after `return`/`throw` keeps its target live, and in TS the edge stays `high`. | 12, 16 | rust, python, go, java, javascript, typescript, c, cpp | [12 #2](matrix/12-dead-code-analysis.md#parity-gaps), [16 #22](matrix/16-known-gaps-limitations.md#parity-gaps) | S |
| 12 | **Constructions do not reach constructors.** `new T()` (C#), `Engine()` (Swift) and `Foo.new` (Ruby) give no edge to `T.T`, `init` or `initialize`. Python, JS/TS, Kotlin, C++ and Java do. The fix is to emit the construction as a call and add a `constructor_names` row. | 03, 16 | csharp, swift, ruby (S); php, scala, groovy, solidity, powershell, objc, fsharp (M) | [03 #3](matrix/03-call-site-forms.md#parity-gaps), [16 #2](matrix/16-known-gaps-limitations.md#parity-gaps) | S–M |
| 13 | **Constructor delegation and `super(…)` record no site.** This covers Java `this(…)`/`super(…)`, JS/TS `super()`, C# `: base()`/`: this()`, C++ base initializers, Solidity base constructors and Ruby bare `super`. Python `super().__init__()` and Kotlin work. | 03, 06 | java, javascript, typescript, csharp, cpp, solidity, ruby | [03 #4](matrix/03-call-site-forms.md#parity-gaps), [06 #11](matrix/06-inheritance-dispatch-overloading.md#parity-gaps) | S |
| 14 | **Kotlin lacks 23 enumerating Java rules, and its Android lifecycle rules have no matchers.** Examples are `@Scheduled`, `@KafkaListener`, `@Bean`, `@PostConstruct`, JAX-RS and Micronaut, plus `junit-sources` `@MethodSource`. The plugin already records the attributes and bases these rules need. The android gap text is stale. | 10, 11, 15 | kotlin | [10 #1](matrix/10-frameworks-entry-points.md#parity-gaps), [11 #4](matrix/11-test-harness-entries.md#parity-gaps), [15 #1](matrix/15-dynamic-use-reflection.md#parity-gaps) | S |
| 15 | **Rust `#[cfg(test)]` helpers are reported as high production dead code.** `rust_test_role` returns `None` for them (`rust.rs:3115`), although the `testfile.rs:69-71` comment says they are handled. | 11, 12, 16 | rust | [11 #5](matrix/11-test-harness-entries.md#parity-gaps), [16 #23](matrix/16-known-gaps-limitations.md#parity-gaps) | S–M |
| 16 | **Typed cross-file overload lookup keeps one arbitrary overload.** Every arity binds the last-declared overload, which is a wrong edge. The set expansion (`cross_file.rs:2619-2623`) requires `OVERLOAD_ARITY_LANGS`. | 06 | swift, groovy | [06 #2](matrix/06-inheritance-dispatch-overloading.md#parity-gaps) | S |
| 17 | **`::`, `/` and `\` qualified heads are never split for the stdlib screen.** `std::fs::read_to_string`, `std::this_thread::sleep_for`, `POSIX::floor`, `stats::median`, `clojure.string/join` and `\strlen` are all filed as `<external>`. `module_is_stdlib` splits on `.` only. | 13 | rust, cpp, php, perl, r, clojure | [13 #1](matrix/13-builtins-stdlib-tables.md#parity-gaps) | S |
| 18 | **Common Java and C# reflective calls are missing from the screen**, so their targets stay `high`. Java: `getDeclaredField`, `getConstructor`, `MethodHandles…findVirtual`. C#: `InvokeMember`, `GetField`, `Delegate.CreateDelegate`. | 15 | java, csharp | [15 #4](matrix/15-dynamic-use-reflection.md#parity-gaps) | S |
| 19 | **Bash steals other languages' shebangs.** The `sh` needle is a substring match over the whole line, so `pwsh`, `powershell`, perl, lua, scala-cli, fish and nix-shell scripts are all routed to Bash. `ts-node` routes to JS. | 01 | powershell, perl, lua, scala, bash, typescript | [01 #1](matrix/01-detection-parsing.md#parity-gaps), [16 #21](matrix/16-known-gaps-limitations.md#parity-gaps) | S |
| 20 | **No filename-based detection.** `CMakeLists.txt`, `BUILD`/`WORKSPACE`/`BUCK`, `Gemfile`/`Rakefile`/`config.ru` and `Jenkinsfile` are never analysed. For CMake and Bazel that is most of their code. | 01 | cmake, starlark, ruby, groovy, bash | [01 #2](matrix/01-detection-parsing.md#parity-gaps) | S |
| 21 | **C++ gtest `TEST`/`TEST_F` bodies are extracted and then reported dead.** The `gtest` rule is gap-only. A rule on those callable names in gtest-including files, plus `SetUp`/`TearDown` on `::testing::Test` subclasses, would close it. libFuzzer `LLVMFuzzerTestOneInput` is a fixed name that is rooted nowhere. | 11 | cpp | [11 #2](matrix/11-test-harness-entries.md#parity-gaps) | S–M |
| 22 | **Registrar capture is not wired, or only partly wired.** C# `IDENT_KINDS` lacks `member_access_expression`, so `app.MapGet("/a", Handlers.List)` mints no entry. swift.rs ignores `ExtractCtx`, so the Swift `aws-lambda` verb `run` can never match. | 03, 10 | csharp, swift | [03 #8](matrix/03-call-site-forms.md#parity-gaps), [10 #7](matrix/10-frameworks-entry-points.md#parity-gaps) | S |
| 23 | **Test-file name rules and test-name roots are uneven.** Thirteen languages have no test-file arm: Perl `t/*.t`, Groovy `*Spec`, Lua `_spec`, PowerShell `*.Tests.ps1`, Solidity `.t.sol`, and others. JS/TS miss `.test.mts`/`.spec.cjs`. The generic `is_test_name` arm roots PowerShell `Test-*` production functions, and the Go arm roots `Testing`. | 01, 11 | perl, groovy, lua, powershell, solidity, objc, haskell, r, clojure, hcl, bash, zig, julia, javascript, typescript, go, csharp | [11 #8](matrix/11-test-harness-entries.md#parity-gaps), [11 #9](matrix/11-test-harness-entries.md#parity-gaps), [01 #8](matrix/01-detection-parsing.md#parity-gaps) | S |
| 24 | **Objective-C records no plain C function calls.** objc.rs has no `call_expression` arm, so `NSLog`, `dispatch_async` and every C helper in a `.m` file are invisible, and C functions defined there read as dead. This also blocks the ObjC macro and FFI parity listed in 08 #1 and 09 #16. | 03, 08, 09 | objc | [03 #2](matrix/03-call-site-forms.md#parity-gaps), [08 #1](matrix/08-macros-preprocessor-codegen.md#parity-gaps), [09 #16](matrix/09-ffi-cross-language-links.md#parity-gaps) | S |
| 25 | **Functions passed as values outside registrar verbs are not references.** Examples are `[1].map(mapper)`, `setTimeout(cb)`, `sortSlice(nil, less)` and `map(::topHelper)`. This is the largest dead-code false-positive class in the corpus. Python, Java, C#, Rust and Elixir capture them, and the shared helpers exist (`registrar.rs:517, 563`). | 03, 16 | javascript, typescript, go, kotlin, c, cpp, ruby, php, swift, dart, scala, lua, … (25 languages) | [03 #1](matrix/03-call-site-forms.md#parity-gaps), [16 #5](matrix/16-known-gaps-limitations.md#parity-gaps) | M per plugin |
| 26 | **Java, C#, Dart, Groovy, C, C++ and ObjC never read a declared return type**, so `var t = make(); t.work()` and `getTool().work()` fan out or stay unresolved. The comment at `type_hints.rs:2650` says Java/C# are handled, but no branch does it. | 05, 16 | java, csharp, dart, groovy, c, cpp, objc | [05 #2](matrix/05-type-inference-receiver-typing.md#parity-gaps), [16 #4](matrix/16-known-gaps-limitations.md#parity-gaps) | M |
| 27 | **There is no locals spec in Go, C#, C or C++, so parameters and locals never shadow.** `run(cb func()){cb()}` binds a same-named function at **high**, and so do C# delegate parameters and C/C++ function-pointer parameters. Rust, Kotlin and Zig have specs. | 07, 14, 16 | go, csharp, c, cpp (also groovy, r, julia, haskell, clojure, nix, starlark, solidity, fsharp, fortran, perl) | [07 #3, #4, #6](matrix/07-scoping-shadowing.md#parity-gaps), [14 #3](matrix/14-confidence-certainty.md#parity-gaps), [16 #1](matrix/16-known-gaps-limitations.md#parity-gaps) | M each |
| 28 | **JS/TS locals are not a typing source.** `const t = new Svc(); t.run()` and `const t: Svc = …` write no `local_types`, so the call fans out at medium, including to another file's `Third.run`. This is the dominant source of medium fan-out in TS. | 05, 14, 16 | javascript, typescript (also ruby, swift, dart, scala, php, lua, perl for constructed locals) | [05 #5, #6](matrix/05-type-inference-receiver-typing.md#parity-gaps), [14 #6](matrix/14-confidence-certainty.md#parity-gaps), [16 #4](matrix/16-known-gaps-limitations.md#parity-gaps) | M |
| 29 | **Overloading languages other than Java, Scala and C++ get no arity narrowing.** Every overload binds at medium. C# and Kotlin come first. The plugins must set `RefRecord::arity`, and `accepts_arity` must handle `params`/`vararg`/defaults/labels. Related: cross-file arity-narrowed Java and Scala overloads never reach `high` (14 #4, S). | 03, 06, 14, 16 | csharp, kotlin, swift, groovy, typescript, solidity, fsharp, julia, powershell | [06 #3](matrix/06-inheritance-dispatch-overloading.md#parity-gaps), [03 #6](matrix/03-call-site-forms.md#parity-gaps), [14 #4, #5](matrix/14-confidence-certainty.md#parity-gaps) | M |
| 30 | **JS/TS have no working test harness.** `jest`, `vitest` and `playwright` are gap-only, there is no mocha or `node:test` rule, and `it(…)` callbacks are not callables. Helpers used only by tests stay dead findings. Java, Python, Go, Rust and C# all mint harness nodes. | 10, 11, 16 | javascript, typescript | [11 #1](matrix/11-test-harness-entries.md#parity-gaps), [10 #2](matrix/10-frameworks-entry-points.md#parity-gaps), [16 #10](matrix/16-known-gaps-limitations.md#parity-gaps) | M–L |

**Just below the line.** These are cheap fixes with smaller reach. They are good candidates to batch with a fix above that touches the same file:

- Explicit-syntax visibility: Elixir `defp`, Zig `pub`, Erlang `-export`, Clojure `defn-`, Haskell export lists. [16 #8](matrix/16-known-gaps-limitations.md#parity-gaps). S each.
- Exports as dead-code roots, JS/TS `export` first. [12 #1](matrix/12-dead-code-analysis.md#parity-gaps). M.
- napi/UniFFI snake→camel and gRPC descriptor `_`/case normalisation. [09 #1, #2](matrix/09-ffi-cross-language-links.md#parity-gaps). S.
- wasm-bindgen and UniFFI phantom edges. [09 #6](matrix/09-ffi-cross-language-links.md#parity-gaps). S.
- Descriptor fan-out cap hard-coded at 4, ignoring `--fanout-cap 0`. [16 #14](matrix/16-known-gaps-limitations.md#parity-gaps), [04 #13](matrix/04-imports-module-resolution.md#parity-gaps). S.
- Blind-spot disclosure, which covers 5 of 46 languages. [12 #14](matrix/12-dead-code-analysis.md#parity-gaps), [15 #13](matrix/15-dynamic-use-reflection.md#parity-gaps). S.
- Missing stdlib manifests for PowerShell, Solidity, F#, Starlark, CMake, Nix, Lean, Kivy, Verilog and asm. Kivy and Starlark can reuse `python.txt`. [13 #2](matrix/13-builtins-stdlib-tables.md#parity-gaps). S.
- Type aliases followed only in C++, Rust and Zig. [05 #11](matrix/05-type-inference-receiver-typing.md#parity-gaps). S per plugin.
- Groovy `packages` and external-module refusal (it joins the JVM arm). [04 #2, #11](matrix/04-imports-module-resolution.md#parity-gaps). S.
- Build-output directories walked for Elixir, Erlang, Lean, SwiftPM, Haskell, OCaml, Zig, Perl and Dart, while `bin/` is denied. [01 #4](matrix/01-detection-parsing.md#parity-gaps). S.
- OCaml module missing from the QN, and C# file-scoped namespaces. [02 #4](matrix/02-callables-extracted.md#parity-gaps). S. This unblocks [04 #4](matrix/04-imports-module-resolution.md#parity-gaps) and [14 #10](matrix/14-confidence-certainty.md#parity-gaps).

**Larger structural items** are not ranked, because their effort is L. They are the JVM, .NET and JS/TS language families (`lang_family.rs:32-34`), DSL-style test blocks in eleven languages, top-level code as a callable, interface → implementation dispatch for Go, Haskell, Elixir and Clojure, markup code-behind beyond C#, and native FFI for the scripting languages. See [04 #5](matrix/04-imports-module-resolution.md#parity-gaps), [11 #7](matrix/11-test-harness-entries.md#parity-gaps), [02 #16](matrix/02-callables-extracted.md#parity-gaps), [06 #14](matrix/06-inheritance-dispatch-overloading.md#parity-gaps), [09 #15, #19](matrix/09-ffi-cross-language-links.md#parity-gaps).

## 4. Coverage notes

- **Language catalogues: complete.** There are 46 files under `languages/`, one for each `reg.register(…)` line in `crates/cgg-lang/src/plugins.rs` (46). Each has all sixteen sections, 01–16, plus a summary scorecard. Jupyter notebooks have no file of their own. They are covered under [python](languages/python.md) §01 and §15, because the notebook extractor feeds the Python plugin.
- **Category matrices: complete.** There are 16 files under `matrix/`, each with a 46-row matrix and a ranked parity-gaps list. No category is missing.
- **Framework catalogue: complete for `rules.rs`.** The four `frameworks/rules-part-*.md` files cover all 475 `RuleSpec` entries by line range. Frameworks that exist only as gap prose inside a rule's `gap` text are listed there, not as separate files.
- **Unverified cells.** Some cells are marked ❔/? ("doc silent, no fixture run"): 37 in category 16, 26 in category 14, 14 in category 03, 3 in category 06 and 2 in category 05. They are excluded from the scores above. Those scores can rise or fall once the cells are checked.
- **Gaps not backed by a fixture.** Gap 4 (the return-type map key) was found by code reading only, as category 05 notes. The external-module behaviour of Perl, Lua and Nix under gap 9 is marked "likely" in the source and was not run.
- **Doc corrections.** Every category file ends with a **Doc corrections** section, which lists places where `CLAUDE.md` or README prose disagrees with the code. One example: CLAUDE.md says a guessed receiver type "is never `high`", and gap 1 shows that this holds cross-file only. These corrections have not been applied. This page does not edit `CLAUDE.md`.
