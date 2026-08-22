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
    /// Any other / legacy reason, preserving the original text.
    Other(String),
}

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
            UnresolvedReason::deserialize(de::value::MapAccessDeserializer::new(map))
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
}
