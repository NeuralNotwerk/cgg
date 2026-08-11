//! Node.js bindings for cgg, over N-API.
//!
//! A translation layer over [`cgg::analyze`] with no analysis logic — the
//! same contract `crates/cgg-py` and `crates/cgg-ffi` hold. There is one
//! pipeline and one resolver ordering; this crate only moves values across
//! the boundary.
//!
//! A native module rather than a wrapper over the C ABI: npm needs a
//! per-platform artifact either way, so the C ABI would buy nothing here
//! while costing an FFI dependency and a slower boundary.
//!
//! The analysis releases the JS thread — `analyze` is exposed as an async
//! task, so a server calling it does not stall its event loop for the
//! ~100ms+ a real tree takes.

use std::sync::Arc;

use napi::bindgen_prelude::*;
use napi_derive::napi;

use cgg_format::OutputFormat;

/// See `crates/cgg/src/main.rs` — extraction is allocation-heavy and the
/// system allocator serialises under it.
#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Options that change the graph.
///
/// Every field is optional and defaults to [`cgg::RunOptions`]'s default,
/// so `analyze(path)` is the whole API for the common case. Named as
/// JavaScript would name them; `napi` maps camelCase to these.
///
/// One rename from the CLI, matching the Python module: `entryNodes: true`
/// rather than `--no-entry-nodes`. Same default; a keyword has no reason
/// to be a double negative.
#[napi(object)]
#[derive(Default)]
pub struct AnalyzeOptions {
    pub filter: Option<Vec<String>>,
    pub hops: Option<i32>,
    pub max_paths: Option<u32>,
    pub exclude_partial: Option<Vec<String>>,
    pub exclude_glob: Option<Vec<String>>,
    pub exclude_regex: Option<Vec<String>>,
    pub lang: Option<Vec<String>>,
    pub jobs: Option<u32>,
    pub ignore_file: Option<String>,
    pub include_external: Option<bool>,
    pub include_stdlib: Option<bool>,
    pub dynamic_dispatch: Option<bool>,
    pub reference_edges: Option<bool>,
    /// `true` (the default) mints `<framework-entry>` nodes.
    pub entry_nodes: Option<bool>,
    pub include_tests: Option<bool>,
    pub dead_code: Option<bool>,
    /// `"high"` (default), `"medium"` or `"low"`.
    pub dead_code_confidence: Option<String>,
    pub ignore_names: Option<Vec<String>>,
    pub ignore_attributes: Option<String>,
    pub roots: Option<String>,
    pub since: Option<String>,
}

/// One callable in the graph.
#[napi(object)]
pub struct Callable {
    pub id: u32,
    pub qualified_name: String,
    pub simple_name: String,
    pub kind: String,
    pub language: String,
    /// Index into [`Graph::files`], not a path.
    pub file: u32,
    pub start_line: u32,
    pub end_line: u32,
    pub signature_hint: String,
    pub visibility: String,
    pub synthetic: bool,
    /// Set when cgg found no caller. **Best effort** — a hypothesis, not
    /// a proof.
    pub unreferenced: Option<String>,
}

/// One resolved call edge.
///
/// Field-for-field the same as the Python module's `Edge`, deliberately:
/// two bindings over one pipeline should not describe the same graph with
/// different words.
#[napi(object)]
pub struct Edge {
    pub src: u32,
    pub dst: u32,
    pub site_line: u32,
    pub site_byte: u32,
    /// `"high"`, `"medium"` or `"low"`.
    pub confidence: String,
    /// How the edge was established — `"direct"`, `"dynamic"`,
    /// `"reference"`, `"external"`, `"stdlib"`, `"ffi"`, `"descriptor"`
    /// or `"framework_entry"`. Filter on this to keep only edges you trust.
    pub via: String,
}

/// Whole-run counters. Not the post-query subgraph, so these can exceed
/// `callables.length` on a filtered run.
#[napi(object)]
pub struct Metrics {
    pub files_discovered: u32,
    pub files_analyzed: u32,
    pub files_skipped: u32,
    pub callables: u32,
    pub edges: u32,
    pub unresolved_calls: u32,
    pub stdlib_calls: u32,
    pub external_calls: u32,
    pub wall_ms: f64,
}

fn confidence_str(c: cgg::Confidence) -> &'static str {
    // An exhaustive match, not `format!("{:?}")`: a new variant becomes a
    // compile error here instead of an unexpected string reaching JS.
    match c {
        cgg::Confidence::High => "high",
        cgg::Confidence::Medium => "medium",
        cgg::Confidence::Low => "low",
    }
}

