//! Receiver-carrying value references (WS6, FINDINGS B3).
//!
//! A Java method reference (`this::m`, `Type::new`) or a C# method group
//! (`Select(F)`, `evt += this.H`) names a method without calling it. The
//! plugins record such a site as the call through that receiver it stands
//! for, marked [`RefRecord::value_ref`], so it is resolved by exactly the
//! machinery a call is — typed receivers, `super`, the calling class and
//! its bases, imports — rather than by bare name across the workspace.
//!
//! Two things differ from a call, and both live here:
//!
//! * **Before resolution** ([`prune`]): a value position holds a variable,
//!   field or property far more often than a method. A record whose name
//!   no callable of its language carries, or whose bare name is a local
//!   binding at the site, is data, not a reference to anything — it is
//!   dropped, as an unmatched [`cgg_core::VALUE_REF_HINT`] record always
//!   has been, instead of reaching the unresolved buckets as a call that
//!   does not exist.
//! * **After resolution** ([`tag_reference_edges`]): every edge drawn from
//!   such a site is a [`Via::Reference`] edge, so it is gated behind
//!   `--reference-edges` like every other reference.

use std::collections::{HashMap, HashSet};

use cgg_core::graph::{Graph, Via};
use cgg_core::ids::FileId;
use cgg_core::{FileFacts, RefRecord};

/// The callable name a value-ref record must match: its last path
/// segment (`a.b.Foo` for a constructor reference is `Foo`).
fn leaf(r: &RefRecord) -> &str {
    r.name.rsplit(['.', ':']).next().unwrap_or(&r.name)
}

/// Drop value-ref records that cannot name a callable: no callable of the
/// file's language has that simple name, or the bare name is a local
/// binding in scope at the site. Facts without a value-ref record are not
/// touched, so this is free on a run that records none.
pub fn prune(all_facts: &mut [FileFacts]) {
    if !all_facts
        .iter()
        .any(|f| f.references.iter().any(|r| r.value_ref))
    {
        return;
    }
    let mut names: HashMap<&str, HashSet<&str>> = HashMap::new();
    for f in all_facts.iter() {
        let set = names.entry(f.language.as_str()).or_default();
        set.extend(f.definitions.iter().map(|d| d.simple_name.as_str()));
    }
    // Owned so the facts can be borrowed mutably below.
    let names: HashMap<String, HashSet<String>> = names
        .into_iter()
        .map(|(l, s)| (l.to_string(), s.into_iter().map(str::to_string).collect()))
        .collect();
    let empty = HashSet::new();
    for f in all_facts.iter_mut() {
        if !f.references.iter().any(|r| r.value_ref) {
            continue;
        }
        let known = names.get(&f.language).unwrap_or(&empty);
        let locals = crate::scope::FileLocals::build(f);
        f.references.retain(|r| {
            !r.value_ref || (known.contains(leaf(r)) && !locals.shadows_bare_call(r))
        });
    }
}

/// Re-tag every direct edge drawn from a value-ref site as a reference
/// edge. Sites are keyed by (file, byte); the plugins place a value-ref
/// site on the referenced name's own token, where no call starts, so a
/// call in the same expression is never re-tagged.
///
/// The edges are whatever the call through that receiver would have
/// drawn, at the same confidence: a `medium` edge here is the base-chain
/// or calling-class binding a `medium` call would be. Which value
/// positions are recorded at all is the plugins' decision — a C# member
/// read through a variable, or a Python attribute of one, is not, being
/// a property far more often than a method group.
pub fn tag_reference_edges(graph: &mut Graph, all_facts: &[FileFacts]) {
    let sites: HashSet<(FileId, u32)> = all_facts
        .iter()
        .flat_map(|f| {
            f.references
                .iter()
                .filter(|r| r.value_ref)
                .map(move |r| (f.file, r.site_byte))
        })
        .collect();
    if sites.is_empty() {
        return;
    }
    let file_of: HashMap<_, FileId> =
        graph.callables.values().map(|c| (c.id, c.file)).collect();
    for e in graph.edges.iter_mut() {
        if e.via != Via::Direct {
            continue;
        }
        if file_of
            .get(&e.src)
            .is_some_and(|file| sites.contains(&(*file, e.site_byte)))
        {
            e.via = Via::Reference;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::DefRecord;

    fn facts(lang: &str, file: u32, defs: &[&str], refs: Vec<RefRecord>) -> FileFacts {
        let mut f = FileFacts::new(FileId::new(file), format!("f{file}").into(), lang);
        f.definitions = defs
            .iter()
            .map(|n| DefRecord {
                simple_name: (*n).to_string(),
                qualified_name: (*n).to_string(),
                ..Default::default()
            })
            .collect();
        f.references = refs;
        f
    }

    fn vref(name: &str, recv: &str) -> RefRecord {
        RefRecord {
            name: name.into(),
            receiver_hint: recv.into(),
            value_ref: true,
            ..Default::default()
        }
    }

    #[test]
    fn a_value_ref_naming_no_callable_is_dropped() {
        let mut all = vec![
            facts(
                "csharp",
                1,
                &["Run"],
                vec![vref("Count", "xs"), vref("Handle", "this")],
            ),
            facts("csharp", 2, &["Handle"], vec![]),
        ];
        prune(&mut all);
        let names: Vec<&str> =
            all[0].references.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["Handle"]);
    }

    #[test]
    fn a_callable_of_another_language_does_not_keep_it() {
        let mut all = vec![
            facts("java", 1, &["run"], vec![vref("onItem", "this")]),
            facts("python", 2, &["onItem"], vec![]),
        ];
        prune(&mut all);
        assert!(all[0].references.is_empty());
    }

    #[test]
    fn a_constructor_reference_matches_by_its_last_segment() {
        let mut all = vec![facts("java", 1, &["Foo"], vec![vref("a.b.Foo", "")])];
        prune(&mut all);
        assert_eq!(all[0].references.len(), 1);
    }

    #[test]
    fn ordinary_calls_are_never_pruned() {
        let call = RefRecord {
            name: "nothing".into(),
            ..Default::default()
        };
        let mut all = vec![facts("java", 1, &["run"], vec![call, vref("x", "")])];
        prune(&mut all);
        assert_eq!(all[0].references.len(), 1);
        assert!(!all[0].references[0].value_ref);
    }

    #[test]
    fn edges_from_value_ref_sites_become_reference_edges() {
        use crate::deadcode::testutil::{edge, graph_with, node};
        let mut g = graph_with(vec![
            node(0, "A.run", "run", "java"),
            node(1, "A.m", "m", "java"),
        ]);
        let mut at_ref = edge(0, 1);
        at_ref.site_byte = 40;
        let mut a_call = edge(0, 1);
        a_call.site_byte = 41;
        g.edges.push(at_ref);
        g.edges.push(a_call);
        let mut r = vref("m", "this");
        r.site_byte = 40;
        let f = facts("java", 0, &["run", "m"], vec![r]);
        tag_reference_edges(&mut g, &[f]);
        let vias: Vec<Via> = g.edges.iter().map(|e| e.via.clone()).collect();
        assert_eq!(vias, vec![Via::Reference, Via::Direct]);
    }
}
