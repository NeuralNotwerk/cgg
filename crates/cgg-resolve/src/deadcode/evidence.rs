//! Evidence gathering and confidence derivation.
//!
//! The dominant false-positive source is a call the resolver could not
//! place. On cgg's own source, 308 call sites are unresolved and 71 of
//! the 410 in-degree-zero callables share a simple name with one of
//! them: for each of those, the resolver saw a real call it could not
//! attribute, and this callable may well be its target. Reporting such
//! a callable as confidently dead would be wrong, so the correlation
//! below is what keeps the top tier honest.
//!
//! Confidence is derived by **capping**, never by adding. Most negative
//! evidence here is "cgg structurally cannot know" — a language with no
//! visibility extraction does not make a finding somewhat weaker, it
//! makes `High` unreachable.

use std::collections::HashMap;

use cgg_core::audit::{AuditFileRecord, AuditUnresolvedCall, UnresolvedReason};
use cgg_core::deadcode::{Evidence, FindingCategory, SiteRef};
use cgg_core::graph::{CallableKind, CallableNode, Confidence, Graph};

use crate::names::owner_from_qn;

/// Receiver hints that name the calling context rather than a concrete
/// type. The resolver deliberately declines to narrow on these, so they
/// cannot be used to rule a candidate out.
const SELF_HINTS: &[&str] = &["self", "Self", "this", "cls", "super"];

/// Unresolved call sites indexed for name correlation.
#[derive(Debug)]
pub(crate) struct UnresolvedIndex<'a> {
    /// From `graph.unresolved`, which the audit reconciliation pass has
    /// already stripped of anything that later resolved — so every entry
    /// here is genuinely unresolved.
    by_name: HashMap<(&'a str, &'a str), Vec<&'a AuditUnresolvedCall>>,
    /// From `AuditFileRecord::external_calls` only.
    ///
    /// Deliberately *not* from `stdlib_calls`: `new`, `len`, `push`,
    /// `clone`, `map` and friends would name-match a large fraction of
    /// any codebase and drown the signal. cgg's own run has 1275 stdlib
    /// hits against 308 unresolved.
    external_by_name: HashMap<(&'a str, &'a str), u32>,
}

impl<'a> UnresolvedIndex<'a> {
    pub(crate) fn build(graph: &'a Graph, file_audits: &'a [AuditFileRecord]) -> Self {
        let lang_of = |f: &cgg_core::ids::FileId| -> &'a str {
            graph
                .files
                .get(f)
                .map(|r| r.language.as_str())
                .unwrap_or("")
        };

        let mut by_name: HashMap<(&str, &str), Vec<&AuditUnresolvedCall>> =
            HashMap::new();
        for u in &graph.unresolved {
            by_name
                .entry((lang_of(&u.file), u.name.as_str()))
                .or_default()
                .push(u);
        }

        let mut external_by_name: HashMap<(&str, &str), u32> = HashMap::new();
        for fa in file_audits {
            for u in &fa.external_calls {
                *external_by_name
                    .entry((fa.language.as_str(), u.name.as_str()))
                    .or_insert(0) += 1;
            }
        }

