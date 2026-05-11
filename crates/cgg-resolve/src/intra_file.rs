//! Intra-file linker.
//!
//! Ported from codescope's `get_containing_def_for_ref`, but byte-based
//! and strongly typed:
//!
//! * For every call site reference in a file, locate the **smallest**
//!   enclosing definition (the callable whose byte range tightly
//!   contains the ref's site byte).
//! * Match the ref's simple name against definitions in the same file.
//!   * **Exactly one match** → `CallEdge` with
//!     `confidence=high` / `resolver="intra-file"`.
//!   * **Zero matches** → `AuditUnresolvedCall` with
//!     `reason="no-candidate-in-scope"`.
//!   * **Two or more matches** → `AuditUnresolvedCall` with
//!     `reason="ambiguous-in-file"` (Task 6's scope-aware resolvers
//!     collapse these).
//!
//! Cycles — including self-calls (`fn f() { f(); }`) — are emitted
//! normally; no deduplication or cycle-breaking is performed.

use cgg_core::{
    audit::AuditUnresolvedCall,
    graph::{CallEdge, Confidence, Via},
    ids::{CallableId, FileId, ResolverId},
    DefRecord, FileFacts, RefRecord,
};

/// Map from a file's definition index (matching `FileFacts.definitions`
/// order) to the final `CallableId` assigned when inserting into the
/// graph. Keyed outside this module by the driver.
pub type DefIdMap = std::collections::HashMap<(FileId, u32), CallableId>;

/// Outcome of linking a single file.
#[derive(Debug, Default)]
pub struct LinkOutcome {
    pub edges: Vec<CallEdge>,
    pub unresolved: Vec<AuditUnresolvedCall>,
}

/// Run the intra-file linker over a single file.
///
/// `def_ids` must contain entries for every `(facts.file, idx)` pair
/// where `idx` is a valid index into `facts.definitions`.
pub fn link_file(facts: &FileFacts, def_ids: &DefIdMap) -> LinkOutcome {
    let mut out = LinkOutcome::default();
    let resolver_id = ResolverId::new("intra-file");

    for rref in &facts.references {
        let enclosing = enclosing_def_index(facts, rref);
        let src = enclosing
            .and_then(|i| def_ids.get(&(facts.file, i as u32)).copied());

        // Candidate defs by simple-name match in this file.
        let mut candidates: Vec<(u32, &DefRecord)> = facts
            .definitions
            .iter()
            .enumerate()
            .filter(|(_, d)| d.simple_name == rref.name)
            .map(|(i, d)| (i as u32, d))
            .collect();

        // Receiver-based narrowing. A call of the form `Foo::bar()`
        // or `obj.bar()` carries a receiver_hint (Rust: `Foo`, `self`;
        // Python: `obj`, `self`). We narrow the candidate set:
        //
        //   * empty receiver            -> no narrowing.
        //   * `self` / `Self` / `cls`   -> no narrowing (enclosing
        //                                  scope handles it).
        //   * anything else             -> candidate qn must contain
        //                                  the receiver hint as a path
        //                                  segment (`::` for Rust,
        //                                  `.` for python). This
        //                                  filters out `Vec::new` from
        //                                  matching a local `FileFacts::new`.
        let rh = rref.receiver_hint.as_str();
        if !rh.is_empty() && rh != "self" && rh != "Self" && rh != "cls" {
            candidates.retain(|(_, d)| {
                d.qualified_name
                    .split("::")
                    .any(|seg| seg == rh)
                    || d.qualified_name
                        .split('.')
                        .any(|seg| seg == rh)
            });
        }

        match candidates.as_slice() {
            [] => {
                out.unresolved.push(AuditUnresolvedCall {
                    src,
                    file: facts.file,
                    site_line: rref.site_line,
                    site_byte: rref.site_byte,
                    name: rref.name.clone(),
                    reason: "no-candidate-in-scope".into(),
                });
            }
            [(cand_idx, _)] => {
                let Some(src_id) = src else {
                    // No enclosing callable (e.g. ref at module top
                    // level) — the edge has no source; record it as
                    // unresolved with a specific reason so Task 6's
                    // resolver can pick it up.
                    out.unresolved.push(AuditUnresolvedCall {
                        src: None,
                        file: facts.file,
                        site_line: rref.site_line,
                        site_byte: rref.site_byte,
                        name: rref.name.clone(),
                        reason: "no-enclosing-callable".into(),
                    });
                    continue;
                };
                let dst_id = def_ids[&(facts.file, *cand_idx)];
                out.edges.push(CallEdge {
                    src: src_id,
                    dst: dst_id,
                    site_line: rref.site_line,
                    site_byte: rref.site_byte,
                    confidence: Confidence::High,
                    via: Via::Direct,
                    resolver: resolver_id.clone(),
                });
            }
            _ => {
                out.unresolved.push(AuditUnresolvedCall {
                    src,
                    file: facts.file,
                    site_line: rref.site_line,
                    site_byte: rref.site_byte,
                    name: rref.name.clone(),
                    reason: "ambiguous-in-file".into(),
                });
            }
        }
    }

    out
}

