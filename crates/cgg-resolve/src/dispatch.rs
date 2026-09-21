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
//! `<Type as Trait>::method` qualified-name form (Rust) or from a class
//! method's `DefRecord::base_types` (Python inheritance).

use std::collections::{BTreeSet, HashMap};

use cgg_core::facts::DefVariant;
use cgg_core::{
    graph::{CallEdge, Confidence, Graph, Via},
    ids::{CallableId, ResolverId},
    DefRecord, FileFacts,
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
    // Sorted. `impls` is a HashMap, and Rust's `RandomState` reseeds per
    // process, so iterating it directly emitted the fan-out edges in a
    // different order on every run — a nondeterministic graph whenever
    // `--dynamic-dispatch` was on, which `--dead-code` force-enables.
    // The edge SET was always correct; only the order varied, which is
    // exactly the kind of defect a byte-diff catches and a spot check
    // does not.
    let mut keys: Vec<&(String, String, String)> = impls.keys().collect();
    keys.sort_unstable();
    for key in keys {
        let impl_ids = &impls[key];
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
                weight: 1,
            });
        }
    }
    edges
}

/// Python class inheritance as declaration → override fan-out.
///
/// `class Child(Parent): def foo` is the same shape as a Rust trait
/// impl: a call that resolves to `Parent.foo` may run `Child.foo` at
/// runtime. Immediate bases are not enough — `class C(B): def foo`
/// still overrides `A.foo` when `B` does not define `foo` — so this
/// walks the recorded `base_types` chain (bounded, cyclic-safe) and
/// emits a `Via::Dynamic` edge from every ancestor method of the same
/// name.
///
/// Nested functions, constructors and trivial bases (`object`,
/// `Exception`, …) are skipped: those are not virtual overrides in the
/// sense dead-code cares about.
pub fn inheritance_fanout(graph: &Graph, facts: &[FileFacts]) -> Vec<CallEdge> {
    let resolver = ResolverId::new("dispatch:inheritance");

    let mut by_owner_method: HashMap<(String, String, String), Vec<CallableId>> =
        HashMap::new();
    let mut by_span: HashMap<(cgg_core::ids::FileId, u32), CallableId> = HashMap::new();
    for c in graph.callables.values() {
        if c.synthetic {
            continue;
        }
        by_span.insert((c.file, c.start_byte), c.id);
        if let Some(owner) = owner_from_qn(&c.qualified_name) {
            by_owner_method
                .entry((c.language.clone(), owner.to_string(), c.simple_name.clone()))
                .or_default()
                .push(c.id);
        }
    }

    let mut by_class_bases: HashMap<(String, String), Vec<String>> = HashMap::new();
    for f in facts {
        if f.language != "python" {
            continue;
        }
        for d in &f.definitions {
            if d.base_types.is_empty() {
                continue;
            }
            let Some(owner) = owner_from_qn(&d.qualified_name) else {
                continue;
            };
            if !looks_like_type(owner) {
                continue;
            }
            by_class_bases
                .entry((f.language.clone(), owner.to_string()))
                .or_insert_with(|| {
                    d.base_types
                        .iter()
                        .map(|b| type_stem(b).to_string())
                        .filter(|s| is_inheritable_base(s))
                        .collect()
                });
        }
    }

    let mut edges = Vec::new();
    let mut seen: BTreeSet<(CallableId, CallableId)> = BTreeSet::new();
    for f in facts {
        if f.language != "python" {
            continue;
        }
        for d in &f.definitions {
            if !is_override_candidate(d) {
                continue;
            }
            let Some(&child) = by_span.get(&(f.file, d.start_byte)) else {
                continue;
            };
            let Some(owner) = owner_from_qn(&d.qualified_name) else {
                continue;
            };
            for ancestor in walk_bases(&f.language, owner, &by_class_bases) {
                let Some(parents) = by_owner_method.get(&(
                    f.language.clone(),
                    ancestor.clone(),
                    d.simple_name.clone(),
                )) else {
                    continue;
                };
                for &parent in parents {
                    if parent == child || !seen.insert((parent, child)) {
                        continue;
                    }
                    let (site_line, site_byte) = graph
                        .callables
                        .get(&parent)
                        .map(|n| (n.start_line, n.start_byte))
                        .unwrap_or((0, 0));
                    edges.push(CallEdge {
                        src: parent,
                        dst: child,
                        site_line,
                        site_byte,
                        confidence: Confidence::Low,
                        via: Via::Dynamic,
                        resolver: resolver.clone(),
                        weight: 1,
                    });
                }
            }
        }
    }
    edges.sort_by_key(|e| (e.src, e.dst, e.site_byte));
    edges
}

