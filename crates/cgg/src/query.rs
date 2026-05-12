//! Query engine — filter, N-hop neighborhood, and full-path enumeration.
//!
//! * `--filter PATTERN` selects seed nodes by regex or glob match on
//!   qualified names.
//! * `-n N` (N > 0) expands the seed set to all nodes within N hops.
//! * `-n 0` enumerates all entry-to-exit paths passing through seeds,
//!   capped by `--max-paths`.
//! * No filter → full graph (no pruning).

use std::collections::{HashSet, VecDeque};

use cgg_core::graph::Graph;
use cgg_core::ids::CallableId;
use regex::Regex;

/// Apply filter + hop logic, returning a pruned graph.
pub fn apply_query(graph: &Graph, filters: &[String], hops: i32, max_paths: u32) -> Graph {
    if filters.is_empty() {
        return graph.clone();
    }

    let seeds = find_seeds(graph, filters);
    if seeds.is_empty() {
        return Graph::new();
    }

    if hops == 0 {
        // Full-path mode: find all entry→exit paths through seeds.
        return paths_through(graph, &seeds, max_paths);
    }

    // N-hop neighborhood (hops > 0, or -1 which we treat as "seeds only
    // with all their direct edges" — but per spec -1 means full graph
    // which is handled above by the empty-filter check).
    let depth = if hops < 0 { 1 } else { hops as u32 };
    neighborhood(graph, &seeds, depth)
}

/// Remove nodes matching any exclusion pattern from the graph.
/// Applied after --filter + -n.
pub fn apply_exclusions(
    graph: &Graph,
    partials: &[String],
    globs: &[String],
    regexes: &[String],
) -> Graph {
    if partials.is_empty() && globs.is_empty() && regexes.is_empty() {
        return graph.clone();
    }

    let glob_pats: Vec<glob::Pattern> = globs.iter()
        .filter_map(|g| glob::Pattern::new(g).ok())
        .collect();
    let regex_pats: Vec<Regex> = regexes.iter()
        .filter_map(|r| Regex::new(r).ok())
        .collect();

    let keep: HashSet<CallableId> = graph.callables.values()
        .filter(|c| {
            let qn = &c.qualified_name;
            // Exclude if any pattern matches
            if partials.iter().any(|p| qn.contains(p.as_str())) {
                return false;
            }
            if glob_pats.iter().any(|g| g.matches(qn)) {
                return false;
            }
            if regex_pats.iter().any(|r| r.is_match(qn)) {
                return false;
            }
            true
        })
        .map(|c| c.id)
        .collect();

    prune(graph, &keep)
}

fn find_seeds(graph: &Graph, filters: &[String]) -> HashSet<CallableId> {
    let mut seeds = HashSet::new();
    let patterns: Vec<Pattern> = filters.iter().map(|f| Pattern::new(f)).collect();
    for c in graph.callables.values() {
        if patterns.iter().any(|p| p.matches(&c.qualified_name)) {
            seeds.insert(c.id);
        }
    }
    seeds
}

fn neighborhood(graph: &Graph, seeds: &HashSet<CallableId>, depth: u32) -> Graph {
    // BFS in both directions from seeds.
    let mut visited: HashSet<CallableId> = seeds.clone();
    let mut frontier: VecDeque<(CallableId, u32)> =
        seeds.iter().map(|&id| (id, 0)).collect();

    // Build adjacency (both directions).
    let mut fwd: std::collections::HashMap<CallableId, Vec<CallableId>> = Default::default();
    let mut rev: std::collections::HashMap<CallableId, Vec<CallableId>> = Default::default();
    for e in &graph.edges {
        fwd.entry(e.src).or_default().push(e.dst);
        rev.entry(e.dst).or_default().push(e.src);
    }

    while let Some((node, d)) = frontier.pop_front() {
        if d >= depth {
            continue;
        }
        for &next in fwd.get(&node).unwrap_or(&Vec::new()) {
            if visited.insert(next) {
                frontier.push_back((next, d + 1));
            }
        }
        for &next in rev.get(&node).unwrap_or(&Vec::new()) {
            if visited.insert(next) {
                frontier.push_back((next, d + 1));
            }
        }
    }

    prune(graph, &visited)
}

fn paths_through(graph: &Graph, seeds: &HashSet<CallableId>, max_paths: u32) -> Graph {
    // Find all nodes on any path from an entry (in-degree 0) to an
    // exit (out-degree 0) that passes through at least one seed.
    let mut fwd: std::collections::HashMap<CallableId, Vec<CallableId>> = Default::default();
    let mut in_degree: std::collections::HashMap<CallableId, u32> = Default::default();
    let mut out_degree: std::collections::HashMap<CallableId, u32> = Default::default();
    for c in graph.callables.keys() {
        in_degree.entry(*c).or_insert(0);
        out_degree.entry(*c).or_insert(0);
    }
    for e in &graph.edges {
        fwd.entry(e.src).or_default().push(e.dst);
        *in_degree.entry(e.dst).or_insert(0) += 1;
        *out_degree.entry(e.src).or_insert(0) += 1;
    }

    let entries: Vec<CallableId> = in_degree
        .iter()
        .filter(|(_, &d)| d == 0)
        .map(|(&id, _)| id)
        .collect();

    // DFS from each entry, collecting nodes on paths that hit a seed.
    let mut on_path: HashSet<CallableId> = HashSet::new();
    let mut path_count: u32 = 0;

    for &entry in &entries {
        if path_count >= max_paths {
            break;
        }
        let mut stack: Vec<CallableId> = Vec::new();
        let mut visited_local: HashSet<CallableId> = HashSet::new();
        dfs_paths(
            entry,
            &fwd,
            &out_degree,
            seeds,
            &mut stack,
            &mut visited_local,
            &mut on_path,
            &mut path_count,
            max_paths,
        );
    }

    prune(graph, &on_path)
}

