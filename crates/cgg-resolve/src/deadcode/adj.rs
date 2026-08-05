//! Compressed adjacency over the call graph.
//!
//! Every traversal in the dead-code engine runs over dense `u32` node
//! indices — a node's position in `Graph::callables`, which is an
//! `IndexMap` and therefore insertion-ordered. That makes iteration
//! order *array order*, so results are deterministic by construction
//! rather than by convention. A `HashMap<CallableId, Vec<CallableId>>`
//! adjacency (as `query.rs` builds) would reintroduce hash ordering into
//! the hot loop and into anything derived from it.
//!
//! The structure is a counting-sorted CSR built in one pass, carrying a
//! parallel array of edge indices so a traversal can cite the actual
//! `CallEdge` that justified each hop — which `--why-live` needs.

use cgg_core::graph::{CallEdge, Graph};
use cgg_core::ids::CallableId;

/// Forward and reverse adjacency in compressed sparse row form.
#[derive(Debug)]
pub(crate) struct Adj {
    pub(crate) n: u32,
    fwd_off: Vec<u32>,
    fwd_dst: Vec<u32>,
    fwd_edge: Vec<u32>,
    rev_off: Vec<u32>,
    rev_src: Vec<u32>,
    rev_edge: Vec<u32>,
}

impl Adj {
    /// Build from `graph`, keeping only edges for which `keep` returns
    /// true and whose endpoints both exist.
    pub(crate) fn build(graph: &Graph, keep: &dyn Fn(&CallEdge) -> bool) -> Self {
        let n = graph.callables.len() as u32;
        let idx = |id: CallableId| -> Option<u32> {
            graph.callables.get_index_of(&id).map(|i| i as u32)
        };

        let mut pairs: Vec<(u32, u32, u32)> = Vec::new(); // (src, dst, edge_idx)
        for (ei, e) in graph.edges.iter().enumerate() {
            if !keep(e) {
                continue;
            }
            if let (Some(s), Some(d)) = (idx(e.src), idx(e.dst)) {
                pairs.push((s, d, ei as u32));
            }
        }

        let build_csr = |key: fn(&(u32, u32, u32)) -> u32,
                         val: fn(&(u32, u32, u32)) -> u32|
         -> (Vec<u32>, Vec<u32>, Vec<u32>) {
            let mut counts = vec![0u32; n as usize + 1];
            for p in &pairs {
                counts[key(p) as usize + 1] += 1;
            }
            for i in 0..n as usize {
                counts[i + 1] += counts[i];
            }
            let off = counts.clone();
            let mut cursor = counts;
            let mut out_val = vec![0u32; pairs.len()];
            let mut out_edge = vec![0u32; pairs.len()];
            // `pairs` is in graph.edges order, so equal keys stay in
            // edge order: a stable, reproducible layout.
            for p in &pairs {
                let slot = cursor[key(p) as usize] as usize;
                out_val[slot] = val(p);
                out_edge[slot] = p.2;
                cursor[key(p) as usize] += 1;
            }
            (off, out_val, out_edge)
        };

        let (fwd_off, fwd_dst, fwd_edge) = build_csr(|p| p.0, |p| p.1);
        let (rev_off, rev_src, rev_edge) = build_csr(|p| p.1, |p| p.0);

        Self { n, fwd_off, fwd_dst, fwd_edge, rev_off, rev_src, rev_edge }
    }

