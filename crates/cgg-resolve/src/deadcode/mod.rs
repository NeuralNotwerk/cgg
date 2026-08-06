//! Dead-code analysis.
//!
//! Reports callables that nothing in the analyzed source appears to
//! call. The output is a report: a set of hypotheses, each carrying the
//! evidence for it and the reasons it might be wrong. cgg does not act
//! on findings and takes no position on what should be done about them
//! — see [`cgg_core::deadcode::DEAD_CODE_DISCLAIMER`].
//!
//! # The model, and why it is not the textbook one
//!
//! The obvious algorithm is reachability from roots: mark `main` and
//! friends live, traverse, report the rest. Measured on cgg's own
//! source that flags **716 of 1178 callables**, because only 132 are
//! reachable from `main` through resolved edges — the graph is simply
//! too incomplete for whole-program reachability to mean anything.
//! Naive in-degree-zero flags 410, which is *better* while being the
//! less sophisticated method.
//!
//! So the engine computes both and reports the difference as
//! *category*, not as a mode:
//!
//! * [`FindingCategory::NeverReferenced`] — in-degree zero. A statement
//!   about the graph, independent of how good root discovery was, and
//!   therefore the only category that can start at `High`.
//! * [`FindingCategory::ReachableOnlyFromDeadCode`],
//!   [`FindingCategory::DeadCycle`],
//!   [`FindingCategory::UnreachableFromRoots`] — whole-program claims,
//!   which are only as good as the root set and are withheld entirely
//!   when root coverage is too thin to support them.
//!
//! # Why the reported set is closed
//!
//! Every node on a root-to-node path is itself reachable, so no
//! reachable callable depends on an unreachable one to connect it to a
//! root. The reported set is therefore closed under the analysis: it
//! contains everything the model can find in one pass, and a second
//! pass over the same graph surfaces nothing further.
//!
//! This matters for how the report should be *read*, not for any
//! workflow. It means the report is a complete statement about this
//! graph rather than the first instalment of one — a name-matching tool
//! cannot say the same, which is why vulture's documentation tells users
//! to run it repeatedly. The honest caveat is that the property holds
//! only over the call graph: cgg does not model imports, constants,
//! types or fixtures, so it makes no claim about those at all.

pub(crate) mod adj;
pub(crate) mod caps;
pub(crate) mod evidence;
pub mod roots;
pub(crate) mod scc;

#[cfg(test)]
pub(crate) mod testutil;

use std::collections::BTreeMap;

use cgg_core::audit::AuditFileRecord;
use cgg_core::deadcode::{
    DEFAULT_MIN_ROOT_COVERAGE_PCT, DeadCodeFinding, DeadCodeReport, DeadCodeSummary,
    DeadRegion, Evidence, FindingCategory, LanguageCapabilityReport, LanguageClass,
    LivenessProof, ProofHop, RegionRole, SuppressedCategory, SuppressionReason,
};
use cgg_core::graph::{CallableKind, CallableNode, Confidence, Graph, Via};
use cgg_core::ids::CallableId;
use cgg_core::{FileFacts, Vis};

use adj::Adj;
use evidence::UnresolvedIndex;

/// Inputs the caller controls.
#[derive(Debug, Default)]
pub struct DeadCodeOptions {
    /// User-declared roots, as `(label, matched callable)` pairs.
    pub user_roots: Vec<(String, CallableId)>,
    /// Root patterns that matched nothing, for stale-suppression
    /// reporting.
    pub stale_patterns: Vec<String>,
    pub include_tests: bool,
    pub reference_edges: bool,
    pub dynamic_dispatch: bool,
    pub confidence_threshold: String,
    pub roots_file: Option<std::path::PathBuf>,
    /// What each language's plugin declares it can extract. Empty means
    /// "unknown", which degrades to observation-only.
    pub language_signals: BTreeMap<String, cgg_core::deadcode::LanguageSignals>,
    /// Callables a framework invokes, as `(rule label, target)` pairs.
    ///
    /// Entries that mint a node need nothing here — the node is a root
    /// and the edge does the work. This carries the bucket-D entries
    /// that deliberately mint none, where the only way to express "the
    /// runtime calls this" is to mark it directly.
    pub framework_roots: Vec<(String, CallableId)>,
}

/// Edges that count as "something uses this".
///
/// Reference and dynamic edges are included deliberately. Both are
/// over-approximations, and over-approximation is the *safe* direction
/// here: an extra edge can only mark something live, so the worst it can
/// do is cause a missed finding. The opposite error — reporting a
/// callable that something really does call — costs a reader's trust in
/// every other finding, so every knob is biased toward false
/// negatives.
///
/// External and stdlib edges are excluded: they point at synthesized
/// leaf nodes, adding out-degree rather than inbound edges to real code.
fn counts_for_liveness(e: &cgg_core::graph::CallEdge) -> bool {
    !matches!(e.via, Via::External | Via::Stdlib)
}

/// Search cost of traversing an edge when proving liveness.
///
/// A proof built from resolved direct calls is worth more than one that
/// leans on an over-approximated dispatch fan-out, so the search prefers
/// the former even when it is longer. This mirrors Go `deadcode`'s
/// preference for static edges over dynamic ones: the point of a proof
/// is to be convincing, not merely to exist.
fn edge_cost(e: &cgg_core::graph::CallEdge) -> u32 {
    match (&e.via, e.confidence) {
        (Via::Direct, Confidence::High) => 0,
        (Via::Dynamic, _) => 2,
        _ => 1,
    }
}

