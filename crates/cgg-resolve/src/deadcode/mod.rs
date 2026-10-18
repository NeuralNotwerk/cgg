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

/// The adjacency liveness is proved over: the edges that
/// [`counts_for_liveness`], minus those whose call site control flow
/// cannot reach (a call after `return` never runs, so it keeps nothing
/// alive).
fn liveness_adjacency(graph: &Graph, facts: &[FileFacts]) -> Adj {
    let unreachable = crate::unreachable::UnreachableIndex::build(facts);
    Adj::build(graph, &|e| {
        counts_for_liveness(e) && !unreachable.covers(graph, e)
    })
}

/// Callee -> how many of its call sites lie in unreachable code (edges
/// [`liveness_adjacency`] leaves out).
fn unreachable_sites(
    graph: &Graph,
    facts: &[FileFacts],
) -> std::collections::HashMap<CallableId, u32> {
    let unreachable = crate::unreachable::UnreachableIndex::build(facts);
    let mut out = std::collections::HashMap::new();
    if unreachable.is_empty() {
        return out;
    }
    for e in &graph.edges {
        if counts_for_liveness(e) && unreachable.covers(graph, e) {
            *out.entry(e.dst).or_insert(0u32) += 1;
        }
    }
    out
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

/// The plugin's finer definition variant, keyed the way a node is
/// located. Empty when the graph was read back without facts.
fn def_variants(
    facts: &[FileFacts],
) -> std::collections::HashMap<(cgg_core::ids::FileId, u32), cgg_core::DefVariant> {
    facts
        .iter()
        .flat_map(|f| {
            f.definitions
                .iter()
                .map(move |d| ((f.file, d.start_byte), d.variant))
        })
        .collect()
}

/// Where the source names a callable without cgg drawing an edge:
/// [`cgg_core::NameMention`]s, and same-file value references
/// (`x.map(to_ms)`) the resolver declined to bind — it never binds a
/// named closure, because an edge into one would manufacture a call.
///
/// Resolved relative to the naming file, never by simple name across the
/// workspace: `default_true` is defined once per module that needs it, and
/// one module's attribute must not vouch for another module's dead twin.
/// A bare name matches in its own file, or through a `use` in the naming
/// file that imports it; a path matches by qualified-name suffix, so
/// `Vec::is_empty` never reaches a workspace `MultiIndex::is_empty`.
#[derive(Default)]
struct Mentions<'a> {
    /// (file, bare name) -> screens, one per site
    bare: std::collections::HashMap<(cgg_core::ids::FileId, &'a str), Vec<&'static str>>,
    /// bare name -> (naming file, screen), for the `use` route
    files_naming:
        std::collections::HashMap<&'a str, Vec<(cgg_core::ids::FileId, &'static str)>>,
    /// last path segment -> (path, screen)
    paths: std::collections::HashMap<&'a str, Vec<(&'a str, &'static str)>>,
    /// file -> its imports
    imports:
        std::collections::HashMap<cgg_core::ids::FileId, &'a [cgg_core::ImportRecord]>,
}

impl<'a> Mentions<'a> {
    fn build(facts: &'a [FileFacts]) -> Self {
        let mut m = Mentions::default();
        for f in facts {
            m.imports.insert(f.file, &f.imports);
            let values = f
                .references
                .iter()
                .filter(|r| r.receiver_hint == cgg_core::VALUE_REF_HINT)
                .map(|r| (r.name.as_str(), "value-ref"));
            let named = f.name_mentions.iter().map(|n| {
                let screen = if n.via.starts_with("attr:") {
                    "attribute-string"
                } else {
                    "value-position"
                };
                (n.name.as_str(), screen)
            });
            for (name, screen) in values.chain(named) {
                let path = name
                    .trim_start_matches("crate::")
                    .trim_start_matches("self::")
                    .trim_start_matches("super::");
                if let Some((_, last)) = path.rsplit_once("::") {
                    m.paths.entry(last).or_default().push((path, screen));
                } else {
                    m.bare.entry((f.file, path)).or_default().push(screen);
                    m.files_naming
                        .entry(path)
                        .or_default()
                        .push((f.file, screen));
                }
            }
        }
        m
    }

    fn evidence_for(&self, nd: &CallableNode) -> Vec<Evidence> {
        let name = nd.simple_name.as_str();
        let mut hits: BTreeMap<&'static str, u32> = BTreeMap::new();
        // A plain value reference names the callable as a value from this
        // file (or one importing it). The resolver binds a unique target
        // as a reference edge, so a finding that still reaches here was
        // named where no edge could be drawn: a named closure it refuses
        // to bind (`x.map(to_ms)`), or a function or method that names
        // itself as a value (`signal.connect(handler)` inside `handler`),
        // a self-edge that proves nothing. Either way the source passes
        // it somewhere cgg cannot follow: doubt, for named functions and
        // methods as well as closures (WS6).
        let hit = |screen: &'static str, hits: &mut BTreeMap<&'static str, u32>| {
            let screen = match screen {
                "value-ref" => "value-position",
                s => s,
            };
            *hits.entry(screen).or_insert(0) += 1;
        };
        for s in self.bare.get(&(nd.file, name)).into_iter().flatten() {
            hit(s, &mut hits);
        }
        // Named in another file that imports it: `use a::b::name;` or
        // `use a::b::{x, name};`.
        let suffix = format!("::{name}");
        for (file, screen) in self.files_naming.get(name).into_iter().flatten() {
            if *file == nd.file || !nd.qualified_name.ends_with(&suffix) {
                continue;
            }
            let imported = self.imports.get(file).is_some_and(|imps| {
                imps.iter().any(|i| {
                    i.path
                        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                        .any(|w| w == name)
                })
            });
            if imported {
                hit(screen, &mut hits);
            }
        }
        for (path, screen) in self.paths.get(name).into_iter().flatten() {
            if nd.qualified_name == *path
                || nd.qualified_name.ends_with(&format!("::{path}"))
            {
                hit(screen, &mut hits);
            }
        }
        hits.into_iter()
            .map(|(screen, sites)| Evidence::NameCollidesWithScreenedSite {
                screen: screen.into(),
                sites,
            })
            .collect()
    }
}

