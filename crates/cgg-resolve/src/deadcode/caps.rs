//! Per-language capability disclosure.
//!
//! Extraction coverage is very uneven: most of the 44 plugins declare no
//! `visibility` and no real `attributes`, and dispatch modelling only
//! produces edges for Rust. A report that presented a Rust finding and a
//! Fortran finding with equal authority would be lying, so every report
//! states per language what cgg could see.
//!
//! Deliberately no counts in this comment. The last version said "2 of
//! 44 plugins" and stayed wrong for seven more plugins; the numbers live
//! in each plugin's `signals()` and are measured at runtime by
//! [`measure`] below.
//!
//! The signal columns are **derived from the run**, not hardcoded. A
//! static 44-row table would drift from reality within a couple of
//! commits and there would be no test that caught it; measuring what
//! actually came out of the extractor cannot rot.

use std::collections::BTreeMap;

use cgg_core::deadcode::{LanguageClass, LanguageSignals, SignalSupport};
use cgg_core::graph::{Graph, Via};

/// Languages whose primary invocation path is top-level script code that
/// cgg does not model as a callable. A property of the language, not of
/// extraction, so this one is a list rather than a measurement.
const SCRIPT_DRIVEN: &[&str] = &[
    "bash",
    "cmake",
    "nix",
    "hcl",
    "starlark",
    "powershell",
    "r",
    "perl",
];

/// Interface / descriptor languages. Their "callables" are schemas and
/// their "edges" are `$ref` pointers, so an unreferenced node is a wire
/// contract whose consumers are by definition in another repository.
const DESCRIPTOR: &[&str] = &["smithy", "proto", "graphql", "openapi", "asyncapi"];

/// Known blind spots worth naming explicitly in the report.
const BLIND_SPOTS: &[(&str, &[&str])] = &[
    (
        "rust",
        &[
            "calls inside macro arguments (format!, writeln!, vec!, assert_eq!)",
            "symbols named only in attribute string literals (#[serde(deserialize_with = \"…\")])",
        ],
    ),
    (
        "python",
        &["getattr/setattr and other string-dispatched calls"],
    ),
    ("javascript", &["obj[expr] dynamic property access"]),
    ("typescript", &["obj[expr] dynamic property access"]),
];

/// What cgg measured about one language in this run.
#[derive(Debug, Clone)]
pub(crate) struct Measured {
    pub(crate) class: LanguageClass,
    pub(crate) visibility: SignalSupport,
    pub(crate) attributes: SignalSupport,
    pub(crate) value_references: SignalSupport,
    pub(crate) dispatch: SignalSupport,
    pub(crate) exports: SignalSupport,
    pub(crate) test_tagging: SignalSupport,
    pub(crate) files: u32,
    pub(crate) callables: u32,
    pub(crate) blind_spots: Vec<String>,
}

impl Measured {
    /// Whether cgg extracted every signal the model knows how to use.
    /// Only a language clearing this bar can produce a `High` finding.
    pub(crate) fn signals_complete(&self) -> bool {
        // Visibility and attributes are the two signals that change what
        // "unreferenced" means. Value-references and dispatch are
        // language features not every language has, so requiring them
        // would bar Go and Python from the top tier forever.
        matches!(self.class, LanguageClass::Analyzable)
            && self.visibility.is_present()
            && self.attributes.is_present()
    }
}

