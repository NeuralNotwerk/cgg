//! Internal graph representation.
//!
//! The `Graph` is the single source of truth for every formatter and
//! every query operation. All entities are owned; all edges are
//! augmented with a `confidence`, a `via` classification, and the
//! `resolver` that produced them.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::audit::{AuditFileRecord, AuditUnresolvedCall, RunMetrics};
use crate::ids::{CallableId, FileId, ResolverId};

/// Kind of a callable node. Matches the "purely callables" scope from
/// the plan: anything you'd put executable code inside.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallableKind {
    /// A free function (`fn foo()`, `def foo():`, `function foo()`).
    Function,
    /// A method bound to a type (`impl T { fn m() }`, `class C { m() {} }`).
    Method,
    /// A constructor (`class C { constructor(...) {} }`, C# `.ctor`).
    Constructor,
    /// A destructor / finalizer (`~C()`, C# `~Foo()`).
    Destructor,
    /// A named closure / lambda bound to an identifier.
    Closure,
    /// A callable property (e.g. JavaScript getter/setter, Python
    /// `@property` with a callable value, Go interface method defaults).
    Property,
}

/// Confidence that an edge is real.
///
/// * `High`   — scope-resolved by a real resolver.
/// * `Medium` — name matches exactly one in-scope candidate, but scope
///              rules didn't fully disambiguate (e.g. macros-in-path,
///              unresolved import).
/// * `Low`    — multiple candidates, or dynamic dispatch to a virtual
///              family.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

/// Classification of how a call got into the graph.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "family")]
pub enum Via {
    /// An ordinary, resolved call within one language.
    Direct,
    /// An ambiguous dispatch collapsed to a family (dynamic dispatch,
    /// virtual call, duck typing).
    Dynamic,
    /// A cross-language edge produced by the FFI linker. The payload
    /// names the family (`"c-abi"`, `"pyo3"`, `"jni"`, `"napi"`,
    /// `"wasm-bindgen"`, `"cbindgen"`, `"uniffi"`).
    Ffi(String),
}

/// A callable node in the graph.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CallableNode {
    pub id: CallableId,
    pub qualified_name: String,
    pub simple_name: String,
    pub kind: CallableKind,
    pub language: String,
    pub file: FileId,

    /// Inclusive, 1-based line numbers for human display.
    pub start_line: u32,
    pub end_line: u32,

    /// Byte range in the source file (half-open, `[start, end)`).
    pub start_byte: u32,
    pub end_byte: u32,

    /// Optional single-line signature preview (may be empty).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signature_hint: String,

    /// Language-native visibility string (`"pub"`, `"public"`,
    /// `"internal"`, `""` for default).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub visibility: String,

    /// Attributes / decorators attached to the callable
    /// (`"#[get]"`, `"@app.route"`, `"@Override"`). Case-preserving.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributes: Vec<String>,
}

/// A directed call edge.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CallEdge {
    pub src: CallableId,
    pub dst: CallableId,
    /// 1-based line of the call site inside `src`.
    pub site_line: u32,
    /// Byte offset of the call site inside the source of `src`'s file.
    pub site_byte: u32,
    pub confidence: Confidence,
    pub via: Via,
    pub resolver: ResolverId,
}

/// Provenance record for a file that entered the analysis.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileRecord {
    pub id: FileId,
    pub path: PathBuf,
    pub language: String,
    pub detected_via: String,
    pub blake3: String,
    pub size_bytes: u64,
    pub lines: u32,
    pub parse_ms: f64,
    pub parse_status: String,
}

/// The complete in-memory representation.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Graph {
    pub callables: IndexMap<CallableId, CallableNode>,
    pub files: IndexMap<FileId, FileRecord>,
    pub edges: Vec<CallEdge>,
    pub unresolved: Vec<AuditUnresolvedCall>,
    /// Per-file audit records. The `files` map above carries the
    /// structural provenance; this carries the richer per-file payload
    /// (callables, unresolved refs, FFI hits) for the audit log.
    pub file_audits: Vec<AuditFileRecord>,
    pub metrics: RunMetrics,
}

impl Graph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a callable, returning its id. Panics if the node's `id`
    /// collides with an existing entry.
    pub fn add_callable(&mut self, node: CallableNode) -> CallableId {
        let id = node.id;
        let prev = self.callables.insert(id, node);
        assert!(prev.is_none(), "duplicate callable id {id}");
        id
    }

    /// Insert a file record, returning its id. Panics on duplicate id.
    pub fn add_file(&mut self, rec: FileRecord) -> FileId {
        let id = rec.id;
        let prev = self.files.insert(id, rec);
        assert!(prev.is_none(), "duplicate file id {id}");
        id
    }

    /// Append an edge. No dedup — the caller is responsible for
    /// ensuring they are not emitting the same edge twice.
    pub fn add_edge(&mut self, edge: CallEdge) {
        self.edges.push(edge);
    }

    /// Count of nodes and edges for quick metrics.
    pub fn size(&self) -> (usize, usize) {
        (self.callables.len(), self.edges.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn mk_callable(id: u32, name: &str, file: FileId) -> CallableNode {
        CallableNode {
            id: CallableId::new(id),
            qualified_name: name.into(),
            simple_name: name.rsplit("::").next().unwrap_or(name).into(),
            kind: CallableKind::Function,
            language: "rust".into(),
            file,
            start_line: 1,
            end_line: 3,
            start_byte: 0,
            end_byte: 32,
            signature_hint: String::new(),
            visibility: String::new(),
            attributes: vec![],
        }
    }

    fn mk_file(id: u32, path: &str) -> FileRecord {
        FileRecord {
            id: FileId::new(id),
            path: PathBuf::from(path),
            language: "rust".into(),
            detected_via: "extension:.rs".into(),
            blake3: "0".repeat(64),
            size_bytes: 10,
            lines: 3,
            parse_ms: 0.1,
            parse_status: "ok".into(),
        }
    }

    #[test]
    fn insert_and_count() {
        let mut g = Graph::new();
        let fid = g.add_file(mk_file(0, "a.rs"));
        let a = g.add_callable(mk_callable(0, "crate::a", fid));
        let b = g.add_callable(mk_callable(1, "crate::b", fid));
        g.add_edge(CallEdge {
            src: a,
            dst: b,
            site_line: 2,
            site_byte: 16,
            confidence: Confidence::High,
            via: Via::Direct,
            resolver: ResolverId::new("intra-file"),
        });
        assert_eq!(g.size(), (2, 1));
        assert_eq!(g.files.len(), 1);
    }

    #[test]
    fn round_trip_json() {
        let mut g = Graph::new();
        let fid = g.add_file(mk_file(0, "a.rs"));
        g.add_callable(mk_callable(0, "crate::foo", fid));
        let s = serde_json::to_string(&g).unwrap();
        let g2: Graph = serde_json::from_str(&s).unwrap();
        assert_eq!(g.size(), g2.size());
    }
}
