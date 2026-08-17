//! Python bindings for cgg.
//!
//! A translation layer and nothing more: [`analyze`] turns keyword
//! arguments into [`cgg::RunOptions`], calls [`cgg::analyze`], and wraps
//! the result. No analysis logic here, so the Python API cannot drift from
//! the CLI — `tests/` has a parity test that proves it.
//!
//! The GIL is released for the analysis; [`Graph`] materializes Python
//! objects lazily and the renderers build none. Both measured in
//! `crates/cgg-py/README.md`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyTuple;
use pyo3::{create_exception, wrap_pyfunction};

use cgg_core::graph::{
    CallEdge, CallableKind, CallableNode, Confidence, FileRecord, Via,
};
use cgg_format::OutputFormat;

/// Mirrors the binary's allocator; see Cargo.toml for the measurement.
#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

create_exception!(
    _cgg,
    CggError,
    pyo3::exceptions::PyException,
    "Raised when a cgg analysis fails."
);

/// `anyhow::Error` -> `CggError`, keeping the whole context chain.
///
/// `{:#}`, not `{}`: the outermost message is usually the least
/// informative part ("running dead-code analysis" over the real cause).
fn to_py_err(e: anyhow::Error) -> PyErr {
    CggError::new_err(format!("{e:#}"))
}

// --- Leaf types --------------------------------------------------------

/// A callable — function, method, closure, or a synthesized entry/exit
/// node.
#[pyclass(frozen, module = "cgg", from_py_object)]
#[derive(Clone)]
pub struct Callable {
    #[pyo3(get)]
    id: u64,
    #[pyo3(get)]
    qualified_name: String,
    #[pyo3(get)]
    simple_name: String,
    #[pyo3(get)]
    kind: &'static str,
    #[pyo3(get)]
    language: String,
    #[pyo3(get)]
    file: u64,
    #[pyo3(get)]
    start_line: u32,
    #[pyo3(get)]
    end_line: u32,
    #[pyo3(get)]
    signature_hint: String,
    #[pyo3(get)]
    visibility: String,
    #[pyo3(get)]
    attributes: Vec<String>,
    /// True for nodes cgg minted rather than found in source: framework
    /// entries, external/stdlib exit nodes, derives.
    #[pyo3(get)]
    synthetic: bool,
    /// Confidence band if cgg found no caller, else `None`.
    ///
    /// BEST EFFORT: a hypothesis, not a proof. It means cgg could not find
    /// a caller, which is not the same as there being none.
    #[pyo3(get)]
    unreferenced: Option<&'static str>,
}

impl Callable {
    fn from_node(n: &CallableNode) -> Self {
        // No `..` rest: a new field on `CallableNode` breaks this until
        // someone decides whether Python should see it. cgg-core changes far
        // more often than `Cli`, so this is where the guard earns its keep.
        let CallableNode {
            id,
            qualified_name,
            simple_name,
            kind,
            language,
            file,
            start_line,
            end_line,
            signature_hint,
            visibility,
            attributes,
            synthetic,
            unreferenced,
            // Deliberately not exposed to Python.
            start_byte: _,
            end_byte: _,
            trait_impl_target: _,
            vis: _,
            test_role: _,
            framework_entry: _,
        } = n;
        Self {
            id: id.as_u64(),
            qualified_name: qualified_name.clone(),
            simple_name: simple_name.clone(),
            kind: kind_str(*kind),
            language: language.clone(),
            file: file.as_u64(),
            start_line: *start_line,
            end_line: *end_line,
            signature_hint: signature_hint.clone(),
            visibility: visibility.clone(),
            attributes: attributes.clone(),
            synthetic: *synthetic,
            unreferenced: unreferenced.map(confidence_str),
        }
    }
}

/// Lowercase name of a `CallableKind`, matching what `-t json` emits.
///
/// Exhaustive `&'static str` rather than `format!("{:?}").to_lowercase()`:
/// that allocated twice per callable, and a new variant would arrive in
/// Python as whatever `Debug` happened to print.
fn kind_str(k: CallableKind) -> &'static str {
    match k {
        CallableKind::Function => "function",
        CallableKind::Method => "method",
        CallableKind::Constructor => "constructor",
        CallableKind::Destructor => "destructor",
        CallableKind::Closure => "closure",
        CallableKind::Property => "property",
    }
}