/// How a visibility evidence entry should spell itself in the report.
///
/// The language-native token when the language wrote one, so a reader
/// sees `pub(crate)` or `public` rather than a normalization of it. Rust
/// and Python spell the default by writing *nothing*, though, and a bare
/// `"token": ""` tells a reader nothing at all — so the normalized class
/// stands in. It can never contradict the finding: it is the same `vis`
/// the entry was derived from.
fn vis_token(node: &CallableNode) -> String {
    if !node.visibility.is_empty() {
        return node.visibility.clone();
    }
    match node.vis {
        Vis::Public => "public",
        Vis::Internal => "internal",
        Vis::Protected => "protected",
        Vis::Private => "private",
        Vis::Unknown => "",
    }
    .to_string()
}

/// Whether a callable may ever be reported.
fn is_candidate(node: &cgg_core::graph::CallableNode) -> bool {
    // Synthesized exit nodes are not source. Destructors are invoked by
    // scope exit in every language that has them, so "nothing calls it"
    // is never evidence of anything.
    !node.synthetic && node.kind != CallableKind::Destructor
}

/// Run the analysis. Pure over `graph`; performs no I/O.
pub fn analyze(
    graph: &Graph,
    file_audits: &[AuditFileRecord],
    facts: &[FileFacts],
    opts: &DeadCodeOptions,
) -> DeadCodeReport {
    let adjacency = Adj::build(graph, &counts_for_liveness);
    let n = adjacency.n as usize;
    let measured = caps::measure(graph, &opts.language_signals);
    let unresolved = UnresolvedIndex::build(graph, file_audits);
    let root_set = roots::discover(graph, facts, &opts.user_roots, &opts.framework_roots);

    // Identifier-shaped literals in reflective positions. Never an edge
    // — only a reason to doubt a finding.
    let dyn_named: std::collections::HashSet<(&str, &str)> = facts
        .iter()
        .flat_map(|f| {
            f.dyn_uses
                .iter()
                .map(move |d| (f.language.as_str(), d.name.as_str()))
        })
        .collect();

    let node_at = |i: u32| graph.callables.get_index(i as usize).map(|(_, v)| v);
    let lang_at = |i: u32| node_at(i).map(|nd| nd.language.clone()).unwrap_or_default();

    // --- Liveness: production roots, then production + test ----------------
    let live_prod = adjacency.reachable_from(&root_set.production);
    let mut all_roots = root_set.production.clone();
    all_roots.extend_from_slice(&root_set.test);
    all_roots.sort_unstable();
    let live_any = adjacency.reachable_from(&all_roots);

    // --- Per-language root coverage ---------------------------------------
    // Whole-program categories are only meaningful when a decent share
    // of a language was actually reached. On a repo whose sole root is
    // `public static void main`, "unreachable from roots" describes the
    // entire codebase.
    let mut lang_callables: BTreeMap<String, u32> = BTreeMap::new();
    let mut lang_reached: BTreeMap<String, u32> = BTreeMap::new();
    let mut lang_roots: BTreeMap<String, u32> = BTreeMap::new();
    for i in 0..n as u32 {
        let Some(nd) = node_at(i) else { continue };
        if !is_candidate(nd) {
            continue;
        }
        *lang_callables.entry(nd.language.clone()).or_insert(0) += 1;
        if live_any[i as usize] {
            *lang_reached.entry(nd.language.clone()).or_insert(0) += 1;
        }
    }
    for r in &root_set.records {
        *lang_roots.entry(r.language.clone()).or_insert(0) += 1;
    }
    let coverage_pct = |lang: &str| -> u8 {
        let total = lang_callables.get(lang).copied().unwrap_or(0);
        if total == 0 {
            return 0;
        }
        let reached = lang_reached.get(lang).copied().unwrap_or(0);
        ((reached as u64 * 100) / total as u64) as u8
    };
    let coverage_ok = |lang: &str| -> bool {
        lang_roots.get(lang).copied().unwrap_or(0) > 0
            && coverage_pct(lang) >= DEFAULT_MIN_ROOT_COVERAGE_PCT
    };

    // --- Classify ----------------------------------------------------------
    let is_root: Vec<bool> = {
        let mut v = vec![false; n];
        for &i in all_roots.iter() {
            v[i as usize] = true;
        }
        v
    };

    let mut category: Vec<Option<FindingCategory>> = vec![None; n];
    for i in 0..n as u32 {
        let Some(nd) = node_at(i) else { continue };
        if !is_candidate(nd) || is_root[i as usize] {
            continue;
        }
        let indeg = adjacency.in_degree(i);
        category[i as usize] = if indeg == 0 {
            Some(FindingCategory::NeverReferenced)
        } else if !live_any[i as usize] {
            Some(FindingCategory::UnreachableFromRoots)
        } else if !live_prod[i as usize] {
            Some(FindingCategory::OnlyUsedByTests)
        } else {
            None
        };
    }

    // Dead subgraph: refine "unreachable" into cycles vs. downstream.
    let in_dead: Vec<bool> = category
        .iter()
        .map(|c| {
            matches!(
                c,
                Some(FindingCategory::NeverReferenced)
                    | Some(FindingCategory::UnreachableFromRoots)
            )
        })
        .collect();
    let (scc_of, sccs) = scc::tarjan_scc(&adjacency, &in_dead);
    let region_of = scc::weak_components(&adjacency, &in_dead);

    for i in 0..n as u32 {
        if !in_dead[i as usize] {
            continue;
        }
        if category[i as usize] == Some(FindingCategory::UnreachableFromRoots) {
            let cid = scc_of[i as usize];
            let in_cycle = cid != u32::MAX && sccs[cid as usize].len() > 1;
            category[i as usize] = Some(if in_cycle {
                FindingCategory::DeadCycle
            } else if adjacency.pred(i).any(|(p, _)| in_dead[p as usize]) {
                FindingCategory::ReachableOnlyFromDeadCode
            } else {
                FindingCategory::UnreachableFromRoots
            });
        }
    }

    // --- Build findings ----------------------------------------------------
    let mut findings: Vec<DeadCodeFinding> = Vec::new();
    let mut withheld: BTreeMap<(String, FindingCategory), (SuppressionReason, u32)> =
        BTreeMap::new();

    for i in 0..n as u32 {
        let Some(cat) = category[i as usize] else {
            continue;
        };
        let Some(nd) = node_at(i) else { continue };
        let lang = nd.language.clone();
        let m = measured.get(&lang);

        // Withhold rather than mislead.
        let suppression = match m.map(|m| m.class) {
            Some(LanguageClass::Descriptor) => {
                Some(SuppressionReason::DescriptorLanguage)
            }
            Some(LanguageClass::ScriptDriven) if cat.depends_on_roots() => {
                Some(SuppressionReason::ScriptDriven)
            }
            _ if cat.depends_on_roots() && !coverage_ok(&lang) => {
                Some(if lang_roots.get(&lang).copied().unwrap_or(0) == 0 {
                    SuppressionReason::NoRootsFound
                } else {
                    SuppressionReason::LowRootCoverage
                })
            }
            _ => None,
        };
        if cat == FindingCategory::OnlyUsedByTests && !opts.include_tests {
            withheld
                .entry((lang.clone(), cat))
                .or_insert((SuppressionReason::MissingSignal, 0))
                .1 += 1;
            continue;
        }
        if let Some(reason) = suppression {
            withheld.entry((lang.clone(), cat)).or_insert((reason, 0)).1 += 1;
            continue;
        }

        // --- Evidence ------------------------------------------------------
        let mut ev: Vec<Evidence> = Vec::new();
        let indeg = adjacency.in_degree(i);
        if indeg == 0 {
            ev.push(Evidence::NoIncomingEdges);
        } else {
            let dead_callers: Vec<CallableId> = adjacency
                .pred(i)
                .filter(|(p, _)| in_dead[*p as usize])
                .filter_map(|(p, _)| node_at(p).map(|x| x.id))
                .collect();
            if let Some(first) = dead_callers.first().copied() {
                ev.push(Evidence::IncomingOnlyFromDeadCode {
                    callers: dead_callers.len() as u32,
                    example: first,
                });
            }
        }
        let cid = scc_of[i as usize];
        if cid != u32::MAX && sccs[cid as usize].len() > 1 {
            ev.push(Evidence::InCycle {
                scc_size: sccs[cid as usize].len() as u32,
            });
        }
        ev.extend(unresolved.evidence_for(graph, nd));
        if dyn_named.contains(&(nd.language.as_str(), nd.simple_name.as_str())) {
            ev.push(Evidence::NameCollidesWithScreenedSite {
                screen: "reflection".into(),
                sites: 1,
            });
        }
        if nd.test_role.is_some() && !opts.include_tests {
            // Test-scope code is reported separately, not as production
            // dead code.
            continue;
        }

        if let Some(m) = m {
            if !m.visibility.is_present() {
                ev.push(Evidence::LanguageLacksVisibility);
            }
            if !m.attributes.is_present() {
                ev.push(Evidence::LanguageLacksAttributes);
            }
            if !m.value_references.is_present() {
                ev.push(Evidence::LanguageLacksValueReferences);
            }
            if !m.dispatch.is_present() {
                ev.push(Evidence::LanguageLacksDispatchModel);
            }
            match m.class {
                LanguageClass::Descriptor => ev.push(Evidence::LanguageIsDescriptor),
                LanguageClass::ScriptDriven => ev.push(Evidence::LanguageIsScriptDriven),
                LanguageClass::Analyzable => ev.push(Evidence::LanguageSignalsComplete),
                LanguageClass::Degraded => {}
            }
        }
        if cat.depends_on_roots() && !coverage_ok(&lang) {
            ev.push(Evidence::LowRootCoverage {
                roots: lang_roots.get(&lang).copied().unwrap_or(0),
                callables: lang_callables.get(&lang).copied().unwrap_or(0),
                reachable_pct: coverage_pct(&lang),
            });
        }
        if evidence::is_implicitly_invokable(nd.kind) {
            ev.push(Evidence::ImplicitlyInvokableKind {
                callable_kind: nd.kind,
            });
        }
        // Read from the normalized `vis`, never from the language-native
        // token: `pub`, `public` and `pub(crate)` fall into three
        // different buckets that no prefix test can separate. The token
        // is still what the report shows.
        match nd.vis {
            // Confined to the analyzed unit, so no out-of-tree caller
            // can exist — the finding is corroborated.
            Vis::Private | Vis::Internal => ev.push(Evidence::PrivateVisibility {
                token: vis_token(nd),
            }),
            // Exported, so a caller may live in code cgg never saw.
            Vis::Public => ev.push(Evidence::PublicVisibility {
                token: vis_token(nd),
            }),
            // `Protected` reaches out-of-tree subtypes but not arbitrary
            // callers, and cgg models no subtype graph; `Unknown` means
            // the plugin never determined it. Claiming either direction
            // for these would be a guess, which is why `Vis::escapes_unit`
            // — true for `Unknown` — is deliberately not used here.
            Vis::Protected | Vis::Unknown => {}
        }

        ev.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
        ev.dedup_by(|a, b| a.slug() == b.slug());

        let signals_complete = m.map(|m| m.signals_complete()).unwrap_or(false);
        let confidence = evidence::derive_confidence(
            cat,
            &ev,
            indeg == 0,
            signals_complete,
            coverage_ok(&lang),
        );
        let size_lines = nd.end_line.saturating_sub(nd.start_line) + 1;
        let region = region_of.get(i as usize).copied().unwrap_or(u32::MAX);
        let role = if cid != u32::MAX && sccs[cid as usize].len() > 1 {
            RegionRole::CycleMember
        } else if indeg == 0 {
            RegionRole::Anchor
        } else {
            RegionRole::Downstream
        };

        findings.push(DeadCodeFinding {
            id: nd.id,
            qualified_name: nd.qualified_name.clone(),
            simple_name: nd.simple_name.clone(),
            language: lang,
            kind: nd.kind,
            def_variant: String::new(),
            file: nd.file,
            path: graph
                .files
                .get(&nd.file)
                .map(|f| f.path.clone())
                .unwrap_or_default(),
            start_line: nd.start_line,
            end_line: nd.end_line,
            size_lines,
            signature_hint: nd.signature_hint.clone(),
            visibility: nd.visibility.clone(),
            category: cat,
            confidence,
            rank: evidence::rank_of(&ev, size_lines),
            region,
            role,
            evidence: ev,
            dead_callers: Vec::new(),
            out_degree: adjacency.out_degree(i),
        });
    }

    // --- Regions -----------------------------------------------------------
    let mut regions_map: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for i in 0..n as u32 {
        if in_dead[i as usize] {
            let r = region_of[i as usize];
            if r != u32::MAX {
                regions_map.entry(r).or_default().push(i);
            }
        }
    }
    let regions: Vec<DeadRegion> = regions_map
        .into_iter()
        .map(|(id, members)| {
            let mut files: Vec<_> = members
                .iter()
                .filter_map(|&i| node_at(i).map(|x| x.file))
                .collect();
            files.sort_unstable_by_key(|f| f.as_u32());
            files.dedup();
            let mut languages: Vec<String> =
                members.iter().map(|&i| lang_at(i)).collect();
            languages.sort();
            languages.dedup();
            let anchors = members
                .iter()
                .filter(|&&i| adjacency.in_degree(i) == 0)
                .filter_map(|&i| node_at(i).map(|x| x.id))
                .collect();
            let cycles = members
                .iter()
                .map(|&i| scc_of[i as usize])
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .filter(|&c| c != u32::MAX && sccs[c as usize].len() > 1)
                .map(|c| {
                    sccs[c as usize]
                        .iter()
                        .filter_map(|&m| node_at(m).map(|x| x.id))
                        .collect()
                })
                .collect();
            let total_lines = members
                .iter()
                .filter_map(|&i| node_at(i))
                .map(|nd| nd.end_line.saturating_sub(nd.start_line) + 1)
                .sum();
            let confidence = members
                .iter()
                .filter_map(|&i| node_at(i))
                .filter_map(|nd| {
                    findings
                        .iter()
                        .find(|f| f.id == nd.id)
                        .map(|f| f.confidence)
                })
                .min_by_key(|c| match c {
                    Confidence::Low => 0,
                    Confidence::Medium => 1,
                    Confidence::High => 2,
                })
                .unwrap_or(Confidence::Low);
            DeadRegion {
                id,
                members: members
                    .iter()
                    .filter_map(|&i| node_at(i).map(|x| x.id))
                    .collect(),
                anchors,
                cycles,
                files,
                total_lines,
                languages,
                confidence,
            }
        })
        .collect();

    // --- Sort: a total order, so output is byte-stable ---------------------
    findings.sort_by(|a, b| {
        let band = |c: Confidence| match c {
            Confidence::High => 0u8,
            Confidence::Medium => 1,
            Confidence::Low => 2,
        };
        band(a.confidence)
            .cmp(&band(b.confidence))
            .then(b.rank.cmp(&a.rank))
            .then(a.path.cmp(&b.path))
            .then(a.start_line.cmp(&b.start_line))
            .then(a.qualified_name.cmp(&b.qualified_name))
    });

    // --- Capability table --------------------------------------------------
    let capabilities: Vec<LanguageCapabilityReport> = measured
        .iter()
        .map(|(lang, m)| LanguageCapabilityReport {
            language: lang.clone(),
            class: m.class,
            visibility: m.visibility,
            attributes: m.attributes,
            value_references: m.value_references,
            dispatch: m.dispatch,
            exports: m.exports,
            test_tagging: m.test_tagging,
            max_confidence: if m.signals_complete() {
                Confidence::High
            } else {
                Confidence::Medium
            },
            root_rules_active: root_set
                .rules_by_language
                .get(lang)
                .cloned()
                .unwrap_or_default(),
            files: m.files,
            callables: m.callables,
            roots: lang_roots.get(lang).copied().unwrap_or(0),
            reachable: lang_reached.get(lang).copied().unwrap_or(0),
            reachable_pct: coverage_pct(lang),
            findings: findings.iter().filter(|f| &f.language == lang).count() as u32,
            blind_spots: m.blind_spots.clone(),
        })
        .collect();

    let candidates = category.iter().filter(|c| c.is_some()).count() as u32;
    let root_reachability = if root_set.production.is_empty() {
        "disabled:no-roots-found".to_string()
    } else {
        format!("enabled:min-coverage-{DEFAULT_MIN_ROOT_COVERAGE_PCT}pct")
    };

    DeadCodeReport {
        config: cgg_core::deadcode::DeadCodeConfig {
            confidence_threshold: opts.confidence_threshold.clone(),
            roots_file: opts.roots_file.clone(),
            include_tests: opts.include_tests,
            reference_edges: opts.reference_edges,
            dynamic_dispatch: opts.dynamic_dispatch,
            root_reachability,
        },
        capabilities,
        summary: DeadCodeSummary {
            review_required: true,
            callables: graph.callables.values().filter(|n| is_candidate(n)).count()
                as u32,
            edges: graph.edges.len() as u32,
            unresolved_call_sites: graph.unresolved.len() as u32,
            roots: root_set.records.len() as u32,
            candidates,
            reported: findings.len() as u32,
            regions: regions.len() as u32,
            withheld: withheld
                .into_iter()
                .map(|((language, category), (reason, would_have_reported))| {
                    SuppressedCategory {
                        language,
                        category,
                        reason,
                        would_have_reported,
                    }
                })
                .collect(),
            stale_suppressions: opts.stale_patterns.clone(),
        },
        roots: root_set.records,
        regions,
        findings,
        ..Default::default()
    }
}

