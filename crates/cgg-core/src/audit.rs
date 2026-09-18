//! Audit and metrics records.
//!
//! The audit log is the primary trust surface of `cgg`. Every file
//! considered, every file skipped, every callable extracted, every
//! unresolved call, and every FFI site is recorded here. Output
//! delivery is handled by `AuditWriter` implementations:
//!
//! * `JsonAudit`  — pretty json, embedded in the main output when the
//!   chosen format is json, or sidecar otherwise.
//! * `JsonlAudit` — one record per line, suitable for SIEM ingestion.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;

use crate::ids::{CallableId, FileId};

/// Why a discovered path was not analyzed.
///
/// Every variant carries enough context for a security reviewer to
/// retrace the decision without re-running the walker.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind", content = "detail")]
pub enum SkipReason {
    /// File matched a `.gitignore` rule (message may carry the rule line).
    Gitignore(String),
    /// File matched a `.cggignore` rule (carries the absolute path + line).
    Cggignore(String),
    /// File matched one of the built-in deny directories (`"node_modules"`,
    /// `"target"`, …).
    Builtin(String),
    /// Extension is not one of the recognized source extensions for v1.
    UnknownExtension,
    /// File's language IS recognized by a plugin, but that language was
    /// excluded by the `--lang` filter. Payload carries the detected
    /// language id so the runner can suggest the right `--lang` value.
    LanguageFilter(String),
    /// Content was flagged as binary (non-UTF-8 / NUL bytes within the
    /// first N bytes, or declared binary by git attributes).
    Binary,
    /// Symlink resolves to a target outside the scanned roots.
    SymlinkOutsideRoot,
    /// Parse produced no tree or a fatal error.
    ParseError(String),
    /// File size exceeded the configured threshold.
    TooLarge,
    /// A minified JS/CSS build artifact: `name.min.<ext>`, or an
    /// average line length over the walker's minified-source
    /// threshold on a minifiable extension. Parsing it burns wall time
    /// for a graph with no useful callable structure.
    Minified,
    /// The parsed syntax tree nests deeper than the extractor's depth
    /// cap. The per-language extractors are recursive descent, so a tree
    /// deeper than the worker stack can hold would overflow and **abort
    /// the process** — uncatchable, since a stack overflow is not a
    /// panic. Such a file is skipped rather than walked. Payload carries
    /// the observed cap so the audit says why.
    TooDeep(usize),
}

impl SkipReason {
    /// Short slug for metrics buckets (`"gitignore"`, `"builtin"`, …).
    pub fn slug(&self) -> &'static str {
        match self {
            SkipReason::Gitignore(_) => "gitignore",
            SkipReason::Cggignore(_) => "cggignore",
            SkipReason::Builtin(_) => "builtin",
            SkipReason::UnknownExtension => "unknown-extension",
            SkipReason::LanguageFilter(_) => "language-filter",
            SkipReason::Binary => "binary",
            SkipReason::SymlinkOutsideRoot => "symlink-outside-root",
            SkipReason::ParseError(_) => "parse-error",
            SkipReason::TooLarge => "too-large",
            SkipReason::Minified => "minified",
            SkipReason::TooDeep(_) => "too-deep",
        }
    }
}

/// How the receiver's type was known at a call site (Issue 9). Recorded
/// so the unresolved population can be sliced by how much was actually
/// known — a missing edge with a known receiver type is a resolver
/// defect; one with no hint at all may be unknowable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReceiverProvenance {
    /// No receiver hint was available at the site.
    #[default]
    None,
    /// Hint came from a function parameter's type annotation.
    ParamAnnotation,
    /// Hint came from a struct/class field's declared type.
    FieldType,
    /// Hint came from a local initializer (`let x = Type::new()`).
    Initializer,
}

impl ReceiverProvenance {
    fn is_none(&self) -> bool {
        matches!(self, ReceiverProvenance::None)
    }
}

/// Candidate counts at each index the resolver consulted before giving
/// up (Issue 9). Lets a precision regression be localized to a stage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateCounts {
    #[serde(default)]
    pub file_local: u32,
    #[serde(default)]
    pub package: u32,
    #[serde(default)]
    pub workspace: u32,
}

impl CandidateCounts {
    fn is_empty(&self) -> bool {
        self.file_local == 0 && self.package == 0 && self.workspace == 0
    }
}

