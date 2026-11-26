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
    SeenFramework, TrustKind, UncoveredLanguage,
};
use cgg_core::graph::{CallableKind, Graph};
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
        let vocab = crate::heads::WorkspaceVocab::build(graph, facts);
        Detection::scan(graph, facts, &all, &vocab)
    };
    let called_in_file = callees_within_their_file(graph);
    let in_tree_imports = {
        let _s = cgg_core::profile::span("frameworks::in-tree-imports");
        InTreeImports::build(facts)
    };
    let mock_names = python_mock_bindings(facts);
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
    let harness = HarnessChoice::build(facts, &active, &evidence);
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
                match_attributes(
                    graph,
                    rule,
                    &evidence,
                    &in_tree_imports,
                    &mock_names,
                    &mut entries,
                );
            }
            {
                let _s = cgg_core::profile::span("frameworks::match-base-types");
                match_base_types(
                    graph,
                    bases,
                    rule,
                    &evidence,
                    &in_tree_imports,
                    &mut entries,
                );
            }
            {
                let _s = cgg_core::profile::span("frameworks::match-methods");
                match_methods(graph, rule, &evidence, &mut entries);
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
                    &harness,
                    &evidence,
                    &in_tree_imports,
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
            {
                let _s = cgg_core::profile::span("frameworks::match-observers");
                match_observer_callbacks(graph, facts, rule, &mut entries);
            }
            {
                let _s = cgg_core::profile::span("frameworks::match-member-sources");
                match_member_sources(graph, names, rule, &mut entries);
            }
            {
                let _s = cgg_core::profile::span("frameworks::match-signatures");
                match_signature_methods(graph, rule, &mut entries);
            }
            {
                let _s = cgg_core::profile::span("frameworks::match-parameter-types");
                match_parameter_types(graph, rule, &mut entries);
            }
            {
                let _s = cgg_core::profile::span("frameworks::match-containers");
                match_container_attributes(graph, facts, rule, &mut entries);
            }
            {
                let _s = cgg_core::profile::span("frameworks::match-markup");
                match_markup_bindings(graph, facts, rule, &mut entries);
            }
            entries.into_iter()
        })
        .collect();

    dedup(&mut entries, graph);
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
    /// (id, language, file) for every file that itself carries a marker.
    file_hits: HashSet<(String, String, FileId)>,
}

