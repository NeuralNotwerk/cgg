//! The graph must not depend on how many threads produced it.
//!
//! cgg parallelises file parsing, extraction and intra-file linking, and
//! 0.5.0 pushes that further. Every one of those is a chance to let a
//! `HashMap` iteration order or a completion order leak into the output.
//! A call graph that changes shape with `--jobs` is not reproducible, and
//! reproducibility is most of what makes the output reviewable in a diff.
//!
//! **Compare structure, not bytes.** The JSON and audit documents embed
//! per-run wall/parse timings, so two identical runs never hash the same
//! and a naive byte comparison reports nondeterminism that is not there —
//! a trap this test exists partly to stop the next person falling into.
//! What must match exactly: the callable set *and its order*, and every
//! edge with its endpoints, site, `via` and confidence.

use std::fs;
use std::process::Command;

use assert_cmd::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

/// A fixture with enough files, cross-file references and framework
/// shapes to give the scheduler something to reorder.
fn fixture(dir: &std::path::Path) {
    fs::create_dir_all(dir.join("pkg")).unwrap();
    for i in 0..24 {
        fs::write(
            dir.join(format!("pkg/mod_{i}.py")),
            format!(
                "from flask import Flask\n\
                 app = Flask(__name__)\n\n\
                 def helper_{i}(x):\n    return x + {i}\n\n\
                 @app.route(\"/r{i}\")\n\
                 def view_{i}():\n    return helper_{i}({i}) + shared()\n\n\
                 def shared():\n    return 1\n"
            ),
        )
        .unwrap();
    }
    fs::write(dir.join("go.mod"), "module demo\ngo 1.21\n").unwrap();
    for i in 0..12 {
        fs::write(
            dir.join(format!("svc_{i}.go")),
            format!(
                "package main\n\n\
                 type Handler{i} struct{{}}\n\n\
                 func (h *Handler{i}) Serve() int {{ return help{i}() }}\n\
                 func help{i}() int {{ return {i} }}\n"
            ),
        )
        .unwrap();
    }
}

/// Everything about the graph that is supposed to be reproducible.
fn structure(json: &str) -> Vec<String> {
    let v: Value = serde_json::from_str(json).expect("valid json");
    let mut out = Vec::new();
    for (id, c) in v["callables"].as_object().expect("callables") {
        out.push(format!(
            "N {id} {} {} {}",
            c["qualified_name"], c["language"], c["start_line"]
        ));
    }
    for e in v["edges"].as_array().expect("edges") {
        out.push(format!(
            "E {} {} {} {} {}",
            e["src"], e["dst"], e["site_line"], e["via"], e["confidence"]
        ));
    }
    out
}

fn run_with_jobs(dir: &std::path::Path, jobs: &str) -> Vec<String> {
    let out = dir.join(format!("g{jobs}.json"));
    Command::cargo_bin("cgg")
        .unwrap()
        .arg(dir)
        .args(["-t", "json", "-o"])
        .arg(&out)
        .args(["--jobs", jobs, "--no-update-check"])
        .assert()
        .success();
    structure(&fs::read_to_string(&out).unwrap())
}

/// Render mermaid at `jobs`, verbatim.
fn mermaid_with_jobs(dir: &std::path::Path, jobs: &str, extra: &[&str]) -> String {
    let out = dir.join(format!("m{jobs}{}.mmd", extra.join("")));
    Command::cargo_bin("cgg")
        .unwrap()
        .arg(dir)
        .args(["-t", "mermaid", "-o"])
        .arg(&out)
        .args(["--jobs", jobs, "--no-update-check"])
        .args(extra)
        .assert()
        .success();
    fs::read_to_string(&out).unwrap()
}

/// Mermaid numbers its nodes by position, so **graph order is directly
/// observable in the output** — and that is new.
///
/// A content-hash id is a pure function of the callable's identity, so
/// before 0.8.2 a reordering of `graph.callables` was invisible in a
/// diagram: every node kept its id and every edge kept its endpoints.
/// An ordinal has no such protection. If any parallel phase can emit
/// callables in a different order at a different worker count, every id
/// shifts and every edge line shifts with it.
///
/// `the_graph_is_identical_at_every_thread_count` pins the JSON order
/// this depends on, which is the root property; this pins the rendering
/// that now reads it, so a future change to how nodes are numbered
/// cannot quietly reintroduce an order dependence. `scripts/
/// determinism-sweep.py` deliberately does not vary `--jobs`, so this is
/// the only place the combination is checked.
#[test]
fn mermaid_is_byte_identical_at_every_thread_count() {
    let tmp = TempDir::new().unwrap();
    fixture(tmp.path());

    let one = mermaid_with_jobs(tmp.path(), "1", &[]);
    assert!(one.contains("N0["), "expected numbered ids:\n{one}");
    for jobs in ["2", "3", "8", "32"] {
        let many = mermaid_with_jobs(tmp.path(), jobs, &[]);
        assert_eq!(
            one, many,
            "--jobs {jobs} changed the mermaid rendering; node ids are \
             ordinals, so this means the graph order moved"
        );
    }

    // The hashed form is the control: it was order-independent before
    // this change, so a failure here would be a different (older) bug.
    let hash_one = mermaid_with_jobs(tmp.path(), "1", &["--node-ids", "hash"]);
    for jobs in ["2", "8", "32"] {
        let many = mermaid_with_jobs(tmp.path(), jobs, &["--node-ids", "hash"]);
        assert_eq!(hash_one, many, "--jobs {jobs} moved the hashed rendering");
    }
}