/// One external module's share of the unresolved calls.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedModuleBucket {
    /// Import path as written (`boto3`, `@aws-sdk/client-s3`), or
    /// `"(unattributed)"` when no import in the calling file explains
    /// the name. Unattributed is a real answer, not a failure: a bare
    /// call to something cgg cannot see has no module to blame.
    pub module: String,
    pub count: u32,
    /// A few of the names, for orientation. Deduplicated and sorted, so
    /// the list is stable across runs.
    pub sample: Vec<String>,
}

/// Which resolution stage rejected an unresolved call, with the
/// stage-specific failure (Issue 9). Replaces the old free-form reason
/// string; legacy string forms still deserialize via [`de_reason`], so
/// old audit JSON remains readable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "stage", content = "detail")]
pub enum UnresolvedReason {
    /// Intra-file: no same-name candidate in the file.
    NoCandidateInFile,
    /// Intra-file: multiple same-name candidates; not disambiguated.
    AmbiguousInFile,
    /// Intra-file: the reference is not inside any callable body.
    NoEnclosingCallable,
    /// Cross-file: no candidate found across the workspace.
    NoCandidateCrossFile,
    /// Rejected by the stack-graphs resolver.
    StackGraphs,
    /// Duck-typed dispatch found more candidates than the fan-out cap
    /// allows, so none were emitted.
    ///
    /// This case used to produce no record at all. A dropped call that
    /// looks identical to "there is no call here" is worse than a
    /// missing edge: it silently understates the caller set, which is
    /// the one thing impact analysis must not do.
    FanoutCapExceeded { candidates: u32 },
    /// The name resolves somewhere, just not in this file.
    ///
    /// Distinct from [`NoCandidateInFile`], which reads as "this name
    /// does not exist" and was being reported for names cgg had parsed
    /// and indexed.
    ///
    /// [`NoCandidateInFile`]: UnresolvedReason::NoCandidateInFile
    CandidatesInOtherFiles { candidates: u32 },
    /// A bare identifier whose only same-name candidates are methods,
    /// in a language with no implicit-receiver call form. They are not
    /// in scope for this call, so the target is elsewhere.
    NotInScopeForBareCall { methods: u32 },
    /// A class instantiation whose class declares no explicit
    /// constructor, so there is no callable to point at.
    ClassWithoutExplicitInit,
    /// `super().m()` where the base class is outside the analyzed tree.
    SuperBaseOutOfGraph,
    /// A `VALUE_REF_HINT` site (a bare identifier passed as an
    /// argument, e.g. `register(handler)`) with two or more same-name
    /// candidates in scope.
    ///
    /// Distinct from [`AmbiguousInFile`], which every existing consumer
    /// — the `ambiguous-in-file` metrics bucket, the dead-code
    /// correlation — reads as an ordinary ambiguous *call*. A value
    /// reference is plumbing, not a call site, and on cgg's own corpus
    /// it was 96.7% of everything `AmbiguousInFile` reported, drowning
    /// the real ambiguous calls the label exists to surface.
    ///
    /// [`AmbiguousInFile`]: UnresolvedReason::AmbiguousInFile
    ValueRefAmbiguous { candidates: u32 },
    /// A `VALUE_REF_HINT` site with no enclosing callable to hang an
    /// edge on (module scope, class body, a decorator argument).
    ///
    /// Distinct from [`NoEnclosingCallable`] for the same reason
    /// [`ValueRefAmbiguous`] is distinct from `AmbiguousInFile`.
    ///
    /// [`NoEnclosingCallable`]: UnresolvedReason::NoEnclosingCallable
    ValueRefNoEnclosing,
    /// Any other / legacy reason, preserving the original text.
    Other(String),
}

/// Every `stage` tag the structured form writes, `Other`'s included.
///
/// This is the deserializer's *recognizer*: a tag in this list is
/// decoded by the derive and a malformed `detail` under it is a hard
/// error, because that is a corrupt document. A tag **not** in this
/// list is a reason written by a newer cgg, and degrades to
/// [`UnresolvedReason::Other`] rather than failing the whole graph.
///
/// Kept in sync by `every_variant_tag_is_a_known_stage`, which walks
/// one instance of every variant and fails if its serialized tag is
/// missing here — so adding a variant and forgetting this list is a
/// test failure, not a silent reclassification.
pub const KNOWN_UNRESOLVED_STAGES: &[&str] = &[
    "no-candidate-in-file",
    "ambiguous-in-file",
    "no-enclosing-callable",
    "no-candidate-cross-file",
    "stack-graphs",
    "fanout-cap-exceeded",
    "candidates-in-other-files",
    "not-in-scope-for-bare-call",
    "class-without-explicit-init",
    "super-base-out-of-graph",
    "value-ref-ambiguous",
    "value-ref-no-enclosing",
    "other",
];

