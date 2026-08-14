//! Rendering for the dead-code report.
//!
//! Two shapes: a ranked text report (the default) and the stable
//! `cgg.deadcode.v1` JSON document. Both are byte-stable for a given
//! graph, and both carry the disclaimer — it is copied from a `cgg-core`
//! constant rather than composed here, so no rendering path can drop it.
//!
//! The text layout is built for cgg's primary consumer, an agent reading
//! output in a context window. That drives two choices that differ from
//! terminal-oriented tools: the disclaimer is repeated at the foot as
//! well as the head, because long output gets truncated from the middle;
//! and findings sort by confidence then path, never by size, because the
//! *top* of the buffer is the high-attention region and scroll position
//! carries no meaning.

use std::io::{self, Write};

use cgg_core::deadcode::{
    DeadCodeReport, LanguageClass, LivenessProof, SignalSupport, SuppressionReason,
};
use cgg_core::graph::Confidence;

/// Order of the confidence bands, strongest first.
const BANDS: [Confidence; 3] = [Confidence::High, Confidence::Medium, Confidence::Low];

fn band_name(c: Confidence) -> &'static str {
    match c {
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
    }
}

fn band_rank(c: Confidence) -> u8 {
    match c {
        Confidence::High => 0,
        Confidence::Medium => 1,
        Confidence::Low => 2,
    }
}

fn support(s: SignalSupport) -> &'static str {
    match s {
        SignalSupport::Full => "yes",
        SignalSupport::Convention => "conv",
        SignalSupport::None => "no",
    }
}

fn class_name(c: LanguageClass) -> &'static str {
    match c {
        LanguageClass::Analyzable => "analyzable",
        LanguageClass::Degraded => "degraded",
        LanguageClass::ScriptDriven => "script-driven",
        LanguageClass::Descriptor => "descriptor",
    }
}

fn suppression_reason(r: SuppressionReason) -> &'static str {
    match r {
        SuppressionReason::DescriptorLanguage => "descriptor language",
        SuppressionReason::ScriptDriven => "script-driven language",
        SuppressionReason::NoRootsFound => "no roots found",
        SuppressionReason::LowRootCoverage => "root coverage too low",
        SuppressionReason::MissingSignal => "signal not extracted",
    }
}

/// Wrap the disclaimer to `width`, prefixing each line with `indent`.
fn wrap(text: &str, width: usize, indent: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > width {
            lines.push(format!("{indent}{cur}"));
            cur.clear();
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        lines.push(format!("{indent}{cur}"));
    }
    lines
}

