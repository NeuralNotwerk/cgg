// Pipeline helpers thread run state explicitly rather than through a
// context struct, which keeps each stage's inputs visible at the call
// site. The arity is the point, not an accident.
#![allow(clippy::too_many_arguments)]
// See cgg-lang/src/lib.rs: the `..Default::default()` spreads are
// deliberate source-compatibility for future optional fields.
#![allow(clippy::needless_update)]
//! The `cgg` pipeline, as a library.
//!
//! ```text
//! walker -> language detector -> parser pool -> extract
//!        -> build Graph -> resolvers -> query -> format + audit
//! ```
//!
//! `src/main.rs` is a thin shim over this: it parses the command line,
//! installs the tracing subscriber and the global allocator, and calls
//! [`analyze`]. Everything that decides what the graph *contains* lives
//! here, so a second consumer — the Python extension module in
//! `crates/cgg-py` — runs the identical pipeline in the identical order
//! rather than reassembling one from the library crates. CLAUDE.md is
//! explicit that the resolver ordering is load-bearing; a second copy of
//! it would drift from this one silently.

pub mod cli;
pub mod emit;
// Public: a library caller naming `--rollup-by` needs to name a
// `RollupLevel`, and `RunOptions` carries one.
pub mod rollup;

// Private: consumers reach these types through the re-exports below, and
// the modules themselves have no out-of-crate callers.
mod deadcode;
mod options;
mod outcome;
mod query;
mod replay;
mod since;
mod stable_ids;

use stable_ids::StableIds;

pub use options::RunOptions;
pub use outcome::{Emission, RunOutcome};

/// Everything needed to call this crate, re-exported.
///
/// A consumer had to add `cgg-format` and `anyhow` to its own manifest
/// just to *use* the API: `analyze` returns `anyhow::Result`, and
/// [`emit::graph_to_string`] takes a `cgg_format::OutputFormat`. Naming a
/// dependency you never `use` directly is a papercut, and worse, it lets
/// a consumer's version drift from the one this crate was built against.
///
/// These are the same types, not wrappers — `cgg::OutputFormat` *is*
/// `cgg_format::OutputFormat`, so a value crosses between them freely.
/// The graph types are re-exported below, on the imports this module
/// already needed, rather than listed twice.
pub use anyhow::Error;
pub use cgg_core as core;
pub use cgg_format::OutputFormat;

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Context;
pub use anyhow::Result;

use rayon::prelude::*;

use cgg_core::audit::{
    AuditCallableRef, AuditEvent, AuditFileRecord, RunMetrics, SkipReason,
};
use cgg_core::graph::FileRecord as GraphFileRecord;
pub use cgg_core::graph::{CallEdge, CallableKind, CallableNode, Confidence, Graph, Via};
use cgg_core::ids::ResolverId;
pub use cgg_core::ids::{CallableId, FileId};
use cgg_core::{
    DefVariant, FileAliases, FileFacts, build_known_names, classify_external,
};
use cgg_lang::{
    PluginRegistry,
    detect::{DetectVerdict, LanguageDetector},
    parser::ParserPool,
};
use cgg_resolve::intra_file::{DefIdMap, link_file};
use cgg_walk::{WalkConfig, walk};

/// Every language id cgg can analyze, in registry order.
///
/// Read off the plugin registry rather than listed anywhere. The count has
/// changed with almost every release, and `scripts/docs-check.py` exists
/// because every hand-maintained copy of it in this repo has been wrong at
/// some point.
pub fn plugin_ids() -> Vec<&'static str> {
    PluginRegistry::with_v1_plugins()
        .all()
        .iter()
        .map(|p| p.id())
        .collect()
}

/// Run the whole pipeline and return what it produced.
///
/// Performs no I/O beyond reading the source tree: no writes, no stdout,
/// no stderr, no `process::exit`. Diagnostics come back as
/// [`RunOutcome::notices`], and everything the run writes — diagnostics and
/// artifacts alike — as [`RunOutcome::transcript`]. `cgg`'s own front end is
/// [`emit::all`] applied to the result.
///
/// Runs inside a worker pool sized by [`RunOptions::jobs`], which every
/// `par_iter` in the pipeline inherits — including those in
/// `cgg_resolve::cross_file` and `cgg_resolve::frameworks`. Per-call, not
/// global: the global pool can only be set once, so a second call would
/// silently reuse the first's thread count.
///
/// Safe to call concurrently. Extraction's two switches — the dead-code
/// signals and the user-supplied registrar verbs — travel in a
/// [`cgg_lang::ExtractCtx`] built per run, so there is no shared state for
/// two analyses to fight over. They were process-globals through 0.5.0,
/// which forced this function to hold a process-wide lock for its whole
/// duration.
/// The default duck-typing fan-out cap, re-exported so front ends can
/// name the same default the CLI does rather than hard-coding 5.
pub fn cross_file_default_fanout_cap() -> u32 {
    cgg_resolve::cross_file::DEFAULT_FANOUT_CAP as u32
}

