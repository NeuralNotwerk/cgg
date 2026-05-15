//! Command-line interface.
//!
//! The flag surface here is the frozen contract from the design:
//!
//! ```text
//! cgg <paths>... [-o FILE] [-t mermaid|json|dot|graphml]
//!               [--filter PATTERN]... [-n N]
//!               [--max-paths N]
//!               [--include-tests] [--ignore-file PATH]
//!               [--jobs N] [--cache DIR] [--no-cache]
//!               [--lang rust,python,...]
//!               [--audit-format json|jsonl] [--metrics FILE]
//!               [-v|-vv|-q]
//! ```

use clap::{ArgAction, Parser, ValueEnum};
use std::path::PathBuf;

/// `cgg` — offline call-graph generator.
///
/// Point it at one or more source folders, pick a format with `-t`,
/// optionally narrow the view with `--filter` + `-n`.
#[derive(Debug, Parser)]
#[command(
    name = "cgg",
    version,
    about = "Call graph generator — point at folders, get a graph.",
    long_about = None,
    arg_required_else_help = true,
)]
pub struct Cli {
    /// One or more source directories or files to analyze.
    #[arg(value_name = "PATH", required = true, num_args = 1..)]
    pub paths: Vec<PathBuf>,

    /// Write output to FILE instead of stdout. Use `-` for stdout.
    #[arg(short = 'o', long = "output", value_name = "FILE")]
    pub output: Option<PathBuf>,

    /// Output format.
    #[arg(short = 't', long = "type", value_enum, default_value_t = OutputFormatArg::Mermaid)]
    pub format: OutputFormatArg,

    /// Filter callables by pattern. Repeatable. Regex by default; prefix
    /// with `glob:` to use glob syntax. Matched against fully-qualified
    /// names.
    #[arg(long = "filter", value_name = "PATTERN")]
    pub filter: Vec<String>,

    /// Seed `--filter` from the functions touched by a git revspec.
    /// Anything `git diff` accepts works: `HEAD~5`, `main..HEAD`,
    /// `abc123..def456`, `main...feature`. The resolved seeds are
    /// *added* to any explicit `--filter` patterns — they do not
    /// replace them.
    ///
    /// A bare ref (e.g. `HEAD~5`) is interpreted by git as
    /// "<ref> vs working tree", which includes uncommitted edits. Use
    /// `HEAD~5..HEAD` if you want committed changes only.
    ///
    /// Requires `git` on PATH and the analysis path to be inside a
    /// repository.
    #[arg(long = "since", value_name = "REVSPEC")]
    pub since: Option<String>,

    /// Exclude callables whose qualified name contains SUBSTRING.
    /// Repeatable. Applied after --filter.
    #[arg(long = "exclude-partial", value_name = "SUBSTRING")]
    pub exclude_partial: Vec<String>,

    /// Exclude callables whose qualified name matches a glob pattern.
    /// Repeatable. Applied after --filter.
    #[arg(long = "exclude-glob", value_name = "PATTERN")]
    pub exclude_glob: Vec<String>,

    /// Exclude callables whose qualified name matches a regex.
    /// Repeatable. Applied after --filter.
    #[arg(long = "exclude-regex", value_name = "PATTERN")]
    pub exclude_regex: Vec<String>,

    /// Neighborhood depth around each `--filter` match. `-n 0` enumerates
    /// full entry-to-exit call paths passing through matches.
    #[arg(short = 'n', long = "hops", value_name = "N", default_value_t = -1)]
    pub hops: i32,

    /// Cap per-match path count in `-n 0` mode. Overflow is recorded in
    /// the audit log.
    #[arg(long = "max-paths", value_name = "N", default_value_t = 1000)]
    pub max_paths: u32,

    /// Include `tests/`, `__tests__/`, `*_test.go`, etc. By default
    /// test-looking files are analyzed but tagged — this flag is a
    /// reserved future knob; honored as a no-op in v1.
    #[arg(long = "include-tests", action = ArgAction::SetTrue)]
    pub include_tests: bool,

    /// Path to an additional ignore file (gitignore syntax).
    #[arg(long = "ignore-file", value_name = "PATH")]
    pub ignore_file: Option<PathBuf>,

    /// Number of parallel jobs. `0` means "auto" (rayon default).
    #[arg(long = "jobs", value_name = "N", default_value_t = 0)]
    pub jobs: usize,

    /// Cache directory. Default: `./.cgg-cache`.
    #[arg(long = "cache", value_name = "DIR")]
    pub cache: Option<PathBuf>,

    /// Disable reading and writing the on-disk cache.
    #[arg(long = "no-cache", action = ArgAction::SetTrue)]
    pub no_cache: bool,