impl UnresolvedReason {
    /// Map a legacy free-form reason slug onto the structured form.
    pub fn from_legacy(s: &str) -> Self {
        match s {
            "no-candidate-in-scope" => UnresolvedReason::NoCandidateInFile,
            "ambiguous-in-file" | "ambiguous" => UnresolvedReason::AmbiguousInFile,
            "no-enclosing-callable" => UnresolvedReason::NoEnclosingCallable,
            other => UnresolvedReason::Other(other.to_string()),
        }
    }

    /// Short stable slug for metrics bucketing.
    pub fn slug(&self) -> &str {
        match self {
            UnresolvedReason::NoCandidateInFile => "no-candidate-in-file",
            UnresolvedReason::AmbiguousInFile => "ambiguous-in-file",
            UnresolvedReason::NoEnclosingCallable => "no-enclosing-callable",
            UnresolvedReason::NoCandidateCrossFile => "no-candidate-cross-file",
            UnresolvedReason::StackGraphs => "stack-graphs",
            UnresolvedReason::FanoutCapExceeded { .. } => "fanout-cap-exceeded",
            UnresolvedReason::CandidatesInOtherFiles { .. } => {
                "candidates-in-other-files"
            }
            UnresolvedReason::NotInScopeForBareCall { .. } => {
                "not-in-scope-for-bare-call"
            }
            UnresolvedReason::ClassWithoutExplicitInit => "class-without-explicit-init",
            UnresolvedReason::SuperBaseOutOfGraph => "super-base-out-of-graph",
            UnresolvedReason::ValueRefAmbiguous { .. } => "value-ref-ambiguous",
            UnresolvedReason::ValueRefNoEnclosing => "value-ref-no-enclosing",
            UnresolvedReason::Other(s) => s.as_str(),
        }
    }
}

/// Deserialize a reason from either the structured object form
/// (`{"stage": "..."}`) or a legacy free-form string. Keeps old audit
/// JSON readable after the Issue 9 schema change.
fn de_reason<'de, D>(d: D) -> Result<UnresolvedReason, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    struct R;
    impl<'de> Visitor<'de> for R {
        type Value = UnresolvedReason;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("an unresolved-reason string or {stage, detail} object")
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<UnresolvedReason, E> {
            Ok(UnresolvedReason::from_legacy(v))
        }
        fn visit_map<A: de::MapAccess<'de>>(
            self,
            map: A,
        ) -> Result<UnresolvedReason, A::Error> {
            // Buffered rather than decoded straight through, so an
            // unrecognised `stage` can fall back to `Other` instead of
            // failing the document.
            //
            // A reader that refuses a whole graph because one reason
            // tag is newer than it is turns an additive change into a
            // hard break — `cgg 0.8.3` does exactly that on the two
            // `value-ref-*` reasons, reporting `unknown variant` at a
            // byte offset rather than a version mismatch. `Other` is
            // documented as "any other / legacy reason, preserving the
            // original text" and this is the forward half of that job.
            //
            // The fallback is *narrow on purpose*: it fires only for a
            // tag outside `KNOWN_UNRESOLVED_STAGES`. A known tag whose
            // `detail` will not decode is a corrupt document, and still
            // fails loudly.
            let v = serde_json::Value::deserialize(
                de::value::MapAccessDeserializer::new(map),
            )?;
            if let Some(stage) = v.get("stage").and_then(serde_json::Value::as_str)
                && !crate::audit::KNOWN_UNRESOLVED_STAGES.contains(&stage)
            {
                return Ok(UnresolvedReason::Other(stage.to_string()));
            }
            UnresolvedReason::deserialize(&v).map_err(de::Error::custom)
        }
    }
    d.deserialize_any(R)
}

/// A single call site that the resolver could not bind to any
/// in-project callable.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditUnresolvedCall {
    pub src: Option<CallableId>,
    pub file: FileId,
    pub site_line: u32,
    pub site_byte: u32,
    pub name: String,
    /// The receiver/qualifier on the call (e.g. `Vec` in `vec.push()`).
    /// Empty if the call is unqualified.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub receiver_hint: String,
    /// Which resolution stage rejected the site (Issue 9). Was a
    /// free-form string; legacy strings still parse.
    #[serde(deserialize_with = "de_reason")]
    pub reason: UnresolvedReason,
    /// How the receiver's type was known at the site, if at all.
    #[serde(default, skip_serializing_if = "ReceiverProvenance::is_none")]
    pub receiver_provenance: ReceiverProvenance,
    /// Candidate counts at each index consulted before giving up.
    #[serde(default, skip_serializing_if = "CandidateCounts::is_empty")]
    pub candidates: CandidateCounts,
    /// Set when a name-based screen (e.g. stdlib vocabulary) was applied
    /// before owner-based lookup: `"stdlib"`, `"external"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_screen_applied: Option<String>,
}