pub fn analyze(opts: &RunOptions) -> Result<RunOutcome> {
    // A replay reads a finished graph instead of a source tree, so there
    // is nothing here for a worker pool to do — building one would cost
    // more than the whole run.
    if opts.from_graph.is_some() {
        return replay::replay(opts);
    }
    // `jobs: 0` means half the PHYSICAL cores. rayon defaults to one per
    // LOGICAL cpu, which on an SMT machine is double that — and cgg's hot
    // loops are allocator-bound, so siblings contend rather than add.
    let jobs = if opts.jobs > 0 {
        opts.jobs
    } else {
        cgg_core::cpu::default_jobs()
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build()
        .context("building the worker thread pool")?;
    pool.install(|| analyze_in_pool(opts))
}

/// The pipeline proper. Always called inside [`analyze`]'s thread pool.
fn analyze_in_pool(opts: &RunOptions) -> Result<RunOutcome> {
    // No "cgg starting" line here. It names the output format, which is a
    // `Cli` concern that never reaches `RunOptions`, and emitting an
    // application's startup banner is the application's call for the same
    // reason installing the subscriber is. `main.rs` logs it.
    if opts.paths.is_empty() {
        return Err(anyhow::anyhow!("no input paths given"));
    }

    // Diagnostics, in the order the run produces them. `emit::all` writes
    // these verbatim; nothing here touches a file descriptor.
    let mut transcript: Vec<Emission> = Vec::new();

    // Read here rather than in `analyze`: this function already runs inside
    // the pool, so this is the width the work actually ran at.
    let jobs = rayon::current_num_threads();

    for p in &opts.paths {
        if !p.exists() {
            return Err(anyhow::anyhow!(
                "input path does not exist: {}",
                p.display()
            ));
        }
    }

    // Dead-code mode biases every knob toward false negatives. Edges
    // that only ever mark something live are switched on; node-only
    // additions, which would be reported as findings themselves, are
    // switched off. The rule: enable anything that can add inbound
    // edges, disable anything that can only add nodes.
    // `--why-live` and `--write-roots` are both questions about the
    // dead-code model, so they turn it on rather than requiring the user
    // to remember `--dead-code` as well. `--write-roots` without it used
    // to emit an ordinary graph — a silent no-op wearing the costume of
    // a baseline.
    let dead_mode = opts.dead_mode();
    // Shadowed so no later `opts.` read can bypass the adjustment.
    let opts = &if dead_mode {
        RunOptions {
            reference_edges: true,
            dynamic_dispatch: true,
            include_external: false,
            include_stdlib: false,
            ..opts.clone()
        }
    } else {
        opts.clone()
    };

    // The config file is read on every run, not only in dead-code mode:
    // it carries local framework rules, and entry nodes are on by
    // default. Discovery starts from the analyzed paths so that
    // `cgg /path/to/project` from elsewhere still finds that project's
    // rules — searching only the working directory made a present config
    // silently do nothing.
    let config_path = match &opts.roots {
        Some(p) => Some(p.clone()),
        None => deadcode::config::DeadCodeConfigFile::discover_for(
            &opts.paths,
            std::env::current_dir().ok().as_deref(),
        ),
    };
    let config = match &config_path {
        Some(p) => deadcode::config::DeadCodeConfigFile::load(p)?,
        None => deadcode::config::DeadCodeConfigFile::default(),
    };

    // Argument capture is gated on the built-in registrar verbs, so a user
    // rule naming a verb cgg does not ship would be inert without this.
    // Lowercased once here rather than compared case-insensitively per call
    // site, and owned by this run alone.
    let extra_verbs: std::collections::HashSet<String> = config
        .frameworks
        .iter()
        .flat_map(|r| r.registrars.iter())
        .map(|v| v.to_ascii_lowercase())
        .collect();
    // Every worker shares this by reference; extraction pays a pointer.
    let extract_ctx = cgg_lang::ExtractCtx::new(dead_mode, &extra_verbs);

    if opts.profile {
        cgg_core::profile::enable();
    }

    let started = Instant::now();

    // --- Phase 1: walk -----------------------------------------------------
    let cfg = WalkConfig {
        roots: opts.paths.clone(),
        extra_ignore_file: opts.ignore_file.clone(),
        ..Default::default()
    };
    let walk_started = Instant::now();
    let outcome = walk(&cfg).context("walking input paths")?;
    let walk_ms = walk_started.elapsed().as_secs_f64() * 1000.0;

    // --- Phase 2: detect + parse + extract --------------------------------
    let registry = PluginRegistry::with_v1_plugins();
    let detector = LanguageDetector::new(&registry);
    let pool = ParserPool::new(&registry);

    let mut events: Vec<AuditEvent> =
        Vec::with_capacity(outcome.candidates.len() + outcome.skips.len() + 2);
    events.push(AuditEvent::RunStarted {
        cgg_version: cgg_core::CGG_VERSION.to_string(),
        argv: std::env::args().collect(),
    });

    let metrics = RunMetrics {
        files_discovered: (outcome.candidates.len() + outcome.skips.len()) as u64,
        ..Default::default()
    };
    let mut metrics = metrics;

    let lang_filter: Vec<&str> = opts.lang.iter().map(|s| s.as_str()).collect();
    let langs_enabled =
        |lang: &str| -> bool { lang_filter.is_empty() || lang_filter.contains(&lang) };

    let parse_started = Instant::now();
    let mut stable_ids = StableIds::new();
    let id_roots = stable_ids::id_roots(&opts.paths);

    // Collected facts per file, used by the intra-file linker below.
    let mut all_facts: Vec<FileFacts> = Vec::new();
    // Audit-grade per-file records, enriched as we go.
    let mut file_records: Vec<AuditFileRecord> = Vec::new();

    // Graph under construction.
    let mut graph = Graph::new();
    // (FileId, def-index) -> CallableId.
    let mut def_ids: DefIdMap = DefIdMap::new();

    // --- Parallel phase: detect + read + hash + parse + extract -----------
    // Each candidate is processed independently; results are merged below.
    struct FileResult {
        path: std::path::PathBuf,
        lang: String,
        detected_via: String,
        hash: String,
        size_bytes: u64,
        lines: u32,
        parse_ms: f64,
        parse_status: String,
        facts: Option<FileFacts>,
    }
    // Clippy flags this as a large-variant enum (`FileResult` ~368
    // bytes vs `Skipped` ~56). Boxing would shrink the Vec element but
    // adds an allocation per analyzed file; measured at 2941ms either
    // way, i.e. inside the noise floor. Left unboxed because the
    // unmeasured version of this change is exactly the kind that gets
    // shipped on a plausible-sounding rationale.
    #[allow(clippy::large_enum_variant)]
    enum FileOutcome {
        Analyzed(FileResult),
        Skipped {
            path: std::path::PathBuf,
            reason: SkipReason,
        },
    }

    // The worker pool is built by `analyze` and entered with
    // `ThreadPool::install`, so every `par_iter` below — here, in type
    // propagation, in the intra-file link, and inside cross_file and
    // frameworks — inherits it. Nothing sets a *global* pool any more.
    let results: Vec<FileOutcome> = outcome
        .candidates
        .par_iter()
        .map(|cand| {
            let det = detector.detect(&cand.path);
            let lang = match det.verdict {
                DetectVerdict::Language(id) if langs_enabled(id) => id,
                DetectVerdict::Language(id) => {
                    return FileOutcome::Skipped {
                        path: cand.path.clone(),
                        reason: SkipReason::LanguageFilter(id.to_string()),
                    };
                }
                DetectVerdict::Unknown => {
                    return FileOutcome::Skipped {
                        path: cand.path.clone(),
                        reason: SkipReason::UnknownExtension,
                    };
                }
            };

            let raw_bytes = match read_file(&cand.path) {
                Ok(b) => b,
                Err(err) => {
                    return FileOutcome::Skipped {
                        path: cand.path.clone(),
                        reason: SkipReason::ParseError(err.to_string()),
                    };
                }
            };

            // `.ipynb` is JSON, not Python source. Pull the code cells
            // out so the Python plugin sees ordinary Python text.
            let bytes = if lang == "python"
                && cand.path.extension().and_then(|e| e.to_str()) == Some("ipynb")
            {
                match cgg_lang::notebook::extract_python_source(&raw_bytes) {
                    Some(transformed) => transformed,
                    None => {
                        return FileOutcome::Skipped {
                            path: cand.path.clone(),
                            reason: SkipReason::ParseError(
                                "ipynb: malformed notebook JSON".into(),
                            ),
                        };
                    }
                }
            } else {
                raw_bytes
            };

            let hash = blake3::hash(&bytes).to_hex().to_string();
            let line_count = count_lines(&bytes);

            let _sp = cgg_core::profile::span("parse::tree-sitter+extract");
            let (parse_status, parse_ms, facts) = match pool.parse(lang, &bytes) {
                Ok(out) => {
                    let status = if out.tree.root_node().has_error() {
                        "error"
                    } else {
                        "ok"
                    };
                    let plugin = pool.plugin(lang);
                    let facts = plugin.map(|p| {
                        let _s = cgg_core::profile::span("parse::extract");
                        // Narrow the registrar-verb gate to this file's
                        // language; the union of every rule in the table
                        // makes each language pay for the others'.
                        p.extract(
                            &extract_ctx.for_language(lang),
                            FileId::new(0),
                            &cand.path,
                            &out.tree,
                            &bytes,
                        )
                    });
                    (status.to_string(), out.parse_ms, facts)
                }
                Err(_) => ("error".to_string(), 0.0, None),
            };

            FileOutcome::Analyzed(FileResult {
                path: cand.path.clone(),
                lang: lang.to_string(),
                detected_via: det.detected_via.clone(),
                hash,
                size_bytes: cand.size_bytes,
                lines: line_count,
                parse_ms,
                parse_status,
                facts,
            })
        })
        .collect();

    // --- Sequential merge: assign IDs and build graph ---------------------
    let _phase_merge_graph_build = cgg_core::profile::span("merge::graph-build");
    for result in results {
        match result {
            FileOutcome::Skipped { path, reason } => {
                events.push(AuditEvent::FileDiscovered { path: path.clone() });
                events.push(AuditEvent::FileSkipped {
                    path,
                    reason: reason.clone(),
                });
                if matches!(reason, SkipReason::ParseError(_)) {
                    metrics.files_errored += 1;
                } else {
                    metrics.files_skipped += 1;
                }
            }
            FileOutcome::Analyzed(fr) => {
                events.push(AuditEvent::FileDiscovered {
                    path: fr.path.clone(),
                });
                metrics.bytes_processed += fr.size_bytes;

                // Relative to the analysis root, NOT the path as typed:
                // hashing the raw display path made an id depend on how
                // cgg was invoked. See `stable_ids::id_path`.
                let relative_path = stable_ids::id_path(&fr.path, &id_roots);
                let file_id = stable_ids.file(&relative_path);

                // Classify test code once, from (path, language), and
                // record it on both the graph and the audit so the
                // reason is inspectable rather than merely asserted.
                let test_role = cgg_core::classify_test_file(&fr.path, &fr.lang);

                graph.add_file(GraphFileRecord {
                    id: file_id,
                    path: fr.path.clone(),
                    language: fr.lang.clone(),
                    detected_via: fr.detected_via.clone(),
                    blake3: fr.hash.clone(),
                    size_bytes: fr.size_bytes,
                    lines: fr.lines,
                    parse_ms: fr.parse_ms,
                    parse_status: fr.parse_status.clone(),
                    test_role,
                    ..Default::default()
                });

                let mut file_audit = AuditFileRecord {
                    file: file_id,
                    test_role,
                    path: fr.path,
                    language: fr.lang.clone(),
                    detected_via: fr.detected_via,
                    blake3: fr.hash,
                    size_bytes: fr.size_bytes,
                    lines: fr.lines,
                    parse_ms: fr.parse_ms,
                    parse_status: fr.parse_status,
                    skip_reason: None,
                    callables: Vec::new(),
                    unresolved_calls: Vec::new(),
                    stdlib_calls: Vec::new(),
                    external_calls: Vec::new(),
                    ffi: Vec::new(),
                };

                if let Some(mut facts) = fr.facts {
                    // Fix the file ID (was placeholder 0 during parallel phase).
                    facts.file = file_id;
                    for (idx, d) in facts.definitions.iter().enumerate() {
                        let owner = cgg_resolve::names::owner_from_qn(&d.qualified_name);
                        // The signature is part of the key: without it,
                        // overloads sharing a qualified name hash
                        // identically — 17.7% of the benchmark corpus —
                        // and their ids fall to declaration order.
                        let cid = stable_ids.callable(
                            &fr.lang,
                            &relative_path,
                            owner,
                            &d.qualified_name,
                            &d.signature_hint,
                        );
                        def_ids.insert((file_id, idx as u32), cid);

                        graph.add_callable(CallableNode {
                            id: cid,
                            qualified_name: d.qualified_name.clone(),
                            simple_name: d.simple_name.clone(),
                            kind: variant_to_kind(d.variant),
                            language: fr.lang.clone(),
                            file: file_id,
                            start_line: d.start_line,
                            end_line: d.end_line,
                            start_byte: d.start_byte,
                            end_byte: d.end_byte,
                            signature_hint: d.signature_hint.clone(),
                            visibility: d.visibility.clone(),
                            vis: d.vis,
                            // A definition inside a test file is test
                            // code even when the plugin found no marker
                            // on the definition itself: Go puts tests in
                            // a separate file, Rust puts them inline, and
                            // neither signal alone covers both.
                            test_role: d.test_role.or_else(|| {
                                test_role.map(|_| cgg_core::TestRole::Support)
                            }),
                            attributes: d.attributes.clone(),
                            synthetic: d
                                .attributes
                                .iter()
                                .any(|a| a == "synthetic" || a.starts_with("derive:")),
                            trait_impl_target: trait_impl_target_from_qn(
                                &d.qualified_name,
                            ),
                            ..Default::default()
                        });

                        file_audit.callables.push(AuditCallableRef {
                            id: cid,
                            qualified_name: d.qualified_name.clone(),
                            kind: format!("{:?}", d.variant).to_lowercase(),
                            start_line: d.start_line,
                            end_line: d.end_line,
                            start_byte: d.start_byte,
                            end_byte: d.end_byte,
                        });
                    }

                    let lang_bucket =
                        metrics.by_language.entry(fr.lang.clone()).or_default();
                    lang_bucket.callables += facts.definitions.len() as u64;
                    metrics.callables += facts.definitions.len() as u64;

                    all_facts.push(facts);
                }

                metrics.files_analyzed += 1;
                metrics.phases.parse_ms += fr.parse_ms;
                metrics.by_language.entry(fr.lang).or_default().files += 1;

                file_records.push(file_audit);
            }
        }
    }

    // --- Phase 3: type propagation + intra-file link -----------------------
    let _phase_resolve_intra_file = cgg_core::profile::span("resolve::intra-file");
    let link_started = Instant::now();
    let return_types_owned: HashMap<String, String> = {
        let _s = cgg_core::profile::span("resolve::type-hints");
        cgg_resolve::type_hints::build_return_type_map(&all_facts)
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    };
    let return_types: HashMap<&str, &str> = return_types_owned
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    {
        // Per-file and independent: each call only mutates its own facts.
        let _s = cgg_core::profile::span("resolve::type-propagate");
        all_facts.par_iter_mut().for_each(|facts| {
            cgg_resolve::type_hints::propagate_types_with_returns(facts, &return_types);
        });
    }
    let known_names = build_known_names(&all_facts);
    // Hoisted out of the per-file loop below. It was rebuilt for every
    // file from the same `known_names`, which is O(files x names) — on
    // netbox that is 1,273 files times ~10,000 names, rebuilt 1,273
    // times to produce the identical set each pass.
    let known_refs: std::collections::HashSet<&str> =
        known_names.iter().map(|s| s.as_str()).collect();
    // Per-file and independent: `link_file` reads only its own facts
    // plus the shared `def_ids`, and `classify_external` is pure. Run
    // them in parallel and fold the results sequentially afterwards —
    // `par_iter().collect()` preserves input order, so the graph's edge
    // order, and therefore the whole output, stays byte-identical.
    type LinkRow = (
        cgg_core::ids::FileId,
        String,
        Vec<cgg_core::graph::CallEdge>,
        cgg_core::external::ClassifyResult,
    );
    let linked: Vec<LinkRow> = {
        let _s = cgg_core::profile::span("resolve::intra-file-parallel");
        all_facts
            .par_iter()
            .map(|facts| {
                let outcome = link_file(facts, &def_ids);
                let aliases = FileAliases::from_facts(facts);
                let mut per_file_aliases = std::collections::HashMap::new();
                per_file_aliases.insert(facts.file, aliases);
                let classified = classify_external(
                    outcome.unresolved,
                    &known_refs,
                    &facts.language,
                    Some(&per_file_aliases),
                );
                (
                    facts.file,
                    facts.language.clone(),
                    outcome.edges,
                    classified,
                )
            })
            .collect()
    };

    // The per-file audit record was found with a linear scan inside the
    // loop, which is O(files^2). Index it once.
    let rec_idx: std::collections::HashMap<cgg_core::ids::FileId, usize> = file_records
        .iter()
        .enumerate()
        .map(|(i, r)| (r.file, i))
        .collect();

    for (file, lang, edges, classified) in linked {
        let lang_bucket = metrics.by_language.entry(lang.clone()).or_default();
        lang_bucket.edges += edges.len() as u64;
        for e in &edges {
            match e.confidence {
                cgg_core::graph::Confidence::High => {
                    metrics.confidence_histogram.high += 1
                }
                cgg_core::graph::Confidence::Medium => {
                    metrics.confidence_histogram.medium += 1
                }
                cgg_core::graph::Confidence::Low => metrics.confidence_histogram.low += 1,
            }
        }
        metrics.edges += edges.len() as u64;
        graph.edges.extend(edges);

        let lang_bucket = metrics.by_language.entry(lang).or_default();
        lang_bucket.unresolved += classified.unresolved.len() as u64;
        lang_bucket.stdlib += classified.stdlib.len() as u64;
        lang_bucket.external += classified.external.len() as u64;
        metrics.unresolved_calls += classified.unresolved.len() as u64;
        metrics.stdlib_calls += classified.stdlib.len() as u64;
        metrics.external_calls += classified.external.len() as u64;

        if let Some(&i) = rec_idx.get(&file) {
            file_records[i].unresolved_calls = classified.unresolved.clone();
            file_records[i].stdlib_calls = classified.stdlib.clone();
            file_records[i].external_calls = classified.external.clone();
        }
        graph.unresolved.extend(classified.unresolved);
    }
    let link_ms = link_started.elapsed().as_secs_f64() * 1000.0;

    // --- Phase 3b: stack-graphs resolution (removed) -----------------------
    //
    // The stack-graphs integration was dropped in the tree-sitter 0.26
    // upgrade — upstream `tree-sitter-stack-graphs` pins tree-sitter 0.24
    // (ABI 14) — and `cgg_resolve::stack_graphs_resolver` has been a no-op
    // stub ever since. The orchestration that used to live here still ran
    // on every invocation: it deep-cloned the graph, the facts, and every
    // file's source bytes into a detached thread, then blocked on a
    // 60-second timeout, all to call a function that returns nothing. It
    // also kept a full copy of the corpus (`sources`) alive for the whole
    // run. Both are gone.
    //
    // `--stack-graphs` is still accepted so existing command lines keep
    // working, but it selects between three identical behaviours, so it is
    // not on `RunOptions` at all — `From<&Cli>` discards it explicitly.

    // --- Phase 3c: cross-file import-chain resolver -----------------------
    let cf_out = {
        let _s = cgg_core::profile::span("resolve::cross-file");
        cgg_resolve::cross_file::resolve(&graph, &all_facts, opts.fanout_cap as usize)
    };
    for e in &cf_out.edges {
        match e.confidence {
            cgg_core::graph::Confidence::High => metrics.confidence_histogram.high += 1,
            cgg_core::graph::Confidence::Medium => {
                metrics.confidence_histogram.medium += 1
            }
            cgg_core::graph::Confidence::Low => metrics.confidence_histogram.low += 1,
        }
    }
    metrics.edges += cf_out.edges.len() as u64;
    graph.edges.extend(cf_out.edges);

    // Module-scope value references the cross-file pass could not turn
    // into an edge. They are folded into the same bucket intra-file
    // fills so the dead-code name correlation sees them, and into the
    // per-file audit records so `--metrics` still explains every site.
    for u in &cf_out.unresolved {
        if let Some(rec) = file_records.iter_mut().find(|r| r.file == u.file) {
            rec.unresolved_calls.push(u.clone());
        }
        if let Some(lang) = graph.files.get(&u.file).map(|f| f.language.clone())
            && let Some(b) = metrics.by_language.get_mut(&lang)
        {
            b.unresolved += 1;
        }
    }
    metrics.unresolved_calls += cf_out.unresolved.len() as u64;
    graph.unresolved.extend(cf_out.unresolved);

    // --- Phase 3d: FFI linker (cross-language edges) ----------------------
    let ffi_out = {
        let _s = cgg_core::profile::span("resolve::ffi");
        cgg_resolve::ffi::link_ffi(&graph, &all_facts)
    };
    for e in &ffi_out.edges {
        match e.confidence {
            cgg_core::graph::Confidence::High => metrics.confidence_histogram.high += 1,
            cgg_core::graph::Confidence::Medium => {
                metrics.confidence_histogram.medium += 1
            }
            cgg_core::graph::Confidence::Low => metrics.confidence_histogram.low += 1,
        }
    }
    metrics.edges += ffi_out.edges.len() as u64;
    graph.edges.extend(ffi_out.edges);

    // Descriptor → implementation, after FFI because it asks the same
    // kind of question one level up and wants the whole graph present.
    let desc_edges = {
        let _s = cgg_core::profile::span("resolve::descriptor");
        cgg_resolve::descriptor::link_descriptors(&graph)
    };
    for e in &desc_edges {
        match e.confidence {
            cgg_core::graph::Confidence::High => metrics.confidence_histogram.high += 1,
            cgg_core::graph::Confidence::Medium => {
                metrics.confidence_histogram.medium += 1
            }
            cgg_core::graph::Confidence::Low => metrics.confidence_histogram.low += 1,
        }
    }
    metrics.edges += desc_edges.len() as u64;
    graph.edges.extend(desc_edges);

    // Reference edges (function-as-value, Issue 4) are captured during
    // extraction but only surfaced under `--reference-edges`. Dropped
    // after *every* resolver has run and before reconciliation/metrics
    // see them — `cross_file` also emits them now (it is what binds
    // `app.get('/x', handler)` to a handler in another module), so
    // filtering earlier let those escape the flag that gates them.
    if !opts.reference_edges {
        graph.edges.retain(|e| !matches!(e.via, Via::Reference));
    }

    // --- Phase 3e: audit reconciliation -----------------------------------
    let _phase_post_reconcile = cgg_core::profile::span("post::reconcile");
    // intra_file is the first resolver and gets first crack at every
    // call site. Calls it can't bind go into per-file `unresolved_calls`
    // / `external_calls` audit buckets. Later resolvers (stack_graphs,
    // cross_file, ffi) often resolve those calls and emit edges into
    // `graph.edges`, but they never reach back to prune the audit
    // buckets. Without this pass the audit log reads like "I couldn't
    // resolve N calls" when in fact most of them got resolved later —
    // misleading anyone trying to diagnose real gaps.
    //
    // Reconciliation: collect every (src_file, site_byte) pair that
    // ended up with a resolved edge, then strip those pairs from the
    // per-file unresolved/external lists, the run-level rollups, and
    // the by-language metrics. After this, "X unresolved, Y external"
    // means what it sounds like.
    let resolved_sites: HashSet<(FileId, u32)> = graph
        .edges
        .iter()
        .filter_map(|e| graph.callables.get(&e.src).map(|c| (c.file, e.site_byte)))
        .collect();

    let mut removed_per_lang_unresolved: HashMap<String, u64> = HashMap::new();
    let mut removed_per_lang_stdlib: HashMap<String, u64> = HashMap::new();
    let mut removed_per_lang_external: HashMap<String, u64> = HashMap::new();
    let mut total_removed_unresolved: u64 = 0;
    let mut total_removed_stdlib: u64 = 0;
    let mut total_removed_external: u64 = 0;
    for rec in &mut file_records {
        let lang = graph
            .files
            .get(&rec.file)
            .map(|f| f.language.clone())
            .unwrap_or_default();

        let before_u = rec.unresolved_calls.len();
        rec.unresolved_calls
            .retain(|c| !resolved_sites.contains(&(c.file, c.site_byte)));
        let dropped_u = (before_u - rec.unresolved_calls.len()) as u64;
        if dropped_u > 0 {
            *removed_per_lang_unresolved.entry(lang.clone()).or_default() += dropped_u;
        }
        total_removed_unresolved += dropped_u;

        let before_s = rec.stdlib_calls.len();
        rec.stdlib_calls
            .retain(|c| !resolved_sites.contains(&(c.file, c.site_byte)));
        let dropped_s = (before_s - rec.stdlib_calls.len()) as u64;
        if dropped_s > 0 {
            *removed_per_lang_stdlib.entry(lang.clone()).or_default() += dropped_s;
        }
        total_removed_stdlib += dropped_s;

        let before_e = rec.external_calls.len();
        rec.external_calls
            .retain(|c| !resolved_sites.contains(&(c.file, c.site_byte)));
        let dropped_e = (before_e - rec.external_calls.len()) as u64;
        if dropped_e > 0 {
            *removed_per_lang_external.entry(lang).or_default() += dropped_e;
        }
        total_removed_external += dropped_e;
    }

    metrics.unresolved_calls = metrics
        .unresolved_calls
        .saturating_sub(total_removed_unresolved);
    metrics.stdlib_calls = metrics.stdlib_calls.saturating_sub(total_removed_stdlib);
    metrics.external_calls = metrics
        .external_calls
        .saturating_sub(total_removed_external);
    for (lang, n) in removed_per_lang_unresolved {
        if let Some(b) = metrics.by_language.get_mut(&lang) {
            b.unresolved = b.unresolved.saturating_sub(n);
        }
    }
    for (lang, n) in removed_per_lang_stdlib {
        if let Some(b) = metrics.by_language.get_mut(&lang) {
            b.stdlib = b.stdlib.saturating_sub(n);
        }
    }
    for (lang, n) in removed_per_lang_external {
        if let Some(b) = metrics.by_language.get_mut(&lang) {
            b.external = b.external.saturating_sub(n);
        }
    }

    // graph.unresolved is the cross-file rollup. Same prune.
    graph
        .unresolved
        .retain(|c| !resolved_sites.contains(&(c.file, c.site_byte)));

    // Two passes can record the same site — the intra-file linker with
    // the generic "no candidate in this file", a later pass with the
    // actual cause. Keep one record per site, preferring the specific
    // reason: the whole point of the reason field is to say *why*, and
    // `no-candidate-in-file` on a name cgg has parsed and indexed reads
    // as "this name does not exist", which is the opposite of the truth.
    {
        use cgg_core::audit::UnresolvedReason as UR;
        let specific = |r: &UR| !matches!(r, UR::NoCandidateInFile | UR::Other(_));
        let mut best: std::collections::HashMap<(FileId, u32), usize> =
            std::collections::HashMap::new();
        for (i, c) in graph.unresolved.iter().enumerate() {
            match best.get(&(c.file, c.site_byte)) {
                Some(&j)
                    if specific(&graph.unresolved[j].reason) || !specific(&c.reason) => {}
                _ => {
                    best.insert((c.file, c.site_byte), i);
                }
            }
        }
        let keep: std::collections::HashSet<usize> = best.into_values().collect();
        let mut i = 0;
        graph.unresolved.retain(|_| {
            let k = keep.contains(&i);
            i += 1;
            k
        });
    }

    // Synthesize external/stdlib exit nodes from the *post-reconciliation*
    // buckets, so calls that a later resolver bound are not surfaced.
    if opts.include_external || opts.include_stdlib {
        let _s = cgg_core::profile::span("post::exit-nodes");
        synthesize_exit_nodes(
            &mut graph,
            &file_records,
            &mut stable_ids,
            opts.include_external,
            opts.include_stdlib,
        );
    }

    // --- Phase 3f: framework entry nodes -----------------------------------
    //
    // The mirror of the exit nodes above, and deliberately asymmetric
    // with them: exit nodes are opt-in, entry nodes are on by default.
    // An exit node tells you nothing you did not already know — you saw
    // the call. An entry node tells you something the source cannot: a
    // handler with in-degree zero is not merely incomplete, it is a
    // false claim that nothing calls it.
    //
    // Runs even in dead-code mode, where node-only additions are
    // switched off, because an entry node is not node-only: it adds an
    // inbound edge to a real callable, which can only ever mark
    // something live.
    let framework_out = if opts.no_entry_nodes {
        cgg_resolve::frameworks::FrameworkOutcome::default()
    } else {
        cgg_resolve::frameworks::detect(&graph, &all_facts, &config.frameworks)
    };
    // Only the entries that mint no node. A node-bearing entry already
    // says "the framework calls this" *in the graph*, and proving
    // liveness through it is strictly more informative — `--why-live`
    // prints the route rather than asserting the handler is its own
    // root. Bucket D (§8) has no node to prove anything through, so
    // marking the target directly is the only way to express it.
    let framework_roots: Vec<(String, CallableId)> = framework_out
        .entries
        .iter()
        .filter(|e| !e.node)
        .map(|e| (format!("{}:{}", e.framework, e.shape.slug()), e.target))
        .collect();
    if !framework_out.entries.is_empty() {
        let _s = cgg_core::profile::span("post::entry-nodes");
        synthesize_entry_nodes(&mut graph, &framework_out.entries, &mut stable_ids);
    }
    let framework_coverage = framework_out.coverage;

    // Interface/trait dynamic-dispatch fan-out (Issue 3). Over-approximated
    // declaration → implementation edges, tagged `Via::Dynamic`; opt-in.
    if opts.dynamic_dispatch {
        let _s = cgg_core::profile::span("resolve::dispatch");
        for e in cgg_resolve::dispatch::fanout(&graph) {
            graph.add_edge(e);
        }
    }

    // Account any synthesized nodes/edges (exit nodes, dispatch fan-out)
    // in the run metrics so the audit and summary stay consistent with
    // the emitted graph. A no-op when no synthesis ran.
    metrics.callables = graph.callables.len() as u64;
    metrics.edges = graph.edges.len() as u64;
    // Inter-file edges of the full graph — computed here (pre-query) so
    // the summary's `cross-file` count is consistent with `edges` even
    // for a `--filter`'d run. (`--filter`/`-n` later narrows what's
    // *emitted*, but the summary reports the whole-analysis totals.)
    let cross_file = graph
        .edges
        .iter()
        .filter(|e| {
            graph.callables.get(&e.src).map(|s| s.file)
                != graph.callables.get(&e.dst).map(|d| d.file)
        })
        .count() as u64;

    // The analysis needs the per-file audit records (for the external
    // call buckets), but the loop below consumes them into events. Clone
    // only in dead-code mode so the ordinary path pays nothing.
    let dead_file_records: Vec<AuditFileRecord> = if dead_mode {
        file_records.clone()
    } else {
        Vec::new()
    };

    // Bucket the unresolved population by the dependency it belongs to,
    // before the records are moved into the event stream.
    let unresolved_modules = group_unresolved_by_module(&file_records, &all_facts);

    // Push every per-file audit record as a FileAnalyzed event.
    for rec in file_records {
        events.push(AuditEvent::FileAnalyzed(rec));
    }

    for skip in &outcome.skips {
        let slug = format!("skip:{}", skip.reason.slug());
        let bucket = metrics.by_language.entry(slug).or_default();
        bucket.files += 1;

        events.push(AuditEvent::FileSkipped {
            path: skip.path.clone(),
            reason: skip.reason.clone(),
        });
        metrics.files_skipped += 1;
    }

    metrics.phases.walk_ms = walk_ms;
    let parse_wall = parse_started.elapsed().as_secs_f64() * 1000.0;
    metrics.phases.extract_ms = (parse_wall - metrics.phases.parse_ms).max(0.0);
    metrics.phases.link_ms = link_ms;
    metrics.wall_ms = started.elapsed().as_secs_f64() * 1000.0;
    // The profile table is rendered by `main` from `metrics.wall_ms` after
    // this returns: several phase spans deliberately live to the end of this
    // function, so rendering here would report them as zero.
    // The coverage disclosure goes into the audit log unconditionally,
    // so a machine consumer sees the gap list even when nobody read
    // stderr. The engine copies the disclaimer in; no writer here can
    // drop it.
    if !opts.no_entry_nodes {
        events.push(AuditEvent::FrameworkCoverage {
            coverage: framework_coverage.clone(),
        });
    }
    if !unresolved_modules.is_empty() {
        events.push(AuditEvent::UnresolvedByModule {
            modules: unresolved_modules.clone(),
        });
        // Top few on stderr, full list in the audit. "What can I not see
        // from here, and how much of it is there" is often the actual
        // question in audit work, and the tally quantifies the evidence
        // gap for free — but only if it is in front of the reader.
        let total: u32 = unresolved_modules.iter().map(|m| m.count).sum();
        let mut line = format!(
            "cgg: {total} unresolved call(s) across {} module(s) — largest:",
            unresolved_modules.len()
        );
        for m in unresolved_modules.iter().take(3) {
            line.push_str(&format!(" {} ({})", m.module, m.count));
        }
        if unresolved_modules.len() > 3 {
            line.push_str(" …");
        }
        line.push_str(" [full list in the audit log]\n");
        transcript.push(Emission::line(line));
    }

    events.push(AuditEvent::RunFinished {
        metrics: metrics.clone(),
    });

    // The graph carries its own copy, which is what `-t json` serializes.
    // Nothing assigned it, so every `-t json` run reported zeros — and
    // `confidence_histogram` and `unresolved_calls` are exactly what a
    // programmatic consumer reads to decide how far to trust the graph.
    // Reading zeros suggests a clean, fully-resolved result, which is
    // the opposite of what an all-zero histogram means.
    graph.metrics = metrics.clone();

    // --- Phase 4: emit ----------------------------------------------------
    let _phase_post_emit = cgg_core::profile::span("post::emit");
    // Deduplicate edges (same src+dst+site_byte, keep highest confidence).
    {
        let _s = cgg_core::profile::span("post::dedup-edges");
        dedup_edges(&mut graph);
    }

    // `--since` augments `--filter` with the qualified names of every
    // callable whose body overlaps a changed line range from the diff.
    let mut effective_filters = opts.filter.clone();
    if let Some(revspec) = opts.since.as_deref() {
        // Anchor the revspec on the tree being analyzed, not on wherever
        // the shell happens to be. Resolving against the process cwd meant
        // `cgg /path/to/project --since HEAD~1..HEAD` diffed whatever
        // repository the *caller* was standing in and seeded the graph
        // from unrelated changes — a wrong answer with no error. `--help`
        // has always said the analysis path is what must be in a repo,
        // and config discovery already anchors this way.
        let anchor = match opts.paths.first() {
            Some(p) if p.is_dir() => p.clone(),
            Some(p) => p
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from(".")),
            None => std::env::current_dir().context("getting current dir for --since")?,
        };
        let ranges = since::resolve_since(revspec, &anchor)
            .with_context(|| format!("resolving --since {revspec}"))?;
        let (seeds, unmatched) = since_seeds(&graph, &ranges);
        events.push(AuditEvent::SinceResolved {
            revspec: revspec.to_string(),
            files_changed: ranges.len() as u64,
            matched_seeds: seeds.clone(),
            unmatched_files: unmatched.clone(),
        });
        // `always`: never gated on `--quiet`, verified against 0.5.0.
        transcript.push(Emission::always(format!(
            "cgg: --since {revspec}: {} file(s) changed, {} callable seed(s), {} unmatched file(s)\n",
            ranges.len(),
            seeds.len(),
            unmatched.len()
        )));
        for name in &seeds {
            // Anchor with `^…$` so each seed selects exactly that one
            // qualified name, not anything containing it.
            effective_filters.push(format!("^{}$", regex::escape(name)));
        }
    }

    // --- Dead-code analysis (pre-query) ------------------------------------
    let _phase_post_deadcode = cgg_core::profile::span("post::deadcode");
    //
    // This must run on the *unpruned* graph. `query::prune` drops
    // callables outright, so a filtered subgraph would report every
    // remaining callable's callers as absent. `--filter` and the
    // `--exclude-*` flags therefore scope the finding list, never the
    // graph the analysis sees.
    // `--dead-code` annotates the graph rather than replacing it. The
    // graph is what cgg is for, and "unreferenced" is a property of a
    // node in it — so the finding rides on the node and every existing
    // formatter renders it. The detailed report goes to a sidecar,
    // following the same convention the audit already uses.
    let mut graph = graph;
    // `--report-unreferenced` replaces the graph, like `--why-live`.
    // Deliberately *not* part of the dead-code pipeline: it asks a
    // strictly weaker question — "does anything point at this?" — and
    // that weakness is the feature. Dead-code reachability cascades, so
    // one unrooted framework handler drags its whole subtree into the
    // report; this cannot, because it never looks past one edge.
    if opts.report_unreferenced {
        let mut referenced: std::collections::HashSet<CallableId> =
            std::collections::HashSet::new();
        for e in &graph.edges {
            referenced.insert(e.dst);
        }
        let roots: HashMap<CallableId, String> = framework_roots
            .iter()
            .map(|(rule, id)| (*id, rule.clone()))
            .collect();
        let mut findings: Vec<crate::outcome::UnreferencedFinding> = graph
            .callables
            .values()
            .filter(|c| !c.synthetic && !referenced.contains(&c.id))
            .map(|c| crate::outcome::UnreferencedFinding {
                qualified_name: c.qualified_name.clone(),
                path: graph
                    .files
                    .get(&c.file)
                    .map(|f| f.path.display().to_string())
                    .unwrap_or_default(),
                start_line: c.start_line,
                root: roots.get(&c.id).cloned(),
            })
            .collect();
        findings.sort_by(|a, b| {
            a.qualified_name
                .cmp(&b.qualified_name)
                .then_with(|| a.start_line.cmp(&b.start_line))
        });
        transcript.push(Emission::Unreferenced(findings));
        transcript.push(Emission::Audit);
        return Ok(RunOutcome {
            graph,
            transcript,
            events,
            metrics,
            framework_coverage,
            dead_code: None,
            dead_code_marked: 0,
            dead_code_threshold: opts.dead_code_confidence,
            cross_file_edges: cross_file,
            jobs,
        });
    }

    let mut dead_code: Option<cgg_core::deadcode::DeadCodeReport> = None;
    let mut dead_code_marked = 0usize;

    if dead_mode {
        // `--why-live` replaces the output: its answer is a proof, not a
        // graph. The early return is load-bearing — the query, run
        // summary, coverage table and `--lang` hints below never ran for
        // it before the analyze/emit split either.
        if !opts.why_live.is_empty() {
            let proofs = why_live_proofs(
                opts,
                &graph,
                &all_facts,
                &config,
                &framework_roots,
                &mut transcript,
            )?;
            // Proof, then audit — the order the pre-split path wrote them.
            transcript.push(Emission::WhyLive(proofs));
            transcript.push(Emission::Audit);
            return Ok(RunOutcome {
                graph,
                transcript,
                events,
                metrics,
                framework_coverage,
                dead_code: None,
                dead_code_marked: 0,
                dead_code_threshold: opts.dead_code_confidence,
                cross_file_edges: cross_file,
                jobs,
            });
        }
        let dc = dead_code_analysis(
            opts,
            &mut graph,
            &dead_file_records,
            &all_facts,
            &effective_filters,
            &config,
            config_path.as_deref(),
            &framework_roots,
            &mut transcript,
        )
        .context("running dead-code analysis")?;
        dead_code_marked = dc.marked;
        if let Some(toml) = dc.roots_baseline {
            // Written where `process::exit(0)` used to be: after the
            // `--ignore-attributes` note, before anything else. No
            // `Emission::Audit` — that is the whole reason no audit is
            // written, rather than a special case in the emitter.
            transcript.push(Emission::RootsBaseline(toml));
            // A config file, not a graph, and no report or audit: the
            // pre-split code exited before either could run. The report
            // data still rides along for library callers.
            return Ok(RunOutcome {
                graph,
                transcript,
                events,
                metrics,
                framework_coverage,
                dead_code: Some(dc.report),
                dead_code_marked,
                dead_code_threshold: opts.dead_code_confidence,
                cross_file_edges: cross_file,
                jobs,
            });
        }
        // The marker was pushed inside `dead_code_analysis`, between the
        // note and the summary; doing it here would land after both.
        dead_code = Some(dc.report);
    }

    let (graph, query_stats) = {
        let _s = cgg_core::profile::span("post::query");
        query::apply_query(&graph, &effective_filters, opts.hops, opts.max_paths)
            .map_err(|e| anyhow::anyhow!(e))?
    };
    // A capped `-n 0` looks exactly like a complete one in the output,
    // so the cap has to announce itself in both places a caller might
    // look: the audit trail after the fact, and stderr right now.
    if query_stats.paths_truncated {
        events.push(AuditEvent::PathsTruncated {
            max_paths: opts.max_paths,
            paths_emitted: query_stats.paths_emitted,
        });
        transcript.push(Emission::line(format!(
            "cgg: -n 0 stopped at --max-paths {} ({} path(s) kept) — the graph \
             omits paths that exist; raise --max-paths or narrow --filter\n",
            opts.max_paths, query_stats.paths_emitted,
        )));
    }
    let graph = query::apply_exclusions(
        &graph,
        &opts.exclude_partial,
        &opts.exclude_glob,
        &opts.exclude_regex,
    )
    .map_err(|e| anyhow::anyhow!(e))?;

    // Last transform on the graph, and deliberately last: it composes
    // with `--filter`, `-n` and `--exclude-*` rather than competing with
    // them, and it must not run before dead-code marking or the
    // `unreferenced` marks it aggregates would not exist yet.
    let graph = apply_rollup(graph, opts, &id_roots, &mut transcript, &mut events)?;

    // Here, before the run summary: without `-o` the graph goes to stdout
    // and the summary to stderr, so the position is visible to anyone piping
    // both. See `Emission`.
    transcript.push(Emission::Graph);
    transcript.push(Emission::Audit);

    // Break "X skipped" down by reason so users don't see e.g. "22
    // skipped" and assume cgg missed 22 Rust files when it's actually
    // 22 Cargo.toml/yaml/.comp files (unknown-extension). We tally
    // from the audit events list rather than outcome.skips because
    // files can also be skipped *during* processing (e.g., language
    // detector returned nothing) and those don't appear in
    // outcome.skips. Sorted by count, descending; ties broken
    // alphabetically.
    //
    // Also tally per-language for the LanguageFilter variant so we
    // can emit an actionable hint ("note: 5 file(s) detected as
    // 'python' were excluded by --lang ...").
    let mut skip_counts: HashMap<&'static str, u64> = HashMap::new();
    let mut lang_filter_counts: HashMap<String, u64> = HashMap::new();
    for ev in &events {
        if let AuditEvent::FileSkipped { reason, .. } = ev {
            *skip_counts.entry(reason.slug()).or_default() += 1;
            if let SkipReason::LanguageFilter(lang) = reason {
                *lang_filter_counts.entry(lang.clone()).or_default() += 1;
            }
        }
    }
    let skip_breakdown = if skip_counts.is_empty() {
        String::new()
    } else {
        let mut pairs: Vec<_> = skip_counts.into_iter().collect();
        pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        let inner = pairs
            .iter()
            .map(|(slug, n)| format!("{n} {slug}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!(" ({inner})")
    };

    // Parsed by `scripts/update-readme-stats.py` on every commit —
    // reflowing it breaks the README patcher silently.
    transcript.push(Emission::line(format!(
        "cgg: {disc} files, {an} analyzed, {sk} skipped{breakdown}; \
         {ca} callables, {ed} edges ({cf} cross-file), \
         {ur} unresolved, {sl} stdlib, {ext} external ({ms:.1} ms)\n",
        disc = metrics.files_discovered,
        an = metrics.files_analyzed,
        sk = metrics.files_skipped,
        breakdown = skip_breakdown,
        ca = metrics.callables,
        ed = metrics.edges,
        cf = cross_file,
        ur = metrics.unresolved_calls,
        sl = metrics.stdlib_calls,
        ext = metrics.external_calls,
        ms = metrics.wall_ms
    )));

    // Framework coverage. Recorded whenever anything was recognised — or
    // whenever a framework was seen and could not be enumerated, which
    // is the case that most needs saying. A user who never sees the gap
    // list has no way to learn which frameworks cgg missed, and a
    // partial list that reads as complete is worse than no list.
    if !opts.no_entry_nodes {
        let worth_printing = opts.framework_coverage
            || !framework_coverage.recognised.is_empty()
            || !framework_coverage.seen_no_rules.is_empty();
        if worth_printing {
            // `render` already ends with one newline, so a leading blank
            // line reproduces the previous `eprintln!(); eprint!(table)`.
            transcript.push(Emission::line(format!(
                "\n{}",
                framework_coverage.render(opts.framework_coverage)
            )));
        }
        if framework_coverage.nodes_minted > 0 || framework_coverage.root_marks_only > 0 {
            transcript.push(Emission::line(format!(
                "cgg: framework entries: {} node(s) minted, {} root-marked only \
                 — INFERRED, not observed\n",
                framework_coverage.nodes_minted, framework_coverage.root_marks_only
            )));
        }
    }

    // Actionable hint when --lang excluded files whose language IS
    // supported by a plugin. Listing each excluded language with its
    // count and the suggested `--lang` value tells the user exactly
    // what to add.
    if !lang_filter_counts.is_empty() {
        let mut pairs: Vec<_> = lang_filter_counts.into_iter().collect();
        pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let user_langs = if opts.lang.is_empty() {
            String::new()
        } else {
            opts.lang.join(",")
        };
        for (lang, n) in pairs {
            let suggestion = if user_langs.is_empty() {
                format!("--lang {lang}")
            } else {
                format!("--lang {user_langs},{lang}")
            };
            // `always`, not `line`: verified against 0.5.0, this hint has
            // never been gated on `--quiet`.
            transcript.push(Emission::always(format!(
                "note: {n} file(s) detected as '{lang}' were excluded by --lang; \
                 pass `{suggestion}` to include them\n"
            )));
        }
    }

    Ok(RunOutcome {
        graph,
        transcript,
        events,
        metrics,
        framework_coverage,
        dead_code,
        dead_code_marked,
        dead_code_threshold: opts.dead_code_confidence,
        cross_file_edges: cross_file,
        jobs,
    })
}

/// Fold the graph to a coarser granularity, if this run asked for one.
///
/// A no-op — not even a render — unless `--rollup` or `--rollup-by` is
/// set, so the default graph and its cost are untouched.
///
/// Shared with `crate::replay`, which is why it takes a transcript and an
/// event list rather than returning diagnostics: both callers have to
/// interleave these into their own stream in their own order.
pub(crate) fn apply_rollup(
    graph: Graph,
    opts: &RunOptions,
    id_roots: &[stable_ids::IdRoot],
    transcript: &mut Vec<Emission>,
    events: &mut Vec<AuditEvent>,
) -> Result<Graph> {
    if opts.rollup.is_none() && opts.rollup_by.is_none() {
        return Ok(graph);
    }
    let _s = cgg_core::profile::span("post::rollup");

    let format = opts.rollup_format;
    // The scheme the artifact will actually be rendered with, not the
    // format's default: `--node-ids hash` makes a mermaid document about
    // a third larger, and a budget measured against the numbered form
    // would let the emitted one sail past it.
    let ids = cgg_format::NodeIds::resolve(opts.node_ids, format);
    let render = move |g: &Graph| emit::graph_to_string_with(g, format, ids);
    // A fresh allocator, not the pipeline's: `replay` has no pipeline and
    // must take the identical path, and group ids draw from their own
    // hash domain either way.
    let mut ids = StableIds::new();
    let before = (graph.callables.len() as u64, graph.edges.len() as u64);

    let fitted = rollup::fit(
        &graph,
        &render,
        opts.rollup,
        opts.rollup_by,
        &mut ids,
        id_roots,
    );

    if !fitted.level.is_rollup() {
        if let Some(budget) = opts.rollup {
            if fitted.over_budget {
                // The budget was needed and could not be met, and every
                // grouping cgg tried came out *larger* than doing nothing
                // — so the un-rolled graph is what got emitted. Saying
                // "not needed" here would be the exact opposite of true.
                transcript.push(Emission::always(format!(
                    "warning: --rollup {budget} could not be met. No grouping \
                     cgg has renders smaller than the un-rolled graph here, so \
                     the output is the full graph at about {} token(s) — OVER \
                     the budget. Narrow it with --filter or --exclude-*.\n",
                    fitted.chosen().tokens
                )));
            } else {
                // Under budget already. Say so — a caller who leaves
                // `--rollup` in a wrapper script wants to know whether it
                // engaged, and silence is ambiguous between "fit" and
                // "ignored".
                transcript.push(Emission::line(format!(
                    "cgg: --rollup {budget} not needed — the graph renders to \
                     about {} token(s)\n",
                    fitted.chosen().tokens
                )));
            }
        }
        return Ok(fitted.graph);
    }

    let chosen = fitted.chosen().clone();
    events.push(AuditEvent::RolledUp {
        level: chosen.level.as_str(),
        budget: opts.rollup,
        estimated_tokens: chosen.tokens,
        nodes_before: before.0,
        edges_before: before.1,
        nodes_after: chosen.nodes as u64,
        edges_after: chosen.edges as u64,
        attempts: fitted
            .attempts
            .iter()
            .map(|a| cgg_core::audit::RollupAttempt {
                level: a.level.as_str(),
                nodes: a.nodes as u64,
                edges: a.edges as u64,
                estimated_tokens: a.tokens,
            })
            .collect(),
        over_budget: fitted.over_budget,
    });

    // `always`, not `line`. Every other advisory here is suppressible
    // because the artifact still says what it is; this one is the only
    // thing distinguishing a graph of your code from a graph of your
    // directory layout, and `-q` must not be able to hide that.
    transcript.push(Emission::always(format!(
        "cgg: ROLLED UP to `{}` — {} callable(s) -> {} group node(s), \
         {} edge(s) -> {} ({} est. tokens{})\n",
        chosen.level,
        before.0,
        chosen.nodes,
        before.1,
        chosen.edges,
        chosen.tokens,
        match opts.rollup {
            Some(b) => format!(", budget {b}"),
            None => String::new(),
        },
    )));
    if fitted.over_budget {
        transcript.push(Emission::always(format!(
            "warning: --rollup {} could not be met. `{}` is the smallest \
             grouping cgg could produce and it still renders to about {} token(s); the \
             output is OVER the budget you asked for. Narrow it with --filter \
             or --exclude-* — no further rollup is available.\n",
            opts.rollup.unwrap_or(0),
            chosen.level,
            chosen.tokens,
        )));
    }

    Ok(fitted.graph)
}

/// What the dead-code pass produced.
///
/// No exit code here: `--fail-on-dead` is a CLI policy decision, so the
/// caller derives it from [`Self::marked`]. A library consumer that reads
/// this struct is not obliged to have an exit status at all.
#[derive(Debug)]
struct DeadCodeOutcome {
    /// The findings, every band.
    report: cgg_core::deadcode::DeadCodeReport,
    /// Findings at or above the confidence threshold, and therefore
    /// marked `unreferenced` on the graph. Distinct from
    /// `report.findings.len()`, which counts every band.
    marked: usize,
    /// `--write-roots` baseline, rendered. `Some` means the run's primary
    /// artifact is this config file rather than the graph, and that the
    /// graph was *not* annotated — the pre-split code exited before it
    /// could be.
    roots_baseline: Option<String>,
}

/// Run the dead-code analysis and annotate the graph.
///
/// Pushes its diagnostics onto `notices` in the order the pre-split code
/// printed them: the `--ignore-attributes` note, then a
/// [`Notice::DeadCodeReport`] marker standing in for the detailed report,
/// then the summary line. Writes nothing.
fn dead_code_analysis(
    opts: &RunOptions,
    graph: &mut Graph,
    file_records: &[AuditFileRecord],
    all_facts: &[FileFacts],
    filters: &[String],
    cfg: &deadcode::config::DeadCodeConfigFile,
    cfg_path: Option<&std::path::Path>,
    framework_roots: &[(String, CallableId)],
    transcript: &mut Vec<Emission>,
) -> Result<DeadCodeOutcome> {
    // Aliased: `analyze` is this crate's own entry point, and an
    // unqualified shadow of it inside one function is a trap for the next
    // reader.
    use cgg_resolve::deadcode::{DeadCodeOptions, analyze as analyze_dead_code};

    let threshold: Confidence = opts.dead_code_confidence;
    let cfg_path = cfg_path.map(|p| p.to_path_buf());

    // Declared roots confer liveness, so they are resolved against the
    // graph before the analysis runs.
    let user_roots = resolve_user_roots(graph, cfg)?;

    // A suppression file rots silently unless someone says so.
    let matched: std::collections::HashSet<&String> =
        user_roots.iter().map(|(p, _)| p).collect();
    let mut stale: Vec<String> = cfg
        .roots
        .iter()
        .filter(|p| !matched.contains(p))
        .cloned()
        .collect();

    // What each plugin declares it can extract. This is the authority
    // for confidence capping — see caps::measure.
    let registry = PluginRegistry::with_v1_plugins();
    let language_signals: std::collections::BTreeMap<String, cgg_core::LanguageSignals> =
        registry
            .all()
            .iter()
            .map(|p| {
                let s = p.signals();
                (
                    p.id().to_string(),
                    cgg_core::LanguageSignals {
                        visibility: s.visibility,
                        attributes: s.attributes,
                        exports: s.exports,
                        test_defs: s.test_defs,
                        value_refs: s.value_refs,
                        dyn_uses: s.dyn_uses,
                        unreachable: s.unreachable,
                        impls: s.impls,
                    },
                )
            })
            .collect();

    // `dc_opts`, not `opts`: shadowing the parameter would redirect every
    // `opts.` read below to a struct without those fields.
    let dc_opts = DeadCodeOptions {
        user_roots,
        language_signals,
        include_tests: opts.include_tests,
        reference_edges: opts.reference_edges,
        dynamic_dispatch: opts.dynamic_dispatch,
        confidence_threshold: format!("{threshold:?}").to_lowercase(),
        roots_file: cfg_path.clone(),
        // Bucket-D entries mint no node (§8), so nothing in the graph
        // says the framework invokes them. Passing them here is what
        // stops `Encoder.forward` and every private helper it calls from
        // being reported — the cascade the design measured.
        framework_roots: framework_roots.to_vec(),
        ..Default::default()
    };

    let mut report = analyze_dead_code(graph, file_records, all_facts, &dc_opts);

    // Scope the *findings*, never the graph. Excluding a caller from the
    // graph would delete its outgoing edges and manufacture findings for
    // its callees — the failure mode of file-level exclusion in
    // name-matching tools.
    let allow_pats: Vec<String> = cfg.allow.iter().map(|a| a.name.clone()).collect();
    {
        let keep = query::compile_patterns(filters).map_err(|e| anyhow::anyhow!(e))?;
        let mut drop_pats = opts.ignore_names.clone();
        drop_pats.extend(allow_pats.iter().cloned());
        let drop = query::compile_patterns(&drop_pats).map_err(|e| anyhow::anyhow!(e))?;
        let attr_pats = query::compile_patterns(&opts.ignore_attributes)
            .map_err(|e| anyhow::anyhow!(e))?;

        let mut used_allow: std::collections::HashSet<usize> = Default::default();
        report.findings.retain(|f| {
            let included =
                keep.is_empty() || keep.iter().any(|p| p.matches(&f.qualified_name));
            let ignored = drop.iter().enumerate().any(|(i, p)| {
                let hit = p.matches(&f.qualified_name);
                if hit && i >= opts.ignore_names.len() {
                    used_allow.insert(i - opts.ignore_names.len());
                }
                hit
            });
            let attr_ignored = !attr_pats.is_empty()
                && graph.callables.get(&f.id).is_some_and(|c| {
                    c.attributes
                        .iter()
                        .any(|a| attr_pats.iter().any(|p| p.matches(a)))
                });
            included && !ignored && !attr_ignored
        });
        for (i, a) in cfg.allow.iter().enumerate() {
            if !used_allow.contains(&i) {
                stale.push(a.name.clone());
            }
        }
        report.summary.reported = report.findings.len() as u32;
        report.summary.stale_suppressions = stale;
    }

    if !opts.ignore_attributes.is_empty()
        && graph.callables.values().all(|c| c.attributes.is_empty())
    {
        // Naming the languages is the actionable half of this note, so
        // the list is read off the plugins rather than typed here. The
        // hardcoded version said "python, rust" long after seven more
        // plugins had learned to capture attributes.
        // `always`: verified against 0.5.0, this note has never been gated
        // on `--quiet`. It reports that a flag the user passed did nothing,
        // which is the one class of advisory worth keeping under `-q`.
        transcript.push(Emission::always(format!(
            "note: --ignore-attributes matched nothing — no callable in this run \
             carries attributes (attribute capture: {})\n",
            languages_capturing_attributes().join(", ")
        )));
    }

    if opts.write_roots {
        // A baseline is a config file, not a graph, so it takes the
        // primary sink and nothing else is emitted.
        //
        // This was `std::process::exit(0)`, which would have killed a
        // host interpreter mid-call. Returning the TOML preserves the old
        // side-effect set exactly — no graph, no report, no audit — since
        // the exit preceded all three. See `emit::all`.
        return Ok(DeadCodeOutcome {
            roots_baseline: Some(deadcode::config::render_baseline(&report)),
            report,
            marked: 0,
        });
    }

    // Annotate the graph: the finding rides on the node so mermaid,
    // dot, graphml and json all render it with no second output path.
    // Only findings at or above the threshold are marked — a mark in a
    // diagram travels without its evidence, so a low-confidence one
    // would mislead.
    let band = |c: Confidence| match c {
        Confidence::High => 0u8,
        Confidence::Medium => 1,
        Confidence::Low => 2,
    };
    let mut shown = 0usize;
    for f in &report.findings {
        if band(f.confidence) > band(threshold) {
            continue;
        }
        shown += 1;
        if let Some(node) = graph.callables.get_mut(&f.id) {
            node.unreferenced = Some(f.confidence);
        }
    }

    // The report is a document, not a diagnostic: `emit` decides where it
    // lands. Only its *position* is recorded — after the note above,
    // before the summary below, which is the order a terminal sees.
    transcript.push(Emission::DeadCodeReport);

    transcript.push(Emission::line(format!(
        "cgg: dead-code: {shown} callable(s) marked unreferenced at {} confidence, \
         {} withheld — BEST EFFORT, every finding is a hypothesis\n",
        format!("{threshold:?}").to_lowercase(),
        report.findings.len().saturating_sub(shown),
    )));

    Ok(DeadCodeOutcome {
        report,
        marked: shown,
        roots_baseline: None,
    })
}

/// Plugin ids that declare they capture attributes/decorators, sorted.
///
/// Measured from the registry, never listed by hand: this set has grown
/// from two plugins to nine, and every hand-maintained copy of it in the
/// tree was wrong by the time anyone checked.
fn languages_capturing_attributes() -> Vec<&'static str> {
    let registry = PluginRegistry::with_v1_plugins();
    let mut ids: Vec<&'static str> = registry
        .all()
        .iter()
        .filter(|p| p.signals().attributes)
        .map(|p| p.id())
        .collect();
    ids.sort_unstable();
    ids
}

/// Resolve the roots declared in `cgg-deadcode.toml` (`roots` patterns
/// and `root_attributes`) against the graph.
///
/// Shared by `--dead-code` and `--why-live` so the two can never
/// disagree about what a root is. They did: `--why-live` used to build
/// its options with `..Default::default()`, which left `user_roots`
/// empty, so a callable the report had just proven live through a
/// declared root came back "NOT REACHED — no path from any known root"
/// when you asked why. That is the one question whose whole purpose is
/// to agree with the report.
fn resolve_user_roots(
    graph: &Graph,
    cfg: &deadcode::config::DeadCodeConfigFile,
) -> Result<Vec<(String, cgg_core::ids::CallableId)>> {
    let mut v = Vec::new();
    for pat in &cfg.roots {
        for id in query::match_callables(graph, std::slice::from_ref(pat))
            .map_err(|e| anyhow::anyhow!(e))?
        {
            v.push((pat.clone(), id));
        }
    }
    // Attribute-declared roots.
    if !cfg.root_attributes.is_empty() {
        let pats = query::compile_patterns(&cfg.root_attributes)
            .map_err(|e| anyhow::anyhow!(e))?;
        for c in graph.callables.values() {
            if c.attributes
                .iter()
                .any(|a| pats.iter().any(|p| p.matches(a)))
            {
                v.push(("root_attributes".to_string(), c.id));
            }
        }
    }
    Ok(v)
}

/// `--why-live`: the shortest path from a root proving a callable is
/// live. A query, not a graph, so its answer replaces the output.
fn why_live_proofs(
    opts: &RunOptions,
    graph: &Graph,
    all_facts: &[FileFacts],
    cfg: &deadcode::config::DeadCodeConfigFile,
    framework_roots: &[(String, CallableId)],
    transcript: &mut Vec<Emission>,
) -> Result<Vec<cgg_core::deadcode::LivenessProof>> {
    use cgg_resolve::deadcode::{DeadCodeOptions, why_live};
    // The same roots `--dead-code` uses. Without them `--why-live` said
    // "NOT REACHED" for exactly the callables the report considers live,
    // which defeats the point of asking the question in the opposite
    // direction.
    //
    // `dc_opts`, not `opts`: see `dead_code_analysis`.
    let dc_opts = DeadCodeOptions {
        user_roots: resolve_user_roots(graph, cfg)?,
        include_tests: opts.include_tests,
        reference_edges: opts.reference_edges,
        dynamic_dispatch: opts.dynamic_dispatch,
        framework_roots: framework_roots.to_vec(),
        ..Default::default()
    };
    let targets =
        query::match_callables(graph, &opts.why_live).map_err(|e| anyhow::anyhow!(e))?;
    if targets.is_empty() {
        // `always`: verified against 0.5.0. `--why-live` produces no other
        // output when nothing matched, so gating this on `-q` would make the
        // run print nothing at all and exit 0.
        transcript.push(Emission::always("cgg: --why-live matched no callables\n"));
    }
    Ok(why_live(graph, all_facts, &dc_opts, &targets))
}

/// Walk the graph's files, intersect each file's callable spans against
/// the diff ranges, and return (matched qualified names,
/// changed-but-unmatched file paths). Both vectors are sorted &
/// deduped for stable audit output and idempotent runs.
fn since_seeds(
    graph: &Graph,
    ranges: &since::ChangedRanges,
) -> (Vec<String>, Vec<PathBuf>) {
    use std::collections::BTreeSet;

    // Index callables by file for O(F + C) instead of O(F·C).
    let mut by_file: HashMap<FileId, Vec<&CallableNode>> = HashMap::new();
    for c in graph.callables.values() {
        by_file.entry(c.file).or_default().push(c);
    }

    // Build a path → FileId lookup. Try canonicalised first, fall back
    // to the raw stored path so non-existent / sandboxed paths still
    // match on string equality.
    let mut path_to_file: HashMap<PathBuf, FileId> = HashMap::new();
    for f in graph.files.values() {
        let key = f.path.canonicalize().unwrap_or_else(|_| f.path.clone());
        path_to_file.insert(key, f.id);
    }

    let mut seeds: BTreeSet<String> = BTreeSet::new();
    let mut unmatched: BTreeSet<PathBuf> = BTreeSet::new();

    for (path, hunks) in ranges {
        let key = path.canonicalize().unwrap_or_else(|_| path.clone());
        let Some(&fid) = path_to_file.get(&key) else {
            // Not a file cgg analyzed (could be a doc, a binary, an
            // ignored path, or a non-source language).
            unmatched.insert(path.clone());
            continue;
        };
        let Some(callables) = by_file.get(&fid) else {
            unmatched.insert(path.clone());
            continue;
        };
        let before = seeds.len();
        for c in callables {
            if since::overlaps_any(c.start_line, c.end_line, hunks) {
                seeds.insert(c.qualified_name.clone());
            }
        }
        if seeds.len() == before {
            // File was analyzed but no callable's body overlapped the
            // hunks — pure-comment edits, whitespace, deleted bodies,
            // or hunks landing in module-level prelude.
            unmatched.insert(path.clone());
        }
    }

    (seeds.into_iter().collect(), unmatched.into_iter().collect())
}

fn count_lines(bytes: &[u8]) -> u32 {
    let mut n = bytes.iter().filter(|&&b| b == b'\n').count() as u32;
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        n += 1;
    }
    n
}

