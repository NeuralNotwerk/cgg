//! Mermaid (flowchart) formatter.
//!
//! Task 5 ships the minimum viable writer so the end-to-end pipeline
//! has a visible output. Task 9 upgrades this with subgraphs per
//! language, edge styles for `via` / `confidence`, and escape rules
//! for mermaid-reserved characters.

use std::io;

use cgg_core::graph::Via;
use cgg_core::Graph;

use crate::{GraphFormatter, OutputFormat};

/// Short label prefix distinguishing an edge's `via` kind in the
/// mermaid label slot (mermaid has no native per-edge styling). Direct
/// edges get no tag so the common case stays clean for agents reading
/// the graph. Over-approximated edges (`dyn`, `ref`) and exit-node
/// edges (`ext`, `std`) are tagged so consumers can filter them.
fn via_tag(via: &Via) -> &'static str {
    match via {
        Via::Direct => "",
        Via::Dynamic => "dyn",
        Via::Reference => "ref",
        Via::External => "ext",
        Via::Stdlib => "std",
        Via::Ffi(_) => "ffi",
    }
}

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
        let any_unreferenced = graph.callables.values().any(|n| n.unreferenced.is_some());
        if any_unreferenced {
            // A mark in a diagram gets pasted into places its evidence
            // does not follow, so the caveat rides along in a comment
            // that survives copy-paste of the block.
            writeln!(
                out,
                "%% cgg: nodes tagged `unreferenced` are BEST-EFFORT findings —"
            )?;
            writeln!(
                out,
                "%% cgg could not find a caller, which is not proof none exists."
            )?;
        }
        writeln!(out, "flowchart LR")?;

        // Nodes. Mermaid ids need to be word-safe; we use `C<n>` where
        // `n` is the callable id's numeric value, and place the
        // qualified name as the display label.
        for (id, node) in &graph.callables {
            let label = mermaid_escape(&node.qualified_name);
            // The tag is part of the label rather than only a style, so
            // it survives renderers that drop classDef and readers who
            // only see the text.
            let tag = if node.unreferenced.is_some() {
                " ⟨unreferenced⟩"
            } else {
                ""
            };
            writeln!(out, "  C{id_n}[\"{label}{tag}\"]", id_n = id.as_u32())?;
        }
        if any_unreferenced {
            writeln!(out, "  classDef unreferenced stroke-dasharray: 4 3;")?;
            let marked: Vec<String> = graph
                .callables
                .iter()
                .filter(|(_, n)| n.unreferenced.is_some())
                .map(|(id, _)| format!("C{}", id.as_u32()))
                .collect();
            for chunk in marked.chunks(32) {
                writeln!(out, "  class {} unreferenced;", chunk.join(","))?;
            }
        }

        // Edges. The internal graph keeps one edge per call site (per
        // distinct `site_byte`), which makes JSON/GraphML faithful to
        // call frequency. For mermaid that produces visually-stacked
        // arrows; collapse identical `(src, dst)` pairs into a single
        // arrow and surface the multiplicity as a `|Nx|` edge label
        // when N > 1. First-occurrence order is preserved so output is
        // deterministic and diff-friendly.
        // Collapse identical (src, dst, via-kind) triples. Distinct via
        // kinds between the same pair stay separate, labeled rows so a
        // direct call and a dynamic-dispatch fan-out don't merge.
        let mut order: Vec<(u32, u32, &str)> = Vec::new();
        let mut counts: std::collections::HashMap<(u32, u32, &str), u32> =
            std::collections::HashMap::new();
        for edge in &graph.edges {
            let key = (edge.src.as_u32(), edge.dst.as_u32(), via_tag(&edge.via));
            if counts.insert(key, counts.get(&key).copied().unwrap_or(0) + 1).is_none() {
                order.push(key);
            }
        }
        for (src, dst, tag) in order {
            let n = counts[&(src, dst, tag)];
            let label = match (tag.is_empty(), n > 1) {
                (true, false) => String::new(),
                (true, true) => format!("|{n}x|"),
                (false, false) => format!("|{tag}|"),
                (false, true) => format!("|{tag} {n}x|"),
            };
            if label.is_empty() {
                writeln!(out, "  C{src} --> C{dst}")?;
            } else {
                writeln!(out, "  C{src} -->{label} C{dst}")?;
            }
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
            ..Default::default()
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
            synthetic: false,
            trait_impl_target: None,
            ..Default::default()
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
            synthetic: false,
            trait_impl_target: None,
            ..Default::default()
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

    #[test]
    fn parallel_edges_collapse_with_count_label() {
        // Three call sites from a -> b at distinct byte positions.
        // The graph keeps three CallEdge entries; the renderer must
        // collapse them into a single arrow with a `|3x|` label.
        let mut g = mk_graph();
        for site in [11_u32, 22, 33] {
            g.add_edge(CallEdge {
                src: CallableId::new(0),
                dst: CallableId::new(1),
                site_line: 1,
                site_byte: site,
                confidence: Confidence::High,
                via: Via::Direct,
                resolver: ResolverId::new("intra-file"),
            });
        }
        let mut buf = Vec::new();
        MermaidFormatter.render(&g, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        // The original edge from mk_graph + 3 new ones = 4 total.
        assert!(s.contains("C0 -->|4x| C1"), "got:\n{s}");
        // Exactly one rendered arrow line for this pair.
        let arrows = s.lines().filter(|l| l.contains("--> ") || l.contains("-->|")).count();
        assert_eq!(arrows, 1, "got:\n{s}");
        // The bare-arrow form must not appear when a label is required.
        assert!(!s.contains("C0 --> C1"), "got:\n{s}");
    }

    #[test]
    fn single_edge_renders_without_label() {
        // mk_graph emits one a->b edge — must NOT carry a count label.
        let mut buf = Vec::new();
        MermaidFormatter.render(&mk_graph(), &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("C0 --> C1"), "got:\n{s}");
        assert!(!s.contains("|1x|"), "got:\n{s}");
    }

    #[test]
    fn first_occurrence_order_preserved() {
        // Build edges in a deliberate non-sorted order; emitted arrows
        // must follow first-occurrence order so output is deterministic
        // and diffs cleanly.
        let mut g = mk_graph();
        let c = g.add_callable(CallableNode {
            id: CallableId::new(2),
            qualified_name: "crate::c".into(),
            simple_name: "c".into(),
            kind: CallableKind::Function,
            language: "rust".into(),
            file: FileId::new(0),
            start_line: 3,
            end_line: 3,
            start_byte: 20,
            end_byte: 30,
            signature_hint: String::new(),
            visibility: String::new(),
            attributes: Vec::new(),
            synthetic: false,
            trait_impl_target: None,
            ..Default::default()
        });
        // Order: a->c first, then a second occurrence of a->b, then a->c again.
        g.add_edge(CallEdge {
            src: CallableId::new(0), dst: c, site_line: 2, site_byte: 100,
            confidence: Confidence::High, via: Via::Direct,
            resolver: ResolverId::new("intra-file"),
        });
        g.add_edge(CallEdge {
            src: CallableId::new(0), dst: CallableId::new(1), site_line: 3, site_byte: 200,
            confidence: Confidence::High, via: Via::Direct,
            resolver: ResolverId::new("intra-file"),
        });
        g.add_edge(CallEdge {
            src: CallableId::new(0), dst: c, site_line: 4, site_byte: 300,
            confidence: Confidence::High, via: Via::Direct,
            resolver: ResolverId::new("intra-file"),
        });
        let mut buf = Vec::new();
        MermaidFormatter.render(&g, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        // a->b was the very first edge from mk_graph(), so it must
        // render before a->c despite a->c being added before the second
        // a->b.
        let ab = s.find("C0 -->|2x| C1").expect("a->b arrow");
        let ac = s.find("C0 -->|2x| C2").expect("a->c arrow");
        assert!(ab < ac, "expected a->b before a->c in output:\n{s}");
    }
}
