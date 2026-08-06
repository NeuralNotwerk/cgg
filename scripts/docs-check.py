#!/usr/bin/env python3
"""Fail the commit when README.md disagrees with the code.

When this runs:
  - Automatically on every commit, by `.githooks/pre-commit` (last step,
    after all README patchers have run, so what it checks is the
    post-patch state).
  - Safe to run by hand from the repo root any time:
    `python3 scripts/docs-check.py`

Six checks:

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

Run from the repo root. Exits non-zero on any mismatch with a
human-readable message naming the offending file.
"""

from __future__ import annotations

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
    "one": 1, "two": 2, "three": 3, "four": 4, "five": 5,
    "six": 6, "seven": 7, "eight": 8, "nine": 9, "ten": 10,
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
    for m in re.finditer(r"(?<![A-Za-z0-9])(--[a-z][a-z0-9-]*|-[a-zA-Z])(?=[ ,\n])", out):
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
        for tok in re.findall(r"`([^`]+)`", first_col):
            entries.append(tok)
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
    ids = set(re.findall(r'id:\s*"([^"]+)"', rules.read_text()))
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

    orphaned = sorted(ids - claimed)
    if orphaned:
        fail(
            f"{len(orphaned)} framework rule(s) with no application in "
            f"scripts/benchmark.sh APPS: {', '.join(orphaned)} — add an app "
            f"that uses the framework, then verify with "
            f"scripts/framework-coverage.py"
        )
    unknown = sorted(claimed - ids)
    if unknown:
        fail(
            f"scripts/benchmark.sh APPS claims framework(s) with no rule in "
            f"rules.rs: {', '.join(unknown)}"
        )


def main() -> None:
    check_language_counts()
    check_framework_apps()
    check_cli_flags()
    check_cli_synopsis()
    check_skill_language_count()
    check_skill_inventory()
    check_attribute_claims()
    print("[docs-check] ok")


if __name__ == "__main__":
    main()
