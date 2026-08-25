#!/usr/bin/env python3
"""Fail the commit when README.md disagrees with the code.

When this runs:
  - Automatically on every commit, by `.githooks/pre-commit` (last step,
    after all README patchers have run, so what it checks is the
    post-patch state).
  - Safe to run by hand from the repo root any time:
    `python3 scripts/docs-check.py`

Fourteen check functions. The numbered discussion below runs 0-12; the
unnumbered `check_framework_apps` is the fourteenth. Check 12 asserts
this count against the skills, so adding a check means updating any
skill that states the number.

0. Benchmark-table coverage. `ENTRIES` in update-readme-stats.sh (which
   writes the README's markdown benchmark table) must cover exactly the
   languages in `REPOS` in benchmark.sh (which clones the corpus).
   `ENTRIES` shipped five languages short of `REPOS`, so the README
   table claimed to cover every supported language while omitting
   smithy, proto, graphql, openapi and asyncapi.

1. Language-count consistency. The number of `register(` calls in
   `crates/cgg-lang/src/plugins.rs` must equal:
     - the `(N)` in README's "Supported languages (N)" heading,
     - the row count of the language table,
     - the entry count of `REPOS=( … )` in `scripts/benchmark.sh`
       (allowing one extra row for the combined `xv6 (c+asm)` entry,
       which is two languages running through one repo).

2. CLI flag freshness. Every flag named in the README's `## CLI` flag
   table must exist in `cgg --help` (catches renames/removals). The
   reverse direction is intentionally not checked — the README is
   curated and omits niche flags by design.

3. Skill language count. A skill saying "Supports N languages" must
   agree with the plugin count. The `cgg` skill's description is what an
   agent reads to decide whether cgg can help with a given repo, so a
   stale N there is a wrong answer, not a typo.

4. Skill inventory. Every `skills/*/SKILL.md` must be linked from the
   README, and the README's "N bundled skills" must match how many there
   are. `cgg-frameworks` shipped in 0.4.0 and went unmentioned for a
   whole release while the README said "two bundled skills".

5. Attribute-capture count. Prose claiming attribute capture covers N
   plugins must match the plugins that declare `attributes: true`. This
   number went from 2 to 9 while three separate copies of it in the docs
   and one in a runtime message all stayed at 2.

6. CLI synopsis coverage. The usage block under `## CLI` must name every
   flag in `cgg --help` that still does something, and must not name one
   that no longer exists. Check 2 only ever looked at the flag *table*,
   so `--since` shipped with a table row and a worked example while the
   synopsis above them never mentioned it. Deprecated no-op flags
   (`--stack-graphs`, `--no-update-check`) are exempt — they identify
   themselves with "No effect" in their help text.

7. Self-analysis showcase. Every place that names the filter for the
   README's showcase graph — README prose and its bash block, the
   pre-commit hook, update-readme-stats.sh, patch-readme-stats.py's
   anchor, CLAUDE.md — must name the SAME callable, and the generated
   mermaid block must span at least three crates. The 0.6.0 library
   split moved the pipeline out of `cgg::run` into
   `cgg::analyze_in_pool`, leaving `cgg::run` a 12-line shim whose name
   several test helpers also share. Everything kept "working": the hook
   regenerated the block, docs-check passed, and the README's flagship
   graph quietly became a wall of test-function names spanning one
   crate, under a sentence promising cross-crate calls. Nothing was
   broken enough to notice, which is exactly why this check exists.

8. Python keyword parity. Every `RunOptions` field must be reachable
   from `cgg-py` as a keyword argument, or be listed in
   `PY_DEFERRED_OPTIONS` below. `From<&Cli> for RunOptions` destructures
   with no `..` rest, so the compiler already catches a new flag that
   never reaches the pipeline — but nothing catches one that reaches the
   pipeline and never reaches Python. With two front ends over one
   pipeline, that is the drift that costs nothing to introduce.

9. Deliberate leaks. `Box::leak`, `.leak()` and `mem::forget` in the
   pipeline crates must appear in `ALLOWED_LEAKS` below, with a reason.
   `type_hints.rs` leaked a copy of every parameter name and type, with a
   comment calling it "acceptable because we're in a short-lived analysis
   pass". That was true while the pipeline was private to a binary that
   analyzed once and exited. 0.6.0 made it a library, a Python module and
   a C ABI callable in a loop, and the same line became ~161 bytes of
   unbounded growth per call in a host process. Nothing failed; the CLI
   never noticed. A leak whose justification is "the process is about to
   exit" needs re-checking every time that stops being true, so it has to
   be listed rather than merely commented.

10. CHANGELOG integrity. The newest entry must match the workspace
    version, version headers must strictly decrease, and every released
    `v*` tag must have an entry. An edit meant to INSERT the 0.6.3 entry
    replaced the `## [0.6.2]` header instead, so 0.6.2 vanished from the
    changelog while remaining published on two registries — its content
    silently absorbed into the entry above it. Nothing noticed, because
    nothing was checking that the file's own structure held together.

11. Skill publish claims. No file under `skills/`, `.claude/skills/` or
    `.kiro/skills/` may call a distribution channel unpublished while
    `.github/workflows/release.yml` has a publish step for it. Nothing
    gated skill *content* — only the inventory (check 4) and the
    language count (check 3) — so three skill files told readers that
    npm was unpublished and that the GitHub releases carried no
    binaries, for as long after 0.7.0 as it took someone to read them.
    Both statements steered users away from working install paths.

12. Skill check-count claims. A skill stating how many checks this file
    has must be right. Same root cause as 11; it is here because the
    number is trivially derivable and was trivially wrong.

Run from the repo root. Exits non-zero on any mismatch with a
human-readable message naming the offending file.
"""

