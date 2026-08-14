//! Framework entry-point detection.
//!
//! Runs after every other resolver, over the finished graph plus the
//! per-file facts, and answers one question: *where does control enter
//! this tree from outside it?*
//!
//! The pipeline is two passes.
//!
//! 1. **Detection.** Which frameworks are actually in use, from import
//!    prefixes, file-path conventions, and — for ecosystems with no
//!    import records at all — the presence of a signature call.
//!    Detection is the gate that makes the matchers safe: `get` is a
//!    route verb in a file that imports Flask and a map lookup
//!    everywhere else.
//!
//! 2. **Matching.** For each detected framework, apply its matchers to
//!    the graph. Attribute matches read `CallableNode::attributes`;
//!    registrar matches read the `context`/`route` slots on
//!    `RefRecord`; base-type matches read `DefRecord::base_types`.
//!
//! The output is [`FrameworkOutcome`] — a list of entries the driver
//! turns into nodes, plus the coverage disclosure that states what this
//! run could *not* do. The second half is not optional. A partial list
//! that reads as complete is worse than no list, and naming the gaps is
//! what makes the recognised half usable.

pub use cgg_core::frameworks::rules;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use rayon::prelude::*;

use cgg_core::frameworks::{
    EntryShape, FrameworkCoverage, FrameworkEntry, FrameworkRule, RecognisedFramework,
    SeenFramework, UncoveredLanguage,
};
use cgg_core::graph::Graph;
use cgg_core::ids::{CallableId, FileId};
use cgg_core::{FileFacts, STRING_REF_HINT, VALUE_REF_HINT};

use crate::deadcode::roots::{attribute_key, attribute_string_arg, attribute_verb};

/// What the framework pass produced.
#[derive(Debug, Default)]
pub struct FrameworkOutcome {
    /// Entries in deterministic order: by target callable id, then by
    /// framework id. Node minting order follows this, so ids are stable
    /// across runs.
    pub entries: Vec<FrameworkEntry>,
    pub coverage: FrameworkCoverage,
}

/// Run detection and matching.
///
/// `user_rules` are appended to the built-in table and take precedence
/// on a tie, so a user can cover a framework cgg does not know without
/// waiting for a release.
pub fn detect(
    graph: &Graph,
    facts: &[FileFacts],
    user_rules: &[FrameworkRule],
) -> FrameworkOutcome {
    let mut all: Vec<FrameworkRule> = user_rules.to_vec();
    all.extend(rules::builtin());

    let _sp = cgg_core::profile::span("frameworks::total");
    let evidence = {
        let _s = cgg_core::profile::span("frameworks::detect-scan");
        Detection::scan(graph, facts, &all)
    };
    let called_in_file = callees_within_their_file(graph);
    let mut name_indexes: HashMap<String, NameIndex> = HashMap::new();
    let mut base_indexes: HashMap<String, BaseTypeIndex> = HashMap::new();
    let mut reg_indexes: HashMap<String, RegistrarIndex> = HashMap::new();

    // Build every per-language index up front. They are shared by all
    // rules of that language, so this must happen before the rules fan
    // out — and it is cheap next to matching.
    {
        let _si = cgg_core::profile::span("frameworks::index-build");
        for rule in &all {
            if !evidence.is_active(&rule.id, &rule.language) {
                continue;
            }
            name_indexes
                .entry(rule.language.clone())
                .or_insert_with(|| NameIndex::build(graph, &rule.language));
            base_indexes
                .entry(rule.language.clone())
                .or_insert_with(|| BaseTypeIndex::build(facts, &rule.language));
            reg_indexes
                .entry(rule.language.clone())
                .or_insert_with(|| RegistrarIndex::build(facts, &rule.language));
        }
    }

    // Rules are independent: each reads the shared read-only indexes and
    // produces its own entries. `par_iter().flat_map().collect()`
    // preserves rule order, so the entry sequence — and every node id
    // minted from it — is identical to the serial form.
    let active: Vec<&FrameworkRule> = all
        .iter()
        .filter(|r| evidence.is_active(&r.id, &r.language))
        .collect();
    let mut entries: Vec<FrameworkEntry> = active
        .par_iter()
        .flat_map_iter(|rule| {
            let rule = *rule;
            let names = &name_indexes[&rule.language];
            let bases = &base_indexes[&rule.language];
            let regs = &reg_indexes[&rule.language];
            let mut entries: Vec<FrameworkEntry> = Vec::new();

            {
                let _s = cgg_core::profile::span("frameworks::match-attributes");
                match_attributes(graph, rule, &mut entries);
            }
            {
                let _s = cgg_core::profile::span("frameworks::match-base-types");
                match_base_types(graph, bases, rule, &mut entries);
            }
            {
                let _s = cgg_core::profile::span("frameworks::match-methods");
                match_methods(graph, rule, &mut entries);
            }
            {
                let _s = cgg_core::profile::span("frameworks::match-registrars");
                match_registrars(
                    graph,
                    facts,
                    regs,
                    names,
                    rule,
                    &called_in_file,
                    &mut entries,
                );
            }
            {
                let _s = cgg_core::profile::span("frameworks::match-self-modules");
                match_self_modules(graph, facts, rule, &called_in_file, &mut entries);
            }
            {
                let _s = cgg_core::profile::span("frameworks::match-visibility");
                match_visibility(graph, rule, &mut entries);
            }
            entries.into_iter()
        })
        .collect();

    dedup(&mut entries);
    let coverage = evidence.into_coverage(&all, &entries, graph);
    FrameworkOutcome { entries, coverage }
}

// ---------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------

/// Per-(framework, language) detection evidence.
struct Detection {
    /// (id, language) -> number of files carrying a marker.
    hits: BTreeMap<(String, String), u32>,
    /// Files per language that cgg analyzed, for the "no rules" line.
    files_by_language: BTreeMap<String, u32>,
}

impl Detection {
    fn scan(graph: &Graph, facts: &[FileFacts], all: &[FrameworkRule]) -> Self {
        let mut hits: BTreeMap<(String, String), u32> = BTreeMap::new();
        let mut files_by_language: BTreeMap<String, u32> = BTreeMap::new();

        // Index rules by language so each file is compared only against
        // the rules that could possibly apply to it.
        let mut by_lang: HashMap<&str, Vec<(usize, &FrameworkRule)>> = HashMap::new();
        for (i, r) in all.iter().enumerate() {
            by_lang.entry(r.language.as_str()).or_default().push((i, r));
        }

        // (language, import prefix) -> rule indices. Built once.
        let mut prefix_index: HashMap<(&str, &str), Vec<usize>> = HashMap::new();
        for (i, r) in all.iter().enumerate() {
            for p in &r.detect {
                prefix_index
                    .entry((r.language.as_str(), p.as_str()))
                    .or_default()
                    .push(i);
            }
        }

        // Attributes per file, read from the *graph* — the same place
        // `match_attributes` reads them. Detecting from `facts` instead
        // would let a marker-only rule detect on a file whose matcher
        // then finds nothing, or the reverse.
        let mut attrs_by_file: HashMap<FileId, Vec<&str>> = HashMap::new();
        for c in graph.callables.values() {
            if c.attributes.is_empty() {
                continue;
            }
            attrs_by_file
                .entry(c.file)
                .or_default()
                .extend(c.attributes.iter().map(|a| a.as_str()));
        }

        for f in facts {
            *files_by_language.entry(f.language.clone()).or_default() += 1;
            let Some(candidates) = by_lang.get(f.language.as_str()) else {
                continue;
            };

            let path = normalize_path(&f.path.to_string_lossy());
            let call_names: HashSet<&str> = f
                .references
                .iter()
                .filter(|r| {
                    r.receiver_hint != VALUE_REF_HINT
                        && r.receiver_hint != STRING_REF_HINT
                })
                .map(|r| r.name.as_str())
                .collect();

            // Which rules this file's imports could possibly match,
            // looked up rather than scanned. The table went from 51
            // rules to several hundred, and the old form was
            // `rules x prefixes x imports` per file — on a TypeScript
            // tree that is ~40 rules times ~30 imports for every file,
            // which measured as a ~9% whole-run regression. Every prefix
            // that can match a path is a cut of that path at a separator,
            // so the candidates are a map lookup per cut instead.
            let mut import_hits: HashSet<usize> = HashSet::new();
            for imp in &f.imports {
                let raw = imp.path.trim().trim_matches(|c| c == '"' || c == '\'');
                for key in prefix_keys(raw) {
                    if let Some(ids) = prefix_index.get(&(f.language.as_str(), key)) {
                        import_hits.extend(ids.iter().copied());
                    }
                }
            }

            for (ri, rule) in candidates {
                let mut hit = import_hits.contains(ri);
                if !hit {
                    hit = rule.detect_paths.iter().any(|p| path_matches(&path, p));
                }
                if !hit {
                    let calls = rules::detect_calls_for(&rule.id, &rule.language);
                    hit = calls.iter().any(|c| call_names.contains(c));
                }
                // A marker-only rule (CUDA's `__global__`) has no import
                // to gate on — the marker on the definition IS the
                // evidence. Treating it as unconditionally detected made
                // every repository with a C++ file report CUDA as a
                // coverage gap, which is the opposite of disclosure.
                if !hit
                    && rule.detect.is_empty()
                    && rule.detect_paths.is_empty()
                    && rules::detect_calls_for(&rule.id, &rule.language).is_empty()
                {
                    hit = attrs_by_file.get(&f.file).is_some_and(|attrs| {
                        attrs.iter().any(|a| {
                            contains_ci(&rule.attributes, attribute_key(a))
                                || contains_ci(&rule.attributes, attribute_verb(a))
                        })
                    });
                    // A visibility rule has nothing to import either: the
                    // keyword on the definition is the evidence.
                    if !hit {
                        let want =
                            rules::visibility_entries_for(&rule.id, &rule.language);
                        hit = !want.is_empty()
                            && f.definitions
                                .iter()
                                .any(|d| eq_any_ci(want, &d.visibility));
                    }
                }
                if hit {
                    *hits
                        .entry((rule.id.clone(), rule.language.clone()))
                        .or_default() += 1;
                }
            }
        }

        // Files the walker classified but that produced no facts still
        // count toward per-language totals.
        for fr in graph.files.values() {
            if !facts.iter().any(|f| f.file == fr.id) {
                *files_by_language.entry(fr.language.clone()).or_default() += 1;
            }
        }

        Self {
            hits,
            files_by_language,
        }
    }