fn read_file(path: &std::path::Path) -> Result<Vec<u8>> {
    let mut f =
        File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut v = Vec::new();
    f.read_to_end(&mut v)
        .with_context(|| format!("reading {}", path.display()))?;
    Ok(v)
}

fn variant_to_kind(v: DefVariant) -> CallableKind {
    v.to_callable_kind()
}

/// A synthetic `FileRecord` standing in for the external / stdlib
/// "module" that exit nodes belong to.
fn sentinel_file(id: FileId, path: &str, lang: &str) -> GraphFileRecord {
    GraphFileRecord {
        id,
        path: PathBuf::from(path),
        language: lang.to_string(),
        detected_via: "synthesized".to_string(),
        blake3: "0".repeat(64),
        size_bytes: 0,
        lines: 0,
        parse_ms: 0.0,
        parse_status: "synthetic".to_string(),
        test_role: None,
        ..Default::default()
    }
}

/// Synthesize deduplicated leaf "exit nodes" for calls into external /
/// stdlib code (`--include-external` / `--include-stdlib`). One node per
/// `(language, receiver, name)` symbol; every call site becomes a Low /
/// `Via::External|Stdlib` edge onto it, so the formatters' parallel-edge
/// collapse surfaces the call multiplicity. Nodes are minted in
/// first-encounter order over the already-deterministic file walk, so
/// ids are stable across runs. Consumes the *post-reconciliation*
/// per-file buckets, so resolved calls are not surfaced.
fn synthesize_exit_nodes(
    graph: &mut Graph,
    file_records: &[AuditFileRecord],
    stable_ids: &mut StableIds,
    include_external: bool,
    include_stdlib: bool,
) {
    let resolver = ResolverId::new("exit-node");
    // (language, receiver, name, is_external) -> exit node id.
    let mut node_ids: HashMap<(String, String, String, bool), CallableId> =
        HashMap::new();
    let mut external_file: Option<FileId> = None;
    let mut stdlib_file: Option<FileId> = None;
    let mut edges: Vec<CallEdge> = Vec::new();

    for rec in file_records {
        let lang = graph
            .files
            .get(&rec.file)
            .map(|f| f.language.clone())
            .unwrap_or_default();
        for is_external in [true, false] {
            let calls = if is_external {
                if !include_external {
                    continue;
                }
                &rec.external_calls
            } else {
                if !include_stdlib {
                    continue;
                }
                &rec.stdlib_calls
            };
            let (kind_label, via) = if is_external {
                ("external", Via::External)
            } else {
                ("stdlib", Via::Stdlib)
            };
            for call in calls {
                // An exit node needs a caller to point from.
                let Some(src) = call.src else { continue };
                let key = (
                    lang.clone(),
                    call.receiver_hint.clone(),
                    call.name.clone(),
                    is_external,
                );
                let node_id = if let Some(&id) = node_ids.get(&key) {
                    id
                } else {
                    let sentinel_path = if is_external {
                        "<external>"
                    } else {
                        "<stdlib>"
                    };
                    let file_id = if is_external {
                        *external_file.get_or_insert_with(|| {
                            let fid = stable_ids.file(sentinel_path);
                            graph.add_file(sentinel_file(fid, "<external>", "external"))
                        })
                    } else {
                        *stdlib_file.get_or_insert_with(|| {
                            let fid = stable_ids.file(sentinel_path);
                            graph.add_file(sentinel_file(fid, "<stdlib>", "stdlib"))
                        })
                    };
                    let qn = if call.receiver_hint.is_empty() {
                        format!("<{kind_label}>::{}", call.name)
                    } else {
                        format!("<{kind_label}>::{}::{}", call.receiver_hint, call.name)
                    };
                    // Synthetic: `node_ids` already dedupes these by key, so no
                    // two reach here with the same qualified name and a
                    // constant byte offset is unambiguous.
                    let id = stable_ids.callable(&lang, sentinel_path, None, &qn, "");
                    graph.add_callable(CallableNode {
                        id,
                        qualified_name: qn,
                        simple_name: call.name.clone(),
                        kind: CallableKind::Function,
                        language: lang.clone(),
                        file: file_id,
                        start_line: 0,
                        end_line: 0,
                        start_byte: 0,
                        end_byte: 0,
                        signature_hint: String::new(),
                        visibility: String::new(),
                        attributes: vec![kind_label.to_string()],
                        synthetic: true,
                        trait_impl_target: None,
                        ..Default::default()
                    });
                    node_ids.insert(key, id);
                    id
                };
                edges.push(CallEdge {
                    src,
                    dst: node_id,
                    site_line: call.site_line,
                    site_byte: call.site_byte,
                    confidence: Confidence::Low,
                    via: via.clone(),
                    resolver: resolver.clone(),
                    weight: 1,
                });
            }
        }
    }
    for e in edges {
        graph.add_edge(e);
    }
}