impl AuditUnresolvedCall {
    /// Construct from a reference with the given stage reason. Evidence
    /// fields (provenance, candidate counts, name screen) default to
    /// unknown and may be filled in by later stages.
    pub fn new(
        src: Option<CallableId>,
        file: FileId,
        site_line: u32,
        site_byte: u32,
        name: String,
        receiver_hint: String,
        reason: UnresolvedReason,
    ) -> Self {
        Self {
            src,
            file,
            site_line,
            site_byte,
            name,
            receiver_hint,
            reason,
            receiver_provenance: ReceiverProvenance::None,
            candidates: CandidateCounts::default(),
            name_screen_applied: None,
        }
    }
}

/// An FFI descriptor detected in source.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditFfiRecord {
    /// `"c-abi"`, `"pyo3"`, `"jni"`, `"cbindgen"`, `"uniffi"`, `"napi"`,
    /// `"wasm-bindgen"`.
    pub family: String,
    /// The exported / imported symbol name.
    pub symbol: String,
    pub site_file: FileId,
    pub site_line: u32,
    pub resolved: bool,
    /// Peer callable, when `resolved` is true.
    pub peer: Option<CallableId>,
}

/// Per-file audit record, the richer companion to `FileRecord`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditFileRecord {
    pub file: FileId,
    pub path: PathBuf,
    pub language: String,
    pub detected_via: String,
    pub blake3: String,
    pub size_bytes: u64,
    pub lines: u32,
    pub parse_ms: f64,
    pub parse_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<SkipReason>,
    /// Set when the file is test code, with the rule that decided it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_role: Option<crate::testfile::TestFileReason>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub callables: Vec<AuditCallableRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_calls: Vec<AuditUnresolvedCall>,
    /// Calls into the language standard library — split out so the
    /// `external_calls` bucket reflects third-party surface only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stdlib_calls: Vec<AuditUnresolvedCall>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub external_calls: Vec<AuditUnresolvedCall>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ffi: Vec<AuditFfiRecord>,
}

/// A reference from the audit's per-file list back into the graph.
/// Denormalized so the audit payload is self-describing without
/// needing the main graph.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditCallableRef {
    pub id: CallableId,
    pub qualified_name: String,
    pub kind: String,
    pub start_line: u32,
    pub end_line: u32,
    pub start_byte: u32,
    pub end_byte: u32,
}

