#!/usr/bin/env python3
"""Local shim: the A/B tool lives in the repo as scripts/graph-ab.py.
Adds the local extra repo and keeps the existing result cache."""
import os, sys
here = os.path.dirname(os.path.abspath(__file__))
os.environ.setdefault("CGG_AB_EXTRA", "/storage/neurosymbolic_kb")
os.environ.setdefault("CGG_AB_CACHE", os.path.join(here, "..", "cache"))
script = os.path.join(here, "..", "..", "..", "scripts", "graph-ab.py")
os.execv(sys.executable, [sys.executable, script, *sys.argv[1:]])