/// Render the text report.
pub fn render_text(
    r: &DeadCodeReport,
    threshold: Confidence,
    out: &mut dyn Write,
) -> io::Result<()> {
    writeln!(out, "cgg dead-code report — {}", r.schema)?;
    writeln!(out)?;
    for line in wrap(&r.disclaimer, 72, "  ") {
        writeln!(out, "{line}")?;
    }
    writeln!(out)?;

    // --- What was analyzed ------------------------------------------------
    let s = &r.summary;
    writeln!(
        out,
        "analyzed  {} callables, {} edges, {} unresolved call site(s)",
        s.callables, s.edges, s.unresolved_call_sites
    )?;
    writeln!(
        out,
        "roots     {} discovered · root-reachability {}",
        s.roots, r.config.root_reachability
    )?;
    writeln!(out)?;

    // --- What cgg could and could not see ---------------------------------
    writeln!(out, "signal coverage — a \"no\" is a place cgg is guessing")?;
    writeln!(
        out,
        "  {:<12} {:>5} {:>9} {:<13} {:>4} {:>5} {:>5} {:>5} {:>6}",
        "language",
        "files",
        "callables",
        "class",
        "vis",
        "attrs",
        "refs",
        "disp",
        "reach"
    )?;
    for c in &r.capabilities {
        writeln!(
            out,
            "  {:<12} {:>5} {:>9} {:<13} {:>4} {:>5} {:>5} {:>5} {:>5}%",
            c.language,
            c.files,
            c.callables,
            class_name(c.class),
            support(c.visibility),
            support(c.attributes),
            support(c.value_references),
            support(c.dispatch),
            c.reachable_pct,
        )?;
    }
    for c in &r.capabilities {
        if !c.blind_spots.is_empty() {
            writeln!(out, "  known blind spots for {}:", c.language)?;
            for b in &c.blind_spots {
                writeln!(out, "    - {b}")?;
            }
        }
    }
    writeln!(out)?;

    // --- Findings ---------------------------------------------------------
    let shown: Vec<_> = r
        .findings
        .iter()
        .filter(|f| band_rank(f.confidence) <= band_rank(threshold))
        .collect();
    let withheld_by_band = |b: Confidence| {
        r.findings
            .iter()
            .filter(|f| f.confidence == b && band_rank(b) > band_rank(threshold))
            .count()
    };

    let mut parts = vec![format!("{} shown ({})", shown.len(), band_name(threshold))];
    for b in BANDS {
        let n = withheld_by_band(b);
        if n > 0 {
            parts.push(format!("{n} withheld ({})", band_name(b)));
        }
    }
    writeln!(out, "findings  {}", parts.join(" · "))?;

    for b in BANDS {
        let in_band: Vec<_> = shown.iter().filter(|f| f.confidence == b).collect();
        if in_band.is_empty() {
            continue;
        }
        writeln!(out)?;
        writeln!(
            out,
            "── {} {}",
            band_name(b),
            "─".repeat(70 - band_name(b).len())
        )?;
        for f in in_band {
            writeln!(
                out,
                "{}:{}: unreferenced {} '{}' ({} line{}) [{}]",
                f.path.display(),
                f.start_line,
                format!("{:?}", f.kind).to_lowercase(),
                f.qualified_name,
                f.size_lines,
                if f.size_lines == 1 { "" } else { "s" },
                f.category.code(),
            )?;
            let why: Vec<&str> = f
                .evidence
                .iter()
                .filter(|e| e.polarity() != cgg_core::deadcode::Polarity::Lowers)
                .map(|e| e.slug())
                .collect();
            if !why.is_empty() {
                writeln!(out, "    why:  {}", why.join(", "))?;
            }
            let risk: Vec<&str> = f
                .evidence
                .iter()
                .filter(|e| e.polarity() == cgg_core::deadcode::Polarity::Lowers)
                .map(|e| e.slug())
                .collect();
            if !risk.is_empty() {
                writeln!(out, "    risk: {}", risk.join(", "))?;
            }
        }
    }

    // --- What was withheld, and why ---------------------------------------
    if !s.withheld.is_empty() {
        writeln!(out)?;
        writeln!(out, "── withheld {}", "─".repeat(60))?;
        for w in &s.withheld {
            writeln!(
                out,
                "  {:<12} {:<30} {:>5} withheld ({})",
                w.language,
                w.category.slug(),
                w.would_have_reported,
                suppression_reason(w.reason),
            )?;
        }
    }
    if !s.stale_suppressions.is_empty() {
        writeln!(out)?;
        writeln!(
            out,
            "note: {} declared root pattern(s) matched nothing — stale:",
            s.stale_suppressions.len()
        )?;
        for p in &s.stale_suppressions {
            writeln!(out, "    {p}")?;
        }
    }

    // --- Footer ------------------------------------------------------------
    writeln!(out)?;
    writeln!(out, "── next {}", "─".repeat(64))?;
    if let Some(f) = shown.first() {
        writeln!(
            out,
            "  check one   cgg <path> --why-live '{}$'",
            f.qualified_name
        )?;
    }
    writeln!(
        out,
        "  as json     cgg <path> --dead-code --dead-code-format json"
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "  Reminder: BEST EFFORT. Every finding above is a hypothesis that"
    )?;
    writeln!(
        out,
        "  cgg could not find a caller for — not proof that none exists."
    )?;
    writeln!(
        out,
        "  Verify each one against the source before acting on it."
    )?;
    Ok(())
}