/// First inheritable base of a Python method, for `trait_impl_target`.
pub fn inheritance_target(d: &DefRecord) -> Option<String> {
    if !is_override_candidate(d) {
        return None;
    }
    d.base_types
        .iter()
        .map(|b| type_stem(b).to_string())
        .find(|s| is_inheritable_base(s))
}

fn is_override_candidate(d: &DefRecord) -> bool {
    if d.base_types.is_empty() {
        return false;
    }
    if matches!(
        d.variant,
        DefVariant::Constructor
            | DefVariant::Destructor
            | DefVariant::FreeFunction
            | DefVariant::NamedLambda
            | DefVariant::NamedClosure
    ) {
        return false;
    }
    if matches!(d.simple_name.as_str(), "__init__" | "__new__" | "__del__") {
        return false;
    }
    owner_from_qn(&d.qualified_name).is_some_and(looks_like_type)
}

fn walk_bases(
    language: &str,
    start: &str,
    by_class_bases: &HashMap<(String, String), Vec<String>>,
) -> Vec<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut out = Vec::new();
    let mut frontier: Vec<String> = by_class_bases
        .get(&(language.to_string(), start.to_string()))
        .cloned()
        .unwrap_or_default();
    for _ in 0..8 {
        if frontier.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for t in frontier {
            if !seen.insert(t.clone()) {
                continue;
            }
            out.push(t.clone());
            if let Some(parents) = by_class_bases.get(&(language.to_string(), t)) {
                next.extend(parents.iter().cloned());
            }
        }
        frontier = next;
    }
    out
}

fn type_stem(name: &str) -> &str {
    let s = name.split(['<', '[']).next().unwrap_or(name).trim();
    s.rsplit(['.', ':']).next().unwrap_or(s)
}

fn looks_like_type(name: &str) -> bool {
    name.trim_start_matches('_')
        .chars()
        .next()
        .is_some_and(|c| c.is_uppercase())
}