/// A single audit event for streaming (jsonl) output.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "event")]
pub enum AuditEvent {
    /// Run lifecycle markers.
    RunStarted {
        cgg_version: String,
        argv: Vec<String>,
    },
    RunFinished {
        metrics: RunMetrics,
    },
    /// File-level events.
    FileDiscovered {
        path: PathBuf,
    },
    FileSkipped {
        path: PathBuf,
        reason: SkipReason,
    },
    FileAnalyzed(AuditFileRecord),
    /// `--since <revspec>` resolved a git diff into seed callables.
    /// `matched_seeds` are the qualified names that became extra
    /// `--filter` patterns. `unmatched_files` lists files that git
    /// reported as changed but for which no current callable's body
    /// overlapped a hunk (deletions, comment-only edits, or
    /// non-source files like docs).
    SinceResolved {
        revspec: String,
        files_changed: u64,
        matched_seeds: Vec<String>,
        unmatched_files: Vec<PathBuf>,
    },
    /// Which frameworks the run recognised, which it saw but could not
    /// enumerate, and which languages have no rules at all.
    ///
    /// Emitted on every run that synthesizes entry nodes, including runs
    /// that recognised nothing. A machine consumer reading only
    /// `recognised` would conclude an unfamiliar framework's app has no
    /// attack surface; the gap list is what stops that, so it travels
    /// with the numbers rather than beside them.
    FrameworkCoverage {
        coverage: crate::frameworks::FrameworkCoverage,
    },
    /// Unresolved calls bucketed by the external module they belong to.
    ///
    /// Answers the question an audit usually has after reading a graph:
    /// *what can I not see from here, and how much of it is there?* On
    /// one package in the field report, 74 unresolved calls mapped
    /// almost exactly onto a dependency the reader had no access to, and
    /// the tally quantified the evidence gap for free — but only after
    /// they grouped it by hand.
    UnresolvedByModule {
        modules: Vec<UnresolvedModuleBucket>,
    },
    /// `-n 0` path enumeration stopped at `--max-paths`.
    ///
    /// Emitted only when the cap actually turned away work that had been
    /// reached, never merely because the count landed on the limit. A
    /// capped path set is indistinguishable from a complete one by
    /// inspection — the caller asked for every route through a callable
    /// and got a prefix of them — so the truncation has to be stated
    /// somewhere the caller can find it after the fact.
    PathsTruncated {
        max_paths: u32,
        paths_emitted: u32,
    },
    /// The graph was folded to a coarser granularity before rendering.
    ///
    /// Emitted whenever a rollup actually happened, which is the only
    /// event here that describes a *view* of the graph rather than the
    /// analysis. It is recorded anyway for the same reason
    /// [`AuditEvent::PathsTruncated`] is: a rolled-up graph is a
    /// perfectly well-formed graph of something that is not what was
    /// analyzed, and nothing in the artifact's shape says which. The
    /// `attempts` list carries every granularity that was measured and
    /// rejected, so the choice can be second-guessed after the fact.
    RolledUp {
        /// The level the output was cut at (`"file"`, `"dir:2"`).
        level: String,
        /// Token budget that forced it, if any. `None` means
        /// `--rollup-by` asked for this level outright.
        budget: Option<u64>,
        /// Estimated tokens of the emitted artifact, by the same formula
        /// the budget is compared against.
        estimated_tokens: u64,
        /// Callables and edges before folding.
        nodes_before: u64,
        edges_before: u64,
        /// Group nodes and folded edges after.
        nodes_after: u64,
        edges_after: u64,
        /// Every granularity measured, finest first.
        attempts: Vec<RollupAttempt>,
        /// The budget could not be met even at the coarsest granularity,
        /// so the artifact is over it.
        over_budget: bool,
    },
    /// A graph was loaded from a previous run's JSON instead of analyzed.
    ///
    /// `source_filtered` is set when the loaded document's own metrics
    /// say the analysis found more callables than the document contains
    /// — i.e. it was already narrowed by `--filter`, and this replay can
    /// only narrow it further.
    GraphReplayed {
        path: PathBuf,
        callables: u64,
        edges: u64,
        source_filtered: bool,
    },
}

/// One granularity [`AuditEvent::RolledUp`] measured on its way to a
/// decision.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RollupAttempt {
    pub level: String,
    pub nodes: u64,
    pub edges: u64,
    pub estimated_tokens: u64,
}

/// Run-level metrics rolled up once, after the last file is done.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RunMetrics {
    pub wall_ms: f64,
    pub phases: PhaseTimings,
    pub files_discovered: u64,
    pub files_analyzed: u64,
    pub files_skipped: u64,
    pub files_errored: u64,
    pub bytes_processed: u64,
    pub callables: u64,
    pub edges: u64,
    pub unresolved_calls: u64,
    /// Call sites targeting the language standard library
    /// (`Vec::push`, `clone()`, `format!`, …). Expected; not a gap.
    pub stdlib_calls: u64,
    /// Call sites targeting code not in the project and not in stdlib
    /// — third-party crates, framework methods, etc.
    pub external_calls: u64,
    pub ffi_detected: u64,
    pub ffi_resolved: u64,
    pub peak_rss_bytes: u64,
    pub by_language: indexmap::IndexMap<String, LanguageMetrics>,
    pub cycles: Vec<Vec<CallableId>>,
    pub longest_path_len: u32,
    pub confidence_histogram: ConfidenceHistogram,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PhaseTimings {
    pub walk_ms: f64,
    pub parse_ms: f64,
    pub extract_ms: f64,
    pub resolve_ms: f64,
    pub link_ms: f64,
    pub query_ms: f64,
    pub format_ms: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LanguageMetrics {
    pub files: u64,
    pub callables: u64,
    pub edges: u64,
    pub unresolved: u64,
    pub stdlib: u64,
    pub external: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ConfidenceHistogram {
    pub high: u64,
    pub medium: u64,
    pub low: u64,
}

/// Incremental builder for a single file's audit record.
#[derive(Clone, Debug, Default)]
pub struct FileAuditBuilder {
    pub record: Option<AuditFileRecord>,
}

impl FileAuditBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn begin(&mut self, rec: AuditFileRecord) {
        self.record = Some(rec);
    }

    pub fn finish(mut self) -> Option<AuditFileRecord> {
        self.record.take()
    }
}

/// Builder for the run-level summary (pure convenience around the
/// `RunMetrics` struct).
#[derive(Clone, Debug, Default)]
pub struct RunAuditBuilder {
    pub metrics: RunMetrics,
}

impl RunAuditBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn finish(self) -> RunMetrics {
        self.metrics
    }
}

