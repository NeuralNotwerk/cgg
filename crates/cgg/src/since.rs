//! `--since <revspec>` support.
//!
//! Shells out to `git diff <revspec> --unified=0 --no-color` and parses
//! the resulting unified diff to recover, per file, the set of line
//! ranges that were touched in the new revision. The orchestrator in
//! `main.rs` intersects those ranges against the callables cgg
//! extracted, turning each modified function into a seed for the
//! existing `--filter` machinery.
//!
//! Why shell out to `git` instead of vendoring a diff parser:
//!   - Rev-spec parsing (`HEAD~5`, `main..HEAD`, `abc..def`,
//!     `main...feature`) is exactly what `git diff` already does. We
//!     don't want to reimplement that.
//!   - Renames are followed by `git diff -M`, so we always read the
//!     post-rename path and post-rename line numbers — no manual
//!     bookkeeping needed.
//!
//! The parser is intentionally narrow. It recognises only the two diff
//! constructs we need: `+++ b/<path>` (new-side filename) and
//! `@@ -... +start[,count] @@` (new-side hunk range). Everything else
//! is skipped.
//!
//! This module is `pub(crate)` — the orchestration in `main.rs` calls
//! `resolve_since` once per run.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};

/// Per-file map of changed line ranges in the *new* (post-change) side
/// of the diff. Ranges are inclusive `[start, end]`, 1-based.
pub type ChangedRanges = BTreeMap<PathBuf, Vec<(u32, u32)>>;

/// Run `git diff <revspec> --unified=0 --no-color -M` and parse the
/// output. Paths in the returned map are absolute (canonicalised
/// against the git toplevel) so they can be matched against cgg's
/// stored file paths regardless of the user's CWD.
pub fn resolve_since(revspec: &str, cwd: &Path) -> Result<ChangedRanges> {
    let toplevel = git_toplevel(cwd).context(
        "`--since` requires a git repository — `git rev-parse --show-toplevel` failed",
    )?;

    let out = Command::new("git")
        .arg("-C")
        .arg(&toplevel)
        .arg("diff")
        .arg("--unified=0")
        .arg("--no-color")
        .arg("-M") // follow renames; new path appears in `+++ b/...`
        .arg(revspec)
        .output()
        .context("failed to invoke `git diff`")?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!(
            "`git diff {revspec}` failed (exit {}): {}",
            out.status.code().unwrap_or(-1),
            stderr.trim()
        );
    }

    let text =
        String::from_utf8(out.stdout).context("`git diff` output was not valid UTF-8")?;
    Ok(parse_diff(&text, &toplevel))
}