/// Return the index of the smallest definition whose byte range
/// contains `rref.site_byte`. Ties are broken by the smaller byte
/// span — matching codescope's "smallest-enclosing" rule.
fn enclosing_def_index(facts: &FileFacts, rref: &RefRecord) -> Option<usize> {
    let b = rref.site_byte;
    let mut best: Option<(usize, u32)> = None;
    for (i, d) in facts.definitions.iter().enumerate() {
        if d.start_byte <= b && b < d.end_byte {
            let span = d.end_byte - d.start_byte;
            match best {
                None => best = Some((i, span)),
                Some((_, bspan)) if span < bspan => best = Some((i, span)),
                _ => {}
            }
        }
    }
    best.map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::{DefRecord, DefVariant, FileFacts, RefRecord};
    use std::path::PathBuf;

    fn mk_def(
        simple: &str,
        qn: &str,
        variant: DefVariant,
        byte_range: (u32, u32),
    ) -> DefRecord {
        DefRecord {
            simple_name: simple.into(),
            qualified_name: qn.into(),
            variant,
            start_line: 1,
            end_line: 1,
            start_byte: byte_range.0,
            end_byte: byte_range.1,
            signature_hint: String::new(),
            visibility: String::new(),
            attributes: Vec::new(),
        }
    }

    fn mk_ref(name: &str, site_byte: u32) -> RefRecord {
        RefRecord {
            name: name.into(),
            receiver_hint: String::new(),
            site_line: 1,
            site_byte,
        }
    }

    fn facts_with(defs: Vec<DefRecord>, refs: Vec<RefRecord>) -> FileFacts {
        FileFacts {
            file: FileId::new(0),
            path: PathBuf::from("t.rs"),
            language: "rust".into(),
            definitions: defs,
            references: refs,
            imports: Vec::new(),
            local_types: Vec::new(),
        }
    }

    fn mk_map(facts: &FileFacts) -> DefIdMap {
        facts
            .definitions
            .iter()
            .enumerate()
            .map(|(i, _)| ((facts.file, i as u32), CallableId::new(i as u32)))
            .collect()
    }

    #[test]
    fn single_match_emits_edge() {
        // def foo at bytes 0..100, def bar at bytes 100..200
        // ref at byte 50 (inside foo) calls "bar"
        let defs = vec![
            mk_def("foo", "m::foo", DefVariant::FreeFunction, (0, 100)),
            mk_def("bar", "m::bar", DefVariant::FreeFunction, (100, 200)),
        ];
        let refs = vec![mk_ref("bar", 50)];
        let facts = facts_with(defs, refs);
        let map = mk_map(&facts);
        let out = link_file(&facts, &map);
        assert_eq!(out.edges.len(), 1);
        assert_eq!(out.unresolved.len(), 0);
        let e = &out.edges[0];
        assert_eq!(e.src, CallableId::new(0));
        assert_eq!(e.dst, CallableId::new(1));
        assert_eq!(e.confidence, Confidence::High);
        assert_eq!(e.resolver.as_str(), "intra-file");
    }

    #[test]
    fn zero_candidates_unresolved_no_candidate() {
        let defs = vec![mk_def("foo", "m::foo", DefVariant::FreeFunction, (0, 100))];
        let refs = vec![mk_ref("baz", 50)];
        let facts = facts_with(defs, refs);
        let map = mk_map(&facts);
        let out = link_file(&facts, &map);
        assert_eq!(out.edges.len(), 0);
        assert_eq!(out.unresolved.len(), 1);
        assert_eq!(out.unresolved[0].reason, "no-candidate-in-scope");
        assert_eq!(out.unresolved[0].name, "baz");
    }

    #[test]
    fn ambiguous_name_flags_unresolved() {
        // Two defs named `m` — common in Rust where `impl A { fn m } impl B { fn m }`.
        let defs = vec![
            mk_def("caller", "m::caller", DefVariant::FreeFunction, (0, 50)),
            mk_def("m", "m::A::m", DefVariant::InherentMethod, (50, 80)),
            mk_def("m", "m::B::m", DefVariant::InherentMethod, (80, 110)),
        ];
        let refs = vec![mk_ref("m", 10)];
        let facts = facts_with(defs, refs);
        let map = mk_map(&facts);
        let out = link_file(&facts, &map);
        assert_eq!(out.edges.len(), 0);
        assert_eq!(out.unresolved.len(), 1);
        assert_eq!(out.unresolved[0].reason, "ambiguous-in-file");
    }

    #[test]
    fn smallest_enclosing_wins() {
        // Outer fn at 0..200, inner named closure at 10..50, ref at byte 20.
        // The ref should be attributed to the inner closure (smallest
        // enclosing), not the outer function.
        let defs = vec![
            mk_def("outer", "m::outer", DefVariant::FreeFunction, (0, 200)),
            mk_def("inner", "m::outer::inner", DefVariant::NamedClosure, (10, 50)),
            mk_def("target", "m::target", DefVariant::FreeFunction, (200, 300)),
        ];
        let refs = vec![mk_ref("target", 20)];
        let facts = facts_with(defs, refs);
        let map = mk_map(&facts);
        let out = link_file(&facts, &map);
        assert_eq!(out.edges.len(), 1);
        // src must be the closure (id 1), not the outer function (id 0).
        assert_eq!(out.edges[0].src, CallableId::new(1));
        assert_eq!(out.edges[0].dst, CallableId::new(2));
    }

    #[test]
    fn self_call_is_preserved_as_edge() {
        // Recursion: `fn f() { f(); }`
        let defs = vec![mk_def("f", "m::f", DefVariant::FreeFunction, (0, 100))];
        let refs = vec![mk_ref("f", 20)];
        let facts = facts_with(defs, refs);
        let map = mk_map(&facts);
        let out = link_file(&facts, &map);
        assert_eq!(out.edges.len(), 1);
        assert_eq!(out.edges[0].src, out.edges[0].dst);
    }

    #[test]
    fn ref_outside_any_def_is_unresolved() {
        // Top-level statement (not inside any callable).
        let defs = vec![mk_def("foo", "m::foo", DefVariant::FreeFunction, (100, 200))];
        let refs = vec![mk_ref("foo", 10)];
        let facts = facts_with(defs, refs);
        let map = mk_map(&facts);
        let out = link_file(&facts, &map);
        assert_eq!(out.edges.len(), 0);
        assert_eq!(out.unresolved.len(), 1);
        assert_eq!(out.unresolved[0].reason, "no-enclosing-callable");
    }
}
