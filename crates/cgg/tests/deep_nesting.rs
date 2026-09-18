//! Deeply nested source must never crash the process.
//!
//! Every language plugin walks the syntax tree by recursive descent, and
//! before 0.8.5 nothing bounded that recursion. A tree deeper than the
//! 2 MiB default worker stack — roughly 2,000 levels, reached by an
//! ordinary generated `a + b + c …` expression, a fluent method chain or
//! a big data literal — overflowed the stack and **aborted the process**.
//! A stack overflow is not a panic, so the `catch_unwind` in the C, Python
//! and Node front ends could not turn it into an error: the host process
//! died with it.
//!
//! Two things fix it, and these tests pin both:
//!
//! * `cgg_lang::exceeds_depth` gates extraction, so a file nesting past
//!   `MAX_TREE_DEPTH` is skipped (`SkipReason::TooDeep`) and reported,
//!   never walked.
//! * The analysis runs on threads with a large stack, so a file *under*
//!   the cap that used to sit on the old 2 MiB cliff now analyzes.

use std::path::PathBuf;

use cgg::RunOptions;
use tempfile::TempDir;

fn analyze_one(dir: &TempDir, name: &str, body: &str) -> cgg::RunOutcome {
    let p: PathBuf = dir.path().join(name);
    std::fs::write(&p, body).unwrap();
    let opts = RunOptions {
        paths: vec![p],
        jobs: 1,
        ..Default::default()
    };
    // Reaching the `expect` at all is the assertion that matters: the
    // old code never returned from this call, it aborted.
    cgg::analyze(&opts).expect("analysis returns instead of aborting")
}

/// A tree far past the cap is skipped, reported, and the process lives.
#[test]
fn file_nested_past_the_cap_is_skipped_not_crashed() {
    let tmp = TempDir::new().unwrap();
    let depth = 100_000;
    let src = format!("y = {}1{}\n", "[".repeat(depth), "]".repeat(depth));
    let out = analyze_one(&tmp, "deep.py", &src);

    assert_eq!(
        out.metrics.files_analyzed, 0,
        "a too-deep file is not analyzed"
    );
    assert_eq!(out.metrics.files_skipped, 1, "…it is skipped, and counted");
    assert!(
        out.graph.callables.is_empty(),
        "nothing is extracted from a skipped file"
    );
    // The skip is named in the audit, so nothing is silently dropped.
    let slugs: Vec<String> = out
        .events
        .iter()
        .filter_map(|e| match e {
            cgg_core::audit::AuditEvent::FileSkipped { reason, .. } => {
                Some(reason.slug().to_string())
            }
            _ => None,
        })
        .collect();
    assert_eq!(slugs, vec!["too-deep"], "audit names the reason: {slugs:?}");
}

/// Nesting under the cap analyzes normally — including depths that sat on
/// the old 2 MiB cliff, because every analysis thread now has headroom.
///
/// Nested *lists*, deliberately: one tree level per bracket, so 3,000 of
/// them is 3,000 levels — past the old crash threshold, under the cap. A
/// nested *call* is two levels (`call` → `argument_list` → `call`), so
/// 3,000 of those would be ~6,000 levels and correctly skipped instead.
#[test]
fn file_nested_under_the_cap_analyzes() {
    let tmp = TempDir::new().unwrap();
    let depth = 3_000;
    let src = format!(
        "def f(x):\n    return x\n\ny = {}1{}\n",
        "[".repeat(depth),
        "]".repeat(depth)
    );
    let out = analyze_one(&tmp, "deep_list.py", &src);

    assert_eq!(out.metrics.files_skipped, 0, "under the cap is not skipped");
    assert_eq!(out.metrics.files_analyzed, 1);
    assert!(
        out.graph.callables.values().any(|c| c.simple_name == "f"),
        "the callable in a deep-but-legal file is still found"
    );
}

/// The shapes generated code actually produces — long operator chains —
/// nest one level per term. 3,000 terms crashed 0.8.4 in C, Go, Java,
/// JavaScript, Python and Rust; they must simply analyze now.
#[test]
fn long_operator_chain_analyzes() {
    let tmp = TempDir::new().unwrap();
    let terms = vec!["1"; 3_000].join(" + ");
    let src = format!("int h(void) {{ return {terms}; }}\n");
    let out = analyze_one(&tmp, "sum.c", &src);

    assert_eq!(out.metrics.files_analyzed, 1);
    assert!(
        out.graph.callables.values().any(|c| c.simple_name == "h"),
        "the function wrapping a 3,000-term expression is found"
    );
}
