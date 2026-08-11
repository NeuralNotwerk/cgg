# cgg — Python bindings

Offline, deterministic call graphs for 44 languages, in-process.

```python
import cgg

g = cgg.analyze("./src")
print(g.to_mermaid())
```

No network calls, no language servers, no build artifacts required. The
analysis is the same Rust pipeline the `cgg` command-line tool runs, in
the same order, so the two cannot disagree — there is a parity test that
compares this module's JSON output against the binary's on the same tree.

## Install

```bash
pip install cgg
```

One wheel per platform covers every CPython ≥ 3.9 (the extension is built
against the stable ABI).

## Usage

```python
import cgg

# Whole tree.
g = cgg.analyze("./src")

# A neighbourhood around what you care about.
g = cgg.analyze("./src", filter=[r"handle_request$"], hops=2)

# Several trees, one graph.
g = cgg.analyze(["./api", "./worker"], lang=["python", "go"])

g.to_mermaid()        # str — what agents read
g.to_json()           # str — identical to `cgg -t json`
g.to_dot()            # str — Graphviz
g.to_graphml()        # str — Gephi / yEd / networkx
g.to_dict()           # dict — the escape hatch

len(g)                # callable count
g.callables           # tuple[Callable, ...]
g.edges               # tuple[Edge, ...]
g.files               # tuple[File, ...]
g.metrics             # run counters
g.notices             # what the CLI would print to stderr
g.jobs                # worker threads the run actually used

g.callable("mypkg.mod.func")     # Callable | None
g.callers_of("mypkg.mod.func")   # list[Callable]
g.callees_of("mypkg.mod.func")   # list[Callable]
```

### Finding code nothing calls

```python
g = cgg.analyze("./src", dead_code=True, dead_code_confidence="high")
paths = {f.id: f.path for f in g.files}
for c in g.callables:
    if c.unreferenced:
        print(f"{c.unreferenced:6} {c.qualified_name}  {paths[c.file]}:{c.start_line}")
```

**BEST EFFORT.** Every finding is a hypothesis. It means cgg could not
find a caller, not that none exists — reflection, FFI, a framework cgg has
no rules for, and dynamic dispatch all produce callers it cannot see.

### Filtering by trust

Every edge carries how it was established and how much cgg trusts it, so
you can narrow to what you are willing to rely on:

```python
solid = [e for e in g.edges if e.confidence == "high" and e.via == "direct"]
```

## Two things worth knowing

**Renderers never build Python objects.** `to_mermaid()` and friends
render straight from the Rust graph; `g.callables` constructs one Python
object per callable, once, then caches. Measured on cgg's own tree (1,943
callables): `to_mermaid()` produces 178 KB in 1.4 ms, and the first
`.callables` access costs 1.0 ms. Both are small here and both scale with
the graph, so on a repository an order of magnitude larger the attribute
path is what you would notice. Reach for the renderer when a string is
what you want.

**Concurrent `analyze()` calls actually run concurrently.** The GIL is
released and there is no internal lock, so a thread pool scales. Measured on
`crates/cgg-lang/src/plugins`, against a single analysis alone:

| threads | wall | vs. one analysis |
| --- | --- | --- |
| 1 | 106 ms | 1.00x |
| 2 | 107 ms | 1.00x |
| 4 | 114 ms | 1.07x |
| 8 | 178 ms | 1.68x |

Earlier builds took a process-wide lock for the whole of `analyze`, because
extraction read two process-global switches. Those now travel in a
per-run context, so the lock is gone: at 4 threads that is 421 ms before
versus 114 ms after. Raising `jobs` on one call still works and is simpler
if you only have one tree to analyze.

## Not in this release

`--why-live` proofs, the `--write-roots` baseline, the audit event stream,
and the framework-coverage table are all reachable from the Rust API but
are not yet exposed here. Use the CLI for those.

## Building from source

```bash
scripts/build-python.sh          # from the repository root
```

Needs a Rust toolchain (≥ 1.85) and `uv`.

`cargo build` compiles the `.so`, but only maturin can make it importable
— it writes the wheel metadata and puts the library where Python will find
it. That is all `build-python.sh` does, plus provisioning an interpreter,
since `abi3-py39` rules out anything older than 3.9 and a system `python3`
often is.

The crate is an ordinary workspace member. It builds without a Python
interpreter present at all: `abi3` fixes the ABI at compile time and
`extension-module` means libpython is never linked, so `Py_*` resolves at
load time from whichever interpreter imports the module.

## License

Apache-2.0 OR MIT.
