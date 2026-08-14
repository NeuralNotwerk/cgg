//! What [`crate::analyze`] returns.
//!
//! Everything a `cgg` run writes to a file descriptor is a field here
//! instead, so `analyze` itself performs no I/O.

use cgg_core::audit::{AuditEvent, RunMetrics};
use cgg_core::deadcode::{DeadCodeReport, LivenessProof};
use cgg_core::frameworks::FrameworkCoverage;
use cgg_core::graph::{Confidence, Graph};

/// One thing a run writes, in the order it writes it.
///
/// Diagnostics and artifacts share one list because their *relative order*
/// is observable: without `-o` the graph goes to stdout while diagnostics
/// go to stderr, and the graph belongs in the middle of the stream — after
/// the `--max-paths` warning, before the run summary. Anyone running
/// `cgg ./src 2>&1 | less` sees the difference.
///
/// The artifact is an element rather than a marker paired with a payload
/// field, which makes "a position with no payload" unrepresentable instead
/// of merely untested. An earlier design had `Primary` as a bare marker
/// with the payload alongside it; forgetting the push produced a run that
/// wrote no graph, exited 0, and printed a summary claiming N callables.
///
/// [`Emission::Diagnostic`] carries verbatim bytes rather than a structured
/// `{ level, message }` record: `scripts/update-readme-stats.py` parses the
/// run-summary line on every commit, so a refactor must not be able to
/// reflow it.
#[derive(Debug, Clone)]
pub enum Emission {
    /// Verbatim bytes for stderr.
    Diagnostic {
        /// Exact bytes, including the trailing newline.
        text: String,
        /// Whether `-q` suppresses this one. Failures are returned as
        /// `Err`, never as diagnostics, so `--quiet` cannot hide one.
        quiet: bool,
    },
    /// The call graph, rendered with whatever `-t` selected.
    Graph,
    /// `--report-unreferenced` findings, which replace the graph.
    Unreferenced(Vec<UnreferencedFinding>),
    /// `--why-live` liveness proofs, which replace the graph.
    WhyLive(Vec<LivenessProof>),
    /// `--write-roots` baseline config, already rendered as TOML.
    RootsBaseline(String),
    /// The detailed dead-code report.
    ///
    /// Payload-free because the report itself is on
    /// [`RunOutcome::dead_code`] for library callers, and *where* it lands
    /// — a sidecar beside `-o`, or stderr — is an I/O decision.
    DeadCodeReport,
    /// The audit sidecar.
    ///
    /// Explicit rather than a side effect of writing the primary artifact.
    /// That is what makes "`--write-roots` writes no audit" a fact about
    /// what `analyze` pushed, rather than a special case in the emitter
    /// justified by what the pre-split code happened to do.
    Audit,
}

impl Emission {
    /// A stderr line that `-q` suppresses. Almost all of them.
    pub fn line(text: impl Into<String>) -> Self {
        Self::Diagnostic {
            text: text.into(),
            quiet: true,
        }
    }

    /// A stderr line printed even under `-q`.
    pub fn always(text: impl Into<String>) -> Self {
        Self::Diagnostic {
            text: text.into(),
            quiet: false,
        }
    }

    /// The bytes, if this is a diagnostic rather than an artifact.
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Diagnostic { text, .. } => Some(text),
            _ => None,
        }
    }
}

/// The complete result of an analysis.
#[derive(Debug)]
pub struct RunOutcome {
    /// Post-query, post-exclusions: what the formatters would render.
    /// Present even when the transcript's artifact is not [`Emission::Graph`].
    pub graph: Graph,

    /// Everything the run writes, in order. See [`Emission`].
    pub transcript: Vec<Emission>,

    /// Audit event stream. The `.audit.json` sidecar is this, serialised.
    pub events: Vec<AuditEvent>,

    /// Whole-analysis counters, not the post-query subgraph — so these can
    /// exceed `graph.callables.len()` on a filtered run.
    pub metrics: RunMetrics,

    /// Frameworks recognised, and those seen but not understood.
    ///
    /// Nothing in this repo reads it — the CLI builds its table from the
    /// local binding and the audit event takes its own clone. It is here
    /// for library callers: the gap list is the only way to learn which
    /// frameworks cgg could *not* enumerate, and `crates/cgg-py` has it
    /// listed as a deferred addition.
    pub framework_coverage: FrameworkCoverage,

    /// The dead-code findings, when dead-code mode ran.
    pub dead_code: Option<DeadCodeReport>,

    /// Findings at or above the threshold, and so marked on the graph.
    /// Drives `--fail-on-dead`; `dead_code.findings.len()` counts all
    /// bands.
    pub dead_code_marked: usize,

    /// The threshold the analysis actually applied.
    ///
    /// Here rather than re-derived from `Cli` by the emitter: a library
    /// caller sets it on [`crate::RunOptions`] and has no `Cli`, so
    /// re-deriving would let the graph annotation and the report header
    /// disagree with nothing to catch it.
    pub dead_code_threshold: Confidence,

    /// Inter-file edges, computed pre-query to stay consistent with
    /// `metrics.edges`.
    pub cross_file_edges: u64,

    /// Worker threads actually used, read inside the pool rather than
    /// echoed back from [`crate::RunOptions::jobs`]. A knob that cannot be
    /// observed cannot be tested.
    pub jobs: usize,
}

impl RunOutcome {
    /// The diagnostic lines only, in order, without their trailing
    /// newlines.
    ///
    /// A derived view rather than a stored field: dropping the artifact
    /// entries is then an intent stated here once, instead of a filter
    /// every front end has to remember to write.
    pub fn notices(&self) -> impl Iterator<Item = &str> {
        self.transcript
            .iter()
            .filter_map(|e| e.text())
            .map(|t| t.trim_end_matches('\n'))
    }
}

/// One callable that nothing in the analyzed tree points at.
#[derive(Clone, Debug)]
pub struct UnreferencedFinding {
    pub qualified_name: String,
    pub path: String,
    pub start_line: u32,
    /// The root rule that explains it, when cgg has one. `None` is the
    /// finding; `Some` is the bucket that explains itself away.
    pub root: Option<String>,
}