/// Synthesize `<framework-entry>` nodes — the mirror of
/// [`synthesize_exit_nodes`].
///
/// One node per distinct entry identity, with a `Via::FrameworkEntry`
/// edge onto the handler. Entries flagged `node: false` by their rule
/// contribute no node at all: per §8 of the design, a single
/// `torch:Module.forward` node fanning out to every model in the
/// repository is visually useless, so those only mark a root.
///
/// Edges carry `Confidence::Low`, as exit-node edges do — and for a
/// stronger reason. An exit node is minted from a call site cgg *saw*;
/// an entry node asserts a caller that appears nowhere in the tree.
fn synthesize_entry_nodes(
    graph: &mut Graph,
    entries: &[cgg_core::frameworks::FrameworkEntry],
    stable_ids: &mut StableIds,
) {
    use cgg_core::frameworks::FRAMEWORK_ENTRY_SENTINEL;

    let resolver = ResolverId::new("framework-entry");
    let mut node_ids: HashMap<String, CallableId> = HashMap::new();
    let mut sentinel: Option<FileId> = None;
    let mut edges: Vec<CallEdge> = Vec::new();

    for entry in entries {
        if !entry.node {
            continue;
        }
        let qn = entry.node_name();
        let node_id = if let Some(&id) = node_ids.get(&qn) {
            id
        } else {
            let file_id = *sentinel.get_or_insert_with(|| {
                let fid = stable_ids.file(FRAMEWORK_ENTRY_SENTINEL);
                graph.add_file(sentinel_file(
                    fid,
                    FRAMEWORK_ENTRY_SENTINEL,
                    "framework-entry",
                ))
            });
            let language = graph
                .callables
                .get(&entry.target)
                .map(|c| c.language.clone())
                .unwrap_or_default();
            // Synthetic and deduped by qualified name just above.
            let id =
                stable_ids.callable(&language, FRAMEWORK_ENTRY_SENTINEL, None, &qn, "");
            let simple = qn.rsplit("::").next().unwrap_or(&qn).to_string();
            graph.add_callable(CallableNode {
                id,
                qualified_name: qn.clone(),
                simple_name: simple,
                kind: CallableKind::Function,
                language,
                file: file_id,
                start_line: 0,
                end_line: 0,
                start_byte: 0,
                end_byte: 0,
                signature_hint: String::new(),
                visibility: String::new(),
                // The evidence rides on the node so a reader who has only
                // the graph can still see which marker produced it.
                attributes: vec!["framework-entry".to_string(), entry.evidence.clone()],
                synthetic: true,
                trait_impl_target: None,
                framework_entry: Some(entry.kind),
                ..Default::default()
            });
            node_ids.insert(qn, id);
            id
        };
        edges.push(CallEdge {
            src: node_id,
            dst: entry.target,
            site_line: entry.site_line,
            site_byte: 0,
            confidence: Confidence::Low,
            via: Via::FrameworkEntry(entry.framework.clone()),
            resolver: resolver.clone(),
            weight: 1,
        });
    }
    for e in edges {
        graph.add_edge(e);
    }
}

