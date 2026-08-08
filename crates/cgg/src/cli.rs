//! Command-line interface.
//!
//! The flag surface here is the frozen contract from the design:
//!
//! ```text
//! cgg <paths>... [-o FILE] [-t mermaid|json|dot|graphml]
//!               [--filter PATTERN]... [-n N]
//!               [--max-paths N]
//!               [--include-tests] [--ignore-file PATH]
//!               [--jobs N] [--lang rust,python,...]
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

    /// Show dead-code findings that live in test scope.
    ///
    /// Test files are *always* walked, parsed and resolved, and a call
    /// from a test always counts as a caller — this flag does not widen
    /// analysis, it widens the report. Without it, findings categorised
    /// `only-used-by-tests` and findings on test callables themselves
    /// are withheld (and counted in the withheld total).
    #[arg(long = "include-tests", action = ArgAction::SetTrue)]
    pub include_tests: bool,

    /// Path to an additional ignore file (gitignore syntax).
    #[arg(long = "ignore-file", value_name = "PATH")]
    pub ignore_file: Option<PathBuf>,

    /// Number of parallel worker threads.
    ///
    /// `0` (the default) means auto: half the machine's **physical**
    /// cores, detected at runtime, capped at 8 and bounded by any cgroup
    /// quota. The cap keeps cgg a good guest on a large shared host —
    /// it is not a claim that more threads stop helping. On a big tree
    /// they do help: pass `--jobs 32` and expect roughly a 2x speedup
    /// over the default.
    #[arg(long = "jobs", value_name = "N", default_value_t = 0)]
    pub jobs: usize,

    /// Restrict analysis to the given comma-separated language ids.
    /// Example: `--lang rust,python`.
    #[arg(long = "lang", value_name = "LIST", value_delimiter = ',')]
    pub lang: Vec<String>,

    /// Shape of the audit output. `json` = batched doc; `jsonl` =
    /// streamed events (one per line, SIEM-friendly).
    #[arg(long = "audit-format", value_enum, default_value_t = AuditFormatArg::Json)]
    pub audit_format: AuditFormatArg,

    /// No effect — accepted for compatibility. Stack-graphs deep
    /// resolution was removed in the tree-sitter 0.26 upgrade (upstream
    /// `tree-sitter-stack-graphs` pins tree-sitter 0.24). The cross-file
    /// resolver and type propagation cover the same ground and run
    /// unconditionally, so all three values behave identically.
    #[arg(long = "stack-graphs", value_enum, default_value_t = StackGraphsArg::Auto)]
    pub stack_graphs: StackGraphsArg,

    /// Include calls into third-party code as deduplicated leaf "exit
    /// nodes" — one node per external symbol, with each call site
    /// collapsed onto it. Off by default; the edges are tagged so
    /// consumers can filter them.
    #[arg(long = "include-external", action = ArgAction::SetTrue)]
    pub include_external: bool,

    /// Include calls into the language standard library as deduplicated
    /// leaf "exit nodes", same as `--include-external` but for the
    /// stdlib bucket.
    #[arg(long = "include-stdlib", action = ArgAction::SetTrue)]
    pub include_stdlib: bool,

    /// Emit interface/trait dynamic-dispatch fan-out edges (declaration
    /// → each implementation), tagged `dynamic`/low-confidence. The
    /// exact call-site → declaration edge is always emitted; this flag
    /// adds the over-approximated fan-out. Off by default.
    #[arg(long = "dynamic-dispatch", action = ArgAction::SetTrue)]
    pub dynamic_dispatch: bool,

    /// Suppress synthesized `<framework-entry>` nodes.
    ///
    /// Entry nodes are ON by default, unlike `--include-external` and
    /// `--include-stdlib`. The asymmetry is deliberate: a route handler
    /// with in-degree zero is not merely an incomplete graph, it is a
    /// false claim that nothing calls it. An exit node, by contrast,
    /// tells you nothing you did not already know from reading the call.
    ///
    /// BEST EFFORT: entry nodes are INFERRED from framework markers, not
    /// observed. Coverage is partial, and every run prints a table
    /// naming which frameworks were recognised and which were seen but
    /// not understood.
    #[arg(long = "no-entry-nodes", action = ArgAction::SetTrue)]
    pub no_entry_nodes: bool,

    /// Print the framework-coverage table even when nothing was
    /// recognised. By default the table is printed only when at least
    /// one framework was detected; the gap list is never suppressed.
    #[arg(long = "framework-coverage", action = ArgAction::SetTrue)]
    pub framework_coverage: bool,

    /// Print a per-phase timing breakdown to stderr after the run.
    ///
    /// The four coarse buckets in the audit stop being useful once a
    /// phase has sub-phases; this shows where the time inside them goes.
    /// Off by default and free when off.
    #[arg(long = "profile", action = ArgAction::SetTrue)]
    pub profile: bool,

    /// Emit reference edges for functions passed by name as values
    /// (`register(handler)`), distinct from call edges and tagged
    /// `reference`. Off by default.
    #[arg(long = "reference-edges", action = ArgAction::SetTrue)]
    pub reference_edges: bool,

    /// Report callables that nothing in the analyzed source appears to
    /// call, marking them `unreferenced` in the normal graph output.
    ///
    /// BEST EFFORT: every finding is a hypothesis, not a fact. cgg
    /// reports what it could not find a caller for, which is not the
    /// same as proving no caller exists.
    ///
    /// The graph is emitted as usual in whatever `-t` selects; the
    /// detailed report goes to a sidecar (see `--dead-code-report`).
    #[arg(long = "dead-code", action = ArgAction::SetTrue)]
    pub dead_code: bool,

    /// Shape of the dead-code report. `text` = ranked and
    /// agent-readable (default); `json` = the stable `cgg.deadcode.v1`
    /// document.
    #[arg(long = "dead-code-format", value_enum, default_value_t = DeadCodeFormatArg::Text)]
    pub dead_code_format: DeadCodeFormatArg,

    /// Lowest confidence band to report. `high` (default) shows only
    /// findings with no mitigating signal on record. `medium` and `low`
    /// widen it. Withheld counts are always printed, whatever the band.
    #[arg(long = "dead-code-confidence", value_enum, default_value_t = DeadCodeConfidenceArg::High)]
    pub dead_code_confidence: DeadCodeConfidenceArg,

    /// Suppress dead-code findings whose qualified name matches
    /// PATTERN. Repeatable. Regex by default; prefix with `glob:` for
    /// glob syntax.
    ///
    /// Suppression is report-only: the callable still counts as a
    /// caller, so its callees do not become findings as a side effect.
    #[arg(long = "ignore-names", value_name = "PATTERN")]
    pub ignore_names: Vec<String>,

    /// Declared roots and accepted findings (TOML). Default: the
    /// nearest `cgg-deadcode.toml`, searching upward from each analyzed
    /// path first and then from the working directory, so
    /// `cgg /path/to/project` picks up that project's rules wherever it
    /// was launched from. Passing this disables that search.
    ///
    /// `roots` entries are entry points: a match is live, and so is
    /// everything it transitively calls. `[[allow]]` entries are
    /// reviewed findings; they are suppressed from the report but are
    /// NOT made live, so anything they reference is still reported.
    #[arg(long = "roots", value_name = "FILE")]
    pub roots: Option<PathBuf>,

    /// Write a `cgg-deadcode.toml` accepting every finding of this run,
    /// for adopting the tool on an existing codebase. Goes to the
    /// primary output *instead of* the graph; cgg never edits files in
    /// place. Implies `--dead-code`.
    #[arg(long = "write-roots", action = ArgAction::SetTrue)]
    pub write_roots: bool,

    /// Suppress dead-code findings on callables carrying a matching
    /// attribute or decorator (`#[no_mangle]`, `glob:@app.route*`).
    /// Repeatable; same pattern syntax as `--ignore-names`.
    ///
    /// Only some plugins capture attributes; on the rest this matches
    /// nothing. The per-language capability table in the report names
    /// which is which, and a run where nothing matched says so on
    /// stderr with the current list.
    #[arg(long = "ignore-attributes", value_name = "PATTERN")]
    pub ignore_attributes: Vec<String>,

    /// Explain why a callable is considered live: print the shortest
    /// path from a root, preferring high-confidence direct edges and
    /// non-test roots. Repeatable; same pattern syntax as `--filter`.
    /// Implies `--dead-code`.
    #[arg(long = "why-live", value_name = "PATTERN")]
    pub why_live: Vec<String>,

    /// Write the detailed dead-code report (evidence, roots, per-language
    /// capability table) to FILE.
    ///
    /// Defaults to a sidecar beside `-o`, named for the format:
    /// `<output>.deadcode.txt` or `<output>.deadcode.json`. With no
    /// `-o`, the text report goes to stderr and the JSON report needs
    /// this flag.
    #[arg(long = "dead-code-report", value_name = "FILE")]
    pub dead_code_report: Option<PathBuf>,

    /// Exit 3 when the dead-code report is non-empty. Off by default —
    /// cgg's exit status is unchanged unless you ask for this.
    #[arg(long = "fail-on-dead", action = ArgAction::SetTrue)]
    pub fail_on_dead: bool,

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

    /// No effect — accepted for compatibility. cgg makes no network
    /// calls at all: the update check that this flag used to disable was
    /// removed, along with the HTTP/TLS dependency it required. Use
    /// `cargo install-update` (from the `cargo-update` crate) if you want
    /// installed binaries refreshed on your own schedule.
    #[arg(long = "no-update-check", action = ArgAction::SetTrue)]
    pub no_update_check: bool,
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
pub enum DeadCodeFormatArg {
    /// Ranked, agent-readable text report.
    Text,
    /// `cgg.deadcode.v1` JSON document.
    Json,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum DeadCodeConfidenceArg {
    /// Only findings with no mitigating signal on record.
    High,
    /// ...plus findings with one over-approximation caveat.
    Medium,
    /// ...plus findings cgg has positive reason to doubt.
    Low,
}

impl From<DeadCodeConfidenceArg> for cgg_core::graph::Confidence {
    fn from(v: DeadCodeConfidenceArg) -> Self {
        match v {
            DeadCodeConfidenceArg::High => cgg_core::graph::Confidence::High,
            DeadCodeConfidenceArg::Medium => cgg_core::graph::Confidence::Medium,
            DeadCodeConfidenceArg::Low => cgg_core::graph::Confidence::Low,
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
