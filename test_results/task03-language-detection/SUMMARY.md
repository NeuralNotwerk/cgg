# Task 3 — Language detection + parser pool

## What shipped

- **9 v1 language plugins** registered in `cgg-lang::plugins`:
  Rust, Python, JavaScript, TypeScript, Go, Java, C, C++, C#.
  Each declares its own set of extensions, shebangs, resolver kind,
  and tree-sitter grammar. (JavaScript and TypeScript use **distinct**
  grammars — `tree-sitter-javascript` and the TSX variant of
  `tree-sitter-typescript` — deliberately avoiding codescope's
  JS/TS conflation. C# uses `tree-sitter-c-sharp`, not the
  `.cs → cpp` hack.)
- **`LanguageDetector`** applying, in order:
  1. `#!` shebang matching against registered keywords.
  2. Extension match (case-sensitive, then case-insensitive fall-back).
  3. `.h` disambiguation — sibling-file heuristic promotes a header
     to C++ if a same-stem `.cpp`/`.cc`/`.cxx`/`.hpp`/`.hh`/`.hxx`
     exists in the same directory.
  4. Anything else → `DetectVerdict::Unknown` → audit
     `skip_reason: unknown-extension`.
  Every verdict carries a `detected_via` label (`"extension:.py"`,
  `"shebang:python3"`, `"header-heuristic:cpp"`).
- **`ParserPool`** with `thread_local!` storage keying parsers by
  plugin id; amortizes `tree_sitter::Parser` allocation across files
  on the same thread. `ParserPool::parse` returns the `Tree` plus
  wall-clock `parse_ms`.
- **`cgg` binary** now emits `AuditEvent::FileAnalyzed` per
  successfully-parsed file with `file`, `language`, `detected_via`,
  `sha256`, `size_bytes`, `lines`, `parse_ms`, and `parse_status`.
- Tree-sitter grammar pool built successfully at tree-sitter 0.22
  with all 9 grammar crates pinned at compatible versions (rust
  0.21, python 0.21, javascript 0.21, typescript 0.21, go 0.21,
  java 0.21, c 0.21, cpp 0.22, c-sharp 0.21).

## Artifacts

- `cargo-test.txt` — full workspace test run (44 tests passed).
- `lang-mix.jsonl` — audit stream from the 13-file fixture.
- `lang-mix.stderr.txt` — stderr summary from the same run.

## Fixture

13 files covering every v1 language plus:

| Path                   | Expected verdict                                    |
|------------------------|-----------------------------------------------------|
| `r.rs` `p.py` `j.js` `t.ts` `g.go` `J.java` `c1.c` `cpp1.cpp` `cs1.cs` | extension match, one per language |
| `includes/lib.h` + `includes/lib.cpp` | `.h` → cpp via header heuristic |
| `tool` (no ext, `#!/usr/bin/env python3`) | python via shebang |
| `notes.txt` | unknown-extension skip |

## Observed behavior

From `lang-mix.jsonl`:

- 13 `file_discovered` events.
- 12 `file_analyzed` events covering all 9 languages plus the shebang
  `tool` file (shebang:python3) and the header-heuristic `lib.h`
  (header-heuristic:cpp).
- 1 `file_skipped` with `{"kind":"unknown-extension"}` for `notes.txt`.
- Per-file `parse_ms` in the 0.08 ms – 0.40 ms range on the
  3-line fixtures.
- Every sha256 is the real blake3-over-bytes hex digest (shown
  instead of a sha256 — schema says `sha256` but the implementation
  uses blake3 for speed; Task 12 renames the field to match).

## Test counts

- `cgg-lang` unit tests: **12 passed** (detection 6, parser pool 3,
  registry 2, plugin-trait 1).
- `cgg` integration tests: `cli.rs` 5 + `walker.rs` 3 + `detect.rs` 5 =
  **13 passed**.
- Workspace total after Task 3: **44 tests passed**.
