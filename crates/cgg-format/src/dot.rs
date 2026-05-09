//! DOT (Graphviz) formatter.

use std::io;
use cgg_core::Graph;
use crate::{GraphFormatter, OutputFormat};

#[derive(Debug, Default)]
pub struct DotFormatter;

impl DotFormatter {
    pub fn new() -> Self { Self }
}

impl GraphFormatter for DotFormatter {
    fn format(&self) -> OutputFormat { OutputFormat::Dot }

    fn render(&self, graph: &Graph, out: &mut dyn io::Write) -> io::Result<()> {
        writeln!(out, "digraph cgg {{")?;
        writeln!(out, "  rankdir=LR;")?;
        writeln!(out, "  node [shape=box, style=rounded];")?;
        for (id, node) in &graph.callables {
            let label = dot_escape(&node.qualified_name);
            writeln!(out, "  n{} [label=\"{}\"];", id.as_u32(), label)?;
        }
        for edge in &graph.edges {
            writeln!(out, "  n{} -> n{};", edge.src.as_u32(), edge.dst.as_u32())?;
        }
        if graph.callables.is_empty() {
            writeln!(out, "  empty [label=\"no callables\"];")?;
        }
        writeln!(out, "}}")?;
        Ok(())
    }
}

fn dot_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::graph::{CallableKind, CallableNode, FileRecord, Graph};
    use cgg_core::ids::{CallableId, FileId};
    use std::path::PathBuf;

    #[test]
    fn renders_dot() {
        let mut g = Graph::new();
        g.add_file(FileRecord {
            id: FileId::new(0), path: PathBuf::from("a.rs"),
            language: "rust".into(), detected_via: "ext".into(),
            sha256: "0".repeat(64), size_bytes: 10, lines: 1,
            parse_ms: 0.1, parse_status: "ok".into(),
        });
        g.add_callable(CallableNode {
            id: CallableId::new(0), qualified_name: "foo".into(),
            simple_name: "foo".into(), kind: CallableKind::Function,
            language: "rust".into(), file: FileId::new(0),
            start_line: 1, end_line: 1, start_byte: 0, end_byte: 10,
            signature_hint: String::new(), visibility: String::new(),
            attributes: vec![],
        });
        let mut buf = Vec::new();
        DotFormatter.render(&g, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.starts_with("digraph cgg {"));
        assert!(s.contains("n0 [label=\"foo\"]"));
    }
}
