//! Tests for the library entry point, `cgg::analyze`.
//!
//! Every other integration test here shells out via `assert_cmd`, so each
//! gets a fresh address space and none can see the bug class this file
//! exists for: state that survives from one analysis to the next. Every
//! test below runs `analyze` more than once in a single process.

use std::fs;
use std::path::{Path, PathBuf};

use cgg::cli::Cli;
use cgg::{Emission, RunOptions, analyze};
use clap::Parser;
use tempfile::TempDir;

/// A two-language tree with cross-file calls, an unreferenced function,
/// and enough files to give the scheduler something to reorder.
fn fixture(dir: &Path) {
    fs::create_dir_all(dir.join("pkg")).unwrap();
    for i in 0..8 {
        fs::write(
            dir.join(format!("pkg/mod_{i}.py")),
            format!(
                "def helper_{i}(x):\n    return x + {i}\n\n\
                 def caller_{i}():\n    return helper_{i}({i})\n\n\
                 def orphan_{i}():\n    return {i}\n"
            ),
        )
        .unwrap();
    }
    // A trait impl and a function passed by name, so dead-code mode
    // (which forces `dynamic_dispatch` and `reference_edges` on) has
    // something to add — otherwise mode comparisons are vacuous.
    fs::write(
        dir.join("lib.rs"),
        "pub trait Greet {\n    fn greet(&self) -> u32;\n}\n\
         pub struct Loud;\n\
         impl Greet for Loud {\n    fn greet(&self) -> u32 { helper() }\n}\n\
         pub fn used() -> u32 { helper() }\n\
         fn helper() -> u32 { 1 }\n\
         fn never_called() -> u32 { 2 }\n\
         pub fn register() { take(helper); }\n\
         fn take(_f: fn() -> u32) {}\n",
    )
    .unwrap();
}

fn opts(dir: &Path) -> RunOptions {
    RunOptions::new(vec![dir.to_path_buf()])
}

/// Callables and edges as comparable strings, ignoring anything that
/// legitimately varies between runs (timings).
fn structure(outcome: &cgg::RunOutcome) -> (Vec<String>, Vec<String>) {
    // `unreferenced` included: it is the graph-visible product of
    // dead-code mode.
    let callables: Vec<String> = outcome
        .graph
        .callables
        .values()
        .map(|c| {
            format!(
                "{}|{}|{}|{:?}",
                c.qualified_name, c.language, c.start_line, c.unreferenced
            )
        })
        .collect();
    let edges: Vec<String> = outcome
        .graph
        .edges
        .iter()
        .map(|e| {
            format!(
                "{:?}->{:?}@{}|{:?}|{:?}",
                e.src, e.dst, e.site_byte, e.via, e.confidence
            )
        })
        .collect();
    (callables, edges)
}

// --- The in-process hazards -------------------------------------------

/// `jobs` must be honoured on every call, not just the first.
///
/// Asserts on `RunOutcome::jobs`, read from `current_num_threads()` inside
/// the pool — the width the work ran at, not the width requested.
/// Comparing graph *output* across `jobs` values would not catch this:
/// identical output is exactly what an ignored `jobs` produces.
#[test]
fn jobs_is_honoured_on_every_call_not_just_the_first() {
    let td = TempDir::new().unwrap();
    fixture(td.path());

    // Up then down, so neither monotonic direction satisfies it by
    // accident; repeats catch a cached pool of the wrong size.
    for jobs in [1usize, 4, 1, 3, 2, 1] {
        let o = analyze(&RunOptions {
            jobs,
            ..opts(td.path())
        })
        .expect("analyze");
        assert_eq!(
            o.jobs, jobs,
            "asked for {jobs} workers, ran on {}; a previous call's pool was reused",
            o.jobs
        );
    }
}