fn via_str(v: &cgg::Via) -> &'static str {
    match v {
        cgg::Via::Direct => "direct",
        cgg::Via::Dynamic => "dynamic",
        cgg::Via::Reference => "reference",
        cgg::Via::External => "external",
        cgg::Via::Stdlib => "stdlib",
        // Tuple variants: the payload is the FFI kind / descriptor / entry
        // name, which the graph carries but the JS view does not need.
        cgg::Via::Ffi(_) => "ffi",
        cgg::Via::Descriptor(_) => "descriptor",
        cgg::Via::FrameworkEntry(_) => "framework_entry",
    }
}

fn kind_str(k: cgg::CallableKind) -> &'static str {
    match k {
        cgg::CallableKind::Function => "function",
        cgg::CallableKind::Method => "method",
        cgg::CallableKind::Constructor => "constructor",
        cgg::CallableKind::Destructor => "destructor",
        cgg::CallableKind::Closure => "closure",
        cgg::CallableKind::Property => "property",
    }
}

fn build_options(
    paths: Vec<String>,
    o: Option<AnalyzeOptions>,
) -> Result<cgg::RunOptions> {
    let o = o.unwrap_or_default();
    let confidence = match o.dead_code_confidence.as_deref() {
        None | Some("high") => cgg::Confidence::High,
        Some("medium") => cgg::Confidence::Medium,
        Some("low") => cgg::Confidence::Low,
        Some(other) => {
            return Err(Error::new(
                Status::InvalidArg,
                format!(
                    "deadCodeConfidence must be \"high\", \"medium\" or \"low\", got {other:?}"
                ),
            ));
        }
    };
    let d = cgg::RunOptions::default();
    Ok(cgg::RunOptions {
        paths: paths.into_iter().map(Into::into).collect(),
        filter: o.filter.unwrap_or_default(),
        hops: o.hops.unwrap_or(d.hops),
        max_paths: o.max_paths.unwrap_or(d.max_paths),
        exclude_partial: o.exclude_partial.unwrap_or_default(),
        exclude_glob: o.exclude_glob.unwrap_or_default(),
        exclude_regex: o.exclude_regex.unwrap_or_default(),
        lang: o.lang.unwrap_or_default(),
        jobs: o.jobs.unwrap_or(0) as usize,
        ignore_file: o.ignore_file.map(Into::into),
        include_external: o.include_external.unwrap_or(false),
        include_stdlib: o.include_stdlib.unwrap_or(false),
        dynamic_dispatch: o.dynamic_dispatch.unwrap_or(false),
        reference_edges: o.reference_edges.unwrap_or(false),
        // Inverted on purpose; see `AnalyzeOptions::entry_nodes`.
        no_entry_nodes: !o.entry_nodes.unwrap_or(true),
        include_tests: o.include_tests.unwrap_or(false),
        dead_code: o.dead_code.unwrap_or(false),
        dead_code_confidence: confidence,
        ignore_names: o.ignore_names.unwrap_or_default(),
        ignore_attributes: o.ignore_attributes.into_iter().collect(),
        roots: o.roots.map(Into::into),
        since: o.since,
        ..d
    })
}

/// An analyzed call graph.
///
/// Holds the whole outcome behind an `Arc` and materializes JavaScript
/// values only when asked. The renderers materialize none, so
/// `toMermaid()` on a large graph does not pay to build one JS object per
/// callable first.
#[napi]
pub struct Graph {
    outcome: Arc<cgg::RunOutcome>,
}

#[napi]
impl Graph {
    /// Callables in the graph.
    #[napi(getter)]
    pub fn callables(&self) -> Vec<Callable> {
        self.outcome
            .graph
            .callables
            .values()
            .map(|c| Callable {
                id: c.id.as_u32(),
                qualified_name: c.qualified_name.clone(),
                simple_name: c.simple_name.clone(),
                kind: kind_str(c.kind).to_string(),
                language: c.language.clone(),
                file: c.file.as_u32(),
                start_line: c.start_line,
                end_line: c.end_line,
                signature_hint: c.signature_hint.clone(),
                // Already a String; `format!("{:?}")` would wrap it in quotes.
                visibility: c.visibility.clone(),
                synthetic: c.synthetic,
                unreferenced: c.unreferenced.map(|u| confidence_str(u).to_string()),
            })
            .collect()
    }

    /// Resolved call edges.
    #[napi(getter)]
    pub fn edges(&self) -> Vec<Edge> {
        self.outcome
            .graph
            .edges
            .iter()
            .map(|e| Edge {
                src: e.src.as_u32(),
                dst: e.dst.as_u32(),
                site_line: e.site_line,
                site_byte: e.site_byte,
                confidence: confidence_str(e.confidence).to_string(),
                via: via_str(&e.via).to_string(),
            })
            .collect()
    }

