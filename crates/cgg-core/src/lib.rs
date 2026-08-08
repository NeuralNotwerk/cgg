//! Core data model for `cgg`.
//!
//! This crate defines the internal representation (IR) that every stage
//! of the pipeline manipulates:
//!
//! * [`Graph`] — callables, files, edges, unresolved calls, and metrics.
//! * [`CallableNode`] / [`FileRecord`] — audit-grade nodes and file
//!   provenance.
//! * [`CallEdge`] — edges tagged with `confidence`, `via`, and the
//!   `resolver` id that produced them.
//! * [`audit`] — per-file and run-level audit records.
//!
//! The IR is deliberately formatter-agnostic: every output shape
//! (mermaid, json, dot, graphml) is a thin transform over [`Graph`].

#![deny(missing_debug_implementations)]
#![warn(unreachable_pub)]

pub mod audit;
pub mod cpu;
pub mod deadcode;
pub mod external;
pub mod facts;
pub mod frameworks;
pub mod graph;
pub mod ids;
pub mod profile;
pub mod stdlib;
pub mod testfile;
pub mod version;

pub use audit::{
    AuditEvent, AuditFfiRecord, AuditFileRecord, AuditUnresolvedCall, AuditWriter,
    CandidateCounts, FileAuditBuilder, JsonAuditWriter, JsonlAuditWriter,
    ReceiverProvenance, RunAuditBuilder, RunMetrics, SkipReason, UnresolvedReason,
};
pub use deadcode::{
    DEAD_CODE_DISCLAIMER, DEFAULT_MIN_ROOT_COVERAGE_PCT, DeadCodeConfig, DeadCodeFinding,
    DeadCodeReport, DeadCodeSummary, DeadRegion, Evidence, FindingCategory,
    LanguageCapabilityReport, LanguageClass, LanguageSignals, LivenessProof, Polarity,
    ProofHop, RegionRole, RootKind, RootRecord, SignalSupport, SiteRef,
    SuppressedCategory, SuppressionReason,
};
pub use external::{
    ClassifyResult, FileAliases, build_alias_map, build_known_names, classify_external,
};
pub use facts::{
    DefRecord, DefVariant, DynUse, ExportRecord, FileFacts, ImportRecord, LocalType,
    RefRecord, STRING_REF_HINT, TestRole, UnreachableRegion, VALUE_REF_HINT, Vis,
};
pub use frameworks::{
    EntryShape, FRAMEWORK_ENTRY_DISCLAIMER, FRAMEWORK_ENTRY_SENTINEL, FrameworkCoverage,
    FrameworkEntry, FrameworkRule, REACHABILITY_NOT_TAINT, RecognisedFramework,
    SeenFramework, TrustKind, UncoveredLanguage,
};
pub use graph::{
    CallEdge, CallableKind, CallableNode, Confidence, FileRecord, Graph, Via,
};
pub use ids::{CallableId, FileId, ResolverId};
pub use testfile::{TestFileReason, classify_test_file};
pub use version::CGG_VERSION;