/// Output must not depend on the worker count. Separate from the test
/// above: that one checks the pool width, this one the graph.
#[test]
fn output_does_not_depend_on_worker_count() {
    let td = TempDir::new().unwrap();
    fixture(td.path());

    let first = structure(
        &analyze(&RunOptions {
            jobs: 1,
            ..opts(td.path())
        })
        .unwrap(),
    );
    for jobs in [2usize, 3, 4, 8] {
        let got = structure(
            &analyze(&RunOptions {
                jobs,
                ..opts(td.path())
            })
            .unwrap(),
        );
        assert_eq!(got, first, "graph differs at jobs={jobs}");
    }
}

/// Two projects with different framework rules, analyzed in one process,
/// must each get their own rules applied.
///
/// **Does not detect the verb latch** — `registrar::capture_value_refs`
/// runs on exactly the calls the gate rejects and emits a subset of the
/// same records, so the handler is captured either way and graph-level
/// comparisons come out equal. That is tested on the primitive, in
/// `cgg-lang`'s `registrar_verbs_are_replaceable_not_latched`. What this
/// covers is the end-to-end path: a per-project config reaching the
/// resolver twice in a row.
#[test]
fn two_projects_each_get_their_own_framework_rules() {
    // A different custom verb each, neither shipped by cgg. `detect`
    // matches the import that proves the framework is in use.
    //
    // The config lives OUTSIDE the analyzed tree: `roots: None` triggers
    // an upward search from the analyzed path, so a config next to
    // `app.py` would be found anyway and the control would not be one.
    let cfgs = TempDir::new().unwrap();
    let project = |dir: &Path, verb: &str| {
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(
            dir.join("src/app.py"),
            format!(
                "import app\n\n\
                 def handler():\n    return 1\n\n\
                 {verb}(\"/route\", handler)\n"
            ),
        )
        .unwrap();
        // `[[framework]]` singular — the field is renamed. The schema is
        // `deny_unknown_fields`, so a wrong key is a hard error.
        let cfg = cfgs.path().join(format!("{verb}.toml"));
        fs::write(
            &cfg,
            format!(
                "[[framework]]\n\
                 id = \"fw_{verb}\"\n\
                 language = \"python\"\n\
                 detect = [\"app\"]\n\
                 registrars = [\"{verb}\"]\n"
            ),
        )
        .unwrap();
        cfg
    };

    let a = TempDir::new().unwrap();
    let cfg_a = project(a.path(), "wire");
    let b = TempDir::new().unwrap();
    let cfg_b = project(b.path(), "attach");

    let run = |dir: &Path, cfg: Option<&Path>| {
        structure(
            &analyze(&RunOptions {
                roots: cfg.map(|p| p.to_path_buf()),
                ..opts(dir)
            })
            .expect("analyze"),
        )
    };

    // Asserting on both, rather than B-before vs B-after, keeps this
    // order-independent: `cargo test` runs tests as parallel threads, so
    // anything depending on which analysis ran first is a race.
    let a_with = run(a.path(), Some(&cfg_a));
    let b_with = run(b.path(), Some(&cfg_b));

    // Controls: same trees, no rules.
    let a_without = run(a.path(), None);
    let b_without = run(b.path(), None);

    assert_ne!(
        a_with, a_without,
        "project A's framework rule never registered — its registrar verb \
         was suppressed by another run's"
    );
    assert_ne!(
        b_with, b_without,
        "project B's framework rule never registered — its registrar verb \
         was suppressed by another run's"
    );
}

/// `--write-roots` must return the baseline, not exit the process.
///
/// This was `std::process::exit(0)`, which would kill a host interpreter.
/// If it comes back this test does not fail — the process dies, which is
/// louder.
#[test]
fn write_roots_returns_a_baseline_instead_of_exiting() {
    let td = TempDir::new().unwrap();
    fixture(td.path());

    let o = analyze(&RunOptions {
        write_roots: true,
        ..opts(td.path())
    })
    .expect("analyze");

    let toml = o.transcript.iter().find_map(|e| match e {
        Emission::RootsBaseline(t) => Some(t),
        _ => None,
    });
    match toml {
        Some(toml) => {
            assert!(
                toml.contains("[[allow]]") || toml.contains("roots"),
                "baseline should be a cgg-deadcode.toml document, got: {toml:?}"
            );
        }
        None => panic!(
            "expected a roots baseline in the transcript: {:?}",
            o.transcript
        ),
    }

    // Reaching this line at all is the assertion that matters: the
    // process is still alive after a --write-roots analysis.
    let after = analyze(&opts(td.path())).expect("analyze still works afterwards");
    assert!(!after.graph.callables.is_empty());
}