from __future__ import annotations

import itertools
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
PLUGINS_RS = REPO_ROOT / "crates/cgg-lang/src/plugins.rs"
PLUGINS_DIR = REPO_ROOT / "crates/cgg-lang/src/plugins"
BENCH_SH = REPO_ROOT / "scripts/benchmark.sh"
README = REPO_ROOT / "README.md"
SKILLS_DIR = REPO_ROOT / "skills"
CGG_BIN = REPO_ROOT / "target/release/cgg"

NUMBER_WORDS = {
    "one": 1,
    "two": 2,
    "three": 3,
    "four": 4,
    "five": 5,
    "six": 6,
    "seven": 7,
    "eight": 8,
    "nine": 9,
    "ten": 10,
    # Past ten because docs-check now has more checks than that, and the
    # count claim is spelled out in prose. Truncating this map does not
    # fail a stale claim, it *silently ignores* it — which is how the
    # push skill's "eleven consistency invariants" survived the first
    # version of check 11.
    "eleven": 11,
    "twelve": 12,
    "thirteen": 13,
    "fourteen": 14,
    "fifteen": 15,
    "sixteen": 16,
    "seventeen": 17,
    "eighteen": 18,
    "nineteen": 19,
    "twenty": 20,
}
WORD_FOR = {v: k for k, v in NUMBER_WORDS.items()}


def fail(msg: str) -> None:
    print(f"[docs-check] {msg}", file=sys.stderr)
    sys.exit(1)


# ---------------------------------------------------------------------
# Check 1: language counts
# ---------------------------------------------------------------------


def count_plugins() -> int:
    text = PLUGINS_RS.read_text()
    return len(re.findall(r"\bregister\(", text))


def readme_intro_count(readme: str) -> int:
    m = re.search(r"## Supported languages \((\d+)\)", readme)
    if not m:
        fail("README is missing '## Supported languages (N)' heading")
    return int(m.group(1))


def readme_lang_table_rows(readme: str) -> int:
    # The language table starts with this header.
    m = re.search(
        r"\| Language \| Cross-file resolution \|.*?\n\|[-| ]+\|\n((?:\|.*\n)+)",
        readme,
    )
    if not m:
        fail("README language table not found")
    body = m.group(1)
    return sum(1 for line in body.splitlines() if line.startswith("|"))


def _bash_array_rows(path: Path, name: str) -> list[str]:
    text = path.read_text()
    m = re.search(rf"{name}=\(\s*\n(.*?)\n\)", text, re.DOTALL)
    if not m:
        fail(f"could not locate {name}=( ... ) array in {path.name}")
    return [
        line.strip().strip('"')
        for line in m.group(1).splitlines()
        if line.strip().startswith('"')
    ]


def check_benchmark_table_languages() -> None:
    """The README benchmark table must cover every benchmarked language.

    Two scripts hold the corpus list: `REPOS` in benchmark.sh (which
    clones and prints a terminal table) and `ENTRIES` in
    update-readme-stats.sh (which writes the README's markdown table).
    Nothing tied them together, and `ENTRIES` shipped five languages
    short — smithy, proto, graphql, openapi and asyncapi each had a
    cloned repository in the corpus and a row in benchmark.sh's output,
    but never appeared in the table a reader actually sees. The README
    claimed 44-language support above a table measuring 39 of them.
    """
    repos = _bash_array_rows(BENCH_SH, "REPOS")
    entries = _bash_array_rows(REPO_ROOT / "scripts/update-readme-stats.sh", "ENTRIES")
    # REPOS: name|url|lang|subdir|ctags_lang|ctags_kinds
    # ENTRIES: display|name|lang|subdir
    repo_langs = sorted(r.split("|")[2] for r in repos)
    entry_langs = sorted(e.split("|")[2] for e in entries)
    if repo_langs != entry_langs:
        missing = sorted(set(repo_langs) - set(entry_langs))
        extra = sorted(set(entry_langs) - set(repo_langs))
        parts = []
        if missing:
            parts.append(
                f"benchmarked but absent from the README table: {', '.join(missing)}"
            )
        if extra:
            parts.append(f"in the README table with no corpus repo: {', '.join(extra)}")
        fail(
            "scripts/benchmark.sh REPOS and scripts/update-readme-stats.sh "
            "ENTRIES disagree — " + "; ".join(parts or ["counts differ"])
        )