/// Shortest proving path from any seed, preferring strong edges.
///
/// Returns `(parent_node, parent_edge, settled)` indexed by node.
/// Ties are broken by lower node index, so the chosen path is the same
/// on every run.
fn search(adj: &Adj, graph: &Graph, seeds: &[u32]) -> (Vec<u32>, Vec<u32>, Vec<bool>) {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;

    let n = adj.n as usize;
    let mut dist = vec![u32::MAX; n];
    let mut parent = vec![u32::MAX; n];
    let mut parent_edge = vec![u32::MAX; n];
    let mut settled = vec![false; n];
    let mut heap: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new();

    for &s in seeds {
        if (s as usize) < n && dist[s as usize] != 0 {
            dist[s as usize] = 0;
            heap.push(Reverse((0, s)));
        }
    }
    while let Some(Reverse((d, v))) = heap.pop() {
        if settled[v as usize] {
            continue;
        }
        settled[v as usize] = true;
        for (w, ei) in adj.succ(v) {
            let c = graph.edges.get(ei as usize).map(edge_cost).unwrap_or(1);
            let nd = d.saturating_add(c);
            if nd < dist[w as usize] {
                dist[w as usize] = nd;
                parent[w as usize] = v;
                parent_edge[w as usize] = ei;
                heap.push(Reverse((nd, w)));
            }
        }
    }
    (parent, parent_edge, settled)
}

