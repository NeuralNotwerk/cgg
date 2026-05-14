#!/usr/bin/env python3
"""Regenerate the README's `cgg`-generated mermaid blocks.

Reads mermaid output emitted by `cgg` (passed as temp-file arguments),
cleans it (strips `::tests::*` nodes, dedupes edges, unescapes the
HTML-escaped angle brackets cgg emits, and strips the `crate::` prefix
for readability), then patches `README.md` between
`<!-- cgg:begin:<marker> -->` and `<!-- cgg:end:<marker> -->`.

When this runs:
  - Automatically on every commit, by `.githooks/pre-commit` (after the
    release build, before the self-stats patch and docs-check). Patches
    the `cgg-walk` and `cgg-lang` mermaid blocks.
  - Manual reruns are fine but rarely needed — the hook keeps these
    blocks in sync.

Idempotent: if the graphs haven't changed, the README won't either.

Usage:
  update-readme-graphs.py <marker> <mmd-file> [<marker> <mmd-file> ...]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path


def clean(mmd_text: str) -> str:
    """Return a tidied mermaid flowchart.

    * Drops test-module nodes (`::tests::*`) and any edges that touch
      them — keeps the reader focused on production code paths.
    * Dedupes edges.
    * Strips the `crate::` prefix (the README already lives inside the
      crate, so this noise is redundant).
    * Unescapes `&lt;`/`&gt;` that mermaid requires inside labels but
      reads awkwardly inline.
    """
    nodes: dict[str, str] = {}
    edges: set[tuple[str, str]] = set()
    keep: set[str] = set()
    for line in mmd_text.splitlines():
        s = line.strip()
        if not s or s.startswith("flowchart"):
            continue
        if s.startswith("C") and "[" in s:
            nid, rest = s.split("[", 1)
            label = rest.rsplit("]", 1)[0].strip('"')
            if "::tests::" in label or label.endswith("::tests"):
                continue
            nodes[nid] = label
            keep.add(nid)
        elif " --> " in s:
            src, dst = (x.strip() for x in s.split("-->"))
            edges.add((src, dst))

    edges = {(s, d) for (s, d) in edges if s in keep and d in keep}

    def tidy(lbl: str) -> str:
        return (
            lbl.replace("crate::", "").replace("&lt;", "<").replace("&gt;", ">")
        )

    out = ["flowchart LR"]
    for nid in sorted(keep, key=lambda x: int(x[1:])):
        out.append(f'  {nid}["{tidy(nodes[nid])}"]')
    for s, d in sorted(edges):
        out.append(f"  {s} --> {d}")
    return "\n".join(out)


def patch(readme: str, marker: str, new_content: str) -> str:
    """Replace the body between the marker comments with new_content."""
    begin = f"<!-- cgg:begin:{marker} -->"
    end = f"<!-- cgg:end:{marker} -->"
    pattern = re.compile(
        re.escape(begin) + r"\n```mermaid\n.*?\n```\n" + re.escape(end),
        re.DOTALL,
    )
    replacement = f"{begin}\n```mermaid\n{new_content}\n```\n{end}"
    patched, n = pattern.subn(replacement, readme)
    if n == 0:
        print(
            f"error: markers {begin!r}/{end!r} not found in README.md",
            file=sys.stderr,
        )
        sys.exit(1)
    return patched


def main() -> None:
    argv = sys.argv[1:]
    if not argv or len(argv) % 2 != 0:
        print("usage: update-readme-graphs.py <marker> <mmd> [...]", file=sys.stderr)
        sys.exit(2)

    readme_path = Path("README.md")
    readme = readme_path.read_text()

    # Process each (marker, mmd file) pair.
    for marker, mmd_path in zip(argv[0::2], argv[1::2]):
        mmd_text = Path(mmd_path).read_text()
        cleaned = clean(mmd_text)
        readme = patch(readme, marker, cleaned)

    readme_path.write_text(readme)


if __name__ == "__main__":
    main()