/// Writer trait implemented by `json` (batch) and `jsonl` (streaming)
/// audit emitters.
pub trait AuditWriter: Send {
    fn emit(&mut self, event: &AuditEvent) -> io::Result<()>;
    fn flush(&mut self) -> io::Result<()>;
}

/// Streams events one JSON object per line (newline-delimited JSON).
///
/// Suitable for SIEM/log ingestion; safe to tail while the run is in
/// progress.
pub struct JsonlAuditWriter<W: io::Write + Send> {
    out: W,
}

impl<W: io::Write + Send> JsonlAuditWriter<W> {
    pub fn new(out: W) -> Self {
        Self { out }
    }
}

impl<W: io::Write + Send> std::fmt::Debug for JsonlAuditWriter<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JsonlAuditWriter").finish()
    }
}

impl<W: io::Write + Send> AuditWriter for JsonlAuditWriter<W> {
    fn emit(&mut self, event: &AuditEvent) -> io::Result<()> {
        serde_json::to_writer(&mut self.out, event).map_err(io::Error::other)?;
        self.out.write_all(b"\n")?;
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

/// Buffers events and writes one pretty-printed JSON document at the
/// end of the run. Used when the user expects a single audit artifact
/// (or when `-t json` embeds the audit inside the graph doc).
pub struct JsonAuditWriter<W: io::Write + Send> {
    out: W,
    buffer: Vec<AuditEvent>,
}

impl<W: io::Write + Send> JsonAuditWriter<W> {
    pub fn new(out: W) -> Self {
        Self {
            out,
            buffer: Vec::new(),
        }
    }

    /// Drain the buffer into the writer as a single pretty array.
    ///
    /// Events are serialized in parallel and joined. The audit is one
    /// event per analyzed file plus one per unresolved call, so on a
    /// large tree it is tens of megabytes of JSON — 569ms of a 7.4s
    /// Druid run, all of it on one core while the rest of the box idled.
    /// Each event is independent, and `par_iter().map().collect()`
    /// preserves order, so the document is byte-identical to the serial
    /// form.
    pub fn finalize(mut self) -> io::Result<()> {
        let _s = crate::profile::span("audit::write");
        if self.buffer.is_empty() {
            self.out.write_all(b"[]\n")?;
            return self.out.flush();
        }
        let parts: Vec<String> = self
            .buffer
            .par_iter()
            .map(|e| {
                // Two-space indent, then re-indented by one level to sit
                // inside the enclosing array — matching what
                // `to_writer_pretty` emits for an array of objects.
                let body = serde_json::to_string_pretty(e)
                    .unwrap_or_else(|_| "null".to_string());
                let mut out = String::with_capacity(body.len() + body.len() / 8);
                for (i, line) in body.lines().enumerate() {
                    if i > 0 {
                        out.push('\n');
                    }
                    out.push_str("  ");
                    out.push_str(line);
                }
                out
            })
            .collect();
        self.out.write_all(b"[\n")?;
        for (i, p) in parts.iter().enumerate() {
            if i > 0 {
                self.out.write_all(b",\n")?;
            }
            self.out.write_all(p.as_bytes())?;
        }
        self.out.write_all(b"\n]")?;
        self.out.write_all(b"\n")?;
        self.out.flush()
    }

    pub fn buffer(&self) -> &[AuditEvent] {
        &self.buffer
    }
}

impl<W: io::Write + Send> std::fmt::Debug for JsonAuditWriter<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JsonAuditWriter")
            .field("buffered", &self.buffer.len())
            .finish()
    }
}

