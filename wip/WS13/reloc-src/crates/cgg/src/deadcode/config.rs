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
    /// Local framework rules, appended to the built-in table.
    ///
    /// The coverage table names every framework cgg detected but has no
    /// rules for; this is what makes that list actionable rather than
    /// merely informative. A user hitting "django — imports found,
    /// entries NOT enumerated" can add the rule here and get coverage
    /// today instead of waiting for a release.
    #[serde(default, rename = "framework", skip_serializing_if = "Vec::is_empty")]
    pub frameworks: Vec<cgg_core::frameworks::FrameworkRule>,
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

    /// Discover the config for a run, searching upward from each
    /// **analyzed path** before falling back to the working directory.
    ///
    /// Searching only the working directory means `cgg /path/to/project`
    /// run from anywhere else silently ignores that project's config —
    /// the same silent-no-op family as a regex that matches nothing. A
    /// project's rules belong to the project, not to wherever the shell
    /// happened to be.
    pub fn discover_for(paths: &[PathBuf], cwd: Option<&Path>) -> Option<PathBuf> {
        for p in paths {
            let start = if p.is_dir() { p.as_path() } else { p.parent()? };
            let abs = start.canonicalize();
            let start = abs.as_deref().unwrap_or(start);
            if let Some(found) = Self::discover(start) {
                return Some(found);
            }
        }
        cwd.and_then(Self::discover)
    }

    /// Every pattern in the file, for stale-entry reporting.
    ///
    /// Exercised by the unit tests but not by the binary's own code
    /// path, and `pub` cannot be reached from outside a binary crate —
    /// so `dead_code` fires in the non-test build only. Kept as part of
    /// the config API surface rather than deleted.
    #[allow(dead_code)]
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
            one_line(&f.path.display().to_string()),
            f.start_line,
            toml_basic_escape(&regex::escape(&f.qualified_name)),
            f.category.code(),
            f.category.slug(),
        ));
    }
    out
}

/// Escape `s` for embedding in a TOML **basic** (double-quoted) string.
///
/// [`render_baseline`] writes each finding's pattern as
/// `name   = "^<regex>$"`, and `regex::escape` emits a backslash before
/// every metacharacter — so a qualified name carrying a `.`, which is
/// every Python, Ruby, or otherwise dot-joined name, produced `\.` inside
/// a basic string. TOML only recognises a fixed set of escapes there
/// (`b f n r t u U \ "`), so `\.` is a hard parse error and the generated
/// baseline could not be loaded back:
///
/// ```text
/// name   = "^cgg\._cgg\.Graph\.callable$"
///                 ^ invalid escape sequence
/// ```
///
/// A TOML *literal* string (`'…'`) would sidestep escaping entirely but
/// cannot represent a name containing an apostrophe, and Rust qualified
/// names carry them routinely in lifetimes (`ExtractCtx<'a>::new`). So the
/// basic string stays and its own escapes are applied on top of the regex
/// escaping, innermost first.
fn toml_basic_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Remaining C0 controls have no literal form in a basic
            // string and must go out as `\u00XX`.
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\u{:04X}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

