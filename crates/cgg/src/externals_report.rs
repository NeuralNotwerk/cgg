//! Render the `--externals` inventory.
//!
//! Text is ranked and capped for a reader with a context window; JSON is
//! the whole `cgg.externals.v1` document, every site included.

use std::io::{self, Write};

use cgg_core::externals::{ExternalKind, ExternalsReport};

/// How many entries each text section lists before "… N more".
const SYMBOLS: usize = 8;
const CALLERS: usize = 5;
const IMPORTS: usize = 3;
const UNATTRIBUTED: usize = 12;

fn wrap(text: &str, indent: &str, width: usize, out: &mut dyn Write) -> io::Result<()> {
    let mut line = String::new();
    for w in text.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + w.len() > width {
            writeln!(out, "{indent}{line}")?;
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(w);
    }
    if !line.is_empty() {
        writeln!(out, "{indent}{line}")?;
    }
    Ok(())
}

fn n(v: u64) -> String {
    let s = v.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn rule(title: &str, out: &mut dyn Write) -> io::Result<()> {
    let pad = 72usize.saturating_sub(title.len() + 4);
    writeln!(out, "\n── {title} {}", "─".repeat(pad))
}

/// The JSON document.
pub fn render_json(report: &ExternalsReport, out: &mut dyn Write) -> io::Result<()> {
    serde_json::to_writer_pretty(&mut *out, report).map_err(io::Error::other)?;
    writeln!(out)
}

/// The ranked text report.
pub fn render_text(report: &ExternalsReport, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "cgg externals report — {}", report.schema)?;
    writeln!(out)?;
    wrap(&report.disclaimer, "  ", 72, out)?;
    writeln!(out)?;
    let t = &report.totals;
    let shown: Vec<&str> = report.kinds.iter().map(|k| k.slug()).collect();
    let withheld: Vec<String> = t
        .withheld
        .iter()
        .map(|(k, v)| format!("{k} {}", n(*v)))
        .collect();
    write!(out, "kinds      {}", shown.join(", "))?;
    if !withheld.is_empty() {
        write!(out, "  (withheld: {})", withheld.join(" · "))?;
    }
    writeln!(out)?;
    let libs: Vec<String> = ExternalKind::ALL
        .iter()
        .filter_map(|k| {
            t.libraries_by_kind
                .get(k.slug())
                .map(|v| format!("{} {}", n(*v), k.slug()))
        })
        .collect();
    writeln!(
        out,
        "libraries  {}",
        if libs.is_empty() {
            "none".to_string()
        } else {
            libs.join(" · ")
        }
    )?;
    let shown_sites: u64 = t.by_confidence.values().sum();
    let conf: Vec<String> = ["high", "medium", "low"]
        .iter()
        .filter_map(|c| t.by_confidence.get(*c).map(|v| format!("{c} {}", n(*v))))
        .collect();
    writeln!(
        out,
        "sites      {} shown of {} that leave the tree ({})",
        n(shown_sites),
        n(t.sites),
        if conf.is_empty() {
            "none".into()
        } else {
            conf.join(" · ")
        }
    )?;

    rule("coverage — what cgg could attribute, per language", out)?;
    writeln!(
        out,
        "  {:<12} {:>6} {:>8} {:>8} {:>10}  forms",
        "language", "files", "imports", "sites", "attributed"
    )?;
    for c in &report.coverage {
        let pct = if c.sites == 0 {
            "—".to_string()
        } else {
            format!("{:.0}%", 100.0 * c.attributed as f64 / c.sites as f64)
        };
        let mut forms = c.forms.join(", ");
        if forms.len() > 48 {
            forms.truncate(45);
            forms.push('…');
        }
        writeln!(
            out,
            "  {:<12} {:>6} {:>8} {:>8} {:>10}  {}",
            c.language,
            n(c.files),
            n(c.imports),
            n(c.sites),
            pct,
            forms
        )?;
    }

    for kind in ExternalKind::ALL {
        if kind == ExternalKind::Unattributed || !report.kinds.contains(&kind) {
            continue;
        }
        let libs: Vec<_> = report.libraries.iter().filter(|l| l.kind == kind).collect();
        if libs.is_empty() {
            continue;
        }
        rule(kind.slug(), out)?;
        for l in libs {
            let callers = l.callers.len() as u64;
            writeln!(
                out,
                "{}  {} · {} import site(s) · {} call site(s) · {} caller(s)",
                l.name,
                l.language,
                n(l.imports.len() as u64),
                n(l.sites.len() as u64),
                n(callers)
            )?;
            if l.modules.len() > 1 || l.modules.first().is_some_and(|m| *m != l.name) {
                let mut ms: Vec<&str> =
                    l.modules.iter().take(4).map(String::as_str).collect();
                let more = l.modules.len().saturating_sub(4);
                let tail = if more > 0 {
                    format!(" … {more} more")
                } else {
                    String::new()
                };
                ms.dedup();
                writeln!(out, "  modules  {}{tail}", ms.join(", "))?;
            }
            if l.sites.is_empty() {
                for i in l.imports.iter().take(IMPORTS) {
                    writeln!(out, "  imported {}:{}", i.path, i.line)?;
                }
                if l.imports.len() > IMPORTS {
                    writeln!(
                        out,
                        "  … {} more import site(s)",
                        l.imports.len() - IMPORTS
                    )?;
                }
                continue;
            }
            let syms: Vec<String> = l
                .symbols
                .iter()
                .take(SYMBOLS)
                .map(|s| format!("{} ×{}", s.symbol, n(s.sites)))
                .collect();
            let more = l.symbols.len().saturating_sub(SYMBOLS);
            write!(out, "  symbols  {}", syms.join(" · "))?;
            if more > 0 {
                write!(out, " · … {more} more")?;
            }
            writeln!(out)?;
            for (i, c) in l.callers.iter().take(CALLERS).enumerate() {
                let label = if i == 0 { "callers " } else { "        " };
                writeln!(
                    out,
                    "  {label} {} ({}:{}) ×{}",
                    c.qualified_name,
                    c.path,
                    c.line,
                    n(c.sites)
                )?;
            }
            if l.callers.len() > CALLERS {
                writeln!(
                    out,
                    "           … {} more caller(s)",
                    l.callers.len() - CALLERS
                )?;
            }
            for b in l.branches.iter().take(2) {
                writeln!(out, "  branch   {}", b.join(" → "))?;
            }
        }
    }

    if report.kinds.contains(&ExternalKind::Unattributed)
        && !report.unattributed.is_empty()
    {
        rule(
            "unattributed — left the tree, no evidence names the library",
            out,
        )?;
        for g in report.unattributed.iter().take(UNATTRIBUTED) {
            let at = g
                .sample
                .first()
                .map(|s| format!(" (e.g. {}:{})", s.path, s.line))
                .unwrap_or_default();
            writeln!(out, "  {:<10} {} ×{}{at}", g.language, g.name, n(g.sites))?;
        }
        if report.unattributed.len() > UNATTRIBUTED {
            writeln!(
                out,
                "  … {} more name(s)",
                report.unattributed.len() - UNATTRIBUTED
            )?;
        }
    }

    rule("next", out)?;
    if let Some(l) = report.libraries.iter().find(|l| !l.sites.is_empty()) {
        writeln!(
            out,
            "  branches into one library  cgg <path> --externals --filter '^{}::{}::' -n 0",
            l.kind.sentinel(),
            regex_escape(&l.name)
        )?;
    }
    writeln!(
        out,
        "  module → library map       cgg <path> --externals --rollup-by module"
    )?;
    writeln!(
        out,
        "  every kind, as json        cgg <path> --externals=all --externals-format json --no-graph"
    )?;
    Ok(())
}

fn regex_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if "\\.+*?()|[]{}^$".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