/// Measure signal availability per language from the graph itself.
/// Measure per-language signal availability.
///
/// `declared` is what each plugin says it can extract. That is the
/// authority: whether a *particular* codebase happens to contain a
/// callback or a trait impl says nothing about the plugin's ability to
/// find one. Inferring capability from observation instead caps every
/// finding in small or simple codebases and silently empties the
/// default report — measured at 0/60 recall on a corpus where 60
/// functions were provably unreferenced.
///
/// Observation is still used, but only to *upgrade*: a plugin that
/// under-declares still gets credit for what it demonstrably produced.
pub(crate) fn measure(
    graph: &Graph,
    declared: &BTreeMap<String, LanguageSignals>,
) -> BTreeMap<String, Measured> {
    let mut vis: BTreeMap<String, bool> = BTreeMap::new();
    let mut attrs: BTreeMap<String, bool> = BTreeMap::new();
    let mut impls: BTreeMap<String, bool> = BTreeMap::new();
    let mut callables: BTreeMap<String, u32> = BTreeMap::new();
    let mut files: BTreeMap<String, u32> = BTreeMap::new();

    for f in graph.files.values() {
        *files.entry(f.language.clone()).or_insert(0) += 1;
    }
    for n in graph.callables.values() {
        if n.synthetic {
            continue;
        }
        let e = callables.entry(n.language.clone()).or_insert(0);
        *e += 1;
        if !n.visibility.is_empty() {
            vis.insert(n.language.clone(), true);
        }
        if !n.attributes.is_empty() {
            attrs.insert(n.language.clone(), true);
        }
        if n.trait_impl_target.is_some() {
            impls.insert(n.language.clone(), true);
        }
    }

    // A language "has value references" if any Reference edge actually
    // landed on one of its callables.
    let mut refs: BTreeMap<String, bool> = BTreeMap::new();
    for e in &graph.edges {
        if matches!(e.via, Via::Reference)
            && let Some(n) = graph.callables.get(&e.dst) {
                refs.insert(n.language.clone(), true);
            }
    }

    let mut out = BTreeMap::new();
    for (lang, n_callables) in callables {
        let d = declared.get(&lang).copied().unwrap_or_default();
        let yes = |m: &BTreeMap<String, bool>, declared_bit: bool| {
            if declared_bit || m.get(&lang).copied().unwrap_or(false) {
                SignalSupport::Full
            } else {
                SignalSupport::None
            }
        };
        let class = if DESCRIPTOR.contains(&lang.as_str()) {
            LanguageClass::Descriptor
        } else if SCRIPT_DRIVEN.contains(&lang.as_str()) {
            LanguageClass::ScriptDriven
        } else if (d.visibility || vis.get(&lang).copied().unwrap_or(false))
            && (d.attributes || attrs.get(&lang).copied().unwrap_or(false))
        {
            LanguageClass::Analyzable
        } else {
            LanguageClass::Degraded
        };
        let blind_spots = BLIND_SPOTS
            .iter()
            .find(|(l, _)| *l == lang)
            .map(|(_, s)| s.iter().map(|x| x.to_string()).collect())
            .unwrap_or_default();

        out.insert(
            lang.clone(),
            Measured {
                class,
                visibility: yes(&vis, d.visibility),
                attributes: yes(&attrs, d.attributes),
                value_references: yes(&refs, d.value_refs),
                dispatch: yes(&impls, d.impls),
                exports: if d.exports {
                    SignalSupport::Full
                } else {
                    SignalSupport::None
                },
                test_tagging: if d.test_defs {
                    SignalSupport::Full
                } else {
                    SignalSupport::None
                },
                files: files.get(&lang).copied().unwrap_or(0),
                callables: n_callables,
                blind_spots,
            },
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deadcode::testutil::{graph_with, node};

    #[test]
    fn a_language_with_no_signals_is_degraded() {
        let g = graph_with(vec![node(0, "pkg.f", "f", "java")]);
        let m = measure(&g, &BTreeMap::new());
        let java = &m["java"];
        assert_eq!(java.class, LanguageClass::Degraded);
        assert_eq!(java.visibility, SignalSupport::None);
        assert_eq!(java.attributes, SignalSupport::None);
        assert!(!java.signals_complete(), "must not qualify for High");
    }

    #[test]
    fn signals_are_measured_not_assumed() {
        let mut n = node(0, "crate::f", "f", "rust");
        n.visibility = "pub".into();
        n.attributes = vec!["#[inline]".into()];
        n.trait_impl_target = Some("Storage".into());
        let m = measure(&graph_with(vec![n]), &BTreeMap::new());
        let rust = &m["rust"];
        assert_eq!(rust.visibility, SignalSupport::Full);
        assert_eq!(rust.attributes, SignalSupport::Full);
        assert_eq!(rust.dispatch, SignalSupport::Full);
        assert_eq!(rust.class, LanguageClass::Analyzable);
        // Visibility + attributes are the bar; value-references and
        // dispatch are language features not every language has.
        assert!(rust.signals_complete());
    }

    #[test]
    fn descriptor_languages_are_classified_apart() {
        let g = graph_with(vec![node(0, "Shape", "Shape", "openapi")]);
        assert_eq!(
            measure(&g, &BTreeMap::new())["openapi"].class,
            LanguageClass::Descriptor
        );
    }

    #[test]
    fn script_languages_are_classified_apart() {
        let g = graph_with(vec![node(0, "f", "f", "bash")]);
        assert_eq!(
            measure(&g, &BTreeMap::new())["bash"].class,
            LanguageClass::ScriptDriven
        );
    }

    #[test]
    fn synthetic_nodes_do_not_count_toward_coverage() {
        let mut n = node(0, "ext::x", "x", "rust");
        n.synthetic = true;
        n.visibility = "pub".into();
        assert!(measure(&graph_with(vec![n]), &BTreeMap::new()).is_empty());
    }

    #[test]
    fn rust_blind_spots_are_named() {
        let g = graph_with(vec![node(0, "crate::f", "f", "rust")]);
        let spots = &measure(&g, &BTreeMap::new())["rust"].blind_spots;
        assert!(spots.iter().any(|s| s.contains("macro arguments")));
    }

    #[test]
    fn declared_capability_beats_what_this_run_happened_to_observe() {
        // Regression: capability used to be inferred purely from
        // observation, so a codebase with no callbacks and no trait
        // impls made cgg conclude the *language* could not express
        // them. That capped every finding to medium and emptied the
        // default report — measured at 0/60 recall on a corpus where 60
        // functions were provably unreferenced.
        let g = graph_with(vec![node(0, "crate::f", "f", "rust")]);

        // Observation alone: the node carries no visibility or
        // attributes, so nothing is measurable.
        let observed = measure(&g, &BTreeMap::new());
        assert_eq!(observed["rust"].class, LanguageClass::Degraded);
        assert!(!observed["rust"].signals_complete());

        // With the plugin's declaration, the same graph is analyzable.
        let mut declared = BTreeMap::new();
        declared.insert(
            "rust".to_string(),
            LanguageSignals {
                visibility: true,
                attributes: true,
                value_refs: true,
                impls: true,
                ..Default::default()
            },
        );
        let d = measure(&g, &declared);
        assert_eq!(d["rust"].class, LanguageClass::Analyzable);
        assert!(
            d["rust"].signals_complete(),
            "declaration must be the authority"
        );
        assert_eq!(d["rust"].value_references, SignalSupport::Full);
    }

    #[test]
    fn observation_can_still_upgrade_an_under_declaring_plugin() {
        let mut n = node(0, "crate::f", "f", "rust");
        n.visibility = "pub".into();
        n.attributes = vec!["#[inline]".into()];
        let m = measure(&graph_with(vec![n]), &BTreeMap::new());
        assert_eq!(m["rust"].visibility, SignalSupport::Full);
        assert_eq!(m["rust"].attributes, SignalSupport::Full);
    }
}