impl Detection {
    fn scan(
        graph: &Graph,
        facts: &[FileFacts],
        all: &[FrameworkRule],
        vocab: &crate::heads::WorkspaceVocab,
    ) -> Self {
        let mut hits: BTreeMap<(String, String), u32> = BTreeMap::new();
        let mut files_by_language: BTreeMap<String, u32> = BTreeMap::new();
        let mut file_hits: HashSet<(String, String, FileId)> = HashSet::new();

        let own_vocabulary = OwnVocabulary::build(graph);

        // Index rules by language so each file is compared only against
        // the rules that could possibly apply to it.
        let mut by_lang: HashMap<&str, Vec<(usize, &FrameworkRule)>> = HashMap::new();
        for (i, r) in all.iter().enumerate() {
            by_lang.entry(r.language.as_str()).or_default().push((i, r));
        }

        // (language, import prefix) -> rule indices. Built once.
        let mut prefix_index: HashMap<(&str, &str), Vec<usize>> = HashMap::new();
        // `detect` entries holding a `*` are name patterns, matched against
        // every segment of an import: a generated stub's naming convention
        // (`*GrpcKt`, `*_pb2_grpc`), which is all a service file imports.
        let mut pattern_index: HashMap<&str, Vec<(usize, &str)>> = HashMap::new();
        for (i, r) in all.iter().enumerate() {
            for p in &r.detect {
                if p.contains('*') {
                    pattern_index
                        .entry(r.language.as_str())
                        .or_default()
                        .push((i, p.as_str()));
                    continue;
                }
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

        // A container's own attribute (`#[pymethods] impl`) is a marker
        // on the file just as one on a callable is.
        for f in facts {
            for c in &f.attributed_containers {
                attrs_by_file
                    .entry(f.file)
                    .or_default()
                    .extend(c.attributes.iter().map(|a| a.as_str()));
            }
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
                // An import of the tree's own module is not a framework,
                // whatever it is called: ai-doc-gen's `agents/` package is
                // not the OpenAI Agents SDK (`agents`). Python only: its
                // top-level module names are not globally unique the way
                // a JVM or C# package, a Go module path, an npm or a crate
                // name is — one of those the tree declares IS the
                // framework (its own repository).
                let in_tree =
                    f.language == "python" && vocab.declares_import(&f.language, raw);
                let head = raw.split('.').next().unwrap_or(raw);
                if let Some(patterns) = pattern_index.get(f.language.as_str()) {
                    for (i, pat) in patterns {
                        if import_segments(raw).any(|seg| name_pattern_matches(pat, seg))
                        {
                            import_hits.insert(*i);
                        }
                    }
                }
                for key in prefix_keys(raw) {
                    if let Some(ids) = prefix_index.get(&(f.language.as_str(), key)) {
                        import_hits.extend(ids.iter().copied().filter(|i| {
                            !in_tree || own_vocabulary.defines(head, &all[*i])
                        }));
                    }
                }
                // A Rust crate re-exporting a framework (`use
                // shuttle_axum::axum::routing::get`) names it after a
                // `::` rather than at the head.
                if f.language == "rust" {
                    for (i, _) in raw.match_indices("::") {
                        for key in prefix_keys(&raw[i + 2..]) {
                            if let Some(ids) =
                                prefix_index.get(&(f.language.as_str(), key))
                            {
                                import_hits.extend(ids.iter().copied());
                            }
                        }
                    }
                }
            }
            // A JVM file sees its own package's types without importing
            // them: a class in `org.springframework.batch.core` names
            // `JobParametersValidator` bare.
            if matches!(f.language.as_str(), "java" | "kotlin") {
                for pkg in &f.packages {
                    for key in prefix_keys(pkg.trim()) {
                        if let Some(ids) = prefix_index.get(&(f.language.as_str(), key)) {
                            import_hits.extend(ids.iter().copied());
                        }
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
                // A class declared with one of the rule's bases written
                // namespace-qualified (`class Weather < RubyLLM::Tool`) is
                // evidence on its own: Rails autoloads the gem, so no file
                // requires it.
                if !hit {
                    hit = declares_qualified_base(f, &rule.base_types);
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
                    let containers: Vec<String> =
                        rules::container_attributes_for(&rule.id, &rule.language)
                            .iter()
                            .map(|a| a.to_string())
                            .collect();
                    hit = attrs_by_file.get(&f.file).is_some_and(|attrs| {
                        attrs.iter().any(|a| {
                            matches_ci_or_pattern(&containers, attribute_key(a))
                                || matches_ci_or_pattern(&containers, attribute_verb(a))
                                || matches_ci_or_pattern(
                                    &rule.attributes,
                                    attribute_key(a),
                                )
                                || matches_ci_or_pattern(
                                    &rule.attributes,
                                    attribute_verb(a),
                                )
                        })
                    });
                    // Nor does a markup rule: a binding in the file's
                    // sibling markup is the evidence.
                    if !hit {
                        let kinds = rules::markup_kinds_for(&rule.id, &rule.language);
                        hit = f
                            .markup_bindings
                            .iter()
                            .any(|b| kinds.contains(&b.kind.as_str()));
                    }
                    // Nor does a rule whose methods are fixed symbols an
                    // engine calls (libFuzzer's `LLVMFuzzerTestOneInput`):
                    // defining one, by its exact name, is the evidence.
                    // Only the exact names count: a pattern a rule carries
                    // beside them (`*_IRQHandler` beside CMSIS's
                    // `SysTick_Handler`) rides on that detection but is
                    // never evidence on its own.
                    if !hit && rule.methods.iter().any(|m| !m.contains('*')) {
                        // Case as the matcher compares it: Fortran's
                        // `subroutine umat` is Abaqus's `UMAT`.
                        let exact =
                            rules::methods_case_sensitive(&rule.id, &rule.language);
                        // An owner-qualified name (`love.update`) is matched
                        // against the qualified name.
                        hit = f.definitions.iter().any(|d| {
                            rule.methods.iter().any(|m| {
                                !m.contains('*')
                                    && (if exact {
                                        *m == d.simple_name
                                    } else {
                                        m.eq_ignore_ascii_case(&d.simple_name)
                                    } || owner_qualified_matches(
                                        m,
                                        &d.qualified_name,
                                    ))
                            })
                        });
                    }
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
                    file_hits.insert((rule.id.clone(), rule.language.clone(), f.file));
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
            file_hits,
        }
    }

    /// Whether `file` itself carries one of the rule's markers.
    fn file_carries(&self, id: &str, language: &str, file: FileId) -> bool {
        self.file_hits
            .contains(&(id.to_string(), language.to_string(), file))
    }

    /// Whether `file` carries `rule`'s evidence for a marker written
    /// through `head` (`app` of `@app.task`, a bare registrar's verb, a
    /// base type's first segment): the file's own, or — Python, JS/TS —
    /// that of the tree's own module `head` is imported from, following
    /// re-exports a few hops.
    fn carries_for(
        &self,
        rule: &FrameworkRule,
        file: FileId,
        head: &str,
        imports: &InTreeImports,
    ) -> bool {
        if self.file_carries(&rule.id, &rule.language, file) {
            return true;
        }
        let mut frontier: Vec<(FileId, String)> = vec![(file, head.to_string())];
        let mut seen: HashSet<(FileId, String)> = HashSet::new();
        for _ in 0..IN_TREE_HOPS {
            let mut next = Vec::new();
            for key in frontier {
                if !seen.insert(key.clone()) {
                    continue;
                }
                for (src, name) in imports.bound.get(&key).into_iter().flatten() {
                    if self.file_carries(&rule.id, &rule.language, *src) {
                        return true;
                    }
                    if name != "*" && name != "default" {
                        next.push((*src, name.clone()));
                    }
                }
            }
            frontier = next;
        }
        false
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
        //
        // Except the anonymous globals harness when a named harness of its
        // language took the test files: it stood in for nothing there.
        let named_harness: HashSet<String> = cov
            .recognised
            .iter()
            .filter(|r| {
                r.kind == TrustKind::Test
                    && r.entries > 0
                    && rules::harness_globals_rank(&r.id, &r.language) != 1
            })
            .map(|r| r.language.clone())
            .collect();
        cov.recognised.retain(|r| {
            r.entries > 0
                || rules::harness_globals_rank(&r.id, &r.language) != 1
                || !named_harness.contains(&r.language)
        });
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

/// The names a Python tree defines under each module name, for telling a
/// framework's own repository from a package that merely shares a
/// framework's name: `src/flask/` defines `route` and `View`, the names
/// the flask rule looks for; ai-doc-gen's `agents/` defines none of the
/// Agents SDK's.
struct OwnVocabulary {
    /// Directory name or file stem → simple names and owner types
    /// defined in a file under it.
    by_segment: HashMap<String, HashSet<String>>,
}

impl OwnVocabulary {
    fn build(graph: &Graph) -> Self {
        let mut by_segment: HashMap<String, HashSet<String>> = HashMap::new();
        let mut segs_of: HashMap<FileId, Vec<String>> = HashMap::new();
        for c in graph.callables.values() {
            if c.language != "python" || c.synthetic {
                continue;
            }
            let segs = segs_of.entry(c.file).or_insert_with(|| {
                graph
                    .files
                    .get(&c.file)
                    .map(|f| {
                        let p = normalize_path(&f.path.to_string_lossy());
                        p.trim_end_matches(".py")
                            .split('/')
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default()
            });
            let owner = crate::names::owner_from_qn(&c.qualified_name)
                .map(|o| last_segment(o).to_string());
            for seg in segs.iter() {
                let e = by_segment.entry(seg.clone()).or_default();
                e.insert(c.simple_name.clone());
                if let Some(o) = &owner {
                    e.insert(o.clone());
                }
            }
        }
        Self { by_segment }
    }

    /// Whether the tree's module `head` defines one of `rule`'s marker
    /// names: an attribute or registrar verb, or a base type.
    fn defines(&self, head: &str, rule: &FrameworkRule) -> bool {
        let Some(names) = self.by_segment.get(head) else {
            return false;
        };
        rule.attributes
            .iter()
            .chain(&rule.registrars)
            .chain(&rule.base_types)
            .map(|m| last_segment(m))
            .any(|m| !m.contains('*') && names.contains(m))
    }
}

// ---------------------------------------------------------------------
// Per-file evidence
// ---------------------------------------------------------------------

/// Where a Python or JS/TS file's imported names come from, when the
/// module is one of the tree's own files: `(file, local name)` →
/// `(source file, the name there)`.
///
/// A decorator or registrar is usually written through a name the file
/// imports from its own project, not from the framework:
/// `from .celeryconf import app` then `@app.task`, `from . import bp`
/// then `@bp.route`, `import { tool } from "@/lib/ai"`. The file
/// carries no framework import of its own, but the module the name
/// comes from does.
#[derive(Default)]
struct InTreeImports {
    bound: HashMap<(FileId, String), Vec<(FileId, String)>>,
}

/// How many re-export hops [`InTreeImports`] follows.
const IN_TREE_HOPS: u8 = 3;

const JS_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"];

impl InTreeImports {
    fn build(facts: &[FileFacts]) -> Self {
        // Path keys, extension and `__init__`/`index` dropped, `/`-joined:
        // the exact key for relative specifiers, every suffix for absolute
        // and aliased ones.
        let mut exact: HashMap<String, Vec<FileId>> = HashMap::new();
        let mut suffix: HashMap<String, Vec<FileId>> = HashMap::new();
        for f in facts {
            if !matches!(f.language.as_str(), "python" | "javascript" | "typescript") {
                continue;
            }
            let path = normalize_path(&f.path.to_string_lossy());
            let Some(key) = module_key(&path) else {
                continue;
            };
            exact.entry(key.clone()).or_default().push(f.file);
            let segs: Vec<&str> = key.split('/').collect();
            for i in 0..segs.len() {
                suffix.entry(segs[i..].join("/")).or_default().push(f.file);
            }
        }
        let mut out = InTreeImports::default();
        for f in facts {
            let lang = f.language.as_str();
            if !matches!(lang, "python" | "javascript" | "typescript") {
                continue;
            }
            let path = normalize_path(&f.path.to_string_lossy());
            let dir = path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
            for imp in &f.imports {
                let spec = imp.path.trim().trim_matches(|c| c == '"' || c == '\'');
                let targets: Vec<FileId> = if lang == "python" {
                    py_targets(dir, spec, &exact, &suffix)
                } else {
                    js_targets(dir, spec, &exact, &suffix)
                };
                for (local, orig) in import_names(lang, imp) {
                    // `from .. import celeryconf` names a submodule as
                    // often as a member of the package.
                    if lang == "python" && imp.kind == "from-import" {
                        let sub = if spec.chars().all(|c| c == '.') {
                            format!("{spec}{orig}")
                        } else {
                            format!("{spec}.{orig}")
                        };
                        for t in py_targets(dir, &sub, &exact, &suffix) {
                            if t != f.file {
                                out.bound
                                    .entry((f.file, local.clone()))
                                    .or_default()
                                    .push((t, "*".to_string()));
                            }
                        }
                    }
                    if targets.is_empty() {
                        continue;
                    }
                    let e = out.bound.entry((f.file, local)).or_default();
                    for t in &targets {
                        if *t != f.file {
                            e.push((*t, orig.clone()));
                        }
                    }
                }
            }
        }
        out
    }
}

/// `a/b/c.py` → `a/b/c`, `a/b/__init__.py` → `a/b`, `a/b/index.ts` →
/// `a/b`; `None` for a file that is not a module.
fn module_key(path: &str) -> Option<String> {
    let (stem, ext) = path.rsplit_once('.')?;
    if ext != "py" && !JS_EXTENSIONS.contains(&ext) {
        return None;
    }
    let stem = stem.strip_suffix(".d").unwrap_or(stem);
    let stem = stem
        .strip_suffix("/__init__")
        .or_else(|| stem.strip_suffix("/index"))
        .unwrap_or(stem);
    let stem = stem.trim_start_matches("./");
    (!stem.is_empty()).then(|| stem.to_string())
}

/// Join `dir` and a relative path, resolving `.` and `..`.
fn join_rel(dir: &str, rel: &str) -> Option<String> {
    let root = if dir.starts_with('/') { "/" } else { "" };
    let mut segs: Vec<&str> = dir
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    for part in rel.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                segs.pop()?;
            }
            p => segs.push(p),
        }
    }
    Some(format!("{root}{}", segs.join("/")))
}

fn py_targets(
    dir: &str,
    spec: &str,
    exact: &HashMap<String, Vec<FileId>>,
    suffix: &HashMap<String, Vec<FileId>>,
) -> Vec<FileId> {
    if spec.is_empty() {
        return Vec::new();
    }
    if spec.starts_with('.') {
        let level = spec.chars().take_while(|c| *c == '.').count();
        let rest = spec[level..].replace('.', "/");
        let up = "../".repeat(level - 1);
        let Some(key) = join_rel(dir, &format!("{up}{rest}")) else {
            return Vec::new();
        };
        return exact.get(&key).cloned().unwrap_or_default();
    }
    suffix
        .get(&spec.replace('.', "/"))
        .cloned()
        .unwrap_or_default()
}

fn js_targets(
    dir: &str,
    spec: &str,
    exact: &HashMap<String, Vec<FileId>>,
    suffix: &HashMap<String, Vec<FileId>>,
) -> Vec<FileId> {
    let strip_ext = |s: &str| -> String {
        match s.rsplit_once('.') {
            Some((stem, ext)) if JS_EXTENSIONS.contains(&ext) => stem.to_string(),
            _ => s.to_string(),
        }
    };
    if spec.starts_with("./") || spec.starts_with("../") {
        let Some(key) = join_rel(dir, &strip_ext(spec)) else {
            return Vec::new();
        };
        return exact.get(&key).cloned().unwrap_or_default();
    }
    // A path alias (`@/lib/ai`, `~/lib/ai`, `#lib/ai`, `$lib/ai`): the
    // rest after its first segment, matched as a path suffix. A bare
    // package name is a dependency, never the tree's.
    let rest =
        if let Some(r) = spec.strip_prefix("@/").or_else(|| spec.strip_prefix("~/")) {
            r
        } else if spec.starts_with(['#', '$']) {
            match spec.split_once('/') {
                Some((_, r)) => r,
                None => return Vec::new(),
            }
        } else {
            return Vec::new();
        };
    suffix.get(&strip_ext(rest)).cloned().unwrap_or_default()
}

/// The names one import record binds, as `(local, name in the module)`.
fn import_names(lang: &str, imp: &cgg_core::ImportRecord) -> Vec<(String, String)> {
    let mut out = Vec::new();
    match imp.kind.as_str() {
        "from-import" | "re-export" => {
            for item in imp.alias.split(',') {
                let item = item.trim().trim_matches(|c| c == '(' || c == ')').trim();
                if item.is_empty() || item == "*" {
                    continue;
                }
                let (orig, local) = match item.split_once(" as ") {
                    Some((o, l)) => (o.trim(), l.trim()),
                    None => (item, item),
                };
                out.push((local.to_string(), orig.to_string()));
            }
        }
        "import" => {
            let alias = imp.alias.trim();
            if lang == "python" {
                // `import pkg.mod as m` binds `m`; `import pkg.mod` binds
                // `pkg`, which is not the module's own name.
                if !alias.is_empty() && !imp.path.contains(',') {
                    out.push((alias.to_string(), "*".to_string()));
                }
            } else if !alias.is_empty() {
                out.push((alias.to_string(), "*".to_string()));
            }
        }
        _ => {}
    }
    out
}

/// The names an import path is made of, for `detect` patterns: each
/// `/`-separated component whole (`helloworld.grpc.pb.h`) and each segment
/// between `.`, `/`, `:` and `\\` (`GreeterGrpcKt` of
/// `com.x.GreeterGrpcKt.GreeterCoroutineImplBase`).
fn import_segments(path: &str) -> impl Iterator<Item = &str> {
    path.split('/')
        .chain(path.split(['.', '/', ':', '\\']))
        .filter(|s| !s.is_empty())
}

/// Whether `f` declares a class (on a method record or a class record)
/// whose base is written exactly as one of `wanted` that names a
/// namespace with `::` or `\\` (`RubyLLM::Tool`, `\\Laravel\\Mcp\\Server\\Tool`).
/// A bare or dotted name is too common to prove a framework.
fn declares_qualified_base(f: &FileFacts, wanted: &[String]) -> bool {
    let qualified: Vec<&str> = wanted
        .iter()
        .map(|w| w.trim_start_matches(['\\', ':']))
        .filter(|w| w.contains("::") || w.contains('\\'))
        .collect();
    if qualified.is_empty() {
        return false;
    }
    let written = |b: &String| {
        let b = b.trim().trim_start_matches(['\\', ':']);
        qualified.iter().any(|w| w.eq_ignore_ascii_case(b))
    };
    f.definitions
        .iter()
        .any(|d| d.base_types.iter().any(written))
        || f.classes.iter().any(|c| c.base_types.iter().any(written))
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
            .filter(|&(i, c)| matches!(c, '.' | '/' | ':' | '\\') || (c == '@' && i > 0))
            .map(move |(i, _)| &path[..i]),
    )
}

/// The predicate the prefix index replaces. Kept as the definition of
/// what "this import proves this framework" means, and pinned to the
/// index by `prefix_index_agrees_with_import_matches` — an index that
/// silently diverged from it would make rules stop firing with no
/// symptom other than a coverage table quietly claiming less.
fn import_matches(path: &str, prefix: &str) -> bool {
    let p = path.trim().trim_matches(|c| c == '"' || c == '\'');
    if p == prefix {
        return true;
    }
    if let Some(rest) = p.strip_prefix(prefix) {
        // `@` is a version pin (`https://deno.land/x/oak@v12.1.0/mod.ts`,
        // `jsr:@oak/oak@^17`), except in first position, where it opens
        // an npm scope and is part of the name.
        return rest.starts_with(['.', '/', ':', '\\'])
            || (!prefix.is_empty() && rest.starts_with('@'));
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
/// `urls.py`, `routes/web.php`). Suffix match on path segments, so
/// `app/urls.py` matches `urls.py` but `myurls.py` does not. A marker
/// ending in `/` names a directory, which may sit anywhere in the path
/// (`app/controllers/` matches `src/app/controllers/x.rb`). A `*` inside
/// a marker segment matches any run of characters within one segment
/// (`lua/*/init.lua`, `apisix/plugins/*.lua`).
///
/// Every position is tried, not only the last: `lua/` must match
/// `x/lua/oil/init.lua`, whose last `lua` is the file extension.
fn path_matches(path: &str, marker: &str) -> bool {
    let dir = marker.ends_with('/');
    let want: Vec<&str> = marker.split('/').filter(|s| !s.is_empty()).collect();
    if want.is_empty() {
        return false;
    }
    let have: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let window_matches = |start: usize| {
        want.iter()
            .zip(&have[start..])
            .all(|(w, h)| segment_glob_matches(w, h))
    };
    if dir {
        // The directory must hold something: at least one segment after it.
        (0..have.len().saturating_sub(want.len()))
            .any(|start| start + want.len() < have.len() && window_matches(start))
    } else {
        have.len() >= want.len() && window_matches(have.len() - want.len())
    }
}

/// One path segment against one marker segment, `*` matching any run of
/// characters (none included) within the segment.
fn segment_glob_matches(pattern: &str, segment: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == segment;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if segment.len() < first.len() + last.len()
        || !segment.starts_with(first)
        || !segment.ends_with(last)
    {
        return false;
    }
    let mut rest = &segment[first.len()..segment.len() - last.len()];
    for mid in &parts[1..parts.len() - 1] {
        match rest.find(mid) {
            Some(i) => rest = &rest[i + mid.len()..],
            None => return false,
        }
    }
    true
}

/// Whether `file` is one a path-gated rule's method and base-type
/// matchers read ([`rules::method_paths_for`]). A rule with no gate reads
/// every file.
fn file_in_rule_paths(graph: &Graph, rule: &FrameworkRule, file: FileId) -> bool {
    let gate = rules::method_paths_for(&rule.id, &rule.language);
    if gate.is_empty() {
        return true;
    }
    graph.files.get(&file).is_some_and(|f| {
        let path = normalize_path(&f.path.to_string_lossy());
        gate.iter().any(|g| path_matches(&path, g))
    })
}

// ---------------------------------------------------------------------
// Matchers
// ---------------------------------------------------------------------

/// Whether a rule's bare registrars and base types need per-file
/// evidence ([`rules::FILE_SCOPED_EVIDENCE`]). A rule detected by path,
/// by call or by its marker alone has no import to ask for.
///
/// Not a test harness: its base classes are reached through the
/// framework's own subclasses (`django.test.TestCase` is a
/// `unittest.TestCase`) that no import of the tree names, and its cases
/// hang off one harness node whatever the file imports.
fn file_scoped(rule: &FrameworkRule) -> bool {
    rule.kind != TrustKind::Test
        && !rule.detect.is_empty()
        && rules::FILE_SCOPED_EVIDENCE.contains(&rule.language.as_str())
}

/// Whether a base type or annotation is one the language makes visible in
/// every file without an import (`Runnable`, `AutoCloseable`, Rust's
/// `Drop`): [`cgg_core::stdlib::IMPLICIT_IMPORTS`]. Written qualified, it
/// must name the implicit package itself (`java.lang.Runnable`), never a
/// lookalike of another package's.
fn implicitly_imported(language: &str, marker: &str) -> bool {
    let m = marker.trim().trim_start_matches(['@', '#', '[']);
    let m = m.split(['<', '[', '(']).next().unwrap_or(m).trim();
    let last = m.rsplit(['.', ':']).next().unwrap_or(m);
    let qualifier = m
        .strip_suffix(last)
        .unwrap_or("")
        .trim_end_matches(['.', ':']);
    let implicit_qualifier = qualifier.is_empty()
        || matches!(
            qualifier,
            "java.lang" | "kotlin" | "Thread" | "std::prelude"
        )
        || qualifier.starts_with("kotlin.");
    implicit_qualifier && cgg_core::stdlib::implicitly_imported(language, last)
}

/// The name a marker is written through: `app` of `app.task`, `tools` of
/// `tools.BaseTool`, `rig` of `rig::tool::Tool`, the whole of a bare one.
fn marker_head(marker: &str) -> &str {
    let m = marker.trim().trim_start_matches(['@', '#', '[']);
    let m = m.split(['<', '[', '(']).next().unwrap_or(m);
    m.split(['.', ':']).next().unwrap_or(m).trim()
}

/// Whether one of `rule`'s registrars names `context` with its receiver:
/// `server.resource` matches `server.resource` and `app.server.resource`,
/// never a bare `resource` entry.
fn names_receiver(rule: &FrameworkRule, context: &str) -> bool {
    let ctx = context.to_ascii_lowercase();
    rule.registrars.iter().any(|reg| {
        let reg = reg.to_ascii_lowercase();
        reg.contains(['.', ':'])
            && (ctx == reg
                || ctx
                    .strip_suffix(reg.as_str())
                    .is_some_and(|head| head.ends_with(['.', ':'])))
    })
}

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

/// [`contains_ci`], plus naming-convention patterns: `test*` matches a
/// name that starts with `test` and continues (`test_x`, `testFoo`,
/// `test1`), and `*_test` one that ends with `_test` after a non-empty
/// stem. Patterns compare case-sensitively — the case IS the convention
/// (`go test` runs `TestFoo`, never `testFoo`) — and a prefix pattern
/// refuses a continuation that starts with a lowercase letter, so `Test*`
/// does not take `Testing` and `test*` does not take `tests`. An infix
/// pattern `get*R` needs both ends around a non-empty middle, with the
/// prefix's word boundary (`getHomeR`, never `getter` or `getR`).
fn matches_ci_or_pattern(list: &[String], needle: &str) -> bool {
    list.iter().any(|m| name_pattern_matches(m, needle))
}

/// A callable's name against a rule's `methods`: [`name_pattern_matches`],
/// except that an exact name is compared case-sensitively for the rules
/// [`rules::methods_case_sensitive`] lists.
fn method_name_matches(rule: &FrameworkRule, name: &str) -> bool {
    if rules::methods_case_sensitive(&rule.id, &rule.language) {
        return rule.methods.iter().any(|m| {
            if m.contains('*') {
                name_pattern_matches(m, name)
            } else {
                m == name
            }
        });
    }
    matches_ci_or_pattern(&rule.methods, name)
}

/// [`method_name_matches`] for a callable, plus owner-qualified patterns:
/// a method written `owner.name` or `owner:name` (`love.update`,
/// `GM:Think`, `GM:*`) matches a callable whose qualified name ends in
/// that owner (case-sensitive, on a segment boundary) and a name the
/// part after the separator matches. A LÖVE `love.update` and a game
/// object's `Player:update` share a simple name; only the owner tells
/// them apart.
fn callable_matches_methods(
    rule: &FrameworkRule,
    node: &cgg_core::graph::CallableNode,
) -> bool {
    if method_name_matches(rule, &node.simple_name) {
        return true;
    }
    rule.methods
        .iter()
        .any(|m| owner_qualified_matches(m, &node.qualified_name))
}

fn owner_qualified_matches(pattern: &str, qualified_name: &str) -> bool {
    let Some(sep) = pattern.rfind(['.', ':']) else {
        return false;
    };
    let (owner, name) = (&pattern[..sep], &pattern[sep + 1..]);
    let owner = owner.trim_end_matches(':');
    if owner.is_empty() || name.is_empty() {
        return false;
    }
    let Some(qsep) = qualified_name.rfind(['.', ':']) else {
        return false;
    };
    let q_owner = qualified_name[..qsep].trim_end_matches(':');
    let q_name = &qualified_name[qsep + 1..];
    let owner_ok = q_owner == owner
        || q_owner
            .strip_suffix(owner)
            .is_some_and(|head| head.ends_with(['.', ':']));
    owner_ok && name_pattern_matches(name, q_name)
}

pub(crate) fn name_pattern_matches(pattern: &str, name: &str) -> bool {
    // An infix pattern needs both ends and a non-empty middle, and the
    // prefix's word boundary as a prefix pattern does: `sqlite3_*_init`,
    // `HAL_*Callback`, yesod's `get*R` (`getHomeR`, never `getR` or
    // `getterR`). Case-sensitive, like the other patterns.
    if let Some((prefix, suffix)) = pattern.split_once('*')
        && !prefix.is_empty()
        && !suffix.is_empty()
        && !suffix.contains('*')
    {
        let bounded = !prefix.ends_with(|c: char| c.is_ascii_alphanumeric());
        return name
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_suffix(suffix))
            .and_then(|middle| middle.chars().next())
            .is_some_and(|c| bounded || !c.is_ascii_lowercase());
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        // `test_*` already ends at a word boundary; `test*` must not run
        // on into a longer lowercase word.
        let bounded = !prefix.ends_with(|c: char| c.is_ascii_alphanumeric());
        return name
            .strip_prefix(prefix)
            .and_then(|rest| rest.chars().next())
            .is_some_and(|c| bounded || !c.is_ascii_lowercase());
    }
    if let Some(suffix) = pattern.strip_prefix('*') {
        return name.len() > suffix.len() && name.ends_with(suffix);
    }
    pattern.eq_ignore_ascii_case(name)
}

/// **Shape A** — a marker on the definition.
///
/// The highest-evidence shape in the set: the framework's own
/// registration syntax is sitting on the callable. Only the precise
/// route string is uncertain, never whether this is an entry point.
fn match_attributes(
    graph: &Graph,
    rule: &FrameworkRule,
    evidence: &Detection,
    imports: &InTreeImports,
    mock_names: &HashMap<FileId, Vec<String>>,
    out: &mut Vec<FrameworkEntry>,
) {
    if rule.attributes.is_empty() {
        return;
    }
    // A JVM annotation is the framework's only where the file imports it
    // (or writes it qualified): `@Scheduled` over a class of the file's
    // own package is that package's annotation, whatever another file of
    // the tree imports.
    let import_scoped = !rule.detect.is_empty()
        && rules::IMPORT_SCOPED_ATTRIBUTES.contains(&rule.language.as_str());
    for (id, node) in &graph.callables {
        if node.language != rule.language || node.synthetic {
            continue;
        }
        // A script (`build.gradle.kts`) is compiled with its host's
        // default imports, which no line of the file states.
        let script = || {
            graph
                .files
                .get(&node.file)
                .is_some_and(|f| f.path.extension().is_some_and(|e| e == "kts"))
        };
        let file_imports = !import_scoped
            || evidence.file_carries(&rule.id, &rule.language, node.file)
            || script();
        let mocks = mock_names.get(&node.file);
        for attr in &node.attributes {
            let key = attribute_key(attr);
            let verb = attribute_verb(attr);
            // `@patch(...)` / `@mock.patch(...)` replace an object for a
            // test; the `patch` verb of an HTTP rule must not read them.
            if mocks.is_some_and(|m| is_mock_decorator(m, key)) {
                continue;
            }
            if !matches_ci_or_pattern(&rule.attributes, key)
                && !matches_ci_or_pattern(&rule.attributes, verb)
            {
                continue;
            }
            if !file_imports
                && !rule.detect.iter().any(|p| import_matches(key, p))
                && !evidence.carries_for(rule, node.file, marker_head(key), imports)
                && !implicitly_imported(&rule.language, key)
            {
                continue;
            }
            let route = if rule.kind == TrustKind::Tool {
                tool_route(
                    &attribute_keyed_name(attr).unwrap_or_default(),
                    &attribute_positional_string(attr).unwrap_or_default(),
                    &node.simple_name,
                )
            } else {
                match attribute_string_arg(attr) {
                    Some(path) => format!("{verb}(\"{path}\")"),
                    None => String::new(),
                }
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

// ---------------------------------------------------------------------
// Tool names
// ---------------------------------------------------------------------

/// Keys whose value names a tool rather than describing it. Matches
/// `cgg_lang`'s registrar capture, which fills `RefRecord::name_arg`
/// from the same keys.
const TOOL_NAME_KEYS: &[&str] = &["name", "id", "uri"];

/// The identity of a `tool`-kind entry: what the model calls it by.
///
/// A route rule takes the first string argument, which for a tool is as
/// often its description (`#[tool(description = "…")]`, LangChain4j
/// `@Tool("…")`) or a model id (`Agent('openai:gpt-4o', tools=[…])`) as
/// its name. So: a `name`/`id`/`uri` argument first, then a positional
/// string only when it is shaped like a name, then the callable's own
/// name — the default every tool framework uses. Empty (the node falls
/// back to the target's qualified name) when none applies, as for an
/// inline closure whose synthesized name means nothing to a model.
fn tool_route(keyed: &str, positional: &str, own_name: &str) -> String {
    let keyed = keyed.trim();
    if !keyed.is_empty() {
        return keyed.to_string();
    }
    if tool_name_shaped(positional) {
        return positional.trim().to_string();
    }
    own_name.trim().to_string()
}

/// A base-type tool (`class Search(BaseTool)`, `impl Tool for Search`)
/// is named after its type, so every method the rule enters lands on the
/// one node of that tool.
fn tool_owner_route(qualified_name: &str) -> String {
    crate::names::owner_from_qn(qualified_name)
        .map(|o| last_segment(o).to_string())
        .unwrap_or_default()
}

/// Whether a string reads as a tool name or a resource URI rather than
/// prose or a model id: no whitespace, and no `:` unless it is a URI's
/// `://`.
fn tool_name_shaped(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() || s.len() > 128 || s.contains(char::is_whitespace) {
        return false;
    }
    if s.contains("://") {
        return true;
    }
    s.chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/'))
}

fn last_segment(path: &str) -> &str {
    path.rsplit(['.', ':', '\\', '/']).next().unwrap_or(path)
}

/// The text between an attribute's outermost parentheses.
fn attribute_args(attr: &str) -> Option<&str> {
    let open = attr.find('(')?;
    let close = attr.rfind(')')?;
    (close > open).then(|| &attr[open + 1..close])
}

/// `"x"` / `'x'` → `x`; anything else → `None`.
fn quoted(s: &str) -> Option<String> {
    let s = s.trim();
    let s = s.trim_start_matches(['r', 'b', 'f', 'u', '@']);
    let first = s.chars().next()?;
    (matches!(first, '"' | '\'' | '`') && s.len() >= 2 && s.ends_with(first))
        .then(|| s[1..s.len() - 1].to_string())
}

/// Split `key = value` / `key: value` when the key is a bare (or quoted)
/// identifier. A URI or a `::` path is not a key.
fn split_keyed(arg: &str) -> Option<(String, &str)> {
    let at = arg.find(['=', ':'])?;
    let (k, rest) = arg.split_at(at);
    let v = &rest[1..];
    if v.starts_with(['=', ':', '/']) {
        return None;
    }
    let k = k.trim();
    let k = quoted(k).unwrap_or_else(|| k.to_string());
    (!k.is_empty() && k.chars().all(|c| c.is_alphanumeric() || c == '_'))
        .then_some((k, v.trim()))
}

/// The string an attribute passes under a `name`/`id`/`uri` key,
/// directly or inside one options object (`@Tool({ name: 'x' })`).
fn attribute_keyed_name(attr: &str) -> Option<String> {
    fn scan(body: &str, depth: u8) -> Option<String> {
        for arg in split_top_level_args(body) {
            if let Some(inner) = arg.strip_prefix('{').and_then(|a| a.strip_suffix('}')) {
                if depth < 2
                    && let Some(v) = scan(inner, depth + 1)
                {
                    return Some(v);
                }
                continue;
            }
            if let Some((k, v)) = split_keyed(arg)
                && TOOL_NAME_KEYS.iter().any(|n| n.eq_ignore_ascii_case(&k))
                && let Some(v) = quoted(v)
            {
                return Some(v);
            }
        }
        None
    }
    scan(attribute_args(attr)?, 0)
}

/// The first string an attribute passes positionally (Java's implicit
/// `value` counts as positional).
fn attribute_positional_string(attr: &str) -> Option<String> {
    for arg in split_top_level_args(attribute_args(attr)?) {
        let value = match split_keyed(arg) {
            Some((k, v)) if k == "value" => v,
            Some(_) => continue,
            None => arg,
        };
        if let Some(v) = quoted(value) {
            return Some(v);
        }
    }
    None
}

/// Names each Python file binds to a mocking module
/// ([`cgg_core::stdlib::PY_MOCK_MODULES`]): `patch` from `from
/// unittest.mock import patch`, `mock` from `from unittest import mock` or
/// `import mock`, `um` from `import unittest.mock as um`, and
/// `unittest.mock` itself from `import unittest.mock`. Files binding none
/// are absent.
fn python_mock_bindings(facts: &[FileFacts]) -> HashMap<FileId, Vec<String>> {
    let modules = cgg_core::stdlib::PY_MOCK_MODULES;
    let mut out: HashMap<FileId, Vec<String>> = HashMap::new();
    for f in facts.iter().filter(|f| f.language == "python") {
        let mut bound: Vec<String> = Vec::new();
        for imp in &f.imports {
            let path = imp.path.trim();
            if imp.kind == "from-import" {
                for item in imp.alias.split(',') {
                    let item = item.trim();
                    if item.is_empty() {
                        continue;
                    }
                    let (name, local) = match item.split_once(" as ") {
                        Some((n, a)) => (n.trim(), a.trim()),
                        None => (item, item),
                    };
                    // `from unittest.mock import patch` binds a member of
                    // the module; `from unittest import mock` the module.
                    let whole = format!("{path}.{name}");
                    if modules.contains(&path) || modules.contains(&whole.as_str()) {
                        bound.push(local.to_string());
                    }
                }
            } else if imp.kind == "import" {
                if !imp.alias.is_empty() {
                    if modules.contains(&path) {
                        bound.push(imp.alias.trim().to_string());
                    }
                    continue;
                }
                for item in path.split(',') {
                    let item = item.trim();
                    if modules.contains(&item) {
                        bound.push(item.to_string());
                    }
                }
            }
        }
        if !bound.is_empty() {
            out.insert(f.file, bound);
        }
    }
    out
}

/// Whether a decorator key is reached through a mock binding: the key is
/// the binding (`patch`) or continues it (`patch.object`, `mock.patch`).
fn is_mock_decorator(bound: &[String], key: &str) -> bool {
    bound.iter().any(|b| {
        key == b
            || key
                .strip_prefix(b.as_str())
                .is_some_and(|rest| rest.starts_with('.'))
    })
}

/// **Shape A, on the container** — `#[pymethods] impl Foo { … }` exposes
/// every member of the block although no member carries a marker.
fn match_container_attributes(
    graph: &Graph,
    facts: &[FileFacts],
    rule: &FrameworkRule,
    out: &mut Vec<FrameworkEntry>,
) {
    let wanted: Vec<String> = rules::container_attributes_for(&rule.id, &rule.language)
        .iter()
        .map(|a| a.to_string())
        .collect();
    if wanted.is_empty() {
        return;
    }
    // (file, container span, the attribute that matched), in input order.
    let mut spans: HashMap<FileId, Vec<(u32, u32, String)>> = HashMap::new();
    for f in facts.iter().filter(|f| f.language == rule.language) {
        for c in &f.attributed_containers {
            let hit = c.attributes.iter().find(|a| {
                matches_ci_or_pattern(&wanted, attribute_key(a))
                    || matches_ci_or_pattern(&wanted, attribute_verb(a))
            });
            if let Some(a) = hit {
                spans.entry(f.file).or_default().push((
                    c.start_byte,
                    c.end_byte,
                    a.trim().to_string(),
                ));
            }
        }
    }
    if spans.is_empty() {
        return;
    }
    for (id, node) in &graph.callables {
        if node.language != rule.language || node.synthetic {
            continue;
        }
        let Some(list) = spans.get(&node.file) else {
            continue;
        };
        let Some((_, _, attr)) = list
            .iter()
            .find(|(s, e, _)| *s <= node.start_byte && node.end_byte <= *e)
        else {
            continue;
        };
        if node
            .attributes
            .iter()
            .any(|a| rules::container_member_skipped(&rule.id, &rule.language, a))
        {
            continue;
        }
        out.push(FrameworkEntry {
            framework: rule.id.clone(),
            kind: rule.kind,
            shape: EntryShape::Attribute,
            route: String::new(),
            target: *id,
            target_name: node.qualified_name.clone(),
            evidence: format!("member of {attr} block"),
            file: node.file,
            site_line: node.start_line,
            node: rule.node,
        });
    }
}

/// **Shape B, written in markup** — `Click="OnSave"` in the sibling
/// `.xaml` of a code-behind. The handler is the code-behind's own method;
/// the event is the entry's key.
fn match_markup_bindings(
    graph: &Graph,
    facts: &[FileFacts],
    rule: &FrameworkRule,
    out: &mut Vec<FrameworkEntry>,
) {
    let kinds = rules::markup_kinds_for(&rule.id, &rule.language);
    if kinds.is_empty() {
        return;
    }
    let mut wanted: HashMap<(FileId, &str), Vec<&cgg_core::MarkupBinding>> =
        HashMap::new();
    for f in facts.iter().filter(|f| f.language == rule.language) {
        for b in &f.markup_bindings {
            if kinds.contains(&b.kind.as_str()) {
                wanted
                    .entry((f.file, b.handler.as_str()))
                    .or_default()
                    .push(b);
            }
        }
    }
    if wanted.is_empty() {
        return;
    }
    for (id, node) in &graph.callables {
        if node.language != rule.language || node.synthetic {
            continue;
        }
        let Some(list) = wanted.get(&(node.file, node.simple_name.as_str())) else {
            continue;
        };
        for b in list {
            out.push(FrameworkEntry {
                framework: rule.id.clone(),
                kind: rule.kind,
                shape: EntryShape::Registrar,
                route: b.event.clone(),
                target: *id,
                target_name: node.qualified_name.clone(),
                evidence: format!(
                    "{}=\"{}\" in {} markup line {}",
                    b.event, b.handler, b.kind, b.line
                ),
                file: node.file,
                site_line: node.start_line,
                node: rule.node,
            });
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
            let value_refs = VALUE_REF_REGISTRAR_LANGS.contains(&language);
            for r in &f.references {
                if r.receiver_hint != VALUE_REF_HINT
                    && r.receiver_hint != STRING_REF_HINT
                    && !(value_refs && r.value_ref)
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
                // A rule may name the receiver path (`Route::get`,
                // `ipcMain.handle`). Key every receiver-qualified suffix,
                // so `electron.ipcMain.handle` answers to `ipcMain.handle`
                // exactly as `ipcMain.handle` does.
                let full = r.context.to_ascii_lowercase();
                let verb_len = verb.len();
                for (i, _) in full.match_indices(['.', ':']) {
                    let suffix = &full[i + 1..];
                    // The first `:` of a `::` is not a boundary.
                    if suffix.len() > verb_len && !suffix.starts_with(':') {
                        by_verb.entry(suffix.to_string()).or_default().push((f, r));
                    }
                }
                if full.len() > verb_len {
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
            // `Agent:tools` takes only what is passed under `tools`.
            let (reg, slot) = cgg_core::frameworks::split_registrar_slot(reg);
            let Some(hits) = self.by_verb.get(&reg.to_ascii_lowercase()) else {
                continue;
            };
            for (f, r) in hits {
                if slot.is_some_and(|s| r.slot != s) {
                    continue;
                }
                if seen.insert(*r as *const _) {
                    out.push((*f, *r));
                }
            }
        }
        out
    }
}

/// An in-tree type's (full owner path, declared bases, declaring file).
type OwnerBases = (String, Vec<String>, FileId);

/// Declared base types for one language, indexed once.
struct BaseTypeIndex {
    /// (file, start_byte) -> base types, from the facts side.
    bases: HashMap<(FileId, u32), Vec<String>>,
    /// Owner type's last segment -> every in-tree type of that name, as
    /// (full owner path, declared bases, declaring file), in input order.
    /// Keyed by the last segment because a declaration writes
    /// `generic.ObjectListView` for the class defined as `ObjectListView`;
    /// the full path is kept so a qualified base (`ActiveRecord::Base`)
    /// does not walk into an unrelated in-tree type of the same last name
    /// (`CLI::Base < Thor`), and the file so a base matched up the chain
    /// is judged where it is declared (per-file framework evidence).
    by_owner: HashMap<String, Vec<OwnerBases>>,
}

impl BaseTypeIndex {
    fn build(facts: &[FileFacts], language: &str) -> Self {
        let mut bases: HashMap<(FileId, u32), Vec<String>> = HashMap::new();
        let mut by_owner: HashMap<String, Vec<OwnerBases>> = HashMap::new();
        for f in facts {
            if f.language != language {
                continue;
            }
            let php = (language == "php").then(|| PhpNames::of(f));
            let expand = |declared: &[String]| -> Vec<String> {
                match &php {
                    Some(names) => names.expand(declared),
                    None => declared.to_vec(),
                }
            };
            for d in &f.definitions {
                if d.base_types.is_empty() {
                    continue;
                }
                let declared = expand(&d.base_types);
                bases.insert((f.file, d.start_byte), declared.clone());
                if let Some(owner) = crate::names::owner_from_qn(&d.qualified_name) {
                    let full = crate::names::split_last_segment(&d.qualified_name)
                        .map(|(o, _)| o)
                        .unwrap_or(owner);
                    let list = by_owner.entry(owner.to_string()).or_default();
                    if !list.iter().any(|(o, _, _)| o == full) {
                        list.push((full.to_string(), declared, f.file));
                    }
                }
            }
        }
        // Class records, after every method: a class with no method
        // (`class ApplicationTool < RubyLLM::Tool; end`, `class
        // Middle(Base): pass`) carries its bases only here, and the chain
        // through it ended at it. A class some method already speaks for
        // keeps that reading.
        for f in facts {
            if f.language != language {
                continue;
            }
            let php = (language == "php").then(|| PhpNames::of(f));
            for c in &f.classes {
                if c.base_types.is_empty() {
                    continue;
                }
                let owner = c.class_qn.rsplit(['.', ':', '\\']).next().unwrap_or("");
                if owner.is_empty() {
                    continue;
                }
                let list = by_owner.entry(owner.to_string()).or_default();
                if list.iter().any(|(o, _, at)| {
                    o == &c.class_qn || (*at == f.file && last_segment(o) == owner)
                }) {
                    continue;
                }
                let declared = match &php {
                    Some(names) => names.expand(&c.base_types),
                    None => c.base_types.clone(),
                };
                list.push((c.class_qn.clone(), declared, f.file));
            }
        }
        if language == "javascript" || language == "typescript" {
            add_js_import_aliases(facts, language, &mut by_owner);
        }
        Self { bases, by_owner }
    }
}

/// A PHP file's `use` aliases and namespace, to read a base type the
/// way PHP does: `extends BaseTool` under `use NeuronAI\Tools\Tool as
/// BaseTool` is `NeuronAI\Tools\Tool`, and an unqualified `extends Tool`
/// in `namespace App` is `App\Tool`. Written names are kept beside the
/// expanded ones, so a rule naming either form matches.
struct PhpNames {
    /// lowercased alias (or last segment) -> fully qualified path.
    uses: HashMap<String, String>,
    namespace: Option<String>,
}

impl PhpNames {
    fn of(f: &FileFacts) -> Self {
        let mut uses = HashMap::new();
        let mut namespace = None;
        for imp in &f.imports {
            let path = imp.path.trim().trim_start_matches('\\');
            if path.is_empty() {
                continue;
            }
            match imp.kind.as_str() {
                "package" => {
                    namespace.get_or_insert_with(|| path.to_string());
                }
                "use" => {
                    let alias = match imp.alias.trim() {
                        "" => path.rsplit('\\').next().unwrap_or(path),
                        a => a,
                    };
                    uses.insert(alias.to_ascii_lowercase(), path.to_string());
                }
                _ => {}
            }
        }
        Self { uses, namespace }
    }

    fn expand(&self, declared: &[String]) -> Vec<String> {
        let mut out = declared.to_vec();
        for d in declared {
            let full = if let Some(abs) = d.strip_prefix('\\') {
                abs.to_string()
            } else {
                let (head, rest) = match d.split_once('\\') {
                    Some((h, r)) => (h, Some(r)),
                    None => (d.as_str(), None),
                };
                match (self.uses.get(&head.to_ascii_lowercase()), rest) {
                    (Some(path), Some(rest)) => format!("{path}\\{rest}"),
                    (Some(path), None) => path.clone(),
                    (None, _) => match &self.namespace {
                        Some(ns) => format!("{ns}\\{d}"),
                        None => continue,
                    },
                }
            };
            if !out.contains(&full) {
                out.push(full);
            }
        }
        out
    }
}

/// Whether a callable is a public instance member as declared: its
/// normalized visibility where the plugin records one, else (PHP) the
/// modifiers in front of `function`, where none means public. Magic
/// `__x` methods and static members are not, since a host that calls
/// every public member (a Livewire action, a controller endpoint) calls
/// neither.
fn declared_public_instance_member(node: &cgg_core::graph::CallableNode) -> bool {
    if node.simple_name.starts_with("__") {
        return false;
    }
    match node.vis {
        cgg_core::Vis::Public => {}
        cgg_core::Vis::Unknown if node.language == "php" => {}
        _ => return false,
    }
    let head = node
        .signature_hint
        .split("function")
        .next()
        .unwrap_or_default();
    !head
        .split_whitespace()
        .any(|w| matches!(w, "private" | "protected" | "static"))
}

/// A JS/TS class extends the name its file imports, which need not be the
/// name the base class was declared under: `import BaseCommand from
/// './base'` (whose default export is `class BaseDataCommand`) or `import
/// { BaseDataCommand as BC }`. Key the imported class's bases under the
/// local name too, so the base chain walks through the rename. A local
/// name some class of the language already declares keeps its own bases.
fn add_js_import_aliases(
    facts: &[FileFacts],
    language: &str,
    by_owner: &mut HashMap<String, Vec<OwnerBases>>,
) {
    let files: Vec<&FileFacts> =
        facts.iter().filter(|f| f.language == language).collect();
    let by_path: HashMap<std::path::PathBuf, &FileFacts> = files
        .iter()
        .map(|f| (normalize_js_path(&f.path), *f))
        .collect();
    // The class a file exports as `default` (`export default class X`).
    let default_export = |f: &FileFacts| -> Option<String> {
        f.imports
            .iter()
            .filter(|i| i.kind == "local-export")
            .find_map(|i| {
                let (local, exported) = i.alias.split_once(" as ")?;
                (exported.trim() == "default").then(|| local.trim().to_string())
            })
    };
    let mut aliases: Vec<(String, String)> = Vec::new();
    for f in &files {
        for imp in f.imports.iter().filter(|i| i.kind == "from-import") {
            for item in imp.alias.split(',') {
                let Some((orig, local)) = item.trim().split_once(" as ") else {
                    continue;
                };
                let (orig, local) = (orig.trim(), local.trim());
                if orig == local || local.is_empty() {
                    continue;
                }
                let orig = if orig == "default" {
                    let Some(target) = js_module_candidates(&f.path, &imp.path)
                        .and_then(|c| c.iter().find_map(|p| by_path.get(p)))
                    else {
                        continue;
                    };
                    let Some(name) = default_export(target) else {
                        continue;
                    };
                    name
                } else {
                    orig.to_string()
                };
                if orig != local {
                    aliases.push((local.to_string(), orig));
                }
            }
        }
    }
    // Sorted so the first alias of a contested local name is the same
    // whatever order the files came in.
    aliases.sort();
    for (local, orig) in aliases {
        if by_owner.contains_key(&local) {
            continue;
        }
        if let Some(parents) = by_owner.get(&orig).cloned() {
            by_owner.insert(local, parents);
        }
    }
}

fn normalize_js_path(p: &std::path::Path) -> std::path::PathBuf {
    let mut out = std::path::PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The paths a relative JS/TS specifier may name, most literal first: as
/// written, with the `.js` a TypeScript ESM import writes for its `.ts`
/// source swapped back, with a source extension added, or an `index`.
fn js_module_candidates(
    from: &std::path::Path,
    spec: &str,
) -> Option<Vec<std::path::PathBuf>> {
    let spec = spec.trim();
    if !(spec.starts_with("./") || spec.starts_with("../")) {
        return None;
    }
    let base = normalize_js_path(&from.parent()?.join(spec));
    let mut candidates = vec![base.clone()];
    const SOURCE_EXTS: &[&str] = &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"];
    match base.extension().and_then(|e| e.to_str()) {
        Some("js" | "jsx" | "mjs" | "cjs") => {
            for ext in SOURCE_EXTS {
                candidates.push(base.with_extension(ext));
            }
        }
        _ => {
            let s = base.to_string_lossy().to_string();
            for ext in SOURCE_EXTS {
                candidates.push(std::path::PathBuf::from(format!("{s}.{ext}")));
            }
            for ext in SOURCE_EXTS {
                candidates.push(base.join(format!("index.{ext}")));
            }
        }
    }
    Some(candidates)
}

fn match_base_types(
    graph: &Graph,
    idx: &BaseTypeIndex,
    rule: &FrameworkRule,
    evidence: &Detection,
    imports: &InTreeImports,
    out: &mut Vec<FrameworkEntry>,
) {
    if rule.base_types.is_empty() || idx.bases.is_empty() {
        return;
    }
    let bases = &idx.bases;
    let by_owner = &idx.by_owner;
    // Every member of a class carries the class's bases and owner, so
    // the walk's inputs (bases, owner path, file) repeat once per member.
    // In a tree with many same-named types (hundreds of nested `Builder`
    // classes) one walk is wide, and repeating it per member was most of
    // the run. The walk depends on nothing else, so cache it per class.
    let mut walks: HashMap<(&[String], &str, FileId), Option<String>> = HashMap::new();

    for (id, node) in &graph.callables {
        if node.language != rule.language || node.synthetic {
            continue;
        }
        let Some(types) = bases.get(&(node.file, node.start_byte)) else {
            continue;
        };
        if !file_in_rule_paths(graph, rule, node.file) {
            continue;
        }
        // A framework contract is usually inherited, not declared
        // directly: NetBox writes `class CircuitListView(generic.
        // ObjectListView)`, and only three levels up does anything name
        // Django's `View`. Matching the immediate bases alone sees none
        // of a real application's class-based views.
        let scoped = file_scoped(rule);
        let accept = |base: &str, at: FileId| {
            !scoped
                || implicitly_imported(&rule.language, base)
                || rule.detect.iter().any(|p| import_matches(base, p))
                || evidence.carries_for(rule, at, marker_head(base), imports)
        };
        let context = crate::names::split_last_segment(&node.qualified_name)
            .map(|(o, _)| o)
            .unwrap_or("");
        let lexical = rules::LEXICAL_CONSTANT_LANGS.contains(&rule.language.as_str());
        let walk = walks
            .entry((types.as_slice(), context, node.file))
            .or_insert_with(|| {
                find_in_base_chain(
                    types,
                    context,
                    node.file,
                    by_owner,
                    &rule.base_types,
                    lexical,
                    &accept,
                )
            });
        let Some(hit) = walk.clone() else {
            continue;
        };
        // QtTest's base is plain `QObject`: a test class is one only
        // where the harness runs it, a test file or one including QTest.
        if rules::base_types_need_harness_file(&rule.id, &rule.language)
            && graph
                .files
                .get(&node.file)
                .is_none_or(|f| f.test_role.is_none())
            && !evidence.file_carries(&rule.id, &rule.language, node.file)
        {
            continue;
        }
        // `<init>` names the matched class's constructor.
        let constructor = node.kind == CallableKind::Constructor
            && rule.methods.iter().any(|m| m == rules::CONSTRUCTOR_METHOD);
        // A host that calls every public member (a Livewire action, a
        // Nextcloud controller endpoint) takes the public ones on top of
        // the named hooks; with no names listed, the public ones only.
        let named = if rules::public_members_are_entries(&rule.id, &rule.language) {
            (!rule.methods.is_empty() && callable_matches_methods(rule, node))
                || declared_public_instance_member(node)
                || constructor
        } else {
            rule.methods.is_empty() || callable_matches_methods(rule, node) || constructor
        };
        // In the languages of `TEST_CLASS_MEMBERS_ARE_ROOTS`, a test
        // class's other members (helpers, fixtures by another name) are
        // test code its cases reach through `self`/`$this` across the
        // class hierarchy, which those resolvers bind weakly: a test root
        // mark, not an entry of the harness node.
        if !named
            && (rule.kind != TrustKind::Test
                || !rules::TEST_CLASS_MEMBERS_ARE_ROOTS.contains(&rule.language.as_str()))
        {
            continue;
        }
        out.push(FrameworkEntry {
            framework: rule.id.clone(),
            kind: rule.kind,
            shape: EntryShape::BaseType,
            route: if rule.kind == TrustKind::Tool {
                tool_owner_route(&node.qualified_name)
            } else {
                String::new()
            },
            target: *id,
            target_name: node.qualified_name.clone(),
            evidence: if named {
                format!("declares `{hit}`")
            } else {
                format!("member of a `{hit}` test class")
            },
            file: node.file,
            site_line: node.start_line,
            node: rule.node && named,
        });
    }
}

/// Walk a type's inheritance chain looking for one the rule names.
///
/// Bounded by a visited set and a depth cap: a base list read from
/// syntax can be cyclic (two files each naming the other's type) and a
/// deep hierarchy is not more evidence than a shallow one.
///
/// `context` is the full owner path of the type declaring `direct`; a
/// bare base name several in-tree types share is read as the one in the
/// nearest enclosing namespace where constants resolve lexically.
///
/// `accept(base, file)` is asked of every match, with the file whose
/// declaration names that base (`file` for `direct`): a base is the
/// framework's only where that file carries its evidence, and a refused
/// match lets the walk go on to a deeper one.
fn find_in_base_chain(
    direct: &[String],
    context: &str,
    file: FileId,
    by_owner: &HashMap<String, Vec<OwnerBases>>,
    wanted: &[String],
    lexical: bool,
    accept: &dyn Fn(&str, FileId) -> bool,
) -> Option<String> {
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    // Where constants do not resolve lexically, a base's in-tree
    // candidates depend on its name alone, never on the type that wrote
    // it. A name many types share as their base (1,406 Django
    // `class Migration(migrations.Migration)` in one tree, each read as a
    // candidate for the others) would otherwise expand once per writer,
    // squaring the walk. Expand each name once, and skip a name already
    // known not to match: what is left of it is its expansion. A name
    // that matches is still asked of `accept` for every declaring file.
    // Keyed by what the expansion reads, not the spelling: Smithy's
    // hundreds of `AbstractTraitBuilder<FooTrait, Builder>` bases differ
    // only in their generic arguments and expand to the same types.
    let mut expanded: HashSet<(bool, String, Option<String>)> = HashSet::new();
    let mut unmatched: HashSet<String> = HashSet::new();
    let mut frontier: Vec<(String, String, FileId)> = direct
        .iter()
        .map(|t| (t.clone(), context.to_string(), file))
        .collect();
    for _ in 0..8 {
        if frontier.is_empty() {
            break;
        }
        let mut next: Vec<(String, String, FileId)> = Vec::new();
        for (t, ctx, declared_in) in frontier {
            if !lexical && unmatched.contains(&t) {
                continue;
            }
            if !seen.insert((t.clone(), ctx.clone())) {
                continue;
            }
            if base_type_matches(wanted, &t) {
                if accept(&t, declared_in) {
                    return Some(t);
                }
            } else if !lexical {
                unmatched.insert(t.clone());
            }
            if !lexical && !expanded.insert(expansion_key(&t)) {
                continue;
            }
            for (owner, parents, at) in in_tree_bases(&t, &ctx, by_owner, lexical) {
                next.extend(parents.iter().map(|p| (p.clone(), owner.to_string(), at)));
            }
        }
        frontier = next;
    }
    None
}

/// What [`in_tree_bases`] reads of a base written `t` where constants do
/// not resolve lexically: its last segment, and whether, and under which
/// parent, a `::`/`\\` path qualifies it. Two spellings with one key
/// expand to the same types.
fn expansion_key(t: &str) -> (bool, String, Option<String>) {
    let segs = type_segments(t);
    let qualified = t.contains("::") || t.contains('\\');
    let last = segs.last().map(|s| s.to_string()).unwrap_or_default();
    let parent = qualified
        .then(|| segs.len().checked_sub(2).map(|i| segs[i].to_string()))
        .flatten();
    (qualified, last, parent)
}

/// Path segments of a type or owner name, generic arguments dropped:
/// `Mastodon::CLI::Base` -> `[Mastodon, CLI, Base]`, `\Livewire\Component`
/// -> `[Livewire, Component]`.
fn type_segments(t: &str) -> Vec<&str> {
    let bare = t.split(['<', '[']).next().unwrap_or(t).trim();
    bare.split(['.', ':', '\\'])
        .filter(|s| !s.is_empty())
        .collect()
}

/// The in-tree types a base written `t` (inside type `ctx`) may name,
/// looked up by its last segment.
///
/// A base written with a namespace path (`ActiveRecord::Base`) refuses a
/// candidate whose own enclosing namespace contradicts the qualifier
/// (`Mastodon::CLI::Base`): the qualifier names where the type lives. A
/// candidate with no namespace of its own (a PHP or Ruby class declared
/// at top level) is not contradicted. Where constants resolve lexically
/// (`lexical`), a bare base reads as the type in the nearest enclosing
/// namespace of `ctx`; elsewhere as every in-tree type of the name.
fn in_tree_bases<'a>(
    t: &str,
    ctx: &str,
    by_owner: &'a HashMap<String, Vec<OwnerBases>>,
    lexical: bool,
) -> Vec<(&'a str, &'a [String], FileId)> {
    let segs = type_segments(t);
    let Some(key) = segs.last() else {
        return Vec::new();
    };
    let Some(cands) = by_owner.get(*key) else {
        return Vec::new();
    };
    // Only a `::`/`\\` path names a declared namespace (Ruby, PHP, C++,
    // Rust). A dotted qualifier is a module or import alias (`generic.
    // ObjectEditView` through a package re-export), which says nothing
    // about the file the class is declared in.
    let written_parent = (t.contains("::") || t.contains('\\'))
        .then(|| segs.len().checked_sub(2).map(|i| segs[i]))
        .flatten();
    let namespace_of = |owner: &'a str| -> Option<Vec<&'a str>> {
        // Owners read from a trait-impl wrapper or a generic carry no
        // namespace this comparison can trust.
        if owner.contains(['<', ' ']) {
            return None;
        }
        let s = type_segments(owner);
        (s.len() >= 2).then(|| s[..s.len() - 1].to_vec())
    };
    let mut kept: Vec<(&'a str, &'a [String], FileId)> = cands
        .iter()
        .filter(
            |(owner, _, _)| match (written_parent, namespace_of(owner)) {
                (Some(wp), Some(ns)) => ns.last() == Some(&wp),
                _ => true,
            },
        )
        .map(|(o, b, f)| (o.as_str(), b.as_slice(), *f))
        .collect();
    let ctx_segs = type_segments(ctx);
    // Where a bare constant resolves lexically (Ruby: the enclosing
    // modules, then the top level), a type in an unrelated namespace is
    // out of reach: `Base` inside `Chewy::Strategy` is not
    // `Mastodon::CLI::Base`.
    if lexical && written_parent.is_none() {
        kept.retain(|(o, _, _)| {
            namespace_of(o).is_none_or(|ns| {
                ns.len() < ctx_segs.len() && ns.iter().zip(&ctx_segs).all(|(a, b)| a == b)
            })
        });
    }
    // Lexically, the innermost enclosing namespace wins. Elsewhere a bare
    // name several in-tree types share walks each: which one it means
    // can turn on an import or a `use … as` alias this index does not
    // read (Symfony's `Tests\HttpClientTestCase extends
    // BaseHttpClientTestCase`), and walking only one lost the chain.
    if lexical && written_parent.is_none() && kept.len() > 1 {
        let shared = |owner: &'a str| -> usize {
            namespace_of(owner).map_or(0, |ns| {
                ns.iter().zip(&ctx_segs).take_while(|(a, b)| a == b).count()
            })
        };
        let best = kept.iter().map(|(o, _, _)| shared(o)).max().unwrap_or(0);
        if best > 0 {
            kept.retain(|(o, _, _)| shared(o) == best);
        }
    }
    kept
}

/// A declared base against a rule's `base_types`: exact names compare
/// case-insensitively, and naming-convention patterns (`*Servicer`,
/// `Unimplemented*`) match as they do for `methods` — the generated
/// gRPC base of every service is named after the service, so no exact
/// list can name them.
fn base_type_matches(wanted: &[String], declared: &str) -> bool {
    // PHP writes a fully qualified base with a leading `\`.
    let d = declared.trim().trim_start_matches('\\');
    if matches_ci_or_pattern(wanted, d) {
        return true;
    }
    if wanted.iter().any(|w| nested_base_matches(w, d)) {
        return true;
    }
    // `nn.Module` should match a rule written as `Module`, and
    // `Sidekiq::Job` a rule written as `Sidekiq::Job` — compare the last
    // segment too, and strip generic arguments (`IConsumer<OrderPlaced>`).
    // A pattern (`I*` for godot-rust's engine classes, `*::Service` for a
    // protoc-generated gRPC base) is compared against the whole written
    // name and its last segment, so `*::Service` needs the qualifier and
    // never takes a bare `Service`.
    let bare = d.split(['<', '[']).next().unwrap_or(d).trim();
    if matches_ci_or_pattern(wanted, bare) {
        return true;
    }
    // The last segment also takes naming patterns: a generated gRPC base
    // is named per service (`QueryGrpc.QueryImplBase`), so a rule can only
    // write it as `*ImplBase`.
    let last = bare.rsplit(['.', ':', '\\']).next().unwrap_or(bare);
    matches_ci_or_pattern(wanted, last)
}

/// A `$.$Suffix` base pattern: the last two segments of `declared`
/// (generic arguments stripped) are an outer type and a type nested in it
/// whose name is the outer one's plus `Suffix`. protoc's C# output nests a
/// service's base class in a class of the service's name, so every gRPC
/// service implementation derives from `Greeter.GreeterBase`; neither name
/// is fixed, but the relation between them is, and `*Base` alone would
/// take every base class named so.
fn nested_base_matches(pattern: &str, declared: &str) -> bool {
    let Some(suffix) = pattern.strip_prefix("$.$") else {
        return false;
    };
    let bare = declared.split(['<', '[']).next().unwrap_or(declared).trim();
    let mut segs = bare.rsplit(['.', ':']).filter(|s| !s.is_empty());
    let (Some(inner), Some(outer)) = (segs.next(), segs.next()) else {
        return false;
    };
    inner.strip_suffix(suffix) == Some(outer)
}

/// Languages whose [`cgg_core::RefRecord::value_ref`] sites carry the
/// registrar context of the call they are an argument of (Rust: a system
/// in a tuple, `add_systems(Update, (a, b))`, or a combinator's receiver,
/// `a.run_if(c)`), so a registrar rule reads them like a bare argument.
const VALUE_REF_REGISTRAR_LANGS: &[&str] = &["rust"];

/// Languages whose callable's qualified name is its attribute path
/// (Nix: `nodes = { machine = …; }` is `nodes.machine`), so a rule's
/// method pattern may name the path (`nodes.*`) as well as the binding.
const ATTR_PATH_QN_LANGS: &[&str] = &["nix"];

/// A method-name rule with no base type — the structural-typing escape
/// hatch.
///
/// Go interfaces are satisfied implicitly: nothing in the source says
/// `Handler implements http.Handler`, so there is no declaration to
/// match and the method name is the only signal available. This is
/// weaker evidence than every other shape, which is why it fires only
/// when the framework's import is present *and* the rule declares no
/// base types at all.
fn match_methods(
    graph: &Graph,
    rule: &FrameworkRule,
    evidence: &Detection,
    out: &mut Vec<FrameworkEntry>,
) {
    if rule.methods.is_empty() || !rule.base_types.is_empty() {
        return;
    }
    for (id, node) in &graph.callables {
        if node.language != rule.language || node.synthetic {
            continue;
        }
        // A test harness collects by name only where it looks: `go test`
        // reads `_test.go` files, EUnit the modules that include it. A
        // name alone, in a file that neither is a test file nor imports
        // the harness, is a coincidence (`TestServer` in a helper
        // package), so the name pattern applies only in those files.
        let in_test_file = graph
            .files
            .get(&node.file)
            .is_some_and(|f| f.test_role.is_some());
        let harness_file =
            in_test_file || evidence.file_carries(&rule.id, &rule.language, node.file);
        if rule.kind == TrustKind::Test && !harness_file {
            continue;
        }
        if !file_in_rule_paths(graph, rule, node.file) {
            continue;
        }
        let path_match = || {
            ATTR_PATH_QN_LANGS.contains(&node.language.as_str())
                && method_name_matches(rule, &node.qualified_name)
        };
        if !(callable_matches_methods(rule, node) || path_match()) {
            continue;
        }
        // Arduino's core calls the sketch's free `setup()`/`loop()`, never
        // a class's `GPS::setup()`.
        if rules::methods_are_free_functions(&rule.id, &rule.language)
            && (node.kind != CallableKind::Function || node.qualified_name.contains("::"))
        {
            continue;
        }
        let route = match rules::path_route_dir(&rule.id, &rule.language) {
            Some(dir) => {
                let Some(route) = graph
                    .files
                    .get(&node.file)
                    .and_then(|f| file_route(&f.path, dir))
                else {
                    continue;
                };
                route
            }
            None => String::new(),
        };
        out.push(FrameworkEntry {
            framework: rule.id.clone(),
            kind: rule.kind,
            shape: EntryShape::BaseType,
            route: if rule.kind == TrustKind::Tool && route.is_empty() {
                tool_owner_route(&node.qualified_name)
            } else {
                route
            },
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

/// The route a file serves under a file-routed framework's `dir`: its
/// path below the last `dir` segment, extension dropped, an `index` file
/// standing for its directory. `None` for a file outside `dir`.
fn file_route(path: &std::path::Path, dir: &str) -> Option<String> {
    let segs: Vec<String> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    let at = segs.iter().rposition(|s| s == dir)?;
    let mut rest: Vec<String> = segs[at + 1..].to_vec();
    let last = rest.pop()?;
    let stem = last.rsplit_once('.').map_or(last.as_str(), |(s, _)| s);
    if stem != "index" {
        rest.push(stem.to_string());
    }
    Some(format!("/{}", rest.join("/")))
}

/// One argument list of an attribute, read for the members it names.
#[derive(Debug, Default, PartialEq)]
struct NamedMembers {
    /// `typeof(T)` / `T.class`, last segment: the owner of the members.
    owner_type: Option<String>,
    /// `(qualifier, member)` per string literal or `nameof(…)`.
    members: Vec<(Option<String>, String)>,
}

/// Split `s` at top-level commas, ignoring commas inside brackets and
/// string literals.
fn split_top_level_args(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start, mut quote): (i32, usize, Option<char>) = (0, 0, None);
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth -= 1,
            ',' if depth == 0 => {
                out.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = s[start..].trim();
    if !last.is_empty() {
        out.push(last);
    }
    out
}

/// The type named by a `typeof(T)` or `T.class` argument, last segment.
fn type_literal(arg: &str) -> Option<String> {
    let a = arg.trim();
    let inner = if let Some(rest) = a.strip_prefix("typeof(") {
        rest.strip_suffix(')')?
    } else {
        a.strip_suffix(".class")
            .or_else(|| a.strip_suffix("::class"))?
    };
    let bare = inner.split('<').next().unwrap_or(inner).trim();
    let last = bare.rsplit(['.', ':']).next().unwrap_or(bare).trim();
    (!last.is_empty()).then(|| last.to_string())
}

/// Members named by one positional argument: a string literal, a
/// `nameof(…)`, or an array initializer of either.
fn members_in(arg: &str, out: &mut Vec<(Option<String>, String)>) {
    let a = arg.trim();
    if let Some(inner) = a.strip_prefix('{').and_then(|r| r.strip_suffix('}')) {
        for el in split_top_level_args(inner) {
            members_in(el, out);
        }
        return;
    }
    if let Some(inner) = a.strip_prefix("nameof(").and_then(|r| r.strip_suffix(')')) {
        let mut parts = inner.trim().rsplitn(2, '.');
        let member = parts.next().unwrap_or("").trim();
        let qual = parts
            .next()
            .map(|q| q.rsplit('.').next().unwrap_or(q).trim().to_string());
        if is_identifierish(member) {
            out.push((qual, member.to_string()));
        }
        return;
    }
    if (a.starts_with('"') && a.ends_with('"') && a.len() >= 2)
        || (a.starts_with('\'') && a.ends_with('\'') && a.len() >= 2)
    {
        let body = &a[1..a.len() - 1];
        // `com.x.Shared#factory(java.lang.String)` -> (Shared, factory).
        let body = body.split('(').next().unwrap_or(body).trim();
        let (qual, member) = match body.rsplit_once('#') {
            Some((q, m)) => (
                Some(q.rsplit(['.', '$']).next().unwrap_or(q).to_string()),
                m,
            ),
            None => (None, body),
        };
        if is_identifierish(member) {
            out.push((qual, member.to_string()));
        }
    }
}

/// Read an attribute's arguments for the members and type they name.
///
/// Named arguments contribute only a type (`MemberType = typeof(T)`),
/// except Java's implicit `value` element, which is the positional one,
/// and the source's own `named` elements (`fallbackMethod = "m"`), whose
/// string names a member — owned by the type the paired class element
/// names, if the attribute writes one. Of the positional arguments, type
/// literals give the owner and the FIRST other one names the members —
/// later positionals are parameters handed to the source
/// (`[MemberData(nameof(D), 3)]`), never names — unless the source has
/// named elements, which then are its only member names.
fn named_members(attr: &str, named: &[rules::NamedElement]) -> NamedMembers {
    let mut out = NamedMembers::default();
    let Some(open) = attr.find('(') else {
        return out;
    };
    let Some(close) = attr.rfind(')') else {
        return out;
    };
    if close <= open {
        return out;
    }
    let args: Vec<(Option<&str>, &str)> = split_top_level_args(&attr[open + 1..close])
        .into_iter()
        .map(|arg| match arg.split_once('=') {
            Some((n, v))
                if is_identifierish(n.trim())
                    && !v.starts_with('=')
                    && !n.contains(['"', '\'', '(']) =>
            {
                (Some(n.trim()), v.trim())
            }
            _ => (None, arg),
        })
        .collect();
    let class_of = |element: &str| -> Option<String> {
        if element.is_empty() {
            return None;
        }
        args.iter()
            .find(|(n, _)| *n == Some(element))
            .and_then(|(_, v)| type_literal(v.trim_matches(['{', '}', '[', ']', ' '])))
    };
    let mut member_arg_seen = false;
    for (name, value) in &args {
        if let Some(el) = name.and_then(|n| named.iter().find(|e| e.element == n)) {
            let mut found = Vec::new();
            members_in(value, &mut found);
            let owner = class_of(el.class_element);
            out.members.extend(
                found
                    .into_iter()
                    .map(|(qual, member)| (qual.or_else(|| owner.clone()), member)),
            );
            continue;
        }
        if let Some(t) = type_literal(value) {
            let paired = name.is_some_and(|n| named.iter().any(|e| e.class_element == n));
            if !paired {
                out.owner_type.get_or_insert(t);
            }
            continue;
        }
        let positional = name.is_none_or(|n| n == "value");
        if positional && !member_arg_seen && named.is_empty() {
            member_arg_seen = true;
            members_in(value, &mut out.members);
        }
    }
    out
}

/// **Shape E, attribute form** — a test-data attribute names the member
/// the runner calls.
///
/// `@MethodSource("words")` sits on a test, but what it makes live is
/// `words`, which no call reaches. Each named member is looked up on the
/// owner the attribute names, or on the annotated method's own class;
/// a definition in the annotated method's file wins, and otherwise only
/// a unique one is taken. Marks a root only — never an edge.
fn match_member_sources(
    graph: &Graph,
    index: &NameIndex,
    rule: &FrameworkRule,
    out: &mut Vec<FrameworkEntry>,
) {
    let sources = rules::member_sources_for(&rule.id, &rule.language);
    if sources.is_empty() {
        return;
    }
    for (id, node) in &graph.callables {
        if node.language != rule.language || node.synthetic {
            continue;
        }
        for attr in &node.attributes {
            let key = attribute_key(attr);
            let verb = attribute_verb(attr);
            let Some(src) = sources.iter().find(|s| {
                s.attribute.eq_ignore_ascii_case(key)
                    || s.attribute.eq_ignore_ascii_case(verb)
            }) else {
                continue;
            };
            let own_owner = crate::names::owner_from_qn(&node.qualified_name);
            let named = named_members(attr, src.named);
            let mut wanted: Vec<(Option<String>, String)> = Vec::new();
            for (qual, member) in &named.members {
                let owner = qual
                    .clone()
                    .or_else(|| named.owner_type.clone())
                    .or_else(|| own_owner.map(str::to_string));
                wanted.push((owner, member.clone()));
            }
            if named.members.is_empty() {
                if let Some(t) = &named.owner_type {
                    for m in src.type_members {
                        wanted.push((Some(t.clone()), (*m).to_string()));
                    }
                } else if src.default_self {
                    wanted
                        .push((own_owner.map(str::to_string), node.simple_name.clone()));
                }
            }
            for (owner, member) in wanted {
                let Some(owner) = owner else { continue };
                for target in index.owner_members(graph, &owner, &member, node.file) {
                    if target == *id {
                        continue;
                    }
                    let Some(t) = graph.callables.get(&target) else {
                        continue;
                    };
                    out.push(FrameworkEntry {
                        framework: rule.id.clone(),
                        kind: rule.kind,
                        shape: EntryShape::StringTarget,
                        route: String::new(),
                        target,
                        target_name: t.qualified_name.clone(),
                        evidence: format!(
                            "named by `{}` on `{}`",
                            attr.trim(),
                            node.qualified_name
                        ),
                        file: node.file,
                        site_line: node.start_line,
                        node: rule.node,
                    });
                }
            }
        }
    }
}

/// The parameter type names of a signature, last segment, generics and
/// array brackets stripped: `void f(final java.io.ObjectInputStream in)`
/// -> `["ObjectInputStream"]`. `None` when no parameter list is found.
fn param_types(signature: &str, name: &str) -> Option<Vec<String>> {
    let start = signature.find(&format!("{name}("))? + name.len() + 1;
    let mut depth = 1i32;
    let mut end = None;
    for (i, c) in signature[start..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(start + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let inner = &signature[start..end?];
    let mut out = Vec::new();
    for p in split_top_level_args(inner) {
        let words: Vec<&str> = p
            .split_whitespace()
            .filter(|w| !w.starts_with('@') && *w != "final")
            .collect();
        // Type then name; a lone word is a type with the name elided.
        let ty = match words.as_slice() {
            [] => continue,
            [t] => *t,
            [.., t, _name] => *t,
        };
        let bare = ty.split(['<', '[']).next().unwrap_or(ty);
        let bare = bare.trim_end_matches("...");
        out.push(bare.rsplit('.').next().unwrap_or(bare).to_string());
    }
    Some(out)
}

/// The parameter TYPES of `name`'s signature, last segment, generics
/// stripped, for both parameter syntaxes the .NET languages write:
/// type-first in a parenthesised list (`string F(string i, ILambdaContext
/// c)`, C#) and name-first, curried or tupled, before the `=` (`let f (r:
/// Req) (_c: ILambdaContext) =`, F#). A parameter with no written type is
/// recorded as an empty string. `None` when `name` is not in the
/// signature.
fn declared_param_types(signature: &str, name: &str) -> Option<Vec<String>> {
    let at = signature.match_indices(name).map(|(i, _)| i).find(|&i| {
        let before = signature[..i].chars().next_back();
        let after = signature[i + name.len()..].chars().next();
        !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
            && !after.is_some_and(|c| c.is_alphanumeric() || c == '_')
    })?;
    let mut rest = signature[at + name.len()..].trim_start();
    // Generic parameters of the method itself: `F<T>(…)`.
    if rest.starts_with('<') {
        let mut depth = 0i32;
        let end = rest.char_indices().find(|&(_, c)| {
            match c {
                '<' => depth += 1,
                '>' => depth -= 1,
                _ => {}
            }
            depth == 0
        })?;
        rest = rest[end.0 + 1..].trim_start();
    }
    let bare_type = |t: &str| -> String {
        let t = t.trim();
        let t = t.split(['<', '[']).next().unwrap_or(t).trim();
        t.rsplit('.').next().unwrap_or(t).to_string()
    };
    // The parenthesised groups at depth 0 up to `=` (or the end).
    let mut groups: Vec<&str> = Vec::new();
    let mut depth = 0i32;
    let mut open = 0usize;
    for (i, c) in rest.char_indices() {
        match c {
            '(' => {
                if depth == 0 {
                    open = i + 1;
                }
                depth += 1;
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    groups.push(&rest[open..i]);
                }
            }
            '=' | '{' if depth == 0 => break,
            _ => {}
        }
    }
    let mut out = Vec::new();
    for g in groups {
        for p in split_params(g) {
            let p = p.trim();
            if p.is_empty() {
                continue;
            }
            // Name-first (`x: T`), else type-first (`T x`, a lone word
            // being a type with the name elided). Attributes go first.
            let p = p
                .split_whitespace()
                .filter(|w| !w.starts_with('[') && !w.starts_with('@'))
                .collect::<Vec<_>>()
                .join(" ");
            if let Some((_, ty)) = p.split_once(':') {
                out.push(bare_type(ty));
                continue;
            }
            let words: Vec<&str> = p
                .split_whitespace()
                .filter(|w| {
                    !matches!(*w, "this" | "ref" | "in" | "out" | "params" | "scoped")
                })
                .collect();
            match words.as_slice() {
                [] => {}
                [t] => out.push(bare_type(t)),
                [ty @ .., _name] => out.push(bare_type(&ty.join(" "))),
            }
        }
    }
    Some(out)
}

/// Split a parameter list at top-level commas: brackets and generic
/// arguments (`Dictionary<string, int>`) nest, an F# arrow's `>` does not.
fn split_params(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start, mut prev) = (0i32, 0usize, ' ');
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '{' | '<' => depth += 1,
            '>' if prev == '-' => {}
            ')' | ']' | '}' | '>' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        prev = c;
    }
    out.push(&s[start..]);
    out
}

/// **Shape D, parameter-type form** — a runtime-invoked callable known by
/// the type of its last parameter (`rules::parameter_types_for`), not by
/// its name: an AWS Lambda .NET handler named only in deployment config.
fn match_parameter_types(
    graph: &Graph,
    rule: &FrameworkRule,
    out: &mut Vec<FrameworkEntry>,
) {
    let shapes = rules::parameter_types_for(&rule.id, &rule.language);
    if shapes.is_empty() {
        return;
    }
    for (id, node) in &graph.callables {
        if node.language != rule.language || node.synthetic {
            continue;
        }
        let Some(params) = declared_param_types(&node.signature_hint, &node.simple_name)
        else {
            continue;
        };
        let Some(last) = params.last() else {
            continue;
        };
        let Some(shape) = shapes
            .iter()
            .find(|s| s.type_name == last && params.len() <= s.max_params)
        else {
            continue;
        };
        out.push(FrameworkEntry {
            framework: rule.id.clone(),
            kind: rule.kind,
            shape: EntryShape::BaseType,
            route: String::new(),
            target: *id,
            target_name: node.qualified_name.clone(),
            evidence: format!("takes the runtime's `{}` last", shape.type_name),
            file: node.file,
            site_line: node.start_line,
            node: rule.node,
        });
    }
}

/// **Shape D, signature form** — a runtime hook found by exact name and
/// parameter list (Java serialization's `readObject(ObjectInputStream)`).
/// A same-named method with any other parameter list is not one.
fn match_signature_methods(
    graph: &Graph,
    rule: &FrameworkRule,
    out: &mut Vec<FrameworkEntry>,
) {
    let hooks = rules::signature_methods_for(&rule.id, &rule.language);
    if hooks.is_empty() {
        return;
    }
    for (id, node) in &graph.callables {
        if node.language != rule.language || node.synthetic {
            continue;
        }
        let Some(hook) = hooks.iter().find(|h| h.name == node.simple_name) else {
            continue;
        };
        let Some(params) = param_types(&node.signature_hint, hook.name) else {
            continue;
        };
        if params.len() != hook.params.len()
            || params.iter().zip(hook.params).any(|(a, b)| a != b)
        {
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
                "`{}({})` is a runtime hook",
                hook.name,
                hook.params.join(", ")
            ),
            file: node.file,
            site_line: node.start_line,
            node: rule.node,
        });
    }
}

/// Prefer a value-ref handler on the same type as the registrar call.
///
/// `self.bind(on_x=self.handler)` is looked up by bare name first, and
/// that lookup gives up (or picks the first same-file hit) when several
/// widgets define `on_mouse_pos`. The bind site sits inside `__init__`
/// (or another method of the widget), so the enclosing callable's owner
/// is the class that meant this handler.
fn handler_on_enclosing_owner(
    graph: &Graph,
    index: &NameIndex,
    file: FileId,
    byte: u32,
    name: &str,
) -> Option<CallableId> {
    let owner = enclosing_owner_name(graph, file, byte)?;
    index.by_owner_method(Some(owner), name, file)
}

fn enclosing_owner_name(graph: &Graph, file: FileId, byte: u32) -> Option<&str> {
    let mut best: Option<(&cgg_core::graph::CallableNode, u32)> = None;
    for c in graph.callables.values() {
        if c.file != file || c.synthetic || c.qualified_name.starts_with('<') {
            continue;
        }
        if c.start_byte > byte || c.end_byte < byte {
            continue;
        }
        let span = c.end_byte.saturating_sub(c.start_byte);
        match best {
            Some((_, sp)) if sp <= span => {}
            _ => best = Some((c, span)),
        }
    }
    crate::names::owner_from_qn(&best?.0.qualified_name)
}

/// **Shapes B, C and E** — the handler sits in argument position.
///
/// All three arrive as `RefRecord`s carrying a `context` (the registrar
/// call) and a `route` (its first string literal). What differs is only
/// how the target is named: by identifier (B), by the synthesized name
/// of an inline closure (C), or by a string that has to be decoded (E).
// One over clippy's argument threshold: the harness choice joined the
// shared read-only indexes this matcher already took.
#[allow(clippy::too_many_arguments)]
fn match_registrars(
    graph: &Graph,
    facts: &[FileFacts],
    regs: &RegistrarIndex<'_>,
    index: &NameIndex,
    rule: &FrameworkRule,
    called_in_file: &HashSet<CallableId>,
    harness: &HarnessChoice,
    evidence: &Detection,
    imports: &InTreeImports,
    out: &mut Vec<FrameworkEntry>,
) {
    if rule.registrars.is_empty() {
        return;
    }
    let is_harness = rule.kind == TrustKind::Test;

    // Walk only the references whose verb some registrar of THIS rule
    // names, instead of every reference in the tree once per rule.
    // Measured on netbox: the old form was 5.3s of a 6.2s framework
    // phase, because seven active rules each re-scanned ~1,300 files
    // worth of references to reject almost all of them.
    for (f, r) in regs.candidates(rule) {
        // A test file's `it(…)` belongs to one harness, not to every
        // harness that shares the verb.
        if is_harness && !harness.allows(rule, f) {
            continue;
        }
        {
            let is_value = r.receiver_hint == VALUE_REF_HINT || r.value_ref;
            let is_string = r.receiver_hint == STRING_REF_HINT;
            let verb = r
                .context
                .rsplit(['.', ':', '-', '>'])
                .next()
                .unwrap_or(&r.context);
            // A hyphenated verb the rule names whole (re-frame's
            // `reg-event-db`) is that verb, not its last word.
            let tail = r
                .context
                .rsplit(['.', ':', '>'])
                .next()
                .unwrap_or(&r.context);
            let verb = if tail.contains('-') && contains_ci(&rule.registrars, tail) {
                tail
            } else {
                verb
            };
            // An ambiguous verb needs corroboration — see AMBIGUOUS_VERBS.
            let receiverless = r.context.eq_ignore_ascii_case(verb);
            // A bare registrar is a name the file imports: `tool({…})` is
            // the framework's only in a file carrying its evidence. (A
            // harness's block verbs are globals; `HarnessChoice` has
            // already decided whose they are.)
            if receiverless
                && !is_harness
                && file_scoped(rule)
                && !evidence.carries_for(rule, f.file, verb, imports)
            {
                continue;
            }
            // Some rules share their receiver-qualified verbs with a
            // sibling library: those count only in a file importing the
            // rule's own (`rules::registrars_need_file_evidence`).
            if !receiverless
                && rules::registrars_need_file_evidence(&rule.id, &rule.language)
                && !evidence.file_carries(&rule.id, &rule.language, f.file)
            {
                continue;
            }
            // A rule naming the receiver (`server.resource`,
            // `ipcMain.handle`) has had it confirmed: that is the
            // corroboration, not the route.
            if AMBIGUOUS_VERBS.iter().any(|v| v.eq_ignore_ascii_case(verb))
                && r.route.is_empty()
                && !receiverless
                && !names_receiver(rule, &r.context)
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
                        route: if rule.kind == TrustKind::Tool {
                            tool_route(&r.name_arg, &r.route, last_segment(&r.name))
                        } else if r.route.is_empty() {
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
                // `self.bind(on_x=self.handler)`: prefer the handler on
                // the same class as the bind() call, but only when the
                // rule opts in — the lookup leaks into every language
                // otherwise (in Clojure the "enclosing owner" is the
                // namespace, so `(get m id)` inside a defn would bind
                // to the namespace's `id`).
                let id = if rule.handler_on_enclosing_owner {
                    handler_on_enclosing_owner(graph, index, f.file, r.site_byte, &r.name)
                } else {
                    None
                }
                .or_else(|| index.by_simple(&r.name, f.file));
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

            let route = if rule.kind == TrustKind::Tool {
                let own = if shape == EntryShape::Closure {
                    ""
                } else {
                    node.simple_name.as_str()
                };
                tool_route(&r.name_arg, &r.route, own)
            } else if r.route.is_empty() {
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

/// Which test harness owns each test file's block verbs.
///
/// `describe`/`it`/`test` are shared by every JavaScript harness, so a
/// rule-per-harness reading of them would enter each case once per
/// harness the tree detects. A file that imports harnesses belongs to
/// those; one that imports none to the language's globals harness
/// ([`rules::harness_globals_rank`]): the highest-ranked active one, then
/// the one detected in the most files, then the first in rule order.
struct HarnessChoice {
    /// file -> ids of the harness rules its own imports name.
    imported: HashMap<FileId, Vec<String>>,
    /// language -> id of its globals harness.
    globals: HashMap<String, String>,
    /// languages with any active harness rule.
    languages: HashSet<String>,
}

impl HarnessChoice {
    fn build(
        facts: &[FileFacts],
        active: &[&FrameworkRule],
        evidence: &Detection,
    ) -> Self {
        let harnesses: Vec<&FrameworkRule> = active
            .iter()
            .copied()
            .filter(|r| r.kind == TrustKind::Test && !r.registrars.is_empty())
            .collect();
        let mut out = Self {
            imported: HashMap::new(),
            globals: HashMap::new(),
            languages: harnesses.iter().map(|r| r.language.clone()).collect(),
        };
        if harnesses.is_empty() {
            return out;
        }
        for f in facts {
            if !out.languages.contains(&f.language) {
                continue;
            }
            let ids: Vec<String> = harnesses
                .iter()
                .filter(|r| r.language == f.language)
                .filter(|r| {
                    f.imports
                        .iter()
                        .any(|i| r.detect.iter().any(|p| import_matches(&i.path, p)))
                })
                .map(|r| r.id.clone())
                .collect();
            if !ids.is_empty() {
                out.imported.insert(f.file, ids);
            }
        }
        let mut best: HashMap<String, (u8, u32, usize)> = HashMap::new();
        for (order, r) in harnesses.iter().enumerate() {
            let rank = rules::harness_globals_rank(&r.id, &r.language);
            if rank == 0 {
                continue;
            }
            let hits = evidence
                .hits
                .get(&(r.id.clone(), r.language.clone()))
                .copied()
                .unwrap_or(0);
            let key = (rank, hits, usize::MAX - order);
            if best.get(&r.language).is_none_or(|b| key > *b) {
                best.insert(r.language.clone(), key);
                out.globals.insert(r.language.clone(), r.id.clone());
            }
        }
        out
    }

    /// Whether `rule`'s block verbs in `file` are its own.
    fn allows(&self, rule: &FrameworkRule, file: &FileFacts) -> bool {
        if let Some(ids) = self.imported.get(&file.file) {
            return ids.contains(&rule.id);
        }
        match self.globals.get(&rule.language) {
            Some(id) => *id == rule.id,
            // No globals harness for the language (a user rule, or one
            // reached only by import): nothing to arbitrate.
            None => true,
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

/// **Observer callbacks** — `on_<field>` methods matching a class-level
/// `field = Type(...)` declaration.
///
/// Driven by `FrameworkRule::observer_types`: when a rule lists type
/// names (e.g. Kivy's `StringProperty`), and the Python plugin recorded
/// a class-level assignment `text = StringProperty(...)` in
/// `FileFacts::class_fields`, the method `on_text` in that class is a
/// framework-invoked callback.  The Python plugin extracts the data
/// generically; interpretation belongs here.
fn match_observer_callbacks(
    graph: &Graph,
    facts: &[FileFacts],
    rule: &FrameworkRule,
    out: &mut Vec<FrameworkEntry>,
) {
    if rule.observer_types.is_empty() {
        return;
    }
    let types: HashSet<&str> = rule.observer_types.iter().map(|s| s.as_str()).collect();

    // Build class_qn → {field_name} from the matching class_fields.
    let mut observers: HashMap<&str, HashSet<String>> = HashMap::new();
    for f in facts {
        if f.language != rule.language {
            continue;
        }
        for cf in &f.class_fields {
            if types.contains(cf.type_name.as_str()) {
                observers
                    .entry(cf.class_qn.as_str())
                    .or_default()
                    .insert(format!("on_{}", cf.field_name));
            }
        }
    }
    if observers.is_empty() {
        return;
    }

    for (id, node) in &graph.callables {
        if node.language != rule.language || node.synthetic {
            continue;
        }
        if !node.simple_name.starts_with("on_") {
            continue;
        }
        let owner = crate::names::owner_from_qn(&node.qualified_name).unwrap_or("");
        // Check every class in the observer map: the method's owner may
        // be the full qualified name or just the last segment.
        let matched = observers.iter().any(|(class_qn, methods)| {
            if !methods.contains(&node.simple_name) {
                return false;
            }
            let class_last = class_qn.rsplit('.').next().unwrap_or(class_qn);
            owner == *class_qn || owner == class_last
        });
        if !matched {
            continue;
        }
        out.push(FrameworkEntry {
            framework: rule.id.clone(),
            kind: rule.kind,
            shape: EntryShape::Attribute,
            route: String::new(),
            target: *id,
            target_name: node.qualified_name.clone(),
            evidence: format!(
                "`{}` is an observer callback for a declared property",
                node.simple_name,
            ),
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
    if node.kind != CallableKind::Function {
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
    /// Method name -> every owner `by_owner_method` holds it under, so
    /// the near-miss owner lookup reads one method's owners instead of
    /// scanning the whole map on every registration.
    owners_by_method: HashMap<String, Vec<String>>,
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
            // A Clojure `letfn` function is local to its form (WS37): a
            // name registered anywhere else is never it.
            if n.attributes
                .iter()
                .any(|a| a == cgg_core::CLOJURE_LETFN_ATTR)
            {
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
        let mut owners_by_method: HashMap<String, Vec<String>> = HashMap::new();
        for (o, m) in by_owner_method.keys() {
            owners_by_method
                .entry(m.clone())
                .or_default()
                .push(o.clone());
        }
        Self {
            by_simple,
            by_owner_method,
            owners_by_method,
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

    /// Every definition of `owner.member`: those in `from`'s file when
    /// there are any, otherwise the one definition if it is unique.
    /// Overloads in one file are all returned — a data source may be
    /// overloaded, and rooting is a claim about the name, not an edge.
    fn owner_members(
        &self,
        graph: &Graph,
        owner: &str,
        member: &str,
        from: FileId,
    ) -> Vec<CallableId> {
        let Some(ids) = self
            .by_owner_method
            .get(&(owner.to_string(), member.to_string()))
        else {
            return Vec::new();
        };
        let local: Vec<CallableId> = ids
            .iter()
            .copied()
            .filter(|i| graph.callables.get(i).is_some_and(|c| c.file == from))
            .collect();
        if !local.is_empty() {
            return local;
        }
        match ids.as_slice() {
            [one] => vec![*one],
            _ => Vec::new(),
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
            .owners_by_method
            .get(method)
            .into_iter()
            .flatten()
            .filter(|o| owner_is_close(o, owner))
            .filter_map(|o| self.by_owner_method.get(&(o.clone(), method.to_string())))
            .flat_map(|ids| ids.iter().copied())
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
fn dedup(entries: &mut Vec<FrameworkEntry>, graph: &Graph) {
    // Strongest shape per target FIRST. Doing it the other way round
    // lets a weaker match that merely ran earlier claim the
    // (target, framework, route) slot and evict the precise one — a
    // Rails `root to: 'photos#index'` losing to "declares
    // ApplicationController", which is the same entry described worse.
    // A harness entry never competes with a framework one: a route
    // registered on a test fixture is still how a request reaches it,
    // and "the test runner calls it" must not evict that.
    let mut best: HashMap<(CallableId, bool), EntryShape> = HashMap::new();
    for e in entries.iter() {
        let rank = shape_rank(e.shape);
        best.entry((e.target, e.kind == TrustKind::Test))
            .and_modify(|s| {
                if rank < shape_rank(*s) {
                    *s = e.shape;
                }
            })
            .or_insert(e.shape);
    }
    entries
        .retain(|e| best.get(&(e.target, e.kind == TrustKind::Test)) == Some(&e.shape));

    // Then one entry per (target, framework, route). Two rules can
    // legitimately match the same handler — a project using both FastAPI
    // and Flask sees `@app.get` under both vocabularies — and two nodes
    // for one handler would double-count the attack surface.
    let mut seen: BTreeSet<(CallableId, String, String)> = BTreeSet::new();
    entries.retain(|e| seen.insert((e.target, e.framework.clone(), e.route.clone())));
    // Order by the target's position in the graph (discovery order),
    // not by id value: ids are content hashes now, so ordering by value
    // is arbitrary — and this order decides which entry wins when two
    // share a synthesized node name in `synthesize_entry_nodes`, which
    // is where the entry node's `language` comes from.
    entries.sort_by_cached_key(|e| {
        (
            graph.callables.get_index_of(&e.target),
            e.framework.clone(),
            e.route.clone(),
        )
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
            "https://deno.land/x/oak@v12.1.0/mod.ts",
            "jsr:@oak/oak@^17",
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
            "https://deno.land/x/oak",
            "jsr:@oak/oak",
            "@oak/oak",
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
    fn a_file_routed_framework_routes_by_the_path_below_its_directory() {
        let r = |p: &str| file_route(std::path::Path::new(p), "routes");
        assert_eq!(r("app/routes/index.dart").as_deref(), Some("/"));
        assert_eq!(r("routes/users/[id].dart").as_deref(), Some("/users/[id]"));
        assert_eq!(r("routes/api/index.dart").as_deref(), Some("/api"));
        assert_eq!(
            r("routes/_middleware.dart").as_deref(),
            Some("/_middleware")
        );
        assert_eq!(r("lib/helpers.dart"), None);
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
        // A version pin after the name is a boundary; a scope `@` is not.
        assert!(import_matches(
            "https://deno.land/x/oak@v12.1.0/mod.ts",
            "https://deno.land/x/oak"
        ));
        assert!(import_matches("jsr:@oak/oak@^17", "jsr:@oak/oak"));
        assert!(!import_matches(
            "https://deno.land/x/oakland@1/mod.ts",
            "https://deno.land/x/oak"
        ));
        assert!(!import_matches("@oak/oak", ""));
        assert!(!import_matches("nextcloud", "next"));
        assert!(!import_matches("expresso", "express"));
    }

    #[test]
    fn path_conventions_match_on_segment_boundaries() {
        assert!(path_matches("app/urls.py", "urls.py"));
        assert!(path_matches("config/routes.rb", "config/routes.rb"));
        assert!(path_matches("src/app/controllers/x.rb", "app/controllers/"));
        assert!(!path_matches("app/myurls.py", "urls.py"));
        assert!(!path_matches("app/controllersx/y.rb", "app/controllers/"));
        assert!(
            !path_matches("app/controllers", "app/controllers/"),
            "a directory marker needs something inside it"
        );
    }

    #[test]
    fn a_directory_marker_matches_before_a_later_same_named_segment() {
        // The last `lua` of the path is the file extension.
        assert!(path_matches("x/lua/oil/init.lua", "lua/"));
        assert!(path_matches("lua/oil/init.lua", "lua/"));
    }

    #[test]
    fn a_star_matches_within_one_segment() {
        assert!(path_matches("lua/oil/init.lua", "lua/*/init.lua"));
        assert!(path_matches("a/lua/oil/init.lua", "lua/*/init.lua"));
        assert!(!path_matches("lua/oil/sub/init.lua", "lua/*/init.lua"));
        assert!(path_matches(
            "apisix/plugins/limit-req.lua",
            "apisix/plugins/*.lua"
        ));
        assert!(!path_matches(
            "apisix/plugins/x/y.lua",
            "apisix/plugins/*.lua"
        ));
        assert!(path_matches("au/pkg/update.ps1", "update.ps1"));
        assert!(path_matches("a/b.install", "*.install"));
        assert!(!path_matches("a/b.installer", "*.install"));
    }

    // --- WS18: harness and framework entries ------------------------------

    #[test]
    fn naming_convention_patterns_respect_word_boundaries() {
        assert!(name_pattern_matches("Test*", "TestFoo"));
        assert!(name_pattern_matches("Test*", "Test_x"));
        assert!(!name_pattern_matches("Test*", "Testing"));
        assert!(!name_pattern_matches("Test*", "Test"));
        assert!(
            !name_pattern_matches("Test*", "testFoo"),
            "case is the convention"
        );
        assert!(name_pattern_matches("test*", "testThing"));
        assert!(!name_pattern_matches("test*", "tests"));
        assert!(name_pattern_matches("test_*", "test_a"));
        assert!(name_pattern_matches("*_test", "add_test"));
        assert!(!name_pattern_matches("*_test", "_test"));
        assert!(!name_pattern_matches("*_test", "latest"));
        // Infix: both ends, a non-empty middle, the prefix's boundary.
        assert!(name_pattern_matches("get*R", "getHomeR"));
        assert!(name_pattern_matches("handle*R", "handleStaticR"));
        assert!(!name_pattern_matches("get*R", "getR"));
        assert!(!name_pattern_matches("get*R", "getterR"));
        assert!(!name_pattern_matches("get*R", "getHome"));
        assert!(!name_pattern_matches("get*R", "sdistR"));
        assert!(
            name_pattern_matches("Fact", "fact"),
            "plain names stay case-insensitive"
        );
    }

    #[test]
    fn base_types_accept_naming_convention_patterns() {
        let servicer = vec!["*Servicer".to_string()];
        assert!(base_type_matches(&servicer, "GreeterServicer"));
        assert!(base_type_matches(
            &servicer,
            "helloworld_pb2_grpc.GreeterServicer"
        ));
        assert!(!base_type_matches(&servicer, "Servicer"));
        assert!(!base_type_matches(&servicer, "GreeterServicerBase"));
        let go = vec!["Unimplemented*".to_string()];
        assert!(base_type_matches(&go, "pb.UnimplementedGreeterServer"));
        // Exact names keep their case-insensitive compare.
        assert!(base_type_matches(&["Module".to_string()], "nn.module"));
    }

    #[test]
    fn mock_decorators_are_found_through_every_import_form() {
        let mut f = FileFacts::new(FileId::new(0), PathBuf::from("tests/t.py"), "python");
        let imp = |kind: &str, path: &str, alias: &str| ImportRecord {
            kind: kind.into(),
            path: path.into(),
            alias: alias.into(),
            site_line: 1,
            site_byte: 0,
        };
        f.imports
            .push(imp("from-import", "unittest.mock", "patch as p, MagicMock"));
        f.imports.push(imp("from-import", "unittest", "mock"));
        f.imports.push(imp("import", "unittest.mock", "um"));
        f.imports.push(imp("import", "mock, os", ""));
        f.imports.push(imp("from-import", "fastapi", "FastAPI"));
        let m = python_mock_bindings(&[f]);
        let bound = &m[&FileId::new(0)];
        for key in ["p", "p.object", "mock.patch", "mock.patch.dict", "um.patch"] {
            assert!(is_mock_decorator(bound, key), "{key}");
        }
        for key in ["app.patch", "patch", "router.patch", "mockery.patch"] {
            assert!(!is_mock_decorator(bound, key), "{key}");
        }
        // No mocking import: nothing is a mock decorator.
        let plain = facts_with_import("python", "api.py", "fastapi");
        assert!(python_mock_bindings(&[plain]).is_empty());
    }

    #[test]
    fn infix_patterns_need_both_ends_and_a_middle() {
        assert!(name_pattern_matches("sqlite3_*_init", "sqlite3_vec_init"));
        assert!(!name_pattern_matches("sqlite3_*_init", "sqlite3__init"));
        assert!(!name_pattern_matches("sqlite3_*_init", "sqlite3_helper"));
        assert!(name_pattern_matches(
            "HAL_*Callback",
            "HAL_UART_RxCpltCallback"
        ));
        assert!(!name_pattern_matches("HAL_*Callback", "hal_x_callback"));
    }

    #[test]
    fn base_type_patterns_read_the_written_name_and_its_last_segment() {
        let w = |xs: &[&str]| xs.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert!(base_type_matches(&w(&["I*"]), "godot::classes::INode2D"));
        assert!(!base_type_matches(&w(&["I*"]), "Iterator"));
        assert!(base_type_matches(
            &w(&["*::Service"]),
            "PredictionService::Service"
        ));
        assert!(!base_type_matches(&w(&["*::Service"]), "Service"));
        assert!(base_type_matches(&w(&["Plugin"]), "bevy::app::Plugin"));
    }

    fn rust_facts() -> FileFacts {
        FileFacts::new(FileId::new(0), PathBuf::from("src/lib.rs"), "rust")
    }

    #[test]
    fn path_qualified_rust_test_attributes_enter_through_one_harness_node() {
        let mut a = node(0, "c::t::a", "a", "rust");
        a.attributes = vec!["#[ntex::test]".into()];
        let mut b = node(1, "c::t::b", "b", "rust");
        b.attributes = vec!["#[crate::rt_test]".into()];
        let mut w = node(2, "c::t::w", "w", "rust");
        w.attributes = vec!["#[wasm_bindgen_test]".into()];
        // Near misses: a `test`-named helper with no attribute, and
        // `#[cfg(test)]`, which gates compilation and runs nothing.
        let helper = node(3, "c::t::helper_test_data", "helper_test_data", "rust");
        let mut gated = node(4, "c::t::gated", "gated", "rust");
        gated.attributes = vec!["#[cfg(test)]".into()];
        let g = graph_with(vec![a, b, w, helper, gated]);
        let out = detect(&g, &[rust_facts()], &[]);
        let ids: Vec<u32> = out
            .entries
            .iter()
            .filter(|e| e.framework == "cargo-test")
            .map(|e| e.target.as_u32())
            .collect();
        assert_eq!(ids, vec![0, 1, 2], "{:?}", out.entries);
        let names: BTreeSet<String> = out.entries.iter().map(|e| e.node_name()).collect();
        assert_eq!(
            names.into_iter().collect::<Vec<_>>(),
            vec!["<framework-entry>::test::cargo-test".to_string()]
        );
    }

    #[test]
    fn a_pymethods_block_exposes_its_members_and_a_plain_impl_does_not() {
        let mut f = rust_facts();
        f.attributed_containers.push(cgg_core::AttributedContainer {
            start_byte: 100,
            end_byte: 200,
            attributes: vec!["#[pymethods]".into()],
        });
        let mut inside = node(0, "c::It::__next__", "__next__", "rust");
        (inside.start_byte, inside.end_byte) = (120, 150);
        let mut outside = node(1, "c::It::helper", "helper", "rust");
        (outside.start_byte, outside.end_byte) = (300, 350);
        let out = detect(&graph_with(vec![inside, outside]), &[f], &[]);
        let ids: Vec<u32> = out.entries.iter().map(|e| e.target.as_u32()).collect();
        assert_eq!(ids, vec![0], "{:?}", out.entries);
        assert_eq!(out.entries[0].framework, "ffi-export");
    }

    #[test]
    fn a_harness_name_counts_only_in_a_test_file_or_one_importing_the_harness() {
        let t = node(0, "x.TestFoo", "TestFoo", "go");
        let mut other = node(1, "y.TestServer", "TestServer", "go");
        other.file = FileId::new(1);
        let mut g = graph_with(vec![t, other]);
        g.add_file(cgg_core::graph::FileRecord {
            id: FileId::new(1),
            path: PathBuf::from("y.go"),
            language: "go".into(),
            ..Default::default()
        });
        // File 0 imports `testing`; file 1 imports nothing, so a
        // `TestServer` there is a coincidence of naming.
        let f0 = facts_with_import("go", "x.go", "testing");
        let f1 = FileFacts::new(FileId::new(1), PathBuf::from("y.go"), "go");
        let out = detect(&g, &[f0, f1], &[]);
        let ids: Vec<u32> = out.entries.iter().map(|e| e.target.as_u32()).collect();
        assert_eq!(ids, vec![0], "{:?}", out.entries);
    }

    // WS20 gap 21: gtest's TEST bodies (parsed as functions named for
    // the macro) and fixture hooks, and libFuzzer's engine entry point.
    #[test]
    fn gtest_cases_fixture_hooks_and_libfuzzer_entries() {
        let mut f = facts_with_import("cpp", "foo_test.cc", "gtest/gtest.h");
        let mut nodes = Vec::new();
        for (i, (qn, name, bases)) in [
            ("TEST", "TEST", vec![]),
            ("Fix::SetUp", "SetUp", vec!["testing::Test".to_string()]),
            ("Fix::helper", "helper", vec!["testing::Test".to_string()]),
            // Near miss: `SetUp` on a class with no gtest base.
            ("Other::SetUp", "SetUp", vec!["Widget".to_string()]),
            // Near miss: a lowercase `test` (a SFINAE probe) is not `TEST`.
            ("is_map::test", "test", vec![]),
        ]
        .into_iter()
        .enumerate()
        {
            let mut n = node(i as u32, qn, name, "cpp");
            n.start_byte = i as u32 * 10;
            f.definitions.push(DefRecord {
                simple_name: name.into(),
                qualified_name: qn.into(),
                start_byte: n.start_byte,
                base_types: bases,
                ..Default::default()
            });
            nodes.push(n);
        }
        let out = detect(&graph_with(nodes), &[f], &[]);
        let got: Vec<(u32, &str)> = out
            .entries
            .iter()
            .map(|e| (e.target.as_u32(), e.framework.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![(0, "gtest"), (1, "gtest-fixture")],
            "{:?}",
            out.entries
        );

        // libFuzzer: no include names the engine; the defined hook does.
        let mut f = FileFacts::new(FileId::new(0), PathBuf::from("fuzz.c"), "c");
        let mut nodes = Vec::new();
        for (i, name) in ["LLVMFuzzerTestOneInput", "parse"].iter().enumerate() {
            let mut n = node(i as u32, name, name, "c");
            n.start_byte = i as u32 * 10;
            f.definitions.push(DefRecord {
                simple_name: name.to_string(),
                qualified_name: name.to_string(),
                start_byte: n.start_byte,
                ..Default::default()
            });
            nodes.push(n);
        }
        let out = detect(&graph_with(nodes), &[f], &[]);
        let got: Vec<(u32, &str)> = out
            .entries
            .iter()
            .map(|e| (e.target.as_u32(), e.framework.as_str()))
            .collect();
        assert_eq!(got, vec![(0, "libfuzzer")], "{:?}", out.entries);
    }

    fn test_class(lang: &str, path: &str, import: &str) -> (Graph, Vec<FileFacts>) {
        let mut f = facts_with_import(lang, path, import);
        let mut nodes = Vec::new();
        for (i, name) in ["test_a", "helper"].iter().enumerate() {
            let mut n = node(i as u32, &format!("m.T.{name}"), name, lang);
            n.start_byte = i as u32 * 10;
            f.definitions.push(DefRecord {
                simple_name: name.to_string(),
                qualified_name: format!("m.T.{name}"),
                start_byte: n.start_byte,
                base_types: vec!["TestCase".into()],
                ..Default::default()
            });
            nodes.push(n);
        }
        (graph_with(nodes), vec![f])
    }

    // A test class's case enters through the harness node; its helper is
    // a test root mark in Python (resolved weakly through `self`), and
    // nothing at all in Java, where an unused helper stays reportable.
    #[test]
    fn test_class_helpers_are_test_roots_only_in_dynamic_languages() {
        let (g, f) = test_class("python", "m.py", "unittest");
        let out = detect(&g, &f, &[]);
        let got: Vec<(u32, bool)> = out
            .entries
            .iter()
            .map(|e| (e.target.as_u32(), e.node))
            .collect();
        assert_eq!(got, vec![(0, true), (1, false)], "{:?}", out.entries);

        let (g, f) = test_class("java", "T.java", "junit.framework.TestCase");
        let out = detect(&g, &f, &[]);
        let got: Vec<u32> = out.entries.iter().map(|e| e.target.as_u32()).collect();
        assert_eq!(got, vec![0], "{:?}", out.entries);
    }

    #[test]
    fn a_xaml_binding_enters_its_handler_keyed_by_the_event() {
        let mut f =
            FileFacts::new(FileId::new(0), PathBuf::from("Main.xaml.cs"), "csharp");
        f.markup_bindings.push(cgg_core::MarkupBinding {
            kind: "xaml".into(),
            event: "Click".into(),
            handler: "OnSave".into(),
            line: 3,
        });
        let h = node(0, "App.Main.OnSave", "OnSave", "csharp");
        let other = node(1, "App.Main.OnLoad", "OnLoad", "csharp");
        let out = detect(&graph_with(vec![h, other]), &[f], &[]);
        assert_eq!(out.entries.len(), 1, "{:?}", out.entries);
        assert_eq!(
            out.entries[0].node_name(),
            "<framework-entry>::ui::xaml::Click"
        );
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

    // A tool entry is named by what the model calls it, never by the
    // first string argument: rmcp's `#[tool(description = "…")]` named
    // every legba tool after its description.
    #[test]
    fn a_tool_entry_is_named_by_the_tool_not_its_description() {
        let mut a = node(0, "Server::scan_ports", "scan_ports", "rust");
        a.attributes = vec!["#[tool(description = \"Scan the given ports\")]".into()];
        let mut b = node(1, "Server::fetch", "fetch", "rust");
        b.attributes =
            vec!["#[tool(name = \"http_get\", description = \"Fetch a URL\")]".into()];
        let mut c = node(2, "Server::ping", "ping", "rust");
        c.attributes = vec!["#[prompt(\"health check please\")]".into()];
        let g = graph_with(vec![a, b, c]);
        let facts = vec![facts_with_import("rust", "src/server.rs", "rmcp")];
        let out = detect(&g, &facts, &[]);
        let mut names: Vec<String> = out.entries.iter().map(|e| e.node_name()).collect();
        names.sort();
        assert_eq!(
            names,
            [
                "<framework-entry>::tool::rmcp::http_get",
                "<framework-entry>::tool::rmcp::ping",
                "<framework-entry>::tool::rmcp::scan_ports",
            ]
        );
        assert!(out.entries.iter().all(|e| e.kind == TrustKind::Tool));
    }

    #[test]
    fn tool_names_come_from_keys_then_name_shaped_positionals() {
        assert_eq!(
            attribute_keyed_name("@mcp.tool(name=\"search\", description=\"d\")"),
            Some("search".into())
        );
        assert_eq!(
            attribute_keyed_name("@Tool({ name: 'search', description: 'x y' })"),
            Some("search".into())
        );
        assert_eq!(
            attribute_keyed_name("[McpServerTool(Name = \"get_x\")]"),
            Some("get_x".into())
        );
        assert_eq!(attribute_keyed_name("@Tool(\"Returns x\")"), None);
        assert_eq!(
            attribute_positional_string("@Tool(value = \"Returns x\", name = \"n\")"),
            Some("Returns x".into())
        );
        assert_eq!(
            attribute_positional_string("@mcp.resource(\"config://app/{id}\")"),
            Some("config://app/{id}".into())
        );
        // A description or a model id never names the tool.
        assert_eq!(tool_route("", "Returns the weather", "weather"), "weather");
        assert_eq!(tool_route("", "openai:gpt-4o", "agent"), "agent");
        assert_eq!(tool_route("", "get_weather", "handler"), "get_weather");
        assert_eq!(tool_route("lookup", "get_weather", "h"), "lookup");
        assert_eq!(tool_route("", "config://app", "h"), "config://app");
        assert_eq!(
            tool_owner_route("tools.search.SearchTool._run"),
            "SearchTool"
        );
    }

    // A registrar's `name=` / options-object `name` (`RefRecord::name_arg`)
    // names a tool handed over by value; an inline closure without one
    // keeps its positional name, never its synthesized `handler_at_N`.
    #[test]
    fn a_tool_registrar_is_named_by_its_name_argument() {
        let h = node(0, "srv.lookup", "lookup", "python");
        let mut closure = node(1, "srv.handler_at_9", "handler_at_9", "python");
        closure.synthetic = false;
        let g = graph_with(vec![h, closure]);
        let mut f = facts_with_import("python", "srv.py", "mytools");
        f.references.push(RefRecord {
            name: "lookup".into(),
            receiver_hint: VALUE_REF_HINT.to_string(),
            site_line: 4,
            site_byte: 40,
            context: "server.add_tool".into(),
            route: "Look things up".into(),
            name_arg: "kb_lookup".into(),
            ..Default::default()
        });
        f.references.push(RefRecord {
            name: "handler_at_9".into(),
            receiver_hint: VALUE_REF_HINT.to_string(),
            site_line: 9,
            site_byte: 90,
            context: "server.add_tool".into(),
            route: "get_weather".into(),
            ..Default::default()
        });
        let rule = FrameworkRule {
            id: "mytools".into(),
            language: "python".into(),
            kind: TrustKind::Tool,
            detect: vec!["mytools".into()],
            registrars: vec!["add_tool".into()],
            node: true,
            ..Default::default()
        };
        let out = detect(&g, &[f], &[rule]);
        let mut names: Vec<String> = out.entries.iter().map(|e| e.node_name()).collect();
        names.sort();
        assert_eq!(
            names,
            [
                "<framework-entry>::tool::mytools::get_weather",
                "<framework-entry>::tool::mytools::kb_lookup",
            ]
        );
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
            property_read: false,
            arg_labels: Vec::new(),
            arg_type: None,
            from_macro_arg: false,
            path_receiver: false,
            dot_call: false,
            arrow: false,
            macro_call: false,
            invocant: false,
            implicit_receiver: false,
            not_def: None,
            arity: None,
            value_ref: false,
            paren_less: false,
            typed_receiver: false,
            weak_receiver_type: false,
            name: "listUsers".into(),
            receiver_hint: VALUE_REF_HINT.to_string(),
            site_line: 4,
            site_byte: 40,
            context: "app.get".into(),
            route: "/users".into(),
            kwargs: Vec::new(),
            retyped_receiver: false,
            receiver_module: None,
            name_arg: String::new(),
            slot: String::new(),
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
            property_read: false,
            arg_labels: Vec::new(),
            arg_type: None,
            from_macro_arg: false,
            path_receiver: false,
            dot_call: false,
            arrow: false,
            macro_call: false,
            invocant: false,
            implicit_receiver: false,
            not_def: None,
            arity: None,
            value_ref: false,
            paren_less: false,
            typed_receiver: false,
            weak_receiver_type: false,
            name: "name".into(),
            receiver_hint: VALUE_REF_HINT.to_string(),
            site_line: 3,
            site_byte: 30,
            context: "crate_ids.get".into(),
            route: String::new(),
            kwargs: Vec::new(),
            retyped_receiver: false,
            receiver_module: None,
            name_arg: String::new(),
            slot: String::new(),
        });
        assert!(detect(&g, &f_vec(bare), &[]).entries.is_empty());

        // axum's own `get(handler)` is a free function — receiver-less,
        // so it needs no route of its own.
        let mut free = facts_with_import("rust", "s.rs", "axum");
        free.references.push(RefRecord {
            property_read: false,
            arg_labels: Vec::new(),
            arg_type: None,
            from_macro_arg: false,
            path_receiver: false,
            dot_call: false,
            arrow: false,
            macro_call: false,
            invocant: false,
            implicit_receiver: false,
            not_def: None,
            arity: None,
            value_ref: false,
            paren_less: false,
            typed_receiver: false,
            weak_receiver_type: false,
            name: "name".into(),
            receiver_hint: VALUE_REF_HINT.to_string(),
            site_line: 3,
            site_byte: 30,
            context: "get".into(),
            route: "/api/x".into(),
            kwargs: Vec::new(),
            retyped_receiver: false,
            receiver_module: None,
            name_arg: String::new(),
            slot: String::new(),
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
            property_read: false,
            arg_labels: Vec::new(),
            arg_type: None,
            from_macro_arg: false,
            path_receiver: false,
            dot_call: false,
            arrow: false,
            macro_call: false,
            invocant: false,
            implicit_receiver: false,
            not_def: None,
            arity: None,
            value_ref: false,
            paren_less: false,
            typed_receiver: false,
            weak_receiver_type: false,
            name: "user_id".into(),
            receiver_hint: STRING_REF_HINT.to_string(),
            site_line: 3,
            site_byte: 30,
            context: "session.get".into(),
            route: "user_id".into(),
            kwargs: Vec::new(),
            retyped_receiver: false,
            receiver_module: None,
            name_arg: String::new(),
            slot: String::new(),
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
            property_read: false,
            arg_labels: Vec::new(),
            arg_type: None,
            from_macro_arg: false,
            path_receiver: false,
            dot_call: false,
            arrow: false,
            macro_call: false,
            invocant: false,
            implicit_receiver: false,
            not_def: None,
            arity: None,
            value_ref: false,
            paren_less: false,
            typed_receiver: false,
            weak_receiver_type: false,
            name: "listUsers".into(),
            receiver_hint: VALUE_REF_HINT.to_string(),
            site_line: 4,
            site_byte: 40,
            context: "app.get".into(),
            route: "/users".into(),
            kwargs: Vec::new(),
            retyped_receiver: false,
            receiver_module: None,
            name_arg: String::new(),
            slot: String::new(),
        });
        let out = detect(&g, &f_vec(f), &[]);
        assert!(out.entries.is_empty());
    }

    #[test]
    fn value_ref_registrar_prefers_handler_on_enclosing_class() {
        // Two widgets both define `on_mouse_pos`. Bare-name lookup in
        // the same file would pick whichever was inserted first (B);
        // the bind site is inside A's `__init__`, so A must win.
        let mut a_init = node(0, "ui.A.__init__", "__init__", "python");
        a_init.start_byte = 0;
        a_init.end_byte = 50;
        let mut b_handler = node(1, "ui.B.on_mouse_pos", "on_mouse_pos", "python");
        b_handler.start_byte = 100;
        b_handler.end_byte = 130;
        let mut a_handler = node(2, "ui.A.on_mouse_pos", "on_mouse_pos", "python");
        a_handler.start_byte = 60;
        a_handler.end_byte = 90;
        let g = graph_with(vec![a_init, b_handler, a_handler]);

        let mut f = facts_with_import("python", "ui.py", "kivy");
        f.references.push(RefRecord {
            property_read: false,
            arg_labels: Vec::new(),
            arg_type: None,
            from_macro_arg: false,
            path_receiver: false,
            dot_call: false,
            arrow: false,
            macro_call: false,
            invocant: false,
            implicit_receiver: false,
            not_def: None,
            arity: None,
            value_ref: false,
            paren_less: false,
            typed_receiver: false,
            weak_receiver_type: false,
            name: "on_mouse_pos".into(),
            receiver_hint: VALUE_REF_HINT.to_string(),
            site_line: 4,
            site_byte: 20,
            context: "self.bind".into(),
            route: String::new(),
            kwargs: Vec::new(),
            retyped_receiver: false,
            receiver_module: None,
            name_arg: String::new(),
            slot: String::new(),
        });
        let out = detect(&g, &f_vec(f), &[]);
        let names: Vec<&str> =
            out.entries.iter().map(|e| e.target_name.as_str()).collect();
        assert!(
            names.contains(&"ui.A.on_mouse_pos"),
            "bind inside A should resolve to A.on_mouse_pos, got {names:?}"
        );
        assert!(
            !names.contains(&"ui.B.on_mouse_pos"),
            "must not steal B.on_mouse_pos: {names:?}"
        );
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
        dedup(&mut v, &Graph::new());
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].shape, EntryShape::Attribute);
        // And an unrelated target survives alongside it.
        a.target = CallableId::new(8);
        let mut v2 = vec![v[0].clone(), a];
        dedup(&mut v2, &Graph::new());
        assert_eq!(v2.len(), 2);
    }

    // A route registered on a test fixture keeps its route entry: the
    // harness entry describes a different origin and must not evict it.
    #[test]
    fn a_harness_entry_does_not_evict_a_route_on_the_same_target() {
        let route = FrameworkEntry {
            framework: "falcon".into(),
            kind: TrustKind::Network,
            shape: EntryShape::Registrar,
            route: "add_route(\"/x\")".into(),
            target: CallableId::new(7),
            target_name: "t.resource".into(),
            evidence: "registrar".into(),
            file: FileId::new(0),
            site_line: 1,
            node: true,
        };
        let mut harness = route.clone();
        harness.framework = "pytest".into();
        harness.kind = TrustKind::Test;
        harness.shape = EntryShape::Attribute;
        harness.route = String::new();
        let mut v = vec![route, harness];
        dedup(&mut v, &Graph::new());
        let got: Vec<&str> = v.iter().map(|e| e.framework.as_str()).collect();
        assert_eq!(got.len(), 2, "{got:?}");
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

    // A service provider's `configure(ServiceRegistration)` carries no
    // annotation; the contract it implements is the evidence. The same
    // name on a class outside the contract stays reportable.
    #[test]
    fn a_service_provider_contract_roots_configure_only() {
        let mut hook = node(0, "g.ScopeServices.configure", "configure", "java");
        hook.start_byte = 100;
        let mut other = node(1, "g.Plain.configure", "configure", "java");
        other.start_byte = 300;
        let g = graph_with(vec![hook, other]);
        let mut f =
            facts_with_import("java", "S.java", "org.gradle.internal.service.Provides");
        f.definitions.push(DefRecord {
            simple_name: "configure".into(),
            qualified_name: "g.ScopeServices.configure".into(),
            start_byte: 100,
            base_types: vec!["ServiceRegistrationProvider".into()],
            ..Default::default()
        });
        let out = detect(&g, &[f], &[]);
        assert_eq!(rooted(&out), vec!["g.ScopeServices.configure".to_string()]);
    }

    #[test]
    fn a_cyclic_base_chain_terminates() {
        let mut map: HashMap<String, Vec<OwnerBases>> = HashMap::new();
        map.insert(
            "A".to_string(),
            vec![("A".to_string(), vec!["B".to_string()], FileId::new(0))],
        );
        map.insert(
            "B".to_string(),
            vec![("B".to_string(), vec!["A".to_string()], FileId::new(0))],
        );
        // Two classes naming each other: must return rather than spin.
        assert_eq!(
            find_in_base_chain(
                &["A".to_string()],
                "",
                FileId::new(0),
                &map,
                &["Nope".to_string()],
                false,
                &|_, _| true
            ),
            None
        );
    }

    #[test]
    fn a_qualified_base_does_not_walk_into_a_same_named_type_of_another_namespace() {
        let mut map: HashMap<String, Vec<OwnerBases>> = HashMap::new();
        map.insert(
            "Base".to_string(),
            vec![(
                "Mastodon::CLI::Base".to_string(),
                vec!["Thor".to_string()],
                FileId::new(0),
            )],
        );
        map.insert(
            "ApplicationRecord".to_string(),
            vec![(
                "ApplicationRecord".to_string(),
                vec!["ActiveRecord::Base".to_string()],
                FileId::new(0),
            )],
        );
        let thor = ["Thor".to_string()];
        // `Account < ApplicationRecord < ActiveRecord::Base` is no command.
        assert_eq!(
            find_in_base_chain(
                &["ApplicationRecord".to_string()],
                "Account",
                FileId::new(0),
                &map,
                &thor,
                false,
                &|_, _| true
            ),
            None
        );
        // A bare or matching qualifier still reaches it, and a dotted
        // qualifier is a module alias, never a contradiction.
        assert_eq!(
            find_in_base_chain(
                &["views.Base".to_string()],
                "x.Y",
                FileId::new(0),
                &map,
                &thor,
                false,
                &|_, _| true
            ),
            Some("Thor".to_string())
        );
        assert_eq!(
            find_in_base_chain(
                &["Base".to_string()],
                "Mastodon::CLI::Accounts",
                FileId::new(0),
                &map,
                &thor,
                true,
                &|_, _| true
            ),
            Some("Thor".to_string())
        );
        // Ruby reads a bare `Base` through the enclosing modules only:
        // inside `Chewy::Strategy` it is not `Mastodon::CLI::Base`.
        assert_eq!(
            find_in_base_chain(
                &["Base".to_string()],
                "Chewy::Strategy::Mastodon",
                FileId::new(0),
                &map,
                &thor,
                true,
                &|_, _| true
            ),
            None
        );
        assert_eq!(
            find_in_base_chain(
                &["CLI::Base".to_string()],
                "Mastodon::X",
                FileId::new(0),
                &map,
                &thor,
                false,
                &|_, _| true
            ),
            Some("Thor".to_string())
        );
    }

    #[test]
    fn a_bare_base_prefers_the_type_in_the_nearest_namespace() {
        let mut map: HashMap<String, Vec<OwnerBases>> = HashMap::new();
        map.insert(
            "Base".to_string(),
            vec![
                (
                    "Admin::Base".to_string(),
                    vec!["Thor".to_string()],
                    FileId::new(0),
                ),
                (
                    "Api::Base".to_string(),
                    vec!["Grape::API".to_string()],
                    FileId::new(0),
                ),
                ("Base".to_string(), vec!["Thor".to_string()], FileId::new(0)),
            ],
        );
        let thor = ["Thor".to_string()];
        // Lexically (Ruby), `Base` inside `Api` is `Api::Base`.
        assert_eq!(
            find_in_base_chain(
                &["Base".to_string()],
                "Api::Users",
                FileId::new(0),
                &map,
                &thor,
                true,
                &|_, _| true
            ),
            None
        );
        assert_eq!(
            find_in_base_chain(
                &["Base".to_string()],
                "Admin::Users",
                FileId::new(0),
                &map,
                &thor,
                true,
                &|_, _| true
            ),
            Some("Thor".to_string())
        );
    }

    #[test]
    fn a_bare_base_shared_by_several_types_walks_each_where_lookup_is_not_lexical() {
        // PHP: `Tests\HttpClientTestCase extends BaseHttpClientTestCase`
        // (a `use … as` alias) and `Contracts\HttpClientTestCase extends
        // TestCase`. Keeping only one copy lost every test of the other.
        let mut map: HashMap<String, Vec<OwnerBases>> = HashMap::new();
        map.insert(
            "HttpClientTestCase".to_string(),
            vec![
                (
                    "Contracts\\HttpClientTestCase".to_string(),
                    vec!["TestCase".to_string()],
                    FileId::new(0),
                ),
                (
                    "Tests\\HttpClientTestCase".to_string(),
                    vec!["BaseHttpClientTestCase".to_string()],
                    FileId::new(0),
                ),
            ],
        );
        assert_eq!(
            find_in_base_chain(
                &["HttpClientTestCase".to_string()],
                "Tests\\AmpHttpClientTest",
                FileId::new(0),
                &map,
                &["TestCase".to_string()],
                false,
                &|_, _| true
            ),
            Some("TestCase".to_string())
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

    #[test]
    fn observer_callback_matches_a_declared_property_field() {
        // `text = StringProperty('')` at class level → `on_text` is a
        // framework-invoked observer. The Python plugin records the
        // field generically; the Kivy rule's `observer_types` is what
        // interprets it.
        let mut n = node(0, "label.L.on_text", "on_text", "python");
        n.file = FileId::new(0);
        n.start_byte = 100;
        let mut unrelated = node(1, "label.L.on_size", "on_size", "python");
        unrelated.file = FileId::new(0);
        unrelated.start_byte = 200;
        let g = graph_with(vec![n, unrelated]);

        let mut f = facts_with_import("python", "label.py", "kivy");
        f.class_fields.push(cgg_core::ClassFieldDecl {
            class_qn: "label.L".into(),
            field_name: "text".into(),
            type_name: "StringProperty".into(),
            line: 3,
        });

        let out = detect(&g, &f_vec(f), &[]);
        let names: Vec<&str> =
            out.entries.iter().map(|e| e.target_name.as_str()).collect();
        assert!(
            names.contains(&"label.L.on_text"),
            "on_text should match the StringProperty field: {names:?}"
        );
        assert!(
            !names.contains(&"label.L.on_size"),
            "on_size has no matching field declaration: {names:?}"
        );
    }

    #[test]
    fn observer_callback_does_not_fire_without_a_matching_field() {
        // An `on_*` method without a corresponding class-level property
        // declaration must not be claimed.
        let mut n = node(0, "label.L.on_phantom", "on_phantom", "python");
        n.file = FileId::new(0);
        let g = graph_with(vec![n]);

        let f = facts_with_import("python", "label.py", "kivy");
        let out = detect(&g, &f_vec(f), &[]);
        assert!(
            out.entries.is_empty(),
            "no field declaration, no observer entry: {:?}",
            out.entries
        );
    }

    fn rooted(out: &FrameworkOutcome) -> Vec<String> {
        let mut v: Vec<String> =
            out.entries.iter().map(|e| e.target_name.clone()).collect();
        v.sort();
        v
    }

    fn py_import(
        file: u32,
        path: &str,
        kind: &str,
        module: &str,
        names: &str,
    ) -> FileFacts {
        let mut f = FileFacts::new(FileId::new(file), PathBuf::from(path), "python");
        f.imports.push(ImportRecord {
            kind: kind.into(),
            path: module.into(),
            alias: names.into(),
            site_line: 1,
            site_byte: 0,
        });
        f
    }

    fn user_rule(id: &str, lang: &str, detect: &str) -> FrameworkRule {
        FrameworkRule {
            id: id.into(),
            language: lang.into(),
            kind: TrustKind::Tool,
            detect: vec![detect.into()],
            node: true,
            ..Default::default()
        }
    }

    // Per-file evidence: a generic decorator key (`tool`) is one
    // framework's only in a file that carries that framework's import.
    // An `@mcp.tool` in a FastMCP server must not take an entry of an
    // agent framework another file of the tree imports.
    #[test]
    fn a_decorator_matches_only_the_framework_its_file_imports() {
        let mut mcp_tool = node(0, "server.search", "search", "python");
        mcp_tool.attributes = vec!["@mcp.tool()".into()];
        let mut agent_tool = node(1, "agent.lookup", "lookup", "python");
        agent_tool.attributes = vec!["@tool".into()];
        agent_tool.file = FileId::new(1);
        let g = graph_with(vec![mcp_tool, agent_tool]);
        let facts = vec![
            py_import(0, "server.py", "from-import", "fastmcp", "FastMCP"),
            py_import(1, "agent.py", "from-import", "agentlib", "tool"),
        ];
        let mut mcp = user_rule("mcp", "python", "fastmcp");
        mcp.attributes = vec!["tool".into()];
        let mut agent = user_rule("agentlib", "python", "agentlib");
        agent.attributes = vec!["tool".into()];
        let out = detect(&g, &facts, &[mcp, agent]);
        // Only the two rules under test: the built-in `mcp-python` rule
        // rightly enters the FastMCP tool too.
        let mut got: Vec<(String, String)> = out
            .entries
            .iter()
            .filter(|e| e.framework == "mcp" || e.framework == "agentlib")
            .map(|e| (e.framework.clone(), e.target_name.clone()))
            .collect();
        got.sort();
        assert_eq!(
            got,
            [
                ("agentlib".to_string(), "agent.lookup".to_string()),
                ("mcp".to_string(), "server.search".to_string()),
            ]
        );
    }

    // The commonest real shape: the decorator's receiver is imported from
    // the project's own module (`from . import bp`, `from .celeryconf
    // import app`), and that module is the one importing the framework.
    #[test]
    fn a_decorator_through_an_in_tree_import_keeps_its_framework() {
        let mut view = node(0, "blog.views.index", "index", "python");
        view.attributes = vec!["@bp.route(\"/\")".into()];
        let mut task = node(1, "shop.tasks.send", "send", "python");
        task.attributes = vec!["@app.task".into()];
        task.file = FileId::new(1);
        let mut stray = node(2, "misc.other.run", "run", "python");
        stray.attributes = vec!["@app.task".into()];
        stray.file = FileId::new(4);
        let mut sub = node(3, "shop.jobs.sync", "sync", "python");
        sub.attributes = vec!["@celeryconf.app.task".into()];
        sub.file = FileId::new(6);
        let g = graph_with(vec![view, task, stray, sub]);
        let facts = vec![
            py_import(0, "blog/views.py", "from-import", ".", "bp"),
            py_import(1, "shop/tasks.py", "from-import", "shop.celeryconf", "app"),
            py_import(2, "blog/__init__.py", "from-import", "flask", "Blueprint"),
            py_import(3, "shop/celeryconf.py", "from-import", "celery", "Celery"),
            // Imports `app` from a module that carries no celery import.
            py_import(4, "misc/other.py", "from-import", ".helpers", "app"),
            py_import(5, "misc/helpers.py", "import", "os", ""),
            // `from . import celeryconf`: a submodule, not a member.
            py_import(6, "shop/jobs.py", "from-import", ".", "celeryconf"),
        ];
        let mut flask = user_rule("myflask", "python", "flask");
        flask.attributes = vec!["route".into()];
        let mut celery = user_rule("mycelery", "python", "celery");
        celery.attributes = vec!["task".into()];
        let out = detect(&g, &facts, &[flask, celery]);
        let mut got: Vec<String> = out
            .entries
            .iter()
            .filter(|e| e.framework.starts_with("my"))
            .map(|e| e.target_name.clone())
            .collect();
        got.sort();
        assert_eq!(
            got,
            vec![
                "blog.views.index".to_string(),
                "shop.jobs.sync".to_string(),
                "shop.tasks.send".to_string()
            ]
        );
    }

    // A bare registrar is a name the file imports: `tool({…})` is a
    // framework's only where the file imports it (directly, or from a
    // project module re-exporting it).
    #[test]
    fn a_bare_registrar_counts_only_where_the_file_imports_the_framework() {
        let mut h1 = node(0, "a.search", "search", "typescript");
        h1.file = FileId::new(0);
        let mut h2 = node(1, "b.lookup", "lookup", "typescript");
        h2.file = FileId::new(1);
        let mut h3 = node(2, "c.fetch", "fetch", "typescript");
        h3.file = FileId::new(2);
        let g = graph_with(vec![h1, h2, h3]);
        let reg = |file: u32, name: &str| {
            let mut f = FileFacts::new(
                FileId::new(file),
                PathBuf::from(format!("src/{name}.ts")),
                "typescript",
            );
            f.references.push(RefRecord {
                name: name.into(),
                receiver_hint: VALUE_REF_HINT.to_string(),
                site_line: 3,
                site_byte: 30,
                context: "tool".into(),
                ..Default::default()
            });
            f
        };
        let mut a = reg(0, "search");
        a.references[0].name = "search".into();
        a.imports.push(ImportRecord {
            kind: "from-import".into(),
            path: "ai".into(),
            alias: "tool".into(),
            site_line: 1,
            site_byte: 0,
        });
        let mut b = reg(1, "lookup");
        b.references[0].name = "lookup".into();
        b.imports.push(ImportRecord {
            kind: "from-import".into(),
            path: "otherlib".into(),
            alias: "tool".into(),
            site_line: 1,
            site_byte: 0,
        });
        let mut c = reg(2, "fetch");
        c.references[0].name = "fetch".into();
        c.imports.push(ImportRecord {
            kind: "from-import".into(),
            path: "@/lib/ai".into(),
            alias: "tool".into(),
            site_line: 1,
            site_byte: 0,
        });
        let mut lib =
            FileFacts::new(FileId::new(3), PathBuf::from("src/lib/ai.ts"), "typescript");
        lib.imports.push(ImportRecord {
            kind: "re-export".into(),
            path: "ai".into(),
            alias: "tool".into(),
            site_line: 1,
            site_byte: 0,
        });
        let mut rule = user_rule("vercel", "typescript", "ai");
        rule.registrars = vec!["tool".into()];
        let mut out = detect(&g, &[a, b, c, lib], &[rule]);
        // The built-in `vercel-ai` row takes the same sites; judge the
        // user rule alone.
        out.entries.retain(|e| e.framework == "vercel");
        assert_eq!(
            rooted(&out),
            vec!["a.search".to_string(), "c.fetch".to_string()]
        );
    }

    // A base type is the framework's only where the file declaring that
    // base imports it (or writes it qualified): a project's own `trait
    // Tool` is not an agent framework's, and a match up the chain is
    // judged in the file of the class that names it.
    #[test]
    fn a_base_type_counts_only_where_its_declaring_file_imports_it() {
        let mk = |id: u32, qn: &str, file: u32| {
            let mut n = node(id, qn, qn.rsplit("::").next().unwrap(), "rust");
            n.file = FileId::new(file);
            n.start_byte = 100 + id;
            n
        };
        let g = graph_with(vec![
            mk(0, "own::Grep::call", 0),
            mk(1, "agent::Search::call", 1),
            mk(2, "agent::Deep::call", 2),
            mk(3, "agent::Qual::call", 3),
        ]);
        let def = |qn: &str, id: u32, base: &str| DefRecord {
            simple_name: "call".into(),
            qualified_name: qn.into(),
            start_byte: 100 + id,
            base_types: vec![base.into()],
            ..Default::default()
        };
        let mut own = FileFacts::new(FileId::new(0), PathBuf::from("src/own.rs"), "rust");
        own.definitions.push(def("own::Grep::call", 0, "Tool"));
        let mut agent = facts_with_import("rust", "src/agent.rs", "rig::tool::Tool");
        agent.file = FileId::new(1);
        agent
            .definitions
            .push(def("agent::Search::call", 1, "Tool"));
        // `Deep` extends `Search`, declared in a file that imports rig.
        let mut deep =
            FileFacts::new(FileId::new(2), PathBuf::from("src/deep.rs"), "rust");
        deep.definitions.push(def("agent::Deep::call", 2, "Search"));
        let mut qual =
            FileFacts::new(FileId::new(3), PathBuf::from("src/qual.rs"), "rust");
        qual.definitions
            .push(def("agent::Qual::call", 3, "rig::tool::Tool"));
        let mut rule = user_rule("rig", "rust", "rig");
        rule.base_types = vec!["Tool".into()];
        rule.methods = vec!["call".into()];
        let out = detect(&g, &[own, agent, deep, qual], &[rule]);
        assert_eq!(
            rooted(&out),
            vec![
                "agent::Deep::call".to_string(),
                "agent::Qual::call".to_string(),
                "agent::Search::call".to_string(),
            ]
        );
    }

    // Evidence a file carries without importing the framework by its own
    // name: a Rust crate re-exporting it (`shuttle_axum::axum`), and a JVM
    // class in the framework's own package, which sees its siblings bare.
    #[test]
    fn re_exporting_crates_and_own_packages_carry_evidence() {
        let mut h = node(0, "app::health", "health", "rust");
        h.file = FileId::new(0);
        let g = graph_with(vec![h]);
        let mut f =
            facts_with_import("rust", "src/main.rs", "shuttle_axum::axum::routing::get");
        f.references.push(RefRecord {
            name: "health".into(),
            receiver_hint: VALUE_REF_HINT.to_string(),
            site_line: 9,
            site_byte: 90,
            context: "get".into(),
            route: "/health".into(),
            ..Default::default()
        });
        let mut rule = user_rule("myaxum", "rust", "axum");
        rule.kind = TrustKind::Network;
        rule.registrars = vec!["get".into()];
        let out = detect(&g, &[f], &[rule]);
        assert!(
            out.entries.iter().any(|e| e.framework == "myaxum"),
            "{:?}",
            out.entries
        );

        let mut v = node(
            0,
            "org.acme.batch.core.RunIdIncrementer.getNext",
            "getNext",
            "java",
        );
        v.start_byte = 100;
        let g = graph_with(vec![v]);
        let mut f = FileFacts::new(
            FileId::new(0),
            PathBuf::from("RunIdIncrementer.java"),
            "java",
        );
        f.packages = vec!["org.acme.batch.core".into()];
        f.definitions.push(DefRecord {
            simple_name: "getNext".into(),
            qualified_name: "org.acme.batch.core.RunIdIncrementer.getNext".into(),
            start_byte: 100,
            base_types: vec!["JobParametersIncrementer".into()],
            ..Default::default()
        });
        let mut rule = user_rule("mybatch", "java", "org.acme.batch");
        rule.base_types = vec!["JobParametersIncrementer".into()];
        let out = detect(&g, &[f], &[rule]);
        assert_eq!(
            rooted(&out),
            vec!["org.acme.batch.core.RunIdIncrementer.getNext"]
        );
    }

    // Detection asks the tree first: an import of the project's own
    // `agents` package is not the OpenAI Agents SDK, while a module that
    // merely shares a framework's name (`proj/celery.py`) leaves the real
    // `celery` import detected.
    #[test]
    fn an_in_tree_module_named_like_a_framework_does_not_detect_it() {
        let mut tool = node(0, "src.agents.documenter.run", "run", "python");
        tool.attributes = vec!["@function_tool".into()];
        let mut task = node(1, "proj.tasks.send", "send", "python");
        task.attributes = vec!["@shared_task".into()];
        task.file = FileId::new(3);
        let g = graph_with(vec![tool, task]);
        let facts = vec![
            py_import(
                0,
                "src/handlers/readme.py",
                "from-import",
                "agents.documenter",
                "Doc",
            ),
            py_import(1, "src/agents/__init__.py", "import", "os", ""),
            py_import(2, "src/agents/documenter.py", "import", "os", ""),
            py_import(3, "proj/tasks.py", "from-import", "celery", "shared_task"),
            py_import(4, "proj/__init__.py", "import", "os", ""),
            py_import(5, "proj/celery.py", "from-import", "celery", "Celery"),
            py_import(6, "src/__init__.py", "import", "os", ""),
        ];
        let mut sdk = user_rule("agents-sdk", "python", "agents");
        sdk.attributes = vec!["function_tool".into()];
        let mut celery = user_rule("mycelery", "python", "celery");
        celery.attributes = vec!["shared_task".into()];
        let out = detect(&g, &facts, &[sdk, celery]);
        let ids: Vec<&str> = out.entries.iter().map(|e| e.framework.as_str()).collect();
        assert!(!ids.contains(&"agents-sdk"), "{ids:?}");
        assert!(ids.contains(&"mycelery"), "{ids:?}");
        assert!(
            !out.coverage
                .seen_no_rules
                .iter()
                .any(|f| f.id == "agents-sdk"),
            "not detected at all: {:?}",
            out.coverage
        );
    }

    // A framework's own repository: its in-tree `flask` package defines
    // the rule's own vocabulary (`route`), so its tests' `from flask
    // import Flask` still detects flask.
    #[test]
    fn a_frameworks_own_repository_still_detects_it() {
        let mut g = Graph::new();
        for (id, path) in [(0u32, "src/flask/app.py"), (1, "tests/test_basic.py")] {
            g.add_file(cgg_core::graph::FileRecord {
                id: FileId::new(id),
                path: PathBuf::from(path),
                language: "python".into(),
                ..Default::default()
            });
        }
        let mut route = node(0, "flask.app.Flask.route", "route", "python");
        route.file = FileId::new(0);
        let mut view = node(1, "test_basic.index", "index", "python");
        view.file = FileId::new(1);
        view.attributes = vec!["@app.route(\"/\")".into()];
        g.add_callable(route);
        g.add_callable(view);
        let facts = vec![
            py_import(0, "src/flask/app.py", "import", "os", ""),
            py_import(1, "tests/test_basic.py", "from-import", "flask", "Flask"),
        ];
        let mut rule = user_rule("myflask", "python", "flask");
        rule.attributes = vec!["route".into()];
        let out = detect(&g, &facts, &[rule]);
        assert!(
            out.entries.iter().any(|e| e.framework == "myflask"),
            "{:?}",
            out.entries
        );
    }

    // An ambiguous verb needs a route or a bare call — unless the rule
    // names the receiver it was called on, which confirms it already:
    // `server.resource(handler)` for a rule registering `server.resource`.
    #[test]
    fn a_rule_naming_the_receiver_corroborates_an_ambiguous_verb() {
        let h = node(0, "srv.read_config", "read_config", "python");
        let g = graph_with(vec![h]);
        let mk = |context: &str| {
            let mut f = facts_with_import("python", "srv.py", "mytools");
            f.references.push(RefRecord {
                name: "read_config".into(),
                receiver_hint: VALUE_REF_HINT.to_string(),
                site_line: 4,
                site_byte: 40,
                context: context.into(),
                ..Default::default()
            });
            f
        };
        let mut qualified = user_rule("q", "python", "mytools");
        qualified.registrars = vec!["server.resource".into()];
        let mut bare = user_rule("b", "python", "mytools");
        bare.registrars = vec!["resource".into()];
        assert_eq!(
            detect(&g, &[mk("app.server.resource")], &[qualified.clone()])
                .entries
                .len(),
            1
        );
        assert!(
            detect(&g, &[mk("cache.resource")], &[qualified])
                .entries
                .is_empty()
        );
        assert!(
            detect(&g, &[mk("server.resource")], &[bare])
                .entries
                .is_empty()
        );
    }

    fn named_members_of(attr: &str) -> NamedMembers {
        named_members(attr, &[])
    }

    fn jvm_class(
        id: u32,
        file: u32,
        qn: &str,
        base: &str,
        path: &str,
        imports: &[&str],
    ) -> (cgg_core::graph::CallableNode, FileFacts) {
        let simple = qn.rsplit('.').next().unwrap();
        let mut n = node(id, qn, simple, "kotlin");
        n.file = FileId::new(file);
        n.start_byte = 100 + id;
        let mut f = FileFacts::new(FileId::new(file), PathBuf::from(path), "kotlin");
        for i in imports {
            f.imports.push(ImportRecord {
                kind: "import".into(),
                path: (*i).into(),
                alias: String::new(),
                site_line: 1,
                site_byte: 0,
            });
        }
        f.definitions.push(DefRecord {
            simple_name: simple.into(),
            qualified_name: qn.into(),
            start_byte: 100 + id,
            base_types: vec![base.into()],
            ..Default::default()
        });
        (n, f)
    }

    // Fixup: a base type the language imports implicitly (`java.lang` /
    // Kotlin default imports) is present in every file, so a `Runnable`
    // in a file importing nothing from java.util.concurrent is still the
    // rule's.
    #[test]
    fn an_implicitly_imported_base_type_needs_no_import() {
        let (n0, f0) = jvm_class(0, 0, "a.Job.run", "Runnable", "Job.kt", &[]);
        let (n1, f1) = jvm_class(1, 1, "b.Pool.call", "Callable", "Pool.kt", &[]);
        let (_, f2) = jvm_class(
            9,
            2,
            "c.X.y",
            "Any",
            "C.kt",
            &["java.util.concurrent.Executors"],
        );
        let g = graph_with(vec![n0, n1]);
        let mut rule = user_rule("conc", "kotlin", "java.util.concurrent");
        rule.base_types = vec!["Runnable".into(), "Callable".into()];
        rule.methods = vec!["run".into(), "call".into()];
        let out = detect(&g, &[f0, f1, f2], &[rule]);
        // `Callable` is java.util.concurrent's: still needs the import.
        let got: Vec<String> = out
            .entries
            .iter()
            .filter(|e| e.framework == "conc")
            .map(|e| e.target_name.clone())
            .collect();
        assert_eq!(got, vec!["a.Job.run".to_string()]);
    }

    // Fixup: a gRPC service file imports only its generated stub
    // (`…GrpcKt`), which a `detect` name pattern accepts as evidence.
    #[test]
    fn a_generated_stub_import_is_framework_evidence() {
        let (n0, f0) = jvm_class(
            0,
            0,
            "svc.Greeter.sayHello",
            "GreeterGrpcKt.GreeterCoroutineImplBase",
            "Greeter.kt",
            &["com.acme.proto.GreeterGrpcKt"],
        );
        let g = graph_with(vec![n0]);
        let mut rule = user_rule("grpcish", "kotlin", "io.grpc");
        rule.detect.push("*GrpcKt".into());
        rule.base_types = vec!["*ImplBase".into()];
        let out = detect(&g, &[f0], &[rule]);
        assert!(
            out.entries.iter().any(|e| e.framework == "grpcish"),
            "{:?}",
            out.entries
        );
        assert!(
            import_segments("com.x.GreeterGrpcKt.Base").any(|s| s == "GreeterGrpcKt")
        );
        assert!(
            import_segments("proto/helloworld.grpc.pb.h")
                .any(|s| name_pattern_matches("*.grpc.pb.h", s))
        );
    }

    #[test]
    fn attribute_arguments_name_members_and_owner_types() {
        let m = named_members_of("@MethodSource({\"alpha\", \"demo.Shared#beta(int)\"})");
        assert_eq!(
            m.members,
            vec![
                (None, "alpha".to_string()),
                (Some("Shared".to_string()), "beta".to_string())
            ]
        );
        let m = named_members_of("TestCaseSource(typeof(Other), nameof(Other.Remote))");
        assert_eq!(m.owner_type.as_deref(), Some("Other"));
        assert_eq!(
            m.members,
            vec![(Some("Other".to_string()), "Remote".to_string())]
        );
        let m =
            named_members_of("MemberData(nameof(Ext.Data), MemberType = typeof(Ext))");
        assert_eq!(m.owner_type.as_deref(), Some("Ext"));
        assert_eq!(
            m.members,
            vec![(Some("Ext".to_string()), "Data".to_string())]
        );
        // Later positionals are parameters for the source, not names.
        let m = named_members_of("MemberData(nameof(Data), \"abc\", 3)");
        assert_eq!(m.members, vec![(None, "Data".to_string())]);
        // Java's implicit `value` element is the positional one.
        let m = named_members_of("@MethodSource(value = \"rows\")");
        assert_eq!(m.members, vec![(None, "rows".to_string())]);
        assert_eq!(named_members_of("@MethodSource"), NamedMembers::default());
        let m = named_members_of("@ArgumentsSource(Rows.class)");
        assert_eq!(m.owner_type.as_deref(), Some("Rows"));
        assert!(m.members.is_empty());
    }

    // B4: a data source is named by a string on the test, not called.
    #[test]
    fn method_source_roots_the_named_member_and_not_its_neighbours() {
        let mut t1 = node(0, "demo.T.named", "named", "java");
        t1.attributes = vec![
            "@ParameterizedTest".into(),
            "@MethodSource(\"words\")".into(),
        ];
        let mut t2 = node(1, "demo.T.same", "same", "java");
        t2.attributes = vec!["@MethodSource".into()];
        let g = graph_with(vec![
            t1,
            t2,
            node(2, "demo.T.words", "words", "java"),
            node(3, "demo.T.same", "same", "java"),
            node(4, "demo.T.unused", "unused", "java"),
            // Same member name on another class: not the source.
            node(5, "demo.U.words", "words", "java"),
        ]);
        let facts = vec![facts_with_import(
            "java",
            "T.java",
            "org.junit.jupiter.params.provider.MethodSource",
        )];
        let out = detect(&g, &facts, &[]);
        let ids: Vec<u32> = out
            .entries
            .iter()
            .filter(|e| e.framework == "junit-sources")
            .map(|e| e.target.as_u32())
            .collect();
        assert!(ids.contains(&2), "{:?}", out.entries);
        assert!(ids.contains(&3), "bare @MethodSource names its own name");
        assert!(
            !ids.contains(&1),
            "the annotated test is not its own source"
        );
        assert!(!ids.contains(&4) && !ids.contains(&5), "{:?}", out.entries);
        // WS18: a test data source is entered by the harness, so it hangs
        // off the harness's one node like the tests themselves.
        assert!(
            out.entries
                .iter()
                .filter(|e| e.framework == "junit-sources")
                .all(|e| e.node
                    && e.node_name() == "<framework-entry>::test::junit-sources"),
            "{:?}",
            out.entries
        );
    }

    #[test]
    fn member_sources_need_the_framework_detected() {
        let mut t = node(0, "demo.T.named", "named", "java");
        t.attributes = vec!["@MethodSource(\"words\")".into()];
        let g = graph_with(vec![t, node(1, "demo.T.words", "words", "java")]);
        let facts = vec![facts_with_import("java", "T.java", "java.util.List")];
        assert!(rooted(&detect(&g, &facts, &[])).is_empty());
    }

    #[test]
    fn a_named_fallback_element_names_the_member_and_its_class() {
        let sentinel = rules::member_sources_for("fault-tolerance", "java")
            .iter()
            .find(|s| s.attribute == "SentinelResource")
            .unwrap();
        let m = named_members(
            "@SentinelResource(value = \"res\", blockHandler = \"onBlock\", \
             blockHandlerClass = Handlers.class, fallback = \"onError\")",
            sentinel.named,
        );
        // `value` is the resource, never a member; the paired class names
        // only `blockHandler`'s owner.
        assert_eq!(
            m.members,
            vec![
                (Some("Handlers".to_string()), "onBlock".to_string()),
                (None, "onError".to_string()),
            ]
        );
        assert_eq!(m.owner_type, None);
        let fallback = rules::member_sources_for("fault-tolerance", "kotlin")
            .iter()
            .find(|s| s.attribute == "Fallback")
            .unwrap();
        let m = named_members("@Fallback(Recovery::class)", fallback.named);
        assert_eq!(m.owner_type.as_deref(), Some("Recovery"));
        assert!(m.members.is_empty());
    }

    // A fallback is called by the library by name: without the row it is
    // reported dead at `high`.
    #[test]
    fn a_resilience4j_fallback_method_is_rooted_and_its_neighbours_are_not() {
        let mut call = node(0, "demo.Svc.call", "call", "java");
        call.attributes = vec![
            "@CircuitBreaker(name = \"backend\", fallbackMethod = \"recover\")".into(),
        ];
        let g = graph_with(vec![
            call,
            node(1, "demo.Svc.recover", "recover", "java"),
            node(2, "demo.Svc.backend", "backend", "java"),
            node(3, "demo.Other.recover", "recover", "java"),
        ]);
        let facts = vec![facts_with_import(
            "java",
            "Svc.java",
            "io.github.resilience4j.circuitbreaker.annotation.CircuitBreaker",
        )];
        let out = detect(&g, &facts, &[]);
        let ids: Vec<u32> = out
            .entries
            .iter()
            .filter(|e| e.framework == "fault-tolerance")
            .map(|e| e.target.as_u32())
            .collect();
        assert_eq!(ids, vec![1], "{:?}", out.entries);
        assert!(out.entries.iter().all(|e| !e.node));
    }

    #[test]
    fn xunit_and_nunit_sources_follow_nameof_and_typeof() {
        let mut a = node(0, "Demo.X.Member", "Member", "csharp");
        a.attributes =
            vec!["MemberData(nameof(Ext.Rows), MemberType = typeof(Ext))".into()];
        let mut b = node(1, "Demo.X.Cls", "Cls", "csharp");
        b.attributes = vec!["ClassData(typeof(Gen))".into()];
        let g = graph_with(vec![
            a,
            b,
            node(2, "Demo.Ext.Rows", "Rows", "csharp"),
            node(3, "Demo.Gen.GetEnumerator", "GetEnumerator", "csharp"),
            node(4, "Demo.Gen.Other", "Other", "csharp"),
        ]);
        let facts = vec![facts_with_import("csharp", "X.cs", "Xunit")];
        assert_eq!(
            rooted(&detect(&g, &facts, &[])),
            vec![
                "Demo.Ext.Rows".to_string(),
                "Demo.Gen.GetEnumerator".to_string()
            ]
        );
    }

    #[test]
    fn serialization_hooks_match_by_name_and_parameter_list() {
        let mut r = node(0, "demo.Rec.readObject", "readObject", "java");
        r.signature_hint =
            "private void readObject(final java.io.ObjectInputStream in) throws IOException"
                .into();
        let mut w = node(1, "demo.Rec.writeReplace", "writeReplace", "java");
        w.signature_hint = "private Object writeReplace()".into();
        // Near misses: the name without the serialization signature.
        let mut n1 = node(2, "demo.Io.readObject", "readObject", "java");
        n1.signature_hint = "private Object readObject(String path)".into();
        let mut n2 = node(3, "demo.Io.readResolve", "readResolve", "java");
        n2.signature_hint = "Object readResolve(int depth)".into();
        let g = graph_with(vec![r, w, n1, n2]);
        let facts = vec![facts_with_import(
            "java",
            "Rec.java",
            "java.io.Serializable",
        )];
        assert_eq!(
            rooted(&detect(&g, &facts, &[])),
            vec![
                "demo.Rec.readObject".to_string(),
                "demo.Rec.writeReplace".to_string()
            ]
        );
    }

    #[test]
    fn lifecycle_annotations_root_only_the_annotated_method() {
        let mut a = node(0, "demo.S.init", "init", "java");
        a.attributes = vec!["@PostConstruct".into()];
        let mut b = node(1, "demo.S.make", "make", "java");
        b.attributes = vec!["@Provides".into()];
        let g = graph_with(vec![a, b, node(2, "demo.S.initLater", "initLater", "java")]);
        let facts = vec![
            facts_with_import("java", "S.java", "javax.annotation.PostConstruct"),
            facts_with_import("java", "M.java", "com.google.inject.Provides"),
        ];
        let out = detect(&g, &facts, &[]);
        assert_eq!(
            rooted(&out),
            vec!["demo.S.init".to_string(), "demo.S.make".to_string()]
        );
        assert!(
            out.entries.iter().all(|e| !e.node),
            "lifecycle mints no node"
        );
    }

    /// WS24: a Kotlin annotation belongs to the framework only in a file
    /// that imports it, or where it is written qualified. The near miss is
    /// a same-named annotation of the file's own package.
    #[test]
    fn jvm_annotations_match_only_where_the_file_imports_them() {
        let mut imported = node(0, "app.Jobs.tick", "tick", "kotlin");
        imported.attributes = vec!["@Scheduled(fixedRate = 1000)".into()];
        let mut own = node(1, "nearmiss.Local.tick", "tick", "kotlin");
        own.attributes = vec!["@Scheduled".into()];
        own.file = FileId::new(1);
        let mut qualified = node(2, "nearmiss.Local.fq", "fq", "kotlin");
        qualified.attributes =
            vec!["@org.springframework.scheduling.annotation.Scheduled".into()];
        qualified.file = FileId::new(1);
        let g = graph_with(vec![imported, own, qualified]);
        let facts = vec![
            facts_with_import(
                "kotlin",
                "Jobs.kt",
                "org.springframework.scheduling.annotation.Scheduled",
            ),
            FileFacts::new(FileId::new(1), PathBuf::from("NearMiss.kt"), "kotlin"),
        ];
        assert_eq!(
            rooted(&detect(&g, &facts, &[])),
            vec!["app.Jobs.tick".to_string(), "nearmiss.Local.fq".to_string()]
        );

        // A Gradle Kotlin script carries its host's default imports.
        let mut task = node(0, "Greet.run", "run", "kotlin");
        task.attributes = vec!["@Scheduled".into()];
        let mut g = graph_with(vec![task]);
        g.files
            .values_mut()
            .for_each(|f| f.path = PathBuf::from("build.gradle.kts"));
        let mut other = facts_with_import(
            "kotlin",
            "Jobs.kt",
            "org.springframework.scheduling.annotation.Scheduled",
        );
        other.file = FileId::new(1);
        let script =
            FileFacts::new(FileId::new(0), PathBuf::from("build.gradle.kts"), "kotlin");
        assert_eq!(
            rooted(&detect(&g, &[script, other], &[])),
            vec!["Greet.run".to_string()]
        );
    }

    /// WS24: Kotlin Android components are matched by their recorded
    /// supertype, as in Java; the method name alone is not enough.
    #[test]
    fn kotlin_android_lifecycle_matches_through_the_supertype() {
        let mut f = facts_with_import(
            "kotlin",
            "MainAct.kt",
            "androidx.appcompat.app.AppCompatActivity",
        );
        let mut nodes = Vec::new();
        for (i, (qn, name, base)) in [
            ("app.MainAct.onCreate", "onCreate", "AppCompatActivity"),
            ("app.MainAct.helper", "helper", "AppCompatActivity"),
            ("app.Plain.onCreate", "onCreate", "Widget"),
        ]
        .into_iter()
        .enumerate()
        {
            let mut n = node(i as u32, qn, name, "kotlin");
            n.start_byte = i as u32 * 10;
            f.definitions.push(DefRecord {
                simple_name: name.into(),
                qualified_name: qn.into(),
                start_byte: n.start_byte,
                base_types: vec![base.to_string()],
                ..Default::default()
            });
            nodes.push(n);
        }
        let out = detect(&graph_with(nodes), &[f], &[]);
        let got: Vec<(u32, &str)> = out
            .entries
            .iter()
            .map(|e| (e.target.as_u32(), e.framework.as_str()))
            .collect();
        assert_eq!(got, vec![(0, "android-activity")], "{:?}", out.entries);
    }

    /// WS24: Kotlin's `@MethodSource` roots its provider, as Java's does.
    #[test]
    fn kotlin_method_source_roots_the_named_member() {
        let mut t = node(0, "app.KTest.p", "p", "kotlin");
        t.attributes = vec![
            "@ParameterizedTest".into(),
            "@MethodSource(\"provider\")".into(),
        ];
        let g = graph_with(vec![
            t,
            node(1, "app.KTest.provider", "provider", "kotlin"),
            node(2, "app.KTest.unusedSource", "unusedSource", "kotlin"),
        ]);
        let facts = vec![facts_with_import(
            "kotlin",
            "KTest.kt",
            "org.junit.jupiter.params.provider.MethodSource",
        )];
        let out = detect(&g, &facts, &[]);
        let sources: Vec<u32> = out
            .entries
            .iter()
            .filter(|e| e.framework == "junit-sources")
            .map(|e| e.target.as_u32())
            .collect();
        assert_eq!(sources, vec![1], "{:?}", out.entries);
    }

    fn block_ref(name: &str, context: &str, line: u32) -> RefRecord {
        RefRecord {
            property_read: false,
            name: name.into(),
            receiver_hint: VALUE_REF_HINT.to_string(),
            site_line: line,
            site_byte: line * 10,
            context: context.into(),
            ..Default::default()
        }
    }

    /// WS24: a test file's block callbacks belong to the harness it
    /// imports, else to the tree's globals harness; never to every
    /// harness that shares the verb.
    #[test]
    fn a_test_file_block_belongs_to_one_harness() {
        // globals file: jest (a config file detects it)
        let mut a = node(0, "handler_at_3", "handler_at_3", "javascript");
        a.file = FileId::new(0);
        // a file importing playwright
        let mut b = node(1, "handler_at_5", "handler_at_5", "javascript");
        b.file = FileId::new(1);
        let g = graph_with(vec![a, b]);
        let mut f0 =
            FileFacts::new(FileId::new(0), PathBuf::from("a.test.js"), "javascript");
        f0.references.push(RefRecord {
            property_read: false,
            name: "it".into(),
            site_line: 3,
            site_byte: 30,
            ..Default::default()
        });
        f0.references.push(block_ref("handler_at_3", "it", 3));
        let mut f1 = facts_with_import("javascript", "b.spec.js", "@playwright/test");
        f1.file = FileId::new(1);
        f1.references
            .push(block_ref("handler_at_5", "test.describe", 5));
        let cfg = FileFacts::new(
            FileId::new(2),
            PathBuf::from("jest.config.js"),
            "javascript",
        );
        let out = detect(&g, &[f0.clone(), f1.clone(), cfg], &[]);
        let got: Vec<(u32, &str)> = out
            .entries
            .iter()
            .map(|e| (e.target.as_u32(), e.framework.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![(0, "jest"), (1, "playwright")],
            "{:?}",
            out.entries
        );
        assert!(out.entries.iter().all(|e| e.node), "{:?}", out.entries);

        // No harness import or config anywhere: the anonymous globals rule.
        let out = detect(&g, &[f0], &[]);
        let got: Vec<(u32, &str)> = out
            .entries
            .iter()
            .map(|e| (e.target.as_u32(), e.framework.as_str()))
            .collect();
        assert_eq!(got, vec![(0, "js-test-globals")], "{:?}", out.entries);
    }

    #[test]
    fn nested_base_pattern_takes_protoc_service_bases_only() {
        assert!(nested_base_matches("$.$Base", "Basket.BasketBase"));
        assert!(nested_base_matches("$.$Base", "eShop.Basket.BasketBase"));
        assert!(nested_base_matches("$.$Base", "Greeter.GreeterBase<T>"));
        // Near misses: another nesting, a bare `*Base`, a plain pattern.
        assert!(!nested_base_matches("$.$Base", "Store.RepoBase"));
        assert!(!nested_base_matches("$.$Base", "BasketBase"));
        assert!(!nested_base_matches("Base", "Basket.BasketBase"));
    }

    #[test]
    fn declared_param_types_reads_type_first_and_name_first_lists() {
        assert_eq!(
            declared_param_types(
                "public string FunctionHandler(string input, ILambdaContext context)",
                "FunctionHandler"
            ),
            Some(vec!["string".to_string(), "ILambdaContext".to_string()])
        );
        assert_eq!(
            declared_param_types(
                "public async Task<R> H<T>(Dictionary<string, int> m, Amazon.Lambda.Core.ILambdaContext c) => x",
                "H"
            ),
            Some(vec!["Dictionary".to_string(), "ILambdaContext".to_string()])
        );
        // F#: curried and tupled, with the body after `=` never read.
        assert_eq!(
            declared_param_types(
                "let PostFormat (request: APIGatewayProxyRequest) (_context: ILambdaContext) = f (x: Other)",
                "PostFormat"
            ),
            Some(vec![
                "APIGatewayProxyRequest".to_string(),
                "ILambdaContext".to_string()
            ])
        );
        assert_eq!(
            declared_param_types(
                "member this.Run(req: string, ctx: ILambdaContext)",
                "Run"
            ),
            Some(vec!["string".to_string(), "ILambdaContext".to_string()])
        );
        assert_eq!(declared_param_types("void Other()", "Missing"), None);
    }
}
