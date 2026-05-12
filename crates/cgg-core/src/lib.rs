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
pub mod external;
pub mod facts;
pub mod graph;
pub mod ids;
pub mod stdlib;
pub mod version;

pub use audit::{
    AuditEvent, AuditFileRecord, AuditFfiRecord, AuditUnresolvedCall, AuditWriter,
    FileAuditBuilder, JsonAuditWriter, JsonlAuditWriter, RunAuditBuilder, RunMetrics,
    SkipReason,
};
pub use facts::{DefRecord, DefVariant, FileFacts, ImportRecord, LocalType, RefRecord};
pub use external::{classify_external, build_known_names, ClassifyResult};
pub use graph::{
    CallEdge, CallableKind, CallableNode, Confidence, FileRecord, Graph, Via,
};
pub use ids::{CallableId, FileId, ResolverId};
pub use version::{CGG_VERSION, RESOLVER_FORMAT_VERSION};
