---
name: docs-sync
description: Before staging a commit in the cgg repo, verify that user-visible surface changes (CLI flags, output formats, supported languages, resolver behavior, performance numbers) are reflected in README.md, the embedded mermaid graphs, the SKILL.md files under skills/, and any benchmark scripts. Trigger when about to `git commit` or open a PR, or after editing crates/cgg/src/cli.rs, crates/cgg-lang/src/plugins.rs, crates/cgg-resolve/**, scripts/benchmark.sh, or any flag/help-text/output-format code path. The pre-commit hook already regenerates the three embedded mermaid blocks via scripts/update-readme-graphs.py — this skill covers everything else: prose, flag tables, language tables, and skill copy that the hook does not touch.
---

# docs-sync — keep cgg docs honest

`cgg` ships its own contract surface: a CLI, a set of output formats,
a language matrix, a resolver pipeline, and three bundled skill files
that teach agents how to use it. When code changes, those documents
drift. This skill is the checklist for catching that drift *before*
a commit lands.

## When this skill fires

Pre-commit, or right after editing any of:

- `crates/cgg/src/cli.rs` — new/renamed flags, defaults, help text
- `crates/cgg/src/lib.rs` — the pipeline (`analyze` → `analyze_in_pool`)
  lives here since the 0.6.0 library split: phase order, audit shape,
  emission order. `main.rs` is only the CLI shim around it.
- `crates/cgg/src/options.rs` / `outcome.rs` — `RunOptions`,
  `RunOutcome`, `Emission`: the library contract both front ends share
- `crates/cgg-lang/src/plugins.rs` and `crates/cgg-lang/src/plugins/**`
  — language support changes
- `crates/cgg-resolve/**` — resolver behavior, FFI detection,
  confidence levels
- `crates/cgg-core/src/frameworks/rules.rs` — framework entry rules
- `crates/cgg-format/**` — output format additions or schema changes
- `scripts/benchmark.sh` — benchmark targets

Skip when: the change is internal refactoring with no user-visible
effect (private helpers, test-only code, formatting).

## What the pre-commit hook already handles

`.githooks/pre-commit` regenerates **three** embedded mermaid blocks in
`README.md` — `<!-- cgg:begin:walk -->`, `<!-- cgg:begin:lang -->` and
`<!-- cgg:begin:self -->` — by running the freshly built `cgg` and
piping through `scripts/update-readme-graphs.py`, then patches the
`<!-- cgg:begin:self-stats -->` line via `scripts/update-readme-stats.py`.
**Don't hand-edit those four regions.**

This skill covers everything *outside* those markers.

## The checklist

Run these checks against your staged diff. Each row is "if the diff
touches X, verify Y."

| If you changed… | Verify… |
| --- | --- |
| A CLI flag (added/removed/renamed/default changed) | The flag table **and the usage synopsis** under `## CLI` in `README.md` both match `cgg --help` — `docs-check.py` gates both directions separately. The recipe sections in `skills/cgg/SKILL.md` still reference real flags. |
| Help text / flag descriptions | `cgg --help` output is what you intended; quoted examples in README and SKILL.md still parse. |
| An output format (added, schema changed) | The "Output formats" table in both `README.md` and `skills/cgg/SKILL.md` lists it with the right use case. |
| Language plugin added/removed | The "Supported languages" table in `README.md`, the count in the README heading ("Supported languages (N)"), `scripts/benchmark.sh`'s `REPOS=( … )` list and `scripts/update-readme-stats.sh`'s `ENTRIES=( … )` list all agree. The `skills/cgg/SKILL.md` frontmatter description's language enumeration is accurate (or generic enough to not drift). |
| Resolver phase order or new resolver | The pipeline ASCII diagram in `README.md` ("## How it works") matches the order in `cgg::analyze_in_pool` (`crates/cgg/src/lib.rs`). `CLAUDE.md`'s "Architecture" section matches. |
| FFI detection (new attribute / language) | The FFI bullet under "## How it works" in `README.md` lists it. |
| Framework rules (`rules.rs`) | Every enumerating rule id is named in `scripts/benchmark.sh`'s `APPS` (or declared in `APPS_UNVERIFIED`) — `docs-check.py` fails otherwise. `skills/cgg-frameworks/SKILL.md`'s shape table and `[[framework]]` key list still match `FrameworkRule` in `crates/cgg-core/src/frameworks.rs`. |
| Benchmark script (`scripts/benchmark.sh`) | Re-run `./scripts/benchmark.sh` and paste current numbers into the README benchmark table, via `scripts/update-readme-stats.sh` or `scripts/patch-readme-stats.py`. |
| Anything about run cost or repeat runs | `skills/cgg/SKILL.md`'s "Performance and limits" section matches reality. It states there is **no cache** and no flag for one — every run re-parses from source. If that ever changes, that bullet is the first lie. |
| Audit format (`json`/`jsonl` shape) | README "## Audit / metrics" matches; SKILL.md's audit-diagnosis recipe still works. |
| A CLI flag that changes the graph | It is routed into `RunOptions` (`crates/cgg/src/options.rs`) **and** exposed as a `cgg-py` keyword argument, with the stub in `crates/cgg-py/python/cgg/_cgg.pyi` updated. `From<&Cli>` destructures with no `..` rest, so the compiler catches the first half; `docs-check.py` check 8 catches the second, unless you list the field in its `PY_DEFERRED_OPTIONS` with a reason. |
| `crates/cgg/src/lib.rs` public API (`analyze`, `RunOptions`, `RunOutcome`, `Emission`) | `crates/cgg-py/src/lib.rs` still compiles and the parity test in `crates/cgg-py/tests/test_analyze.py` still passes. `CLAUDE.md`'s **cgg (library + binary)** bullet matches. |
| Anything under `crates/cgg-py/` | `README.md`'s "## Python" section and `crates/cgg-py/README.md` agree, and every measured number in them was re-measured, not carried over. |
| `LanguagePlugin::extract` signature or `ExtractCtx` | All 44 plugins updated; `CLAUDE.md`'s "Adding a new language" step 2 matches. No new process-global — that is the bug `ExtractCtx` exists to prevent. |
| A `Box::leak` / `.leak()` / `mem::forget` anywhere in the pipeline crates | It is in `ALLOWED_LEAKS` in `scripts/docs-check.py` **with a reason that does not depend on the process exiting**. `cgg::analyze` runs in a loop from the library, cgg-py and cgg-ffi. Check 9 fails otherwise. |
| Anything that writes to stdout/stderr | It is an `Emission` on the transcript (`crates/cgg/src/outcome.rs`), in the right position, with the right `-q` gating. `crates/cgg/tests/cli_surface.rs` asserts on both streams; add a case if the position is new. |

## Verification commands

Run these as part of the check. Each is a deterministic oracle — never
assert a count or a flag from memory.

```bash
# Real CLI surface — the source of truth
./target/release/cgg --help

# Count language plugins actually registered (44 at time of writing).
# NB: the calls read `reg.register(...)`, so a `plugin.register` grep
# silently returns zero and "proves" nothing changed.
grep -c 'register(' crates/cgg-lang/src/plugins.rs

# Benchmark corpus size — parse the array, don't count quoted lines.
# `grep -c '^    "' scripts/benchmark.sh` counts APPS and everything
# else too (131), not the 45 REPOS entries.
python3 -c 'import re;t=open("scripts/benchmark.sh").read();m=re.search(r"REPOS=\(\s*\n(.*?)\n\)",t,re.S);print(sum(1 for l in m.group(1).splitlines() if l.strip().startswith(chr(34))))'

# Every consistency invariant at once — the cheapest gate there is
python3 scripts/docs-check.py

# Find stale command references in docs
rg --no-heading 'cgg [-]{1,2}[a-z]' README.md CLAUDE.md skills/
```

If any check turns up drift, fix the docs in the *same commit* as the
code change. A separate "docs catch-up" commit gets forgotten and the
README rots.

## What `docs-check.py` already gates

Don't hand-verify these; run the script. It is the last step of the
pre-commit hook, and it enforces:

0. `REPOS` in `benchmark.sh` covers exactly the same languages as
   `ENTRIES` in `update-readme-stats.sh`.
1. Plugin count == README `(N)` heading == language-table rows ==
   `REPOS` count (± the combined `xv6 (c+asm)` row).
2. Every flag in the README `## CLI` **table** exists in `cgg --help`.
3. A skill saying "Supports N languages" agrees with the plugin count.
4. Every `skills/*/SKILL.md` is linked from README, and "N bundled
   skills" matches how many exist.
5. Prose claiming attribute capture for "N plugins listed in Step 2"
   matches the plugins declaring `attributes: true`.
6. The README `## CLI` **usage synopsis** names every live flag and no
   dead one (no-op flags marked "No effect" are exempt).
