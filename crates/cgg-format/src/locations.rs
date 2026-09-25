//! Call-site file and line, for the renderers that do not already print it.
//!
//! JSON carries `site_line` on every edge and the caller's file on the
//! source callable. Mermaid, DOT and GraphML do not, unless `--locations`
//! asks them to. The graph is unchanged either way: this module only
//! decides which sites a renderer may name, and how to spell a list of
//! them on one collapsed arrow.

use cgg_core::graph::{CallEdge, Graph};
use cgg_core::ids::CallableId;

/// One concrete call site: the caller's file and the 1-based line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Site {
    pub path: String,
    pub line: u32,
}

/// Sites accumulated for one collapsed arrow.
///
/// `None` once any contributing edge has no single file and line — a
/// rolled-up edge (`weight > 1`, `site_line == 0`), or a synthetic caller
/// whose file is not source. The renderer then keeps the `Nx` label
/// rather than inventing a location for part of the arrow.
#[derive(Clone, Debug)]
pub(crate) struct SiteList {
    sites: Option<Vec<Site>>,
}

impl Default for SiteList {
    fn default() -> Self {
        Self::new()
    }
}

impl SiteList {
    pub(crate) fn new() -> Self {
        Self {
            sites: Some(Vec::new()),
        }
    }

    pub(crate) fn observe(&mut self, graph: &Graph, edge: &CallEdge) {
        let Some(buf) = self.sites.as_mut() else {
            return;
        };
        if let Some(site) = concrete_site(graph, edge) {
            buf.push(site);
        } else {
            self.sites = None;
        }
    }

    /// `path:line`, or `path:l1,l2` for several sites in the same file,
    /// files in edge order. `None` when the list is incomplete or empty.
    pub(crate) fn label(&self) -> Option<String> {
        let sites = self.sites.as_ref().filter(|s| !s.is_empty())?;
        Some(format_sites(sites))
    }
}

/// The caller's file and line, when this edge is exactly one call in a
/// real source file.
///
/// A synthetic source — a `<framework-entry>` node, a rolled-up group —
/// does not qualify. A framework-entry edge does carry the marker's
/// line, but not the marker's file (the source node is the sentinel),
/// and naming the callee's file instead would be wrong for a registrar
/// that lives in a different one.
pub(crate) fn concrete_site(graph: &Graph, edge: &CallEdge) -> Option<Site> {
    if edge.weight != 1 || edge.site_line == 0 {
        return None;
    }
    let path = caller_path(graph, edge.src)?;
    Some(Site {
        path,
        line: edge.site_line,
    })
}

fn caller_path(graph: &Graph, src: CallableId) -> Option<String> {
    let node = graph.callables.get(&src)?;
    if node.synthetic {
        return None;
    }
    let rec = graph.files.get(&node.file)?;
    let path = rec.path.to_string_lossy();
    if path.is_empty() {
        return None;
    }
    Some(path.into_owned())
}

/// `a.rs:10,12 b.rs:3` — consecutive sites that share a file collapse
/// onto one path, and a later site in an earlier file starts a new group
/// so the label stays in edge order.
fn format_sites(sites: &[Site]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < sites.len() {
        let path = &sites[i].path;
        let mut lines = vec![sites[i].line.to_string()];
        i += 1;
        while i < sites.len() && sites[i].path == *path {
            lines.push(sites[i].line.to_string());
            i += 1;
        }
        parts.push(format!("{path}:{}", lines.join(",")));
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::graph::{
        CallEdge, CallableKind, CallableNode, Confidence, FileRecord, Via,
    };
    use cgg_core::ids::{FileId, ResolverId};
    use std::path::PathBuf;

    fn edge(line: u32, weight: u32) -> CallEdge {
        CallEdge {
            src: CallableId::new(0),
            dst: CallableId::new(1),
            site_line: line,
            site_byte: line,
            confidence: Confidence::High,
            via: Via::Direct,
            resolver: ResolverId::new("intra-file"),
            weight,
        }
    }

    fn graph(synthetic_src: bool) -> Graph {
        let mut g = Graph::new();
        g.add_file(FileRecord {
            id: FileId::new(0),
            path: PathBuf::from("src/lib.rs"),
            language: "rust".into(),
            ..Default::default()
        });
        g.add_file(FileRecord {
            id: FileId::new(1),
            path: PathBuf::from("src/other.rs"),
            language: "rust".into(),
            ..Default::default()
        });
        g.add_callable(CallableNode {
            id: CallableId::new(0),
            qualified_name: "a".into(),
            file: FileId::new(0),
            synthetic: synthetic_src,
            kind: CallableKind::Function,
            ..Default::default()
        });
        g.add_callable(CallableNode {
            id: CallableId::new(1),
            qualified_name: "b".into(),
            file: FileId::new(1),
            kind: CallableKind::Function,
            ..Default::default()
        });
        g
    }

    #[test]
    fn one_site_is_path_and_line() {
        let g = graph(false);
        let mut list = SiteList::new();
        list.observe(&g, &edge(42, 1));
        assert_eq!(list.label().as_deref(), Some("src/lib.rs:42"));
    }

    #[test]
    fn repeated_sites_keep_every_line_in_order() {
        let g = graph(false);
        let mut list = SiteList::new();
        list.observe(&g, &edge(10, 1));
        list.observe(&g, &edge(4, 1));
        list.observe(&g, &edge(10, 1));
        assert_eq!(list.label().as_deref(), Some("src/lib.rs:10,4,10"));
    }

    #[test]
    fn an_aggregate_edge_drops_the_list() {
        let g = graph(false);
        let mut list = SiteList::new();
        list.observe(&g, &edge(10, 1));
        list.observe(&g, &edge(0, 4));
        assert_eq!(list.label(), None);
    }

    #[test]
    fn a_synthetic_caller_is_not_a_location() {
        let g = graph(true);
        let mut list = SiteList::new();
        list.observe(&g, &edge(10, 1));
        assert_eq!(list.label(), None);
    }
}