fn git_toplevel(cwd: &Path) -> Result<PathBuf> {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()
        .context("failed to invoke `git rev-parse`")?;
    if !out.status.success() {
        return Err(anyhow!("{}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let s = String::from_utf8(out.stdout)
        .context("`git rev-parse` output was not valid UTF-8")?;
    Ok(PathBuf::from(s.trim()))
}

/// Parse a unified-zero diff. Returns absolute paths (joined against
/// `repo_root`) and the new-side line ranges that changed in each.
///
/// Pure deletions (a hunk like `@@ -10,5 +9,0 @@` — five old lines
/// removed, zero new lines) intentionally produce *no* range. Such
/// changes can't anchor a current-source callable, so emitting a range
/// would just inflate the unmatched-seed count.
fn parse_diff(text: &str, repo_root: &Path) -> ChangedRanges {
    let mut out: ChangedRanges = BTreeMap::new();
    let mut current_path: Option<PathBuf> = None;

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("+++ ") {
            current_path = parse_new_side_path(rest, repo_root);
            continue;
        }
        if line.starts_with("@@") {
            let Some(path) = current_path.as_ref() else {
                continue;
            };
            if let Some(range) = parse_hunk_new_range(line) {
                out.entry(path.clone()).or_default().push(range);
            }
        }
    }
    out
}

/// `+++ b/path/to/file` → `<repo_root>/path/to/file`.
/// `+++ /dev/null` (file fully deleted) → `None`.
fn parse_new_side_path(field: &str, repo_root: &Path) -> Option<PathBuf> {
    let field = field.trim();
    if field == "/dev/null" {
        return None;
    }
    // Strip the `b/` (or `a/`) prefix git uses by default.
    let stripped = field
        .strip_prefix("b/")
        .or_else(|| field.strip_prefix("a/"))
        .unwrap_or(field);
    Some(repo_root.join(stripped))
}

/// `@@ -10,3 +20,5 @@ optional context` → `Some((20, 24))`.
/// `@@ -10,5 +9,0 @@`                      → `None` (pure deletion).
/// `@@ -10 +20 @@`                         → `Some((20, 20))` (single line each side).
fn parse_hunk_new_range(line: &str) -> Option<(u32, u32)> {
    // Find the `+` field between the two `@@` markers.
    let body = line.strip_prefix("@@")?.trim_start();
    let plus = body.split_whitespace().find(|tok| tok.starts_with('+'))?;
    let plus = plus.strip_prefix('+')?;
    let (start_str, count_str) = match plus.split_once(',') {
        Some((s, c)) => (s, c),
        None => (plus, "1"),
    };
    let start: u32 = start_str.parse().ok()?;
    let count: u32 = count_str.parse().ok()?;
    if count == 0 {
        // Pure deletion at this position.
        return None;
    }
    Some((start, start + count - 1))
}

/// Does `[s, e]` (callable span) overlap any range in `ranges`?
pub fn overlaps_any(s: u32, e: u32, ranges: &[(u32, u32)]) -> bool {
    ranges.iter().any(|&(rs, re)| s <= re && rs <= e)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from("/repo")
    }

    #[test]
    fn parses_single_hunk_with_count() {
        let diff = "\
diff --git a/src/a.rs b/src/a.rs
--- a/src/a.rs
+++ b/src/a.rs
@@ -10,3 +12,4 @@ fn ctx
 unchanged
+added
+added
";
        let r = parse_diff(diff, &root());
        assert_eq!(r[&PathBuf::from("/repo/src/a.rs")], vec![(12, 15)]);
    }

    #[test]
    fn parses_single_line_hunk_without_count() {
        // git diff --unified=0 emits `+20` (no comma) for single-line edits.
        let diff = "+++ b/src/a.rs\n@@ -10 +20 @@\n+x\n";
        let r = parse_diff(diff, &root());
        assert_eq!(r[&PathBuf::from("/repo/src/a.rs")], vec![(20, 20)]);
    }

    #[test]
    fn pure_deletion_drops_range() {
        // `+9,0` = zero new lines at this point. Should be skipped.
        let diff = "+++ b/src/a.rs\n@@ -10,5 +9,0 @@\n";
        let r = parse_diff(diff, &root());
        assert!(
            r.is_empty(),
            "deletion-only hunk should not produce a range"
        );
    }

    #[test]
    fn dev_null_new_side_is_ignored() {
        let diff = "+++ /dev/null\n@@ -1,5 +0,0 @@\n";
        let r = parse_diff(diff, &root());
        assert!(r.is_empty());
    }

    #[test]
    fn multiple_files_and_hunks() {
        let diff = "\
+++ b/src/a.rs
@@ -1 +1 @@
-old
+new
@@ -10,2 +10,3 @@
+added
+added
+added
+++ b/src/b.rs
@@ -5,1 +5,2 @@
+x
+y
";
        let r = parse_diff(diff, &root());
        assert_eq!(r[&PathBuf::from("/repo/src/a.rs")], vec![(1, 1), (10, 12)]);
        assert_eq!(r[&PathBuf::from("/repo/src/b.rs")], vec![(5, 6)]);
    }

    #[test]
    fn overlaps_any_inclusive_endpoints() {
        let r = vec![(10, 20), (30, 40)];
        assert!(overlaps_any(15, 16, &r));
        assert!(overlaps_any(20, 25, &r)); // touches at 20
        assert!(overlaps_any(5, 10, &r)); // touches at 10
        assert!(!overlaps_any(21, 29, &r));
    }

    #[test]
    fn rejects_malformed_hunk_header() {
        // Missing `+` field.
        assert_eq!(parse_hunk_new_range("@@ -10,3 @@"), None);
        // Non-numeric.
        assert_eq!(parse_hunk_new_range("@@ -10,3 +foo,4 @@"), None);
    }
}