/// Which trait declarations the workspace implements, and which of their
/// implementations something calls directly. Built once, sorted where it
/// is iterated, so `--jobs` cannot change an answer.
struct TraitSurfaces<'a> {
    /// (language, trait, method) -> implementations of that method.
    impls: std::collections::HashMap<(&'a str, &'a str, &'a str), Vec<CallableId>>,
    /// (language, trait) -> number of implementing methods.
    implemented: std::collections::HashMap<(&'a str, &'a str), u32>,
    /// Callables with at least one incoming edge that is not the
    /// declaration -> impl fan-out itself.
    called: std::collections::HashSet<CallableId>,
    /// (file, simple name) -> callables, for files declaring a bodiless
    /// (`abstract`-tagged) member: the bodies an enum constant or an
    /// anonymous class in that file gives the member.
    same_file_named:
        std::collections::HashMap<(cgg_core::ids::FileId, &'a str), Vec<CallableId>>,
}

impl<'a> TraitSurfaces<'a> {
    fn build(graph: &'a Graph) -> Self {
        let mut impls: std::collections::HashMap<(&str, &str, &str), Vec<CallableId>> =
            std::collections::HashMap::new();
        let mut implemented: std::collections::HashMap<(&str, &str), u32> =
            std::collections::HashMap::new();
        for c in graph.callables.values() {
            if let Some(t) = c.trait_impl_target.as_deref() {
                impls
                    .entry((c.language.as_str(), t, c.simple_name.as_str()))
                    .or_default()
                    .push(c.id);
                *implemented.entry((c.language.as_str(), t)).or_insert(0) += 1;
            }
        }
        let called = graph
            .edges
            .iter()
            .filter(|e| !matches!(e.via, Via::Dynamic))
            .map(|e| e.dst)
            .collect();
        let abstract_files: std::collections::HashSet<cgg_core::ids::FileId> = graph
            .callables
            .values()
            .filter(|c| c.attributes.iter().any(|a| a == ABSTRACT_MARKER))
            .map(|c| c.file)
            .collect();
        let mut same_file_named: std::collections::HashMap<
            (cgg_core::ids::FileId, &str),
            Vec<CallableId>,
        > = std::collections::HashMap::new();
        if !abstract_files.is_empty() {
            for c in graph.callables.values() {
                if abstract_files.contains(&c.file) {
                    same_file_named
                        .entry((c.file, c.simple_name.as_str()))
                        .or_default()
                        .push(c.id);
                }
            }
        }
        TraitSurfaces {
            impls,
            implemented,
            called,
            same_file_named,
        }
    }

    /// [`Evidence::ImplementedTraitSurface`] for `nd` when it is a method
    /// declared on a trait the workspace implements, and something shows
    /// that method being dispatched to. Requires the declaration itself
    /// in the graph (an inherent method or an impl never qualifies), so
    /// a bare-name collision with an external trait cannot trigger it.
    fn evidence_for(
        &self,
        nd: &CallableNode,
        variant: Option<&cgg_core::DefVariant>,
        unresolved: &UnresolvedIndex<'_>,
        contracts: &Contracts<'_>,
    ) -> Option<Evidence> {
        if nd.trait_impl_target.is_some() || nd.qualified_name.contains(" as ") {
            return None;
        }
        // A bodiless declaration whose bodies an enum constant or an
        // anonymous class of its own file provides (`abstract boolean f();`
        // beside `CONST { boolean f() { … } }`): calls bind to those bodies,
        // so the declaration's own in-degree of zero says nothing once one
        // of them is called.
        if nd.attributes.iter().any(|a| a == ABSTRACT_MARKER)
            && let Some(named) = self
                .same_file_named
                .get(&(nd.file, nd.simple_name.as_str()))
        {
            let bodies: Vec<CallableId> =
                named.iter().copied().filter(|c| *c != nd.id).collect();
            let called_impls =
                bodies.iter().filter(|c| self.called.contains(c)).count() as u32;
            let unresolved_sites = unresolved.sites_named(&nd.language, &nd.simple_name);
            if !bodies.is_empty() && (called_impls > 0 || unresolved_sites > 0) {
                return Some(Evidence::ImplementedTraitSurface {
                    trait_name: owner_stem(&nd.qualified_name).unwrap_or("").to_string(),
                    impls: bodies.len() as u32,
                    unresolved_sites,
                    called_impls,
                });
            }
        }
        // A member of a class that classes below it override: the same
        // dispatch surface, read off the declared bases. Callers typed to
        // a subclass bind to its override, so the base member's own
        // in-degree of zero says nothing once an override is called.
        if crate::dispatch::INHERITANCE_LANGS.contains(&nd.language.as_str()) {
            let lang = nd.language.as_str();
            if matches!(
                variant,
                Some(
                    cgg_core::DefVariant::Constructor
                        | cgg_core::DefVariant::StaticMethod
                )
            ) || (matches!(variant, Some(cgg_core::DefVariant::FreeFunction))
                && !crate::dispatch::free_variant_is_member(lang))
            {
                return None;
            }
            let owner = owner_stem(&nd.qualified_name)?;
            let below = contracts.overrides_below(lang, owner, &nd.simple_name);
            if below.is_empty() {
                return None;
            }
            let called_impls =
                below.iter().filter(|c| self.called.contains(c)).count() as u32;
            let unresolved_sites = unresolved.sites_named(lang, &nd.simple_name);
            if called_impls == 0 && unresolved_sites == 0 {
                return None;
            }
            return Some(Evidence::ImplementedTraitSurface {
                trait_name: owner.to_string(),
                impls: below.len() as u32,
                unresolved_sites,
                called_impls,
            });
        }
        if matches!(
            variant,
            Some(
                cgg_core::DefVariant::InherentMethod | cgg_core::DefVariant::FreeFunction
            )
        ) {
            return None;
        }
        let owner = crate::names::owner_from_qn(&nd.qualified_name)?;
        let lang = nd.language.as_str();
        let impls = *self.implemented.get(&(lang, owner))?;
        let method_impls = self
            .impls
            .get(&(lang, owner, nd.simple_name.as_str()))
            .map_or(&[][..], |v| v.as_slice());
        let called_impls = method_impls
            .iter()
            .filter(|c| self.called.contains(c))
            .count() as u32;
        let unresolved_sites = unresolved.sites_named(lang, &nd.simple_name);
        if called_impls == 0 && unresolved_sites == 0 {
            return None;
        }
        Some(Evidence::ImplementedTraitSurface {
            trait_name: owner.to_string(),
            impls,
            unresolved_sites,
            called_impls,
        })
    }
}

/// The marker plugins put on a bodiless interface or abstract member.
const ABSTRACT_MARKER: &str = "abstract";

/// Markers a plugin records on a member that implements one its class's
/// base declares: Java's `@Override`, the `override` modifier (C#, and
/// the plugin's spelling of a C# explicit interface implementation),
/// Dart's `@override`, PHP 8.3's `#[\Override]` (read through
/// [`override_marker_text`]). Language keywords and annotations, never
/// names.
const OVERRIDE_MARKERS: &[&str] = &["@Override", "override", "@override", "Override"];

/// An attribute as [`OVERRIDE_MARKERS`] spells it: PHP's attribute-group
/// brackets (`#[…]`) and a leading namespace separator (`\Override`)
/// stripped. Other spellings pass through trimmed.
fn override_marker_text(a: &str) -> &str {
    let a = a.trim();
    let a = a
        .strip_prefix("#[")
        .and_then(|x| x.strip_suffix(']'))
        .unwrap_or(a)
        .trim();
    a.strip_prefix('\\').unwrap_or(a)
}

/// The class hierarchy as dead-code needs it: what a member overrides,
/// what overrides it, and whether a contract it implements lies outside
/// the analyzed tree. Built once from the facts; every list it iterates
/// is sorted, so `--jobs` cannot change an answer.
struct Contracts<'a> {
    hierarchy: crate::dispatch::ClassHierarchy,
    /// (language, base) -> classes naming it as a base.
    derived: std::collections::HashMap<(String, String), Vec<String>>,
    /// (language, bare owner, method) -> callables.
    members: std::collections::HashMap<(&'a str, &'a str, &'a str), Vec<CallableId>>,
    /// (language, bare owner) of every declared callable — the types the
    /// graph knows, including Rust traits by their declarations.
    owners: std::collections::HashSet<(&'a str, &'a str)>,
}

impl<'a> Contracts<'a> {
    fn build(graph: &'a Graph, facts: &[FileFacts]) -> Self {
        let mut members: std::collections::HashMap<(&str, &str, &str), Vec<CallableId>> =
            std::collections::HashMap::new();
        let mut owners: std::collections::HashSet<(&str, &str)> =
            std::collections::HashSet::new();
        for c in graph.callables.values() {
            if c.synthetic || c.trait_impl_target.is_some() {
                // An impl's owner is the implementing type, which says
                // nothing about which traits the tree declares.
                if let (false, Some(o)) = (c.synthetic, owner_stem(&c.qualified_name)) {
                    members
                        .entry((c.language.as_str(), o, c.simple_name.as_str()))
                        .or_default()
                        .push(c.id);
                }
                continue;
            }
            if let Some(o) = owner_stem(&c.qualified_name) {
                members
                    .entry((c.language.as_str(), o, c.simple_name.as_str()))
                    .or_default()
                    .push(c.id);
                owners.insert((c.language.as_str(), o));
            }
        }
        let hierarchy = crate::dispatch::ClassHierarchy::build(facts);
        let derived = hierarchy.derived_index();
        Contracts {
            hierarchy,
            derived,
            members,
            owners,
        }
    }

    fn declares_member(&self, lang: &str, owner: &str, name: &str) -> bool {
        self.members
            .get(&(lang, owner, name))
            .is_some_and(|v| !v.is_empty())
    }

    /// [`Evidence::ImplementsExternalContract`] when `nd` implements a
    /// member whose declaration is outside the tree.
    fn external_contract(&self, nd: &CallableNode) -> Option<Evidence> {
        let lang = nd.language.as_str();
        let name = nd.simple_name.as_str();
        // Rust names the trait on the impl; a trait the tree declares
        // has at least one method declaration of its own.
        if lang == "rust" {
            let t = nd.trait_impl_target.as_deref()?;
            let t = crate::dispatch::type_stem(t);
            if self.owners.contains(&(lang, t)) {
                return None;
            }
            return Some(Evidence::ImplementsExternalContract {
                contract: t.to_string(),
            });
        }
        // A partial method part: its other half and the callers it
        // stands for are routinely generated code or outside the tree.
        if nd
            .attributes
            .iter()
            .any(|a| a == cgg_core::PARTIAL_DECLARATION)
        {
            return Some(Evidence::ImplementsExternalContract {
                contract: "partial".to_string(),
            });
        }
        let owner = owner_stem(&nd.qualified_name)?;
        let marked = nd
            .attributes
            .iter()
            .any(|a| OVERRIDE_MARKERS.contains(&override_marker_text(a)));
        // Python marks nothing: any method of a class deriving from an
        // outside base may be the hook that base calls. A name-mangled
        // `__m` cannot be: the base would have to spell this class's name.
        // A method is a definition whose owner is a class, whatever its
        // kind: `async def` methods are recorded with kind `function`.
        let mangled = name.starts_with("__") && !name.ends_with("__");
        let python = lang == "python" && !mangled && self.hierarchy.declares(lang, owner);
        if !marked && !python {
            return None;
        }
        let ancestors = self.hierarchy.ancestors(lang, owner);
        if ancestors
            .iter()
            .any(|a| self.declares_member(lang, a, name))
        {
            // The contract is in the tree: the dispatch fan-out from its
            // declaration is what reaches this override.
            return None;
        }
        let outside = ancestors
            .iter()
            .find(|a| !self.hierarchy.declares(lang, a))
            .cloned();
        match outside {
            Some(a) => Some(Evidence::ImplementsExternalContract { contract: a }),
            // A marker with no outside ancestor in view: the base it
            // names is still outside (an implicit root type, or a base
            // cgg could not read off the declaration).
            None if marked => Some(Evidence::ImplementsExternalContract {
                contract: String::new(),
            }),
            None => None,
        }
    }

    /// The same-named members of the classes below `owner`, transitively
    /// (depth 8), sorted.
    fn overrides_below(&self, lang: &str, owner: &str, name: &str) -> Vec<CallableId> {
        let mut out: Vec<CallableId> = Vec::new();
        let mut seen: std::collections::BTreeSet<&str> =
            std::collections::BTreeSet::new();
        seen.insert(owner);
        let mut frontier: Vec<&str> = vec![owner];
        for _ in 0..8 {
            let mut next: Vec<&str> = Vec::new();
            for t in frontier {
                let Some(kids) = self.derived.get(&(lang.to_string(), t.to_string()))
                else {
                    continue;
                };
                for k in kids {
                    if !seen.insert(k.as_str()) {
                        continue;
                    }
                    if let Some(v) = self.members.get(&(lang, k.as_str(), name)) {
                        out.extend(v.iter().copied());
                    }
                    next.push(k.as_str());
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Whether a receiver hint on an unresolved same-name site rules
    /// this candidate out: only a type the tree declares, unrelated to
    /// the candidate's owner by inheritance either way. A variable
    /// (`p`, `orderService`), a type parameter (`T`), an expression or a
    /// type cgg never saw says nothing about which class receives it.
    /// `stem` is the receiver hint already reduced to a type-shaped name.
    fn hint_contradicts(&self, lang: &str, owner: &str, stem: &str) -> bool {
        if stem == owner || !self.owners.contains(&(lang, stem)) {
            return false;
        }
        let up = self.hierarchy.ancestors(lang, owner);
        if up.iter().any(|a| a == stem) {
            return false;
        }
        !self
            .hierarchy
            .ancestors(lang, stem)
            .iter()
            .any(|a| a == owner)
    }
}

/// The bare owner segment of a qualified name (`Box<T>` -> `Box`).
fn owner_stem(qn: &str) -> Option<&str> {
    crate::names::owner_from_qn(qn).map(crate::dispatch::type_stem)
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
    let adjacency = liveness_adjacency(graph, facts);
    let unreachable_sites = unreachable_sites(graph, facts);
    let n = adjacency.n as usize;
    let measured = caps::measure(graph, &opts.language_signals);
    let unresolved = UnresolvedIndex::build(graph, file_audits);
    let root_set = roots::discover(graph, facts, &opts.user_roots, &opts.framework_roots);

    // Identifier-shaped literals in reflective positions. Never an edge
    // — only a reason to doubt a finding. A member lookup by a literal
    // name (`getattr(o, "m")`, `getDeclaredMethod("m")`, `send(:m)`)
    // caps the finding at medium; a module load (`require("x")`) names a
    // file, not a callable, and is reported without a cap.
    let mut dyn_named: std::collections::HashMap<(&str, &str), &'static str> =
        std::collections::HashMap::new();
    for f in facts {
        for d in &f.dyn_uses {
            let screen = if cgg_core::DYN_MODULE_LOADERS.contains(&d.via.as_str()) {
                "module-load"
            } else {
                "reflection"
            };
            let slot = dyn_named
                .entry((f.language.as_str(), d.name.as_str()))
                .or_insert(screen);
            if screen == "reflection" {
                *slot = screen;
            }
        }
    }

    let variants = def_variants(facts);

    let surfaces = TraitSurfaces::build(graph);
    let contracts = {
        let _s = cgg_core::profile::span("dead::contracts-build");
        Contracts::build(graph, facts)
    };
    let mentions = Mentions::build(facts);
    // Java: the package each file declares.
    let package_of: std::collections::HashMap<cgg_core::ids::FileId, &str> = facts
        .iter()
        .filter(|f| f.language == "java")
        .filter_map(|f| f.packages.first().map(|p| (f.file, p.as_str())))
        .collect();
    // Whether an unresolved site could legally call `nd` at all.
    let can_reach = |nd: &CallableNode,
                     nd_owner: Option<&str>,
                     nd_package: Option<&str>,
                     u: &cgg_core::audit::AuditUnresolvedCall|
     -> bool {
        match (nd.language.as_str(), nd.vis) {
            // Java `private`: the top-level class, which is the file.
            ("java", Vis::Private) => u.file == nd.file,
            // Package-private: the package.
            ("java", Vis::Internal) => match (package_of.get(&u.file), nd_package) {
                (Some(a), Some(b)) => *a == b,
                _ => true,
            },
            // C# `private`: the class, which a `partial` class spreads
            // over several files.
            ("csharp", Vis::Private) => {
                u.file == nd.file
                    || u.src
                        .and_then(|c| graph.callables.get(&c))
                        .and_then(|c| owner_stem(&c.qualified_name))
                        .is_some_and(|o| Some(o) == nd_owner)
            }
            _ => true,
        }
    };

    let node_at = |i: u32| graph.callables.get_index(i as usize).map(|(_, v)| v);
    let lang_at = |i: u32| node_at(i).map(|nd| nd.language.clone()).unwrap_or_default();

    // --- Liveness: production roots, then production + test ----------------
    let live_prod = adjacency.reachable_from(&root_set.production);
    let mut all_roots = root_set.production.clone();
    all_roots.extend_from_slice(&root_set.test);
    all_roots.sort_unstable();
    let live_any = adjacency.reachable_from(&all_roots);
    // Root coverage is measured from production roots and the tests the
    // built-in rules recognise — not from every test a framework harness
    // enters (see `RootSet::coverage_test`).
    let live_cov = {
        let mut seeds = root_set.production.clone();
        seeds.extend(&root_set.coverage_test);
        seeds.sort_unstable();
        seeds.dedup();
        adjacency.reachable_from(&seeds)
    };

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
        if live_cov[i as usize] {
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
            match unreachable_sites.get(&nd.id) {
                Some(&sites) => {
                    ev.push(Evidence::IncomingOnlyFromUnreachableCode { sites })
                }
                None => ev.push(Evidence::NoIncomingEdges),
            }
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
        let variant = variants.get(&(nd.file, nd.start_byte));
        let free = (matches!(variant, Some(cgg_core::DefVariant::FreeFunction))
            && !crate::dispatch::free_variant_is_member(&nd.language))
            || owner_stem(&nd.qualified_name).is_none();
        let _s_unres = cgg_core::profile::span("dead::unresolved-evidence");
        let nd_owner = owner_stem(&nd.qualified_name);
        let nd_package = package_of.get(&nd.file).copied();
        ev.extend(unresolved.evidence_for(
            graph,
            nd,
            &|hint_type: Option<&str>| {
                // A receiver can only reach a member. For a free function a
                // hint naming anything but its own module rules it out, as
                // before; for a member only a declared, unrelated type does
                // — a variable or expression says nothing.
                match (nd_owner, hint_type) {
                    (Some(_), None) if !free => false,
                    (Some(o), Some(t)) if !free => {
                        contracts.hint_contradicts(&nd.language, o, t)
                    }
                    _ => true,
                }
            },
            &|u| can_reach(nd, nd_owner, nd_package, u),
        ));
        drop(_s_unres);
        let _s_disp = cgg_core::profile::span("dead::dispatch-evidence");
        if let Some(e) = surfaces.evidence_for(nd, variant, &unresolved, &contracts) {
            ev.push(e);
        }
        if let Some(e) = contracts.external_contract(nd) {
            ev.push(e);
        }
        ev.extend(mentions.evidence_for(nd));
        if let Some(m) = nd.attributes.iter().find_map(|a| a.strip_prefix("macro:")) {
            ev.push(Evidence::MacroGenerated {
                macro_name: m.to_string(),
            });
        }
        if let Some(screen) =
            dyn_named.get(&(nd.language.as_str(), nd.simple_name.as_str()))
        {
            ev.push(Evidence::NameCollidesWithScreenedSite {
                screen: (*screen).into(),
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
            // `Protected` is in the same position: any subtype may call
            // it, a subtype may be declared outside the analyzed tree, and
            // cgg models no subtype graph that could rule one out. Saying
            // nothing would not be neutral — a never-referenced finding
            // *starts* at High, so silence left a protected member in the
            // top band exactly like a private one. The token keeps
            // `protected` visible to a reader.
            Vis::Public | Vis::Protected => ev.push(Evidence::PublicVisibility {
                token: vis_token(nd),
            }),
            // The plugin never determined it. Claiming either direction
            // would be a guess, which is why `Vis::escapes_unit` — true
            // for `Unknown` — is deliberately not used here; a language
            // that determines none is capped by `LanguageLacksVisibility`.
            Vis::Unknown => {}
        }

        ev.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
        // One entry per kind — except screened-site evidence, where each
        // screen is a different reason (and `external` carries no cap
        // while `value-position` does, so collapsing them lost the cap).
        ev.dedup_by(|a, b| {
            a.slug() == b.slug()
                && match (a, b) {
                    (
                        Evidence::NameCollidesWithScreenedSite { screen: x, .. },
                        Evidence::NameCollidesWithScreenedSite { screen: y, .. },
                    ) => x == y,
                    _ => true,
                }
        });

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
            def_variant: variants
                .get(&(nd.file, nd.start_byte))
                .map(|v| v.as_str().to_string())
                .unwrap_or_default(),
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
    // A region's confidence is the weakest of its members' findings. Look
    // findings up by id once rather than scanning the list per member:
    // the scan was O(dead members × findings), which on a tree with
    // ~300k dead callables (one big region) ran for minutes. The first
    // finding for an id wins, exactly as the linear `find` did.
    let finding_confidence: std::collections::HashMap<CallableId, Confidence> = {
        let mut m = std::collections::HashMap::with_capacity(findings.len());
        for f in &findings {
            m.entry(f.id).or_insert(f.confidence);
        }
        m
    };
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
            // Discovery order (IndexMap position), not hash order.
            files.sort_by_cached_key(|f| graph.files.get_index_of(f));
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
                .filter_map(|nd| finding_confidence.get(&nd.id).copied())
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
    let adjacency = liveness_adjacency(graph, facts);
    let root_set = roots::discover(graph, facts, &opts.user_roots, &opts.framework_roots);

    let mut all_roots = root_set.production.clone();
    all_roots.extend_from_slice(&root_set.test);
    all_roots.sort_unstable();

    // Production roots first, so a proof from `main` always wins over a
    // proof from a test.
    let prod = search(&adjacency, graph, &root_set.production);
    let any = search(&adjacency, graph, &all_roots);
    let surfaces = TraitSurfaces::build(graph);
    let contracts = Contracts::build(graph, facts);
    let unresolved = UnresolvedIndex::build(graph, &[]);
    let variants = def_variants(facts);

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
                        Via::Descriptor(f) => format!("descriptor:{f}"),
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

        let dispatch_surface = (status == "dead")
            .then(|| {
                surfaces.evidence_for(
                    node,
                    variants.get(&(node.file, node.start_byte)),
                    &unresolved,
                    &contracts,
                )
            })
            .flatten();
        out.push(LivenessProof {
            target,
            target_qualified_name: node.qualified_name.clone(),
            status: status.to_string(),
            root: root_record,
            hops,
            weakest_link,
            dispatch_surface,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::audit::{AuditUnresolvedCall, UnresolvedReason};
    use testutil::{edge, graph_with, link, node};

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
    fn a_region_takes_the_weakest_confidence_of_its_members() {
        // `a` (private) -> `b` (public): one region, two findings whose
        // confidences differ. The region reports the weaker of the two.
        let mut g = graph_with(vec![
            node(0, "crate::main", "main", "rust"),
            vis_node(1, "crate::a", "a", "", Vis::Private),
            vis_node(2, "crate::b", "b", "pub", Vis::Public),
        ]);
        link(&mut g, 1, 2);
        let r = analyze(&g, &[], &[], &opts_rust_full());
        assert_eq!(r.regions.len(), 1);
        let band = |c: Confidence| match c {
            Confidence::Low => 0,
            Confidence::Medium => 1,
            Confidence::High => 2,
        };
        let confs: Vec<Confidence> = r.findings.iter().map(|f| f.confidence).collect();
        assert_eq!(confs.len(), 2);
        assert_ne!(confs[0], confs[1], "the test needs differing confidences");
        let weakest = confs.iter().copied().min_by_key(|c| band(*c)).unwrap();
        assert_eq!(r.regions[0].confidence, weakest);
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

    /// `trait Store { fn put(); }` + `impl Store for Disk`, with the
    /// dispatch fan-out edge `--dead-code` adds. `main` calls nothing.
    fn trait_surface_graph() -> Graph {
        let mut decl = vis_node(1, "crate::Store::put", "put", "", Vis::Private);
        decl.kind = CallableKind::Method;
        let mut imp = vis_node(2, "crate::<Disk as Store>::put", "put", "", Vis::Private);
        imp.trait_impl_target = Some("Store".into());
        let mut g = graph_with(vec![node(0, "crate::main", "main", "rust"), decl, imp]);
        let mut e = testutil::edge(1, 2);
        e.via = Via::Dynamic;
        g.edges.push(e);
        g
    }

    fn decl_finding(r: &DeadCodeReport) -> &DeadCodeFinding {
        r.findings
            .iter()
            .find(|f| f.qualified_name == "crate::Store::put")
            .expect("the declaration is reported")
    }

    // F2: a trait declaration is the dispatch surface. Its callers bind
    // through `dyn Store`, which the graph records as edges into the
    // impls, so its own in-degree of zero is not evidence of anything —
    // once something shows the method being dispatched to.
    #[test]
    fn an_implemented_trait_declaration_with_dispatch_evidence_is_capped() {
        // Holding a trait object is not calling it: no site, no direct
        // call into an impl -> the declaration is genuinely unreferenced.
        let g = trait_surface_graph();
        let r = analyze(&g, &[], &[], &opts_rust_full());
        let f = decl_finding(&r);
        assert_eq!(f.confidence, Confidence::High, "{:?}", f.evidence);
        assert!(
            !f.evidence
                .iter()
                .any(|e| e.slug() == "implemented-trait-surface")
        );

        // A same-name call the resolver could not place.
        let mut g = trait_surface_graph();
        g.unresolved.push(cgg_core::audit::AuditUnresolvedCall::new(
            Some(CallableId::new(0)),
            cgg_core::ids::FileId::new(0),
            3,
            30,
            "put".into(),
            "store".into(),
            cgg_core::audit::UnresolvedReason::NoCandidateInFile,
        ));
        let r = analyze(&g, &[], &[], &opts_rust_full());
        let f = decl_finding(&r);
        // The site's receiver is a variable, which says nothing about
        // its type: the site itself is doubt too (same file: low).
        assert_ne!(f.confidence, Confidence::High, "{:?}", f.evidence);
        assert!(
            f.evidence
                .iter()
                .any(|e| e.slug() == "implemented-trait-surface")
        );

        // An ordinary call into an implementation.
        let mut g = trait_surface_graph();
        link(&mut g, 0, 2);
        let r = analyze(&g, &[], &[], &opts_rust_full());
        assert_eq!(decl_finding(&r).confidence, Confidence::Medium);
    }

    // WS22: a bodiless declaration whose bodies an enum constant or an
    // anonymous class of the same file provides is capped once one of
    // those bodies is called; with none called it stays `high`.
    #[test]
    fn an_abstract_declaration_with_a_called_same_file_body_is_capped() {
        let build = |called: bool| {
            let mut decl = vis_node(1, "crate::E::f", "f", "", Vis::Private);
            decl.kind = CallableKind::Method;
            decl.attributes = vec!["abstract".into()];
            let mut body = vis_node(2, "crate::E::f", "f", "", Vis::Private);
            body.kind = CallableKind::Method;
            let mut g =
                graph_with(vec![node(0, "crate::main", "main", "rust"), decl, body]);
            if called {
                link(&mut g, 0, 2);
            }
            analyze(&g, &[], &[], &opts_rust_full())
        };
        let r = build(true);
        let f = r
            .findings
            .iter()
            .find(|f| f.qualified_name == "crate::E::f")
            .expect("the declaration is reported");
        assert_eq!(f.confidence, Confidence::Medium, "{:?}", f.evidence);
        assert!(
            f.evidence
                .iter()
                .any(|e| e.slug() == "implemented-trait-surface")
        );
        // Near miss: no body is called, so nothing doubts the finding.
        let r = build(false);
        assert!(
            r.findings
                .iter()
                .filter(|f| f.qualified_name == "crate::E::f")
                .all(|f| !f
                    .evidence
                    .iter()
                    .any(|e| e.slug() == "implemented-trait-surface"))
        );
    }

    #[test]
    fn why_live_names_the_dispatch_surface_the_report_doubts() {
        let mut g = trait_surface_graph();
        link(&mut g, 0, 2);
        let p = why_live(&g, &[], &opts_rust_full(), &[CallableId::new(1)]);
        assert_eq!(p[0].status, "dead");
        assert_eq!(
            p[0].dispatch_surface.as_ref().map(|e| e.slug()),
            Some("implemented-trait-surface")
        );
        // No dispatch evidence: a plain dead proof, nothing added.
        let p = why_live(
            &trait_surface_graph(),
            &[],
            &opts_rust_full(),
            &[CallableId::new(1)],
        );
        assert!(p[0].dispatch_surface.is_none());
    }

    // F5: an attribute string names a helper relative to its own file —
    // one module's `default = "default_true"` must not vouch for another
    // module's dead `default_true`, and a path (`Vec::is_empty`) matches
    // by qualified-name suffix, never by simple name.
    #[test]
    fn attribute_string_mentions_cap_only_what_they_name() {
        let mut other = vis_node(
            2,
            "crate::b::default_true",
            "default_true",
            "",
            Vis::Private,
        );
        other.file = cgg_core::ids::FileId::new(1);
        let g = {
            let mut g = graph_with(vec![
                node(0, "crate::main", "main", "rust"),
                vis_node(
                    1,
                    "crate::a::default_true",
                    "default_true",
                    "",
                    Vis::Private,
                ),
                other,
                vis_node(
                    3,
                    "crate::MultiIndex::is_empty",
                    "is_empty",
                    "",
                    Vis::Private,
                ),
            ]);
            g.add_file(cgg_core::graph::FileRecord {
                id: cgg_core::ids::FileId::new(1),
                path: "b.rs".into(),
                language: "rust".into(),
                ..Default::default()
            });
            g
        };
        let mut fa = FileFacts::new(cgg_core::ids::FileId::new(0), "a.rs".into(), "rust");
        for (name, via) in [
            ("default_true", "attr:default"),
            ("Vec::is_empty", "attr:skip_serializing_if"),
        ] {
            fa.name_mentions.push(cgg_core::NameMention {
                name: name.into(),
                via: via.into(),
                site_line: 1,
                site_byte: 0,
            });
        }
        let r = analyze(&g, &[], &[fa], &opts_rust_full());
        let conf = |qn: &str| {
            r.findings
                .iter()
                .find(|f| f.qualified_name == qn)
                .map(|f| f.confidence)
                .unwrap()
        };
        assert_eq!(conf("crate::a::default_true"), Confidence::Medium);
        assert_eq!(conf("crate::b::default_true"), Confidence::High);
        assert_eq!(conf("crate::MultiIndex::is_empty"), Confidence::High);
    }

    // B3: a reflective lookup by a literal name (`getattr(o, "m")`,
    // `getDeclaredMethod("m")`) caps a same-named finding at medium; a
    // module load (`require("m")`) names a file and caps nothing; a
    // callable nothing names stays high.
    #[test]
    fn reflective_lookups_cap_at_medium_module_loads_do_not() {
        let g = graph_with(vec![
            node(0, "crate::main", "main", "rust"),
            vis_node(1, "crate::looked_up", "looked_up", "", Vis::Private),
            vis_node(2, "crate::loaded", "loaded", "", Vis::Private),
            vis_node(3, "crate::untouched", "untouched", "", Vis::Private),
        ]);
        let mut fa = FileFacts::new(cgg_core::ids::FileId::new(0), "a.rs".into(), "rust");
        for (name, via) in [("looked_up", "getattr"), ("loaded", "require")] {
            fa.dyn_uses.push(cgg_core::DynUse {
                name: name.into(),
                via: via.into(),
                site_line: 1,
                site_byte: 0,
            });
        }
        let r = analyze(&g, &[], &[fa], &opts_rust_full());
        let conf = |qn: &str| {
            r.findings
                .iter()
                .find(|f| f.qualified_name == qn)
                .map(|f| f.confidence)
                .unwrap()
        };
        assert_eq!(conf("crate::looked_up"), Confidence::Medium);
        assert_eq!(conf("crate::loaded"), Confidence::High);
        assert_eq!(conf("crate::untouched"), Confidence::High);
    }

    // B3: a named function passing itself as a value (`connect(handler)`
    // inside `handler`) is a self-reference, never an edge — but it is
    // named as a value, so doubt, like a named closure. One named nowhere
    // stays high.
    #[test]
    fn a_value_reference_to_a_named_function_is_doubt() {
        let g = graph_with(vec![
            node(0, "crate::main", "main", "rust"),
            vis_node(1, "crate::handler", "handler", "", Vis::Private),
            vis_node(2, "crate::other", "other", "", Vis::Private),
        ]);
        let mut fa = FileFacts::new(cgg_core::ids::FileId::new(0), "a.rs".into(), "rust");
        fa.references.push(cgg_core::RefRecord {
            name: "handler".into(),
            receiver_hint: cgg_core::VALUE_REF_HINT.into(),
            ..Default::default()
        });
        let r = analyze(&g, &[], &[fa], &opts_rust_full());
        let conf = |qn: &str| {
            r.findings
                .iter()
                .find(|f| f.qualified_name == qn)
                .map(|f| f.confidence)
                .unwrap()
        };
        assert_eq!(conf("crate::handler"), Confidence::Medium);
        assert_eq!(conf("crate::other"), Confidence::High);
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
        ]);
        let r = analyze(&g, &[], &[], &opts_rust_full());
        assert_eq!(r.findings.len(), 1);
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
    fn a_protected_callable_cannot_reach_high() {
        // A subtype may call it, and a subtype may be declared outside
        // the analyzed tree. Before, `Protected` added no evidence at
        // all, and since a never-referenced finding starts at High that
        // left it in the top band beside private code.
        let g = graph_with(vec![
            node(0, "crate::main", "main", "rust"),
            vis_node(1, "crate::guarded", "guarded", "", Vis::Protected),
            vis_node(2, "crate::hidden", "hidden", "", Vis::Private),
        ]);
        let r = analyze(&g, &[], &[], &opts_rust_full());
        let f = r
            .findings
            .iter()
            .find(|f| f.simple_name == "guarded")
            .unwrap();
        assert_eq!(f.confidence, Confidence::Medium);
        assert!(matches!(
            f.evidence.iter().find(|e| e.slug() == "public-visibility"),
            Some(Evidence::PublicVisibility { token }) if token == "protected",
        ));
        // Never corroborated as if it were private.
        assert!(!f.evidence.iter().any(|e| e.slug() == "private-visibility"));
        // Near-miss: the private sibling keeps the top band.
        let p = r
            .findings
            .iter()
            .find(|f| f.simple_name == "hidden")
            .unwrap();
        assert_eq!(p.confidence, Confidence::High);
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

    // --- WS5: dispatch through declared bases --------------------------

    fn opts_full(lang: &str) -> DeadCodeOptions {
        let mut o = opts();
        o.language_signals.insert(
            lang.into(),
            cgg_core::deadcode::LanguageSignals {
                visibility: true,
                attributes: true,
                value_refs: true,
                impls: true,
                ..Default::default()
            },
        );
        o
    }

    /// A private member of `owner` in `lang`, at a distinct byte offset.
    fn member(
        id: u32,
        lang: &str,
        owner: &str,
        name: &str,
        attrs: &[&str],
    ) -> CallableNode {
        let mut n = node(id, &format!("p.{owner}.{name}"), name, lang);
        n.kind = CallableKind::Method;
        n.vis = Vis::Private;
        n.start_byte = id * 100;
        n.end_byte = id * 100 + 50;
        n.attributes = attrs.iter().map(|a| a.to_string()).collect();
        n
    }

    fn def_of(n: &CallableNode, bases: &[&str]) -> cgg_core::DefRecord {
        cgg_core::DefRecord {
            simple_name: n.simple_name.clone(),
            qualified_name: n.qualified_name.clone(),
            variant: cgg_core::DefVariant::InherentMethod,
            start_byte: n.start_byte,
            end_byte: n.end_byte,
            base_types: bases.iter().map(|b| b.to_string()).collect(),
            ..Default::default()
        }
    }

    fn facts_for(lang: &str, defs: Vec<cgg_core::DefRecord>) -> Vec<FileFacts> {
        let mut f = FileFacts::new(
            cgg_core::ids::FileId::new(0),
            std::path::PathBuf::from("t.src"),
            lang,
        );
        // Java/C# record every class with its bases; mirror that.
        for d in &defs {
            if let Some((owner_path, _)) =
                crate::names::split_last_segment(&d.qualified_name)
                && !f.classes.iter().any(|c| c.class_qn == owner_path)
            {
                f.classes.push(cgg_core::ClassDecl {
                    class_qn: owner_path.to_string(),
                    base_types: d.base_types.clone(),
                    line: 1,
                });
            }
        }
        f.definitions = defs;
        vec![f]
    }

    fn finding<'r>(r: &'r DeadCodeReport, qn: &str) -> &'r DeadCodeFinding {
        r.findings
            .iter()
            .find(|f| f.qualified_name == qn)
            .unwrap_or_else(|| panic!("{qn} not reported"))
    }

    #[test]
    fn an_override_of_an_out_of_tree_base_member_is_not_high() {
        // `class Ext extends Handler { @Override void handle() }` with
        // `Handler` outside the tree: the runtime calls `handle`.
        let main = node(0, "p.Main.main", "main", "java");
        let ext = member(1, "java", "Ext", "handle", &["@Override"]);
        // Near miss: the same class's unmarked package-private helper.
        let helper = member(2, "java", "Ext", "unused", &[]);
        let facts = facts_for(
            "java",
            vec![def_of(&ext, &["Handler"]), def_of(&helper, &["Handler"])],
        );
        let g = graph_with(vec![main, ext, helper]);
        let r = analyze(&g, &[], &facts, &opts_full("java"));
        let f = finding(&r, "p.Ext.handle");
        assert_eq!(f.confidence, Confidence::Medium, "{:?}", f.evidence);
        assert!(f.evidence.iter().any(|e| matches!(
            e,
            Evidence::ImplementsExternalContract { contract } if contract == "Handler"
        )));
        assert_eq!(finding(&r, "p.Ext.unused").confidence, Confidence::High);
    }

    // WS20 gap 6: an `async def` method is kind `function`; the owner
    // decides that it is a method.
    #[test]
    fn a_python_async_method_of_an_outside_base_class_is_not_high() {
        let mut hook = member(1, "python", "P", "_ahook", &[]);
        hook.kind = CallableKind::Function;
        // Near miss: a module-level async function has no class owner.
        let mut free = node(2, "p.helpers._free", "_free", "python");
        free.vis = Vis::Private;
        free.start_byte = 200;
        free.end_byte = 250;
        let facts = facts_for(
            "python",
            vec![def_of(&hook, &["external.Base"]), def_of(&free, &[])],
        );
        let g = graph_with(vec![node(0, "p.main", "main", "python"), hook, free]);
        let r = analyze(&g, &[], &facts, &opts_full("python"));
        let f = finding(&r, "p.P._ahook");
        assert_eq!(f.confidence, Confidence::Medium, "{:?}", f.evidence);
        assert!(
            !finding(&r, "p.helpers._free")
                .evidence
                .iter()
                .any(|e| e.slug() == "implements-external-contract")
        );
    }

    // WS20 gap 5: PHP 8.3 writes the marker `#[\Override]`.
    #[test]
    fn php_override_attribute_matches_in_any_spelling() {
        for a in ["#[\\Override]", "\\Override", "Override", "#[Override]"] {
            assert!(OVERRIDE_MARKERS.contains(&override_marker_text(a)), "{a}");
        }
        assert!(!OVERRIDE_MARKERS.contains(&override_marker_text("#[Route]")));
        let h = member(1, "php", "Ctl", "handle", &["\\Override"]);
        let plain = member(2, "php", "Ctl", "other", &[]);
        let facts = facts_for(
            "php",
            vec![
                def_of(&h, &["Vendor\\Base"]),
                def_of(&plain, &["Vendor\\Base"]),
            ],
        );
        let g = graph_with(vec![node(0, "p.main", "main", "php"), h, plain]);
        let r = analyze(&g, &[], &facts, &opts_full("php"));
        let has = |qn: &str| {
            finding(&r, qn)
                .evidence
                .iter()
                .any(|e| e.slug() == "implements-external-contract")
        };
        assert!(has("p.Ctl.handle"));
        // PHP methods are not hooks by mere membership.
        assert!(!has("p.Ctl.other"));
    }

    // WS20 gap 11: a call after `return` keeps nothing alive.
    #[test]
    fn a_call_in_unreachable_code_does_not_keep_its_target_live() {
        let mut main = node(0, "crate::main", "main", "rust");
        main.start_byte = 0;
        main.end_byte = 100;
        let mut dead = node(1, "crate::target", "target", "rust");
        dead.start_byte = 200;
        dead.end_byte = 250;
        let mut live = node(2, "crate::live", "live", "rust");
        live.start_byte = 300;
        live.end_byte = 350;
        let mut g = graph_with(vec![main, dead, live]);
        let mut e = edge(0, 1);
        e.site_byte = 60;
        g.edges.push(e);
        // Near miss: a call before the terminator.
        let mut e = edge(0, 2);
        e.site_byte = 20;
        g.edges.push(e);
        let mut f = FileFacts::new(
            cgg_core::ids::FileId::new(0),
            std::path::PathBuf::from("t.src"),
            "rust",
        );
        f.unreachable.push(cgg_core::UnreachableRegion {
            start_line: 5,
            end_line: 6,
            start_byte: 50,
            end_byte: 90,
            cause: "after-return".into(),
        });
        let facts = vec![f];
        let r = analyze(&g, &[], &facts, &opts_full("rust"));
        let t = finding(&r, "crate::target");
        assert!(t.evidence.iter().any(|e| matches!(
            e,
            Evidence::IncomingOnlyFromUnreachableCode { sites: 1 }
        )));
        assert!(!r.findings.iter().any(|f| f.qualified_name == "crate::live"));
        // The graph edge is not structurally certain any more.
        assert_eq!(crate::unreachable::cap_unreachable_edges(&mut g, &facts), 1);
        assert_eq!(g.edges[0].confidence, Confidence::Medium);
        assert_eq!(g.edges[1].confidence, Confidence::High);
    }

    #[test]
    fn a_partial_method_part_is_a_contract_member_not_high() {
        // C# `partial void OnLoaded();`: its other half and its callers
        // are routinely generated or outside the tree.
        let part = member(
            1,
            "csharp",
            "Data",
            "OnLoaded",
            &[cgg_core::PARTIAL_DECLARATION],
        );
        let plain = member(2, "csharp", "Data", "Helper", &[]);
        let facts = facts_for("csharp", vec![def_of(&part, &[]), def_of(&plain, &[])]);
        let g = graph_with(vec![node(0, "p.Main.main", "main", "csharp"), part, plain]);
        let r = analyze(&g, &[], &facts, &opts_full("csharp"));
        let has = |qn: &str| {
            finding(&r, qn)
                .evidence
                .iter()
                .any(|e| e.slug() == "implements-external-contract")
        };
        assert!(has("p.Data.OnLoaded"));
        assert!(!has("p.Data.Helper"));
    }

    #[test]
    fn an_override_of_an_in_tree_base_member_carries_no_external_contract() {
        // The base declares it: the dispatch fan-out from the declaration
        // is what reaches the override, not a cap.
        let base = member(1, "java", "Base", "apply", &[]);
        let kid = member(2, "java", "Kid", "apply", &["@Override"]);
        let facts = facts_for("java", vec![def_of(&base, &[]), def_of(&kid, &["Base"])]);
        let g = graph_with(vec![node(0, "p.Main.main", "main", "java"), base, kid]);
        let r = analyze(&g, &[], &facts, &opts_full("java"));
        assert!(
            !finding(&r, "p.Kid.apply")
                .evidence
                .iter()
                .any(|e| e.slug() == "implements-external-contract")
        );
    }

    #[test]
    fn csharp_explicit_interface_impl_of_an_outside_interface_is_not_high() {
        let imp = member(1, "csharp", "Proc", "Tick", &["override"]);
        let facts = facts_for("csharp", vec![def_of(&imp, &["IExternal"])]);
        let g = graph_with(vec![node(0, "p.Main.Main", "Main", "csharp"), imp]);
        let r = analyze(&g, &[], &facts, &opts_full("csharp"));
        assert_eq!(finding(&r, "p.Proc.Tick").confidence, Confidence::Medium);
    }

    #[test]
    fn python_method_of_a_class_with_an_outside_base_is_not_high() {
        let hook = member(1, "python", "MyForm", "_save_m2m", &[]);
        // Name-mangled: no base can call it by this name.
        let mangled = member(2, "python", "MyForm", "__secret", &[]);
        // No outside base at all.
        let plain = member(3, "python", "Plain", "_helper", &[]);
        let facts = facts_for(
            "python",
            vec![
                def_of(&hook, &["ModelForm"]),
                def_of(&mangled, &["ModelForm"]),
                def_of(&plain, &[]),
            ],
        );
        let g = graph_with(vec![
            node(0, "p.main", "main", "python"),
            hook,
            mangled,
            plain,
        ]);
        let r = analyze(&g, &[], &facts, &opts_full("python"));
        assert_eq!(
            finding(&r, "p.MyForm._save_m2m").confidence,
            Confidence::Medium
        );
        assert_eq!(
            finding(&r, "p.MyForm.__secret").confidence,
            Confidence::High
        );
        assert_eq!(finding(&r, "p.Plain._helper").confidence, Confidence::High);
    }

    #[test]
    fn rust_impl_of_an_outside_trait_is_not_high_but_an_in_tree_one_is_judged() {
        let mut ext = vis_node(
            1,
            "crate::<Cors as Fairing>::info",
            "info",
            "",
            Vis::Unknown,
        );
        ext.trait_impl_target = Some("Fairing".into());
        let decl = vis_node(2, "crate::Store::load", "load", "", Vis::Private);
        let mut imp =
            vis_node(3, "crate::<Mem as Store>::load", "load", "", Vis::Unknown);
        imp.trait_impl_target = Some("Store".into());
        let g = graph_with(vec![node(0, "crate::main", "main", "rust"), ext, decl, imp]);
        let r = analyze(&g, &[], &[], &opts_rust_full());
        let f = finding(&r, "crate::<Cors as Fairing>::info");
        assert_eq!(f.confidence, Confidence::Medium, "{:?}", f.evidence);
        assert!(
            !finding(&r, "crate::<Mem as Store>::load")
                .evidence
                .iter()
                .any(|e| e.slug() == "implements-external-contract")
        );
    }

    fn unresolved_site(
        name: &str,
        hint: &str,
        reason: UnresolvedReason,
    ) -> AuditUnresolvedCall {
        AuditUnresolvedCall::new(
            Some(CallableId::new(0)),
            cgg_core::ids::FileId::new(0),
            3,
            30,
            name.into(),
            hint.into(),
            reason,
        )
    }

    #[test]
    fn a_variable_or_generic_receiver_is_doubt_and_an_unrelated_declared_type_is_not() {
        let target = member(1, "java", "Worker", "process", &[]);
        let other = member(2, "java", "Ledger", "post", &[]);
        let facts = facts_for("java", vec![def_of(&target, &[]), def_of(&other, &[])]);
        let base_graph = || {
            graph_with(vec![
                node(0, "p.Main.main", "main", "java"),
                target.clone(),
                other.clone(),
            ])
        };
        for hint in ["orderService", "p", "T", "this.repo"] {
            let mut g = base_graph();
            g.unresolved.push(unresolved_site(
                "process",
                hint,
                UnresolvedReason::NoCandidateInFile,
            ));
            let r = analyze(&g, &[], &facts, &opts_full("java"));
            let f = finding(&r, "p.Worker.process");
            assert_ne!(f.confidence, Confidence::High, "{hint}: {:?}", f.evidence);
        }
        // `Ledger` is a type the tree declares and `Worker` is not related
        // to it: that site cannot be this method.
        let mut g = base_graph();
        g.unresolved.push(unresolved_site(
            "process",
            "Ledger",
            UnresolvedReason::NoCandidateInFile,
        ));
        let r = analyze(&g, &[], &facts, &opts_full("java"));
        assert_eq!(finding(&r, "p.Worker.process").confidence, Confidence::High);
    }

    #[test]
    fn an_over_cap_site_is_doubt_for_every_candidate_and_an_outside_super_is_not() {
        let target = member(1, "java", "Worker", "process", &[]);
        let other = member(2, "java", "Ledger", "post", &[]);
        let facts = facts_for("java", vec![def_of(&target, &[]), def_of(&other, &[])]);
        let mk = |hint: &str, reason: UnresolvedReason| {
            let mut g = graph_with(vec![
                node(0, "p.Main.main", "main", "java"),
                target.clone(),
                other.clone(),
            ]);
            g.unresolved.push(unresolved_site("process", hint, reason));
            analyze(&g, &[], &facts, &opts_full("java"))
        };
        let r = mk(
            "Ledger",
            UnresolvedReason::FanoutCapExceeded { candidates: 9 },
        );
        assert_ne!(finding(&r, "p.Worker.process").confidence, Confidence::High);
        let r = mk("super", UnresolvedReason::SuperBaseOutOfGraph);
        assert_eq!(finding(&r, "p.Worker.process").confidence, Confidence::High);
    }

    #[test]
    fn a_base_member_whose_override_is_called_is_a_dispatch_surface() {
        let base = member(1, "java", "Shape", "draw", &[]);
        let kid = member(2, "java", "Square", "draw", &["@Override"]);
        let facts = facts_for("java", vec![def_of(&base, &[]), def_of(&kid, &["Shape"])]);
        let mut g = graph_with(vec![node(0, "p.Main.main", "main", "java"), base, kid]);
        // Uncalled override: the base member is plain dead code.
        let r = analyze(&g, &[], &facts, &opts_full("java"));
        assert_eq!(finding(&r, "p.Shape.draw").confidence, Confidence::High);
        // Called directly on the subclass: the base member is the surface.
        link(&mut g, 0, 2);
        let r = analyze(&g, &[], &facts, &opts_full("java"));
        let f = finding(&r, "p.Shape.draw");
        assert_eq!(f.confidence, Confidence::Medium, "{:?}", f.evidence);
        assert!(
            f.evidence
                .iter()
                .any(|e| e.slug() == "implemented-trait-surface")
        );
    }

    #[test]
    fn a_site_that_cannot_legally_call_a_java_private_member_is_no_doubt() {
        // `private` is the top-level class, which is the file: a site in
        // another file is never this method's call.
        let mut g = graph_with(vec![
            node(0, "p.Main.main", "main", "java"),
            member(1, "java", "Worker", "process", &[]),
        ]);
        g.add_file(cgg_core::graph::FileRecord {
            id: cgg_core::ids::FileId::new(1),
            path: std::path::PathBuf::from("o.java"),
            language: "java".into(),
            ..Default::default()
        });
        let mut u = unresolved_site("process", "x", UnresolvedReason::NoCandidateInFile);
        u.file = cgg_core::ids::FileId::new(1);
        g.unresolved.push(u.clone());
        let facts = facts_for("java", vec![]);
        let r = analyze(&g, &[], &facts, &opts_full("java"));
        assert_eq!(finding(&r, "p.Worker.process").confidence, Confidence::High);
        // The same site in the member's own file is doubt.
        u.file = cgg_core::ids::FileId::new(0);
        g.unresolved = vec![u];
        let r = analyze(&g, &[], &facts, &opts_full("java"));
        assert_ne!(finding(&r, "p.Worker.process").confidence, Confidence::High);
    }
}
