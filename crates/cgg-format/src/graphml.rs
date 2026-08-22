//! GraphML formatter.

use crate::{GraphFormatter, OutputFormat};
use cgg_core::Graph;
use cgg_core::graph::Via;
use std::io;

/// Stable `via` slug for the edge attribute. Direct calls carry none so
/// the common case stays terse.
fn via_slug(via: &Via) -> &'static str {
    match via {
        Via::Direct => "",
        Via::Dynamic => "dynamic",
        Via::Reference => "reference",
        Via::External => "external",
        Via::Stdlib => "stdlib",
        Via::Ffi(_) => "ffi",
        Via::Descriptor(_) => "descriptor",
        Via::FrameworkEntry(_) => "framework-entry",
    }
}

#[derive(Debug, Default)]
pub struct GraphmlFormatter;

impl GraphmlFormatter {
    pub fn new() -> Self {
        Self
    }
}

impl GraphFormatter for GraphmlFormatter {
    fn format(&self) -> OutputFormat {
        OutputFormat::Graphml
    }

    fn render(&self, graph: &Graph, out: &mut dyn io::Write) -> io::Result<()> {
        writeln!(out, r#"<?xml version="1.0" encoding="UTF-8"?>"#)?;
        // The GraphML namespace is graphdrawing.org — this read
        // "graphstruct.org", which is not the spec's URI and not a real
        // domain. Importers that key on the namespace (yEd, Gephi — the
        // two the README names as the reason this format exists) can
        // reject or mis-handle a document declaring an unknown one.
        writeln!(
            out,
            r#"<graphml xmlns="http://graphml.graphdrawing.org/xmlns">"#
        )?;
        writeln!(
            out,
            r#"  <key id="label" for="node" attr.name="label" attr.type="string"/>"#
        )?;
        writeln!(
            out,
            r#"  <key id="lang" for="node" attr.name="language" attr.type="string"/>"#
        )?;
        writeln!(
            out,
            r#"  <key id="unreferenced" for="node" attr.name="unreferenced" attr.type="string"/>"#
        )?;
        writeln!(
            out,
            r#"  <key id="framework_entry" for="node" attr.name="framework_entry" attr.type="string"/>"#
        )?;
        writeln!(
            out,
            r#"  <key id="via" for="edge" attr.name="via" attr.type="string"/>"#
        )?;
        // Declared only when something uses it, so an ordinary graph's
        // document is byte-for-byte what it was before edge weights
        // existed. Same reasoning as `skip_serializing_if` on the JSON
        // side: a rollup is opt-in, and opting out of it should leave no
        // trace anywhere.
        if graph.edges.iter().any(|e| e.weight != 1) {
            writeln!(
                out,
                r#"  <key id="weight" for="edge" attr.name="weight" attr.type="int"/>"#
            )?;
        }
        writeln!(out, r#"  <graph id="G" edgedefault="directed">"#)?;
        for (id, node) in &graph.callables {
            writeln!(out, r#"    <node id="n{}">"#, id.token())?;
            writeln!(
                out,
                r#"      <data key="label">{}</data>"#,
                xml_escape(&node.qualified_name)
            )?;
            writeln!(
                out,
                r#"      <data key="lang">{}</data>"#,
                xml_escape(&node.language)
            )?;
            if let Some(kind) = node.framework_entry {
                // SYNTHESIZED: no call to this node exists in source.
                writeln!(
                    out,
                    r#"      <data key="framework_entry">{}</data>"#,
                    kind.slug()
                )?;
            }
            if let Some(c) = node.unreferenced {
                // Best-effort finding: cgg found no caller, which is not
                // proof that none exists.
                writeln!(
                    out,
                    r#"      <data key="unreferenced">{}</data>"#,
                    match c {
                        cgg_core::graph::Confidence::High => "high",
                        cgg_core::graph::Confidence::Medium => "medium",
                        cgg_core::graph::Confidence::Low => "low",
                    }
                )?;
            }
            writeln!(out, r#"    </node>"#)?;
        }
        for (i, edge) in graph.edges.iter().enumerate() {
            // Without the `via` tag a GraphML consumer cannot tell an
            // inferred entry edge from a resolved call — the one
            // distinction every other formatter surfaces.
            let via = via_slug(&edge.via);
            // A rolled-up edge stands for many call sites. GraphML is
            // the import format for graph-analysis tools, where an
            // unweighted aggregate edge silently flattens call frequency
            // — the one thing this format keeps that mermaid drops.
            let weight = if edge.weight == 1 {
                String::new()
            } else {
                format!(r#"<data key="weight">{}</data>"#, edge.weight)
            };
            if via.is_empty() && weight.is_empty() {
                writeln!(
                    out,
                    r#"    <edge id="e{}" source="n{}" target="n{}"/>"#,
                    i,
                    edge.src.token(),
                    edge.dst.token()
                )?;
            } else {
                let via_data = if via.is_empty() {
                    String::new()
                } else {
                    format!(r#"<data key="via">{via}</data>"#)
                };
                writeln!(
                    out,
                    r#"    <edge id="e{}" source="n{}" target="n{}">{}{}</edge>"#,
                    i,
                    edge.src.token(),
                    edge.dst.token(),
                    via_data,
                    weight
                )?;
            }
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
            id: FileId::new(0),
            path: PathBuf::from("a.rs"),
            language: "rust".into(),
            detected_via: "ext".into(),
            blake3: "0".repeat(64),
            size_bytes: 10,
            lines: 1,
            parse_ms: 0.1,
            parse_status: "ok".into(),
            ..Default::default()
        });
        g.add_callable(CallableNode {
            id: CallableId::new(0),
            qualified_name: "foo<T>".into(),
            simple_name: "foo".into(),
            kind: CallableKind::Function,
            language: "rust".into(),
            file: FileId::new(0),
            start_line: 1,
            end_line: 1,
            start_byte: 0,
            end_byte: 10,
            signature_hint: String::new(),
            visibility: String::new(),
            attributes: vec![],
            synthetic: false,
            trait_impl_target: None,
            ..Default::default()
        });
        let mut buf = Vec::new();
        GraphmlFormatter.render(&g, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("<graphml"));
        assert!(s.contains("foo&lt;T&gt;"));
    }
}
