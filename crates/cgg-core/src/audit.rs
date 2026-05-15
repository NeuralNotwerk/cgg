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
            SkipReason::Binary => "binary",
            SkipReason::SymlinkOutsideRoot => "symlink-outside-root",
            SkipReason::ParseError(_) => "parse-error",
            SkipReason::TooLarge => "too-large",
        }
    }
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
    /// Human-readable explanation (`"no-candidate-in-scope"`,
    /// `"ambiguous"`, `"macro-expansion"`, `"dynamic-dispatch"`).
    pub reason: String,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub callables: Vec<AuditCallableRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_calls: Vec<AuditUnresolvedCall>,
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
    /// Call sites targeting symbols not defined in any scanned file
    /// (stdlib, third-party deps, framework methods, etc.).
    pub external_calls: u64,
    pub ffi_detected: u64,
    pub ffi_resolved: u64,
    pub cache: CacheMetrics,
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
pub struct CacheMetrics {
    pub hits: u64,
    pub misses: u64,
    pub bytes_read: u64,
    pub bytes_written: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LanguageMetrics {
    pub files: u64,
    pub callables: u64,
    pub edges: u64,
    pub unresolved: u64,
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
        serde_json::to_writer(&mut self.out, event)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
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
    pub fn finalize(mut self) -> io::Result<()> {
        serde_json::to_writer_pretty(&mut self.out, &self.buffer)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
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
        assert_eq!(
            SkipReason::Builtin("node_modules".into()).slug(),
            "builtin"
        );
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
}
