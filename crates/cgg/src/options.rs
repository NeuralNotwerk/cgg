//! Analysis options, decoupled from the command line.
//!
//! [`RunOptions`] is what [`crate::analyze`] takes. It carries only the
//! knobs that change what the graph *contains*; everything about where
//! bytes go — `-o`, `-t`, `--metrics`, `--audit-format`, `-q`, `-v`,
//! `--dead-code-report` — stays on [`crate::cli::Cli`] and is consumed
//! by [`crate::emit`].
//!
//! Anything here is a question about the analysis, so both front ends
//! need it; anything left on `Cli` is a question about I/O.

use std::path::PathBuf;

use cgg_core::graph::Confidence;

/// Everything [`crate::analyze`] needs to decide what the graph contains.
///
/// `Default` mirrors the clap defaults exactly — `hops: -1`,
/// `max_paths: 1000`, `dead_code_confidence: High`, everything else
/// empty or off. `crates/cgg/tests/lib_api.rs` asserts that equivalence
/// against a freshly parsed bare command line, so the two cannot drift.
/// `serde` so the options can cross a boundary that has no Rust types:
/// `crates/cgg-ffi` takes them as a JSON document, which is what lets one
/// C ABI serve every language without gaining a function per flag. Every
/// field is `#[serde(default)]` via the container attribute, so a caller
/// sends only what it wants to change and a *new* field never breaks a
/// caller that predates it.
///
/// `deny_unknown_fields` on purpose: a typo'd key in a hand-written JSON
/// options blob would otherwise be silently ignored, which is the same
/// class of failure as a framework rule naming a verb cgg does not ship.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RunOptions {
    /// Source directories or files to analyze. Must be non-empty.
    pub paths: Vec<PathBuf>,

    /// Callable-name patterns to keep. Regex, or `glob:` prefixed.
    pub filter: Vec<String>,
    /// Git revspec whose changed callables are appended to `filter`.
    pub since: Option<String>,
    /// Drop callables whose qualified name contains this substring.
    pub exclude_partial: Vec<String>,
    /// Drop callables whose qualified name matches this glob.
    pub exclude_glob: Vec<String>,
    /// Drop callables whose qualified name matches this regex.
    pub exclude_regex: Vec<String>,
    /// Neighborhood depth around each `filter` match. `-1` means no
    /// query at all (emit everything); `0` enumerates full call paths.
    pub hops: i32,
    /// Cap on per-match path count in `hops == 0` mode.
    pub max_paths: u32,
    /// Emit the unreferenced-callable report in place of the graph.
    pub report_unreferenced: bool,
    /// Max same-named candidates before a duck-typed method call's
    /// fan-out is dropped. The drop is always recorded.
    pub fanout_cap: u32,

    /// Additional gitignore-syntax ignore file.
    pub ignore_file: Option<PathBuf>,
    /// Restrict analysis to these language ids.
    pub lang: Vec<String>,
    /// Worker threads. `0` means auto (half the physical cores).
    pub jobs: usize,

    /// Add deduplicated leaf nodes for calls into third-party code.
    pub include_external: bool,
    /// Add deduplicated leaf nodes for calls into the standard library.
    pub include_stdlib: bool,
    /// Add over-approximated interface → implementation fan-out edges.
    pub dynamic_dispatch: bool,
    /// Emit edges for functions passed by name as values.
    pub reference_edges: bool,
    /// Suppress synthesized `<framework-entry>` nodes. Negative sense
    /// to match the flag; entry nodes are ON by default.
    pub no_entry_nodes: bool,
    /// Include the full coverage table in the notices even when nothing
    /// was recognised. Affects only the notice text, never the graph or
    /// [`crate::outcome::RunOutcome::framework_coverage`].
    pub framework_coverage: bool,

    /// Report callables nothing appears to call.
    pub dead_code: bool,
    /// Lowest confidence band to report.
    pub dead_code_confidence: Confidence,
    /// Suppress findings whose qualified name matches.
    pub ignore_names: Vec<String>,
    /// Suppress findings on callables carrying a matching attribute.
    pub ignore_attributes: Vec<String>,
    /// Declared-roots config file. `None` triggers the upward search for
    /// `cgg-deadcode.toml` from the analyzed paths.
    pub roots: Option<PathBuf>,
    /// Produce a baseline config accepting every finding, instead of a
    /// graph. Implies `dead_code`.
    pub write_roots: bool,
    /// Explain why these callables are live. Implies `dead_code`.
    pub why_live: Vec<String>,
    /// Show findings that live in test scope.
    pub include_tests: bool,

    /// Collect per-phase timings. `cgg_core::profile`'s registry is
    /// process-global and accumulates, so a long-lived process would
    /// report the sum of every run so far.
    pub profile: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            paths: Vec::new(),
            filter: Vec::new(),
            since: None,
            exclude_partial: Vec::new(),
            exclude_glob: Vec::new(),
            exclude_regex: Vec::new(),
            // -1, not 0: 0 is the "enumerate full paths" mode, so the
            // sentinel for "no query" has to be outside the valid range.
            hops: -1,
            max_paths: 1000,
            report_unreferenced: false,
            fanout_cap: cgg_resolve::cross_file::DEFAULT_FANOUT_CAP as u32,
            ignore_file: None,
            lang: Vec::new(),
            jobs: 0,
            include_external: false,
            include_stdlib: false,
            dynamic_dispatch: false,
            reference_edges: false,
            no_entry_nodes: false,
            framework_coverage: false,
            dead_code: false,
            dead_code_confidence: Confidence::High,
            ignore_names: Vec::new(),
            ignore_attributes: Vec::new(),
            roots: None,
            write_roots: false,
            why_live: Vec::new(),
            include_tests: false,
            profile: false,
        }
    }
}

