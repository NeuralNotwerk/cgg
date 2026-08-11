"""cgg — offline, deterministic call graphs for 44 languages.

Point it at a source tree, get a graph back::

    import cgg

    g = cgg.analyze("./src")
    print(g.to_mermaid())

    by_id = {f.id: f.path for f in g.files}
    for c in g.callables:
        if c.unreferenced:
            print(f"{c.qualified_name}  {by_id[c.file]}:{c.start_line}")

Everything is computed in-process by the same Rust pipeline the ``cgg``
command-line tool runs, in the same order, so the two cannot disagree.
There are no network calls and no language servers.

Two things worth knowing: the renderers never build Python objects (so
``to_mermaid()`` is the cheap path, while ``graph.callables`` constructs
one object per callable, once, then caches), and ``analyze()`` releases the
GIL and holds no internal lock, so calling it from a thread pool actually
scales. Both are explained in ``crates/cgg-py/README.md``.
"""

from __future__ import annotations

from ._cgg import (
    Callable,
    CggError,
    Edge,
    File,
    Graph,
    Metrics,
    __version__,
    analyze,
    languages,
)

__all__ = [
    "analyze",
    "languages",
    "Graph",
    "Callable",
    "Edge",
    "File",
    "Metrics",
    "CggError",
    "__version__",
]
