---
name: docs-sync
description: Before staging a commit in the cgg repo, verify that user-visible surface changes (CLI flags, output formats, supported languages, resolver behavior, performance numbers) are reflected in README.md, the embedded mermaid graphs, the SKILL.md files under skills/, and any benchmark scripts. Trigger when about to `git commit` or open a PR, or after editing crates/cgg/src/cli.rs, crates/cgg-lang/src/plugins.rs, crates/cgg-resolve/**, scripts/benchmark.sh, or any flag/help-text/output-format code path. The pre-commit hook already regenerates the two embedded mermaid blocks via scripts/update-readme-graphs.py — this skill covers everything else: prose, flag tables, language tables, and skill copy that the hook does not touch.
---

# docs-sync — keep cgg docs honest

`cgg` ships its own contract surface: a CLI, a set of output formats,
a language matrix, a resolver pipeline, and two embedded skill files
that teach agents how to use it. When code changes, those documents
drift. This skill is the checklist for catching that drift *before*
a commit lands.

## When this skill fires

Pre-commit, or right after editing any of:

- `crates/cgg/src/cli.rs` — new/renamed flags, defaults, help text
- `crates/cgg/src/main.rs` (the `run` orchestration) — phase order,
  audit shape
- `crates/cgg-lang/src/plugins.rs` and `crates/cgg-lang/src/plugins/**`
  — language support changes
- `crates/cgg-resolve/**` — resolver behavior, FFI detection,
  confidence levels
- `crates/cgg-format/**` — output format additions or schema changes
- `scripts/benchmark.sh` — benchmark targets

Skip when: the change is internal refactoring with no user-visible
effect (private helpers, test-only code, formatting).

## What the pre-commit hook already handles

`.githooks/pre-commit` regenerates the two embedded mermaid blocks in
`README.md` (between `<!-- cgg:begin:walk -->` / `cgg:begin:lang`
markers) by running the freshly built `cgg` and piping through
`scripts/update-readme-graphs.py`. **Don't hand-edit those blocks.**

This skill covers everything *outside* those markers.

## The checklist

Run these checks against your staged diff. Each row is "if the diff
touches X, verify Y."

| If you changed… | Verify… |
| --- | --- |
| A CLI flag (added/removed/renamed/default changed) | The flag table in `README.md` ("## CLI") matches `cgg --help`. The recipe sections in `skills/cgg/SKILL.md` still reference real flags. |
| Help text / flag descriptions | `cgg --help` output is what you intended; quoted examples in README and SKILL.md still parse. |
| An output format (added, schema changed) | The "Output formats" table in both `README.md` and `skills/cgg/SKILL.md` lists it with the right use case. |
| Language plugin added/removed | The "Supported languages" table in `README.md`, the count in the README intro ("Supported languages (N)"), and `scripts/benchmark.sh`'s `REPOS=( … )` list all agree. The `skills/cgg/SKILL.md` frontmatter description's language enumeration is accurate (or generic enough to not drift). |
| Resolver phase order or new resolver | The pipeline ASCII diagram in `README.md` ("## How it works") matches the order in `cgg::analyze_in_pool` (`crates/cgg/src/lib.rs`). `CLAUDE.md`'s "Architecture" section matches. |
| FFI detection (new attribute / language) | The FFI bullet under "## How it works" in `README.md` lists it. |
| Benchmark script (`scripts/benchmark.sh`) | Re-run `./scripts/benchmark.sh` and paste current numbers into the README benchmark table. Use `scripts/patch-readme-stats.py` if it applies. |
| Anything about caching / `.cgg-cache` | `skills/cgg/SKILL.md`'s "Performance and limits" section matches reality. |
| Audit format (`json`/`jsonl` shape) | README "## Audit / metrics" matches; SKILL.md's audit-diagnosis recipe still works. |
| A CLI flag that changes the graph | It is routed into `RunOptions` (`crates/cgg/src/options.rs`) **and** exposed as a `cgg-py` keyword argument, with the stub in `crates/cgg-py/python/cgg/_cgg.pyi` updated. `From<&Cli>` destructures with no `..` rest, so the compiler catches the first half; nothing catches the second. |
| `crates/cgg/src/lib.rs` public API (`analyze`, `RunOptions`, `RunOutcome`, `Emission`) | `crates/cgg-py/src/lib.rs` still compiles and the parity test in `crates/cgg-py/tests/test_analyze.py` still passes. `CLAUDE.md`'s **cgg (library + binary)** bullet matches. |
| Anything under `crates/cgg-py/` | `README.md`'s "## Python" section and `crates/cgg-py/README.md` agree, and every measured number in them was re-measured, not carried over. |
| `LanguagePlugin::extract` signature or `ExtractCtx` | All 44 plugins updated; `CLAUDE.md`'s "Adding a new language" step 2 matches. No new process-global — that is the bug `ExtractCtx` exists to prevent. |
| Anything that writes to stdout/stderr | It is an `Emission` on the transcript, in the right position, with the right `-q` gating. `crates/cgg/tests/cli_surface.rs` captures both streams as one file; add a case if the position is new. |

## Verification commands

Run these as part of the check:

```bash
# Real CLI surface — the source of truth
./target/release/cgg --help

# Count language plugins actually registered
rg -c 'plugin\.register' crates/cgg-lang/src/plugins.rs

# Benchmark targets vs language table
grep -c '^    "' scripts/benchmark.sh

# Find stale references in docs
rg --no-heading 'cgg [-]{1,2}[a-z]' README.md CLAUDE.md skills/
```

If any check turns up drift, fix the docs in the *same commit* as the
code change. A separate "docs catch-up" commit gets forgotten and the
README rots.

## Special-case: changing skills/cgg/SKILL.md itself

That file is installed into users' agent configurations by
`scripts/install-skill.sh`. Changes there ship to every user who
re-runs the installer, so:

- Keep the frontmatter `description:` accurate — agents use it to
  decide whether to invoke the skill. Drift here means the skill
  stops firing or fires on the wrong prompts.
- Don't reference CLI flags that don't exist (`cgg --help` is the
  source of truth).
- Test by running `scripts/install-skill.sh --dry-run` and reading
  the would-be output.

## What this skill does NOT do

- Does not run `cargo test` — the pre-commit hook does that.
- Does not regenerate mermaid graphs — the pre-commit hook does that.
- Does not auto-commit — surfaces the diff to fix; the human/agent
  edits and stages.
- Does not enforce style — there is no lint step beyond `cargo`'s.

The goal is "the README and SKILL.md describe what the code actually
does," not "every doc is rewritten every commit."
