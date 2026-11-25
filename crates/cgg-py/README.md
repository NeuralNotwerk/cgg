# cgg — Python bindings

Offline, deterministic call graphs for 46 languages, in-process.

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
pip install cgg-callgraphgenerator
```

```python
import cgg
```

**The distribution is `cgg-callgraphgenerator`; the import is `cgg`.**
PyPI's `cgg` belongs to an unrelated GGUF tool, so the short name was not
available. Python separates these two names routinely — `pip install
pillow` gives you `import PIL`.

> One caveat, because the other package also installs a top-level `cgg`
> module: **do not install both into the same environment.** Both write to
> `site-packages/cgg/`, pip will not stop you, and whichever lands second
> overwrites the first. If you already have `pip install cgg` (the GGUF
> tool), use a separate virtualenv.

The extension is built against the stable ABI (`abi3-py39`), so a single
wheel per platform covers every CPython ≥ 3.9 — no per-version builds.

**Five prebuilt wheels, ~13 MB each**, so `pip install` needs no compiler
on any of them:

| Wheel | Covers |
| --- | --- |
| `manylinux_2_17_x86_64` | x86-64 Linux (glibc ≥ 2.17) |
| `manylinux_2_28_aarch64` | arm64 Linux (glibc ≥ 2.28) |
| `macosx_10_12_x86_64` | Intel macOS |
| `macosx_11_0_arm64` | Apple-silicon macOS |
| `win_amd64` | x86-64 Windows |

An sdist ships too, so `pip install` still succeeds off that list — musl
Linux (Alpine) and Windows on arm64 are the ones that reach for it. There
pip builds from source, which needs a Rust toolchain (≥ 1.85) and takes a
few minutes.

## Usage

```python
import cgg

# Whole tree.
g = cgg.analyze("./src")

# A neighbourhood around what you care about.
g = cgg.analyze("./src", filter=[r"handle_request$"], hops=2)

# Several trees, one graph.
g = cgg.analyze(["./api", "./worker"], lang=["python", "go"])

g.to_mermaid()        # str — what agents read; byte-identical to `cgg -t mermaid`
g.to_mermaid(node_ids="hash", locations=True)   # `--node-ids hash --locations`
g.to_json()           # str — `cgg -t json`, bar the per-run timings it embeds
g.to_dot()            # str — Graphviz; to_dot(locations=True) too
g.to_graphml()        # str — Gephi / yEd / networkx; locations=True too
g.to_dict()           # dict — the escape hatch

len(g)                # callable count
g.callables           # tuple[Callable, ...]
g.edges               # tuple[Edge, ...]
g.files               # tuple[File, ...]
g.metrics             # run counters
g.notices             # what the CLI would print to stderr
g.jobs                # worker threads the run actually used
g.externals           # dict | None — the `cgg.externals.v1` document (externals=…)
g.externals_text()    # str | None — the ranked text report

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
object per callable, once, then caches. Measured on cgg's own `crates/`
(5,090 callables, 0.13.3): `to_mermaid()` produces 540 KiB in 9 ms, the
first `.callables` access costs 2.1 ms, and every access after it costs
0.1 µs.
Both are small here and both scale with the graph, so on a repository an
order of magnitude larger the attribute path is what you would notice.
Reach for the renderer when a string is what you want.

**Concurrent `analyze()` calls actually run concurrently.** The GIL is
released (`py.detach`) and there is no internal lock, so a thread pool
scales. N analyses of `crates/cgg-lang/src/plugins` from a
`ThreadPoolExecutor(N)`, against one analysis alone (187 ms) — medians of
four repetitions, 32-core host, `jobs` at its default of 16 there:

| threads | wall | vs. one analysis |
| --- | --- | --- |
| 1 | 167 ms | 0.90x |
| 2 | 220 ms | 1.18x |
| 4 | 271 ms | 1.45x |
| 8 | 426 ms | 2.28x |

Four analyses for 1.45x the wall clock of one; eight for 2.28x. Absolute
numbers are machine-specific and each analysis is already internally
parallel — regenerate them rather than trusting them.

Earlier builds would have had to take a process-wide lock for the whole of
`analyze`, because extraction read two process-global switches
(`DEADCODE_SIGNALS` and `EXTRA_REGISTRAR_VERBS`) that a second concurrent
call would corrupt. Those now travel in a per-run `cgg_lang::ExtractCtx`,
so there is no lock and no shared cell. Raising `jobs` on one call still
works and is simpler if you only have one tree to analyze.

## Not in this release

`--why-live` proofs, the `--write-roots` baseline, the ranked dead-code
report (each callable's `unreferenced` band is here; the report with its
evidence is not) and the audit event stream are reachable from the Rust
API but have no keyword and no `Graph` attribute here. Use the CLI for
those.

The framework-coverage table is *not* missing — it arrives rendered, as
one of the strings in `g.notices`, naming both what cgg recognised and
what it saw without rules. What is missing is a structured object; parse
the notice or use `cgg --framework-coverage` if you need fields.

## Building from source

```bash
scripts/build-python.sh          # from the repository root
```

Needs `cargo` (Rust ≥ 1.85), `uv` and `git` — the script checks for all
three up front and stops if one is missing.

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