    fn is_active(&self, id: &str, language: &str) -> bool {
        self.hits
            .contains_key(&(id.to_string(), language.to_string()))
    }

    fn into_coverage(
        self,
        all: &[FrameworkRule],
        entries: &[FrameworkEntry],
        graph: &Graph,
    ) -> FrameworkCoverage {
        let mut cov = FrameworkCoverage::new();

        // Keyed by language, not just id. A framework with a rule per
        // language (Express has JavaScript and TypeScript ones) would
        // otherwise report the *combined* total on both rows — Ghost
        // printed "express 349 entries" twice for one set of 349, which
        // overstates the surface and invites summing it to 698.
        let mut counts: BTreeMap<(&str, &str), u32> = BTreeMap::new();
        for e in entries {
            let lang = graph
                .callables
                .get(&e.target)
                .map(|c| c.language.as_str())
                .unwrap_or_default();
            *counts.entry((e.framework.as_str(), lang)).or_default() += 1;
        }

        for ((id, language), files) in &self.hits {
            let rule = all.iter().find(|r| r.id == *id && r.language == *language);
            let Some(rule) = rule else { continue };
            let n = *counts.get(&(id.as_str(), language.as_str())).unwrap_or(&0);

            if rule.has_matchers() {
                cov.recognised.push(RecognisedFramework {
                    id: id.clone(),
                    language: language.clone(),
                    kind: rule.kind,
                    entries: n,
                });
            } else {
                let gap = rules::gap_for(id, language);
                cov.seen_no_rules.push(SeenFramework {
                    id: id.clone(),
                    language: language.clone(),
                    files: *files,
                    reason: if gap.is_empty() {
                        "cgg has no entry rules for this framework".to_string()
                    } else {
                        gap.to_string()
                    },
                });
            }
        }
        // A recognised framework that produced nothing is reported as a
        // gap too. Zero entries beside a name reads as "there are none",
        // and on a framework cgg claims to understand that is the most
        // dangerous line in the table.
        let mut i = 0;
        while i < cov.recognised.len() {
            if cov.recognised[i].entries == 0 {
                let r = cov.recognised.remove(i);
                let files = *self
                    .hits
                    .get(&(r.id.clone(), r.language.clone()))
                    .unwrap_or(&0);
                cov.seen_no_rules.push(SeenFramework {
                    id: r.id,
                    language: r.language,
                    files,
                    reason: "detected, but no entry point matched its rules".to_string(),
                });
            } else {
                i += 1;
            }
        }

        let covered = rules::languages_with_rules();
        for (language, files) in &self.files_by_language {
            if !covered.contains(&language.as_str()) {
                cov.no_markers.push(UncoveredLanguage {
                    language: language.clone(),
                    files: *files,
                });
            }
        }

        cov.recognised.sort_by(|a, b| {
            b.entries
                .cmp(&a.entries)
                .then_with(|| a.id.cmp(&b.id))
                .then_with(|| a.language.cmp(&b.language))
        });
        cov.seen_no_rules
            .sort_by(|a, b| a.id.cmp(&b.id).then_with(|| a.language.cmp(&b.language)));
        cov.no_markers.sort_by(|a, b| {
            b.files
                .cmp(&a.files)
                .then_with(|| a.language.cmp(&b.language))
        });

        cov.nodes_minted = entries.iter().filter(|e| e.node).count() as u32;
        cov.root_marks_only = entries.iter().filter(|e| !e.node).count() as u32;
        cov
    }
}

/// Whether an import path is, or is under, `prefix`.
///
/// Segment-aware so `next` does not match `nextcloud` and `express` does
/// not match `expresso`, while `flask` still matches `flask.views` and
/// `github.com/gin-gonic/gin` matches itself with a version suffix.
/// Every prefix of `path` that [`import_matches`] would accept: the
/// whole path, and the path cut at each separator. Looking these up in
/// the prefix index is equivalent to testing the path against every
/// rule's prefixes, without visiting the rules that cannot match.
fn prefix_keys(path: &str) -> impl Iterator<Item = &str> {
    std::iter::once(path).chain(
        path.char_indices()
            .filter(|(_, c)| matches!(c, '.' | '/' | ':' | '\\'))
            .map(move |(i, _)| &path[..i]),
    )
}

/// The predicate the prefix index replaces. Kept as the definition of
/// what "this import proves this framework" means, and pinned to the
/// index by `prefix_index_agrees_with_import_matches` — an index that
/// silently diverged from it would make rules stop firing with no
/// symptom other than a coverage table quietly claiming less.
#[cfg_attr(not(test), allow(dead_code))]
fn import_matches(path: &str, prefix: &str) -> bool {
    let p = path.trim().trim_matches(|c| c == '"' || c == '\'');
    if p == prefix {
        return true;
    }
    if let Some(rest) = p.strip_prefix(prefix) {
        return rest.starts_with(['.', '/', ':', '\\']);
    }
    // Rust `use axum::routing::get` arrives as `axum::routing::get`;
    // Go module paths may carry a `/v2` major-version suffix.
    p.split(['.', '/', ':', '\\'])
        .next()
        .is_some_and(|head| head == prefix)
}

fn normalize_path(p: &str) -> String {
    p.replace('\\', "/")
}

/// Whether a file path matches a convention marker (`config/routes.rb`,
/// `urls.py`, `routes/web.php`). Suffix match on a path-segment
/// boundary, so `app/urls.py` matches `urls.py` but `myurls.py` does not.
fn path_matches(path: &str, marker: &str) -> bool {
    let m = marker.trim_end_matches('/');
    if let Some(idx) = path.rfind(m) {
        let ends_ok = idx + m.len() == path.len() || marker.ends_with('/');
        let starts_ok = idx == 0 || path.as_bytes()[idx - 1] == b'/';
        return ends_ok && starts_ok;
    }
    false
}

// ---------------------------------------------------------------------
// Matchers
// ---------------------------------------------------------------------

/// Registrar verbs that collide with ordinary vocabulary.
///
/// `get`, `add` and `use` name HTTP routes *and* map lookups, list
/// appends and dependency injection. Detection gates them to projects
/// that use the framework at all, but inside such a project every
/// `session.get(k)` still looks like a route.
///
/// A match on one of these needs corroboration: either the call carries
/// an identity (a route string), or it is receiver-less — axum's
/// `get(handler)` is a free function, `crate_ids.get(id)` is not.
/// Distinctive verbs (`HandleFunc`, `RegisterWorkflow`, `add_action`)
/// need neither, because nothing else is called that.
const AMBIGUOUS_VERBS: &[&str] = &[
    "get", "post", "put", "delete", "patch", "head", "options", "add", "all", "use",
    "route", "match", "handle", "process", "on", "view", "resource", "any", "list",
    "create", "update",
];

/// Whether a marker list contains `needle`, comparing case-insensitively.
///
/// Go writes `GET`, Rails writes `get` and NestJS writes `Get` for the
/// same concept; a case-sensitive compare would need three copies of
/// every verb list.
fn contains_ci(list: &[String], needle: &str) -> bool {
    list.iter().any(|m| m.eq_ignore_ascii_case(needle))
}