/// Repeated identical calls must produce identical graphs — the general
/// form of the hazards above.
#[test]
fn repeated_identical_calls_are_identical() {
    let td = TempDir::new().unwrap();
    fixture(td.path());

    let first = structure(&analyze(&opts(td.path())).expect("analyze"));
    for n in 2..=4 {
        let got = structure(&analyze(&opts(td.path())).expect("analyze"));
        assert_eq!(got, first, "call {n} differs from call 1");
    }
}

/// Interleaving different option sets must not contaminate either.
#[test]
fn interleaved_option_sets_do_not_contaminate_each_other() {
    let td = TempDir::new().unwrap();
    fixture(td.path());

    let dead = || RunOptions {
        dead_code: true,
        ..opts(td.path())
    };

    let plain_a = analyze(&opts(td.path())).unwrap();
    let dead_a = analyze(&dead()).unwrap();
    let plain_b = analyze(&opts(td.path())).unwrap();
    let dead_b = analyze(&dead()).unwrap();

    assert_eq!(
        structure(&plain_a),
        structure(&plain_b),
        "a dead-code run changed the plain result"
    );
    assert_eq!(
        structure(&dead_a),
        structure(&dead_b),
        "a plain run changed the dead-code result"
    );

    // Non-vacuity: dead-code mode must have done something, or the two
    // equalities above compare identical no-ops.
    assert!(
        plain_a.dead_code.is_none(),
        "a plain run should carry no dead-code report"
    );
    let report = dead_a
        .dead_code
        .as_ref()
        .expect("dead-code mode must produce a report");
    assert!(
        !report.findings.is_empty(),
        "fixture produced no findings; the interleaving test would be vacuous"
    );
}

/// Concurrent `analyze` calls each return the graph they asked for.
///
/// These genuinely run at the same time: extraction's switches travel in a
/// per-run `ExtractCtx`, so there is no shared state and nothing to lock.
/// An earlier version held a process-wide mutex here, which made concurrent
/// callers serial — 4.02x wall for four threads, against 1.07x now.
///
/// Alternating the option sets is the point. If those switches ever became
/// shared again, a dead-code run overlapping a plain one would produce the
/// other's graph, and that is exactly what this compares.
#[test]
fn concurrent_analyze_calls_each_get_the_graph_they_asked_for() {
    use std::sync::Arc;
    use std::thread;

    let td = Arc::new(TempDir::new().unwrap());
    fixture(td.path());

    // Sequential, so the expectation cannot itself be a race victim.
    let expected_plain = structure(&analyze(&opts(td.path())).unwrap());
    let expected_dead = structure(
        &analyze(&RunOptions {
            dead_code: true,
            ..opts(td.path())
        })
        .unwrap(),
    );

    // Alternating, so a lost switch write shows up as the wrong graph.
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let td = Arc::clone(&td);
            thread::spawn(move || {
                let dead = i % 2 == 0;
                let o = analyze(&RunOptions {
                    dead_code: dead,
                    jobs: 2,
                    ..RunOptions::new(vec![td.path().to_path_buf()])
                })
                .expect("analyze");
                (dead, structure(&o))
            })
        })
        .collect();

    for h in handles {
        let (dead, got) = h.join().expect("thread panicked");
        let want = if dead {
            &expected_dead
        } else {
            &expected_plain
        };
        assert_eq!(
            &got, want,
            "a concurrent analyze produced the wrong graph (dead_code={dead}); \
             the extraction globals were interleaved"
        );
    }
}

// --- RunOptions <-> Cli ------------------------------------------------

