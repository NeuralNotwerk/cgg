//! DOT (Graphviz) formatter.

use crate::{GraphFormatter, OutputFormat};
use cgg_core::Graph;
use cgg_core::graph::Via;
use cgg_core::ids::CallableId;
use std::io;

/// Per-`via` DOT rendering: a short label tag and extra edge attributes
/// so over-approximated (dynamic/reference), exit-node
/// (external/stdlib) and entry-node edges are visually distinct from
/// direct calls.
fn via_dot(via: &Via) -> (&'static str, &'static str) {
    match via {
        Via::Direct => ("", ""),
        Via::Dynamic => ("dyn", ", style=dashed"),
        Via::Reference => ("ref", ", style=dotted"),
        Via::External => ("ext", ", color=\"#2266cc\""),
        Via::Stdlib => ("std", ", color=\"#22aa66\""),
        Via::Ffi(_) => ("ffi", ", color=\"#cc6622\""),
        // Same hue family as FFI: both cross a language boundary.
        Via::Descriptor(_) => ("desc", ", color=\"#cc9922\", style=bold"),
        // Bold, because an entry node is where control comes *in*: on a
        // rendered graph these are the sources a reader traces from.
        Via::FrameworkEntry(_) => ("entry", ", color=\"#aa22cc\", style=bold"),
    }
}

#[derive(Debug, Default)]
pub struct DotFormatter;

impl DotFormatter {
    pub fn new() -> Self {
        Self
    }
}

impl GraphFormatter for DotFormatter {
    fn format(&self) -> OutputFormat {
        OutputFormat::Dot
    }

    fn render(&self, graph: &Graph, out: &mut dyn io::Write) -> io::Result<()> {
        writeln!(out, "digraph cgg {{")?;
        writeln!(out, "  rankdir=LR;")?;
        writeln!(out, "  node [shape=box, style=rounded];")?;
        for (id, node) in &graph.callables {
            let label = dot_escape(&node.qualified_name);
            if let Some(kind) = node.framework_entry {
                writeln!(
                    out,
                    "  n{} [label=\"{}\", shape=invhouse, color=\"#aa22cc\", \
                     tooltip=\"framework entry callback ({}) — SYNTHESIZED: no call to \
                     this node exists in your source\"];",
                    id.token(),
                    label,
                    kind.slug()
                )?;
            } else if node.unreferenced.is_some() {
                writeln!(
                    out,
                    "  n{} [label=\"{}\", style=dashed, \
                     tooltip=\"unreferenced (best effort: cgg found no caller, \
                     which is not proof none exists)\"];",
                    id.token(),
                    label
                )?;
            } else {
                writeln!(out, "  n{} [label=\"{}\"];", id.token(), label)?;
            }
        }
        // Collapse parallel edges (same src/dst pair, different call
        // sites in source) into a single rendered edge. Preserve
        // first-occurrence order for deterministic, diff-friendly
        // output. JSON/GraphML still emit one entry per call site —
        // this is purely a render-time concern.
        // Keys stay `Copy`; the base36 token is rendered only at write
        // time. See the matching note in mermaid.rs.
        let mut order: Vec<(CallableId, CallableId, &str, &str)> = Vec::new();
        let mut counts: std::collections::HashMap<(CallableId, CallableId, &str), u32> =
            std::collections::HashMap::new();
        for edge in &graph.edges {
            let (tag, style) = via_dot(&edge.via);
            let key = (edge.src, edge.dst, tag);
            // `weight`, not `1` — see the same loop in `mermaid.rs`. An
            // ordinary edge carries weight 1, so this is a no-op for
            // every graph that was not rolled up.
            let entry = counts.entry(key).or_insert(0);
            let first = *entry == 0;
            *entry += edge.weight;
            if first {
                order.push((key.0, key.1, tag, style));
            }
        }
        for (src, dst, tag, style) in order {
            let n = counts[&(src, dst, tag)];
            let label = match (tag.is_empty(), n > 1) {
                (true, false) => String::new(),
                (true, true) => format!("{n}x"),
                (false, false) => tag.to_string(),
                (false, true) => format!("{tag} {n}x"),
            };
            let (src, dst) = (src.token(), dst.token());
            if label.is_empty() && style.is_empty() {
                writeln!(out, "  n{src} -> n{dst};")?;
            } else if style.is_empty() {
                writeln!(out, "  n{src} -> n{dst} [label=\"{label}\"];")?;
            } else if label.is_empty() {
                writeln!(
                    out,
                    "  n{src} -> n{dst} [{}];",
                    style.trim_start_matches(", ")
                )?;
            } else {
                writeln!(out, "  n{src} -> n{dst} [label=\"{label}\"{style}];")?;
            }
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
    use cgg_core::graph::{
        CallEdge, CallableKind, CallableNode, Confidence, FileRecord, Graph, Via,
    };
    use cgg_core::ids::{CallableId, FileId, ResolverId};
    use std::path::PathBuf;

    fn mk_graph_with_edges(edge_pairs: &[(u32, u32, u32)]) -> Graph {
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
        for id in 0..3 {
            g.add_callable(CallableNode {
                id: CallableId::new(id),
                qualified_name: format!("c{id}"),
                simple_name: format!("c{id}"),
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
        }
        for &(s, d, byte) in edge_pairs {
            g.add_edge(CallEdge {
                src: CallableId::new(s),
                dst: CallableId::new(d),
                site_line: 1,
                site_byte: byte,
                confidence: Confidence::High,
                via: Via::Direct,
                resolver: ResolverId::new("intra-file"),
                weight: 1,
            });
        }
        g
    }

    #[test]
    fn renders_dot() {
        let g = mk_graph_with_edges(&[(0, 1, 5)]);
        let mut buf = Vec::new();
        DotFormatter.render(&g, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.starts_with("digraph cgg {"));
        assert!(s.contains("n0 [label=\"c0\"]"));
        // Single edge: no count label.
        assert!(s.contains("n0 -> n1;"), "got:\n{s}");
        assert!(!s.contains("label=\"1x\""));
    }

    #[test]
    fn parallel_edges_collapse_with_count_label() {
        // Same caller/callee at three distinct byte positions.
        let g = mk_graph_with_edges(&[(0, 1, 5), (0, 1, 50), (0, 1, 500)]);
        let mut buf = Vec::new();
        DotFormatter.render(&g, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("n0 -> n1 [label=\"3x\"];"), "got:\n{s}");
        let edge_lines = s.lines().filter(|l| l.contains(" -> ")).count();
        assert_eq!(edge_lines, 1, "expected 1 collapsed edge:\n{s}");
        // Bare form must not coexist with the labeled form.
        assert!(!s.contains("n0 -> n1;"), "got:\n{s}");
    }

    #[test]
    fn first_occurrence_order_preserved() {
        // a->b, then a->c, then a second a->b. Output must list a->b
        // before a->c despite the interleaving.
        let g = mk_graph_with_edges(&[(0, 1, 10), (0, 2, 20), (0, 1, 30)]);
        let mut buf = Vec::new();
        DotFormatter.render(&g, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        let ab = s.find("n0 -> n1 [label=\"2x\"]").expect("a->b edge");
        let ac = s.find("n0 -> n2;").expect("a->c edge");
        assert!(ab < ac, "expected a->b before a->c:\n{s}");
    }
}
