//! `cgg-deadcode.toml` — declared roots and accepted findings.
//!
//! The file has two sections with deliberately different semantics, and
//! the distinction is the most important thing in it:
//!
//! * **`roots`** declare entry points. A match is live, and everything
//!   it transitively calls becomes live too. This is the only mechanism
//!   that can change what the analysis *concludes*.
//! * **`allow`** records findings that have been reviewed and accepted.
//!   Matches are filtered out of the report but are **not** made live,
//!   so their callees keep being reported on their own merits.
//!
//! Name-matching tools cannot express this split: their whitelist is a
//! list of names that count as used, so accepting one entry necessarily
//! silences everything it calls. Keeping the two apart means an accepted
//! finding hides itself and nothing else.
//!
//! Parsed with `deny_unknown_fields`, so a typo is a hard error rather
//! than a silently ignored line — a suppression file that quietly stops
//! working is worse than no suppression file.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// The default file name, discovered by walking up from the working
/// directory.
pub const CONFIG_NAME: &str = "cgg-deadcode.toml";

/// A reviewed and accepted finding.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AllowEntry {
    /// Pattern matched against the qualified name. Regex by default;
    /// `glob:` prefix for glob syntax.
    pub name: String,
    /// Why it was accepted. Free text, for the next reader.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeadCodeConfigFile {
    /// Entry points. A match is live, and so is everything it reaches.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots: Vec<String>,
    /// Attribute/decorator markers whose bearers are entry points
    /// (`#[no_mangle]`, `glob:@app.route*`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub root_attributes: Vec<String>,
    /// Accepted findings. Suppressed from the report, **not** made live.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<AllowEntry>,
}

impl DeadCodeConfigFile {
    pub fn parse(text: &str) -> Result<Self> {
        toml::from_str(text).context("parsing dead-code configuration")
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("in {}", path.display()))
    }

    /// Search upward from `start` for [`CONFIG_NAME`].
    pub fn discover(start: &Path) -> Option<PathBuf> {
        let mut dir = Some(start);
        while let Some(d) = dir {
            let candidate = d.join(CONFIG_NAME);
            if candidate.is_file() {
                return Some(candidate);
            }
            dir = d.parent();
        }
        None
    }

    /// Every pattern in the file, for stale-entry reporting.
    pub fn all_patterns(&self) -> Vec<String> {
        self.roots
            .iter()
            .cloned()
            .chain(self.allow.iter().map(|a| a.name.clone()))
            .collect()
    }
}

/// Render a baseline that accepts every finding in `report`.
///
/// Deliberately emits `allow` entries and never `roots`: accepting a
/// finding must not silence the callables it references. Deliberately
/// carries no timestamp, because determinism is a product promise and a
/// generated date would make the output differ run to run — `git blame`
/// already knows when it was written.
pub fn render_baseline(report: &cgg_core::deadcode::DeadCodeReport) -> String {
    let mut out = String::new();
    out.push_str(
        "# cgg dead-code configuration.\n\
         #\n\
         # `roots` entries are entry points: a match is live, and so is\n\
         # everything it transitively calls.\n\
         #\n\
         # `[[allow]]` entries are findings that have been reviewed and\n\
         # accepted. They are suppressed from the report but are NOT made\n\
         # live, so anything they reference is still reported on its own\n\
         # merits.\n\
         #\n\
         # Patterns use --filter syntax: regex by default, `glob:` prefix\n\
         # for glob.\n\n",
    );
    out.push_str("roots = [\n]\n\nroot_attributes = [\n]\n\n");
    out.push_str(&format!(
        "# {} finding(s) accepted from a cgg {} run.\n",
        report.findings.len(),
        report.cgg_version
    ));
    for f in &report.findings {
        out.push_str(&format!(
            "\n# {}:{}\n[[allow]]\nname   = \"^{}$\"\nreason = \"baseline — {} {}\"\n",
            f.path.display(),
            f.start_line,
            regex::escape(&f.qualified_name),
            f.category.code(),
            f.category.slug(),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_roots_and_allow() {
        let c = DeadCodeConfigFile::parse(
            r##"
            roots = ["^crate::main$", "glob:*::handlers::*"]
            root_attributes = ["#[no_mangle]"]
            [[allow]]
            name = "^crate::api$"
            reason = "public surface"
            "##,
        )
        .unwrap();
        assert_eq!(c.roots.len(), 2);
        assert_eq!(c.root_attributes.len(), 1);
        assert_eq!(c.allow.len(), 1);
        assert_eq!(c.allow[0].reason, "public surface");
    }

    #[test]
    fn a_typo_is_a_hard_error_not_a_silent_no_op() {
        // A suppression file that quietly stops working is the worst
        // possible failure mode, so unknown keys must not be ignored.
        let err = DeadCodeConfigFile::parse("rootz = [\"x\"]").unwrap_err();
        assert!(format!("{err:#}").contains("rootz") || format!("{err:#}").contains("unknown"));

        let err = DeadCodeConfigFile::parse("[[allow]]\nnaem = \"x\"").unwrap_err();
        assert!(!format!("{err:#}").is_empty());
    }

    #[test]
    fn an_empty_file_is_valid() {
        let c = DeadCodeConfigFile::parse("").unwrap();
        assert!(c.roots.is_empty() && c.allow.is_empty());
        assert!(c.all_patterns().is_empty());
    }

    #[test]
    fn baseline_uses_allow_never_roots() {
        // Accepting a finding must not confer liveness on its callees.
        let mut r = cgg_core::deadcode::DeadCodeReport::default();
        r.findings.push(cgg_core::deadcode::DeadCodeFinding {
            id: cgg_core::ids::CallableId::new(0),
            qualified_name: "a::b".into(),
            simple_name: "b".into(),
            language: "rust".into(),
            kind: cgg_core::graph::CallableKind::Function,
            def_variant: String::new(),
            file: cgg_core::ids::FileId::new(0),
            path: PathBuf::from("a.rs"),
            start_line: 1,
            end_line: 2,
            size_lines: 2,
            signature_hint: String::new(),
            visibility: String::new(),
            category: cgg_core::deadcode::FindingCategory::NeverReferenced,
            confidence: cgg_core::graph::Confidence::High,
            rank: 0,
            region: 0,
            role: cgg_core::deadcode::RegionRole::Anchor,
            evidence: vec![],
            dead_callers: vec![],
            out_degree: 0,
        });
        let text = render_baseline(&r);
        assert!(text.contains("[[allow]]"));
        assert!(text.contains(r#"name   = "^a::b$""#));
        assert_eq!(text.matches("roots = [\n]").count(), 1, "roots stays empty");
    }

    #[test]
    fn baseline_round_trips_and_has_no_timestamp() {
        let r = cgg_core::deadcode::DeadCodeReport::default();
        let a = render_baseline(&r);
        let b = render_baseline(&r);
        assert_eq!(a, b, "determinism: no generated date");
        DeadCodeConfigFile::parse(&a).expect("generated baseline must parse");
    }
}