/// Lowercase name of a `Confidence`. Same reasoning as [`kind_str`].
fn confidence_str(c: Confidence) -> &'static str {
    match c {
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
    }
}

#[pymethods]
impl Callable {
    fn __repr__(&self) -> String {
        format!(
            "Callable(id={}, qualified_name={:?}, language={:?}, lines={}-{})",
            self.id, self.qualified_name, self.language, self.start_line, self.end_line
        )
    }
}

/// A call edge between two callables.
#[pyclass(frozen, module = "cgg", skip_from_py_object)]
#[derive(Clone)]
pub struct Edge {
    #[pyo3(get)]
    src: u64,
    #[pyo3(get)]
    dst: u64,
    #[pyo3(get)]
    site_line: u32,
    #[pyo3(get)]
    site_byte: u32,
    /// `"high"`, `"medium"` or `"low"`.
    #[pyo3(get)]
    confidence: &'static str,
    /// How the edge was established — `"direct"`, `"dynamic"`,
    /// `"reference"`, `"external"`, `"stdlib"`, `"ffi"`, `"descriptor"`,
    /// `"framework_entry"`. Filter on this to keep only edges you trust.
    #[pyo3(get)]
    via: &'static str,
}

impl Edge {
    fn from_edge(e: &CallEdge) -> Self {
        // No `..` rest — see `Callable::from_node`.
        let CallEdge {
            src,
            dst,
            site_line,
            site_byte,
            confidence,
            via,
            // `resolver` is provenance for debugging a surprising edge; it
            // is in `to_json()` but costs an allocation per edge here.
            resolver: _,
        } = e;
        Self {
            src: src.as_u64(),
            dst: dst.as_u64(),
            site_line: *site_line,
            site_byte: *site_byte,
            confidence: confidence_str(*confidence),
            via: via_kind(via),
        }
    }
}

/// The `kind` tag of a `Via`, matching what `-t json` emits.
///
/// Exhaustive rather than a `serde_json` round-trip: no allocation per
/// edge, and a new variant is a compile error here instead of arriving in
/// Python as `"unknown"`. `test_parity_with_cli` checks these against the
/// CLI's JSON, which is what keeps them in step with `Via`'s serde tags.
fn via_kind(via: &Via) -> &'static str {
    match via {
        Via::Direct => "direct",
        Via::Dynamic => "dynamic",
        Via::Reference => "reference",
        Via::External => "external",
        Via::Stdlib => "stdlib",
        Via::Ffi(_) => "ffi",
        Via::Descriptor(_) => "descriptor",
        Via::FrameworkEntry(_) => "framework_entry",
    }
}

#[pymethods]
impl Edge {
    fn __repr__(&self) -> String {
        format!(
            "Edge(src={}, dst={}, via={:?}, confidence={:?}, line={})",
            self.src, self.dst, self.via, self.confidence, self.site_line
        )
    }
}

/// An analyzed source file.
#[pyclass(frozen, module = "cgg", skip_from_py_object)]
#[derive(Clone)]
pub struct File {
    #[pyo3(get)]
    id: u64,
    #[pyo3(get)]
    path: PathBuf,
    #[pyo3(get)]
    language: String,
    /// How the language was decided: extension, shebang, or header.
    #[pyo3(get)]
    detected_via: String,
    #[pyo3(get)]
    blake3: String,
    #[pyo3(get)]
    size_bytes: u64,
    #[pyo3(get)]
    lines: u32,
    /// `"ok"` or `"error"`. `"error"` means tree-sitter recovered from a
    /// syntax error; the file still contributes what could be parsed.
    #[pyo3(get)]
    parse_status: String,
}

impl File {
    fn from_record(f: &FileRecord) -> Self {
        // No `..` rest — see `Callable::from_node`.
        let FileRecord {
            id,
            path,
            language,
            detected_via,
            blake3,
            size_bytes,
            lines,
            parse_status,
            // Per-run timing and the test classification are not exposed.
            parse_ms: _,
            test_role: _,
        } = f;
        Self {
            id: id.as_u64(),
            path: path.clone(),
            language: language.clone(),
            detected_via: detected_via.clone(),
            blake3: blake3.clone(),
            size_bytes: *size_bytes,
            lines: *lines,
            parse_status: parse_status.clone(),
        }
    }
}