/// Collapse anything that would end the line early, for text going into a
/// generated `#` comment. A TOML comment runs to the end of the line, so a
/// path containing a newline would push the rest of the comment out into
/// the document as if it were syntax.
fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
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
        assert!(
            format!("{err:#}").contains("rootz")
                || format!("{err:#}").contains("unknown")
        );

        let err = DeadCodeConfigFile::parse("[[allow]]\nnaem = \"x\"").unwrap_err();
        assert!(!format!("{err:#}").is_empty());
    }

    #[test]
    fn an_empty_file_is_valid() {
        let c = DeadCodeConfigFile::parse("").unwrap();
        assert!(c.roots.is_empty() && c.allow.is_empty());
        assert!(c.all_patterns().is_empty());
    }

    /// A finding with `qn` as its qualified name; every other field is
    /// filler, since `render_baseline` only reads the name, path, line
    /// and category.
    fn finding(qn: &str) -> cgg_core::deadcode::DeadCodeFinding {
        cgg_core::deadcode::DeadCodeFinding {
            id: cgg_core::ids::CallableId::new(0),
            qualified_name: qn.into(),
            simple_name: qn.into(),
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
        }
    }

    /// Render one finding and read its single pattern back out.
    fn round_trip(qn: &str) -> String {
        let mut r = cgg_core::deadcode::DeadCodeReport::default();
        r.findings.push(finding(qn));
        let text = render_baseline(&r);
        let cfg = DeadCodeConfigFile::parse(&text)
            .unwrap_or_else(|e| panic!("baseline naming `{qn}` must parse: {e}"));
        let mut pats = cfg.all_patterns();
        assert_eq!(pats.len(), 1, "exactly one allow entry");
        pats.pop().unwrap()
    }

    #[test]
    fn baseline_uses_allow_never_roots() {
        // Accepting a finding must not confer liveness on its callees.
        let mut r = cgg_core::deadcode::DeadCodeReport::default();
        r.findings.push(finding("a::b"));
        let text = render_baseline(&r);
        assert!(text.contains("[[allow]]"));
        assert!(text.contains(r#"name   = "^a::b$""#));
        assert_eq!(text.matches("roots = [\n]").count(), 1, "roots stays empty");
    }

    /// `regex::escape` puts a backslash before every metacharacter, and a
    /// TOML basic string accepts only a fixed escape set — so a DOTTED
    /// qualified name rendered `\.`, which is not one of them, and the
    /// generated file could not be loaded back at all:
    ///
    /// ```text
    /// name   = "^cgg\._cgg\.Graph\.callable$"
    ///                 ^ invalid escape sequence
    /// ```
    ///
    /// That is every Python callable, so `--write-roots` produced an
    /// unusable baseline for any tree containing Python.
    #[test]
    fn a_dotted_name_round_trips_and_its_dots_stay_literal() {
        let pat = round_trip("cgg._cgg.Graph.callable");
        let re = regex::Regex::new(&pat).expect("pattern must compile");
        assert!(
            re.is_match("cgg._cgg.Graph.callable"),
            "pattern {pat} should match the name it was generated from"
        );
        assert!(
            !re.is_match("cggX_cggXGraphXcallable"),
            "the `.` must stay a literal dot, not decay into a wildcard"
        );
    }

    /// An apostrophe is why this cannot simply switch to a TOML *literal*
    /// string: Rust lifetimes put one straight into the qualified name.
    #[test]
    fn a_name_carrying_a_lifetime_round_trips() {
        let qn = "cgg_lang::ExtractCtx<'a>::new";
        let pat = round_trip(qn);
        assert!(
            regex::Regex::new(&pat).expect("must compile").is_match(qn),
            "pattern {pat} should match {qn}"
        );
    }

    /// A backslash or a quote in the name must survive both escaping
    /// layers rather than closing the string early.
    #[test]
    fn a_name_with_a_quote_or_backslash_round_trips() {
        for qn in [r#"weird::says"hi""#, r"weird::back\slash"] {
            let pat = round_trip(qn);
            assert!(
                regex::Regex::new(&pat).expect("must compile").is_match(qn),
                "pattern {pat} should match {qn}"
            );
        }
    }

    #[test]
    fn baseline_round_trips_and_has_no_timestamp() {
        // Deliberately NOT the default (empty) report. An empty one emits
        // no `[[allow]]` line at all, so this test passed for the whole
        // time `--write-roots` was writing files that could not be read
        // back — the findings are what exercise the escaping.
        let mut r = cgg_core::deadcode::DeadCodeReport::default();
        r.findings.push(finding("cgg._cgg.Graph.callable"));
        r.findings.push(finding("a::b"));
        let a = render_baseline(&r);
        let b = render_baseline(&r);
        assert_eq!(a, b, "determinism: no generated date");
        DeadCodeConfigFile::parse(&a).expect("generated baseline must parse");
    }

    #[test]
    fn toml_basic_escape_covers_the_basic_string_escape_set() {
        assert_eq!(toml_basic_escape(r"a\.b"), r"a\\.b");
        assert_eq!(toml_basic_escape("say \"hi\""), r#"say \"hi\""#);
        assert_eq!(toml_basic_escape("a\nb"), r"a\nb");
        assert_eq!(toml_basic_escape("a\tb"), r"a\tb");
        assert_eq!(toml_basic_escape("a\u{1}b"), "a\\u0001b");
        assert_eq!(toml_basic_escape("plain"), "plain");
    }

    #[test]
    fn a_newline_in_a_path_cannot_break_out_of_its_comment() {
        let mut r = cgg_core::deadcode::DeadCodeReport::default();
        let mut f = finding("a::b");
        f.path = PathBuf::from("weird\nroots = [\"*\"]\n.rs");
        r.findings.push(f);
        let text = render_baseline(&r);
        let cfg = DeadCodeConfigFile::parse(&text).expect("must still parse");
        assert!(cfg.roots.is_empty(), "the path must not inject a root");
    }
}