/// **Shape A** — a marker on the definition.
///
/// The highest-evidence shape in the set: the framework's own
/// registration syntax is sitting on the callable. Only the precise
/// route string is uncertain, never whether this is an entry point.
fn match_attributes(graph: &Graph, rule: &FrameworkRule, out: &mut Vec<FrameworkEntry>) {
    if rule.attributes.is_empty() {
        return;
    }
    for (id, node) in &graph.callables {
        if node.language != rule.language || node.synthetic {
            continue;
        }
        for attr in &node.attributes {
            let key = attribute_key(attr);
            let verb = attribute_verb(attr);
            if !contains_ci(&rule.attributes, key) && !contains_ci(&rule.attributes, verb)
            {
                continue;
            }
            let route = match attribute_string_arg(attr) {
                Some(path) => format!("{verb}(\"{path}\")"),
                None => String::new(),
            };
            out.push(FrameworkEntry {
                framework: rule.id.clone(),
                kind: rule.kind,
                shape: EntryShape::Attribute,
                route,
                target: *id,
                target_name: node.qualified_name.clone(),
                evidence: attr.trim().to_string(),
                file: node.file,
                site_line: node.start_line,
                node: rule.node,
            });
            // No `break`: stacked decorators are one handler serving
            // several routes, and `@app.get("/a")` + `@app.post("/a")`
            // are two entry points. `dedup` keys on the route, so
            // identical markers still collapse.
        }
    }
}

/// **Shape D** — a base class or interface declares the contract.
///
/// Per §8 most of these only mark a root: a single node fanning out to
/// every model in the repository is visually useless, and the entry has
/// no identity of its own to name. The exceptions are the ones that do —
/// one Akka actor's `createReceive`, one Quartz `IJob` — and those carry
/// `node: true` in the rule.
/// Registrar-shaped references for one language, bucketed by verb.
///
/// A reference is a candidate only if it is a value/string ref with a
/// non-empty context; those three tests and the language filter are the
/// same for every rule, so they run once here rather than once per rule.
struct RegistrarIndex<'a> {
    by_verb: HashMap<String, Vec<(&'a FileFacts, &'a cgg_core::RefRecord)>>,
}

impl<'a> RegistrarIndex<'a> {
    fn build(facts: &'a [FileFacts], language: &str) -> Self {
        let mut by_verb: HashMap<String, Vec<(&'a FileFacts, &'a cgg_core::RefRecord)>> =
            HashMap::new();
        for f in facts {
            if f.language != language {
                continue;
            }
            for r in &f.references {
                if r.receiver_hint != VALUE_REF_HINT && r.receiver_hint != STRING_REF_HINT
                {
                    continue;
                }
                if r.context.is_empty() {
                    continue;
                }
                let verb = r
                    .context
                    .rsplit(['.', ':', '-', '>'])
                    .next()
                    .unwrap_or(&r.context);
                by_verb
                    .entry(verb.to_ascii_lowercase())
                    .or_default()
                    .push((f, r));
                // A rule may name the whole receiver path
                // (`Route::get`), which the old scan also accepted.
                let full = r.context.to_ascii_lowercase();
                if full != verb.to_ascii_lowercase() {
                    by_verb.entry(full).or_default().push((f, r));
                }
            }
        }
        Self { by_verb }
    }

    /// Every (file, reference) this rule's registrars could match, each
    /// at most once.
    fn candidates(
        &self,
        rule: &FrameworkRule,
    ) -> Vec<(&'a FileFacts, &'a cgg_core::RefRecord)> {
        // Dedup by record IDENTITY, not by site. One site can carry
        // several distinct references: `SiteView.as_view()` emits both
        // the callee (`as_view`) and its qualifier (`SiteView`) at the
        // same byte, and the qualifier is the one that binds. Keying on
        // (file, site_byte) dropped it and silently un-did Django's
        // class-based-view support.
        let mut seen: HashSet<*const cgg_core::RefRecord> = HashSet::new();
        let mut out = Vec::new();
        for reg in &rule.registrars {
            let Some(hits) = self.by_verb.get(&reg.to_ascii_lowercase()) else {
                continue;
            };
            for (f, r) in hits {
                if seen.insert(*r as *const _) {
                    out.push((*f, *r));
                }
            }
        }
        out
    }
}

/// Declared base types for one language, indexed once.
struct BaseTypeIndex {
    /// (file, start_byte) -> base types, from the facts side.
    bases: HashMap<(FileId, u32), Vec<String>>,
    /// owner type -> its declared bases, for walking the chain.
    by_owner: HashMap<String, Vec<String>>,
}

impl BaseTypeIndex {
    fn build(facts: &[FileFacts], language: &str) -> Self {
        let mut bases: HashMap<(FileId, u32), Vec<String>> = HashMap::new();
        let mut by_owner: HashMap<String, Vec<String>> = HashMap::new();
        for f in facts {
            if f.language != language {
                continue;
            }
            for d in &f.definitions {
                if d.base_types.is_empty() {
                    continue;
                }
                bases.insert((f.file, d.start_byte), d.base_types.clone());
                if let Some(owner) = crate::names::owner_from_qn(&d.qualified_name) {
                    by_owner.insert(owner.to_string(), d.base_types.clone());
                }
            }
        }
        Self { bases, by_owner }
    }
}

fn match_base_types(
    graph: &Graph,
    idx: &BaseTypeIndex,
    rule: &FrameworkRule,
    out: &mut Vec<FrameworkEntry>,
) {
    if rule.base_types.is_empty() || idx.bases.is_empty() {
        return;
    }
    let bases = &idx.bases;
    let by_owner = &idx.by_owner;

    for (id, node) in &graph.callables {
        if node.language != rule.language || node.synthetic {
            continue;
        }
        let Some(types) = bases.get(&(node.file, node.start_byte)) else {
            continue;
        };
        // A framework contract is usually inherited, not declared
        // directly: NetBox writes `class CircuitListView(generic.
        // ObjectListView)`, and only three levels up does anything name
        // Django's `View`. Matching the immediate bases alone sees none
        // of a real application's class-based views.
        let Some(hit) = find_in_base_chain(types, by_owner, &rule.base_types) else {
            continue;
        };
        if !rule.methods.is_empty() && !contains_ci(&rule.methods, &node.simple_name) {
            continue;
        }
        out.push(FrameworkEntry {
            framework: rule.id.clone(),
            kind: rule.kind,
            shape: EntryShape::BaseType,
            route: String::new(),
            target: *id,
            target_name: node.qualified_name.clone(),
            evidence: format!("declares `{hit}`"),
            file: node.file,
            site_line: node.start_line,
            node: rule.node,
        });
    }
}

/// Walk a type's inheritance chain looking for one the rule names.
///
/// Bounded by a visited set and a depth cap: a base list read from
/// syntax can be cyclic (two files each naming the other's type) and a
/// deep hierarchy is not more evidence than a shallow one.
fn find_in_base_chain(
    direct: &[String],
    by_owner: &HashMap<String, Vec<String>>,
    wanted: &[String],
) -> Option<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut frontier: Vec<String> = direct.to_vec();
    for _ in 0..8 {
        if frontier.is_empty() {
            break;
        }
        let mut next: Vec<String> = Vec::new();
        for t in frontier {
            if !seen.insert(t.clone()) {
                continue;
            }
            if base_type_matches(wanted, &t) {
                return Some(t);
            }
            // Look the base up by its own last segment, since a
            // declaration writes `generic.ObjectListView` but the class
            // is defined as `ObjectListView`.
            let key = t
                .split(['<', '['])
                .next()
                .unwrap_or(&t)
                .rsplit(['.', ':', '\\'])
                .next()
                .unwrap_or(&t);
            if let Some(parents) = by_owner.get(key) {
                next.extend(parents.iter().cloned());
            }
        }
        frontier = next;
    }
    None
}

fn base_type_matches(wanted: &[String], declared: &str) -> bool {
    let d = declared.trim();
    if contains_ci(wanted, d) {
        return true;
    }
    // `nn.Module` should match a rule written as `Module`, and
    // `Sidekiq::Job` a rule written as `Sidekiq::Job` — compare the last
    // segment too, and strip generic arguments (`IConsumer<OrderPlaced>`).
    let bare = d.split(['<', '[']).next().unwrap_or(d).trim();
    if contains_ci(wanted, bare) {
        return true;
    }
    let last = bare.rsplit(['.', ':', '\\']).next().unwrap_or(bare);
    contains_ci(wanted, last)
}

/// A method-name rule with no base type — the structural-typing escape
/// hatch.
///
/// Go interfaces are satisfied implicitly: nothing in the source says
/// `Handler implements http.Handler`, so there is no declaration to
/// match and the method name is the only signal available. This is
/// weaker evidence than every other shape, which is why it fires only
/// when the framework's import is present *and* the rule declares no
/// base types at all.
fn match_methods(graph: &Graph, rule: &FrameworkRule, out: &mut Vec<FrameworkEntry>) {
    if rule.methods.is_empty() || !rule.base_types.is_empty() {
        return;
    }
    for (id, node) in &graph.callables {
        if node.language != rule.language || node.synthetic {
            continue;
        }
        if !contains_ci(&rule.methods, &node.simple_name) {
            continue;
        }
        out.push(FrameworkEntry {
            framework: rule.id.clone(),
            kind: rule.kind,
            shape: EntryShape::BaseType,
            route: String::new(),
            target: *id,
            target_name: node.qualified_name.clone(),
            evidence: format!(
                "`{}` satisfies an implicitly-implemented {} interface",
                node.simple_name, rule.id
            ),
            file: node.file,
            site_line: node.start_line,
            node: rule.node,
        });
    }
}