/// Extract the implemented trait from a Rust trait-impl qualified name
/// (Issue 3): `<DiskStorage as Storage>::put` → `Some("Storage")`,
/// possibly nested under a module path. Returns `None` for non-impl
/// names. The bare trait name (last path segment) is returned so it
/// matches a trait declaration's owner.
fn trait_impl_target_from_qn(qn: &str) -> Option<String> {
    let open = qn.find('<')?;
    let rest = &qn[open + 1..];
    let close = rest.find('>')?;
    let inner = &rest[..close];
    let as_pos = inner.find(" as ")?;
    let trait_part = inner[as_pos + 4..].trim();
    Some(
        trait_part
            .rsplit("::")
            .next()
            .unwrap_or(trait_part)
            .to_string(),
    )
}

/// Deduplicate edges: keep only one edge per (src, dst, site_byte)
/// triple, preferring the highest confidence.
fn dedup_edges(graph: &mut Graph) {
    use cgg_core::graph::Confidence;
    use cgg_core::ids::CallableId;
    use std::collections::HashMap;
    let mut best: HashMap<(CallableId, CallableId, u32), usize> = HashMap::new();
    let conf_rank = |c: Confidence| match c {
        Confidence::High => 2,
        Confidence::Medium => 1,
        Confidence::Low => 0,
    };
    for (i, e) in graph.edges.iter().enumerate() {
        let key = (e.src, e.dst, e.site_byte);
        let entry = best.entry(key).or_insert(i);
        if conf_rank(e.confidence) > conf_rank(graph.edges[*entry].confidence) {
            *entry = i;
        }
    }
    let keep: std::collections::HashSet<usize> = best.into_values().collect();
    let mut idx = 0;
    graph.edges.retain(|_| {
        let k = keep.contains(&idx);
        idx += 1;
        k
    });
}

