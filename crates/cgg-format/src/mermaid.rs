//! Mermaid (flowchart) formatter.
//!
//! Task 5 ships the minimum viable writer so the end-to-end pipeline
//! has a visible output. Task 9 upgrades this with subgraphs per
//! language, edge styles for `via` / `confidence`, and escape rules
//! for mermaid-reserved characters.

use std::io;

use cgg_core::Graph;

use crate::{GraphFormatter, OutputFormat};

#[derive(Debug, Default)]
pub struct MermaidFormatter;

impl MermaidFormatter {
    pub fn new() -> Self {
        Self
    }
}

impl GraphFormatter for MermaidFormatter {
    fn format(&self) -> OutputFormat {
        OutputFormat::Mermaid
    }

    fn render(&self, graph: &Graph, out: &mut dyn io::Write) -> io::Result<()> {
        writeln!(out, "flowchart LR")?;

        // Nodes. Mermaid ids need to be word-safe; we use `C<n>` where
        // `n` is the callable id's numeric value, and place the
        // qualified name as the display label.
        for (id, node) in &graph.callables {
            let label = mermaid_escape(&node.qualified_name);
            writeln!(out, "  C{id_n}[\"{label}\"]", id_n = id.as_u32())?;
        }

        // Edges. `-->` is the default directed arrow. The resolver and
        // confidence travel through to Task 9's richer rendering.
        for edge in &graph.edges {
            writeln!(
                out,
                "  C{src} --> C{dst}",
                src = edge.src.as_u32(),
                dst = edge.dst.as_u32()
            )?;
        }

        if graph.callables.is_empty() {
            // Mermaid needs at least one node to render anything — emit
            // a structured placeholder so the file is still valid.
            writeln!(out, "  Empty[\"no callables\"]")?;
        }

        Ok(())
    }
}

/// Escape characters that mermaid treats specially inside `["..."]`.
/// `"` collides with the bracket delimiter; `<`/`>` can be misread as
/// HTML. Keep everything else as-is — mermaid tolerates colons and `::`.
fn mermaid_escape(s: &str) -> String {
    s.replace('"', "'")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::{
        graph::{
            CallEdge, CallableKind, CallableNode, Confidence, FileRecord, Graph, Via,
        },
        ids::{CallableId, FileId, ResolverId},
    };
    use std::path::PathBuf;

    fn mk_graph() -> Graph {
        let mut g = Graph::new();
        g.add_file(FileRecord {
            id: FileId::new(0),
            path: PathBuf::from("t.rs"),
            language: "rust".into(),
            detected_via: "extension:.rs".into(),
            blake3: "0".repeat(64),
            size_bytes: 10,
            lines: 1,
            parse_ms: 0.1,
            parse_status: "ok".into(),
        });
        let a = g.add_callable(CallableNode {
            id: CallableId::new(0),
            qualified_name: "crate::a".into(),
            simple_name: "a".into(),
            kind: CallableKind::Function,
            language: "rust".into(),
            file: FileId::new(0),
            start_line: 1,
            end_line: 1,
            start_byte: 0,
            end_byte: 10,
            signature_hint: String::new(),
            visibility: String::new(),
            attributes: Vec::new(),
        });
        let b = g.add_callable(CallableNode {
            id: CallableId::new(1),
            qualified_name: "crate::b".into(),
            simple_name: "b".into(),
            kind: CallableKind::Function,
            language: "rust".into(),
            file: FileId::new(0),
            start_line: 2,
            end_line: 2,
            start_byte: 10,
            end_byte: 20,
            signature_hint: String::new(),
            visibility: String::new(),
            attributes: Vec::new(),
        });
        g.add_edge(CallEdge {
            src: a,
            dst: b,
            site_line: 1,
            site_byte: 5,
            confidence: Confidence::High,
            via: Via::Direct,
            resolver: ResolverId::new("intra-file"),
        });
        g
    }

    #[test]
    fn renders_nodes_and_edge() {
        let mut buf = Vec::new();
        MermaidFormatter.render(&mk_graph(), &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.starts_with("flowchart LR\n"));
        assert!(s.contains("C0[\"crate::a\"]"));
        assert!(s.contains("C1[\"crate::b\"]"));
        assert!(s.contains("C0 --> C1"));
    }

    #[test]
    fn empty_graph_is_still_valid() {
        let g = Graph::new();
        let mut buf = Vec::new();
        MermaidFormatter.render(&g, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("flowchart LR"));
        assert!(s.contains("no callables"));
    }

    #[test]
    fn angle_brackets_escaped() {
        let mut g = mk_graph();
        g.callables.get_mut(&CallableId::new(0)).unwrap().qualified_name =
            "crate::<A as B>::m".into();
        let mut buf = Vec::new();
        MermaidFormatter.render(&g, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("&lt;A as B&gt;"));
        assert!(!s.contains("<A"));
    }
}
