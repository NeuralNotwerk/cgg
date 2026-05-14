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
mod query;

use std::collections::HashMap;
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
    CallableKind, CallableNode, FileRecord as GraphFileRecord, Graph,
};
use cgg_core::ids::{CallableId, FileId};
use cgg_core::{classify_external, build_known_names, FileFacts, DefVariant};
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
        Ok(()) => ExitCode::SUCCESS,
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

fn run(cli: Cli) -> Result<()> {
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
    // Retained source bytes per file, used by the stack-graphs resolver.
    let mut sources: Vec<(FileId, String, Vec<u8>)> = Vec::new();

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
        bytes: Vec<u8>,
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
                        reason: SkipReason::Builtin(format!("lang-filter:{id}")),
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
                bytes,
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
                });

                let mut file_audit = AuditFileRecord {
                    file: file_id,
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
                            attributes: d.attributes.clone(),
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
                sources.push((file_id, fr.lang.clone(), fr.bytes));

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

        // Classify unresolved calls as internal vs external.
        let known_refs: std::collections::HashSet<&str> = known_names.iter().map(|s| s.as_str()).collect();
        let classified = classify_external(outcome.unresolved, &known_refs, &facts.language);

        let lang = facts.language.clone();
        let lang_bucket = metrics.by_language.entry(lang).or_default();
        lang_bucket.unresolved += classified.unresolved.len() as u64;
        lang_bucket.external += classified.external.len() as u64;
        metrics.unresolved_calls += classified.unresolved.len() as u64;
        metrics.external_calls += classified.external.len() as u64;
        metrics.edges += outcome.edges.len() as u64;

        // Attach only truly-unresolved calls to the per-file audit record.
        if let Some(rec) = file_records.iter_mut().find(|r| r.file == facts.file) {
            rec.unresolved_calls = classified.unresolved.clone();
            rec.external_calls = classified.external.clone();
        }
        graph.unresolved.extend(classified.unresolved);
    }
    let link_ms = link_started.elapsed().as_secs_f64() * 1000.0;

    // --- Phase 3b: stack-graphs resolution (with timeout) ------------------
    let resolve_started = Instant::now();
    let sg_out = match cli.stack_graphs {
        cli::StackGraphsArg::Off => {
            cgg_resolve::stack_graphs_resolver::ResolveOutput::default()
        }
        cli::StackGraphsArg::On => {
            let sg_inputs: Vec<cgg_resolve::stack_graphs_resolver::FileInput<'_>> = sources
                .iter()
                .map(|(fid, lang, bytes)| cgg_resolve::stack_graphs_resolver::FileInput {
                    file: *fid,
                    language: lang.as_str(),
                    source: bytes.as_slice(),
                })
                .collect();
            cgg_resolve::stack_graphs_resolver::resolve(&graph, &all_facts, &sg_inputs)
        }
        cli::StackGraphsArg::Auto => {
            // Run full resolve in a detached thread with a 60-second timeout.
            // If it times out, run the lightweight BFS-based resolver instead.
            let graph_clone = graph.clone();
            let facts_clone = all_facts.clone();
            let owned_sources: Vec<(FileId, String, Vec<u8>)> = sources
                .iter()
                .map(|(fid, lang, bytes)| (*fid, lang.clone(), bytes.clone()))
                .collect();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let sg_inputs: Vec<cgg_resolve::stack_graphs_resolver::FileInput<'_>> =
                    owned_sources
                        .iter()
                        .map(|(fid, lang, bytes)| {
                            cgg_resolve::stack_graphs_resolver::FileInput {
                                file: *fid,
                                language: lang.as_str(),
                                source: bytes.as_slice(),
                            }
                        })
                        .collect();
                let result = cgg_resolve::stack_graphs_resolver::resolve(
                    &graph_clone,
                    &facts_clone,
                    &sg_inputs,
                );
                let _ = tx.send(result);
            });
            rx.recv_timeout(std::time::Duration::from_secs(60))
                .unwrap_or_else(|_| {
                    eprintln!(
                        "cgg: stack-graphs full resolve timed out (>60s), \
                         running lightweight fallback"
                    );
                    // Run the light BFS-based resolver (fast, no path stitching).
                    // Skip if there are too many files (tsg compilation dominates).
                    let sg_inputs: Vec<cgg_resolve::stack_graphs_resolver::FileInput<'_>> =
                        sources
                            .iter()
                            .filter(|(_, lang, _)| {
                                cgg_resolve::stack_graphs_resolver::is_sg_language(lang)
                            })
                            .map(|(fid, lang, bytes)| {
                                cgg_resolve::stack_graphs_resolver::FileInput {
                                    file: *fid,
                                    language: lang.as_str(),
                                    source: bytes.as_slice(),
                                }
                            })
                            .collect();
                    if sg_inputs.len() > 200 {
                        eprintln!(
                            "cgg: too many files ({}) for light fallback, skipping",
                            sg_inputs.len()
                        );
                        cgg_resolve::stack_graphs_resolver::ResolveOutput::default()
                    } else {
                        cgg_resolve::stack_graphs_resolver::resolve_light(
                            &graph, &all_facts, &sg_inputs,
                        )
                    }
                })
        }
    };

    for e in &sg_out.edges {
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
    metrics.edges += sg_out.edges.len() as u64;
    let sg_classified = {
        let known_refs: std::collections::HashSet<&str> = known_names.iter().map(|s| s.as_str()).collect();
        classify_external(sg_out.unresolved, &known_refs, "")
    };
    metrics.unresolved_calls += sg_classified.unresolved.len() as u64;
    metrics.external_calls += sg_classified.external.len() as u64;
    graph.edges.extend(sg_out.edges);
    graph.unresolved.extend(sg_classified.unresolved);
    let resolve_ms = resolve_started.elapsed().as_secs_f64() * 1000.0;
    metrics.phases.resolve_ms = resolve_ms;

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

    let graph = query::apply_query(&graph, &cli.filter, cli.hops, cli.max_paths);
    let graph = query::apply_exclusions(
        &graph, &cli.exclude_partial, &cli.exclude_glob, &cli.exclude_regex,
    );
    emit_graph(&cli, &graph).context("emitting graph")?;
    emit_audit(&cli, &events).context("writing audit")?;

    let cross_file = metrics.edges - graph.edges.iter()
        .filter(|e| {
            graph.callables.get(&e.src).map(|s| s.file)
                == graph.callables.get(&e.dst).map(|d| d.file)
        })
        .count() as u64;
    eprintln!(
        "cgg: {disc} files, {an} analyzed, {sk} skipped; \
         {ca} callables, {ed} edges ({cf} cross-file), \
         {ur} unresolved, {ext} external ({ms:.1} ms)",
        disc = metrics.files_discovered,
        an = metrics.files_analyzed,
        sk = metrics.files_skipped,
        ca = metrics.callables,
        ed = metrics.edges,
        cf = cross_file,
        ur = metrics.unresolved_calls,
        ext = metrics.external_calls,
        ms = metrics.wall_ms
    );

    Ok(())
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


