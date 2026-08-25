"""Tests for the cgg Python bindings.

The important one is `test_parity_with_cli`. Everything else checks that
the translation layer works; that one checks it did not quietly become a
*different* analysis from the one the command-line tool runs. It is the
test that has to keep passing as the pipeline evolves.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import threading
from pathlib import Path

import cgg
import pytest

# --- fixtures ----------------------------------------------------------

REPO = Path(__file__).resolve().parents[3]


def _cli() -> Path:
    """The cgg binary built from this same tree.

    Honours CGG_BIN so the parity test can be pointed at a specific build.
    Skips rather than fails when absent: a missing binary means the test
    cannot run, not that the bindings are broken.
    """
    env = os.environ.get("CGG_BIN")
    if env:
        return Path(env)
    return REPO / "target" / "release" / "cgg"


@pytest.fixture
def tree(tmp_path: Path) -> Path:
    """A small multi-language tree with cross-file calls and an orphan."""
    (tmp_path / "pkg").mkdir()
    for i in range(4):
        (tmp_path / "pkg" / f"mod_{i}.py").write_text(
            f"def helper_{i}(x):\n"
            f"    return x + {i}\n"
            f"\n"
            f"def caller_{i}():\n"
            f"    return helper_{i}({i})\n"
            f"\n"
            f"def orphan_{i}():\n"
            f"    return {i}\n"
        )
    # A trait with an impl and a function passed by name, so
    # `dynamic_dispatch` and `reference_edges` have something to add. Without
    # them those flags change nothing here and any test comparing them to a
    # default run is vacuous.
    (tmp_path / "lib.rs").write_text(
        "pub trait Greet {\n"
        "    fn greet(&self) -> u32;\n"
        "}\n"
        "pub struct Loud;\n"
        "impl Greet for Loud {\n"
        "    fn greet(&self) -> u32 { helper() }\n"
        "}\n"
        "pub fn used() -> u32 { helper() }\n"
        "fn helper() -> u32 { 1 }\n"
        "fn never_called() -> u32 { 2 }\n"
        "pub fn register() { take(helper); }\n"
        "fn take(_f: fn() -> u32) {}\n"
    )
    (tmp_path / "svc.go").write_text(
        "package main\n"
        "\n"
        "func Serve() int { return help() }\n"
        "func help() int { return 1 }\n"
    )
    return tmp_path


def structure(doc: dict) -> tuple[list[str], list[str]]:
    """Everything about a graph that must be reproducible.

    Deliberately excludes per-run timings (`parse_ms`, `wall_ms`): the
    JSON embeds them, so two runs of the *same* build never compare equal
    byte-for-byte. crates/cgg/tests/determinism.rs documents the same trap.
    """
    callables = [
        f"{k}|{v['qualified_name']}|{v['language']}|{v['start_line']}"
        for k, v in doc["callables"].items()
    ]
    edges = [
        f"{e['src']}->{e['dst']}@{e['site_byte']}"
        f"|{json.dumps(e['via'], sort_keys=True)}|{e['confidence']}"
        for e in doc["edges"]
    ]
    return callables, edges


# --- the parity test ---------------------------------------------------


def test_parity_with_cli(tree: Path) -> None:
    """`cgg.analyze(...).to_json()` must match `cgg -t json` structurally.

    This is the test that stops the bindings drifting from the CLI. Both
    must run the same pipeline in the same order over the same tree, so
    every callable, every edge, and every edge's provenance and confidence
    must agree exactly.
    """
    binary = _cli()
    if not binary.exists():
        pytest.skip(
            f"cgg binary not built at {binary}; run cargo build --release -p cgg"
        )

    proc = subprocess.run(
        [str(binary), str(tree), "-t", "json"],
        capture_output=True,
        text=True,
        check=True,
    )
    from_cli = json.loads(proc.stdout)
    from_py = json.loads(cgg.analyze(tree).to_json())

    cli_c, cli_e = structure(from_cli)
    py_c, py_e = structure(from_py)

    assert py_c == cli_c, "callable sets differ between the module and the CLI"
    assert py_e == cli_e, "edge sets differ between the module and the CLI"
    # Same file set, and the same content hashes for them.
    assert {f["path"]: f["blake3"] for f in from_py["files"].values()} == {
        f["path"]: f["blake3"] for f in from_cli["files"].values()
    }


def test_parity_with_cli_under_options(tree: Path) -> None:
    """Parity must survive the option surface, not just the default run."""
    binary = _cli()
    if not binary.exists():
        pytest.skip("cgg binary not built")

    cases = [
        (
            ["--include-external", "--include-stdlib"],
            {"include_external": True, "include_stdlib": True},
        ),
        (
            ["--dynamic-dispatch", "--reference-edges"],
            {"dynamic_dispatch": True, "reference_edges": True},
        ),
        (["--no-entry-nodes"], {"entry_nodes": False}),
        (["--lang", "python"], {"lang": ["python"]}),
        (["--filter", "caller_1", "-n", "1"], {"filter": ["caller_1"], "hops": 1}),
        (["--exclude-partial", "orphan"], {"exclude_partial": ["orphan"]}),
        (["--jobs", "3"], {"jobs": 3}),
    ]
    baseline = structure(json.loads(cgg.analyze(tree).to_json()))
    changed_something = False

    for argv, kwargs in cases:
        proc = subprocess.run(
            [str(binary), str(tree), "-t", "json", *argv],
            capture_output=True,
            text=True,
            check=True,
        )
        cli = structure(json.loads(proc.stdout))
        py = structure(json.loads(cgg.analyze(tree, **kwargs).to_json()))
        assert py == cli, f"parity broken for {argv} / {kwargs}"
        if py != baseline:
            changed_something = True

    # Non-vacuity: if every option produced the default graph, the loop
    # above would pass even if the module ignored its keyword arguments
    # entirely.
    assert changed_something, (
        "no option changed the graph on this fixture, so parity-under-options "
        "cannot detect an ignored keyword argument"
    )


# --- basics ------------------------------------------------------------


def test_analyze_returns_a_populated_graph(tree: Path) -> None:
    g = cgg.analyze(tree)
    assert len(g) > 0
    assert len(g) == len(g.callables)
    assert len(g.files) >= 3  # python, rust, go
    assert {f.language for f in g.files} >= {"python", "rust", "go"}
    assert g.metrics.files_analyzed == len(g.files)
    assert "Graph" in repr(g)


def test_paths_accepts_str_pathlike_and_sequences(tree: Path) -> None:
    by_str = len(cgg.analyze(str(tree)))
    by_path = len(cgg.analyze(tree))
    by_list = len(cgg.analyze([tree]))
    by_tuple = len(cgg.analyze((str(tree),)))
    assert by_str == by_path == by_list == by_tuple

    # Two paths named separately must equal analyzing what contains them.
    split = cgg.analyze([tree / "pkg", tree / "lib.rs"])
    assert len(split) > 0
    assert len(split) < by_path  # svc.go excluded


def test_all_four_renderers(tree: Path) -> None:
    g = cgg.analyze(tree)
    assert g.to_mermaid().startswith("flowchart")
    assert "digraph" in g.to_dot()
    assert g.to_graphml().lstrip().startswith("<?xml") or "<graphml" in g.to_graphml()
    doc = json.loads(g.to_json())
    assert set(doc) >= {"callables", "edges", "files"}


def test_mermaid_numbers_its_nodes_by_default(tree: Path) -> None:
    """The module must emit the same mermaid ids the CLI does.

    The default is per-format and lives on `OutputFormat`, precisely so
    the four front ends cannot drift on it — but nothing enforces that
    from Python's side except this.
    """
    g = cgg.analyze(tree)
    text = g.to_mermaid()
    assert re.search(r"(?m)^  N\d+\[", text), text[:400]
    assert not re.search(r"(?m)^  C[0-9a-z]{4,}\[", text), text[:400]


def test_mermaid_node_ids_hash_matches_the_json_ids(tree: Path) -> None:
    g = cgg.analyze(tree)
    text = g.to_mermaid(node_ids="hash")
    ids = set(json.loads(g.to_json())["callables"])
    assert ids, "fixture produced no callables"
    for cid in ids:
        assert f"  {cid}[" in text, f"{cid} missing from hashed mermaid"


def test_mermaid_node_ids_short_is_the_explicit_default(tree: Path) -> None:
    g = cgg.analyze(tree)
    assert g.to_mermaid(node_ids="short") == g.to_mermaid()


def test_mermaid_node_ids_rejects_anything_else(tree: Path) -> None:
    g = cgg.analyze(tree)
    with pytest.raises(ValueError, match="short"):
        g.to_mermaid(node_ids="sequential")


def test_mermaid_scheme_changes_only_the_names(tree: Path) -> None:
    """Same graph either way — same node count, same arrows, same labels."""
    g = cgg.analyze(tree)
    short, hashed = g.to_mermaid(), g.to_mermaid(node_ids="hash")
    labels = lambda s: re.findall(r'\["([^"]*)"\]', s)  # noqa: E731
    assert labels(short) == labels(hashed)
    assert short.count(" --> ") == hashed.count(" --> ")
    assert len(short.splitlines()) == len(hashed.splitlines())


def test_to_dict_agrees_with_to_json(tree: Path) -> None:
    g = cgg.analyze(tree)
    assert g.to_dict() == json.loads(g.to_json())


def test_renderers_do_not_need_attribute_access(tree: Path) -> None:
    """A renderer must work without ever materializing Python objects.

    Not a performance assertion — that would be flaky — but it does pin the
    ordering: if `to_mermaid` ever came to depend on the cached tuples,
    this would still pass, so the point is only that the lazy path is not
    required to be primed first.
    """
    g = cgg.analyze(tree)
    text = g.to_mermaid()  # first touch is the renderer
    assert "flowchart" in text
    assert len(g.callables) > 0  # and the lazy path still works afterwards


def test_callables_are_cached_and_identical_between_accesses(tree: Path) -> None:
    g = cgg.analyze(tree)
    first, second = g.callables, g.callables
    assert first is second, "callables should be built once and cached"


# --- graph queries -----------------------------------------------------


def test_callable_lookup_and_adjacency(tree: Path) -> None:
    g = cgg.analyze(tree)

    caller = g.callable("pkg.mod_0.caller_0")
    if caller is None:  # qualified-name shape is plugin-specific
        caller = next(c for c in g.callables if c.simple_name == "caller_0")

    callees = g.callees_of(caller)
    assert any(c.simple_name == "helper_0" for c in callees), (
        f"caller_0 should call helper_0; got {[c.simple_name for c in callees]}"
    )

    helper = next(c for c in g.callables if c.simple_name == "helper_0")
    callers = g.callers_of(helper)
    assert any(c.simple_name == "caller_0" for c in callers)

    # By qualified name, same answer as by object.
    assert [c.id for c in g.callees_of(caller.qualified_name)] == [
        c.id for c in callees
    ]


def test_unknown_name_yields_no_neighbours(tree: Path) -> None:
    g = cgg.analyze(tree)
    assert g.callable("no::such::function") is None
    assert g.callers_of("no::such::function") == []
    assert g.callees_of("no::such::function") == []


def test_bad_target_type_is_a_value_error(tree: Path) -> None:
    g = cgg.analyze(tree)
    with pytest.raises(ValueError):
        g.callers_of(object())


# --- options -----------------------------------------------------------


def test_filter_and_hops_narrow_the_graph(tree: Path) -> None:
    everything = cgg.analyze(tree)
    narrowed = cgg.analyze(tree, filter=["caller_1"], hops=1)
    assert 0 < len(narrowed) < len(everything)
    assert any(c.simple_name == "caller_1" for c in narrowed.callables)


def test_lang_restricts_the_languages_analyzed(tree: Path) -> None:
    g = cgg.analyze(tree, lang=["python"])
    assert {f.language for f in g.files} == {"python"}


def test_entry_nodes_defaults_on_and_can_be_turned_off(tmp_path: Path) -> None:
    """`entry_nodes=True` is the default, matching the CLI.

    The keyword is positive where the flag is negative (`--no-entry-nodes`);
    the default behaviour is identical.
    """
    (tmp_path / "app.py").write_text(
        "from flask import Flask\n"
        "app = Flask(__name__)\n"
        "\n"
        "@app.route('/x')\n"
        "def view():\n"
        "    return helper()\n"
        "\n"
        "def helper():\n"
        "    return 1\n"
    )
    with_entries = cgg.analyze(tmp_path)
    without = cgg.analyze(tmp_path, entry_nodes=False)
    assert len(with_entries) > len(without), (
        "entry nodes should be on by default and add at least one node"
    )
    assert any(c.synthetic for c in with_entries.callables)
    assert not any(c.synthetic for c in without.callables)


def test_include_external_and_stdlib_add_exit_nodes(tree: Path) -> None:
    plain = cgg.analyze(tree)
    wide = cgg.analyze(tree, include_external=True, include_stdlib=True)
    assert len(wide) >= len(plain)


def test_dead_code_marks_unreferenced_callables(tree: Path) -> None:
    g = cgg.analyze(tree, dead_code=True, dead_code_confidence="low")
    marked = [c for c in g.callables if c.unreferenced]
    assert marked, "the fixture's orphans should be reported at low confidence"
    assert all(c.unreferenced in {"high", "medium", "low"} for c in marked)

    # Nothing is marked without asking.
    plain = cgg.analyze(tree)
    assert not any(c.unreferenced for c in plain.callables)


def test_bad_confidence_is_a_value_error(tree: Path) -> None:
    with pytest.raises(ValueError, match="high"):
        cgg.analyze(tree, dead_code=True, dead_code_confidence="sometimes")


def test_edges_carry_provenance_and_confidence(tree: Path) -> None:
    g = cgg.analyze(tree)
    assert g.edges
    assert all(e.confidence in {"high", "medium", "low"} for e in g.edges)
    assert all(isinstance(e.via, str) and e.via for e in g.edges)
    # Filtering by trust must be possible, which is the point of exposing it.
    solid = [e for e in g.edges if e.confidence == "high" and e.via == "direct"]
    assert solid


# --- errors ------------------------------------------------------------


def test_missing_path_raises_cgg_error() -> None:
    with pytest.raises(cgg.CggError) as e:
        cgg.analyze("/no/such/cgg/directory")
    assert "/no/such/cgg/directory" in str(e.value)


def test_empty_path_list_raises_cgg_error() -> None:
    with pytest.raises(cgg.CggError):
        cgg.analyze([])


def test_bad_regex_raises_cgg_error(tree: Path) -> None:
    with pytest.raises(cgg.CggError):
        cgg.analyze(tree, filter=["("])


def test_bad_paths_type_raises_value_error() -> None:
    with pytest.raises(ValueError):
        cgg.analyze(42)


def test_cgg_error_is_catchable_as_exception(tree: Path) -> None:
    try:
        cgg.analyze("/no/such/dir")
    except Exception as e:  # noqa: BLE001 - that is the assertion
        assert isinstance(e, cgg.CggError)
    else:
        pytest.fail("expected CggError")


# --- in-process behaviour ----------------------------------------------


def test_repeated_calls_are_identical(tree: Path) -> None:
    """The regression test for the process-global state fixed in commit C.

    A subprocess gets a fresh address space every run, so the CLI could
    never have caught state that survives a call. This can.
    """
    first = structure(cgg.analyze(tree).to_dict())
    for n in range(2, 5):
        assert structure(cgg.analyze(tree).to_dict()) == first, f"call {n} differs"


def test_jobs_is_honoured_on_every_call(tree: Path) -> None:
    """`jobs` must mean something on the second call, not just the first."""
    for jobs in (1, 4, 1, 2):
        g = cgg.analyze(tree, jobs=jobs)
        assert g.jobs == jobs, f"asked for {jobs} workers, ran on {g.jobs}"


def test_interleaved_option_sets_do_not_contaminate(tree: Path) -> None:
    plain_a = structure(cgg.analyze(tree).to_dict())
    dead_a = structure(cgg.analyze(tree, dead_code=True).to_dict())
    plain_b = structure(cgg.analyze(tree).to_dict())
    dead_b = structure(cgg.analyze(tree, dead_code=True).to_dict())
    assert plain_a == plain_b
    assert dead_a == dead_b


def test_analyze_releases_the_gil(tree: Path) -> None:
    """Another Python thread must make progress during an analysis.

    Without `Python::detach` around `cgg::analyze` the interpreter would be
    frozen for the whole parse, which on a real repository is seconds.
    """
    ticks = 0
    stop = threading.Event()

    def tick() -> None:
        nonlocal ticks
        while not stop.is_set():
            ticks += 1

    t = threading.Thread(target=tick, daemon=True)
    t.start()
    try:
        # Analyze something substantial enough to take real time.
        for _ in range(3):
            cgg.analyze(REPO / "crates" / "cgg-lang" / "src" / "plugins")
    finally:
        stop.set()
        t.join(timeout=5)

    assert ticks > 0, "the GIL was held for the whole analysis"


def test_concurrent_analyses_return_correct_results(tree: Path) -> None:
    """Concurrent calls must each return the right graph.

    `analyze()` releases the GIL and holds no lock, so these genuinely run
    at the same time — which is exactly why the result has to be checked.
    A hang here rather than a failure would mean a lock came back.
    """
    expected = structure(cgg.analyze(tree).to_dict())
    results: list[tuple] = []
    lock = threading.Lock()

    def work() -> None:
        s = structure(cgg.analyze(tree).to_dict())
        with lock:
            results.append(s)

    threads = [threading.Thread(target=work) for _ in range(6)]
    for t in threads:
        t.start()
    for t in threads:
        t.join(timeout=120)
        assert not t.is_alive(), "a concurrent analyze did not finish"

    assert len(results) == 6
    assert all(r == expected for r in results)


# --- module surface ----------------------------------------------------


def test_languages_matches_the_plugin_count() -> None:
    langs = cgg.languages()
    assert langs == sorted(langs)
    assert len(langs) == len(set(langs))
    assert {"python", "rust", "go", "smithy", "openapi"} <= set(langs)


def test_version_is_exposed() -> None:
    assert cgg.__version__.count(".") == 2


def test_public_surface_is_what_all_says() -> None:
    for name in cgg.__all__:
        assert hasattr(cgg, name), f"__all__ names {name}, which is missing"


def test_type_stubs_ship_with_the_package() -> None:
    """A typed package must actually carry its markers.

    `py.typed` missing means every type checker silently ignores the stubs,
    which looks exactly like having no types at all.
    """
    pkg = Path(cgg.__file__).parent
    assert (pkg / "py.typed").exists(), "py.typed marker not installed"
    assert (pkg / "_cgg.pyi").exists(), "type stubs not installed"
