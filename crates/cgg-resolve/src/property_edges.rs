//! Property edges (`--property-edges`): attribute reads that run code.
//!
//! Reading a Python `@property` executes the getter's body, so
//! `pbr_voxel.feats` reaches code exactly as `obj.method()` does — the
//! source just spells it without parentheses. This pass mints those
//! edges, mirroring what callgraph does for the same source and what
//! `--reference-edges` did for value references: opt-in, precise or
//! silent. A read is bound only when three things hold at once:
//!
//! 1. The **receiver's type is declared**, not guessed — a typed
//!    parameter of the enclosing def (`pipeline: Pipeline`, recorded by
//!    extraction into `FileFacts::param_types`), a `self`/`cls` read
//!    inside its own class, a receiver the type propagator already
//!    rewrote from a class-field annotation, or a variable assigned
//!    from a call whose return type every same-named definition agrees
//!    on (`pbr_voxel = pipeline.decode_tex_slat(…)`, joined through the
//!    call's minted edge and `FileFacts::function_returns`).
//! 2. The type — or, walking `base_types`, a base of it — declares
//!    **exactly one** property under the attribute's name. Zero
//!    candidates or two in the first defining class: the read is
//!    dropped silently. A value read that is not a property is data,
//!    not a miss, and an ambiguous name is a guess this pass does not
//!    make.
//! 3. The edge is not a self-edge.
//!
//! Nothing here feeds back into call resolution: `param_types` and
//! `function_returns` are tables of their own, invisible to the type
//! propagator, so a run without the flag records none of this and its
//! graph is byte-identical to one from before the pass existed.

use std::collections::{HashMap, HashSet};

use cgg_core::{
    CallableKind, Confidence, FileFacts, Graph, LocalType, Via,
    ids::{CallableId, FileId},
};

use crate::cross_file::enclosing_callable_id;

/// Resolution confidence floor: `High` — every accepted receiver type
/// here comes from a declaration (an annotation, the enclosing class,
/// or an agree-typed return), never from the loose file-wide heuristics
/// the propagator marks `weak_receiver_type`. A rewritten receiver that
/// *is* weak still binds, at `Medium`, so the graph keeps saying what
/// it is sure of.
fn confidence_for(weak: bool) -> Confidence {
    if weak {
        Confidence::Medium
    } else {
        Confidence::High
    }
}

/// The class segment of a qualified name (`pkg.module.SparseTensor.feats`
/// → `SparseTensor`), i.e. the second-to-last segment. `None` for names
/// too short to own the attribute — a property lives on a class.
fn class_of(qualified_name: &str) -> Option<&str> {
    let mut segs = qualified_name.split('.');
    let _attr = segs.next_back()?;
    segs.next_back()
}