/// **Shapes B, C and E** — the handler sits in argument position.
///
/// All three arrive as `RefRecord`s carrying a `context` (the registrar
/// call) and a `route` (its first string literal). What differs is only
/// how the target is named: by identifier (B), by the synthesized name
/// of an inline closure (C), or by a string that has to be decoded (E).
fn match_registrars(
    graph: &Graph,
    facts: &[FileFacts],
    regs: &RegistrarIndex<'_>,
    index: &NameIndex,
    rule: &FrameworkRule,
    called_in_file: &HashSet<CallableId>,
    out: &mut Vec<FrameworkEntry>,
) {
    if rule.registrars.is_empty() {
        return;
    }

    // Walk only the references whose verb some registrar of THIS rule
    // names, instead of every reference in the tree once per rule.
    // Measured on netbox: the old form was 5.3s of a 6.2s framework
    // phase, because seven active rules each re-scanned ~1,300 files
    // worth of references to reject almost all of them.
    for (f, r) in regs.candidates(rule) {
        {
            let is_value = r.receiver_hint == VALUE_REF_HINT;
            let is_string = r.receiver_hint == STRING_REF_HINT;
            let verb = r
                .context
                .rsplit(['.', ':', '-', '>'])
                .next()
                .unwrap_or(&r.context);
            // An ambiguous verb needs corroboration — see AMBIGUOUS_VERBS.
            let receiverless = r.context.eq_ignore_ascii_case(verb);
            if contains_ci(
                &AMBIGUOUS_VERBS
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>(),
                verb,
            ) && r.route.is_empty()
                && !receiverless
            {
                continue;
            }

            // **Shape F** — the string names a whole module rather than
            // a callable. `new Worker('./jobs/resize.js')` is the only
            // reference to that file anywhere, so without this the
            // entire module reads as dead.
            if is_string
                && let Some(target_file) = resolve_module_path(graph, f.file, &r.name)
            {
                let mut any = false;
                for (id, node) in &graph.callables {
                    if node.file != target_file || node.synthetic {
                        continue;
                    }
                    if !is_module_surface(node, facts, target_file, called_in_file) {
                        continue;
                    }
                    any = true;
                    out.push(FrameworkEntry {
                        framework: rule.id.clone(),
                        kind: rule.kind,
                        shape: EntryShape::ModulePath,
                        route: format!("{}(\"{}\")", verb, r.name),
                        target: *id,
                        target_name: node.qualified_name.clone(),
                        evidence: format!(
                            "module loaded by `{}(\"{}\")`",
                            r.context, r.name
                        ),
                        file: f.file,
                        site_line: r.site_line,
                        node: rule.node,
                    });
                }
                if any {
                    continue;
                }
            }

            // A reference that names a *type* rather than a callable:
            // `path("/x", SiteView.as_view())`, `router.register("s",
            // SiteViewSet)`. cgg has no node for a type, so the entry is
            // every method of that type the rule calls an entry point.
            // Gated on `rule.methods` — without it this would claim every
            // method of every class handed to a registrar.
            if is_value && !rule.methods.is_empty() {
                let mut any = false;
                for m in &rule.methods {
                    let Some(target) = index.by_owner_method(Some(&r.name), m, f.file)
                    else {
                        continue;
                    };
                    let Some(node) = graph.callables.get(&target) else {
                        continue;
                    };
                    any = true;
                    out.push(FrameworkEntry {
                        framework: rule.id.clone(),
                        kind: rule.kind,
                        shape: EntryShape::BaseType,
                        route: if r.route.is_empty() {
                            String::new()
                        } else {
                            format!("{}(\"{}\")", verb.to_ascii_lowercase(), r.route)
                        },
                        target,
                        target_name: node.qualified_name.clone(),
                        evidence: format!(
                            "`{}` handed to `{}`; `{}` is an entry method of it",
                            r.name, r.context, m
                        ),
                        file: f.file,
                        site_line: r.site_line,
                        node: rule.node,
                    });
                }
                if any {
                    continue;
                }
            }

            let (candidates, shape) = if is_value {
                let id = index.by_simple(&r.name, f.file);
                // A handler written in place is shape C, not B: the
                // graph already reaches its body, and the node exists
                // only to name the route.
                let shape = if r.name.starts_with("handler_at_")
                    || r.name.starts_with("closure_at_")
                    || r.name.starts_with("async_at_")
                {
                    EntryShape::Closure
                } else {
                    EntryShape::Registrar
                };
                (id, shape)
            } else {
                // Shape E is opt-in per rule: decoding a string into a
                // handler *name* is only correct where the framework
                // really routes that way. (Shape F above is a module
                // path, which is a different question and ungated.)
                if !rule.string_targets {
                    continue;
                }
                match decode_string_target(&r.name, &rule.language) {
                    Some((owner, method)) => (
                        index.by_owner_method(owner.as_deref(), &method, f.file),
                        EntryShape::StringTarget,
                    ),
                    None => continue,
                }
            };
            // A registrar reference that names nothing cgg can see is
            // dropped rather than guessed at. §8: string routing may
            // lower confidence, it must never manufacture an edge.
            let Some(target) = candidates else { continue };
            let Some(node) = graph.callables.get(&target) else {
                continue;
            };

            let route = if r.route.is_empty() {
                String::new()
            } else {
                format!("{}(\"{}\")", verb.to_ascii_lowercase(), r.route)
            };
            out.push(FrameworkEntry {
                framework: rule.id.clone(),
                kind: rule.kind,
                shape,
                route,
                target,
                target_name: node.qualified_name.clone(),
                evidence: if r.route.is_empty() {
                    format!("registered by `{}`", r.context)
                } else {
                    format!("registered by `{}(\"{}\")`", r.context, r.route)
                },
                file: f.file,
                site_line: r.site_line,
                node: rule.node,
            });
        }
    }
}

/// Resolve a module path written in a registration call to a file cgg
/// analyzed — `new Worker('./jobs/resize.js')` from `src/queue.js`
/// resolves to `src/jobs/resize.js`.
///
/// Only relative paths, and only to files already in the graph. A bare
/// package name (`'bullmq'`) is a dependency, not a worker module, and
/// resolving one would claim third-party code as an entry point.
fn resolve_module_path(graph: &Graph, from: FileId, raw: &str) -> Option<FileId> {
    let s = raw.trim();
    if !(s.starts_with("./") || s.starts_with("../")) {
        return None;
    }
    let base = graph.files.get(&from)?.path.parent()?.to_path_buf();
    let joined = base.join(s);

    // Normalize `.` / `..` textually: the file may not exist on disk in
    // a sandboxed or partial checkout, and the graph is the authority
    // for what was analyzed anyway.
    let mut parts: Vec<std::ffi::OsString> = Vec::new();
    for c in joined.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                parts.pop();
            }
            other => parts.push(other.as_os_str().to_os_string()),
        }
    }
    let mut norm = std::path::PathBuf::new();
    for p in parts {
        norm.push(p);
    }

    let has_ext = norm.extension().is_some();
    for f in graph.files.values() {
        if f.path == norm {
            return Some(f.id);
        }
        if !has_ext {
            for ext in ["js", "mjs", "cjs", "ts", "tsx", "py", "rb"] {
                if f.path == norm.with_extension(ext) {
                    return Some(f.id);
                }
            }
        }
    }
    None
}

/// A visibility keyword that is itself the trust boundary.
///
/// Solidity needs no framework and no import: `public` and `external`
/// mean any address on the chain can call the function. That is the
/// whole attack surface of a contract, and no other matcher can state
/// it — the other four all key on something written *around* a
/// definition, while this is a keyword *on* it.
///
/// Reads `CallableNode::visibility`, the language-native string, rather
/// than the normalized `vis` enum: Solidity populates the former and
/// leaves the latter `Unknown`.
/// Case-insensitive membership over a static list. `contains_ci` takes
/// `&[String]` (rule fields are owned); the out-of-band lookups are
/// `&'static [&'static str]`.
fn eq_any_ci(set: &[&str], value: &str) -> bool {
    set.iter().any(|w| w.eq_ignore_ascii_case(value))
}