#[pymethods]
impl File {
    fn __repr__(&self) -> String {
        format!(
            "File(id={}, path={:?}, language={:?}, lines={})",
            self.id, self.path, self.language, self.lines
        )
    }
}

/// Run-level counters.
///
/// These describe the **whole analysis**, not the graph you got back, so
/// on a filtered run `callables` legitimately exceeds
/// `len(graph.callables)`.
#[pyclass(frozen, module = "cgg", skip_from_py_object)]
#[derive(Clone)]
pub struct Metrics {
    #[pyo3(get)]
    files_discovered: u64,
    #[pyo3(get)]
    files_analyzed: u64,
    #[pyo3(get)]
    files_skipped: u64,
    #[pyo3(get)]
    files_errored: u64,
    #[pyo3(get)]
    callables: u64,
    #[pyo3(get)]
    edges: u64,
    #[pyo3(get)]
    cross_file_edges: u64,
    #[pyo3(get)]
    unresolved_calls: u64,
    #[pyo3(get)]
    stdlib_calls: u64,
    #[pyo3(get)]
    external_calls: u64,
    #[pyo3(get)]
    bytes_processed: u64,
    #[pyo3(get)]
    wall_ms: f64,
}

#[pymethods]
impl Metrics {
    fn __repr__(&self) -> String {
        format!(
            "Metrics(files_analyzed={}, callables={}, edges={}, wall_ms={:.1})",
            self.files_analyzed, self.callables, self.edges, self.wall_ms
        )
    }
}

// --- Graph -------------------------------------------------------------

/// The result of an analysis.
///
/// Attribute access materializes Python objects on first use and caches
/// them; the renderers materialize nothing.
#[pyclass(module = "cgg")]
pub struct Graph {
    inner: Arc<cgg_core::graph::Graph>,
    metrics: Metrics,
    notices: Vec<String>,
    jobs: usize,

    callables_cache: OnceLock<Py<PyTuple>>,
    edges_cache: OnceLock<Py<PyTuple>>,
    files_cache: OnceLock<Py<PyTuple>>,

    /// Built on first traversal, never at construction — the renderers are
    /// the common case and pay nothing for these.
    ///
    /// Without them `callers_of`/`callees_of` are O(E) and `callable()` is
    /// O(N), so the obvious `for c in g.callables: g.callers_of(c)` is
    /// O(N*E): 8.6M edge visits on cgg's own graph, ~10^10 on a large one.
    by_name: OnceLock<HashMap<String, u64>>,
    adjacency: OnceLock<Adjacency>,
}

/// Inbound and outbound neighbours per callable id.
#[derive(Default)]
struct Adjacency {
    callers: HashMap<u64, Vec<u64>>,
    callees: HashMap<u64, Vec<u64>>,
}

impl Graph {
    fn from_outcome(o: cgg::RunOutcome) -> Self {
        let m = &o.metrics;
        let metrics = Metrics {
            files_discovered: m.files_discovered,
            files_analyzed: m.files_analyzed,
            files_skipped: m.files_skipped,
            files_errored: m.files_errored,
            callables: m.callables,
            edges: m.edges,
            cross_file_edges: o.cross_file_edges,
            unresolved_calls: m.unresolved_calls,
            stdlib_calls: m.stdlib_calls,
            external_calls: m.external_calls,
            bytes_processed: m.bytes_processed,
            wall_ms: m.wall_ms,
        };
        // `notices()` is the derived diagnostics-only view of the
        // transcript, already stripped of trailing newlines.
        let notices = o.notices().map(str::to_string).collect();
        Self {
            inner: Arc::new(o.graph),
            metrics,
            notices,
            jobs: o.jobs,
            callables_cache: OnceLock::new(),
            edges_cache: OnceLock::new(),
            files_cache: OnceLock::new(),
            by_name: OnceLock::new(),
            adjacency: OnceLock::new(),
        }
    }

    fn by_name(&self) -> &HashMap<String, u64> {
        self.by_name.get_or_init(|| {
            self.inner
                .callables
                .values()
                .map(|c| (c.qualified_name.clone(), c.id.as_u64()))
                .collect()
        })
    }