/// `RunOptions::default()` must match what a bare command line parses to.
///
/// Two independent default sets — clap's attributes and `impl Default` —
/// and nothing but this test stops them drifting.
#[test]
fn run_options_default_matches_a_bare_command_line() {
    let cli = Cli::try_parse_from(["cgg", "/some/path"]).expect("parses");
    let from_cli = RunOptions::from(&cli);

    let expected = RunOptions {
        paths: vec![PathBuf::from("/some/path")],
        ..RunOptions::default()
    };

    assert_eq!(
        from_cli, expected,
        "clap's defaults and RunOptions::default() have drifted"
    );
}

/// Every analysis-relevant flag must survive `Cli` -> `RunOptions`.
///
/// Forgetting a field is already a compile error; this covers routing one
/// to the wrong place, which is not.
#[test]
fn cli_analysis_flags_all_reach_run_options() {
    let cli = Cli::try_parse_from([
        "cgg",
        "/a",
        "/b",
        "--filter",
        "foo",
        "--exclude-partial",
        "tests::",
        "--exclude-glob",
        "*::internal::*",
        "--exclude-regex",
        "^test_",
        "-n",
        "3",
        "--max-paths",
        "7",
        "--lang",
        "rust,python",
        "--jobs",
        "5",
        "--include-external",
        "--include-stdlib",
        "--dynamic-dispatch",
        "--reference-edges",
        "--no-entry-nodes",
        "--framework-coverage",
        "--dead-code",
        "--dead-code-confidence",
        "low",
        "--ignore-names",
        "drop_me",
        "--ignore-attributes",
        "no_mangle",
        "--include-tests",
        "--why-live",
        "target",
        "--since",
        "HEAD~2",
    ])
    .expect("parses");

    let o = RunOptions::from(&cli);

    assert_eq!(o.paths, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
    assert_eq!(o.filter, ["foo"]);
    assert_eq!(o.exclude_partial, ["tests::"]);
    assert_eq!(o.exclude_glob, ["*::internal::*"]);
    assert_eq!(o.exclude_regex, ["^test_"]);
    assert_eq!(o.hops, 3);
    assert_eq!(o.max_paths, 7);
    assert_eq!(o.lang, ["rust", "python"]);
    assert_eq!(o.jobs, 5);
    assert!(o.include_external);
    assert!(o.include_stdlib);
    assert!(o.dynamic_dispatch);
    assert!(o.reference_edges);
    assert!(o.no_entry_nodes);
    assert!(o.framework_coverage);
    assert!(o.dead_code);
    assert_eq!(o.dead_code_confidence, cgg_core::graph::Confidence::Low);
    assert_eq!(o.ignore_names, ["drop_me"]);
    assert_eq!(o.ignore_attributes, ["no_mangle"]);
    assert!(o.include_tests);
    assert_eq!(o.why_live, ["target"]);
    assert_eq!(o.since.as_deref(), Some("HEAD~2"));

    // `--why-live` and `--dead-code` both imply dead-code mode.
    assert!(o.dead_mode());
}

// --- analyze's contract -----------------------------------------------

/// `analyze` must not write anything to the filesystem — the point of the
/// analyze/emit split. Also catches a sidecar landing next to the input.
#[test]
fn analyze_writes_nothing() {
    let td = TempDir::new().unwrap();
    fixture(td.path());

    let before = walk_sorted(td.path());
    let _ = analyze(&RunOptions {
        dead_code: true,
        ..opts(td.path())
    })
    .expect("analyze");
    let after = walk_sorted(td.path());

    assert_eq!(before, after, "analyze created or modified files");
}

fn walk_sorted(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(format!("{}:{}", p.display(), e.metadata().unwrap().len()));
            }
        }
    }
    out.sort();
    out
}

/// An empty `paths` is an error, not a panic or an empty graph.
#[test]
fn empty_paths_is_an_error() {
    let err = analyze(&RunOptions::default()).expect_err("should reject empty paths");
    assert!(
        err.to_string().contains("no input paths"),
        "unexpected error: {err}"
    );
}