fn match_visibility(graph: &Graph, rule: &FrameworkRule, out: &mut Vec<FrameworkEntry>) {
    let wanted =
        cgg_core::frameworks::rules::visibility_entries_for(&rule.id, &rule.language);
    if wanted.is_empty() {
        return;
    }
    for (id, node) in &graph.callables {
        if node.language != rule.language || node.synthetic {
            continue;
        }
        if !eq_any_ci(wanted, &node.visibility) {
            continue;
        }
        out.push(FrameworkEntry {
            framework: rule.id.clone(),
            kind: rule.kind,
            // The keyword is a marker on the definition, which is
            // shape A even though no framework is involved.
            shape: EntryShape::Attribute,
            route: node.simple_name.clone(),
            target: *id,
            target_name: node.qualified_name.clone(),
            evidence: format!("`{}` visibility", node.visibility),
            file: node.file,
            site_line: node.start_line,
            node: rule.node,
        });
    }
}

/// **Shape F, self-identifying** — the file itself is what the framework
/// enters, and it says so.
///
/// The spawn-site form (`new Worker('./jobs/x.js')`) needs a literal
/// path. Ghost writes its workers the other way round: the job module
/// imports `node:worker_threads` and talks to `parentPort`, and the
/// spawner passes a variable. Nothing references the file, so every
/// callable in it reads as dead — the exact failure this shape exists to
/// prevent, just approached from the other end.
///
/// Requires *both* the framework import and a receive-side marker in the
/// same file. The import alone would also match the spawner, which is
/// not entered as a thread.
fn match_self_modules(
    graph: &Graph,
    facts: &[FileFacts],
    rule: &FrameworkRule,
    called_in_file: &HashSet<CallableId>,
    out: &mut Vec<FrameworkEntry>,
) {
    let markers =
        cgg_core::frameworks::rules::self_module_markers_for(&rule.id, &rule.language);
    if markers.is_empty() {
        return;
    }

    for f in facts {
        if f.language != rule.language {
            continue;
        }
        let imported = f.imports.iter().any(|i| {
            rule.detect
                .iter()
                .any(|d| i.path == *d || i.path.starts_with(&format!("{d}/")))
        });
        if !imported {
            continue;
        }
        let Some(marker) = f.references.iter().find_map(|r| {
            markers
                .iter()
                .find(|m| r.receiver_hint == **m || r.name == **m)
                .copied()
        }) else {
            continue;
        };

        // The module's own file name is its identity — there is no route
        // string, and every worker module is a distinct entry point.
        let stem = f
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("worker")
            .to_string();

        for (id, node) in &graph.callables {
            if node.file != f.file || node.synthetic {
                continue;
            }
            if !is_module_surface(node, facts, f.file, called_in_file) {
                continue;
            }
            out.push(FrameworkEntry {
                framework: rule.id.clone(),
                kind: rule.kind,
                shape: EntryShape::ModulePath,
                route: format!("module(\"{stem}\")"),
                target: *id,
                target_name: node.qualified_name.clone(),
                evidence: format!(
                    "worker module: imports {} and uses `{marker}`",
                    rule.language
                ),
                file: f.file,
                site_line: 1,
                node: rule.node,
            });
        }
    }
}

/// Whether a callable is part of a module's externally-visible surface.
///
/// A worker module is *entered*, not called, so there is no single
/// target. Pointing at every callable in the file would be exactly the
/// fan-out §8 rejects — `resize` and its private `normalize` helper are
/// not both entry points, and saying so would misdescribe the module.
///
/// Declared exports when the file has them; otherwise the callables
/// nothing inside the file calls, which is the same question asked of
/// the graph instead of the syntax.
fn is_module_surface(
    node: &cgg_core::graph::CallableNode,
    facts: &[FileFacts],
    file: FileId,
    called_in_file: &HashSet<CallableId>,
) -> bool {
    if node.kind != cgg_core::graph::CallableKind::Function {
        return false;
    }
    if let Some(f) = facts.iter().find(|f| f.file == file)
        && !f.exports.is_empty()
    {
        return f.exports.iter().any(|e| e.name == node.simple_name);
    }
    !called_in_file.contains(&node.id)
}

/// Callables that already have a caller inside their own file.
fn callees_within_their_file(graph: &Graph) -> HashSet<CallableId> {
    graph
        .edges
        .iter()
        .filter(|e| {
            let (Some(s), Some(d)) =
                (graph.callables.get(&e.src), graph.callables.get(&e.dst))
            else {
                return false;
            };
            s.file == d.file && s.id != d.id
        })
        .map(|e| e.dst)
        .collect()
}

/// Trailing segments that mean the string is a filename rather than a
/// `module.function` handler path.
const ASSET_EXTENSIONS: &[&str] = &[
    "html", "htm", "css", "js", "mjs", "cjs", "json", "yml", "yaml", "toml", "xml",
    "txt", "md", "csv", "svg", "png", "jpg", "jpeg", "gif", "ico", "pdf", "zip", "gz",
    "lock", "sql", "sh", "log",
];

/// Decode a string that names a callable — shape E.
///
/// This is the only genuinely framework-specific decoding in the module,
/// and it is deliberately narrow. A string that does not match one of
/// these forms yields `None` rather than a guess.
///
/// * Rails    — `"photos#index"` → (`PhotosController`, `index`)
/// * Laravel  — `"App\\Http\\C@method"` → (`C`, `method`)
/// * PHP/misc — `"C::method"` → (`C`, `method`)
/// * AWS      — `"app.lambda_handler"` → (`app`, `lambda_handler`)
/// * bare     — `"handler_name"` → (none, `handler_name`)
fn decode_string_target(s: &str, language: &str) -> Option<(Option<String>, String)> {
    let s = s.trim();
    if s.is_empty() || s.len() > 200 {
        return None;
    }
    if let Some((ctrl, action)) = s.split_once('#')
        && language == "ruby"
        && is_identifierish(action)
    {
        // Rails names controllers by convention: `photos#index` is
        // `PhotosController#index`. Match on the action plus a
        // controller whose name starts with the camelized segment,
        // which the index resolves loosely.
        let owner = camelize(ctrl.rsplit('/').next().unwrap_or(ctrl));
        return Some((Some(format!("{owner}Controller")), action.to_string()));
    }
    if let Some((class, method)) = s.split_once('@')
        && is_identifierish(method)
    {
        let owner = class.rsplit('\\').next().unwrap_or(class);
        return Some((Some(owner.to_string()), method.to_string()));
    }
    if let Some((class, method)) = s.rsplit_once("::")
        && is_identifierish(method)
    {
        let owner = class.rsplit('\\').next().unwrap_or(class);
        return Some((Some(owner.to_string()), method.to_string()));
    }
    // `module.function` — how every AWS runtime names a Lambda handler,
    // in a CDK stack (`handler="app.lambda_handler"`), a SAM template or
    // serverless.yml (`src/handlers/user.create`). The module segment is
    // the owner, which is exactly what `owner_from_qn` derives for a
    // free function in a dot-joined language.
    //
    // Deliberately narrow. The directory prefix is only stripped when
    // what remains still carries a dot, so `"application/json"` does not
    // become a claim on a callable named `json`. Both halves must be
    // identifiers, and an unresolvable target is dropped rather than
    // guessed at — a wrong entry node is worse than a missing one.
    let tail = s.rsplit('/').next().unwrap_or(s);
    if let Some((module, func)) = tail.rsplit_once('.')
        && is_identifierish(module)
        && is_identifierish(func)
        // `index.html` is indistinguishable from `module.function` by
        // shape alone. Resolution would drop it anyway — nothing in the
        // graph is called `html` on an owner called `index` — but a
        // filename is never a handler, and saying so here keeps the
        // failure impossible rather than merely unlikely.
        && !ASSET_EXTENSIONS
            .iter()
            .any(|e| func.eq_ignore_ascii_case(e))
    {
        return Some((Some(module.to_string()), func.to_string()));
    }
    if is_identifierish(s) {
        return Some((None, s.to_string()));
    }
    None
}

