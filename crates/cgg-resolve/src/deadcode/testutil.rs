//! Small graph builders for the dead-code unit tests.
//!
//! Mirrors the local `mk_graph()` helpers already used by `dispatch.rs`
//! and `cross_file.rs`, shared here because the engine's modules all
//! need the same shapes.

use cgg_core::graph::{
    CallEdge, CallableKind, CallableNode, Confidence, FileRecord, Graph, Via,
};
use cgg_core::ids::{CallableId, FileId, ResolverId};

/// A source-backed callable in file 0.
pub(crate) fn node(id: u32, qn: &str, simple: &str, lang: &str) -> CallableNode {
    CallableNode {
        id: CallableId::new(id),
        qualified_name: qn.to_string(),
        simple_name: simple.to_string(),
        kind: CallableKind::Function,
        language: lang.to_string(),
        file: FileId::new(0),
        start_line: id * 10 + 1,
        end_line: id * 10 + 3,
        ..Default::default()
    }
}

pub(crate) fn edge(src: u32, dst: u32) -> CallEdge {
    CallEdge {
        src: CallableId::new(src),
        dst: CallableId::new(dst),
        site_line: 1,
        site_byte: 0,
        confidence: Confidence::High,
        via: Via::Direct,
        resolver: ResolverId::new("test"),
    }
}

fn file(lang: &str) -> FileRecord {
    FileRecord {
        id: FileId::new(0),
        path: std::path::PathBuf::from("t.src"),
        language: lang.to_string(),
        detected_via: "test".into(),
        blake3: "0".repeat(64),
        size_bytes: 0,
        lines: 0,
        parse_ms: 0.0,
        parse_status: "ok".into(),
        ..Default::default()
    }
}

/// A graph containing exactly `nodes`, all in one file whose language is
/// taken from the first node.
pub(crate) fn graph_with(nodes: Vec<CallableNode>) -> Graph {
    let lang = nodes.first().map(|n| n.language.clone()).unwrap_or_default();
    let mut g = Graph::new();
    g.add_file(file(&lang));
    for n in nodes {
        g.add_callable(n);
    }
    g
}

/// A graph of `n` nodes wired by `edges`, for the traversal tests.
/// Edge `i` gets `site_line = i + 1` so tests can filter on it.
pub(crate) fn mk_graph(edges: &[(u32, u32)], n: u32) -> Graph {
    let mut g = graph_with(
        (0..n)
            .map(|i| node(i, &format!("crate::fn_{i}"), &format!("fn_{i}"), "rust"))
            .collect(),
    );
    for (i, &(s, d)) in edges.iter().enumerate() {
        let mut e = edge(s, d);
        e.site_line = i as u32 + 1;
        g.edges.push(e);
    }
    g
}

/// Add a `Via::Direct` edge.
pub(crate) fn link(g: &mut Graph, src: u32, dst: u32) {
    g.edges.push(edge(src, dst));
}
