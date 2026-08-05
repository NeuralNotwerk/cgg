//! `cgg` entry point.
//!
//! Task 5 pipeline:
//! ```text
//! walker -> language detector -> parser pool -> extract
//!        -> build Graph -> intra-file linker -> format + audit
//! ```
//!
//! Cross-file scope-aware resolution lands in Task 6; additional
//! formatters in Task 9; query engine in Task 10.

mod cli;
mod deadcode;
mod query;
mod since;

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, BufWriter, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::Parser;
use tracing::info;
use tracing_subscriber::{fmt, EnvFilter};

use rayon::prelude::*;

use cgg_core::audit::{
    AuditCallableRef, AuditEvent, AuditFileRecord, JsonAuditWriter, JsonlAuditWriter,
    RunMetrics, SkipReason,
};
use cgg_core::graph::{
    CallEdge, CallableKind, CallableNode, Confidence, FileRecord as GraphFileRecord, Graph, Via,
};
use cgg_core::ids::{CallableId, FileId, ResolverId};
use cgg_core::{
    build_known_names, classify_external, DefVariant, FileAliases, FileFacts,
};
use cgg_format::{
    DotFormatter, GraphFormatter, GraphmlFormatter, JsonFormatter, MermaidFormatter, OutputFormat,
};
use cgg_lang::{
    detect::{DetectVerdict, LanguageDetector},
    parser::ParserPool,
    PluginRegistry,
};
use cgg_resolve::intra_file::{link_file, DefIdMap};
use cgg_walk::{walk, WalkConfig};
use cli::{AuditFormatArg, Cli};

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            let code = e.exit_code();
            e.print().ok();
            return ExitCode::from(code as u8);
        }
    };

    init_tracing(&cli);

    match run(cli) {
        Ok(code) => code,
        // An error means the analysis was incomplete, so any findings it
        // did produce are untrustworthy. Errors therefore dominate the
        // findings exit code: 1 beats 3.
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn init_tracing(cli: &Cli) {
    let default_level = if cli.quiet {
        "error"
    } else {
        match cli.verbose {
            0 => "warn",
            1 => "info",
            _ => "debug",
        }
    };
    let filter = EnvFilter::try_from_env("CGG_LOG")
        .unwrap_or_else(|_| EnvFilter::new(default_level));

    let _ = fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init();
}

fn run(cli: Cli) -> Result<ExitCode> {
    info!(
        version = cgg_core::CGG_VERSION,
        paths = cli.paths.len(),
        format = %OutputFormat::from(cli.format),
        "cgg starting"
    );

    for p in &cli.paths {
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
    let dead_mode = cli.dead_code || !cli.why_live.is_empty();
    let cli = if dead_mode {
        Cli {
            reference_edges: true,
            dynamic_dispatch: true,
            include_external: false,
            include_stdlib: false,
            ..cli
        }
    } else {
        cli
    };

    // Extraction signals that only `--dead-code` consumes are an extra
    // tree walk per file; an ordinary run must not pay for them.
    cgg_lang::set_deadcode_signals(dead_mode);

    let started = Instant::now();

    // --- Phase 1: walk -----------------------------------------------------
    let cfg = WalkConfig {
        roots: cli.paths.clone(),
        extra_ignore_file: cli.ignore_file.clone(),
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

    let mut metrics = RunMetrics::default();
    metrics.files_discovered =
        (outcome.candidates.len() + outcome.skips.len()) as u64;

    let lang_filter: Vec<&str> = cli.lang.iter().map(|s| s.as_str()).collect();
    let langs_enabled = |lang: &str| -> bool {
        lang_filter.is_empty() || lang_filter.iter().any(|s| *s == lang)
    };

    let parse_started = Instant::now();
    let mut next_file_id: u32 = 0;
    let mut next_callable_id: u32 = 0;

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
    enum FileOutcome {
        Analyzed(FileResult),
        Skipped { path: std::path::PathBuf, reason: SkipReason },
    }

    // Configure rayon thread pool if --jobs specified.
    if cli.jobs > 0 {
        let _ = rayon::ThreadPoolBuilder::new()
            .num_threads(cli.jobs)
            .build_global();
    }

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

            let (parse_status, parse_ms, facts) = match pool.parse(lang, &bytes) {
                Ok(out) => {
                    let status = if out.tree.root_node().has_error() {
                        "error"
                    } else {
                        "ok"
                    };
                    let plugin = pool.plugin(lang);
                    let facts = plugin.map(|p| {
                        p.extract(FileId::new(0), &cand.path, &out.tree, &bytes)
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
    for result in results {
        match result {
            FileOutcome::Skipped { path, reason } => {
                events.push(AuditEvent::FileDiscovered { path: path.clone() });
                events.push(AuditEvent::FileSkipped { path, reason: reason.clone() });
                if matches!(reason, SkipReason::ParseError(_)) {
                    metrics.files_errored += 1;
                } else {
                    metrics.files_skipped += 1;
                }
            }
            FileOutcome::Analyzed(fr) => {
                events.push(AuditEvent::FileDiscovered { path: fr.path.clone() });
                metrics.bytes_processed += fr.size_bytes;

                let file_id = FileId::new(next_file_id);
                next_file_id += 1;

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
                        let cid = CallableId::new(next_callable_id);
                        next_callable_id += 1;
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
                            trait_impl_target: trait_impl_target_from_qn(&d.qualified_name),
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

                    let lang_bucket = metrics
                        .by_language
                        .entry(fr.lang.clone())
                        .or_default();
                    lang_bucket.callables += facts.definitions.len() as u64;
                    metrics.callables += facts.definitions.len() as u64;

                    all_facts.push(facts);
                }

                metrics.files_analyzed += 1;
                metrics.phases.parse_ms += fr.parse_ms;
                metrics
                    .by_language
                    .entry(fr.lang)
                    .or_default()
                    .files += 1;

                file_records.push(file_audit);
            }
        }
    }

    // --- Phase 3: type propagation + intra-file link -----------------------
    let link_started = Instant::now();
    let return_types_owned: HashMap<String, String> = cgg_resolve::type_hints::build_return_type_map(&all_facts)
        .into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let return_types: HashMap<&str, &str> = return_types_owned.iter()
        .map(|(k, v)| (k.as_str(), v.as_str())).collect();
    for facts in &mut all_facts {
        cgg_resolve::type_hints::propagate_types_with_returns(facts, &return_types);
    }
    let known_names = build_known_names(&all_facts);
    for facts in &all_facts {
        let outcome = link_file(facts, &def_ids);
        let lang = facts.language.clone();
        let lang_bucket = metrics.by_language.entry(lang).or_default();
        lang_bucket.edges += outcome.edges.len() as u64;

        // Fold edges into the graph and per-file audit.
        for e in &outcome.edges {
            match e.confidence {
                cgg_core::graph::Confidence::High => {
                    metrics.confidence_histogram.high += 1
                }
                cgg_core::graph::Confidence::Medium => {
                    metrics.confidence_histogram.medium += 1
                }
                cgg_core::graph::Confidence::Low => {
                    metrics.confidence_histogram.low += 1
                }
            }
        }
        graph.edges.extend(outcome.edges.clone());

        // Classify unresolved calls into unresolved / stdlib / external.
        let known_refs: std::collections::HashSet<&str> = known_names.iter().map(|s| s.as_str()).collect();
        let aliases = FileAliases::from_facts(&facts);
        let mut per_file_aliases = std::collections::HashMap::new();
        per_file_aliases.insert(facts.file, aliases);
        let classified = classify_external(
            outcome.unresolved,
            &known_refs,
            &facts.language,
            Some(&per_file_aliases),
        );

        let lang = facts.language.clone();
        let lang_bucket = metrics.by_language.entry(lang).or_default();
        lang_bucket.unresolved += classified.unresolved.len() as u64;
        lang_bucket.stdlib += classified.stdlib.len() as u64;
        lang_bucket.external += classified.external.len() as u64;
        metrics.unresolved_calls += classified.unresolved.len() as u64;
        metrics.stdlib_calls += classified.stdlib.len() as u64;
        metrics.external_calls += classified.external.len() as u64;
        metrics.edges += outcome.edges.len() as u64;

        // Attach the three buckets to the per-file audit record.
        if let Some(rec) = file_records.iter_mut().find(|r| r.file == facts.file) {
            rec.unresolved_calls = classified.unresolved.clone();
            rec.stdlib_calls = classified.stdlib.clone();
            rec.external_calls = classified.external.clone();
        }
        graph.unresolved.extend(classified.unresolved);
    }
    // Reference edges (function-as-value, Issue 4) are captured during
    // extraction but only surfaced under `--reference-edges`. Drop them
    // here when off, before reconciliation/metrics see them.
    if !cli.reference_edges {
        graph.edges.retain(|e| !matches!(e.via, Via::Reference));
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
    // working, but it selects between three identical behaviours.
    let _ = cli.stack_graphs;

    // --- Phase 3c: cross-file import-chain resolver -----------------------
    let cf_out = cgg_resolve::cross_file::resolve(&graph, &all_facts);
    for e in &cf_out.edges {
        match e.confidence {
            cgg_core::graph::Confidence::High => {
                metrics.confidence_histogram.high += 1
            }
            cgg_core::graph::Confidence::Medium => {
                metrics.confidence_histogram.medium += 1
            }
            cgg_core::graph::Confidence::Low => {
                metrics.confidence_histogram.low += 1
            }
        }
    }
    metrics.edges += cf_out.edges.len() as u64;
    graph.edges.extend(cf_out.edges);

    // --- Phase 3d: FFI linker (cross-language edges) ----------------------
    let ffi_out = cgg_resolve::ffi::link_ffi(&graph, &all_facts);
    for e in &ffi_out.edges {
        match e.confidence {
            cgg_core::graph::Confidence::High => metrics.confidence_histogram.high += 1,
            cgg_core::graph::Confidence::Medium => metrics.confidence_histogram.medium += 1,
            cgg_core::graph::Confidence::Low => metrics.confidence_histogram.low += 1,
        }
    }
    metrics.edges += ffi_out.edges.len() as u64;
    graph.edges.extend(ffi_out.edges);

    // --- Phase 3e: audit reconciliation -----------------------------------
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

    metrics.unresolved_calls = metrics.unresolved_calls.saturating_sub(total_removed_unresolved);
    metrics.stdlib_calls = metrics.stdlib_calls.saturating_sub(total_removed_stdlib);
    metrics.external_calls = metrics.external_calls.saturating_sub(total_removed_external);
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

    // Synthesize external/stdlib exit nodes from the *post-reconciliation*
    // buckets, so calls that a later resolver bound are not surfaced.
    if cli.include_external || cli.include_stdlib {
        synthesize_exit_nodes(
            &mut graph,
            &file_records,
            &mut next_file_id,
            &mut next_callable_id,
            cli.include_external,
            cli.include_stdlib,
        );
    }

    // Interface/trait dynamic-dispatch fan-out (Issue 3). Over-approximated
    // declaration → implementation edges, tagged `Via::Dynamic`; opt-in.
    if cli.dynamic_dispatch {
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
    events.push(AuditEvent::RunFinished {
        metrics: metrics.clone(),
    });

    // --- Phase 4: emit ----------------------------------------------------
    // Deduplicate edges (same src+dst+site_byte, keep highest confidence).
    dedup_edges(&mut graph);

    // `--since` augments `--filter` with the qualified names of every
    // callable whose body overlaps a changed line range from the diff.
    let mut effective_filters = cli.filter.clone();
    if let Some(revspec) = cli.since.as_deref() {
        let cwd = std::env::current_dir().context("getting current dir for --since")?;
        let ranges = since::resolve_since(revspec, &cwd)
            .with_context(|| format!("resolving --since {revspec}"))?;
        let (seeds, unmatched) = since_seeds(&graph, &ranges);
        events.push(AuditEvent::SinceResolved {
            revspec: revspec.to_string(),
            files_changed: ranges.len() as u64,
            matched_seeds: seeds.clone(),
            unmatched_files: unmatched.clone(),
        });
        eprintln!(
            "cgg: --since {revspec}: {} file(s) changed, {} callable seed(s), {} unmatched file(s)",
            ranges.len(),
            seeds.len(),
            unmatched.len()
        );
        for name in &seeds {
            // Anchor with `^…$` so each seed selects exactly that one
            // qualified name, not anything containing it.
            effective_filters.push(format!("^{}$", regex::escape(name)));
        }
    }

    // --- Dead-code analysis (pre-query) ------------------------------------
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
    let mut exit = ExitCode::SUCCESS;
    let mut graph = graph;
    if dead_mode {
        // `--why-live` asks the opposite question, so it does replace
        // the output: its answer is a proof, not a graph.
        if !cli.why_live.is_empty() {
            let code = run_why_live(&cli, &graph, &all_facts)?;
            emit_audit(&cli, &events).context("writing audit")?;
            return Ok(code);
        }
        exit = run_dead_code(
            &cli,
            &mut graph,
            &dead_file_records,
            &all_facts,
            &effective_filters,
        )
        .context("running dead-code analysis")?;
    }

    let graph = query::apply_query(&graph, &effective_filters, cli.hops, cli.max_paths)
        .map_err(|e| anyhow::anyhow!(e))?;
    let graph = query::apply_exclusions(
        &graph, &cli.exclude_partial, &cli.exclude_glob, &cli.exclude_regex,
    )
    .map_err(|e| anyhow::anyhow!(e))?;
    emit_graph(&cli, &graph).context("emitting graph")?;
    emit_audit(&cli, &events).context("writing audit")?;

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

    eprintln!(
        "cgg: {disc} files, {an} analyzed, {sk} skipped{breakdown}; \
         {ca} callables, {ed} edges ({cf} cross-file), \
         {ur} unresolved, {sl} stdlib, {ext} external ({ms:.1} ms)",
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
    );

    // Actionable hint when --lang excluded files whose language IS
    // supported by a plugin. Listing each excluded language with its
    // count and the suggested `--lang` value tells the user exactly
    // what to add.
    if !lang_filter_counts.is_empty() {
        let mut pairs: Vec<_> = lang_filter_counts.into_iter().collect();
        pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let user_langs = if cli.lang.is_empty() {
            String::new()
        } else {
            cli.lang.join(",")
        };
        for (lang, n) in pairs {
            let suggestion = if user_langs.is_empty() {
                format!("--lang {lang}")
            } else {
                format!("--lang {user_langs},{lang}")
            };
            eprintln!(
                "note: {n} file(s) detected as '{lang}' were excluded by --lang; \
                 pass `{suggestion}` to include them"
            );
        }
    }

    Ok(exit)
}

/// Run the dead-code analysis and render its report.
///
/// Returns the process exit code: `3` only when `--fail-on-dead` was
/// asked for and the report is non-empty. The default is unchanged, so
/// adding `--dead-code` to an existing pipeline cannot break it — and a
/// tool whose every finding is explicitly a hypothesis has no business
/// failing a build unless told to.
fn run_dead_code(
    cli: &Cli,
    graph: &mut Graph,
    file_records: &[AuditFileRecord],
    all_facts: &[FileFacts],
    filters: &[String],
) -> Result<ExitCode> {
    use cgg_resolve::deadcode::{analyze, DeadCodeOptions};

    use deadcode::config::DeadCodeConfigFile;

    let threshold: Confidence = cli.dead_code_confidence.into();

    // Load declared roots / accepted findings. An explicit --roots
    // disables the upward search, so a scripted run can be pinned.
    let cfg_path = match &cli.roots {
        Some(p) => Some(p.clone()),
        None => std::env::current_dir()
            .ok()
            .and_then(|d| DeadCodeConfigFile::discover(&d)),
    };
    let cfg = match &cfg_path {
        Some(p) => DeadCodeConfigFile::load(p)?,
        None => DeadCodeConfigFile::default(),
    };

    // Declared roots confer liveness, so they are resolved against the
    // graph before the analysis runs.
    let user_roots: Vec<(String, cgg_core::ids::CallableId)> = {
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
        v
    };

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
    let language_signals: std::collections::BTreeMap<String, cgg_core::LanguageSignals> = registry
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

    let opts = DeadCodeOptions {
        user_roots,
        language_signals,
        include_tests: cli.include_tests,
        reference_edges: cli.reference_edges,
        dynamic_dispatch: cli.dynamic_dispatch,
        confidence_threshold: format!("{threshold:?}").to_lowercase(),
        roots_file: cfg_path.clone(),
        ..Default::default()
    };

    let mut report = analyze(graph, file_records, all_facts, &opts);

    // Scope the *findings*, never the graph. Excluding a caller from the
    // graph would delete its outgoing edges and manufacture findings for
    // its callees — the failure mode of file-level exclusion in
    // name-matching tools.
    let allow_pats: Vec<String> = cfg.allow.iter().map(|a| a.name.clone()).collect();
    {
        let keep = query::compile_patterns(filters).map_err(|e| anyhow::anyhow!(e))?;
        let mut drop_pats = cli.ignore_names.clone();
        drop_pats.extend(allow_pats.iter().cloned());
        let drop = query::compile_patterns(&drop_pats).map_err(|e| anyhow::anyhow!(e))?;
        let attr_pats =
            query::compile_patterns(&cli.ignore_attributes).map_err(|e| anyhow::anyhow!(e))?;

        let mut used_allow: std::collections::HashSet<usize> = Default::default();
        report.findings.retain(|f| {
            let included = keep.is_empty() || keep.iter().any(|p| p.matches(&f.qualified_name));
            let ignored = drop.iter().enumerate().any(|(i, p)| {
                let hit = p.matches(&f.qualified_name);
                if hit && i >= cli.ignore_names.len() {
                    used_allow.insert(i - cli.ignore_names.len());
                }
                hit
            });
            let attr_ignored = !attr_pats.is_empty()
                && graph.callables.get(&f.id).is_some_and(|c| {
                    c.attributes.iter().any(|a| attr_pats.iter().any(|p| p.matches(a)))
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

    if !cli.ignore_attributes.is_empty()
        && graph.callables.values().all(|c| c.attributes.is_empty())
    {
        eprintln!(
            "note: --ignore-attributes matched nothing — no callable in this run \
             carries attributes (attribute capture: python, rust)"
        );
    }

    if cli.write_roots {
        // A baseline is a config file, not a graph, so it goes to the
        // primary sink and nothing else is emitted.
        let dest = cli.output.clone().unwrap_or_else(|| PathBuf::from("-"));
        let mut sink = open_sink(&dest)?;
        write!(sink, "{}", deadcode::config::render_baseline(&report))?;
        sink.flush()?;
        std::process::exit(0);
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

    // The detailed report — evidence, roots, capability table — goes to
    // a sidecar, exactly as the audit does. `<output>.deadcode.json`
    // beside the graph, or `--dead-code-report FILE`.
    if let Some(path) = dead_code_report_path(cli) {
        let mut sink = open_sink(&path)?;
        match cli.dead_code_format {
            cli::DeadCodeFormatArg::Text => {
                deadcode::report::render_text(&report, threshold, &mut sink)?
            }
            cli::DeadCodeFormatArg::Json => deadcode::report::render_json(&report, &mut sink)?,
        }
        sink.flush()?;
    }

    if !cli.quiet {
        eprintln!(
            "cgg: dead-code: {shown} callable(s) marked unreferenced at {} confidence, \
             {} withheld — BEST EFFORT, every finding is a hypothesis",
            format!("{threshold:?}").to_lowercase(),
            report.findings.len().saturating_sub(shown),
        );
    }

    Ok(if cli.fail_on_dead && shown > 0 {
        ExitCode::from(3)
    } else {
        ExitCode::SUCCESS
    })
}

/// Where the detailed dead-code report goes.
///
/// Mirrors `emit_audit`: an explicit `--dead-code-report` wins,
/// otherwise a sidecar beside `-o`, and nothing at all when the graph is
/// going to stdout (the graph is the thing being piped; a report
/// interleaved into it would corrupt it).
fn dead_code_report_path(cli: &Cli) -> Option<PathBuf> {
    if let Some(p) = &cli.dead_code_report {
        return Some(p.clone());
    }
    match &cli.output {
        Some(p) if *p != PathBuf::from("-") => {
            let mut s = p.clone();
            s.as_mut_os_string().push(".deadcode.json");
            Some(s)
        }
        _ => None,
    }
}

/// `--why-live`: print the shortest path from a root proving a callable
/// is live. A query, not a graph, so it does replace the output.
fn run_why_live(cli: &Cli, graph: &Graph, all_facts: &[FileFacts]) -> Result<ExitCode> {
    use cgg_resolve::deadcode::{why_live, DeadCodeOptions};
    let opts = DeadCodeOptions {
        include_tests: cli.include_tests,
        reference_edges: cli.reference_edges,
        dynamic_dispatch: cli.dynamic_dispatch,
        ..Default::default()
    };
    let targets =
        query::match_callables(graph, &cli.why_live).map_err(|e| anyhow::anyhow!(e))?;
    if targets.is_empty() {
        eprintln!("cgg: --why-live matched no callables");
    }
    let dest = cli.output.clone().unwrap_or_else(|| PathBuf::from("-"));
    let mut sink = open_sink(&dest)?;
    let proofs = why_live(graph, all_facts, &opts, &targets);
    deadcode::report::render_why_live(&proofs, &mut sink)?;
    sink.flush()?;
    Ok(ExitCode::SUCCESS)
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

    (
        seeds.into_iter().collect(),
        unmatched.into_iter().collect(),
    )
}

fn count_lines(bytes: &[u8]) -> u32 {
    let mut n = bytes.iter().filter(|&&b| b == b'\n').count() as u32;
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        n += 1;
    }
    n
}

fn read_file(path: &std::path::Path) -> Result<Vec<u8>> {
    let mut f = File::open(path)
        .with_context(|| format!("opening {}", path.display()))?;
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
    next_file_id: &mut u32,
    next_callable_id: &mut u32,
    include_external: bool,
    include_stdlib: bool,
) {
    let resolver = ResolverId::new("exit-node");
    // (language, receiver, name, is_external) -> exit node id.
    let mut node_ids: HashMap<(String, String, String, bool), CallableId> = HashMap::new();
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
                    let file_id = if is_external {
                        *external_file.get_or_insert_with(|| {
                            let fid = FileId::new(*next_file_id);
                            *next_file_id += 1;
                            graph.add_file(sentinel_file(fid, "<external>", "external"))
                        })
                    } else {
                        *stdlib_file.get_or_insert_with(|| {
                            let fid = FileId::new(*next_file_id);
                            *next_file_id += 1;
                            graph.add_file(sentinel_file(fid, "<stdlib>", "stdlib"))
                        })
                    };
                    let id = CallableId::new(*next_callable_id);
                    *next_callable_id += 1;
                    let qn = if call.receiver_hint.is_empty() {
                        format!("<{kind_label}>::{}", call.name)
                    } else {
                        format!("<{kind_label}>::{}::{}", call.receiver_hint, call.name)
                    };
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
                });
            }
        }
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
    use std::collections::HashMap;
    use cgg_core::graph::Confidence;
    let mut best: HashMap<(u32, u32, u32), usize> = HashMap::new();
    let conf_rank = |c: Confidence| match c {
        Confidence::High => 2,
        Confidence::Medium => 1,
        Confidence::Low => 0,
    };
    for (i, e) in graph.edges.iter().enumerate() {
        let key = (e.src.as_u32(), e.dst.as_u32(), e.site_byte);
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

/// Emit the graph to the user-facing output destination.
fn emit_graph(cli: &Cli, graph: &Graph) -> Result<()> {
    let format: OutputFormat = cli.format.into();
    let dest = resolve_primary_sink(cli);
    let mut sink = open_sink(&dest)?;
    let formatter: Box<dyn GraphFormatter> = match format {
        OutputFormat::Mermaid => Box::new(MermaidFormatter::new()),
        OutputFormat::Json => Box::new(JsonFormatter::new()),
        OutputFormat::Dot => Box::new(DotFormatter::new()),
        OutputFormat::Graphml => Box::new(GraphmlFormatter::new()),
    };
    formatter.render(graph, &mut sink)?;
    Ok(())
}

fn resolve_primary_sink(cli: &Cli) -> PathBuf {
    cli.output
        .clone()
        .unwrap_or_else(|| PathBuf::from("-"))
}

fn emit_audit(cli: &Cli, events: &[AuditEvent]) -> Result<()> {
    // Rules:
    //   * `--metrics FILE`           -> audit to FILE.
    //   * other formats + no metrics -> sidecar `<output>.audit.json`.
    //   * no output file             -> stderr (skip audit).
    let dest = if let Some(p) = &cli.metrics {
        p.clone()
    } else {
        match &cli.output {
            Some(p) if *p != PathBuf::from("-") => {
                let mut s = p.clone();
                s.as_mut_os_string().push(".audit.json");
                s
            }
            _ => return Ok(()), // No sidecar for stdout output
        }
    };
    let sink = open_sink(&dest)?;

    match cli.audit_format {
        AuditFormatArg::Jsonl => {
            let mut w = JsonlAuditWriter::new(sink);
            for e in events {
                use cgg_core::AuditWriter as _;
                w.emit(e)?;
            }
            use cgg_core::AuditWriter as _;
            w.flush()?;
        }
        AuditFormatArg::Json => {
            let mut w = JsonAuditWriter::new(sink);
            for e in events {
                use cgg_core::AuditWriter as _;
                w.emit(e)?;
            }
            w.finalize()?;
        }
    }
    Ok(())
}

fn open_sink(dest: &PathBuf) -> Result<Box<dyn Write + Send>> {
    if dest == &PathBuf::from("-") {
        Ok(Box::new(BufWriter::new(io::stdout())))
    } else {
        let f = File::create(dest)
            .with_context(|| format!("creating output file {}", dest.display()))?;
        Ok(Box::new(BufWriter::new(f)))
    }
}