    fn adjacency(&self) -> &Adjacency {
        self.adjacency.get_or_init(|| {
            let mut a = Adjacency::default();
            for e in &self.inner.edges {
                a.callees
                    .entry(e.src.as_u64())
                    .or_default()
                    .push(e.dst.as_u64());
                a.callers
                    .entry(e.dst.as_u64())
                    .or_default()
                    .push(e.src.as_u64());
            }
            a
        })
    }

    /// Callables for a list of ids.
    ///
    /// Sorted by position in the graph and deduplicated. Ids are
    /// content-derived hashes now, not sequential indices, so sorting by
    /// *value* would no longer restore graph order — instead we sort by
    /// each id's position in `self.inner.callables` (an `IndexMap`, so
    /// that lookup is O(1) and the order it reports is insertion/analysis
    /// order). Dedup means two call sites from one caller yield one entry
    /// rather than two.
    fn nodes_for(&self, ids: Option<&Vec<u64>>) -> Vec<Callable> {
        let Some(ids) = ids else { return Vec::new() };
        let mut ids = ids.clone();
        ids.sort_unstable();
        ids.dedup();
        ids.sort_unstable_by_key(|id| {
            self.inner
                .callables
                .get_index_of(&cgg_core::ids::CallableId::new_u64(*id))
                .unwrap_or(usize::MAX)
        });
        ids.iter()
            .filter_map(|id| {
                self.inner
                    .callables
                    .get(&cgg_core::ids::CallableId::new_u64(*id))
            })
            .map(Callable::from_node)
            .collect()
    }

    /// Build-and-cache. A lost race builds twice and discards one copy,
    /// cheaper than holding a lock across allocation.
    fn cached<'py, T, I>(
        cache: &OnceLock<Py<PyTuple>>,
        py: Python<'py>,
        items: I,
    ) -> PyResult<Py<PyTuple>>
    where
        I: IntoIterator<Item = T>,
        I::IntoIter: ExactSizeIterator,
        T: IntoPyObject<'py>,
    {
        if let Some(t) = cache.get() {
            return Ok(t.clone_ref(py));
        }
        let tuple = PyTuple::new(py, items)?.unbind();
        let _ = cache.set(tuple.clone_ref(py));
        Ok(tuple)
    }
}

#[pymethods]
impl Graph {
    /// Every callable, in analysis order.
    #[getter]
    fn callables(&self, py: Python<'_>) -> PyResult<Py<PyTuple>> {
        Self::cached(
            &self.callables_cache,
            py,
            self.inner.callables.values().map(Callable::from_node),
        )
    }

    /// Every edge, in analysis order.
    #[getter]
    fn edges(&self, py: Python<'_>) -> PyResult<Py<PyTuple>> {
        Self::cached(
            &self.edges_cache,
            py,
            self.inner.edges.iter().map(Edge::from_edge),
        )
    }

    /// Every analyzed file.
    #[getter]
    fn files(&self, py: Python<'_>) -> PyResult<Py<PyTuple>> {
        Self::cached(
            &self.files_cache,
            py,
            self.inner.files.values().map(File::from_record),
        )
    }

    #[getter]
    fn metrics(&self) -> Metrics {
        self.metrics.clone()
    }

    /// The diagnostics the CLI would have written to stderr, in order,
    /// without their trailing newlines.
    #[getter]
    fn notices(&self) -> Vec<String> {
        self.notices.clone()
    }

    /// Worker threads the analysis actually ran on.
    #[getter]
    fn jobs(&self) -> usize {
        self.jobs
    }

    /// Callable count. `len(graph)` is the same number.
    fn __len__(&self) -> usize {
        self.inner.callables.len()
    }

    fn __repr__(&self) -> String {
        format!(
            "<cgg.Graph {} callables, {} edges, {} files>",
            self.inner.callables.len(),
            self.inner.edges.len(),
            self.inner.files.len()
        )
    }

    /// Mermaid `flowchart`. The default, and what agents read.
    fn to_mermaid(&self, py: Python<'_>) -> String {
        py.detach(|| cgg::emit::graph_to_string(&self.inner, OutputFormat::Mermaid))
    }