impl RunOptions {
    /// Options for a plain analysis of `paths`, all defaults otherwise.
    pub fn new(paths: Vec<PathBuf>) -> Self {
        Self {
            paths,
            ..Self::default()
        }
    }

    /// Whether this run is in dead-code mode.
    ///
    /// `--why-live` and `--write-roots` are both questions about the
    /// dead-code model, so they turn it on rather than requiring the
    /// caller to remember `dead_code` as well. `write_roots` without it
    /// used to emit an ordinary graph — a silent no-op wearing the
    /// costume of a baseline.
    pub fn dead_mode(&self) -> bool {
        self.dead_code || !self.why_live.is_empty() || self.write_roots
    }
}

impl From<&crate::cli::Cli> for RunOptions {
    /// Route every `Cli` field to its analysis counterpart, or explicitly
    /// to `_` when it is an I/O concern.
    ///
    /// The pattern below has **no `..` rest**, so adding a field to `Cli`
    /// breaks this function until someone routes it. Field-by-field
    /// `cli.foo` reads would compile while ignoring a new flag.
    fn from(cli: &crate::cli::Cli) -> Self {
        let crate::cli::Cli {
            paths,
            filter,
            since,
            exclude_partial,
            exclude_glob,
            exclude_regex,
            hops,
            max_paths,
            report_unreferenced,
            // Output-shape only: `--no-graph` suppresses the artifact,
            // it does not change the graph, so `RunOptions` — which is
            // strictly what changes the graph — must not carry it.
            no_graph: _,
            fanout_cap,
            include_tests,
            ignore_file,
            jobs,
            lang,
            include_external,
            include_stdlib,
            dynamic_dispatch,
            no_entry_nodes,
            framework_coverage,
            reference_edges,
            dead_code,
            dead_code_confidence,
            ignore_names,
            ignore_attributes,
            roots,
            write_roots,
            why_live,
            profile,

            // --- I/O and presentation: consumed by `crate::emit`. ---
            output: _,
            format: _,
            metrics: _,
            audit_format: _,
            dead_code_format: _,
            dead_code_report: _,
            fail_on_dead: _,
            quiet: _,
            verbose: _,

            // --- Accepted for compatibility, no effect. ---
            stack_graphs: _,
            no_update_check: _,
        } = cli;

        Self {
            paths: paths.clone(),
            filter: filter.clone(),
            since: since.clone(),
            exclude_partial: exclude_partial.clone(),
            exclude_glob: exclude_glob.clone(),
            exclude_regex: exclude_regex.clone(),
            hops: *hops,
            max_paths: *max_paths,
            report_unreferenced: *report_unreferenced,
            fanout_cap: *fanout_cap,
            ignore_file: ignore_file.clone(),
            lang: lang.clone(),
            jobs: *jobs,
            include_external: *include_external,
            include_stdlib: *include_stdlib,
            dynamic_dispatch: *dynamic_dispatch,
            reference_edges: *reference_edges,
            no_entry_nodes: *no_entry_nodes,
            framework_coverage: *framework_coverage,
            dead_code: *dead_code,
            dead_code_confidence: (*dead_code_confidence).into(),
            ignore_names: ignore_names.clone(),
            ignore_attributes: ignore_attributes.clone(),
            roots: roots.clone(),
            write_roots: *write_roots,
            why_live: why_live.clone(),
            include_tests: *include_tests,
            profile: *profile,
        }
    }
}
