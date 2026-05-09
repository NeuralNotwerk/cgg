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

use std::fs::File;
use std::io::{self, BufWriter, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::Parser;
use tracing::{debug, info};
use tracing_subscriber::{fmt, EnvFilter};

use cgg_core::audit::{
    AuditCallableRef, AuditEvent, AuditFileRecord, JsonAuditWriter, JsonlAuditWriter,
    RunMetrics, SkipReason,
};
use cgg_core::graph::{
    CallableKind, CallableNode, FileRecord as GraphFileRecord, Graph,
};
use cgg_core::ids::{CallableId, FileId};
use cgg_core::{FileFacts, DefVariant};
use cgg_format::{GraphFormatter, MermaidFormatter, OutputFormat};
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

    for cand in &outcome.candidates {
        metrics.bytes_processed += cand.size_bytes;
        events.push(AuditEvent::FileDiscovered {
            path: cand.path.clone(),
        });

        let det = detector.detect(&cand.path);
        let lang = match det.verdict {
            DetectVerdict::Language(id) if langs_enabled(id) => id,
            DetectVerdict::Language(id) => {
                events.push(AuditEvent::FileSkipped {
                    path: cand.path.clone(),
                    reason: SkipReason::Builtin(format!("lang-filter:{id}")),
                });
                metrics.files_skipped += 1;
                continue;
            }
            DetectVerdict::Unknown => {
                events.push(AuditEvent::FileSkipped {
                    path: cand.path.clone(),
                    reason: SkipReason::UnknownExtension,
                });
                metrics.files_skipped += 1;
                continue;
            }
        };

        let bytes = match read_file(&cand.path) {
            Ok(b) => b,
            Err(err) => {
                events.push(AuditEvent::FileSkipped {
                    path: cand.path.clone(),
                    reason: SkipReason::ParseError(err.to_string()),
                });
                metrics.files_errored += 1;
                continue;
            }
        };

        let sha = blake3::hash(&bytes).to_hex().to_string();
        let line_count = count_lines(&bytes);
        let file_id = FileId::new(next_file_id);

        let (parse_status, parse_ms, tree) = match pool.parse(lang, &bytes) {
            Ok(out) => {
                let status = if out.tree.root_node().has_error() {
                    "error"
                } else {
                    "ok"
                };
                (status.to_string(), out.parse_ms, Some(out.tree))
            }
            Err(e) => {
                debug!(path = %cand.path.display(), error = %e, "parse failed");
                ("error".to_string(), 0.0, None)
            }
        };

        // Insert FileRecord into the graph and the audit record list.
        graph.add_file(GraphFileRecord {
            id: file_id,
            path: cand.path.clone(),
            language: lang.to_string(),
            detected_via: det.detected_via.clone(),
            sha256: sha.clone(),
            size_bytes: cand.size_bytes,
            lines: line_count,
            parse_ms,
            parse_status: parse_status.clone(),
        });

        // Extract facts and insert callables into the graph.
        let mut file_audit = AuditFileRecord {
            file: file_id,
            path: cand.path.clone(),
            language: lang.to_string(),
            detected_via: det.detected_via.clone(),
            sha256: sha,
            size_bytes: cand.size_bytes,
            lines: line_count,
            parse_ms,
            parse_status,
            skip_reason: None,
            callables: Vec::new(),
            unresolved_calls: Vec::new(),
            ffi: Vec::new(),
        };

        if let (Some(tree), Some(plugin)) = (tree.as_ref(), pool.plugin(lang)) {
            let facts = plugin.extract(file_id, &cand.path, tree, &bytes);
            for (idx, d) in facts.definitions.iter().enumerate() {
                let cid = CallableId::new(next_callable_id);
                next_callable_id += 1;
                def_ids.insert((file_id, idx as u32), cid);

                let node = CallableNode {
                    id: cid,
                    qualified_name: d.qualified_name.clone(),
                    simple_name: d.simple_name.clone(),
                    kind: variant_to_kind(d.variant),
                    language: lang.to_string(),
                    file: file_id,
                    start_line: d.start_line,
                    end_line: d.end_line,
                    start_byte: d.start_byte,
                    end_byte: d.end_byte,
                    signature_hint: d.signature_hint.clone(),
                    visibility: d.visibility.clone(),
                    attributes: d.attributes.clone(),
                };
                graph.add_callable(node);

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
                .entry(lang.to_string())
                .or_default();
            lang_bucket.callables += facts.definitions.len() as u64;
            metrics.callables += facts.definitions.len() as u64;

            all_facts.push(facts);
        }
        sources.push((file_id, lang.to_string(), bytes));

        next_file_id += 1;
        metrics.files_analyzed += 1;
        metrics.phases.parse_ms += parse_ms;
        metrics
            .by_language
            .entry(lang.to_string())
            .or_default()
            .files += 1;

        file_records.push(file_audit);
    }

    // --- Phase 3: intra-file link -----------------------------------------
    let link_started = Instant::now();
    for facts in &all_facts {
        let outcome = link_file(facts, &def_ids);
        let lang = facts.language.clone();
        let lang_bucket = metrics.by_language.entry(lang).or_default();
        lang_bucket.edges += outcome.edges.len() as u64;
        lang_bucket.unresolved += outcome.unresolved.len() as u64;
        metrics.edges += outcome.edges.len() as u64;
        metrics.unresolved_calls += outcome.unresolved.len() as u64;

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

        // Attach unresolved calls to the per-file audit record.
        if let Some(rec) = file_records.iter_mut().find(|r| r.file == facts.file) {
            rec.unresolved_calls = outcome.unresolved.clone();
        }
        graph.unresolved.extend(outcome.unresolved);
    }
    let link_ms = link_started.elapsed().as_secs_f64() * 1000.0;

    // --- Phase 3b: stack-graphs resolution (Python/JS/TS/Java) ------------
    let resolve_started = Instant::now();
    let sg_inputs: Vec<cgg_resolve::stack_graphs_resolver::FileInput<'_>> = sources
        .iter()
        .map(|(fid, lang, bytes)| cgg_resolve::stack_graphs_resolver::FileInput {
            file: *fid,
            language: lang.as_str(),
            source: bytes.as_slice(),
        })
        .collect();
    let sg_out = cgg_resolve::stack_graphs_resolver::resolve(&graph, &all_facts, &sg_inputs);

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
    metrics.unresolved_calls += sg_out.unresolved.len() as u64;
    graph.edges.extend(sg_out.edges);
    graph.unresolved.extend(sg_out.unresolved);
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
    emit_graph(&cli, &graph).context("emitting graph")?;
    emit_audit(&cli, &events).context("writing audit")?;

    eprintln!(
        "cgg: {disc} files discovered, {an} analyzed, {sk} skipped; \
         {ca} callables, {ed} edges, {ur} unresolved ({ms:.1} ms). \
         [Task 5: intra-file linker wired]",
        disc = metrics.files_discovered,
        an = metrics.files_analyzed,
        sk = metrics.files_skipped,
        ca = metrics.callables,
        ed = metrics.edges,
        ur = metrics.unresolved_calls,
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

/// Emit the graph to the user-facing output destination.
///
/// * Mermaid: we own a writer; render.
/// * JSON: Task 5 wires audit-only JSON; Task 9 replaces with a
///   unified graph+audit document.
/// * DOT / GraphML: Task 9 adds these.
fn emit_graph(cli: &Cli, graph: &Graph) -> Result<()> {
    let format: OutputFormat = cli.format.into();
    match format {
        OutputFormat::Mermaid => {
            let dest = resolve_primary_sink(cli);
            let mut sink = open_sink(&dest)?;
            MermaidFormatter::new().render(graph, &mut sink)?;
        }
        OutputFormat::Json | OutputFormat::Dot | OutputFormat::Graphml => {
            // Non-mermaid formats land in Task 9; for Task 5 the audit
            // path is the only primary output for JSON. DOT/GraphML
            // are CLI-accepted but emit a friendly "coming soon" note
            // to the graph sink to avoid silent no-ops.
            if matches!(format, OutputFormat::Json) {
                // Audit emitter writes the JSON output in this mode.
                return Ok(());
            }
            let dest = resolve_primary_sink(cli);
            let mut sink = open_sink(&dest)?;
            writeln!(
                sink,
                "# {format} output not yet implemented (Task 9). \
                 {n} callables, {e} edges pending render.",
                n = graph.callables.len(),
                e = graph.edges.len()
            )?;
        }
    }
    Ok(())
}

fn resolve_primary_sink(cli: &Cli) -> PathBuf {
    cli.output
        .clone()
        .unwrap_or_else(|| PathBuf::from("-"))
}

fn emit_audit(cli: &Cli, events: &[AuditEvent]) -> Result<()> {
    let format: OutputFormat = cli.format.into();
    // Rules:
    //   * `--metrics FILE`           -> audit to FILE.
    //   * `-t json` + no --metrics   -> audit to primary output.
    //   * other formats + no metrics -> sidecar `<output>.audit.json`.
    let dest = if let Some(p) = &cli.metrics {
        p.clone()
    } else if matches!(format, OutputFormat::Json) {
        resolve_primary_sink(cli)
    } else {
        match &cli.output {
            Some(p) if *p != PathBuf::from("-") => {
                let mut s = p.clone();
                s.as_mut_os_string().push(".audit.json");
                s
            }
            _ => PathBuf::from("-"),
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