def benchmark_repo_count() -> int:
    text = BENCH_SH.read_text()
    # benchmark.sh has lines like:  "name|lang|url|subdir"  inside REPOS=( ... )
    m = re.search(r"REPOS=\(\s*\n(.*?)\n\)", text, re.DOTALL)
    if not m:
        fail("could not locate REPOS=( ... ) array in scripts/benchmark.sh")
    body = m.group(1)
    return sum(1 for line in body.splitlines() if line.strip().startswith('"'))


def check_language_counts() -> None:
    plugins = count_plugins()
    readme = README.read_text()
    intro = readme_intro_count(readme)
    rows = readme_lang_table_rows(readme)
    bench = benchmark_repo_count()
    # benchmark.sh allows one extra entry: the combined "xv6 (c+asm)"
    # row exercises the multi-language path with no new plugin.
    bench_expected_min, bench_expected_max = plugins, plugins + 1

    if intro != plugins:
        fail(
            f"plugin count {plugins} != README intro '({intro})' — update "
            f"'## Supported languages (N)' in README.md"
        )
    if rows != plugins:
        fail(
            f"plugin count {plugins} != README language table rows {rows} — "
            f"add/remove a row in the language table"
        )
    if not bench_expected_min <= bench <= bench_expected_max:
        fail(
            f"plugin count {plugins} but scripts/benchmark.sh has {bench} "
            f"REPOS entries (expected {plugins} or {plugins + 1})"
        )


# ---------------------------------------------------------------------
# Check 2: CLI flag freshness
# ---------------------------------------------------------------------