    /// Restrict analysis to the given comma-separated language ids.
    /// Example: `--lang rust,python`.
    #[arg(long = "lang", value_name = "LIST", value_delimiter = ',')]
    pub lang: Vec<String>,

    /// Shape of the audit output. `json` = batched doc; `jsonl` =
    /// streamed events (one per line, SIEM-friendly).
    #[arg(long = "audit-format", value_enum, default_value_t = AuditFormatArg::Json)]
    pub audit_format: AuditFormatArg,

    /// Control stack-graphs deep resolution. `auto` (default) runs it
    /// with a 60-second timeout — if exceeded, falls back to the
    /// cross-file resolver only. `on` forces it without timeout; `off`
    /// skips it entirely.
    #[arg(long = "stack-graphs", value_enum, default_value_t = StackGraphsArg::Auto)]
    pub stack_graphs: StackGraphsArg,

    /// Force a sidecar metrics file. Useful when `-t json` already
    /// embeds the audit but an external tool wants a split file.
    #[arg(long = "metrics", value_name = "FILE")]
    pub metrics: Option<PathBuf>,

    /// Increase verbosity. Repeat: `-v`, `-vv`.
    #[arg(short = 'v', long = "verbose", action = ArgAction::Count)]
    pub verbose: u8,

    /// Silence everything except errors.
    #[arg(short = 'q', long = "quiet", action = ArgAction::SetTrue)]
    pub quiet: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum OutputFormatArg {
    Mermaid,
    Json,
    Dot,
    Graphml,
}

impl From<OutputFormatArg> for cgg_format::OutputFormat {
    fn from(v: OutputFormatArg) -> Self {
        match v {
            OutputFormatArg::Mermaid => cgg_format::OutputFormat::Mermaid,
            OutputFormatArg::Json => cgg_format::OutputFormat::Json,
            OutputFormatArg::Dot => cgg_format::OutputFormat::Dot,
            OutputFormatArg::Graphml => cgg_format::OutputFormat::Graphml,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum AuditFormatArg {
    /// Single JSON document (pretty).
    Json,
    /// One JSON object per line.
    Jsonl,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum StackGraphsArg {
    /// Run with 60-second timeout; fall back if exceeded.
    Auto,
    /// Always run (no timeout).
    On,
    /// Skip entirely.
    Off,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn help_renders() {
        // Ensures clap doesn't panic on help text assembly and keeps the
        // command name stable. Snapshot-style comparison handled by a
        // separate integration test.
        let cmd = Cli::command();
        let name = cmd.get_name().to_string();
        assert_eq!(name, "cgg");
    }

    #[test]
    fn requires_path() {
        // No paths -> should fail parsing.
        let res = Cli::try_parse_from(["cgg"]);
        assert!(res.is_err());
    }

    #[test]
    fn parses_full_surface() {
        let cli = Cli::try_parse_from([
            "cgg",
            "./a",
            "./b",
            "-o",
            "out.json",
            "-t",
            "json",
            "--filter",
            "foo",
            "--filter",
            "glob:bar_*",
            "--exclude-partial",
            "tests::",
            "--exclude-glob",
            "*::internal::*",
            "--exclude-regex",
            "^test_.*",
            "-n",
            "2",
            "--max-paths",
            "50",
            "--jobs",
            "4",
            "--cache",
            ".cache",
            "--lang",
            "rust,python",
            "--audit-format",
            "jsonl",
            "-vv",
        ])
        .expect("should parse");
        assert_eq!(cli.paths.len(), 2);
        assert_eq!(cli.filter.len(), 2);
        assert_eq!(cli.exclude_partial, vec!["tests::".to_string()]);
        assert_eq!(cli.exclude_glob, vec!["*::internal::*".to_string()]);
        assert_eq!(cli.exclude_regex, vec!["^test_.*".to_string()]);
        assert_eq!(cli.hops, 2);
        assert_eq!(cli.max_paths, 50);
        assert_eq!(cli.jobs, 4);
        assert_eq!(cli.lang, vec!["rust".to_string(), "python".to_string()]);
        assert!(matches!(cli.format, OutputFormatArg::Json));
        assert!(matches!(cli.audit_format, AuditFormatArg::Jsonl));
        assert_eq!(cli.verbose, 2);
    }

    #[test]
    fn default_hops_is_sentinel_minus_one() {
        let cli = Cli::try_parse_from(["cgg", "./a"]).unwrap();
        // -1 means "no hop limit / no filtering active" — Task 10 reads
        // this as "emit the full graph".
        assert_eq!(cli.hops, -1);
    }
}