/// Render `--why-live` proofs.
pub fn render_why_live(proofs: &[LivenessProof], out: &mut dyn Write) -> io::Result<()> {
    for p in proofs {
        writeln!(out, "{}", p.target_qualified_name)?;
        match p.status.as_str() {
            "live" | "test-live" => {
                // A zero-hop proof is definition-side liveness: the
                // callable *is* a root, not something a call path
                // reaches. Rendering both as a bare `LIVE` invited
                // over-reading — a function only ever *named* in a
                // module-level registry dict, and invoked reflectively
                // through it, proved "LIVE — 0 hop(s) from itself" and
                // read like a verified call path.
                let label = match (p.status.as_str(), p.hops.is_empty()) {
                    ("test-live", _) => "LIVE ONLY IN TESTS",
                    (_, true) => "LIVE (root itself — no call path)",
                    (_, false) => "LIVE (call path)",
                };
                let root = p
                    .root
                    .as_ref()
                    .map(|r| format!("{} [{:?}]", r.qualified_name, r.kind))
                    .unwrap_or_else(|| "<unknown root>".into());
                writeln!(
                    out,
                    "  {label} — proof: {} hop(s) from {root}",
                    p.hops.len()
                )?;
                for h in &p.hops {
                    writeln!(
                        out,
                        "   └→ {:<44} {}:{}  {} / {:?}",
                        h.to_qualified_name,
                        h.path.display(),
                        h.line,
                        h.via,
                        h.confidence,
                    )?;
                }
                if let Some(w) = p.weakest_link {
                    writeln!(out, "  weakest hop: {w:?}")?;
                }
            }
            _ => {
                writeln!(out, "  NOT REACHED — no path from any known root.")?;
                writeln!(
                    out,
                    "  This is the same claim the report makes, shown as a derivation."
                )?;
            }
        }
        writeln!(out)?;
    }
    writeln!(
        out,
        "BEST EFFORT: the absence of a proven path is not proof that no caller"
    )?;
    writeln!(
        out,
        "exists. See `cgg <path> --dead-code` for cgg's blind spots."
    )?;
    Ok(())
}

/// Render the stable JSON document.
pub fn render_json(r: &DeadCodeReport, out: &mut dyn Write) -> io::Result<()> {
    serde_json::to_writer_pretty(out, r).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::deadcode::DEAD_CODE_DISCLAIMER;

    fn render(r: &DeadCodeReport, t: Confidence) -> String {
        let mut buf = Vec::new();
        render_text(r, t, &mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn the_banner_leads_and_is_repeated_at_the_foot() {
        let s = render(&DeadCodeReport::default(), Confidence::High);
        // Agents truncate from the middle, so both ends must carry it.
        let head: String = s.lines().take(8).collect::<Vec<_>>().join(" ");
        assert!(head.contains("BEST EFFORT"), "missing at head:\n{s}");
        assert!(head.contains("HYPOTHESIS"));
        let tail: String = s.lines().rev().take(5).collect::<Vec<_>>().join(" ");
        assert!(tail.contains("BEST EFFORT"), "missing at foot:\n{s}");
    }

    #[test]
    fn the_disclaimer_text_is_reproduced_word_for_word() {
        let s = render(&DeadCodeReport::default(), Confidence::High);
        let flat: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
        let want: String = DEAD_CODE_DISCLAIMER
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(flat.contains(&want), "disclaimer was altered in rendering");
    }

    #[test]
    fn an_empty_report_still_prints_everything() {
        // Silence is indistinguishable from a crash.
        let s = render(&DeadCodeReport::default(), Confidence::High);
        assert!(s.contains("BEST EFFORT"));
        assert!(s.contains("analyzed"));
        assert!(s.contains("signal coverage"));
        assert!(s.contains("findings"));
        assert!(s.contains("0 shown"));
    }

    #[test]
    fn rendering_is_byte_stable() {
        let r = DeadCodeReport::default();
        assert_eq!(render(&r, Confidence::High), render(&r, Confidence::High));
    }

    #[test]
    fn json_carries_the_schema_and_disclaimer() {
        let mut buf = Vec::new();
        render_json(&DeadCodeReport::default(), &mut buf).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(v["schema"], "cgg.deadcode.v1");
        assert_eq!(v["best_effort"], true);
        assert_eq!(v["summary"]["review_required"], true);
        assert_eq!(v["disclaimer"].as_str().unwrap(), DEAD_CODE_DISCLAIMER);
    }

    #[test]
    fn wrap_keeps_every_word() {
        let text = "one two three four five six seven eight nine ten";
        let joined = wrap(text, 12, "  ").join(" ").replace("  ", " ");
        for w in text.split(' ') {
            assert!(joined.contains(w), "lost {w}");
        }
    }
}
