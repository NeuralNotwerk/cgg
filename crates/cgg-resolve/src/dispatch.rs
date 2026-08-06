//! Interface / trait dynamic-dispatch fan-out (Issue 3).
//!
//! A call through an abstract interface (`dyn Trait`, an interface
//! reference, a generic bound) resolves exactly to the interface's
//! method *declaration* — that edge is precise and is emitted by the
//! normal resolvers. What the declaration cannot tell a reader is *which
//! concrete implementation runs*. This stage adds the conservative
//! one-to-many answer: from each trait/interface method declaration, a
//! `Via::Dynamic` (low-confidence) edge to every concrete implementation
//! of that method.
//!
//! These edges over-approximate (a given call site reaches at most one
//! implementation at runtime), so they are tagged `Dynamic` and gated
//! behind `--dynamic-dispatch` at the driver. No flow analysis is
//! performed — the implements-relationship is read straight off
//! `CallableNode::trait_impl_target`, which the driver fills in from the
//! `<Type as Trait>::method` qualified-name form.

use std::collections::HashMap;

use cgg_core::{
    graph::{CallEdge, Confidence, Graph, Via},
    ids::{CallableId, ResolverId},
};

use crate::names::owner_from_qn;

/// Produce declaration → implementation fan-out edges for every trait
/// method that has both a declaration node and one or more concrete
/// implementations. Pure over the graph; emits no edge when a trait has
/// no implementations or no visible declaration.
pub fn fanout(graph: &Graph) -> Vec<CallEdge> {
    let resolver = ResolverId::new("dispatch:fanout");

    // (language, trait, method) -> concrete implementation method ids.
    let mut impls: HashMap<(String, String, String), Vec<CallableId>> = HashMap::new();
    // (language, owner, method) -> a declaration id. A trait method
    // declaration's owner segment *is* the trait name, so the keys align
    // with the `impls` keys above.
    let mut decls: HashMap<(String, String, String), CallableId> = HashMap::new();

    for c in graph.callables.values() {
        if let Some(tr) = &c.trait_impl_target {
            impls
                .entry((c.language.clone(), tr.clone(), c.simple_name.clone()))
                .or_default()
                .push(c.id);
        } else if let Some(owner) = owner_from_qn(&c.qualified_name) {
            decls
                .entry((c.language.clone(), owner.to_string(), c.simple_name.clone()))
                .or_insert(c.id);
        }
    }

    let mut edges = Vec::new();
    for (key, impl_ids) in &impls {
        let Some(&decl) = decls.get(key) else {
            continue;
        };
        // Anchor every fan-out edge at the declaration's own location.
        // Each edge has a distinct `dst`, so `dedup_edges` (keyed on
        // src+dst+site_byte) preserves them all.
        let (site_line, site_byte) = graph
            .callables
            .get(&decl)
            .map(|n| (n.start_line, n.start_byte))
            .unwrap_or((0, 0));
        for &impl_id in impl_ids {
            if impl_id == decl {
                continue;
            }
            edges.push(CallEdge {
                src: decl,
                dst: impl_id,
                site_line,
                site_byte,
                confidence: Confidence::Low,
                via: Via::Dynamic,
                resolver: resolver.clone(),
            });
        }
    }
    edges
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::graph::{CallableKind, CallableNode};
    use cgg_core::ids::FileId;

    fn node(id: u32, qn: &str, trait_impl: Option<&str>) -> CallableNode {
        CallableNode {
            id: CallableId::new(id),
            qualified_name: qn.into(),
            simple_name: qn.rsplit("::").next().unwrap_or(qn).into(),
            kind: CallableKind::Method,
            language: "rust".into(),
            file: FileId::new(0),
            start_line: 1,
            end_line: 1,
            start_byte: id,
            end_byte: id + 1,
            signature_hint: String::new(),
            visibility: String::new(),
            attributes: vec![],
            synthetic: false,
            trait_impl_target: trait_impl.map(|s| s.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn fans_out_declaration_to_each_impl() {
        let mut g = Graph::new();
        g.add_callable(node(0, "crate::Storage::put", None));
        g.add_callable(node(1, "crate::<DiskStorage as Storage>::put", Some("Storage")));
        g.add_callable(node(2, "crate::<MemStorage as Storage>::put", Some("Storage")));
        let edges = fanout(&g);
        assert_eq!(edges.len(), 2);
        assert!(edges.iter().all(|e| e.src == CallableId::new(0)));
        assert!(edges.iter().all(|e| matches!(e.via, Via::Dynamic)));
        let dsts: Vec<u32> = edges.iter().map(|e| e.dst.as_u32()).collect();
        assert!(dsts.contains(&1) && dsts.contains(&2));
    }

    #[test]
    fn no_decl_means_no_fanout() {
        // Implementations exist but the trait declaration is not in the
        // analyzed set — emit nothing rather than guess.
        let mut g = Graph::new();
        g.add_callable(node(1, "crate::<DiskStorage as Storage>::put", Some("Storage")));
        assert!(fanout(&g).is_empty());
    }
}
