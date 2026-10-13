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
//! `<Type as Trait>::method` qualified-name form (Rust). Python class
//! inheritance is a separate pass (`inheritance_fanout`) over
//! `DefRecord::base_types`; it must not write `trait_impl_target`, or
//! `fanout` would emit declaration→override edges for every language
//! that records bases, including non-virtual C++ methods.

use std::collections::{BTreeSet, HashMap};

use cgg_core::facts::DefVariant;
use cgg_core::{
    DefRecord, FileFacts,
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

/// Languages whose methods dispatch on the runtime type of the receiver,
/// so a call that lands on a base class's or interface's method may run
/// a same-named method of a class below it. Each records its classes'
/// bases (`DefRecord::base_types`, and `FileFacts::classes` where the
/// plugin writes them). Not C++ — its methods are virtual only when
/// declared so, which `cpp_virtual_dispatch` reads — and not Rust, whose
/// implementations name their trait (`fanout`).
pub(crate) const INHERITANCE_LANGS: &[&str] = &[
    "python",
    "java",
    "csharp",
    "kotlin",
    "scala",
    "groovy",
    "typescript",
    "javascript",
    "swift",
    "dart",
    "php",
    "ruby",
];

/// Languages whose plugin records every class with exactly its own bases
/// (`FileFacts::classes`). For them a method record's `base_types` is
/// not read into its owner's bases: a Java anonymous class body's
/// methods carry the anonymous class's base, which the enclosing class
/// does not have.
pub(crate) const CLASS_RECORDS_ARE_EXACT: &[&str] = &["java", "csharp"];

/// Every class the analyzed tree declares and the bases each names, per
/// language, keyed by bare class name (the form a qualified name's owner
/// segment and a base list both reduce to). Same-named classes in
/// different packages share a key; their bases are unioned, which can
/// only add an ancestor — the over-approximating direction, for a pass
/// whose edges only ever mark code live.
pub(crate) struct ClassHierarchy {
    /// (language, class) -> bases, filtered of trivial roots, sorted.
    bases: HashMap<(String, String), Vec<String>>,
    /// (language, class) for every class declared in the tree: a class
    /// record, or the owner of a method definition.
    declared: std::collections::HashSet<(String, String)>,
    /// Memoized [`ClassHierarchy::ancestors`]: dead-code asks for the same
    /// class's chain once per finding and per unresolved site.
    /// Keyed language → class, so a hit allocates nothing.
    #[allow(clippy::type_complexity)]
    memo: std::cell::RefCell<HashMap<String, HashMap<String, std::rc::Rc<Vec<String>>>>>,
}

impl ClassHierarchy {
    pub(crate) fn build(facts: &[FileFacts]) -> Self {
        let mut bases: HashMap<(String, String), Vec<String>> = HashMap::new();
        let mut declared: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();
        let mut add = |lang: &str, owner: &str, list: &[String]| {
            let slot = bases
                .entry((lang.to_string(), owner.to_string()))
                .or_default();
            for b in list {
                let s = type_stem(b);
                if is_inheritable_base(s) && s != owner && !slot.iter().any(|x| x == s) {
                    slot.push(s.to_string());
                }
            }
        };
        for f in facts {
            if !INHERITANCE_LANGS.contains(&f.language.as_str()) {
                continue;
            }
            // Every class the plugin saw, including `class Middle(Base):
            // pass`. A bodiless class has no method record, so seeding
            // from definitions alone would end the chain at it and
            // `Leaf.run` would never link to `Base.run`.
            for c in &f.classes {
                let name = type_stem(&c.class_qn);
                if !looks_like_type(name) {
                    continue;
                }
                declared.insert((f.language.clone(), name.to_string()));
                add(&f.language, name, &c.base_types);
            }
            for d in &f.definitions {
                let Some(owner) = owner_from_qn(&d.qualified_name) else {
                    continue;
                };
                let owner = type_stem(owner);
                if !looks_like_type(owner) {
                    continue;
                }
                declared.insert((f.language.clone(), owner.to_string()));
                if !d.base_types.is_empty()
                    && !CLASS_RECORDS_ARE_EXACT.contains(&f.language.as_str())
                {
                    add(&f.language, owner, &d.base_types);
                }
            }
        }
        for v in bases.values_mut() {
            v.sort();
        }
        ClassHierarchy {
            bases,
            declared,
            memo: Default::default(),
        }
    }

    /// The ancestors of `owner`, breadth first, bounded (depth 8) and
    /// cycle-safe. Includes ancestors the tree does not declare.
    pub(crate) fn ancestors(
        &self,
        language: &str,
        owner: &str,
    ) -> std::rc::Rc<Vec<String>> {
        if let Some(v) = self.memo.borrow().get(language).and_then(|m| m.get(owner)) {
            return v.clone();
        }
        let mut v = walk_bases(language, owner, &self.bases);
        v.retain(|a| a != owner);
        let v = std::rc::Rc::new(v);
        self.memo
            .borrow_mut()
            .entry(language.to_string())
            .or_default()
            .insert(owner.to_string(), v.clone());
        v
    }

    /// Whether the tree declares a class of this name.
    pub(crate) fn declares(&self, language: &str, class: &str) -> bool {
        self.declared
            .contains(&(language.to_string(), class.to_string()))
    }

    /// (language, base) -> the classes naming it as a base, sorted.
    pub(crate) fn derived_index(&self) -> HashMap<(String, String), Vec<String>> {
        let mut out: HashMap<(String, String), Vec<String>> = HashMap::new();
        for ((lang, owner), bases) in &self.bases {
            for b in bases {
                out.entry((lang.clone(), b.clone()))
                    .or_default()
                    .push(owner.clone());
            }
        }
        for v in out.values_mut() {
            v.sort();
            v.dedup();
        }
        out
    }
}

/// Class inheritance as declaration → override fan-out.
///
/// `class Child(Parent): def foo` is the same shape as a Rust trait
/// impl: a call that resolves to `Parent.foo` may run `Child.foo` at
/// runtime. Immediate bases are not enough — `class C(B): def foo`
/// still overrides `A.foo` when `B` does not define `foo` — so this
/// walks the recorded bases (bounded, cyclic-safe) and emits a
/// `Via::Dynamic` edge from every ancestor method of the same name.
/// Java, C#, Kotlin, Scala, TypeScript and the other
/// [`INHERITANCE_LANGS`] dispatch the same way, interfaces included: an
/// `implements Shape` class's `draw` runs for a call typed `Shape.draw`.
///
/// Nested functions, constructors, static methods (outside Python, whose
/// `@staticmethod` is still looked up through `self`) and trivial bases
/// (`object`, `Exception`, …) are skipped: those are not virtual
/// overrides in the sense dead-code cares about.
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
                .entry((
                    c.language.clone(),
                    type_stem(owner).to_string(),
                    c.simple_name.clone(),
                ))
                .or_default()
                .push(c.id);
        }
    }

    let hierarchy = ClassHierarchy::build(facts);

    let mut edges = Vec::new();
    let mut seen: BTreeSet<(CallableId, CallableId)> = BTreeSet::new();
    for f in facts {
        if !INHERITANCE_LANGS.contains(&f.language.as_str()) {
            continue;
        }
        for d in &f.definitions {
            if !is_override_candidate(&f.language, d) {
                continue;
            }
            let Some(&child) = by_span.get(&(f.file, d.start_byte)) else {
                continue;
            };
            let Some(owner) = owner_from_qn(&d.qualified_name) else {
                continue;
            };
            // The class's ancestors, plus — for a Java anonymous class
            // body — the anonymous class's base and its ancestors.
            let mut up: Vec<String> =
                hierarchy.ancestors(&f.language, type_stem(owner)).to_vec();
            for b in &d.base_types {
                let b = type_stem(b);
                if !is_inheritable_base(b) || b == type_stem(owner) {
                    continue;
                }
                for a in std::iter::once(b.to_string())
                    .chain(hierarchy.ancestors(&f.language, b).iter().cloned())
                {
                    if !up.contains(&a) {
                        up.push(a);
                    }
                }
            }
            for ancestor in up {
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

/// The Scala and Dart plugins record every function as a free function,
/// members of a class, object, trait or mixin included; the owner segment
/// of the qualified name is what makes one a member.
pub(crate) fn free_variant_is_member(language: &str) -> bool {
    matches!(language, "scala" | "dart")
}

/// Whether a definition can override a same-named member of a base.
///
/// Bases are walked from the owner's class record too, so a method
/// whose own record carries no bases still qualifies when its class
/// has some.
pub(crate) fn is_override_candidate(language: &str, d: &DefRecord) -> bool {
    if matches!(
        d.variant,
        DefVariant::Constructor
            | DefVariant::Destructor
            | DefVariant::NamedLambda
            | DefVariant::NamedClosure
    ) {
        return false;
    }
    if d.variant == DefVariant::FreeFunction && !free_variant_is_member(language) {
        return false;
    }
    // A static member hides rather than overrides, except in Python,
    // where `self.m()` still looks a `@staticmethod` up on the instance.
    if d.variant == DefVariant::StaticMethod && language != "python" {
        return false;
    }
    // A C# accessor is named for its keyword (`get`/`set`), not its
    // property: two same-named accessors say nothing about each other.
    if d.variant == DefVariant::Property && language == "csharp" {
        return false;
    }
    if matches!(d.simple_name.as_str(), "__init__" | "__new__" | "__del__") {
        return false;
    }
    owner_from_qn(&d.qualified_name).is_some_and(|o| looks_like_type(type_stem(o)))
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

pub(crate) fn type_stem(name: &str) -> &str {
    let s = name.split(['<', '[']).next().unwrap_or(name).trim();
    s.rsplit(['.', ':', '\\']).next().unwrap_or(s)
}

fn looks_like_type(name: &str) -> bool {
    cgg_core::looks_like_type_name(name)
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
    fn python_override_walks_past_a_bodiless_intermediate_class() {
        // `class Middle(Base): pass` has no method record. Its bases
        // come from the class record, or the chain ends at it.
        let mut g = Graph::new();
        g.add_callable(py_node(0, "steps.BaseStep.execute", 10));
        g.add_callable(py_node(1, "steps.ScoreStep.execute", 50));
        let mut f = FileFacts::new(
            FileId::new(0),
            std::path::PathBuf::from("steps.py"),
            "python",
        );
        f.definitions
            .push(py_def("execute", "steps.BaseStep.execute", 10, &[]));
        f.definitions.push(py_def(
            "execute",
            "steps.ScoreStep.execute",
            50,
            &["Middle"],
        ));
        for (qn, bases) in [
            ("steps.BaseStep", vec![]),
            ("steps.Middle", vec!["BaseStep".to_string()]),
            ("steps.ScoreStep", vec!["Middle".to_string()]),
        ] {
            f.classes.push(cgg_core::ClassDecl {
                class_qn: qn.into(),
                base_types: bases,
                line: 1,
            });
        }
        let edges = inheritance_fanout(&g, &[f]);
        assert_eq!(edges.len(), 1, "{edges:?}");
        assert_eq!(edges[0].src, CallableId::new(0));
        assert_eq!(edges[0].dst, CallableId::new(1));
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

    fn lang_node(id: u32, lang: &str, qn: &str, start: u32) -> CallableNode {
        let mut n = py_node(id, qn, start);
        n.language = lang.into();
        n.simple_name = qn.rsplit(['.', ':']).next().unwrap_or(qn).into();
        n
    }

    fn lang_facts(lang: &str, defs: Vec<DefRecord>) -> FileFacts {
        let mut f = FileFacts::new(FileId::new(0), std::path::PathBuf::from("x"), lang);
        f.definitions = defs;
        f
    }

    #[test]
    fn java_interface_and_abstract_members_fan_out_to_implementations() {
        let mut g = Graph::new();
        g.add_callable(lang_node(0, "java", "p.Shape.draw", 10));
        g.add_callable(lang_node(1, "java", "p.Square.draw", 40));
        g.add_callable(lang_node(2, "java", "p.Other.draw", 70));
        let f = lang_facts(
            "java",
            vec![
                py_def("draw", "p.Shape.draw", 10, &[]),
                py_def("draw", "p.Square.draw", 40, &["Shape"]),
                // Near miss: same name, no relation.
                py_def("draw", "p.Other.draw", 70, &[]),
            ],
        );
        let edges = inheritance_fanout(&g, &[f]);
        assert_eq!(edges.len(), 1, "{edges:?}");
        assert_eq!(
            (edges[0].src, edges[0].dst),
            (CallableId::new(0), CallableId::new(1))
        );
    }

    #[test]
    fn java_chain_runs_through_a_class_with_no_methods() {
        let mut g = Graph::new();
        g.add_callable(lang_node(0, "java", "p.Base.get", 10));
        g.add_callable(lang_node(1, "java", "p.Leaf.get", 40));
        let mut f = lang_facts(
            "java",
            vec![
                py_def("get", "p.Base.get", 10, &[]),
                py_def("get", "p.Leaf.get", 40, &["Middle"]),
            ],
        );
        f.classes.push(cgg_core::ClassDecl {
            class_qn: "p.Middle".into(),
            base_types: vec!["Base".into()],
            line: 1,
        });
        let edges = inheritance_fanout(&g, &[f]);
        assert_eq!(edges.len(), 1, "{edges:?}");
    }

    #[test]
    fn static_members_and_csharp_accessors_do_not_override() {
        let mut g = Graph::new();
        g.add_callable(lang_node(0, "java", "p.A.make", 10));
        g.add_callable(lang_node(1, "java", "p.B.make", 40));
        g.add_callable(lang_node(2, "csharp", "N.A.get", 70));
        g.add_callable(lang_node(3, "csharp", "N.B.get", 90));
        let mut st = py_def("make", "p.B.make", 40, &["A"]);
        st.variant = DefVariant::StaticMethod;
        let java = lang_facts("java", vec![py_def("make", "p.A.make", 10, &[]), st]);
        let mut acc = py_def("get", "N.B.get", 90, &["A"]);
        acc.variant = DefVariant::Property;
        let cs = lang_facts("csharp", vec![py_def("get", "N.A.get", 70, &[]), acc]);
        assert!(inheritance_fanout(&g, &[java, cs]).is_empty());
    }

    #[test]
    fn scala_and_dart_members_recorded_as_free_functions_still_override() {
        let mut g = Graph::new();
        g.add_callable(lang_node(0, "scala", "Shape::area", 10));
        g.add_callable(lang_node(1, "scala", "Square::area", 40));
        let mut decl = py_def("area", "Shape::area", 10, &[]);
        decl.variant = DefVariant::TraitMethod;
        let mut imp = py_def("area", "Square::area", 40, &["Shape"]);
        imp.variant = DefVariant::FreeFunction;
        let edges = inheritance_fanout(&g, &[lang_facts("scala", vec![decl, imp])]);
        assert_eq!(edges.len(), 1, "{edges:?}");
    }

    #[test]
    fn cpp_and_rust_are_left_to_their_own_passes() {
        let mut g = Graph::new();
        g.add_callable(lang_node(0, "cpp", "A::f", 10));
        g.add_callable(lang_node(1, "cpp", "B::f", 40));
        let f = lang_facts(
            "cpp",
            vec![
                py_def("f", "A::f", 10, &[]),
                py_def("f", "B::f", 40, &["A"]),
            ],
        );
        assert!(inheritance_fanout(&g, &[f]).is_empty());
    }

    #[test]
    fn hierarchy_reports_declared_and_outside_ancestors() {
        let mut f = lang_facts(
            "java",
            vec![py_def("handle", "p.Ext.handle", 10, &["Handler<String>"])],
        );
        f.classes.push(cgg_core::ClassDecl {
            class_qn: "p.Ext".into(),
            base_types: vec!["Handler<String>".into()],
            line: 1,
        });
        f.classes.push(cgg_core::ClassDecl {
            class_qn: "p.Loop".into(),
            base_types: vec!["Loop".into()],
            line: 1,
        });
        let h = ClassHierarchy::build(&[f]);
        assert_eq!(*h.ancestors("java", "Ext"), vec!["Handler".to_string()]);
        assert!(h.declares("java", "Ext"));
        assert!(!h.declares("java", "Handler"));
        // A class naming itself terminates and is not its own ancestor.
        assert!(h.ancestors("java", "Loop").is_empty());
    }
}