#[test]
fn the_graph_is_identical_at_every_thread_count() {
    let tmp = TempDir::new().unwrap();
    fixture(tmp.path());

    let one = run_with_jobs(tmp.path(), "1");
    assert!(!one.is_empty(), "fixture produced no graph");

    for jobs in ["2", "8", "32"] {
        let many = run_with_jobs(tmp.path(), jobs);
        assert_eq!(
            one.len(),
            many.len(),
            "--jobs {jobs} produced {} graph elements, --jobs 1 produced {}",
            many.len(),
            one.len()
        );
        if one != many {
            let first = one.iter().zip(&many).position(|(a, b)| a != b).unwrap_or(0);
            panic!(
                "--jobs {jobs} reordered the graph. First difference at {first}:\n  \
                 jobs=1  {}\n  jobs={jobs}  {}",
                one[first], many[first]
            );
        }
    }
}

/// The two defects the first version of this file was blind to, both
/// found by an adversarial review rather than by this test:
///
/// * `--dead-code` force-enables `--dynamic-dispatch`, and
///   `dispatch::fanout` iterated a `HashMap`, so the fan-out edges came
///   out in a different order every run. Needs a *trait with several
///   implementations* to show up at all.
/// * `-n 0` with `--max-paths` walks entry points in `HashMap` order, so
///   which paths survive the cap varied. Needs truncation to actually
///   bite — without the cap the result is the same set either way, which
///   is exactly why it hid.
fn dispatch_fixture(dir: &std::path::Path) {
    fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"d\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::create_dir_all(dir.join("src")).unwrap();
    let mut src =
        String::from("pub trait Store {\n    fn put(&self);\n    fn get(&self);\n}\n");
    for i in 0..16 {
        src.push_str(&format!(
            "pub struct S{i};\nimpl Store for S{i} {{\n             \x20   fn put(&self) {{ helper{i}(); }}\n             \x20   fn get(&self) {{}}\n}}\n             fn helper{i}() {{}}\n"
        ));
    }
    src.push_str("pub fn drive(s: &dyn Store) { s.put(); s.get(); }\n");
    fs::write(dir.join("src/lib.rs"), src).unwrap();
}

fn run_args(dir: &std::path::Path, tag: &str, extra: &[&str]) -> Vec<String> {
    let out = dir.join(format!("g_{tag}.json"));
    Command::cargo_bin("cgg")
        .unwrap()
        .arg(dir)
        .args(["-t", "json", "-o"])
        .arg(&out)
        .args(["--no-update-check"])
        .args(extra)
        .assert()
        .success();
    structure(&fs::read_to_string(&out).unwrap())
}

#[test]
fn dynamic_dispatch_fanout_is_ordered() {
    let tmp = TempDir::new().unwrap();
    dispatch_fixture(tmp.path());
    let first = run_args(tmp.path(), "d0", &["--dead-code", "--jobs", "8"]);
    for i in 1..4 {
        let again = run_args(
            tmp.path(),
            &format!("d{i}"),
            &["--dead-code", "--jobs", "8"],
        );
        assert_eq!(
            first, again,
            "--dead-code (which enables --dynamic-dispatch) reordered the \
             graph between runs on attempt {i}"
        );
    }
}

#[test]
fn truncated_path_extraction_is_ordered() {
    let tmp = TempDir::new().unwrap();
    dispatch_fixture(tmp.path());
    // A cap low enough to actually turn work away; that is the only
    // regime in which the entry ordering could ever have mattered.
    let args = [
        "-n",
        "0",
        "--max-paths",
        "4",
        "--filter",
        "helper",
        "--jobs",
        "8",
    ];
    let first = run_args(tmp.path(), "p0", &args);
    for i in 1..4 {
        let again = run_args(tmp.path(), &format!("p{i}"), &args);
        assert_eq!(
            first, again,
            "`-n 0 --max-paths 4` produced a different graph on attempt {i}"
        );
    }
}

