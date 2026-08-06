//! Cycle and island detection over the dead subgraph.
//!
//! Two different groupings are needed, and conflating them produces a
//! bad report:
//!
//! * **Strongly-connected components** identify mutual recursion. An
//!   unreferenced SCC of size >= 2 has no entry point: every member is
//!   referenced, but only from inside the ring. This is the case a
//!   name-matching tool structurally cannot report, since every member
//!   counts as a use of the others.
//!
//! * **Weak components** identify the connected *group*. A chain
//!   `a -> b -> c` is three SCCs but one cluster, and reporting it as
//!   three independent findings overstates how much was actually found:
//!   `b` and `c` are contingent on `a`, not separate discoveries.
//!
//! Tarjan's algorithm is written iteratively: these groups are shallow
//! in real codebases, but a recursive version would put a
//! generator-controlled bound on the stack.

use super::adj::Adj;

/// Strongly-connected components of the subgraph induced by `members`.
///
/// Returns one entry per node index (`u32::MAX` for nodes outside the
/// subgraph) plus the components themselves, each sorted by node index
/// and emitted in ascending order of least member — a total order, so
/// the output is byte-stable.
pub(crate) fn tarjan_scc(adj: &Adj, in_sub: &[bool]) -> (Vec<u32>, Vec<Vec<u32>>) {
    let n = adj.n as usize;
    let mut index_of = vec![u32::MAX; n];
    let mut low = vec![0u32; n];
    let mut num = vec![u32::MAX; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<u32> = Vec::new();
    let mut comps: Vec<Vec<u32>> = Vec::new();
    let mut counter: u32 = 0;

    // Frame: (node, successor cursor). Iterating seeds in index order
    // keeps component discovery order deterministic.
    let mut work: Vec<(u32, u32)> = Vec::new();

    for start in 0..n as u32 {
        if !in_sub[start as usize] || num[start as usize] != u32::MAX {
            continue;
        }
        work.push((start, 0));

        while let Some(&mut (v, ref mut ci)) = work.last_mut() {
            if *ci == 0 {
                num[v as usize] = counter;
                low[v as usize] = counter;
                counter += 1;
                stack.push(v);
                on_stack[v as usize] = true;
            }

            let succs: Vec<u32> = adj
                .succ(v)
                .map(|(w, _)| w)
                .filter(|&w| in_sub[w as usize])
                .collect();

            if (*ci as usize) < succs.len() {
                let w = succs[*ci as usize];
                *ci += 1;
                if num[w as usize] == u32::MAX {
                    work.push((w, 0));
                } else if on_stack[w as usize] {
                    let lv = low[v as usize];
                    low[v as usize] = lv.min(num[w as usize]);
                }
                continue;
            }

            // v is finished.
            work.pop();
            if low[v as usize] == num[v as usize] {
                let mut comp: Vec<u32> = Vec::new();
                while let Some(w) = stack.pop() {
                    on_stack[w as usize] = false;
                    comp.push(w);
                    if w == v {
                        break;
                    }
                }
                comp.sort_unstable();
                let cid = comps.len() as u32;
                for &m in &comp {
                    index_of[m as usize] = cid;
                }
                comps.push(comp);
            }
            if let Some(&mut (p, _)) = work.last_mut() {
                let lp = low[p as usize];
                low[p as usize] = lp.min(low[v as usize]);
            }
        }
    }

    (index_of, comps)
}

/// Weak (undirected) components of the subgraph induced by `members`.
///
/// Component id is the least member index, so ids are stable across
/// runs and independent of discovery order.
pub(crate) fn weak_components(adj: &Adj, in_sub: &[bool]) -> Vec<u32> {
    let n = adj.n as usize;
    let mut comp = vec![u32::MAX; n];
    let mut queue: Vec<u32> = Vec::new();

    for start in 0..n as u32 {
        if !in_sub[start as usize] || comp[start as usize] != u32::MAX {
            continue;
        }
        // Seeding in ascending index order means the first node of a
        // component is its least member, so `start` is the id.
        comp[start as usize] = start;
        queue.push(start);
        while let Some(v) = queue.pop() {
            for (w, _) in adj.succ(v).chain(adj.pred(v)) {
                if in_sub[w as usize] && comp[w as usize] == u32::MAX {
                    comp[w as usize] = start;
                    queue.push(w);
                }
            }
        }
    }
    comp
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deadcode::testutil::mk_graph;

    fn all(n: usize) -> Vec<bool> {
        vec![true; n]
    }

    #[test]
    fn chain_has_no_nontrivial_scc() {
        let g = mk_graph(&[(0, 1), (1, 2)], 3);
        let adj = Adj::build(&g, &|_| true);
        let (_, comps) = tarjan_scc(&adj, &all(3));
        assert_eq!(comps.len(), 3);
        assert!(comps.iter().all(|c| c.len() == 1));
    }

    #[test]
    fn mutual_recursion_forms_one_scc() {
        // 0 <-> 1, and 2 dangling off it.
        let g = mk_graph(&[(0, 1), (1, 0), (1, 2)], 3);
        let adj = Adj::build(&g, &|_| true);
        let (_, comps) = tarjan_scc(&adj, &all(3));
        let big: Vec<_> = comps.iter().filter(|c| c.len() > 1).collect();
        assert_eq!(big.len(), 1);
        assert_eq!(*big[0], vec![0, 1]);
    }

    #[test]
    fn three_cycle_is_a_single_scc() {
        let g = mk_graph(&[(0, 1), (1, 2), (2, 0)], 3);
        let adj = Adj::build(&g, &|_| true);
        let (_, comps) = tarjan_scc(&adj, &all(3));
        assert_eq!(comps.len(), 1);
        assert_eq!(comps[0], vec![0, 1, 2]);
    }

    #[test]
    fn scc_respects_the_subgraph_mask() {
        let g = mk_graph(&[(0, 1), (1, 0)], 2);
        let adj = Adj::build(&g, &|_| true);
        // Exclude node 1: the cycle must not be found.
        let (_, comps) = tarjan_scc(&adj, &[true, false]);
        assert_eq!(comps.len(), 1);
        assert_eq!(comps[0], vec![0]);
    }

    #[test]
    fn a_chain_is_one_connected_group() {
        let g = mk_graph(&[(0, 1), (1, 2)], 3);
        let adj = Adj::build(&g, &|_| true);
        let comp = weak_components(&adj, &all(3));
        assert_eq!(comp, vec![0, 0, 0], "chain should be one group");
    }

    #[test]
    fn disjoint_groups_get_distinct_ids_named_by_least_member() {
        // {0,1} and {2,3}
        let g = mk_graph(&[(0, 1), (2, 3)], 4);
        let adj = Adj::build(&g, &|_| true);
        let comp = weak_components(&adj, &all(4));
        assert_eq!(comp, vec![0, 0, 2, 2]);
    }

    #[test]
    fn weak_components_join_across_edge_direction() {
        // 0 -> 2 <- 1 : undirected, all one group.
        let g = mk_graph(&[(0, 2), (1, 2)], 3);
        let adj = Adj::build(&g, &|_| true);
        let comp = weak_components(&adj, &all(3));
        assert_eq!(comp, vec![0, 0, 0]);
    }

    #[test]
    fn results_are_independent_of_edge_order() {
        let a = mk_graph(&[(0, 1), (1, 2), (2, 0)], 3);
        let b = mk_graph(&[(2, 0), (0, 1), (1, 2)], 3);
        let (aa, ba) = (Adj::build(&a, &|_| true), Adj::build(&b, &|_| true));
        let (_, ca) = tarjan_scc(&aa, &all(3));
        let (_, cb) = tarjan_scc(&ba, &all(3));
        assert_eq!(ca, cb);
        assert_eq!(weak_components(&aa, &all(3)), weak_components(&ba, &all(3)));
    }
}