/// The last segment of a (possibly dotted or generic) base type as
/// written: `module.ImageTo3DPipeline` → `ImageTo3DPipeline`,
/// `Generic[T]` → `Generic`.
fn base_head(base: &str) -> Option<&str> {
    let name = base.split('.').next_back()?;
    let name = name.split('[').next()?;
    let name = name.trim();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Mint `Via::Property` edges for every property read the declarative
/// tables can resolve. Free when the run recorded no reads (`--property-edges`
/// off): the scan is one pass over `all_facts` and the early return is
/// the first thing it does.
pub fn emit(graph: &mut Graph, all_facts: &[FileFacts]) {
    let reads_any = all_facts
        .iter()
        .any(|f| f.references.iter().any(|r| r.property_read));
    if !reads_any {
        return;
    }

    // Property callables by (owning class, attribute). A name defined by
    // two classes keeps both candidates: ambiguity is decided per
    // *defining* class during the walk, not by dropping the index.
    let mut prop_by_class_attr: HashMap<(&str, &str), Vec<CallableId>> = HashMap::new();
    for c in graph.callables.values() {
        if c.kind != CallableKind::Property {
            continue;
        }
        let Some(class) = class_of(&c.qualified_name) else {
            continue;
        };
        prop_by_class_attr
            .entry((class, c.simple_name.as_str()))
            .or_default()
            .push(c.id);
    }

    // Class → declared bases, by simple name, merged across files. Two
    // same-named classes pool their bases; that only widens a walk that
    // candidate uniqueness still gates.
    let mut bases_by_class: HashMap<&str, Vec<&str>> = HashMap::new();
    for f in all_facts {
        for cls in &f.classes {
            let Some(name) = cls.class_qn.rsplit('.').next() else {
                continue;
            };
            let entry = bases_by_class.entry(name).or_default();
            for b in &cls.base_types {
                if let Some(head) = base_head(b)
                    && !entry.contains(&head)
                {
                    entry.push(head);
                }
            }
        }
    }

    // Declared return types by the def's qualified name — the exact
    // table extraction fills for this run (`FileFacts::function_returns`).
    // Two defs of one qualified name (duplicate trees) that disagree
    // leave the name untyped, same precision rule as everything here.
    let mut return_by_qn: HashMap<&str, Option<&str>> = HashMap::new();
    for f in all_facts {
        for fr in &f.function_returns {
            let slot = return_by_qn.entry(fr.def_qn.as_str()).or_insert(None);
            match slot {
                None => *slot = Some(fr.type_name.as_str()),
                Some(prev) if *prev == fr.type_name => {}
                Some(_) => *slot = Some(""), // disagreement: never typed
            }
        }
    }

    // `(file, start, end) -> callable`, for the enclosing-def lookup.
    let mut callables_by_span: HashMap<(FileId, u32, u32), CallableId> = HashMap::new();
    for c in graph.callables.values() {
        callables_by_span
            .entry((c.file, c.start_byte, c.end_byte))
            .or_insert(c.id);
    }

    // `(callee's file, call site line) -> callees`, for the assignment
    // join. More than one callee at a line leaves the variable untyped —
    // two calls on one line do not name one type.
    let mut edge_targets_by_line: HashMap<(FileId, u32), Vec<CallableId>> = HashMap::new();
    for e in &graph.edges {
        let Some(src) = graph.callables.get(&e.src) else {
            continue;
        };
        edge_targets_by_line
            .entry((src.file, e.site_line))
            .or_default()
            .push(e.dst);
    }

    // Callable id -> qualified name, for the assignment join.
    let mut qn_by_cid: HashMap<CallableId, &str> = HashMap::new();
    for c in graph.callables.values() {
        qn_by_cid.insert(c.id, c.qualified_name.as_str());
    }

    let resolver = cgg_core::ids::ResolverId::new("property");
    let mut minted: HashSet<(CallableId, CallableId, u32)> = HashSet::new();

    for facts in all_facts {
        if !facts.references.iter().any(|r| r.property_read) {
            continue;
        }
        let file = facts.file;
        // Typed parameters of this file, per defining def's start byte.
        let mut params_by_scope: HashMap<u32, Vec<&LocalType>> = HashMap::new();
        for lt in &facts.param_types {
            params_by_scope.entry(lt.scope_byte).or_default().push(lt);
        }
        // Call assignments of this file, per variable, in source order.
        let mut assigns_by_var: HashMap<&str, Vec<&cgg_core::CallAssign>> = HashMap::new();
        for ca in &facts.call_assignments {
            assigns_by_var
                .entry(ca.var_name.as_str())
                .or_default()
                .push(ca);
        }

        for r in &facts.references {
            if !r.property_read {
                continue;
            }
            let Some(src) = enclosing_callable_id(&callables_by_span, facts, r.site_byte)
            else {
                continue;
            };
            let enclosing_def = facts
                .definitions
                .iter()
                .filter(|d| d.start_byte <= r.site_byte && r.site_byte < d.end_byte)
                .min_by_key(|d| d.end_byte - d.start_byte);

            // --- Receiver type. ---
            let rh = r.receiver_hint.as_str();
            if rh.is_empty() {
                continue;
            }
            let typed = receiver_type(
                rh,
                r.site_line,
                file,
                enclosing_def,
                &params_by_scope,
                &assigns_by_var,
                &edge_targets_by_line,
                &return_by_qn,
                &qn_by_cid,
            );
            let Some((type_name, weak)) = typed else {
                continue;
            };

            // --- Walk the class chain for the attribute. ---
            let Some(dst) =
                resolve_property(&type_name, r.name.as_str(), &prop_by_class_attr, &bases_by_class)
            else {
                continue;
            };
            if dst == src || !minted.insert((src, dst, r.site_byte)) {
                continue;
            }
            graph.edges.push(cgg_core::CallEdge {
                src,
                dst,
                site_line: r.site_line,
                site_byte: r.site_byte,
                confidence: confidence_for(weak),
                via: Via::Property,
                resolver: resolver.clone(),
                weight: 1,
            });
        }
    }
}

/// The declared type of a read's receiver, or `None`. The bool is the
/// weak-type flag for confidence: every source here is a declaration
/// except a receiver the propagator rewrote from a guess.
#[allow(clippy::too_many_arguments)]
fn receiver_type<'a>(
    rh: &'a str,
    site_line: u32,
    file: FileId,
    enclosing_def: Option<&cgg_core::DefRecord>,
    params_by_scope: &HashMap<u32, Vec<&'a LocalType>>,
    assigns_by_var: &HashMap<&str, Vec<&'a cgg_core::CallAssign>>,
    edge_targets_by_line: &HashMap<(FileId, u32), Vec<CallableId>>,
    return_by_qn: &HashMap<&str, Option<&str>>,
    qn_by_cid: &HashMap<CallableId, &str>,
) -> Option<(String, bool)> {
    // Already a type: the propagator rewrote a `self.field` path (or a
    // constructor assignment) to the class it declared. Its weak flag
    // says how anchored the rewrite was.
    if rh.starts_with(char::is_uppercase) {
        return Some((rh.to_string(), false));
    }
    let head = rh.split('.').next().unwrap_or(rh);
    // `self`/`cls` inside a def name the class the def belongs to — the
    // qualified name's second-to-last segment.
    if head == "self" || head == "cls" {
        let class = enclosing_def.and_then(|d| class_of(&d.qualified_name));
        return class.map(|c| (c.to_string(), false));
    }
    // A parameter of the enclosing def with a declared type.
    if let Some(def) = enclosing_def
        && let Some(lts) = params_by_scope.get(&def.start_byte)
        && let Some(lt) = lts.iter().find(|lt| lt.var_name == head)
    {
        return Some((lt.type_name.clone(), false));
    }
    // A variable some call's declared return produced: the latest
    // assignment to the name inside the enclosing def, joined to the
    // edge minted for its call. Two distinct callees at one line name
    // no type.
    if let Some(def) = enclosing_def
        && let Some(assigns) = assigns_by_var.get(head)
    {
        let best = assigns
            .iter()
            .filter(|ca| {
                ca.site_line >= def.start_line
                    && ca.site_line <= def.end_line
                    && ca.site_line <= site_line
            })
            .max_by_key(|ca| ca.site_line);
        if let Some(ca) = best
            && let Some(targets) = edge_targets_by_line.get(&(file, ca.site_line))
        {
            let mut unique: Vec<CallableId> = targets.to_vec();
            unique.sort_unstable();
            unique.dedup();
            if let [dst] = unique.as_slice()
                && let Some(callee_qn) = qn_by_cid.get(dst)
                && let Some(Some(ret)) = return_by_qn.get(*callee_qn)
                && !ret.is_empty()
            {
                return Some((ret.to_string(), false));
            }
        }
    }
    None
}

