//! Writing a [`RunOutcome`] out: sinks, formats, sidecars, stderr.
//!
//! Every file descriptor cgg touches is touched here — which is why the
//! Python module needs none of it.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use cgg_core::audit::{AuditEvent, JsonAuditWriter, JsonlAuditWriter};
use cgg_core::graph::{Confidence, Graph};
use cgg_format::{
    DotFormatter, GraphFormatter, GraphmlFormatter, JsonFormatter, MermaidFormatter,
    OutputFormat,
};

use crate::cli::{AuditFormatArg, Cli, DeadCodeFormatArg};
use crate::deadcode;
use crate::outcome::{Emission, RunOutcome};

/// The formatter for `f`. One factory, so a fifth output format is one
/// edit rather than two.
fn formatter(f: OutputFormat) -> Box<dyn GraphFormatter> {
    match f {
        OutputFormat::Mermaid => Box::new(MermaidFormatter::new()),
        OutputFormat::Json => Box::new(JsonFormatter::new()),
        OutputFormat::Dot => Box::new(DotFormatter::new()),
        OutputFormat::Graphml => Box::new(GraphmlFormatter::new()),
    }
}

/// Open the primary sink, or a named file. `-` means stdout.
fn open_sink(dest: &Path) -> Result<Box<dyn Write + Send>> {
    if dest == Path::new("-") {
        Ok(Box::new(BufWriter::new(io::stdout())))
    } else {
        let f = File::create(dest)
            .with_context(|| format!("creating output file {}", dest.display()))?;
        Ok(Box::new(BufWriter::new(f)))
    }
}

/// Where the graph (or the `--why-live` proof, or the baseline) goes.
fn primary_sink(cli: &Cli) -> PathBuf {
    cli.output.clone().unwrap_or_else(|| PathBuf::from("-"))
}

/// Render `graph` in the format `-t` selected.
fn graph(cli: &Cli, graph: &Graph) -> Result<()> {
    let format: OutputFormat = cli.format.into();
    let dest = primary_sink(cli);
    let mut sink = open_sink(&dest)?;
    // Streams into the sink rather than going through `graph_to_string`,
    // which would buffer the whole rendering first.
    formatter(format).render(graph, &mut sink)?;
    Ok(())
}

/// Render a graph to a `String`, for callers with no file descriptor.
/// Used by `crates/cgg-py`.
pub fn graph_to_string(g: &Graph, format: OutputFormat) -> String {
    // Seeded so a large graph does not walk up through ~22 doubling
    // reallocations. Rough: mermaid averages a few dozen bytes per node.
    let mut buf = Vec::with_capacity((g.callables.len() + g.edges.len()) * 48);
    // Formatters only fail on a failing writer, and `Vec` cannot fail.
    formatter(format)
        .render(g, &mut buf)
        .expect("formatters cannot fail writing to a Vec");
    String::from_utf8(buf).expect("formatters emit UTF-8")
}

/// Where a sidecar goes: an explicit override wins, else `-o` plus `ext`,
/// else nowhere — because then the graph owns stdout and anything else
/// interleaved into it would corrupt it.
///
/// Shared by the audit and the dead-code report, which differ only in their
/// override flag and extension.
fn sidecar(cli: &Cli, explicit: Option<&Path>, ext: &str) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return Some(p.to_path_buf());
    }
    match &cli.output {
        Some(p) if p.as_os_str() != "-" => {
            let mut s = p.clone();
            s.as_mut_os_string().push(ext);
            Some(s)
        }
        _ => None,
    }
}