        Self {
            by_name,
            external_by_name,
        }
    }

    /// Evidence that this callable may in fact be called, from sites the
    /// resolver failed to place.
    pub(crate) fn evidence_for(
        &self,
        graph: &Graph,
        node: &CallableNode,
    ) -> Vec<Evidence> {
        let key = (node.language.as_str(), node.simple_name.as_str());
        let mut out = Vec::new();

        let Some(sites) = self.by_name.get(&key) else {
            out.push(Evidence::NoUnresolvedSiteWithName);
            if let Some(&n) = self.external_by_name.get(&key) {
                out.push(Evidence::NameCollidesWithScreenedSite {
                    screen: "external".into(),
                    sites: n,
                });
            }
            return out;
        };

        let owner = owner_from_qn(&node.qualified_name);
        let site_ref = |u: &AuditUnresolvedCall| SiteRef {
            file: u.file,
            path: graph
                .files
                .get(&u.file)
                .map(|f| f.path.clone())
                .unwrap_or_default(),
            line: u.site_line,
            name: u.name.clone(),
            receiver_hint: u.receiver_hint.clone(),
        };

        // The sharpest case: the resolver had two or more same-name
        // candidates *in this very file* and refused to choose. This is
        // the dominant unresolved bucket (172 of 308 on cgg itself).
        let same_file_ambiguous: Vec<_> = sites
            .iter()
            .filter(|u| {
                u.file == node.file
                    && matches!(u.reason, UnresolvedReason::AmbiguousInFile)
            })
            .collect();
        if let Some(first) = same_file_ambiguous.first() {
            out.push(Evidence::AmbiguousSiteInSameFile {
                sites: same_file_ambiguous.len() as u32,
                file_local_candidates: first.candidates.file_local,
                example: site_ref(first),
            });
        }

        // Sites whose receiver hint does not contradict this candidate.
        let plausible: Vec<_> = sites
            .iter()
            .filter(|u| {
                let rh = u.receiver_hint.as_str();
                rh.is_empty()
                    || SELF_HINTS.contains(&rh)
                    || owner.is_some_and(|o| o == rh)
            })
            .collect();

        if let Some(first) = plausible.first() {
            let same_file =
                plausible.iter().filter(|u| u.file == node.file).count() as u32;
            let owner_match = plausible
                .iter()
                .filter(|u| owner.is_some_and(|o| o == u.receiver_hint))
                .count() as u32;
            out.push(Evidence::NameMatchesUnresolvedSite {
                name: node.simple_name.clone(),
                sites: plausible.len() as u32,
                reason: first.reason.slug().to_string(),
                same_file_sites: same_file,
                owner_match_sites: owner_match,
                example: site_ref(first),
            });
        } else if same_file_ambiguous.is_empty() {
            // Every site named a different owner, so none of them can be
            // this callable.
            out.push(Evidence::NoUnresolvedSiteWithName);
        }

        out
    }
}

/// Kinds that a language routinely invokes by syntax rather than by a
/// visible call.
pub(crate) fn is_implicitly_invokable(kind: CallableKind) -> bool {
    matches!(
        kind,
        CallableKind::Constructor | CallableKind::Destructor | CallableKind::Property
    )
}

/// Reduce a base confidence by every cap the evidence imposes.
pub(crate) fn apply_caps(base: Confidence, evidence: &[Evidence]) -> Confidence {
    let rank = |c: Confidence| match c {
        Confidence::High => 2u8,
        Confidence::Medium => 1,
        Confidence::Low => 0,
    };
    let mut cur = base;
    for e in evidence {
        if let Some(cap) = e.cap()
            && rank(cap) < rank(cur) {
                cur = cap;
            }
    }
    cur
}

/// Final confidence for a finding.
///
/// There is exactly one promotion path and it is conjunctive: `High`
/// means *nothing references this, in a language where cgg can see every
/// signal it models, with adequate root coverage, and with no reason on
/// record to doubt it*. Anything less lands at `Medium` or below.
pub(crate) fn derive_confidence(
    category: FindingCategory,
    evidence: &[Evidence],
    in_degree_zero: bool,
    signals_complete: bool,
    root_coverage_ok: bool,
) -> Confidence {
    let capped = apply_caps(category.base_confidence(), evidence);
    let has_doubt = evidence
        .iter()
        .any(|e| matches!(e.polarity(), cgg_core::deadcode::Polarity::Lowers));

    if capped == Confidence::Medium
        && in_degree_zero
        && signals_complete
        && root_coverage_ok
        && !has_doubt
    {
        return Confidence::High;
    }
    capped
}

