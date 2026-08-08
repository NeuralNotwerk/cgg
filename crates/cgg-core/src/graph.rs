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
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallableKind {
    /// A free function (`fn foo()`, `def foo():`, `function foo()`).
    #[default]
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
/// * `High` — scope-resolved by a real resolver.
/// * `Medium` — name matches exactly one in-scope candidate, but scope
///   rules didn't fully disambiguate (e.g. macros-in-path, unresolved
///   import).
/// * `Low` — multiple candidates, or dynamic dispatch to a virtual
///   family.
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
    /// virtual call, duck typing). Issue 3's interface/trait-dispatch
    /// fan-out edges (declaration → each implementation) use this and
    /// are gated behind `--dynamic-dispatch`.
    Dynamic,
    /// A function referenced as a value (passed by name to a registrar /
    /// callback slot) rather than called (Issue 4). Gated behind
    /// `--reference-edges`.
    Reference,
    /// An edge to a synthesized leaf "exit node" standing in for a call
    /// into third-party code, surfaced by `--include-external`.
    External,
    /// An edge to a synthesized leaf "exit node" standing in for a call
    /// into the language standard library, surfaced by `--include-stdlib`.
    Stdlib,
    /// A cross-language edge produced by the FFI linker. The payload
    /// names the family (`"c-abi"`, `"pyo3"`, `"jni"`, `"napi"`,
    /// `"wasm-bindgen"`, `"cbindgen"`, `"uniffi"`).
    Ffi(String),
    /// An edge from an interface *descriptor* to the code implementing
    /// it: a `.proto` rpc to the Go/Java method that serves it, a
    /// GraphQL field to its resolver. The payload names the family
    /// (`"grpc"`, `"graphql"`).
    ///
    /// The mirror of [`Via::Ffi`] one level up: FFI links two
    /// implementations across a language boundary, this links a
    /// *declaration* to an implementation across one. Both describe a
    /// call that is real and that no single language's parser can see.
    Descriptor(String),
    /// An edge from a synthesized `<framework-entry>` node into the
    /// handler a framework invokes. The payload names the framework
    /// (`"flask"`, `"spring"`, `"gin"`).
    ///
    /// The mirror image of `External`/`Stdlib`: those are sinks for a
    /// call cgg *saw* and could not resolve, this is a source for a
    /// caller that appears nowhere in the source at all. It is therefore
    /// an inference rather than an observation — see
    /// [`crate::frameworks::FRAMEWORK_ENTRY_DISCLAIMER`].
    FrameworkEntry(String),
}

/// A callable node in the graph.
///
/// `Default` is derived so construction sites can use
/// `..Default::default()` and stay source-compatible as optional
/// fields are added. The derived `id`/`file` are placeholder zeros —
/// every real construction site sets them explicitly.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
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

    /// True for nodes not written literally in source: synthesized
    /// exit nodes for external/stdlib calls (`--include-external` /
    /// `--include-stdlib`) and derive/codegen-surfaced methods (Issue 8).
    /// Lets consumers filter the over-approximated surface.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub synthetic: bool,

    /// For a concrete trait/interface implementation method, the trait it
    /// implements (Issue 3). `Some("Storage")` for
    /// `<DiskStorage as Storage>::put`. Drives the dispatch fan-out index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trait_impl_target: Option<String>,

    /// Normalized visibility, mirrored from `DefRecord::vis`.
    #[serde(default, skip_serializing_if = "crate::facts::Vis::is_unknown")]
    pub vis: crate::facts::Vis,

    /// Test role, mirrored from `DefRecord::test_role`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_role: Option<crate::facts::TestRole>,

    /// Set by `--dead-code` when nothing in the analyzed source appears
    /// to reference this callable; the value is the confidence of that
    /// finding.
    ///
    /// Lives on the node so that every formatter can render it without
    /// a second output path — "unreferenced" is a property of a node in
    /// the graph, not a separate document. `None` when the analysis did
    /// not run, so the default graph is byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unreferenced: Option<Confidence>,

    /// Set on a synthesized `<framework-entry>` node, naming the trust
    /// boundary control crosses there.
    ///
    /// A field rather than a name prefix the formatters sniff for: the
    /// qualified name carries the kind too, but a formatter deciding how
    /// to label a node should be reading a typed value, not parsing a
    /// string it also emits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub framework_entry: Option<crate::frameworks::TrustKind>,
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
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
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
    /// Set when the file is test code, with the rule that decided it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_role: Option<crate::testfile::TestFileReason>,
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

// Test fixtures spell every field, so the spread is redundant here —
// but it keeps them identical in shape to the production construction
// sites, which is what makes a new field a one-line change.
#[allow(clippy::needless_update)]
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
            synthetic: false,
            trait_impl_target: None,
            ..Default::default()
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
            test_role: None,
            ..Default::default()
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