    /// Analyzed file paths, indexed by `Callable.file`.
    #[napi(getter)]
    pub fn files(&self) -> Vec<String> {
        self.outcome
            .graph
            .files
            .values()
            .map(|f| f.path.display().to_string())
            .collect()
    }

    /// Whole-run counters.
    #[napi(getter)]
    pub fn metrics(&self) -> Metrics {
        let m = &self.outcome.metrics;
        Metrics {
            files_discovered: m.files_discovered as u32,
            files_analyzed: m.files_analyzed as u32,
            files_skipped: m.files_skipped as u32,
            callables: m.callables as u32,
            edges: m.edges as u32,
            unresolved_calls: m.unresolved_calls as u32,
            stdlib_calls: m.stdlib_calls as u32,
            external_calls: m.external_calls as u32,
            wall_ms: m.wall_ms,
        }
    }

    /// The diagnostics the CLI would print to stderr, in order.
    #[napi(getter)]
    pub fn notices(&self) -> Vec<String> {
        self.outcome.notices().map(str::to_string).collect()
    }

    /// Worker threads the run actually used.
    #[napi(getter)]
    pub fn jobs(&self) -> u32 {
        self.outcome.jobs as u32
    }

    /// Number of callables, without materializing them.
    #[napi(getter)]
    pub fn callable_count(&self) -> u32 {
        self.outcome.graph.callables.len() as u32
    }

    /// Render as a mermaid flowchart — what the CLI emits by default.
    #[napi]
    pub fn to_mermaid(&self) -> String {
        cgg::emit::graph_to_string(&self.outcome.graph, OutputFormat::Mermaid)
    }

    /// Render as `cgg.graph.v1` JSON, byte-identical to `cgg -t json`.
    #[napi]
    pub fn to_json(&self) -> String {
        cgg::emit::graph_to_string(&self.outcome.graph, OutputFormat::Json)
    }

    /// Render as Graphviz DOT.
    #[napi]
    pub fn to_dot(&self) -> String {
        cgg::emit::graph_to_string(&self.outcome.graph, OutputFormat::Dot)
    }

    /// Render as GraphML.
    #[napi]
    pub fn to_graphml(&self) -> String {
        cgg::emit::graph_to_string(&self.outcome.graph, OutputFormat::Graphml)
    }
}

/// Analyze a source tree.
///
/// Accepts one path or several. Returns a promise: the analysis runs on
/// libuv's thread pool, not the JS thread, so an event loop stays
/// responsive for the ~100ms+ a real tree costs.
///
/// ```js
/// const cgg = require("cgg-callgraphgenerator");
/// const g = await cgg.analyze("./src");
/// console.log(g.toMermaid());
/// ```
#[napi]
pub async fn analyze(
    paths: Either<String, Vec<String>>,
    options: Option<AnalyzeOptions>,
) -> Result<Graph> {
    let paths = match paths {
        Either::A(p) => vec![p],
        Either::B(v) => v,
    };
    let opts = build_options(paths, options)?;
    // The pipeline is synchronous and CPU-bound; run it off the JS thread.
    let outcome = napi::tokio::task::spawn_blocking(move || cgg::analyze(&opts))
        .await
        .map_err(|e| {
            Error::new(Status::GenericFailure, format!("analysis panicked: {e}"))
        })?
        // `{:#}` so the whole anyhow context chain crosses, not just the
        // outermost message.
        .map_err(|e| Error::new(Status::GenericFailure, format!("{e:#}")))?;
    Ok(Graph {
        outcome: Arc::new(outcome),
    })
}

/// Analyze synchronously. Blocks the JS thread — prefer [`analyze`].
///
/// Here for scripts and CLIs, where blocking is what you want and a
/// promise is ceremony.
#[napi]
pub fn analyze_sync(
    paths: Either<String, Vec<String>>,
    options: Option<AnalyzeOptions>,
) -> Result<Graph> {
    let paths = match paths {
        Either::A(p) => vec![p],
        Either::B(v) => v,
    };
    let opts = build_options(paths, options)?;
    let outcome = cgg::analyze(&opts)
        .map_err(|e| Error::new(Status::GenericFailure, format!("{e:#}")))?;
    Ok(Graph {
        outcome: Arc::new(outcome),
    })
}

/// The cgg version this module was built from.
#[napi]
pub fn version() -> String {
    cgg_core::CGG_VERSION.to_string()
}

/// Every language id cgg can analyze, in registry order.
#[napi]
pub fn languages() -> Vec<String> {
    cgg::plugin_ids().into_iter().map(str::to_string).collect()
}