/// A nonexistent path is an error naming the path.
#[test]
fn nonexistent_path_is_an_error_naming_it() {
    let err = analyze(&RunOptions::new(vec![PathBuf::from("/no/such/cgg/dir")]))
        .expect_err("should reject a missing path");
    let msg = err.to_string();
    assert!(msg.contains("/no/such/cgg/dir"), "unexpected error: {msg}");
}

/// The renderers must agree with what the graph contains.
#[test]
fn renderers_reflect_the_analyzed_graph() {
    use cgg_format::OutputFormat;

    let td = TempDir::new().unwrap();
    fixture(td.path());
    let o = analyze(&opts(td.path())).expect("analyze");

    let mermaid = cgg::emit::graph_to_string(&o.graph, OutputFormat::Mermaid);
    let json = cgg::emit::graph_to_string(&o.graph, OutputFormat::Json);
    let dot = cgg::emit::graph_to_string(&o.graph, OutputFormat::Dot);
    let graphml = cgg::emit::graph_to_string(&o.graph, OutputFormat::Graphml);

    assert!(
        mermaid.starts_with("flowchart"),
        "mermaid: {:?}",
        &mermaid[..40.min(mermaid.len())]
    );
    assert!(dot.contains("digraph"), "dot should be a digraph");
    assert!(
        graphml.contains("<graphml"),
        "graphml should have a root element"
    );

    let v: serde_json::Value = serde_json::from_str(&json).expect("json parses");
    assert_eq!(
        v["callables"].as_object().map(|m| m.len()),
        Some(o.graph.callables.len()),
        "json callable count disagrees with the graph"
    );

    // A known cross-file Python call must be present in the rendering,
    // so this is testing the graph and not just the string shape.
    assert!(
        mermaid.contains("caller_0"),
        "expected caller_0 in the mermaid output"
    );
}

/// Exactly one artifact per run, in exactly one place.
///
/// The transcript makes a position without a payload unrepresentable, but
/// nothing stops a path from pushing *two* artifacts or none. The failure
/// is silent either way: no artifact means a run that writes no graph,
/// exits 0, and prints a summary claiming N callables.
#[test]
fn exactly_one_artifact_per_run() {
    let td = TempDir::new().unwrap();
    fixture(td.path());

    // Exactly one artifact per run: Graph, WhyLive or RootsBaseline.
    let count = |o: &cgg::RunOutcome| {
        o.transcript
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    Emission::Graph | Emission::WhyLive(_) | Emission::RootsBaseline(_)
                )
            })
            .count()
    };

    // Every path through `analyze` must place it exactly once, including the
    // two that return early with a different primary.
    let plain = analyze(&opts(td.path())).unwrap();
    assert_eq!(count(&plain), 1, "ordinary run");

    let dead = analyze(&RunOptions {
        dead_code: true,
        ..opts(td.path())
    })
    .unwrap();
    assert_eq!(count(&dead), 1, "--dead-code");

    let roots = analyze(&RunOptions {
        write_roots: true,
        ..opts(td.path())
    })
    .unwrap();
    assert_eq!(count(&roots), 1, "--write-roots");

    let why = analyze(&RunOptions {
        why_live: vec!["helper".to_string()],
        ..opts(td.path())
    })
    .unwrap();
    assert_eq!(count(&why), 1, "--why-live");
}

/// Notices come back as data, and the run summary is among them.
#[test]
fn notices_carry_the_run_summary() {
    let td = TempDir::new().unwrap();
    fixture(td.path());
    let o = analyze(&opts(td.path())).expect("analyze");

    let summary = o
        .notices()
        .find(|t| t.starts_with("cgg: "))
        .expect("a run summary notice");

    assert!(summary.contains("callables"), "summary: {summary:?}");
    assert!(summary.contains("edges"), "summary: {summary:?}");
    // `notices()` strips the trailing newline the raw transcript carries.
    assert!(!summary.ends_with('\n'), "notices() should be trimmed");
}