/// Attribute each unresolved call to the dependency it most likely
/// belongs to, and count them.
///
/// Attribution is by the calling file's own import table, which is the
/// only evidence available: a name cgg could not resolve is by
/// definition not in the graph, so nothing downstream can say where it
/// lives. Three signals, in order of strength — the receiver's first
/// segment matching an import alias or module, then the bare name
/// appearing in a `from X import name` list, then nothing.
fn group_unresolved_by_module(
    file_records: &[cgg_core::audit::AuditFileRecord],
    facts: &[cgg_core::FileFacts],
) -> Vec<cgg_core::audit::UnresolvedModuleBucket> {
    use std::collections::{BTreeMap, BTreeSet};

    // Per-file lookup tables, built once. Scanning a file's import list
    // per unresolved call is O(calls x imports), which on a JavaScript
    // tree with a quarter-million unresolved calls was ~4% of the run.
    struct FileIndex {
        by_head: HashMap<String, String>,
        by_name: HashMap<String, String>,
    }
    let index: HashMap<FileId, FileIndex> = facts
        .iter()
        .map(|f| {
            let mut by_head: HashMap<String, String> = HashMap::new();
            let mut by_name: HashMap<String, String> = HashMap::new();
            for imp in &f.imports {
                let last = imp.path.rsplit(['.', '/', ':']).next().unwrap_or(&imp.path);
                for key in [imp.alias.as_str(), imp.path.as_str(), last] {
                    if !key.is_empty() {
                        by_head
                            .entry(key.to_string())
                            .or_insert_with(|| imp.path.clone());
                    }
                }
                if imp.kind == "from-import" {
                    for n in imp.alias.split(',') {
                        let n = n.split_whitespace().next_back().unwrap_or("").trim();
                        if !n.is_empty() {
                            by_name
                                .entry(n.to_string())
                                .or_insert_with(|| imp.path.clone());
                        }
                    }
                }
            }
            (f.file, FileIndex { by_head, by_name })
        })
        .collect();

    // Borrowed keys: the index owns every module string, so the hot loop
    // allocates nothing. Cloning one `String` per unresolved call cost
    // real time on a tree with a quarter-million of them.
    let mut buckets: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut counts: BTreeMap<&str, u32> = BTreeMap::new();

    // Read the per-file audit buckets, not `graph.unresolved`: the
    // latter is the cross-file rollup and a call screened as external
    // never reaches it — which is exactly the population an auditor
    // asking "what can't I see?" cares about.
    for rec in file_records {
        let idx = index.get(&rec.file);
        for u in rec.unresolved_calls.iter().chain(rec.external_calls.iter()) {
            let module: &str = idx
                .and_then(|idx| {
                    let head = u
                        .receiver_hint
                        .split(['.', ':'])
                        .find(|s| !s.is_empty())
                        .unwrap_or("");
                    idx.by_head
                        .get(head)
                        .or_else(|| idx.by_name.get(&u.name))
                        .map(String::as_str)
                })
                .unwrap_or("(unattributed)");
            *counts.entry(module).or_default() += 1;
            let names = buckets.entry(module).or_default();
            if names.len() < 8 {
                names.insert(u.name.as_str());
            }
        }
    }

    let mut out: Vec<cgg_core::audit::UnresolvedModuleBucket> = counts
        .into_iter()
        .map(|(module, count)| cgg_core::audit::UnresolvedModuleBucket {
            sample: buckets
                .remove(module)
                .unwrap_or_default()
                .into_iter()
                .map(str::to_string)
                .collect(),
            module: module.to_string(),
            count,
        })
        .collect();
    // Largest gap first — the whole point is to see how much is missing.
    // Ties break on the module name so the order is stable.
    out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.module.cmp(&b.module)));
    out
}