/// Write the audit document.
///
/// Rules, unchanged from 0.5.0:
///   * `--metrics FILE`  -> audit to FILE.
///   * otherwise `-o P`  -> sidecar `P.audit.json`.
///   * no output file    -> nothing (the graph owns stdout).
fn audit(cli: &Cli, events: &[AuditEvent]) -> Result<()> {
    let Some(dest) = sidecar(cli, cli.metrics.as_deref(), ".audit.json") else {
        return Ok(());
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

/// Where the detailed dead-code report goes.
///
/// Mirrors [`audit`]: an explicit `--dead-code-report` wins, otherwise a
/// sidecar beside `-o`, and `None` when the graph is going to stdout (the
/// graph is the thing being piped; a report interleaved into it would
/// corrupt it). The caller sends the `None` case to stderr rather than
/// discarding it.
///
/// The extension follows `--dead-code-format`. Writing the ranked text
/// report to a file named `.deadcode.json` — which is what this did
/// before — breaks every consumer that trusts a suffix, starting with
/// `jq`.
fn dead_code_report_path(cli: &Cli) -> Option<PathBuf> {
    let ext = match cli.dead_code_format {
        DeadCodeFormatArg::Text => ".deadcode.txt",
        DeadCodeFormatArg::Json => ".deadcode.json",
    };
    sidecar(cli, cli.dead_code_report.as_deref(), ext)
}

/// Write the dead-code report to its sidecar, or to stderr when there is
/// none to derive.
///
/// Text falls back to stderr; JSON does not, because interleaving it with
/// the summary lines would be worse than saying where to put it.
fn dead_code_report(
    cli: &Cli,
    report: &cgg_core::deadcode::DeadCodeReport,
    threshold: Confidence,
) -> Result<()> {
    match dead_code_report_path(cli) {
        Some(path) => {
            let mut sink = open_sink(&path)?;
            match cli.dead_code_format {
                DeadCodeFormatArg::Text => {
                    deadcode::report::render_text(report, threshold, &mut sink)?
                }
                DeadCodeFormatArg::Json => {
                    deadcode::report::render_json(report, &mut sink)?
                }
            }
            sink.flush()?;
        }
        None => match cli.dead_code_format {
            DeadCodeFormatArg::Text if !cli.quiet => {
                let mut err = io::stderr();
                deadcode::report::render_text(report, threshold, &mut err)?;
                err.flush()?;
            }
            // With the graph suppressed, stdout is free and the report is
            // what the run is for.
            DeadCodeFormatArg::Json if cli.no_graph => {
                let mut sink = io::stdout();
                deadcode::report::render_json(report, &mut sink)?;
                sink.flush()?;
            }
            // Otherwise stdout belongs to the graph, so there is nowhere
            // to put it. Exiting 0 after discarding the report is a
            // silent-failure trap for scripted use — the note was right
            // and the exit code undermined it.
            DeadCodeFormatArg::Json => {
                anyhow::bail!(
                    "--dead-code-format json has nowhere to go: stdout is \
                     carrying the graph. Pass `-o FILE` (report lands at \
                     FILE.deadcode.json), `--dead-code-report FILE`, or \
                     `--no-graph` to send the report to stdout."
                );
            }
            _ => {}
        },
    }
    Ok(())
}

/// Write everything a `cgg` invocation writes, in the order it writes it.
///
/// [`RunOutcome::transcript`] already *is* that order, so this makes no
/// ordering decisions of its own — which is the point. See [`Emission`].
pub fn all(cli: &Cli, outcome: &RunOutcome) -> Result<()> {
    for e in &outcome.transcript {
        match e {
            Emission::Diagnostic { text, quiet } => {
                if !(cli.quiet && *quiet) {
                    eprint!("{text}");
                }
            }
            // `--no-graph` drops the artifact, not the analysis: every
            // diagnostic and report still lands, in the same order.
            Emission::Graph => {
                if !cli.no_graph {
                    graph(cli, &outcome.graph)?;
                }
            }
            Emission::Unreferenced(findings) => {
                write_primary(cli, |sink| unreferenced_report(findings, sink))?;
            }
            Emission::WhyLive(proofs) => {
                write_primary(cli, |sink| {
                    deadcode::report::render_why_live(proofs, sink)
                })?;
            }
            Emission::RootsBaseline(toml) => {
                write_primary(cli, |sink| write!(sink, "{toml}"))?;
            }
            Emission::DeadCodeReport => {
                if let Some(report) = &outcome.dead_code {
                    // The threshold the analysis applied, not one re-derived
                    // from `Cli` — see `RunOutcome::dead_code_threshold`.
                    dead_code_report(cli, report, outcome.dead_code_threshold)?;
                }
            }
            Emission::Audit => audit(cli, &outcome.events)?,
        }
    }
    Ok(())
}

/// Write to the primary sink and flush.
///
/// Shared by the two artifacts that replace the graph; the graph itself
/// streams through a formatter instead.
fn write_primary(
    cli: &Cli,
    render: impl FnOnce(&mut dyn Write) -> io::Result<()>,
) -> Result<()> {
    let dest = primary_sink(cli);
    let mut sink = open_sink(&dest)?;
    render(&mut sink)?;
    sink.flush()?;
    Ok(())
}

/// Render `--report-unreferenced`.
///
/// Two buckets, because they carry different weight. A callable nothing
/// points at *and* that no root rule explains is the finding — a class
/// documented as a contract between two pipeline stages and imported by
/// nothing, which is the case that prompted this mode. One that a root
/// rule does explain is listed separately rather than hidden, so the
/// reader can check the rule rather than trust it.
fn unreferenced_report(
    findings: &[crate::outcome::UnreferencedFinding],
    out: &mut dyn Write,
) -> io::Result<()> {
    let (explained, bare): (Vec<_>, Vec<_>) =
        findings.iter().partition(|f| f.root.is_some());

    writeln!(out, "cgg unreferenced-callable report")?;
    writeln!(
        out,
        "\nNothing in the analyzed tree points at these. This is a \
         *reference* check,\nnot reachability: no cascade, so nothing \
         here is guilty by association. A\ncaller outside the tree, or \
         through reflection, is still possible."
    )?;

    writeln!(
        out,
        "\n== unreferenced, and no root rule explains it ({}) ==",
        bare.len()
    )?;
    for f in &bare {
        writeln!(
            out,
            "  {:<52} {}:{}",
            f.qualified_name, f.path, f.start_line
        )?;
    }
    if bare.is_empty() {
        writeln!(out, "  (none)")?;
    }

    writeln!(
        out,
        "\n== unreferenced, but cgg treats it as a root ({}) ==",
        explained.len()
    )?;
    for f in &explained {
        writeln!(
            out,
            "  {:<52} {}:{}  [{}]",
            f.qualified_name,
            f.path,
            f.start_line,
            f.root.as_deref().unwrap_or("")
        )?;
    }
    if explained.is_empty() {
        writeln!(out, "  (none)")?;
    }
    Ok(())
}