/// Walk `type_name` through its declared bases, own class first, and
/// return the property that exactly one candidate defines. Ambiguity
/// inside the first defining class drops the read; a base class
/// defining the attribute loses to the subclass that overrides it.
fn resolve_property(
    type_name: &str,
    attr: &str,
    prop_by_class_attr: &HashMap<(&str, &str), Vec<CallableId>>,
    bases_by_class: &HashMap<&str, Vec<&str>>,
) -> Option<CallableId> {
    let mut queue: Vec<&str> = vec![type_name.rsplit('.').next()?];
    let mut seen: HashSet<&str> = HashSet::new();
    while let Some(class) = queue.pop() {
        if !seen.insert(class) {
            continue;
        }
        match prop_by_class_attr.get(&(class, attr)).map(Vec::as_slice) {
            Some([dst]) => return Some(*dst),
            Some(_) => return None, // two same-named properties on one class
            None => {}
        }
        if let Some(bases) = bases_by_class.get(class) {
            for b in bases {
                queue.push(b);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::{CallAssign, CallableNode, ClassDecl, DefRecord, FunctionReturn, RefRecord};

    fn callable(id: u32, qn: &str, kind: CallableKind, file: u32, start: u32, end: u32) -> CallableNode {
        CallableNode {
            id: CallableId::new(id),
            qualified_name: qn.into(),
            simple_name: qn.rsplit('.').next().unwrap().into(),
            kind,
            language: "python".into(),
            file: FileId::new(file),
            start_byte: start,
            end_byte: end,
            ..Default::default()
        }
    }

    fn def(qn: &str, kind: cgg_core::DefVariant, start: u32, end: u32) -> DefRecord {
        DefRecord {
            simple_name: qn.rsplit('.').next().unwrap().into(),
            qualified_name: qn.into(),
            variant: kind,
            start_byte: start,
            end_byte: end,
            start_line: 1,
            end_line: 999,
            ..Default::default()
        }
    }

    fn read(name: &str, recv: &str, byte: u32, line: u32) -> RefRecord {
        RefRecord {
            name: name.into(),
            receiver_hint: recv.into(),
            site_byte: byte,
            site_line: line,
            property_read: true,
            ..Default::default()
        }
    }

    /// The fixture every test shapes: `class Pipe(Pipeline)` in m.py with
    /// a method whose body reads typed receivers; vendor classes in v.py
    /// (`SparseTensor(VarLenTensor)` overriding `shape`, a `decode_tex_slat`
    /// declaring `-> SparseTensor`). Call ids: 0 process, 1 Pipe.device
    /// (inherited property), 2 Pipeline.device, 3 decode_tex_slat,
    /// 4 SparseTensor.feats, 5 SparseTensor.shape, 6 VarLenTensor.shape.
    fn fixture() -> (Graph, Vec<FileFacts>) {
        let mut graph = Graph::new();
        graph.add_callable(callable(0, "Pipe.process", CallableKind::Method, 0, 100, 400));
        graph.add_callable(callable(2, "Pipeline.device", CallableKind::Property, 1, 20, 60));
        graph.add_callable(callable(3, "Trellis2.decode_tex_slat", CallableKind::Method, 1, 100, 200));
        graph.add_callable(callable(4, "SparseTensor.feats", CallableKind::Property, 1, 220, 260));
        graph.add_callable(callable(5, "SparseTensor.shape", CallableKind::Property, 1, 280, 320));
        graph.add_callable(callable(6, "VarLenTensor.shape", CallableKind::Property, 1, 340, 380));
        // The call the assignment join reads: `pbr_voxel = pipeline.decode_tex_slat(slat)`
        // on line 5 of m.py, minted by the ordinary call pipeline.
        graph.add_edge(cgg_core::CallEdge {
            src: CallableId::new(0),
            dst: CallableId::new(3),
            site_line: 5,
            site_byte: 250,
            confidence: Confidence::High,
            via: Via::Direct,
            resolver: cgg_core::ids::ResolverId::new("intra-file"),
            weight: 1,
        });

        let mut m = FileFacts::new(FileId::new(0), "m.py".into(), "python");
        m.definitions = vec![def("Pipe.process", cgg_core::DefVariant::InherentMethod, 100, 400)];
        m.classes = vec![ClassDecl {
            class_qn: "Pipe".into(),
            base_types: vec!["Pipeline".into()],
            line: 1,
        }];
        m.param_types = vec![LocalType {
            var_name: "pipeline".into(),
            type_name: "Trellis2".into(),
            scope_byte: 100,
        }];
        m.call_assignments = vec![CallAssign {
            var_name: "pbr_voxel".into(),
            site_line: 5,
        }];

        let mut v = FileFacts::new(FileId::new(1), "v.py".into(), "python");
        v.definitions = vec![
            def("Pipeline.device", cgg_core::DefVariant::Property, 20, 60),
            def("Trellis2.decode_tex_slat", cgg_core::DefVariant::InherentMethod, 100, 200),
            def("SparseTensor.feats", cgg_core::DefVariant::Property, 220, 260),
            def("SparseTensor.shape", cgg_core::DefVariant::Property, 280, 320),
            def("VarLenTensor.shape", cgg_core::DefVariant::Property, 340, 380),
        ];
        v.classes = vec![
            ClassDecl {
                class_qn: "Trellis2".into(),
                base_types: vec!["ImageTo3D".into()],
                line: 1,
            },
            ClassDecl {
                class_qn: "ImageTo3D".into(),
                base_types: vec!["Pipeline".into()],
                line: 2,
            },
            ClassDecl {
                class_qn: "SparseTensor".into(),
                base_types: vec!["VarLenTensor".into()],
                line: 3,
            },
            ClassDecl {
                class_qn: "VarLenTensor".into(),
                base_types: vec![],
                line: 4,
            },
        ];
        v.function_returns = vec![FunctionReturn {
            def_qn: "Trellis2.decode_tex_slat".into(),
            type_name: "SparseTensor".into(),
        }];
        (graph, vec![m, v])
    }

    #[test]
    fn typed_parameter_receiver_binds_through_the_base_chain() {
        let (mut graph, mut facts) = fixture();
        // `device=pipeline.device` inside process (byte 300, line 7):
        // pipeline: Trellis2 → ImageTo3D → Pipeline.device.
        facts[0].references = vec![read("device", "pipeline", 300, 7)];
        emit(&mut graph, &facts);
        let props: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.via == Via::Property)
            .map(|e| (e.src, e.dst, e.confidence))
            .collect();
        assert_eq!(props, vec![(CallableId::new(0), CallableId::new(2), Confidence::High)], "two base hops");
    }

    #[test]
    fn self_read_uses_the_enclosing_class() {
        let (mut graph, mut facts) = fixture();
        facts[0].references = vec![read("device", "self", 300, 7)];
        emit(&mut graph, &facts);
        let props: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.via == Via::Property)
            .map(|e| (e.src, e.dst))
            .collect();
        // Pipe does not define `device`; the base class does.
        assert_eq!(props, vec![(CallableId::new(0), CallableId::new(2))]);
    }

    #[test]
    fn own_class_property_overrides_the_base() {
        let (mut graph, mut facts) = fixture();
        facts[0].param_types.push(LocalType {
            var_name: "slat".into(),
            type_name: "SparseTensor".into(),
            scope_byte: 100,
        });
        facts[0].references = vec![read("shape", "slat", 300, 7)];
        emit(&mut graph, &facts);
        let props: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.via == Via::Property)
            .map(|e| e.dst)
            .collect();
        assert_eq!(props, vec![CallableId::new(5)], "SparseTensor.shape, not VarLenTensor's");
    }

    #[test]
    fn return_typed_assignment_binds_the_read() {
        let (mut graph, mut facts) = fixture();
        facts[0].references = vec![read("feats", "pbr_voxel", 300, 7)];
        emit(&mut graph, &facts);
        let props: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.via == Via::Property)
            .map(|e| e.dst)
            .collect();
        // `pbr_voxel = pipeline.decode_tex_slat(slat)` on line 5 → the
        // callee declares `-> SparseTensor` → SparseTensor.feats.
        assert_eq!(props, vec![CallableId::new(4)]);
    }

    #[test]
    fn an_ambiguous_property_drops_the_read() {
        let (mut graph, mut facts) = fixture();
        // A second SparseTensor in another file defines `feats` too: the
        // first defining class is ambiguous and the pass makes no guess.
        graph.add_callable(callable(
            7,
            "other.SparseTensor.feats",
            CallableKind::Property,
            2,
            220,
            260,
        ));
        facts[0].references = vec![read("feats", "pbr_voxel", 300, 7)];
        emit(&mut graph, &facts);
        assert!(graph.edges.iter().all(|e| e.via != Via::Property));
    }

    #[test]
    fn an_undeclared_receiver_is_dropped_silently() {
        let (mut graph, mut facts) = fixture();
        facts[0].references = vec![
            read("feats", "untyped_local", 300, 7),
            read("device", "", 310, 8),
        ];
        emit(&mut graph, &facts);
        assert!(graph.edges.iter().all(|e| e.via != Via::Property));
    }

    #[test]
    fn a_self_read_in_the_property_itself_mints_no_self_edge() {
        let (mut graph, mut facts) = fixture();
        // Inside `Pipeline.device` (v.py) reading `self.device`: the
        // resolution lands on the reader. Dropped.
        let mut v2 = FileFacts::new(FileId::new(1), "v.py".into(), "python");
        v2.definitions = facts[1].definitions.clone();
        v2.references = vec![read("device", "self", 40, 3)];
        facts[1] = v2;
        emit(&mut graph, &facts);
        assert!(graph.edges.iter().all(|e| e.via != Via::Property));
    }

    #[test]
    fn no_reads_anywhere_is_free() {
        let (mut graph, facts) = fixture();
        let before = graph.edges.len();
        emit(&mut graph, &facts);
        assert_eq!(graph.edges.len(), before);
    }
}