fn is_identifierish(s: &str) -> bool {
    !s.is_empty()
        && s.starts_with(|c: char| c.is_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// `photos` → `Photos`, `blog_posts` → `BlogPosts`.
fn camelize(s: &str) -> String {
    s.split('_')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut c = p.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// Name lookup over the graph, scoped to one language.
struct NameIndex {
    by_simple: HashMap<String, Vec<(FileId, CallableId)>>,
    by_owner_method: HashMap<(String, String), Vec<CallableId>>,
    /// `(file stem, simple name)`. A deployment string like
    /// `"orders.processOrder"` names a *module*, not a type, and in
    /// every AWS runtime the module is the file stem. Python happens to
    /// qualify its callables that way already, so `by_owner_method`
    /// finds them; JavaScript, TypeScript and Go do not qualify by file
    /// at all, and without this the same string resolves to nothing.
    by_stem_method: HashMap<(String, String), Vec<CallableId>>,
}

impl NameIndex {
    fn build(graph: &Graph, language: &str) -> Self {
        let mut by_simple: HashMap<String, Vec<(FileId, CallableId)>> = HashMap::new();
        let mut by_owner_method: HashMap<(String, String), Vec<CallableId>> =
            HashMap::new();
        let mut by_stem_method: HashMap<(String, String), Vec<CallableId>> =
            HashMap::new();
        for (id, n) in &graph.callables {
            // Sentinel nodes (`<external>`, `<stdlib>`,
            // `<framework-entry>`) are not handlers and must never be
            // resolved to. A *synthesized name* is a different thing
            // from a synthesized node, though: the `handler_at_12`
            // minted for an inline closure names real source, and it is
            // precisely what a shape-C registration points at.
            if n.language != language || n.qualified_name.starts_with('<') {
                continue;
            }
            by_simple
                .entry(n.simple_name.clone())
                .or_default()
                .push((n.file, *id));
            if let Some(owner) = crate::names::owner_from_qn(&n.qualified_name) {
                by_owner_method
                    .entry((owner.to_string(), n.simple_name.clone()))
                    .or_default()
                    .push(*id);
            }
            if let Some(stem) = graph
                .files
                .get(&n.file)
                .and_then(|f| f.path.file_stem())
                .and_then(|s| s.to_str())
            {
                by_stem_method
                    .entry((stem.to_string(), n.simple_name.clone()))
                    .or_default()
                    .push(*id);
            }
        }
        Self {
            by_simple,
            by_owner_method,
            by_stem_method,
        }
    }

    /// Resolve a bare name, preferring a definition in the registering
    /// file. Ambiguity across files is resolved by preferring the local
    /// one and otherwise giving up — an entry node pointing at the wrong
    /// handler is worse than none.
    fn by_simple(&self, name: &str, from: FileId) -> Option<CallableId> {
        let cands = self.by_simple.get(name)?;
        if let Some((_, id)) = cands.iter().find(|(f, _)| *f == from) {
            return Some(*id);
        }
        match cands.as_slice() {
            [(_, id)] => Some(*id),
            _ => None,
        }
    }

    fn by_owner_method(
        &self,
        owner: Option<&str>,
        method: &str,
        from: FileId,
    ) -> Option<CallableId> {
        let Some(owner) = owner else {
            return self.by_simple(method, from);
        };
        if let Some(ids) = self
            .by_owner_method
            .get(&(owner.to_string(), method.to_string()))
            && let [id] = ids.as_slice()
        {
            return Some(*id);
        }
        // Rails' convention pluralizes and suffixes, so an exact owner
        // match often fails where a suffix match succeeds. Only accept
        // it when exactly one owner matches — a near-miss must not
        // become a confident claim.
        let matches: Vec<CallableId> = self
            .by_owner_method
            .iter()
            .filter(|((o, m), _)| m == method && owner_is_close(o, owner))
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect();
        if let [id] = matches.as_slice() {
            return Some(*id);
        }
        // Last: read the qualifier as a module rather than a type. A
        // Lambda handler string is `<file stem>.<function>`, and outside
        // Python nothing qualifies a callable by its file, so this is
        // the only lookup that can resolve one. Still requires a unique
        // hit — two files with the same stem and the same function name
        // resolve to neither.
        match self
            .by_stem_method
            .get(&(owner.to_string(), method.to_string()))
            .map(Vec::as_slice)
        {
            Some([id]) => Some(*id),
            _ => None,
        }
    }
}

/// Whether two owner names are the same type modulo namespace and the
/// `Controller` suffix Rails adds by convention.
fn owner_is_close(declared: &str, wanted: &str) -> bool {
    let d = declared.rsplit([':', '.', '\\']).next().unwrap_or(declared);
    if d.eq_ignore_ascii_case(wanted) {
        return true;
    }
    // `photos#index` asks for `PhotosController`; a project may declare
    // it as `PhotosController` (exact, handled above) or as `Photos`.
    let stripped = wanted.strip_suffix("Controller").unwrap_or(wanted);
    d.eq_ignore_ascii_case(stripped)
}

/// Collapse the matches on one handler to the best description of it.
fn dedup(entries: &mut Vec<FrameworkEntry>) {
    // Strongest shape per target FIRST. Doing it the other way round
    // lets a weaker match that merely ran earlier claim the
    // (target, framework, route) slot and evict the precise one — a
    // Rails `root to: 'photos#index'` losing to "declares
    // ApplicationController", which is the same entry described worse.
    let mut best: HashMap<u32, EntryShape> = HashMap::new();
    for e in entries.iter() {
        let rank = shape_rank(e.shape);
        best.entry(e.target.as_u32())
            .and_modify(|s| {
                if rank < shape_rank(*s) {
                    *s = e.shape;
                }
            })
            .or_insert(e.shape);
    }
    entries.retain(|e| best.get(&e.target.as_u32()) == Some(&e.shape));

    // Then one entry per (target, framework, route). Two rules can
    // legitimately match the same handler — a project using both FastAPI
    // and Flask sees `@app.get` under both vocabularies — and two nodes
    // for one handler would double-count the attack surface.
    let mut seen: BTreeSet<(u32, String, String)> = BTreeSet::new();
    entries.retain(|e| {
        seen.insert((e.target.as_u32(), e.framework.clone(), e.route.clone()))
    });
    entries.sort_by(|a, b| {
        a.target
            .as_u32()
            .cmp(&b.target.as_u32())
            .then_with(|| a.framework.cmp(&b.framework))
            .then_with(|| a.route.cmp(&b.route))
    });
}

/// Lower is stronger evidence.
fn shape_rank(s: EntryShape) -> u8 {
    match s {
        EntryShape::Attribute => 0,
        EntryShape::Registrar => 1,
        EntryShape::Closure => 2,
        EntryShape::StringTarget => 3,
        EntryShape::ModulePath => 4,
        EntryShape::BaseType => 5,
    }
}

#[cfg(test)]
mod tests {
    /// The optimization guard: for every (path, prefix) pair, looking
    /// the prefix up among `prefix_keys(path)` must give the same answer
    /// as calling `import_matches` directly.
    #[test]
    fn prefix_index_agrees_with_import_matches() {
        let paths = [
            "flask",
            "flask.views",
            "org.springframework.web.bind.annotation",
            "axum::routing::get",
            "github.com/go-chi/chi/v5",
            "node:worker_threads",
            "@nestjs/schedule",
            "package:flutter/material.dart",
            "System.Web.Mvc",
            "a",
            "",
        ];
        let prefixes = [
            "flask",
            "flask.views",
            "org",
            "org.springframework",
            "axum",
            "github.com/go-chi/chi",
            "node",
            "node:worker_threads",
            "@nestjs",
            "package:flutter",
            "System.Web",
            "a",
            "b",
            "",
            "org.springframework.web.bind.annotation",
        ];
        for p in paths {
            let keys: Vec<&str> = prefix_keys(p).collect();
            for pre in prefixes {
                let want = import_matches(p, pre);
                let got = keys.contains(&pre);
                assert_eq!(
                    want, got,
                    "path {p:?} prefix {pre:?}: import_matches={want} \
                     index={got} (keys {keys:?})"
                );
            }
        }
    }

    use super::*;
    use crate::deadcode::testutil::{graph_with, node};
    use cgg_core::frameworks::TrustKind;
    use cgg_core::{DefRecord, ImportRecord, RefRecord};
    use std::path::PathBuf;

    fn facts_with_import(lang: &str, path: &str, import: &str) -> FileFacts {
        let mut f = FileFacts::new(FileId::new(0), PathBuf::from(path), lang);
        f.imports.push(ImportRecord {
            kind: "import".into(),
            path: import.into(),
            alias: String::new(),
            site_line: 1,
            site_byte: 0,
        });
        f
    }

    #[test]
    fn import_prefix_matching_is_segment_aware() {
        assert!(import_matches("flask", "flask"));
        assert!(import_matches("flask.views", "flask"));
        assert!(import_matches("axum::routing::get", "axum"));
        assert!(import_matches(
            "github.com/gin-gonic/gin",
            "github.com/gin-gonic/gin"
        ));
        // The whole point of the gate: a lookalike must not activate a
        // rule that would then claim every `get` in the file.
        assert!(!import_matches("flasky", "flask"));
        assert!(!import_matches("nextcloud", "next"));
        assert!(!import_matches("expresso", "express"));
    }

    #[test]
    fn path_conventions_match_on_segment_boundaries() {
        assert!(path_matches("app/urls.py", "urls.py"));
        assert!(path_matches("config/routes.rb", "config/routes.rb"));
        assert!(path_matches("src/app/controllers/x.rb", "app/controllers/"));
        assert!(!path_matches("app/myurls.py", "urls.py"));
    }

    #[test]
    fn undetected_frameworks_contribute_nothing() {
        // A Python file with a `route` decorator but no Flask import is
        // not a Flask app. Without this gate every decorator named
        // `route` in every codebase becomes attack surface.
        let mut n = node(0, "svc.list_users", "list_users", "python");
        n.attributes = vec!["@route('/users')".into()];
        let g = graph_with(vec![n]);
        let facts = vec![facts_with_import("python", "svc.py", "os")];
        let out = detect(&g, &facts, &[]);
        assert!(out.entries.is_empty(), "{:?}", out.entries);
    }

    #[test]
    fn a_flask_route_becomes_a_network_entry() {
        let mut n = node(0, "svc.list_users", "list_users", "python");
        n.attributes = vec!["@app.route(\"/users\")".into()];
        let g = graph_with(vec![n]);
        let facts = vec![facts_with_import("python", "svc.py", "flask")];
        let out = detect(&g, &facts, &[]);
        assert_eq!(out.entries.len(), 1, "{:?}", out.entries);
        let e = &out.entries[0];
        assert_eq!(e.framework, "flask");
        assert_eq!(e.kind, TrustKind::Network);
        assert_eq!(e.shape, EntryShape::Attribute);
        assert_eq!(
            e.node_name(),
            "<framework-entry>::network::flask::route(\"/users\")"
        );
        assert_eq!(out.coverage.network_entries(), 1);
    }

    #[test]
    fn coverage_reports_a_detected_framework_it_cannot_enumerate() {
        // The single most important test in the set: a framework cgg
        // sees but cannot read must be NAMED, not silently counted as
        // zero. Otherwise "0 network entries" reads as "this app has no
        // attack surface".
        let g = graph_with(vec![node(0, "app.Handler", "Handler", "javascript")]);
        let facts = vec![facts_with_import("javascript", "page.js", "next")];
        let out = detect(&g, &facts, &[]);
        assert!(out.entries.is_empty());
        let seen: Vec<&str> = out
            .coverage
            .seen_no_rules
            .iter()
            .map(|s| s.id.as_str())
            .collect();
        assert!(seen.contains(&"nextjs"), "{:?}", out.coverage.seen_no_rules);
        let text = out.coverage.render_text();
        assert!(text.contains("nextjs"), "{text}");
        assert!(text.contains("entries NOT enumerated"), "{text}");
    }

    #[test]
    fn a_recognised_framework_with_zero_entries_is_reported_as_a_gap() {
        // "flask (network, 0 entries)" reads as "this app has no
        // routes". It has to move to the gap list instead.
        let g = graph_with(vec![node(0, "svc.helper", "helper", "python")]);
        let facts = vec![facts_with_import("python", "svc.py", "flask")];
        let out = detect(&g, &facts, &[]);
        assert!(out.coverage.recognised.is_empty());
        assert_eq!(out.coverage.seen_no_rules.len(), 1);
        assert!(
            out.coverage.seen_no_rules[0]
                .reason
                .contains("no entry point matched")
        );
    }

    #[test]
    fn languages_with_no_rules_at_all_are_disclosed() {
        // Uses a language with no rule in the table. Fortran served here
        // until it gained one; if this fails because `verilog` gains a
        // rule, pick another language with none rather than weakening
        // the assertion — the disclosure is the thing being tested.
        let g = graph_with(vec![node(0, "top", "top", "verilog")]);
        let facts = vec![FileFacts::new(
            FileId::new(0),
            PathBuf::from("top.v"),
            "verilog",
        )];
        let out = detect(&g, &facts, &[]);
        assert_eq!(out.coverage.no_markers.len(), 1);
        assert_eq!(out.coverage.no_markers[0].language, "verilog");
        assert!(out.coverage.render_text().contains("verilog"));
    }

    #[test]
    fn base_type_lifecycle_entries_mark_a_root_without_minting_a_node() {
        // §8: a `torch:Module.forward` node per model is visually
        // useless, so the entry exists but mints nothing.
        let mut n = node(0, "model.Encoder.forward", "forward", "python");
        n.file = FileId::new(0);
        n.start_byte = 100;
        let g = graph_with(vec![n]);
        let mut f = facts_with_import("python", "model.py", "torch.nn");
        f.definitions.push(DefRecord {
            simple_name: "forward".into(),
            qualified_name: "model.Encoder.forward".into(),
            start_byte: 100,
            base_types: vec!["nn.Module".into()],
            ..Default::default()
        });
        let out = detect(&g, &f_vec(f), &[]);
        assert_eq!(out.entries.len(), 1, "{:?}", out.entries);
        assert!(
            !out.entries[0].node,
            "lifecycle entries must not mint nodes"
        );
        assert_eq!(out.coverage.root_marks_only, 1);
        assert_eq!(out.coverage.nodes_minted, 0);
    }

    fn f_vec(f: FileFacts) -> Vec<FileFacts> {
        vec![f]
    }

    #[test]
    fn a_registrar_binds_the_handler_named_in_argument_position() {
        let mut handler = node(0, "app.listUsers", "listUsers", "javascript");
        handler.file = FileId::new(0);
        let g = graph_with(vec![handler]);
        let mut f = facts_with_import("javascript", "app.js", "express");
        f.references.push(RefRecord {
            name: "listUsers".into(),
            receiver_hint: VALUE_REF_HINT.to_string(),
            site_line: 4,
            site_byte: 40,
            context: "app.get".into(),
            route: "/users".into(),
            kwargs: Vec::new(),
        });
        let out = detect(&g, &f_vec(f), &[]);
        assert_eq!(out.entries.len(), 1, "{:?}", out.entries);
        assert_eq!(out.entries[0].shape, EntryShape::Registrar);
        assert_eq!(
            out.entries[0].node_name(),
            "<framework-entry>::network::express::get(\"/users\")"
        );
    }

    #[test]
    fn an_ambiguous_verb_on_a_receiver_needs_an_identity() {
        // `crate_ids.get(name)` inside a project that uses axum is a map
        // lookup, not a route. Detection gates the rule to axum users;
        // inside such a project only corroboration separates the two.
        let mut handler = node(0, "crate::name", "name", "rust");
        handler.file = FileId::new(0);
        let g = graph_with(vec![handler]);

        let mut bare = facts_with_import("rust", "s.rs", "axum");
        bare.references.push(RefRecord {
            name: "name".into(),
            receiver_hint: VALUE_REF_HINT.to_string(),
            site_line: 3,
            site_byte: 30,
            context: "crate_ids.get".into(),
            route: String::new(),
            kwargs: Vec::new(),
        });
        assert!(detect(&g, &f_vec(bare), &[]).entries.is_empty());

        // axum's own `get(handler)` is a free function — receiver-less,
        // so it needs no route of its own.
        let mut free = facts_with_import("rust", "s.rs", "axum");
        free.references.push(RefRecord {
            name: "name".into(),
            receiver_hint: VALUE_REF_HINT.to_string(),
            site_line: 3,
            site_byte: 30,
            context: "get".into(),
            route: "/api/x".into(),
            kwargs: Vec::new(),
        });
        assert_eq!(detect(&g, &f_vec(free), &[]).entries.len(), 1);
    }

    #[test]
    fn string_targets_are_opt_in_per_rule() {
        // `session.get("user_id")` must not become a route just because
        // some callable in the project is named `user_id`. Only
        // frameworks that really route by string decode one.
        let mut handler = node(0, "crate::user_id", "user_id", "rust");
        handler.file = FileId::new(0);
        let g = graph_with(vec![handler]);
        let mut f = facts_with_import("rust", "s.rs", "axum");
        f.references.push(RefRecord {
            name: "user_id".into(),
            receiver_hint: STRING_REF_HINT.to_string(),
            site_line: 3,
            site_byte: 30,
            context: "session.get".into(),
            route: "user_id".into(),
            kwargs: Vec::new(),
        });
        assert!(detect(&g, &f_vec(f), &[]).entries.is_empty());
        // Rails, which does route by string, keeps the behaviour.
        assert!(
            rules::builtin()
                .iter()
                .any(|r| r.id == "rails" && r.string_targets)
        );
        assert!(
            rules::builtin()
                .iter()
                .any(|r| r.id == "axum" && !r.string_targets)
        );
    }

    #[test]
    fn a_marker_only_rule_is_not_detected_without_its_marker() {
        // CUDA has no import to gate on, so it used to count as
        // "detected" in every repository containing a C++ file and be
        // reported as a coverage gap in all of them.
        let g = graph_with(vec![node(0, "plain", "plain", "cpp")]);
        let facts = vec![FileFacts::new(
            FileId::new(0),
            PathBuf::from("a.cpp"),
            "cpp",
        )];
        let out = detect(&g, &facts, &[]);
        assert!(
            out.coverage.seen_no_rules.iter().all(|f| f.id != "cuda"),
            "{:?}",
            out.coverage
        );
    }

    #[test]
    fn a_registrar_naming_nothing_visible_is_dropped_not_guessed() {
        let g = graph_with(vec![node(0, "app.other", "other", "javascript")]);
        let mut f = facts_with_import("javascript", "app.js", "express");
        f.references.push(RefRecord {
            name: "listUsers".into(),
            receiver_hint: VALUE_REF_HINT.to_string(),
            site_line: 4,
            site_byte: 40,
            context: "app.get".into(),
            route: "/users".into(),
            kwargs: Vec::new(),
        });
        let out = detect(&g, &f_vec(f), &[]);
        assert!(out.entries.is_empty());
    }

    #[test]
    fn rails_string_routing_decodes_to_a_controller_action() {
        assert_eq!(
            decode_string_target("photos#index", "ruby"),
            Some((Some("PhotosController".into()), "index".into()))
        );
        assert_eq!(
            decode_string_target("admin/blog_posts#show", "ruby"),
            Some((Some("BlogPostsController".into()), "show".into()))
        );
    }

    #[test]
    fn laravel_string_and_array_targets_decode() {
        assert_eq!(
            decode_string_target("App\\Http\\Controllers\\UserController@index", "php"),
            Some((Some("UserController".into()), "index".into()))
        );
        assert_eq!(
            decode_string_target("UserController::store", "php"),
            Some((Some("UserController".into()), "store".into()))
        );
    }

    #[test]
    fn a_string_that_names_nothing_callable_decodes_to_nothing() {
        // §8: string routing may lower confidence; it must never
        // manufacture an edge.
        assert_eq!(decode_string_target("/users/:id", "ruby"), None);
        assert_eq!(decode_string_target("", "php"), None);
        assert_eq!(decode_string_target("a b c", "ruby"), None);
    }

    #[test]
    fn lambda_handler_paths_decode_to_module_and_function() {
        // How every AWS runtime names a handler: in a CDK stack, a SAM
        // template and serverless.yml alike.
        assert_eq!(
            decode_string_target("app.lambda_handler", "python"),
            Some((Some("app".into()), "lambda_handler".into()))
        );
        // serverless.yml writes the module as a path.
        assert_eq!(
            decode_string_target("src/handlers/user.create", "python"),
            Some((Some("user".into()), "create".into()))
        );
        assert_eq!(
            decode_string_target("orders.processOrder", "typescript"),
            Some((Some("orders".into()), "processOrder".into()))
        );
    }

    #[test]
    fn a_path_without_a_dotted_tail_is_not_a_handler() {
        // The directory prefix is only stripped when what remains still
        // carries a dot. Otherwise `"application/json"` — a content type
        // sitting in some registrar's argument list — would become a
        // claim on any callable named `json`.
        assert_eq!(decode_string_target("application/json", "typescript"), None);
        assert_eq!(decode_string_target("text/html", "python"), None);
        // A dotted string whose halves are not identifiers stays out.
        assert_eq!(decode_string_target("v1.2.3", "python"), None);
        assert_eq!(decode_string_target("index.html", "python"), None);
    }

    #[test]
    fn duplicate_matches_on_one_handler_collapse_to_the_strongest() {
        let mut a = FrameworkEntry {
            framework: "flask".into(),
            kind: TrustKind::Network,
            shape: EntryShape::BaseType,
            route: String::new(),
            target: CallableId::new(7),
            target_name: "x".into(),
            evidence: "loose".into(),
            file: FileId::new(0),
            site_line: 1,
            node: true,
        };
        let mut b = a.clone();
        b.shape = EntryShape::Attribute;
        b.route = "get(\"/x\")".into();
        b.evidence = "precise".into();
        let mut v = vec![a.clone(), b];
        dedup(&mut v);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].shape, EntryShape::Attribute);
        // And an unrelated target survives alongside it.
        a.target = CallableId::new(8);
        let mut v2 = vec![v[0].clone(), a];
        dedup(&mut v2);
        assert_eq!(v2.len(), 2);
    }

    #[test]
    fn user_rules_cover_a_framework_cgg_does_not_know() {
        let mut n = node(0, "svc.handle", "handle", "python");
        n.attributes = vec!["@myfw.endpoint(\"/z\")".into()];
        let g = graph_with(vec![n]);
        let facts = vec![facts_with_import("python", "svc.py", "myfw")];
        let user = vec![FrameworkRule {
            id: "myfw".into(),
            language: "python".into(),
            kind: TrustKind::Network,
            detect: vec!["myfw".into()],
            attributes: vec!["endpoint".into()],
            node: true,
            ..Default::default()
        }];
        let out = detect(&g, &facts, &user);
        assert_eq!(out.entries.len(), 1);
        assert_eq!(
            out.entries[0].node_name(),
            "<framework-entry>::network::myfw::endpoint(\"/z\")"
        );
    }

    #[test]
    fn a_framework_contract_is_found_through_the_inheritance_chain() {
        // Real applications never inherit the framework base directly:
        // NetBox writes `class CircuitListView(generic.ObjectListView)`
        // and only three levels up does anything name Django's `View`.
        // Matching immediate bases alone missed half its views.
        let mut leaf = node(0, "app.CircuitListView.get", "get", "python");
        leaf.file = FileId::new(0);
        leaf.start_byte = 100;
        let g = graph_with(vec![leaf]);

        let mut f = facts_with_import("python", "views.py", "django");
        f.definitions.push(DefRecord {
            simple_name: "get".into(),
            qualified_name: "app.CircuitListView.get".into(),
            start_byte: 100,
            base_types: vec!["generic.ObjectListView".into()],
            ..Default::default()
        });
        // The intermediate link, declared elsewhere in the project.
        f.definitions.push(DefRecord {
            simple_name: "setup".into(),
            qualified_name: "generic.ObjectListView.setup".into(),
            start_byte: 200,
            base_types: vec!["View".into()],
            ..Default::default()
        });
        let out = detect(&g, &f_vec(f), &[]);
        assert_eq!(out.entries.len(), 1, "{:?}", out.entries);
        assert_eq!(out.entries[0].shape, EntryShape::BaseType);
    }

    #[test]
    fn a_cyclic_base_chain_terminates() {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        map.insert("A".to_string(), vec!["B".to_string()]);
        map.insert("B".to_string(), vec!["A".to_string()]);
        // Two classes naming each other: must return rather than spin.
        assert_eq!(
            find_in_base_chain(&["A".to_string()], &map, &["Nope".to_string()]),
            None
        );
    }

    #[test]
    fn a_worker_module_path_resolves_relative_to_the_registering_file() {
        let mut g = Graph::default();
        g.add_file(cgg_core::graph::FileRecord {
            id: FileId::new(0),
            path: PathBuf::from("src/queue.js"),
            language: "javascript".into(),
            ..Default::default()
        });
        g.add_file(cgg_core::graph::FileRecord {
            id: FileId::new(1),
            path: PathBuf::from("src/jobs/resize.js"),
            language: "javascript".into(),
            ..Default::default()
        });
        assert_eq!(
            resolve_module_path(&g, FileId::new(0), "./jobs/resize.js"),
            Some(FileId::new(1))
        );
        // Extension-less specifiers resolve too.
        assert_eq!(
            resolve_module_path(&g, FileId::new(0), "./jobs/resize"),
            Some(FileId::new(1))
        );
        // `..` is normalized textually — the file need not exist on disk.
        assert_eq!(
            resolve_module_path(&g, FileId::new(1), "../queue.js"),
            Some(FileId::new(0))
        );
        // A bare package name is a dependency, not a worker module.
        // Resolving one would claim third-party code as an entry point.
        assert_eq!(resolve_module_path(&g, FileId::new(0), "bullmq"), None);
        assert_eq!(
            resolve_module_path(&g, FileId::new(0), "./missing.js"),
            None
        );
    }

    #[test]
    fn a_cuda_kernel_is_an_entry_point_without_any_import() {
        // §8: the grammar cannot parse `saxpy<<<a,b>>>(args)`, so the
        // launch produces no edge and the kernel reads as dead. The
        // qualifier is the evidence, so this rule needs no import gate.
        let mut n = node(0, "saxpy", "saxpy", "cpp");
        n.attributes = vec!["__global__".into()];
        let g = graph_with(vec![n]);
        let facts = vec![FileFacts::new(FileId::new(0), PathBuf::from("k.cu"), "cpp")];
        let out = detect(&g, &facts, &[]);
        assert_eq!(out.entries.len(), 1, "{:?}", out.entries);
        assert_eq!(out.entries[0].framework, "cuda");
        assert_eq!(out.entries[0].kind, TrustKind::Lifecycle);
    }

    #[test]
    fn go_servehttp_is_recognised_without_any_implements_declaration() {
        // Go interfaces are structural, so this is the only shape
        // available — and it is precisely the false positive the
        // design's measurement recorded.
        let g = graph_with(vec![node(0, "main.Handler.ServeHTTP", "ServeHTTP", "go")]);
        let facts = vec![facts_with_import("go", "main.go", "net/http")];
        let out = detect(&g, &facts, &[]);
        assert_eq!(out.entries.len(), 1, "{:?}", out.entries);
        assert_eq!(out.entries[0].framework, "net-http");
        assert_eq!(out.entries[0].kind, TrustKind::Network);
    }
}