    /// The full graph as JSON — byte-identical to `cgg -t json`.
    fn to_json(&self, py: Python<'_>) -> String {
        py.detach(|| cgg::emit::graph_to_string(&self.inner, OutputFormat::Json))
    }

    /// Graphviz DOT.
    fn to_dot(&self, py: Python<'_>) -> String {
        py.detach(|| cgg::emit::graph_to_string(&self.inner, OutputFormat::Dot))
    }

    /// GraphML, for Gephi / yEd / networkx.
    fn to_graphml(&self, py: Python<'_>) -> String {
        py.detach(|| cgg::emit::graph_to_string(&self.inner, OutputFormat::Graphml))
    }

    /// The graph as a plain `dict` — the escape hatch for anything the
    /// typed attributes omit. Goes through JSON rather than a hand-written
    /// converter so it cannot disagree with `to_json()`.
    fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let json = self.to_json(py);
        let loads = py.import("json")?.getattr("loads")?;
        Ok(loads.call1((json,))?.unbind())
    }

    /// The callable with this exact qualified name, or `None`.
    fn callable(&self, qualified_name: &str) -> Option<Callable> {
        let id = *self.by_name().get(qualified_name)?;
        self.inner
            .callables
            .get(&cgg_core::ids::CallableId::new_u64(id))
            .map(Callable::from_node)
    }

    /// Callables that call `target`, a qualified name or a `Callable`.
    ///
    /// `[]` for an unknown name — which is also what a real callable with
    /// no callers gives, so use `callable()` to tell them apart.
    fn callers_of(&self, target: &Bound<'_, PyAny>) -> PyResult<Vec<Callable>> {
        let id = self.resolve_id(target)?;
        Ok(self.nodes_for(self.adjacency().callers.get(&id)))
    }

    /// Callables that `target` calls. Same conventions as `callers_of`.
    fn callees_of(&self, target: &Bound<'_, PyAny>) -> PyResult<Vec<Callable>> {
        let id = self.resolve_id(target)?;
        Ok(self.nodes_for(self.adjacency().callees.get(&id)))
    }
}

impl Graph {
    /// Accept a `Callable` or a qualified name.
    fn resolve_id(&self, target: &Bound<'_, PyAny>) -> PyResult<u64> {
        // `cast` + `get`, not `extract`: the class is `frozen`, so this
        // reads the id straight out of the cell instead of cloning five
        // `String`s and a `Vec` to throw them away.
        if let Ok(c) = target.cast::<Callable>() {
            return Ok(c.get().id);
        }
        if let Ok(name) = target.extract::<String>() {
            // An unmatched name yields an unmatched id, so the caller gets
            // `[]` rather than an exception. Ids are content hashes now
            // rather than a sequential counter starting at 0, but they
            // are drawn from at most 64 bits, so u64::MAX is
            // vanishingly unlikely ever to be minted for a real node.
            return Ok(self.by_name().get(&name).copied().unwrap_or(u64::MAX));
        }
        Err(PyValueError::new_err(
            "expected a Callable or a qualified name",
        ))
    }
}

// --- analyze -----------------------------------------------------------

/// `paths` may be one path or an iterable of them. `PathBuf` first: it
/// covers `str` and `os.PathLike`, and a `str` would otherwise iterate as
/// one-character paths.
fn extract_paths(paths: &Bound<'_, PyAny>) -> PyResult<Vec<PathBuf>> {
    if let Ok(p) = paths.extract::<PathBuf>() {
        return Ok(vec![p]);
    }
    if let Ok(v) = paths.extract::<Vec<PathBuf>>() {
        return Ok(v);
    }
    Err(PyValueError::new_err(
        "paths must be a str, an os.PathLike, or an iterable of them",
    ))
}

fn confidence_from_str(s: &str) -> PyResult<cgg_core::graph::Confidence> {
    match s {
        "high" => Ok(cgg_core::graph::Confidence::High),
        "medium" => Ok(cgg_core::graph::Confidence::Medium),
        "low" => Ok(cgg_core::graph::Confidence::Low),
        other => Err(PyValueError::new_err(format!(
            "dead_code_confidence must be 'high', 'medium' or 'low', got {other:?}"
        ))),
    }
}