/// Explain why a callable is considered live.
///
/// The dual of a finding, and the reason the analysis can be argued with
/// in both directions: a reader who doubts that something is live can
/// ask for the proof and inspect every hop. When no path exists, that is
/// itself the answer — and it is the same claim the report makes, shown
/// as a derivation rather than an assertion.
pub fn why_live(
    graph: &Graph,
    facts: &[FileFacts],
    opts: &DeadCodeOptions,
    targets: &[CallableId],
) -> Vec<LivenessProof> {
    let adjacency = Adj::build(graph, &counts_for_liveness);
    let root_set = roots::discover(graph, facts, &opts.user_roots, &opts.framework_roots);

    let mut all_roots = root_set.production.clone();
    all_roots.extend_from_slice(&root_set.test);
    all_roots.sort_unstable();

    // Production roots first, so a proof from `main` always wins over a
    // proof from a test.
    let prod = search(&adjacency, graph, &root_set.production);
    let any = search(&adjacency, graph, &all_roots);

    let mut out = Vec::new();
    for &target in targets {
        let Some(ti) = graph.callables.get_index_of(&target) else {
            continue;
        };
        let ti = ti as u32;
        let node = &graph.callables[&target];

        let (status, chosen) = if prod.2[ti as usize] {
            ("live", Some(&prod))
        } else if any.2[ti as usize] {
            ("test-live", Some(&any))
        } else {
            ("dead", None)
        };

        let mut hops = Vec::new();
        let mut root_record = None;
        if let Some((parent, parent_edge, _)) = chosen {
            // Walk parents back to the seed, then reverse.
            let mut chain: Vec<(u32, u32)> = Vec::new();
            let mut cur = ti;
            while parent[cur as usize] != u32::MAX {
                chain.push((cur, parent_edge[cur as usize]));
                cur = parent[cur as usize];
            }
            chain.reverse();
            if let Some((_, rnode)) = graph.callables.get_index(cur as usize) {
                root_record = root_set.records.iter().find(|r| r.id == rnode.id).cloned();
            }
            for (to_idx, ei) in chain {
                let Some(e) = graph.edges.get(ei as usize) else {
                    continue;
                };
                let Some((_, to)) = graph.callables.get_index(to_idx as usize) else {
                    continue;
                };
                hops.push(ProofHop {
                    from: e.src,
                    to: e.dst,
                    to_qualified_name: to.qualified_name.clone(),
                    path: graph
                        .files
                        .get(&to.file)
                        .map(|f| f.path.clone())
                        .unwrap_or_default(),
                    line: to.start_line,
                    site_line: e.site_line,
                    via: match &e.via {
                        Via::Direct => "direct".into(),
                        Via::Dynamic => "dynamic".into(),
                        Via::Reference => "reference".into(),
                        Via::External => "external".into(),
                        Via::Stdlib => "stdlib".into(),
                        Via::Ffi(f) => format!("ffi:{f}"),
                        Via::FrameworkEntry(f) => format!("framework-entry:{f}"),
                    },
                    confidence: e.confidence,
                    resolver: e.resolver.as_str().to_string(),
                });
            }
        }

        let weakest_link = hops.iter().map(|h| h.confidence).min_by_key(|c| match c {
            Confidence::Low => 0u8,
            Confidence::Medium => 1,
            Confidence::High => 2,
        });

        out.push(LivenessProof {
            target,
            target_qualified_name: node.qualified_name.clone(),
            status: status.to_string(),
            root: root_record,
            hops,
            weakest_link,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use testutil::{graph_with, link, node};

    fn opts() -> DeadCodeOptions {
        DeadCodeOptions {
            confidence_threshold: "high".into(),
            ..Default::default()
        }
    }

    /// Options declaring rust's real signal coverage, mirroring
    /// `plugins::rust::signals()`.
    ///
    /// Without this `caps::measure` sees a toy graph carrying none of
    /// those strings, classes rust as `Degraded`, and caps *every*
    /// finding at `Medium` — which would make a "cannot reach high"
    /// assertion pass for the wrong reason.
    fn opts_rust_full() -> DeadCodeOptions {
        let mut o = opts();
        o.language_signals.insert(
            "rust".into(),
            cgg_core::deadcode::LanguageSignals {
                visibility: true,
                attributes: true,
                exports: true,
                test_defs: true,
                value_refs: true,
                impls: true,
                unreachable: true,
                dyn_uses: false,
            },
        );
        o
    }

    /// An unreferenced callable with an explicit visibility.
    fn vis_node(id: u32, qn: &str, simple: &str, token: &str, vis: Vis) -> CallableNode {
        let mut n = node(id, qn, simple, "rust");
        n.visibility = token.to_string();
        n.vis = vis;
        n
    }

    #[test]
    fn an_unreferenced_function_is_reported_and_its_caller_is_not() {
        let mut g = graph_with(vec![
            node(0, "crate::main", "main", "rust"),
            node(1, "crate::used", "used", "rust"),
            node(2, "crate::orphan", "orphan", "rust"),
        ]);
        link(&mut g, 0, 1);
        let r = analyze(&g, &[], &[], &opts());
        let names: Vec<_> = r
            .findings
            .iter()
            .map(|f| f.qualified_name.as_str())
            .collect();
        assert_eq!(names, vec!["crate::orphan"]);
        assert_eq!(r.findings[0].category, FindingCategory::NeverReferenced);
    }

    #[test]
    fn roots_are_never_reported() {
        let g = graph_with(vec![node(0, "crate::main", "main", "rust")]);
        assert!(analyze(&g, &[], &[], &opts()).findings.is_empty());
    }

    #[test]
    fn a_dead_cycle_is_found_and_marked() {
        // main is live; a <-> b are mutually recursive and unreachable.
        // A name-matching tool cannot report this: each marks the other used.
        let mut g = graph_with(vec![
            node(0, "crate::main", "main", "rust"),
            node(1, "crate::a", "a", "rust"),
            node(2, "crate::b", "b", "rust"),
        ]);
        link(&mut g, 1, 2);
        link(&mut g, 2, 1);
        let r = analyze(&g, &[], &[], &opts());
        assert_eq!(r.findings.len(), 2, "both cycle members reported");
        assert!(
            r.findings
                .iter()
                .all(|f| f.category == FindingCategory::DeadCycle)
        );
        assert!(r.findings.iter().all(|f| f.role == RegionRole::CycleMember));
        // One group, no entry point: every member is referenced, but
        // only from inside the ring.
        assert_eq!(r.regions.len(), 1);
        assert!(r.regions[0].anchors.is_empty());
        assert_eq!(r.regions[0].cycles.len(), 1);
    }

    #[test]
    fn a_chain_is_one_region_with_one_entry_point() {
        let mut g = graph_with(vec![
            node(0, "crate::main", "main", "rust"),
            node(1, "crate::a", "a", "rust"),
            node(2, "crate::b", "b", "rust"),
        ]);
        link(&mut g, 1, 2);
        let r = analyze(&g, &[], &[], &opts());
        assert_eq!(r.regions.len(), 1);
        assert_eq!(r.regions[0].members.len(), 2);
        assert_eq!(
            r.regions[0].anchors.len(),
            1,
            "`a` is the group's entry point"
        );
        let b = r.findings.iter().find(|f| f.simple_name == "b").unwrap();
        assert_eq!(b.category, FindingCategory::ReachableOnlyFromDeadCode);
    }

    #[test]
    fn the_reported_set_is_closed_in_one_pass() {
        let mut g = graph_with(vec![
            node(0, "crate::main", "main", "rust"),
            node(1, "crate::live", "live", "rust"),
            node(2, "crate::a", "a", "rust"),
            node(3, "crate::b", "b", "rust"),
            node(4, "crate::c", "c", "rust"),
        ]);
        link(&mut g, 0, 1);
        link(&mut g, 2, 3);
        link(&mut g, 3, 4);
        let first = analyze(&g, &[], &[], &opts());
        assert_eq!(first.findings.len(), 3);

        // Remove every reported callable from the graph and re-run: a
        // second pass must have nothing further to say.
        let dead: std::collections::HashSet<_> =
            first.findings.iter().map(|f| f.id).collect();
        let mut g2 = g.clone();
        g2.callables.retain(|id, _| !dead.contains(id));
        g2.edges
            .retain(|e| !dead.contains(&e.src) && !dead.contains(&e.dst));
        assert!(
            analyze(&g2, &[], &[], &opts()).findings.is_empty(),
            "the reported set must be closed"
        );
    }

    #[test]
    fn synthetic_and_destructor_nodes_are_never_candidates() {
        let mut ext = node(1, "ext::thing", "thing", "rust");
        ext.synthetic = true;
        let mut drop_impl = node(2, "crate::T::drop", "drop", "rust");
        drop_impl.kind = CallableKind::Destructor;
        let g = graph_with(vec![node(0, "crate::main", "main", "rust"), ext, drop_impl]);
        assert!(analyze(&g, &[], &[], &opts()).findings.is_empty());
    }

    #[test]
    fn external_and_stdlib_edges_do_not_confer_liveness() {
        let mut g = graph_with(vec![
            node(0, "crate::main", "main", "rust"),
            node(1, "crate::orphan", "orphan", "rust"),
        ]);
        // An External edge into `orphan` must not save it.
        let mut e = testutil::edge(0, 1);
        e.via = Via::External;
        g.edges.push(e);
        let r = analyze(&g, &[], &[], &opts());
        assert_eq!(r.findings.len(), 1);
        assert_eq!(r.findings[0].simple_name, "orphan");
    }

    #[test]
    fn reference_edges_confer_liveness() {
        // `register(handler)` — the canonical false positive.
        let mut g = graph_with(vec![
            node(0, "crate::main", "main", "rust"),
            node(1, "crate::handler", "handler", "rust"),
        ]);
        let mut e = testutil::edge(0, 1);
        e.via = Via::Reference;
        g.edges.push(e);
        assert!(analyze(&g, &[], &[], &opts()).findings.is_empty());
    }

    #[test]
    fn findings_are_independent_of_edge_insertion_order() {
        let build = |order: &[(u32, u32)]| {
            let mut g = graph_with(vec![
                node(0, "crate::main", "main", "rust"),
                node(1, "crate::a", "a", "rust"),
                node(2, "crate::b", "b", "rust"),
                node(3, "crate::c", "c", "rust"),
            ]);
            for &(s, d) in order {
                link(&mut g, s, d);
            }
            analyze(&g, &[], &[], &opts())
        };
        let a = build(&[(0, 1), (2, 3)]);
        let b = build(&[(2, 3), (0, 1)]);
        let names = |r: &DeadCodeReport| -> Vec<String> {
            r.findings
                .iter()
                .map(|f| f.qualified_name.clone())
                .collect()
        };
        assert_eq!(names(&a), names(&b));
        assert_eq!(format!("{:?}", a.findings), format!("{:?}", b.findings));
    }

    #[test]
    fn the_report_always_carries_the_disclaimer() {
        let g = graph_with(vec![node(0, "crate::main", "main", "rust")]);
        let r = analyze(&g, &[], &[], &opts());
        assert_eq!(r.disclaimer, cgg_core::deadcode::DEAD_CODE_DISCLAIMER);
        assert!(r.best_effort);
        assert!(r.summary.review_required);
    }

    #[test]
    fn a_language_with_no_signals_cannot_reach_high_confidence() {
        let mut g = graph_with(vec![
            node(0, "pkg.Main.main", "main", "java"),
            node(1, "pkg.Helper.unused", "unused", "java"),
        ]);
        link(&mut g, 0, 0);
        let r = analyze(&g, &[], &[], &opts());
        let f = r
            .findings
            .iter()
            .find(|f| f.simple_name == "unused")
            .unwrap();
        assert_ne!(f.confidence, Confidence::High, "java lacks every signal");
        assert!(
            f.evidence
                .iter()
                .any(|e| e.slug() == "language-lacks-visibility")
        );
        assert!(
            f.evidence
                .iter()
                .any(|e| e.slug() == "language-lacks-attributes")
        );
    }

    #[test]
    fn an_exported_callable_cannot_reach_high_but_a_private_one_can() {
        // The whole point: for a library crate, "nothing in this tree
        // references it" is the normal state of the public API, and cgg
        // structurally cannot see the callers that would refute it.
        let g = graph_with(vec![
            node(0, "crate::main", "main", "rust"),
            vis_node(1, "crate::exported_api", "exported_api", "pub", Vis::Public),
            vis_node(
                2,
                "crate::private_orphan",
                "private_orphan",
                "",
                Vis::Private,
            ),
        ]);
        let r = analyze(&g, &[], &[], &opts_rust_full());

        let pubf = r
            .findings
            .iter()
            .find(|f| f.simple_name == "exported_api")
            .unwrap();
        assert_eq!(
            pubf.confidence,
            Confidence::Medium,
            "an out-of-tree caller is possible, so this must not be top-band"
        );
        assert!(
            pubf.evidence
                .iter()
                .any(|e| e.slug() == "public-visibility")
        );
        assert!(matches!(
            pubf.evidence.iter().find(|e| e.slug() == "public-visibility"),
            Some(Evidence::PublicVisibility { token }) if token == "pub",
        ));
        assert!(
            !pubf
                .evidence
                .iter()
                .any(|e| e.slug() == "private-visibility")
        );

        let privf = r
            .findings
            .iter()
            .find(|f| f.simple_name == "private_orphan")
            .unwrap();
        assert_eq!(
            privf.confidence,
            Confidence::High,
            "nothing outside the tree can reach it, so nothing is withheld from cgg"
        );
        assert!(
            privf
                .evidence
                .iter()
                .any(|e| e.slug() == "private-visibility")
        );
        assert!(
            !privf
                .evidence
                .iter()
                .any(|e| e.slug() == "public-visibility")
        );
        // Rust spells private by writing nothing; the report must still
        // say something.
        assert!(matches!(
            privf.evidence.iter().find(|e| e.slug() == "private-visibility"),
            Some(Evidence::PrivateVisibility { token }) if token == "private",
        ));
    }

    #[test]
    fn internal_visibility_corroborates_and_keeps_the_native_token() {
        // `pub(crate)` escapes no compilation unit, so it corroborates
        // exactly like a private item — but the old prefix test read it
        // as "pub" and drew the opposite conclusion.
        let g = graph_with(vec![
            node(0, "crate::main", "main", "rust"),
            vis_node(1, "crate::helper", "helper", "pub(crate)", Vis::Internal),
        ]);
        let r = analyze(&g, &[], &[], &opts_rust_full());
        let f = r
            .findings
            .iter()
            .find(|f| f.simple_name == "helper")
            .unwrap();
        assert_eq!(f.confidence, Confidence::High);
        assert!(matches!(
            f.evidence.iter().find(|e| e.slug() == "private-visibility"),
            Some(Evidence::PrivateVisibility { token }) if token == "pub(crate)",
        ));
    }

    #[test]
    fn undetermined_visibility_claims_neither_direction() {
        // `Vis::escapes_unit()` is true for `Unknown`, so deriving the
        // caveat from it would turn "cgg did not look" into "this is
        // exported". Neither entry may appear.
        let g = graph_with(vec![
            node(0, "crate::main", "main", "rust"),
            vis_node(1, "crate::inherited", "inherited", "", Vis::Unknown),
            vis_node(2, "crate::guarded", "guarded", "protected", Vis::Protected),
        ]);
        let r = analyze(&g, &[], &[], &opts_rust_full());
        assert_eq!(r.findings.len(), 2);
        for f in &r.findings {
            assert!(
                !f.evidence.iter().any(|e| matches!(
                    e.slug(),
                    "public-visibility" | "private-visibility"
                )),
                "{} must claim neither direction",
                f.simple_name
            );
        }
    }

    #[test]
    fn descriptor_language_findings_are_withheld_with_a_count() {
        let g = graph_with(vec![node(0, "Shape", "Shape", "openapi")]);
        let r = analyze(&g, &[], &[], &opts());
        assert!(
            r.findings.is_empty(),
            "an unreferenced schema is a wire contract"
        );
        assert_eq!(r.summary.withheld.len(), 1);
        assert_eq!(r.summary.withheld[0].would_have_reported, 1);
        assert_eq!(
            r.summary.withheld[0].reason,
            SuppressionReason::DescriptorLanguage
        );
    }

    #[test]
    fn capability_table_discloses_missing_signals() {
        let g = graph_with(vec![node(0, "pkg.f", "f", "java")]);
        let r = analyze(&g, &[], &[], &opts());
        let java = r
            .capabilities
            .iter()
            .find(|c| c.language == "java")
            .unwrap();
        assert_eq!(java.class, LanguageClass::Degraded);
        assert_eq!(java.max_confidence, Confidence::Medium);
    }

    #[test]
    fn why_live_proves_liveness_with_a_path_from_a_root() {
        let mut g = graph_with(vec![
            node(0, "crate::main", "main", "rust"),
            node(1, "crate::a", "a", "rust"),
            node(2, "crate::b", "b", "rust"),
        ]);
        link(&mut g, 0, 1);
        link(&mut g, 1, 2);
        let p = &why_live(&g, &[], &opts(), &[CallableId::new(2)])[0];
        assert_eq!(p.status, "live");
        assert_eq!(p.hops.len(), 2, "main -> a -> b");
        assert_eq!(p.hops[0].to_qualified_name, "crate::a");
        assert_eq!(p.hops[1].to_qualified_name, "crate::b");
        assert_eq!(p.root.as_ref().unwrap().qualified_name, "crate::main");
    }

    #[test]
    fn why_live_on_an_unreferenced_callable_reports_dead_with_no_path() {
        let g = graph_with(vec![
            node(0, "crate::main", "main", "rust"),
            node(1, "crate::orphan", "orphan", "rust"),
        ]);
        let p = &why_live(&g, &[], &opts(), &[CallableId::new(1)])[0];
        assert_eq!(p.status, "dead");
        assert!(p.hops.is_empty());
        assert!(p.root.is_none());
    }

    #[test]
    fn why_live_distinguishes_test_only_liveness() {
        let mut g = graph_with(vec![
            node(0, "crate::main", "main", "rust"),
            node(1, "crate::helper", "helper", "rust"),
            node(2, "crate::tests::t", "t", "rust"),
        ]);
        g.callables[&CallableId::new(2)].attributes = vec!["#[test]".into()];
        link(&mut g, 2, 1);
        let p = &why_live(&g, &[], &opts(), &[CallableId::new(1)])[0];
        assert_eq!(p.status, "test-live", "only a test reaches it");
        assert!(p.root.as_ref().unwrap().kind.is_test());
    }

    #[test]
    fn why_live_prefers_a_direct_proof_over_a_dynamic_one() {
        // main -> mid -> target (two direct hops) vs main -> target
        // (one dynamic hop). The shorter path is the weaker proof, so
        // the search must not take it.
        let mut g = graph_with(vec![
            node(0, "crate::main", "main", "rust"),
            node(1, "crate::mid", "mid", "rust"),
            node(2, "crate::target", "target", "rust"),
        ]);
        link(&mut g, 0, 1);
        link(&mut g, 1, 2);
        let mut dyn_edge = testutil::edge(0, 2);
        dyn_edge.via = Via::Dynamic;
        dyn_edge.confidence = Confidence::Low;
        g.edges.push(dyn_edge);
        let p = &why_live(&g, &[], &opts(), &[CallableId::new(2)])[0];
        assert_eq!(p.hops.len(), 2, "should route through the direct chain");
        assert!(p.hops.iter().all(|h| h.via == "direct"));
        assert_eq!(p.weakest_link, Some(Confidence::High));
    }

    #[test]
    fn why_live_reports_the_weakest_hop_so_a_shaky_proof_looks_shaky() {
        let mut g = graph_with(vec![
            node(0, "crate::main", "main", "rust"),
            node(1, "crate::target", "target", "rust"),
        ]);
        let mut e = testutil::edge(0, 1);
        e.confidence = Confidence::Low;
        e.via = Via::Reference;
        g.edges.push(e);
        let p = &why_live(&g, &[], &opts(), &[CallableId::new(1)])[0];
        assert_eq!(p.status, "live");
        assert_eq!(p.weakest_link, Some(Confidence::Low));
    }

    #[test]
    fn why_live_is_deterministic_when_two_proofs_tie() {
        let build = |flip: bool| {
            let mut g = graph_with(vec![
                node(0, "crate::main", "main", "rust"),
                node(1, "crate::x", "x", "rust"),
                node(2, "crate::y", "y", "rust"),
                node(3, "crate::t", "t", "rust"),
            ]);
            let pairs = if flip {
                [(0u32, 2u32), (2, 3), (0, 1), (1, 3)]
            } else {
                [(0, 1), (1, 3), (0, 2), (2, 3)]
            };
            for (s, d) in pairs {
                link(&mut g, s, d);
            }
            why_live(&g, &[], &opts(), &[CallableId::new(3)])[0].clone()
        };
        assert_eq!(format!("{:?}", build(false)), format!("{:?}", build(true)));
    }
}