7. The self-analysis showcase filter is identical in the hook,
   `update-readme-stats.sh`, `patch-readme-stats.py`, README and
   CLAUDE.md — and the generated block still spans ≥3 crates.
8. Every `RunOptions` field is reachable from `cgg-py` as a keyword, or
   listed in `PY_DEFERRED_OPTIONS` with a reason.
9. Every deliberate leak in the pipeline crates is in `ALLOWED_LEAKS`
   with a reason.

Plus one unnumbered check: every enumerating framework rule id in
`rules.rs` is named by a `benchmark.sh` `APPS` entry or declared in
`APPS_UNVERIFIED`.

## Special-case: changing a bundled skill

`skills/cgg/SKILL.md`, `skills/cgg-frameworks/SKILL.md` and
`skills/cgg-install/SKILL.md` are installed into users' agent
configurations by `scripts/install-skill.sh`. Changes there ship to
every user who re-runs the installer, so:

- Keep the frontmatter `description:` accurate — agents use it to
  decide whether to invoke the skill. Drift here means the skill
  stops firing or fires on the wrong prompts.
- Don't reference CLI flags that don't exist (`cgg --help` is the
  source of truth).
- Adding or removing a skill directory changes the README's "N bundled
  skills" phrase and its link list — check 4 will fail otherwise.
- Test by running `scripts/install-skill.sh --dry-run` and reading
  the would-be output.

## What this skill does NOT do

- Does not run `cargo test` — the pre-commit hook does that.
- Does not regenerate mermaid graphs or self-stats — the pre-commit
  hook does that.
- Does not auto-commit — surfaces the diff to fix; the human/agent
  edits and stages.
- Does not enforce style — there is no lint step beyond `cargo`'s.
- Does not cover pushing or releasing — that is the `push` skill.

The goal is "the README and SKILL.md describe what the code actually
does," not "every doc is rewritten every commit."
