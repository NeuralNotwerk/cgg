//! GraphML formatter.

use std::io;
use cgg_core::Graph;
use crate::{GraphFormatter, OutputFormat};

#[derive(Debug, Default)]
pub struct GraphmlFormatter;

impl GraphmlFormatter {
    pub fn new() -> Self { Self }
}

impl GraphFormatter for GraphmlFormatter {
    fn format(&self) -> OutputFormat { OutputFormat::Graphml }

    fn render(&self, graph: &Graph, out: &mut dyn io::Write) -> io::Result<()> {
        writeln!(out, r#"<?xml version="1.0" encoding="UTF-8"?>"#)?;
        writeln!(out, r#"<graphml xmlns="http://graphml.graphstruct.org/xmlns">"#)?;
        writeln!(out, r#"  <key id="label" for="node" attr.name="label" attr.type="string"/>"#)?;
        writeln!(out, r#"  <key id="lang" for="node" attr.name="language" attr.type="string"/>"#)?;
        writeln!(out, r#"  <graph id="G" edgedefault="directed">"#)?;
        for (id, node) in &graph.callables {
            writeln!(out, r#"    <node id="n{}">"#, id.as_u32())?;
            writeln!(out, r#"      <data key="label">{}</data>"#, xml_escape(&node.qualified_name))?;
            writeln!(out, r#"      <data key="lang">{}</data>"#, xml_escape(&node.language))?;
            writeln!(out, r#"    </node>"#)?;
        }
        for (i, edge) in graph.edges.iter().enumerate() {
            writeln!(
                out,
                r#"    <edge id="e{}" source="n{}" target="n{}"/>"#,
                i, edge.src.as_u32(), edge.dst.as_u32()
            )?;
        }
        writeln!(out, "  </graph>")?;
        writeln!(out, "</graphml>")?;
        Ok(())
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::graph::{CallableKind, CallableNode, FileRecord, Graph};
    use cgg_core::ids::{CallableId, FileId};
    use std::path::PathBuf;

    #[test]
    fn renders_graphml() {
        let mut g = Graph::new();
        g.add_file(FileRecord {
            id: FileId::new(0), path: PathBuf::from("a.rs"),
            language: "rust".into(), detected_via: "ext".into(),
            sha256: "0".repeat(64), size_bytes: 10, lines: 1,
            parse_ms: 0.1, parse_status: "ok".into(),
        });
        g.add_callable(CallableNode {
            id: CallableId::new(0), qualified_name: "foo<T>".into(),
            simple_name: "foo".into(), kind: CallableKind::Function,
            language: "rust".into(), file: FileId::new(0),
            start_line: 1, end_line: 1, start_byte: 0, end_byte: 10,
            signature_hint: String::new(), visibility: String::new(),
            attributes: vec![],
        });
        let mut buf = Vec::new();
        GraphmlFormatter.render(&g, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("<graphml"));
        assert!(s.contains("foo&lt;T&gt;"));
    }
}