impl<W: io::Write + Send> AuditWriter for JsonAuditWriter<W> {
    fn emit(&mut self, event: &AuditEvent) -> io::Result<()> {
        self.buffer.push(event.clone());
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn skip_reason_slug_stable() {
        assert_eq!(SkipReason::UnknownExtension.slug(), "unknown-extension");
        assert_eq!(SkipReason::Builtin("node_modules".into()).slug(), "builtin");
        assert_eq!(SkipReason::Binary.slug(), "binary");
        assert_eq!(SkipReason::Minified.slug(), "minified");
    }

    /// Number of `UnresolvedReason` variants. Lives next to the
    /// no-wildcard `match` below, which is what forces this to be
    /// revisited: a new variant fails to compile until it has an arm,
    /// and the arm's index must then be admitted here or the length
    /// assertion fails. Two checks, because the arm alone only proves
    /// the *match* is exhaustive — not that the vec holds an instance.
    const UNRESOLVED_REASON_VARIANTS: usize = 13;

    /// One instance of every `UnresolvedReason` variant.
    fn every_unresolved_reason() -> Vec<UnresolvedReason> {
        let all = vec![
            UnresolvedReason::NoCandidateInFile,
            UnresolvedReason::AmbiguousInFile,
            UnresolvedReason::NoEnclosingCallable,
            UnresolvedReason::NoCandidateCrossFile,
            UnresolvedReason::StackGraphs,
            UnresolvedReason::FanoutCapExceeded { candidates: 7 },
            UnresolvedReason::CandidatesInOtherFiles { candidates: 3 },
            UnresolvedReason::NotInScopeForBareCall { methods: 2 },
            UnresolvedReason::ClassWithoutExplicitInit,
            UnresolvedReason::SuperBaseOutOfGraph,
            UnresolvedReason::ValueRefAmbiguous { candidates: 2 },
            UnresolvedReason::ValueRefNoEnclosing,
            UnresolvedReason::Other("legacy-thing".into()),
        ];
        // Compile-time exhaustiveness check, then a runtime one: every
        // arm index must appear, so a variant with an arm but no
        // instance above is caught rather than silently untested.
        let mut seen = std::collections::BTreeSet::new();
        for r in &all {
            let idx: usize = match r {
                UnresolvedReason::NoCandidateInFile => 0,
                UnresolvedReason::AmbiguousInFile => 1,
                UnresolvedReason::NoEnclosingCallable => 2,
                UnresolvedReason::NoCandidateCrossFile => 3,
                UnresolvedReason::StackGraphs => 4,
                UnresolvedReason::FanoutCapExceeded { .. } => 5,
                UnresolvedReason::CandidatesInOtherFiles { .. } => 6,
                UnresolvedReason::NotInScopeForBareCall { .. } => 7,
                UnresolvedReason::ClassWithoutExplicitInit => 8,
                UnresolvedReason::SuperBaseOutOfGraph => 9,
                UnresolvedReason::ValueRefAmbiguous { .. } => 10,
                UnresolvedReason::ValueRefNoEnclosing => 11,
                UnresolvedReason::Other(_) => 12,
            };
            seen.insert(idx);
        }
        assert_eq!(
            seen.len(),
            UNRESOLVED_REASON_VARIANTS,
            "every_unresolved_reason() is missing an instance for some variant"
        );
        assert_eq!(all.len(), UNRESOLVED_REASON_VARIANTS);
        all
    }

    /// `KNOWN_UNRESOLVED_STAGES` is the deserializer's recognizer, so a
    /// variant missing from it would be decoded as a *newer cgg's*
    /// reason and silently flattened to `Other` — losing its detail
    /// with nothing reported. Adding a variant and forgetting the list
    /// must be a test failure, not a downgrade.
    #[test]
    fn every_variant_tag_is_a_known_stage() {
        for r in every_unresolved_reason() {
            let v = serde_json::to_value(&r).unwrap();
            let stage = v["stage"].as_str().unwrap().to_string();
            assert!(
                KNOWN_UNRESOLVED_STAGES.contains(&stage.as_str()),
                "{stage:?} is written by {r:?} but is not in \
                 KNOWN_UNRESOLVED_STAGES"
            );
        }
    }

    /// Round-trip through the `de_reason` path every variant takes on
    /// the wire, so the buffered fallback cannot have changed what a
    /// *recognised* tag decodes to.
    #[test]
    fn known_stages_round_trip_through_de_reason() {
        for r in every_unresolved_reason() {
            let doc = serde_json::json!({
                "file": "F0", "site_line": 1, "site_byte": 2, "name": "f",
                "reason": serde_json::to_value(&r).unwrap(),
            });
            let back: AuditUnresolvedCall = serde_json::from_value(doc).unwrap();
            assert_eq!(back.reason, r, "round trip changed {r:?}");
        }
    }

    /// A reason tag this binary has never heard of degrades to `Other`
    /// carrying the original text, instead of failing the document.
    ///
    /// This is the defect 0.8.3 has: it refuses a graph written by this
    /// version with `unknown variant value-ref-ambiguous` at a byte
    /// offset. We cannot fix 0.8.3, but from here on an additive reason
    /// is an additive change.
    #[test]
    fn an_unknown_stage_degrades_to_other_rather_than_failing() {
        let doc = serde_json::json!({
            "file": "F0", "site_line": 1, "site_byte": 2, "name": "f",
            "reason": {"stage": "reason-from-the-future", "detail": {"n": 4}},
        });
        let back: AuditUnresolvedCall = serde_json::from_value(doc).unwrap();
        assert_eq!(
            back.reason,
            UnresolvedReason::Other("reason-from-the-future".into())
        );
        // `slug()` reports the original text, so metrics bucketing is
        // still meaningful rather than collapsing to a literal "other".
        assert_eq!(back.reason.slug(), "reason-from-the-future");
    }

    /// The fallback is narrow: a *recognised* tag whose detail will not
    /// decode is a corrupt document and must still be refused, not
    /// quietly reclassified as `Other`.
    #[test]
    fn a_known_stage_with_a_broken_detail_is_still_an_error() {
        let doc = serde_json::json!({
            "file": "F0", "site_line": 1, "site_byte": 2, "name": "f",
            "reason": {"stage": "fanout-cap-exceeded",
                       "detail": {"candidates": "not-a-number"}},
        });
        let err = serde_json::from_value::<AuditUnresolvedCall>(doc).unwrap_err();
        assert!(
            format!("{err}").contains("u32"),
            "the detail's type error must survive, not be swallowed \
             into Other: {err}"
        );
    }

    #[test]
    fn metrics_round_trip() {
        let m = RunMetrics {
            wall_ms: 123.4,
            files_analyzed: 10,
            ..Default::default()
        };
        let s = serde_json::to_string(&m).unwrap();
        let m2: RunMetrics = serde_json::from_str(&s).unwrap();
        assert_eq!(m.files_analyzed, m2.files_analyzed);
    }

    #[test]
    fn jsonl_writer_one_per_line() {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut w = JsonlAuditWriter::new(&mut buf);
            w.emit(&AuditEvent::FileDiscovered {
                path: PathBuf::from("a.py"),
            })
            .unwrap();
            w.emit(&AuditEvent::FileSkipped {
                path: PathBuf::from("b.dat"),
                reason: SkipReason::Binary,
            })
            .unwrap();
            w.flush().unwrap();
        }
        let text = String::from_utf8(buf).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.contains("file_discovered"));
        assert!(text.contains("file_skipped"));
        assert!(text.contains("\"kind\":\"binary\""));
    }

    #[test]
    fn json_writer_emits_single_doc() {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut w = JsonAuditWriter::new(&mut buf);
            w.emit(&AuditEvent::FileDiscovered {
                path: PathBuf::from("a.py"),
            })
            .unwrap();
            w.finalize().unwrap();
        }
        let text = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(parsed.is_array());
        assert_eq!(parsed.as_array().unwrap().len(), 1);
    }

    #[test]
    fn unresolved_reason_round_trip_and_legacy() {
        // Structured form round-trips.
        let mut c = AuditUnresolvedCall::new(
            None,
            FileId::new(0),
            1,
            2,
            "m".into(),
            "T".into(),
            UnresolvedReason::AmbiguousInFile,
        );
        c.candidates.file_local = 3;
        let s = serde_json::to_string(&c).unwrap();
        assert!(s.contains("\"stage\":\"ambiguous-in-file\""));
        let c2: AuditUnresolvedCall = serde_json::from_str(&s).unwrap();
        assert_eq!(c2.reason, UnresolvedReason::AmbiguousInFile);
        assert_eq!(c2.candidates.file_local, 3);

        // Legacy free-form string form still parses.
        let legacy = r#"{"src":null,"file":"F0","site_line":1,"site_byte":2,"name":"m","reason":"ambiguous-in-file"}"#;
        let c3: AuditUnresolvedCall = serde_json::from_str(legacy).unwrap();
        assert_eq!(c3.reason, UnresolvedReason::AmbiguousInFile);
    }

    #[test]
    fn value_ref_reasons_round_trip_and_have_stable_slugs() {
        let c = AuditUnresolvedCall::new(
            None,
            FileId::new(0),
            1,
            2,
            "handler".into(),
            String::new(),
            UnresolvedReason::ValueRefAmbiguous { candidates: 2 },
        );
        assert_eq!(c.reason.slug(), "value-ref-ambiguous");
        let s = serde_json::to_string(&c).unwrap();
        assert!(s.contains("\"stage\":\"value-ref-ambiguous\""));
        let c2: AuditUnresolvedCall = serde_json::from_str(&s).unwrap();
        assert_eq!(
            c2.reason,
            UnresolvedReason::ValueRefAmbiguous { candidates: 2 }
        );

        let n = UnresolvedReason::ValueRefNoEnclosing;
        assert_eq!(n.slug(), "value-ref-no-enclosing");
    }
}