/// Deterministic ordering key within a confidence band. Larger sorts
/// first. Never a probability.
pub(crate) fn rank_of(evidence: &[Evidence], size_lines: u32) -> i32 {
    let w: i32 = evidence.iter().map(|e| e.weight()).sum();
    // Bigger dead blocks are worth a reviewer's attention sooner, but
    // only as a tiebreak within the band.
    w * 100 + (size_lines.min(999) as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::deadcode::Polarity;

    fn ev_low() -> Evidence {
        Evidence::AmbiguousSiteInSameFile {
            sites: 1,
            file_local_candidates: 2,
            example: SiteRef {
                file: cgg_core::ids::FileId::new(0),
                path: Default::default(),
                line: 1,
                name: "x".into(),
                receiver_hint: String::new(),
            },
        }
    }

    #[test]
    fn caps_only_lower_never_raise() {
        assert_eq!(apply_caps(Confidence::High, &[]), Confidence::High);
        assert_eq!(
            apply_caps(Confidence::High, &[Evidence::LanguageLacksVisibility]),
            Confidence::Medium
        );
        assert_eq!(apply_caps(Confidence::High, &[ev_low()]), Confidence::Low);
        // A raising evidence must not promote a capped value.
        assert_eq!(
            apply_caps(
                Confidence::Low,
                &[Evidence::PrivateVisibility {
                    token: "pub".into()
                }]
            ),
            Confidence::Low
        );
    }

    #[test]
    fn strongest_cap_wins_regardless_of_order() {
        let a = [Evidence::LanguageLacksVisibility, ev_low()];
        let b = [ev_low(), Evidence::LanguageLacksVisibility];
        assert_eq!(apply_caps(Confidence::High, &a), Confidence::Low);
        assert_eq!(apply_caps(Confidence::High, &b), Confidence::Low);
    }

    #[test]
    fn promotion_to_high_is_conjunctive() {
        let good = [
            Evidence::NoIncomingEdges,
            Evidence::NoUnresolvedSiteWithName,
        ];
        // NeverReferenced already starts High.
        assert_eq!(
            derive_confidence(FindingCategory::NeverReferenced, &good, true, true, true),
            Confidence::High
        );
        // A root-dependent category can be promoted only when every
        // condition holds.
        assert_eq!(
            derive_confidence(FindingCategory::DeadCycle, &good, true, true, true),
            Confidence::High
        );
        for (idz, sig, cov) in [
            (false, true, true),
            (true, false, true),
            (true, true, false),
        ] {
            assert_eq!(
                derive_confidence(FindingCategory::DeadCycle, &good, idz, sig, cov),
                Confidence::Medium,
                "promotion must require all three"
            );
        }
    }

    #[test]
    fn any_doubt_blocks_promotion() {
        let doubtful = [Evidence::NoIncomingEdges, Evidence::LanguageLacksAttributes];
        assert_eq!(
            derive_confidence(FindingCategory::DeadCycle, &doubtful, true, true, true),
            Confidence::Medium
        );
    }

    #[test]
    fn unresolved_name_collision_caps_a_never_referenced_finding() {
        let ev = [
            Evidence::NoIncomingEdges,
            Evidence::NameMatchesUnresolvedSite {
                name: "run".into(),
                sites: 2,
                reason: "no-candidate-in-file".into(),
                same_file_sites: 0,
                owner_match_sites: 0,
                example: SiteRef {
                    file: cgg_core::ids::FileId::new(0),
                    path: Default::default(),
                    line: 3,
                    name: "run".into(),
                    receiver_hint: String::new(),
                },
            },
        ];
        assert_eq!(
            derive_confidence(FindingCategory::NeverReferenced, &ev, true, true, true),
            Confidence::Medium,
            "a call the resolver could not place must block the top tier"
        );
    }

    #[test]
    fn implicitly_invokable_kinds() {
        assert!(is_implicitly_invokable(CallableKind::Constructor));
        assert!(is_implicitly_invokable(CallableKind::Destructor));
        assert!(is_implicitly_invokable(CallableKind::Property));
        assert!(!is_implicitly_invokable(CallableKind::Function));
        assert!(!is_implicitly_invokable(CallableKind::Method));
    }

    #[test]
    fn rank_orders_corroborated_findings_above_doubted_ones() {
        let strong = rank_of(&[Evidence::NoUnresolvedSiteWithName], 10);
        let weak = rank_of(&[Evidence::LanguageLacksVisibility], 10);
        assert!(strong > weak);
        // Size only breaks ties inside a band.
        let big = rank_of(&[Evidence::NoUnresolvedSiteWithName], 400);
        assert!(big > strong);
    }

    #[test]
    fn polarity_partitions_the_evidence_enum() {
        assert_eq!(Evidence::NoIncomingEdges.polarity(), Polarity::Neutral);
        assert_eq!(Evidence::LanguageIsDescriptor.polarity(), Polarity::Lowers);
        assert_eq!(Evidence::NotReexported.polarity(), Polarity::Raises);
    }
}