/// The dead-code *report* is a separate artifact from the graph, written
/// to its own sidecar, and nothing covered it.
///
/// Scope, stated because an overclaimed test is worse than none: this
/// does **not** catch the `dispatch::fanout` ordering defect — verified
/// by reverting that fix, which leaves this test green while
/// `dynamic_dispatch_fanout_is_ordered` fails. Fan-out order reaches the
/// graph's edge list, not the report's finding list. What this covers is
/// the report artifact itself: finding order, and any future
/// nondeterminism in the ranking or grouping that produces it.
#[test]
fn the_dead_code_report_is_identical_at_every_thread_count() {
    let tmp = TempDir::new().unwrap();
    dispatch_fixture(tmp.path());

    let read_report = |tag: &str, jobs: &str| -> String {
        let out = tmp.path().join(format!("dc_{tag}.json"));
        Command::cargo_bin("cgg")
            .unwrap()
            .arg(tmp.path())
            .args(["-t", "json", "-o"])
            .arg(&out)
            .args([
                "--dead-code",
                "--dead-code-format",
                "json",
                "--jobs",
                jobs,
                "--no-update-check",
            ])
            .assert()
            .success();
        let report = out.with_extension("json.deadcode.json");
        fs::read_to_string(&report)
            .unwrap_or_else(|e| panic!("no report at {}: {e}", report.display()))
    };

    let first = read_report("a", "1");
    assert!(!first.trim().is_empty(), "report was empty");
    for (tag, jobs) in [("b", "4"), ("c", "16"), ("d", "1")] {
        assert_eq!(
            first,
            read_report(tag, jobs),
            "the dead-code report differed at --jobs {jobs}"
        );
    }
}

/// Type-hint propagation resolves a receiver name to a return type, and
/// the match is deliberately loose: it accepts an exact name match OR a
/// plural. So `Config` and `Configs` both claim a receiver called
/// `configs`, and `HttpClient`/`HTTPClient` collide once lowercased.
/// Both lookups walked hash-ordered collections, so the winner — and the
/// resulting edge — differed between runs of the same binary on the same
/// input, single-threaded, with default flags.
///
/// This fixture exists to create exactly those collisions. Without a
/// colliding pair the code path is deterministic by luck, which is why
/// the rest of the suite never saw it.
///
/// Scope, verified by reverting each fix separately: this test catches
/// the `return_types` lookup (`type_hints.rs`, the plural matcher). It
/// does **not** catch the `type_names` lookup in
/// `find_constructor_assignments` — reverting that one leaves this test
/// green, because reaching it needs a lowercase collision that also
/// survives constructor inference, which this fixture does not produce.
/// That site is fixed and covered only by inspection. Saying so is
/// better than implying coverage this does not have.
fn colliding_types_fixture(dir: &std::path::Path) {
    fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"c\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("src/main.rs"),
        "pub struct Config;\nimpl Config { pub fn apply(&self) {} }\n         pub struct Configs;\nimpl Configs { pub fn apply(&self) {} }\n         pub fn get_config() -> Config { Config }\n         pub fn list_configs() -> Configs { Configs }\n         pub fn run(configs: u8) {\n         \x20   let _ = configs;\n         \x20   get_config();\n         \x20   list_configs();\n         \x20   configs.apply();\n}\n         fn main() { run(0); }\n",
    )
    .unwrap();
    // A second language, with a lowercase collision of its own.
    fs::write(dir.join("go.mod"), "module d\ngo 1.21\n").unwrap();
    fs::write(
        dir.join("b.go"),
        "package main\ntype Repo struct{}\nfunc (r *Repo) Find() {}\n         type REPO struct{}\nfunc (r *REPO) Find() {}\n         func NewRepo() *Repo { return &Repo{} }\n         func run(repo *Repo) { NewRepo(); repo.Find() }\n",
    )
    .unwrap();
}

#[test]
fn type_hint_collisions_resolve_the_same_way_every_run() {
    let tmp = TempDir::new().unwrap();
    colliding_types_fixture(tmp.path());
    // Single-threaded on purpose: this defect had nothing to do with
    // parallelism, and running it at --jobs 1 proves that.
    let first = run_with_jobs(tmp.path(), "1");
    for i in 1..12 {
        assert_eq!(
            first,
            run_with_jobs(tmp.path(), "1"),
            "type-hint resolution differed between identical runs (attempt {i})"
        );
    }
}

#[test]
fn repeated_runs_agree() {
    // Guards against a source of nondeterminism that does not depend on
    // thread count at all — `RandomState` reseeds per process, so a
    // HashMap iteration order that leaks into the output differs between
    // runs even single-threaded.
    let tmp = TempDir::new().unwrap();
    fixture(tmp.path());
    let a = run_with_jobs(tmp.path(), "4");
    let b = run_with_jobs(tmp.path(), "4");
    assert_eq!(a, b, "two identical runs disagreed");
}