fn dfs_paths(
    node: CallableId,
    fwd: &std::collections::HashMap<CallableId, Vec<CallableId>>,
    out_degree: &std::collections::HashMap<CallableId, u32>,
    seeds: &HashSet<CallableId>,
    stack: &mut Vec<CallableId>,
    visited: &mut HashSet<CallableId>,
    on_path: &mut HashSet<CallableId>,
    count: &mut u32,
    max: u32,
) {
    if *count >= max {
        return;
    }
    if visited.contains(&node) {
        return;
    }
    visited.insert(node);
    stack.push(node);

    let is_exit = out_degree.get(&node).copied().unwrap_or(0) == 0;
    if is_exit {
        // Check if path contains a seed.
        if stack.iter().any(|n| seeds.contains(n)) {
            on_path.extend(stack.iter().copied());
            *count += 1;
        }
    } else if let Some(nexts) = fwd.get(&node) {
        for &next in nexts {
            dfs_paths(next, fwd, out_degree, seeds, stack, visited, on_path, count, max);
        }
    }

    stack.pop();
    visited.remove(&node);
}

fn prune(graph: &Graph, keep: &HashSet<CallableId>) -> Graph {
    let mut out = Graph::new();
    // Copy files that have at least one kept callable.
    let kept_files: HashSet<_> = graph.callables.values()
        .filter(|c| keep.contains(&c.id))
        .map(|c| c.file)
        .collect();
    for (fid, frec) in &graph.files {
        if kept_files.contains(fid) {
            out.files.insert(*fid, frec.clone());
        }
    }
    for (id, c) in &graph.callables {
        if keep.contains(id) {
            out.callables.insert(*id, c.clone());
        }
    }
    for e in &graph.edges {
        if keep.contains(&e.src) && keep.contains(&e.dst) {
            out.edges.push(e.clone());
        }
    }
    out.metrics = graph.metrics.clone();
    out
}

enum Pattern {
    Regex(Regex),
    Glob(glob::Pattern),
}

impl Pattern {
    fn new(s: &str) -> Self {
        if let Some(g) = s.strip_prefix("glob:") {
            Pattern::Glob(glob::Pattern::new(g).unwrap_or_else(|_| glob::Pattern::new("*").unwrap()))
        } else {
            Pattern::Regex(Regex::new(s).unwrap_or_else(|_| Regex::new(".*").unwrap()))
        }
    }

    fn matches(&self, name: &str) -> bool {
        match self {
            Pattern::Regex(r) => r.is_match(name),
            Pattern::Glob(g) => g.matches(name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::graph::{CallEdge, CallableKind, CallableNode, Confidence, FileRecord, Via};
    use cgg_core::ids::{FileId, ResolverId};
    use std::path::PathBuf;

    fn mk_graph() -> Graph {
        let mut g = Graph::new();
        g.add_file(FileRecord {
            id: FileId::new(0), path: PathBuf::from("a.rs"),
            language: "rust".into(), detected_via: "ext".into(),
            blake3: "0".repeat(64), size_bytes: 10, lines: 5,
            parse_ms: 0.1, parse_status: "ok".into(),
        });
        for i in 0..4u32 {
            g.add_callable(CallableNode {
                id: CallableId::new(i),
                qualified_name: format!("fn_{i}"),
                simple_name: format!("fn_{i}"),
                kind: CallableKind::Function,
                language: "rust".into(),
                file: FileId::new(0),
                start_line: i + 1, end_line: i + 1,
                start_byte: i * 10, end_byte: (i + 1) * 10,
                signature_hint: String::new(), visibility: String::new(),
                attributes: vec![],
            });
        }
        // Chain: fn_0 -> fn_1 -> fn_2 -> fn_3
        for i in 0..3u32 {
            g.add_edge(CallEdge {
                src: CallableId::new(i), dst: CallableId::new(i + 1),
                site_line: i + 1, site_byte: i * 10 + 5,
                confidence: Confidence::High, via: Via::Direct,
                resolver: ResolverId::new("test"),
            });
        }
        g
    }

    #[test]
    fn filter_selects_seed() {
        let g = mk_graph();
        let out = apply_query(&g, &["fn_1".into()], 1, 100);
        // 1-hop from fn_1: fn_0, fn_1, fn_2
        assert_eq!(out.callables.len(), 3);
    }

    #[test]
    fn n_zero_paths_through() {
        let g = mk_graph();
        // fn_0 is entry, fn_3 is exit. Path fn_0->fn_1->fn_2->fn_3
        // passes through fn_2.
        let out = apply_query(&g, &["fn_2".into()], 0, 100);
        assert_eq!(out.callables.len(), 4); // entire chain
    }

    #[test]
    fn glob_pattern() {
        let g = mk_graph();
        let out = apply_query(&g, &["glob:fn_[01]".into()], 1, 100);
        // Seeds: fn_0, fn_1. 1-hop adds fn_2.
        assert_eq!(out.callables.len(), 3);
    }
}