    /// Successors of `v` as `(node, edge_index)`.
    pub(crate) fn succ(&self, v: u32) -> impl Iterator<Item = (u32, u32)> + '_ {
        let lo = self.fwd_off[v as usize] as usize;
        let hi = self.fwd_off[v as usize + 1] as usize;
        (lo..hi).map(move |i| (self.fwd_dst[i], self.fwd_edge[i]))
    }

    /// Predecessors of `v` as `(node, edge_index)`.
    pub(crate) fn pred(&self, v: u32) -> impl Iterator<Item = (u32, u32)> + '_ {
        let lo = self.rev_off[v as usize] as usize;
        let hi = self.rev_off[v as usize + 1] as usize;
        (lo..hi).map(move |i| (self.rev_src[i], self.rev_edge[i]))
    }

    pub(crate) fn in_degree(&self, v: u32) -> u32 {
        self.rev_off[v as usize + 1] - self.rev_off[v as usize]
    }

    pub(crate) fn out_degree(&self, v: u32) -> u32 {
        self.fwd_off[v as usize + 1] - self.fwd_off[v as usize]
    }

    /// Forward BFS from `seeds`, returning the reachable set as a
    /// bitmask over node indices.
    pub(crate) fn reachable_from(&self, seeds: &[u32]) -> Vec<bool> {
        let mut seen = vec![false; self.n as usize];
        let mut stack: Vec<u32> = Vec::with_capacity(seeds.len());
        for &s in seeds {
            if (s as usize) < seen.len() && !seen[s as usize] {
                seen[s as usize] = true;
                stack.push(s);
            }
        }
        while let Some(v) = stack.pop() {
            for (w, _) in self.succ(v) {
                if !seen[w as usize] {
                    seen[w as usize] = true;
                    stack.push(w);
                }
            }
        }
        seen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deadcode::testutil::mk_graph;

    #[test]
    fn degrees_and_adjacency_match_the_edges() {
        // 0 -> 1 -> 2, plus 0 -> 2
        let g = mk_graph(&[(0, 1), (1, 2), (0, 2)], 3);
        let adj = Adj::build(&g, &|_| true);
        assert_eq!(adj.n, 3);
        assert_eq!(adj.out_degree(0), 2);
        assert_eq!(adj.in_degree(0), 0);
        assert_eq!(adj.in_degree(2), 2);
        let succ: Vec<u32> = adj.succ(0).map(|(d, _)| d).collect();
        assert_eq!(succ, vec![1, 2]);
        let pred: Vec<u32> = adj.pred(2).map(|(s, _)| s).collect();
        assert_eq!(pred, vec![1, 0]);
    }

    #[test]
    fn edge_indices_point_back_at_the_real_edge() {
        let g = mk_graph(&[(0, 1), (1, 2)], 3);
        let adj = Adj::build(&g, &|_| true);
        for v in 0..adj.n {
            for (w, ei) in adj.succ(v) {
                let e = &g.edges[ei as usize];
                assert_eq!(g.callables.get_index_of(&e.src).unwrap() as u32, v);
                assert_eq!(g.callables.get_index_of(&e.dst).unwrap() as u32, w);
            }
        }
    }

    #[test]
    fn keep_predicate_filters_edges() {
        let g = mk_graph(&[(0, 1), (1, 2)], 3);
        let adj = Adj::build(&g, &|e| e.site_line != 1);
        // mk_graph sets site_line = index + 1, so edge 0 is dropped.
        assert_eq!(adj.out_degree(0), 0);
        assert_eq!(adj.out_degree(1), 1);
    }

    #[test]
    fn reachable_from_follows_forward_edges_only() {
        let g = mk_graph(&[(0, 1), (1, 2), (3, 2)], 4);
        let adj = Adj::build(&g, &|_| true);
        let live = adj.reachable_from(&[0]);
        assert_eq!(live, vec![true, true, true, false]);
    }

    #[test]
    fn adjacency_is_independent_of_edge_insertion_order() {
        let a = mk_graph(&[(0, 1), (1, 2), (0, 2)], 3);
        let b = mk_graph(&[(0, 2), (1, 2), (0, 1)], 3);
        let (aa, ba) = (Adj::build(&a, &|_| true), Adj::build(&b, &|_| true));
        for v in 0..3 {
            let mut x: Vec<u32> = aa.succ(v).map(|(d, _)| d).collect();
            let mut y: Vec<u32> = ba.succ(v).map(|(d, _)| d).collect();
            x.sort_unstable();
            y.sort_unstable();
            assert_eq!(x, y, "successors of {v} differ");
            assert_eq!(aa.in_degree(v), ba.in_degree(v));
        }
    }
}
