#!/usr/bin/env python3
"""Regenerate the README's `cgg`-generated mermaid blocks.

Reads mermaid output emitted by `cgg` (passed as temp-file arguments),
cleans it (strips `::tests::*` nodes, dedupes edges, unescapes the
HTML-escaped angle brackets cgg emits, and strips the `crate::` prefix
for readability), then patches `README.md` between
`<!-- cgg:begin:<marker> -->` and `<!-- cgg:end:<marker> -->`.

Prefix a marker with `raw:` to insert cgg's output verbatim instead.
That is the right mode for a block the README presents as "here is what
this command prints": cleaning it would make the documented command and
the documented output disagree.

When this runs:
  - Automatically on every commit, by `.githooks/pre-commit` (after the
    release build, before the self-stats patch and docs-check). Patches
    the `cgg-walk`, `cgg-lang` and self-analysis mermaid blocks.
  - Manual reruns are fine but rarely needed — the hook keeps these
    blocks in sync.

Idempotent: if the graphs haven't changed, the README won't either.

Usage:
  update-readme-graphs.py <marker> <mmd-file> [<marker> <mmd-file> ...]
  update-readme-graphs.py raw:self self.mmd
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
    # (src, dst) -> edge label ("" for a bare arrow). cgg collapses
    # repeated call sites into `A -->|3x| B`, and a `" --> "` test misses
    # those entirely — which silently *deleted* every multi-site edge
    # from the README graphs rather than rendering it unlabelled.
    edges: dict[tuple[str, str], str] = {}
    keep: set[str] = set()
    edge_re = re.compile(r"^(\S+)\s*-->\s*(?:\|([^|]*)\|\s*)?(\S+)$")
    for line in mmd_text.splitlines():
        s = line.strip()
        if not s or s.startswith(("flowchart", "%%")):
            continue
        if s.startswith("C") and "[" in s:
            nid, rest = s.split("[", 1)
            label = rest.rsplit("]", 1)[0].strip('"')
            if "::tests::" in label or label.endswith("::tests"):
                continue
            nodes[nid] = label
            keep.add(nid)
            continue
        m = edge_re.match(s)
        if m:
            src, lbl, dst = m.group(1), m.group(2) or "", m.group(3)
            edges[(src, dst)] = lbl

    edges = {(s, d): lbl for (s, d), lbl in edges.items() if s in keep and d in keep}

    def tidy(lbl: str) -> str:
        return lbl.replace("crate::", "").replace("&lt;", "<").replace("&gt;", ">")

    out = ["flowchart LR"]
    # Node ids are content-derived base36 hashes now, not sequential
    # integers, so there's no numeric value to sort by — `nodes` already
    # preserves cgg's own (deterministic) emission order, which is what
    # we want anyway.
    for nid in nodes:
        out.append(f'  {nid}["{tidy(nodes[nid])}"]')
    for (s, d), lbl in sorted(edges.items()):
        arrow = f"-->|{lbl}|" if lbl else "-->"
        out.append(f"  {s} {arrow} {d}")
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


def self_test() -> None:
    """Guard the cleaner against silently dropping edges.

    `clean()` used to match edges with `" --> " in line`, which is false
    for cgg's collapsed multi-site form `A -->|3x| B`. Every such edge
    vanished from the README graphs with no error and no diff to notice,
    because the nodes stayed and only the arrow went missing.
    """
    src = """flowchart LR
  C0["a::f"]
  C1["a::g"]
  C2["a::tests::t"]
  C0 --> C1
  C1 -->|3x| C0
  C2 --> C0"""
    out = clean(src)
    assert "C1 -->|3x| C0" in out, f"multiplicity label dropped:\n{out}"
    assert "C0 --> C1" in out, f"bare edge dropped:\n{out}"
    assert "C2" not in out, f"test node kept:\n{out}"
    print("[update-readme-graphs] self-test ok")


def main() -> None:
    argv = sys.argv[1:]
    if argv == ["--self-test"]:
        self_test()
        return
    if not argv or len(argv) % 2 != 0:
        print("usage: update-readme-graphs.py <marker> <mmd> [...]", file=sys.stderr)
        sys.exit(2)

    readme_path = Path("README.md")
    readme = readme_path.read_text()

    # Process each (marker, mmd file) pair. `raw:` disables cleaning.
    for marker, mmd_path in zip(argv[0::2], argv[1::2]):
        mmd_text = Path(mmd_path).read_text()
        if marker.startswith("raw:"):
            marker, body = marker[4:], mmd_text.rstrip("\n")
        else:
            body = clean(mmd_text)
        readme = patch(readme, marker, body)

    readme_path.write_text(readme)


if __name__ == "__main__":
    main()