/// Analyze one or more source trees and return the call graph.
///
/// Mirrors the CLI flag-for-flag, with one rename: `entry_nodes=True`
/// rather than `--no-entry-nodes`. Same default, no double negative.
#[pyfunction]
#[pyo3(signature = (
    paths,
    *,
    filter = None,
    hops = -1,
    max_paths = 1000,
    fanout_cap = cgg::cross_file_default_fanout_cap(),
    report_unreferenced = false,
    exclude_partial = None,
    exclude_glob = None,
    exclude_regex = None,
    lang = None,
    jobs = 0,
    ignore_file = None,
    include_external = false,
    include_stdlib = false,
    dynamic_dispatch = false,
    reference_edges = false,
    entry_nodes = true,
    include_tests = false,
    dead_code = false,
    dead_code_confidence = "high",
    ignore_names = None,
    ignore_attributes = None,
    roots = None,
    since = None,
))]
#[allow(clippy::too_many_arguments)]
fn analyze(
    py: Python<'_>,
    paths: &Bound<'_, PyAny>,
    filter: Option<Vec<String>>,
    hops: i32,
    max_paths: u32,
    fanout_cap: u32,
    report_unreferenced: bool,
    exclude_partial: Option<Vec<String>>,
    exclude_glob: Option<Vec<String>>,
    exclude_regex: Option<Vec<String>>,
    lang: Option<Vec<String>>,
    jobs: usize,
    ignore_file: Option<PathBuf>,
    include_external: bool,
    include_stdlib: bool,
    dynamic_dispatch: bool,
    reference_edges: bool,
    entry_nodes: bool,
    include_tests: bool,
    dead_code: bool,
    dead_code_confidence: &str,
    ignore_names: Option<Vec<String>>,
    ignore_attributes: Option<Vec<String>>,
    roots: Option<PathBuf>,
    since: Option<String>,
) -> PyResult<Graph> {
    let opts = cgg::RunOptions {
        paths: extract_paths(paths)?,
        filter: filter.unwrap_or_default(),
        since,
        exclude_partial: exclude_partial.unwrap_or_default(),
        exclude_glob: exclude_glob.unwrap_or_default(),
        exclude_regex: exclude_regex.unwrap_or_default(),
        hops,
        max_paths,
        fanout_cap,
        report_unreferenced,
        ignore_file,
        lang: lang.unwrap_or_default(),
        jobs,
        include_external,
        include_stdlib,
        dynamic_dispatch,
        reference_edges,
        // The one inversion, done here so it is done exactly once.
        no_entry_nodes: !entry_nodes,
        // The detail flag only widens a notice the CLI prints; the data
        // itself is on the outcome regardless.
        framework_coverage: false,
        dead_code,
        dead_code_confidence: confidence_from_str(dead_code_confidence)?,
        ignore_names: ignore_names.unwrap_or_default(),
        ignore_attributes: ignore_attributes.unwrap_or_default(),
        roots,
        // Neither belongs in a library call: one produces a config file
        // instead of a graph, the other answers a different question.
        write_roots: false,
        why_live: Vec::new(),
        include_tests,
        // Process-global registry: in a long-lived interpreter it would
        // report the sum of every analysis so far.
        profile: false,
    };

    // Pure Rust, touching no Python object: without releasing the GIL a
    // multi-second parse would freeze every other thread.
    let outcome = py.detach(|| cgg::analyze(&opts)).map_err(to_py_err)?;

    Ok(Graph::from_outcome(outcome))
}

/// The language ids cgg can analyze, sorted. Read off the registry.
#[pyfunction]
fn languages() -> Vec<&'static str> {
    let mut ids = cgg::plugin_ids();
    ids.sort_unstable();
    ids
}

#[pymodule]
fn _cgg(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", cgg_core::CGG_VERSION)?;
    m.add("CggError", m.py().get_type::<CggError>())?;
    m.add_class::<Graph>()?;
    m.add_class::<Callable>()?;
    m.add_class::<Edge>()?;
    m.add_class::<File>()?;
    m.add_class::<Metrics>()?;
    m.add_function(wrap_pyfunction!(analyze, m)?)?;
    m.add_function(wrap_pyfunction!(languages, m)?)?;
    Ok(())
}