fn is_inheritable_base(name: &str) -> bool {
    looks_like_type(name)
        && !matches!(
            name,
            "object"
                | "type"
                | "Enum"
                | "IntEnum"
                | "StrEnum"
                | "ABC"
                | "ABCMeta"
                | "Generic"
                | "Protocol"
                | "TypedDict"
                | "NamedTuple"
                | "Exception"
                | "BaseException"
        )
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
        g.add_callable(node(
            1,
            "crate::<DiskStorage as Storage>::put",
            Some("Storage"),
        ));
        g.add_callable(node(
            2,
            "crate::<MemStorage as Storage>::put",
            Some("Storage"),
        ));
        let edges = fanout(&g);
        assert_eq!(edges.len(), 2);
        assert!(edges.iter().all(|e| e.src == CallableId::new(0)));
        assert!(edges.iter().all(|e| matches!(e.via, Via::Dynamic)));
        let dsts: Vec<u64> = edges.iter().map(|e| e.dst.as_u64()).collect();
        assert!(dsts.contains(&1) && dsts.contains(&2));
    }

    #[test]
    fn no_decl_means_no_fanout() {
        // Implementations exist but the trait declaration is not in the
        // analyzed set — emit nothing rather than guess.
        let mut g = Graph::new();
        g.add_callable(node(
            1,
            "crate::<DiskStorage as Storage>::put",
            Some("Storage"),
        ));
        assert!(fanout(&g).is_empty());
    }

    fn py_node(id: u32, qn: &str, start: u32) -> CallableNode {
        CallableNode {
            id: CallableId::new(id),
            qualified_name: qn.into(),
            simple_name: qn.rsplit('.').next().unwrap_or(qn).into(),
            kind: CallableKind::Method,
            language: "python".into(),
            file: FileId::new(0),
            start_line: 1,
            end_line: 1,
            start_byte: start,
            end_byte: start + 10,
            ..Default::default()
        }
    }

    fn py_def(simple: &str, qn: &str, start: u32, bases: &[&str]) -> DefRecord {
        DefRecord {
            simple_name: simple.into(),
            qualified_name: qn.into(),
            variant: DefVariant::InherentMethod,
            start_byte: start,
            end_byte: start + 10,
            base_types: bases.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn python_override_fans_out_from_the_base_method() {
        let mut g = Graph::new();
        g.add_callable(py_node(0, "ops.OperationsBase.generate", 10));
        g.add_callable(py_node(1, "ops.BoreOperation.generate", 40));
        let mut f =
            FileFacts::new(FileId::new(0), std::path::PathBuf::from("ops.py"), "python");
        f.definitions.push(py_def(
            "generate",
            "ops.OperationsBase.generate",
            10,
            &["ABC"],
        ));
        f.definitions.push(py_def(
            "generate",
            "ops.BoreOperation.generate",
            40,
            &["OperationsBase"],
        ));
        let edges = inheritance_fanout(&g, &[f]);
        assert_eq!(edges.len(), 1, "{edges:?}");
        assert_eq!(edges[0].src, CallableId::new(0));
        assert_eq!(edges[0].dst, CallableId::new(1));
        assert!(matches!(edges[0].via, Via::Dynamic));
    }

    #[test]
    fn python_override_walks_past_a_base_that_does_not_define_the_method() {
        let mut g = Graph::new();
        g.add_callable(py_node(0, "ops.A.foo", 10));
        g.add_callable(py_node(1, "ops.B.other", 30));
        g.add_callable(py_node(2, "ops.C.foo", 50));
        let mut f =
            FileFacts::new(FileId::new(0), std::path::PathBuf::from("ops.py"), "python");
        f.definitions
            .push(py_def("foo", "ops.A.foo", 10, &["object"]));
        f.definitions
            .push(py_def("other", "ops.B.other", 30, &["A"]));
        f.definitions.push(py_def("foo", "ops.C.foo", 50, &["B"]));
        let edges = inheritance_fanout(&g, &[f]);
        assert_eq!(edges.len(), 1, "{edges:?}");
        assert_eq!(edges[0].src, CallableId::new(0));
        assert_eq!(edges[0].dst, CallableId::new(2));
    }

    #[test]
    fn python_nested_function_is_not_an_override() {
        let mut g = Graph::new();
        g.add_callable(py_node(0, "ops.A.foo", 10));
        g.add_callable(py_node(1, "ops.B.foo.foo", 40));
        let mut f =
            FileFacts::new(FileId::new(0), std::path::PathBuf::from("ops.py"), "python");
        f.definitions.push(py_def("foo", "ops.A.foo", 10, &[]));
        f.definitions
            .push(py_def("foo", "ops.B.foo.foo", 40, &["A"]));
        let edges = inheritance_fanout(&g, &[f]);
        assert!(edges.is_empty(), "{edges:?}");
    }

    #[test]
    fn python_underscore_prefixed_class_is_an_override() {
        let mut g = Graph::new();
        g.add_callable(py_node(0, "ui.ToolTipButton.on_mouse_pos", 10));
        g.add_callable(py_node(
            1,
            "ui.PlayProgressBar._MarkerHoverToolTip.on_mouse_pos",
            40,
        ));
        let mut f =
            FileFacts::new(FileId::new(0), std::path::PathBuf::from("ui.py"), "python");
        f.definitions.push(py_def(
            "on_mouse_pos",
            "ui.ToolTipButton.on_mouse_pos",
            10,
            &["Button"],
        ));
        f.definitions.push(py_def(
            "on_mouse_pos",
            "ui.PlayProgressBar._MarkerHoverToolTip.on_mouse_pos",
            40,
            &["ToolTipButton"],
        ));
        let edges = inheritance_fanout(&g, &[f]);
        assert_eq!(edges.len(), 1, "{edges:?}");
        assert_eq!(edges[0].src, CallableId::new(0));
        assert_eq!(edges[0].dst, CallableId::new(1));
    }

    #[test]
    fn python_init_is_not_fanned_out() {
        let mut g = Graph::new();
        g.add_callable(py_node(0, "ops.A.__init__", 10));
        g.add_callable(py_node(1, "ops.B.__init__", 40));
        let mut f =
            FileFacts::new(FileId::new(0), std::path::PathBuf::from("ops.py"), "python");
        f.definitions
            .push(py_def("__init__", "ops.A.__init__", 10, &["object"]));
        let mut child = py_def("__init__", "ops.B.__init__", 40, &["A"]);
        child.variant = DefVariant::Constructor;
        f.definitions.push(child);
        assert!(inheritance_fanout(&g, &[f]).is_empty());
    }
}