def cgg_help_text() -> str:
    if not CGG_BIN.exists():
        fail(f"{CGG_BIN} not found — run `cargo build --release -p cgg` first")
    return subprocess.run(
        [str(CGG_BIN), "--help"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout


def cgg_help_flags() -> set[str]:
    out = cgg_help_text()
    flags: set[str] = set()
    for m in re.finditer(
        r"(?<![A-Za-z0-9])(--[a-z][a-z0-9-]*|-[a-zA-Z])(?=[ ,\n])", out
    ):
        flags.add(m.group(1))
    return flags


def readme_flag_table_entries(readme: str) -> list[str]:
    # Slurp lines under "## CLI" between the table header and the next
    # blank line / next heading.
    # NB: no re.DOTALL. With it, `.` matches newlines and the trailing
    # `(?:\|.*\n)+` runs past the end of the table to the end of the
    # file, sweeping up every later line that happens to start with a
    # pipe — which is how an unrelated table elsewhere in the README can
    # be mistaken for undocumented flags. `[^\n]` keeps each repetition
    # on its own line so the capture stops at the first non-table line.
    m = re.search(
        r"## CLI\b[\s\S]*?\n\| Flag \| Default \| Description \|\n\|[-| ]+\|\n"
        r"((?:\|[^\n]*\n)+)",
        readme,
    )
    if not m:
        fail("README '## CLI' flag table not found")
    body = m.group(1)
    entries: list[str] = []
    for line in body.splitlines():
        if not line.startswith("|"):
            continue
        first_col = line.split("|", 2)[1].strip()
        # First column entries look like `` `--filter` `` or `` `-t` ``.
        entries.extend(re.findall(r"`([^`]+)`", first_col))
    return entries


def check_cli_flags() -> None:
    readme = README.read_text()
    documented = readme_flag_table_entries(readme)
    real = cgg_help_flags()
    stale = [f for f in documented if f not in real]
    if stale:
        fail(
            f"README documents flag(s) that no longer exist in `cgg --help`: "
            f"{', '.join(stale)} — update the '## CLI' table in README.md"
        )


# ---------------------------------------------------------------------
# Check 6: CLI synopsis coverage
# ---------------------------------------------------------------------

# Clap long-help lays each option out as either
#     -o, --output <FILE>
# or
#         --filter <PATTERN>
# both indented two spaces, with the description indented further below.
_OPT_HEADER = re.compile(r"^  (?:(-[a-zA-Z]), |    )(--[a-z][a-z0-9-]*)")


def cgg_help_options() -> list[tuple[str | None, str, str]]:
    """Every option in `--help` as (short, long, description)."""
    opts: list[tuple[str | None, str, str]] = []
    cur: tuple[str | None, str] | None = None
    desc: list[str] = []
    for line in cgg_help_text().splitlines():
        m = _OPT_HEADER.match(line)
        if m:
            if cur:
                opts.append((cur[0], cur[1], "\n".join(desc)))
            cur, desc = (m.group(1), m.group(2)), []
        elif cur is not None:
            desc.append(line)
    if cur:
        opts.append((cur[0], cur[1], "\n".join(desc)))
    return opts


def readme_synopsis_tokens(readme: str) -> set[str]:
    m = re.search(r"## CLI\b\s*\n+```text\n(.*?)```", readme, re.DOTALL)
    if not m:
        fail("README '## CLI' synopsis code block not found")
    block = m.group(1)
    long_flags = set(re.findall(r"--[a-z][a-z0-9-]*", block))
    short_flags = set(re.findall(r"(?<![\w-])(-[a-zA-Z])(?![\w-])", block))
    return long_flags | short_flags


def check_cli_synopsis() -> None:
    """The usage synopsis must name every live flag.

    Check 2 validates the flag *table*; the synopsis above it drifted
    unnoticed because nothing looked at it — `--since` shipped with its
    own README section and a table row but never made it into the
    usage block. Deprecated no-op flags are exempt: they announce
    themselves with "No effect" in their help text, and the synopsis is
    the wrong place to advertise a flag that does nothing.
    """
    readme = README.read_text()
    present = readme_synopsis_tokens(readme)
    opts = cgg_help_options()

    missing = [
        long
        for short, long, desc in opts
        if long not in ("--help", "--version")
        and "No effect" not in desc
        and long not in present
        and (short is None or short not in present)
    ]
    if missing:
        fail(
            f"README '## CLI' usage synopsis omits live flag(s): "
            f"{', '.join(sorted(missing))} — add them to the ```text "
            f"block under '## CLI' in README.md"
        )

    real_long = {long for _, long, _ in opts} | {"--help", "--version"}
    bogus = sorted(f for f in present if f.startswith("--") and f not in real_long)
    if bogus:
        fail(
            f"README '## CLI' usage synopsis names flag(s) that do not exist "
            f"in `cgg --help`: {', '.join(bogus)}"
        )


# ---------------------------------------------------------------------
# Checks 3-5: the bundled skills
# ---------------------------------------------------------------------


def skill_files() -> list[Path]:
    return sorted(SKILLS_DIR.glob("*/SKILL.md"))


def check_skill_language_count() -> None:
    plugins = count_plugins()
    for skill in skill_files():
        for m in re.finditer(r"[Ss]upports (\d+) languages", skill.read_text()):
            if int(m.group(1)) != plugins:
                fail(
                    f"{skill.relative_to(REPO_ROOT)} says 'Supports {m.group(1)} "
                    f"languages' but there are {plugins} plugins"
                )


def check_skill_inventory() -> None:
    readme = README.read_text()
    skills = skill_files()
    for skill in skills:
        link = f"skills/{skill.parent.name}/SKILL.md"
        if link not in readme:
            fail(
                f"{link} is not linked from README.md — a bundled skill "
                f"nobody is told about is a skill nobody loads"
            )
    want = WORD_FOR.get(len(skills))
    m = re.search(r"(\w+) bundled skills", readme)
    if not m:
        fail("README is missing an 'N bundled skills' phrase")
    got = m.group(1).lower()
    if got != want and got != str(len(skills)):
        fail(
            f"README says '{m.group(1)} bundled skills' but skills/ has "
            f"{len(skills)} ({want})"
        )


def count_attribute_plugins() -> int:
    """Plugins whose `signals()` declares `attributes: true`."""
    n = 0
    for f in PLUGINS_DIR.glob("*.rs"):
        text = f.read_text()
        m = re.search(r"fn signals\(&self\).*?\n    \}", text, re.DOTALL)
        if m and re.search(r"attributes:\s*true", m.group(0)):
            n += 1
    return n


def check_attribute_claims() -> None:
    real = count_attribute_plugins()
    for doc in [README, *skill_files()]:
        for m in re.finditer(r"(\w+) plugins listed in Step 2", doc.read_text()):
            claimed = NUMBER_WORDS.get(m.group(1).lower())
            if claimed is None and m.group(1).isdigit():
                claimed = int(m.group(1))
            if claimed != real:
                fail(
                    f"{doc.relative_to(REPO_ROOT)} claims attribute capture for "
                    f"'{m.group(1)}' plugins but {real} declare `attributes: true`"
                )


# ---------------------------------------------------------------------


def check_framework_apps() -> None:
    """Every framework rule needs a real application that exercises it.

    Text-only: asserts each `id:` in rules.rs is named by some APPS entry
    in benchmark.sh. Whether the rule actually *fires* on that app needs
    the corpus, so it lives in scripts/framework-coverage.py — this gate
    only catches the cheap failure of shipping a rule with no app behind
    it at all.
    """
    rules = REPO_ROOT / "crates/cgg-core/src/frameworks/rules.rs"
    if not rules.exists():
        return
    text_rs = rules.read_text()
    # Only rules that claim to *enumerate* need an application behind
    # them. A detect-only rule — no matchers, non-empty `gap` — asserts
    # the opposite: that cgg sees the framework and cannot enumerate it.
    # There is no enumeration to verify, so demanding an app would mean
    # cloning a repository per entry in a table whose whole purpose is to
    # be broad. scripts/framework-coverage.py reports which of these were
    # never observed in the corpus, as information rather than failure.
    ids: set[str] = set()
    for block in re.split(r"(?=RuleSpec\s*\{)", text_rs):
        m = re.search(r'id:\s*"([^"]+)"', block)
        if not m:
            continue
        enumerates = any(
            re.search(rf"{f}:\s*&\[", block)
            for f in ("attributes", "registrars", "base_types", "methods")
        )
        gap = re.search(r'gap:\s*"([^"]*)"', block)
        detect_only = not enumerates and gap and gap.group(1)
        if not detect_only:
            ids.add(m.group(1))
    if not ids:
        fail(f"could not parse any framework rule ids out of {rules}")

    text = BENCH_SH.read_text()
    m = re.search(r"APPS=\(\s*\n(.*?)\n\)", text, re.DOTALL)
    if not m:
        fail("could not locate APPS=( ... ) array in scripts/benchmark.sh")
    claimed: set[str] = set()
    for line in m.group(1).splitlines():
        line = line.strip().strip('"')
        if not line or line.startswith("#"):
            continue
        parts = line.split("|")
        if len(parts) >= 3:
            # A leading `~` marks a framework cgg detects but enumerates
            # no entries from. It still counts as having an application —
            # whether the gap is disclosed is checked against the corpus
            # by scripts/framework-coverage.py, not here.
            claimed |= {f.lstrip("~") for f in parts[2].split(",") if f}

    # Rules explicitly declared to have no real-world application. The
    # declaration is the point: it is greppable and reviewable, where a
    # missing app is not.
    unverified: set[str] = set()
    mu = re.search(r"APPS_UNVERIFIED=\(\s*\n(.*?)\n\)", text, re.DOTALL)
    if mu:
        for line in mu.group(1).splitlines():
            line = line.strip().strip('"')
            if line and not line.startswith("#"):
                unverified.add(line.split("|")[0])

    orphaned = sorted(ids - claimed - unverified)
    if orphaned:
        fail(
            f"{len(orphaned)} framework rule(s) with no application in "
            f"scripts/benchmark.sh APPS: {', '.join(orphaned)} — add an app "
            f"that uses the framework, then verify with "
            f"scripts/framework-coverage.py"
        )
    all_ids = set(re.findall(r'id:\s*"([^"]+)"', text_rs))
    unknown = sorted((claimed | unverified) - all_ids)
    if unknown:
        fail(
            f"scripts/benchmark.sh APPS claims framework(s) with no rule in "
            f"rules.rs: {', '.join(unknown)}"
        )


def check_self_analysis_showcase() -> None:
    """Check 7 — the showcase filter agrees everywhere and still shows a
    cross-crate graph.

    Two failure modes, both silent. The filter drifts in one file and not
    the others, so the hook regenerates a block the README's prose no
    longer describes; or the filter still resolves but to something
    trivial, so the block regenerates into a graph that proves nothing.
    The second is what happened when the pipeline moved out of
    `cgg::run`, and only the second needs the graph itself to catch.
    """
    sources = {
        ".githooks/pre-commit": r"--filter\s+'([^']+)'",
        "scripts/update-readme-stats.sh": r"--filter\s+'([^']+)'\s+-n 1 -o /tmp/cgg_self_graph",
        "README.md": r"cgg \./crates -t mermaid --filter '([^']+)'",
        "CLAUDE.md": r"--filter '([^']+)' -n 1",
    }
    found: dict[str, str] = {}
    for rel, pattern in sources.items():
        path = REPO_ROOT / rel
        m = re.search(pattern, path.read_text())
        if not m:
            fail(f"{rel}: could not find the self-analysis --filter (check 7)")
        found[rel] = m.group(1)

    distinct = set(found.values())
    if len(distinct) != 1:
        detail = "\n".join(f"    {k}: {v}" for k, v in sorted(found.items()))
        fail(
            "the self-analysis showcase filter disagrees between files "
            f"(check 7):\n{detail}\n"
            "  All of these regenerate or describe the same README block."
        )
    filt = distinct.pop()

    # patch-readme-stats.py locates the block by substring, so it has to
    # contain the bare callable name rather than the anchored regex.
    bare = filt.rstrip("$")
    patcher = (REPO_ROOT / "scripts/patch-readme-stats.py").read_text()
    if bare not in patcher:
        fail(
            f"scripts/patch-readme-stats.py does not mention '{bare}', so its "
            "anchor cannot find the self-analysis mermaid block (check 7)"
        )

    # The generated block must still be the cross-crate graph the prose
    # promises. Read the committed block rather than re-running cgg, so
    # this check costs nothing and works without a built binary.
    readme = README.read_text()
    m = re.search(
        r"<!-- cgg:begin:self -->(.*?)<!-- cgg:end:self -->", readme, re.DOTALL
    )
    if not m:
        fail("README.md: no <!-- cgg:begin:self --> block found (check 7)")
    block = m.group(1)
    crates = {
        n.split("::")[0]
        # Mermaid numbers its nodes `N0`, `N1`, … by default and carries
        # the content-derived base36 hash (`Ce8btaz0c7d`) under
        # `--node-ids hash`. Accept either; this check is about the
        # labels, and pinning the id shape only makes it fail whenever the
        # default moves.
        for n in re.findall(r'^\s*[CN][0-9a-z]+\["([^"]+)"\]', block, re.MULTILINE)
    }
    if len(crates) < 3:
        fail(
            "README.md's self-analysis graph spans only "
            f"{len(crates)} crate(s) ({', '.join(sorted(crates)) or 'none'}), but the "
            "prose above it calls every edge a real cross-crate function "
            f"call (check 7).\n  The filter is '{filt}'. If the pipeline moved, "
            "retarget it everywhere check 7 looks."
        )


# `RunOptions` fields with no `cgg-py` keyword, and why. Anything not
# listed here must be reachable from Python.
PY_DEFERRED_OPTIONS = {
    # Replace the graph with a different artifact entirely. Exposing them
    # as keywords on a function that returns a Graph would mean returning
    # something that is not one; they want their own entry points.
    "why_live": "returns proofs, not a graph — needs its own entry point",
    "write_roots": "returns a TOML baseline, not a graph — ditto",
    # CLI-only concerns that do not change the graph.
    "framework_coverage": "controls a stderr table; Python reads .notices",
    "profile": "renders a stderr timing table from a process-global",
}


def check_python_option_parity() -> None:
    """Check 8 — every graph-changing option is reachable from Python."""
    opts_rs = (REPO_ROOT / "crates/cgg/src/options.rs").read_text()
    # Fields of `pub struct RunOptions`, up to its closing brace.
    m = re.search(r"pub struct RunOptions \{(.*?)\n\}", opts_rs, re.DOTALL)
    if not m:
        fail("crates/cgg/src/options.rs: could not find `pub struct RunOptions`")
    fields = set(re.findall(r"^\s*pub ([a-z_0-9]+):", m.group(1), re.MULTILINE))
    if not fields:
        fail("crates/cgg/src/options.rs: parsed RunOptions but found no fields")

    py_rs = (REPO_ROOT / "crates/cgg-py/src/lib.rs").read_text()
    # Anchored on `fn analyze` rather than "the first signature in the
    # file". Renderers carry their own `#[pyo3(signature = ...)]` now, and
    # an unanchored search silently read one of those instead — which
    # reported every keyword on `analyze` as missing.
    sig = re.search(
        r"#\[pyo3\(signature = \((.*?)\)\)\](?:\s*#\[[^\]]*\])*\s*fn analyze\b",
        py_rs,
        re.DOTALL,
    )
    if not sig:
        fail(
            "crates/cgg-py/src/lib.rs: could not find the "
            "#[pyo3(signature = …)] on `fn analyze`"
        )
    kwargs = set(re.findall(r"^\s*\*?\s*([a-z_0-9]+)", sig.group(1), re.MULTILINE))
    # `--no-entry-nodes` is deliberately un-negated as `entry_nodes`.
    if "entry_nodes" in kwargs:
        kwargs.add("no_entry_nodes")

    missing = sorted(fields - kwargs - set(PY_DEFERRED_OPTIONS))
    if missing:
        fail(
            "RunOptions field(s) with no cgg-py keyword argument: "
            f"{', '.join(missing)} (check 8).\n"
            "  Add the keyword in crates/cgg-py/src/lib.rs and the stub in\n"
            "  crates/cgg-py/python/cgg/_cgg.pyi, or add it to\n"
            "  PY_DEFERRED_OPTIONS in this script with a reason."
        )

    stale = sorted(set(PY_DEFERRED_OPTIONS) - fields)
    if stale:
        fail(
            "PY_DEFERRED_OPTIONS names field(s) that no longer exist on "
            f"RunOptions: {', '.join(stale)} (check 8)"
        )

    # A deferred option that quietly gained a keyword should stop being
    # listed as deferred, or the list becomes decoration.
    contradictory = sorted(set(PY_DEFERRED_OPTIONS) & kwargs)
    if contradictory:
        fail(
            f"PY_DEFERRED_OPTIONS lists {', '.join(contradictory)}, but cgg-py "
            "exposes it as a keyword (check 8) — drop it from the list"
        )


# Deliberate leaks that are genuinely bounded, and why. Anything else in
# the pipeline crates is a check-9 failure.
#
# Key is "<path>:<symbol>". The reason is the point: a leak is only
# acceptable while its justification holds, and these are the ones whose
# justification does not depend on the process being about to exit.
ALLOWED_LEAKS = {
    "crates/cgg-core/src/profile.rs:Box::leak": (
        "One Counters per distinct span name. Span names are &'static str "
        "literals, so the set is bounded by the source, not by input or by "
        "call count — a cache, not a leak. Also #[cfg(debug_assertions)], "
        "so it is compiled out of release entirely."
    ),
}

# Where a leak would ride on user input rather than on the source.
LEAK_SCANNED_CRATES = (
    "cgg-core",
    "cgg-lang",
    "cgg-resolve",
    "cgg-format",
    "cgg-walk",
    "cgg",
)

LEAK_PATTERN = re.compile(r"\bBox::leak\b|\.leak\(\)|\bmem::forget\b")


def check_deliberate_leaks() -> None:
    """Check 9 — no unlisted deliberate leak in the pipeline crates."""
    found: dict[str, list[int]] = {}
    for crate in LEAK_SCANNED_CRATES:
        src = REPO_ROOT / "crates" / crate / "src"
        if not src.is_dir():
            continue
        for path in sorted(src.rglob("*.rs")):
            rel = path.relative_to(REPO_ROOT).as_posix()
            for i, line in enumerate(path.read_text().splitlines(), 1):
                # Skip the doc/comment prose that explains the rule.
                stripped = line.lstrip()
                if stripped.startswith(("//", "///", "//!", "*")):
                    continue
                m = LEAK_PATTERN.search(line)
                if not m:
                    continue
                symbol = m.group(0).replace("()", "").lstrip(".")
                symbol = "Box::leak" if "Box::leak" in m.group(0) else symbol
                key = f"{rel}:{symbol}"
                if key in ALLOWED_LEAKS:
                    continue
                found.setdefault(key, []).append(i)

    if found:
        detail = "\n".join(
            f"    {k} (line{'s' if len(v) > 1 else ''} {', '.join(map(str, v))})"
            for k, v in sorted(found.items())
        )
        fail(
            "deliberate leak(s) in the pipeline with no entry in "
            f"ALLOWED_LEAKS (check 9):\n{detail}\n"
            "  `cgg::analyze` is called in a loop by the library, the Python\n"
            "  module and the C ABI, so a per-call leak grows without bound in\n"
            "  a host process. If it is genuinely bounded, add it to\n"
            "  ALLOWED_LEAKS in this script with the reason it is bounded."
        )

    # split on the FIRST colon: the path has none, but the symbol
    # (`Box::leak`) is full of them.
    stale = sorted(
        k for k in ALLOWED_LEAKS if not (REPO_ROOT / k.split(":", 1)[0]).exists()
    )
    if stale:
        fail(f"ALLOWED_LEAKS names missing file(s): {', '.join(stale)} (check 9)")


def check_changelog() -> None:
    """Check 10 — the changelog agrees with the version and the tags."""
    path = REPO_ROOT / "CHANGELOG.md"
    headers = re.findall(r"^## \[(\d+\.\d+\.\d+)\]", path.read_text(), re.MULTILINE)
    if not headers:
        fail("CHANGELOG.md has no `## [x.y.z]` version headers (check 10)")

    def key(v: str) -> tuple[int, ...]:
        return tuple(int(p) for p in v.split("."))

    cargo = re.search(
        r'^version = "(.+?)"', (REPO_ROOT / "Cargo.toml").read_text(), re.MULTILINE
    )
    if cargo and headers[0] != cargo.group(1):
        fail(
            f"CHANGELOG.md's newest entry is {headers[0]} but Cargo.toml is "
            f"{cargo.group(1)} (check 10) — release notes and the version "
            "shipped must not disagree"
        )

    for a, b in itertools.pairwise(headers):
        if key(a) <= key(b):
            fail(
                f"CHANGELOG.md versions are not strictly decreasing: {a} then {b} (check 10)"
            )

    # A gap in the patch series inside one minor means an entry went
    # missing — which is the actual failure this check exists for, and
    # which the tag rule below cannot see because an untagged release has
    # no tag to compare against. Deliberate skips are declarable.
    text = path.read_text()
    for a, b in itertools.pairwise(headers):
        ka, kb = key(a), key(b)
        if ka[:2] != kb[:2]:
            continue  # different minor; a patch gap is meaningless
        for missing in range(kb[2] + 1, ka[2]):
            gap = f"{ka[0]}.{ka[1]}.{missing}"
            if f"changelog:skipped {gap}" in text:
                continue
            fail(
                f"CHANGELOG.md jumps {b} -> {a}, so {gap} has no entry "
                f"(check 10). If {gap} was never released, declare it with "
                f"an HTML comment containing `changelog:skipped {gap}`."
            )

    # A tag is a promise that a version was released; it needs an entry.
    try:
        tags = subprocess.run(
            ["git", "tag", "-l", "v*"],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            check=True,
        ).stdout.split()
    except (subprocess.CalledProcessError, FileNotFoundError):
        return  # not a git checkout; the rest of the check still ran
    have = set(headers)
    missing = sorted(
        (
            t[1:]
            for t in tags
            if re.fullmatch(r"v\d+\.\d+\.\d+", t) and t[1:] not in have
        ),
        key=key,
    )
    if missing:
        fail(
            f"released tag(s) with no CHANGELOG entry: {', '.join('v' + m for m in missing)} "
            "(check 10)"
        )


# ---------------------------------------------------------------------
# Checks 11 and 12: skill claims about publish channels, and about
# how many checks this file has
#
# Nothing gated the *content* of skills/, .claude/skills/ or
# .kiro/skills/ — only the inventory and the language count — which is
# exactly where drift accumulated. Three skill files told a reader that
# npm was unpublished and that docs-check had eleven checks, both false,
# for as long as it took someone to read them.
#
# Only mechanically-decidable claims are gated here. Prose that needs
# judgement stays the push skill's job.
# ---------------------------------------------------------------------

SKILL_DIRS = (
    REPO_ROOT / "skills",
    REPO_ROOT / ".claude/skills",
    REPO_ROOT / ".kiro/skills",
)
RELEASE_YML = REPO_ROOT / ".github/workflows/release.yml"

# channel -> (regex proving release.yml publishes it, tokens a skill uses
#             to name it). A skill may only call a channel unpublished if
#             the workflow has no publish step for it.
PUBLISH_CHANNELS = {
    "npm": (r"npm publish", ("npm", "cgg-node")),
    "PyPI": (r"pypi-publish|twine upload", ("pypi", "pip install")),
    "crates.io": (r"cargo publish|publish-crates", ("crates.io", "cargo install")),
    "GitHub binaries": (r"action-gh-release", ("release tarball", "prebuilt binar")),
}
UNPUBLISHED_RE = re.compile(
    r"(?:is |are )?\*{0,2}not\s+published\*{0,2}|carr(?:y|ies) no prebuilt",
    re.IGNORECASE,
)


def _skill_files() -> list[Path]:
    return sorted(
        p for d in SKILL_DIRS if d.is_dir() for p in d.rglob("SKILL.md")
    )


def check_skill_publish_claims() -> None:
    """No skill may call a channel unpublished that release.yml publishes."""
    if not RELEASE_YML.is_file():
        return
    workflow = RELEASE_YML.read_text()
    live = {
        name
        for name, (proof, _) in PUBLISH_CHANNELS.items()
        if re.search(proof, workflow)
    }
    for path in _skill_files():
        rel = path.relative_to(REPO_ROOT)
        for lineno, line in enumerate(path.read_text().splitlines(), 1):
            if not UNPUBLISHED_RE.search(line):
                continue
            for name in live:
                if any(t in line.lower() for t in PUBLISH_CHANNELS[name][1]):
                    fail(
                        f"{rel}:{lineno} calls {name} unpublished, but "
                        f"release.yml publishes it (check 11): {line.strip()}"
                    )


def check_skill_docs_check_count() -> None:
    """A skill counting docs-check's checks must match how many exist."""
    actual = len(re.findall(r"^def check_", Path(__file__).read_text(), re.M))
    pat = re.compile(
        r"\b(\w+)\s+(?:consistency invariants|check functions|checks)\b",
        re.IGNORECASE,
    )
    for path in _skill_files():
        text = path.read_text()
        if "docs-check" not in text:
            continue
        rel = path.relative_to(REPO_ROOT)
        for lineno, line in enumerate(text.splitlines(), 1):
            if "docs-check" not in line and "consistency invariant" not in line:
                continue
            m = pat.search(line)
            if not m:
                continue
            word = m.group(1).lower()
            claimed = NUMBER_WORDS.get(word)
            if claimed is None and word.isdigit():
                claimed = int(word)
            if claimed is not None and claimed != actual:
                fail(
                    f"{rel}:{lineno} claims {word} docs-check checks; "
                    f"there are {actual} (check 12)"
                )


def main() -> None:
    check_language_counts()
    check_benchmark_table_languages()
    check_framework_apps()
    check_cli_flags()
    check_cli_synopsis()
    check_skill_language_count()
    check_skill_inventory()
    check_attribute_claims()
    check_self_analysis_showcase()
    check_python_option_parity()
    check_deliberate_leaks()
    check_changelog()
    check_skill_publish_claims()
    check_skill_docs_check_count()
    print("[docs-check] ok")


if __name__ == "__main__":
    main()
