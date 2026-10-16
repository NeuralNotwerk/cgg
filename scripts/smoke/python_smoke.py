"""Load the installed `cgg` module and make it analyse a real tree.

The release workflow runs this against every wheel it builds, and
`scripts/build-python.sh` runs it against the local build, so a check that
would fail the tag fails here first (0.11.0's first tag run died on a
mermaid assertion nothing local had ever executed).

Usage: python scripts/smoke/python_smoke.py [TREE]   (default crates/cgg-walk)
"""

import sys

import cgg

tree = sys.argv[1] if len(sys.argv) > 1 else "crates/cgg-walk"
g = cgg.analyze(tree)
assert len(g.callables) > 0, "no callables"
# Entry nodes put a `%%` banner above the flowchart line.
lines = g.to_mermaid().splitlines()
assert any(line.startswith("flowchart") for line in lines), "bad mermaid"
print("ok", cgg.__version__, len(g.callables), "callables")
