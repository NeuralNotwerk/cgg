//! GraphML formatter.

use crate::locations::concrete_site;
use crate::node_ids::{NodeIds, NodeNamer};
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

#[derive(Debug)]
pub struct GraphmlFormatter {
    node_ids: NodeIds,
    /// Print each call's file and line. Off by default so an ordinary
    /// document is byte-identical to one from before the flag existed.
    locations: bool,
}

impl Default for GraphmlFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphmlFormatter {
    pub fn new() -> Self {
        Self {
            node_ids: OutputFormat::Graphml.default_node_ids(),
            locations: false,
        }
    }

    pub fn with_node_ids(node_ids: NodeIds) -> Self {
        Self {
            node_ids,
            locations: false,
        }
    }

    pub fn with_locations(mut self, locations: bool) -> Self {
        self.locations = locations;
        self
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
        // Declared only when an edge will carry them. An ordinary graph,
        // and a locations run whose every edge is an aggregate, stay free
        // of keys nothing uses.
        let any_site = self.locations
            && graph
                .edges
                .iter()
                .any(|e| concrete_site(graph, e).is_some());
        if any_site {
            writeln!(
                out,
                r#"  <key id="site_file" for="edge" attr.name="site_file" attr.type="string"/>"#
            )?;
            writeln!(
                out,
                r#"  <key id="site_line" for="edge" attr.name="site_line" attr.type="int"/>"#
            )?;
        }
        writeln!(out, r#"  <graph id="G" edgedefault="directed">"#)?;
        let namer = NodeNamer::new(graph, self.node_ids, "n", "n");
        for (id, node) in &graph.callables {
            writeln!(out, r#"    <node id="{}">"#, namer.name(*id))?;
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
            let site = if self.locations {
                concrete_site(graph, edge)
            } else {
                None
            };
            let site_data = match site {
                Some(site) => format!(
                    r#"<data key="site_file">{}</data><data key="site_line">{}</data>"#,
                    xml_escape(&site.path),
                    site.line
                ),
                None => String::new(),
            };
            if via.is_empty() && weight.is_empty() && site_data.is_empty() {
                writeln!(
                    out,
                    r#"    <edge id="e{}" source="{}" target="{}"/>"#,
                    i,
                    namer.name(edge.src),
                    namer.name(edge.dst)
                )?;
            } else {
                let via_data = if via.is_empty() {
                    String::new()
                } else {
                    format!(r#"<data key="via">{via}</data>"#)
                };
                writeln!(
                    out,
                    r#"    <edge id="e{}" source="{}" target="{}">{}{}{}</edge>"#,
                    i,
                    namer.name(edge.src),
                    namer.name(edge.dst),
                    via_data,
                    weight,
                    site_data
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
    use cgg_core::graph::{
        CallEdge, CallableKind, CallableNode, Confidence, FileRecord, Graph, Via,
    };
    use cgg_core::ids::ResolverId;
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
        GraphmlFormatter::new().render(&g, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("<graphml"));
        assert!(s.contains("foo&lt;T&gt;"));
        assert!(!s.contains("site_file"), "locations are opt-in:\n{s}");
    }

    #[test]
    fn locations_add_file_and_line_per_edge() {
        let mut g = Graph::new();
        g.add_file(FileRecord {
            id: FileId::new(0),
            path: PathBuf::from("a.rs"),
            language: "rust".into(),
            ..Default::default()
        });
        g.add_callable(CallableNode {
            id: CallableId::new(0),
            qualified_name: "a".into(),
            file: FileId::new(0),
            kind: CallableKind::Function,
            ..Default::default()
        });
        g.add_callable(CallableNode {
            id: CallableId::new(1),
            qualified_name: "b".into(),
            file: FileId::new(0),
            kind: CallableKind::Function,
            ..Default::default()
        });
        g.add_edge(CallEdge {
            src: CallableId::new(0),
            dst: CallableId::new(1),
            site_line: 12,
            site_byte: 40,
            confidence: Confidence::High,
            via: Via::Direct,
            resolver: ResolverId::new("intra-file"),
            weight: 1,
        });
        let mut buf = Vec::new();
        GraphmlFormatter::new()
            .with_locations(true)
            .render(&g, &mut buf)
            .unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains(r#"attr.name="site_file""#), "got:\n{s}");
        assert!(
            s.contains(
                r#"<data key="site_file">a.rs</data><data key="site_line">12</data>"#
            ),
            "got:\n{s}"
        );
    }
}
