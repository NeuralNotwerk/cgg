//! End-to-end coverage for the CLI flags that had none.
//!
//! `cli.rs` covers argument validation and the dead-code sidecar rules.
//! This file covers the rest of the documented surface — the flags a
//! user reaches for that no test exercised: `--why-live`, `--roots`,
//! `--since`, `--fail-on-dead`, `--include-stdlib`, `--include-tests`,
//! the `--ignore-*` family, `--framework-coverage`, and the verbosity
//! switches.
//!
//! Two of these tests are regressions for bugs this file's own
//! authoring turned up, both marked REGRESSION below.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;

fn cgg() -> Command {
    Command::cargo_bin("cgg").expect("cgg binary built")
}

/// One live chain (`entry` → `leaf`) plus one callable nothing reaches.
fn fixture() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("m.py"),
        concat!(
            "def leaf():\n    return 1\n",
            "def entry():\n    return leaf()\n",
            "def orphan():\n    return 2\n",
        ),
    )
    .unwrap();
    tmp
}

fn write(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

// ---------------------------------------------------------------------
// Version and verbosity
// ---------------------------------------------------------------------

#[test]
fn version_prints_the_crate_version() {
    cgg()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn quiet_silences_the_run_summary() {
    // REGRESSION: `--quiet` promises "silence everything except
    // errors", and every advisory line in the run was gated on it
    // except the one-line run summary — the single noisiest thing cgg
    // prints. `-q` was therefore a no-op for the output that actually
    // motivates reaching for it.
    let tmp = fixture();
    let out = tmp.path().join("g.mmd");
    cgg()
        .arg(tmp.path())
        .arg("-o")
        .arg(&out)
        .arg("-q")
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
}

#[test]
fn quiet_does_not_swallow_errors() {
    // The other half of the contract: quiet is not silent.
    cgg()
        .arg("/tmp/definitely-not-a-real-path-4f3e2d1c")
        .arg("-q")
        .assert()
        .failure()
        .stderr(predicate::str::contains("input path does not exist"));
}

#[test]
fn verbose_adds_structured_log_lines() {
    let tmp = fixture();
    let out = tmp.path().join("g.mmd");
    cgg()
        .arg(tmp.path())
        .arg("-o")
        .arg(&out)
        .arg("-vv")
        .assert()
        .success()
        .stderr(predicate::str::contains("cgg starting"));
}

#[test]
fn no_update_check_is_accepted_and_changes_nothing() {
    // Kept only so existing command lines keep working. The graph must
    // be byte-identical with and without it.
    let tmp = fixture();
    let a = tmp.path().join("a.mmd");
    let b = tmp.path().join("b.mmd");
    cgg().arg(tmp.path()).arg("-o").arg(&a).assert().success();
    cgg()
        .arg(tmp.path())
        .arg("-o")
        .arg(&b)
        .arg("--no-update-check")
        .assert()
        .success();
    assert_eq!(
        std::fs::read_to_string(&a).unwrap(),
        std::fs::read_to_string(&b).unwrap(),
        "--no-update-check must be inert"
    );
}

// ---------------------------------------------------------------------
// Exit nodes
// ---------------------------------------------------------------------

#[test]
fn include_stdlib_mints_tagged_leaf_nodes() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        "s.py",
        "import os\ndef f():\n    return os.path.join('a', 'b')\n",
    );
    let out = tmp.path().join("g.mmd");
    cgg()
        .arg(tmp.path())
        .arg("-o")
        .arg(&out)
        .arg("--include-stdlib")
        .assert()
        .success();
    let g = std::fs::read_to_string(&out).unwrap();
    assert!(
        g.contains("&lt;stdlib&gt;"),
        "stdlib exit node missing:\n{g}"
    );
    assert!(
        g.contains("|std|"),
        "stdlib edge must carry the `std` tag:\n{g}"
    );
}

#[test]
fn stdlib_exit_nodes_are_off_by_default() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        "s.py",
        "import os\ndef f():\n    return os.path.join('a', 'b')\n",
    );
    let out = tmp.path().join("g.mmd");
    cgg().arg(tmp.path()).arg("-o").arg(&out).assert().success();
    let g = std::fs::read_to_string(&out).unwrap();
    assert!(
        !g.contains("stdlib"),
        "default graph must stay the direct call graph:\n{g}"
    );
}

// ---------------------------------------------------------------------
// Report-shaping flags
// ---------------------------------------------------------------------

/// Count findings in a `cgg.deadcode.v1` JSON report.
fn findings(report: &Path) -> Vec<String> {
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(report).unwrap()).unwrap();
    v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["qualified_name"].as_str().unwrap().to_string())
        .collect()
}

fn dead_code_run(dir: &Path, report: &Path, extra: &[&str]) {
    let mut c = cgg();
    c.arg(dir)
        .args(["--dead-code", "--dead-code-confidence", "low"])
        .args(["--dead-code-format", "json"])
        .arg("--dead-code-report")
        .arg(report);
    for a in extra {
        c.arg(a);
    }
    c.assert().success();
}

#[test]
fn include_tests_widens_the_report_to_test_scope() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        "m.py",
        "def helper():\n    return 1\ndef orphan():\n    return 2\n",
    );
    write(
        tmp.path(),
        "tests/test_m.py",
        "from m import helper\ndef test_helper():\n    assert helper() == 1\n",
    );

    let without = tmp.path().join("a.json");
    dead_code_run(tmp.path(), &without, &[]);
    let with = tmp.path().join("b.json");
    dead_code_run(tmp.path(), &with, &["--include-tests"]);

    let a = findings(&without);
    let b = findings(&with);
    assert!(
        a.iter().any(|f| f.ends_with("orphan")),
        "the non-test orphan is a finding either way: {a:?}"
    );
    // `helper` is called only from `tests/`, so it is categorised
    // `only-used-by-tests` and withheld until the flag asks for it.
    assert!(
        !a.iter().any(|f| f.ends_with("helper")),
        "only-used-by-tests findings are withheld by default: {a:?}"
    );
    assert!(
        b.iter().any(|f| f.ends_with("helper")),
        "--include-tests must surface only-used-by-tests findings: {b:?}"
    );
    assert!(b.len() > a.len(), "the flag widens, never narrows");
}

#[test]
fn ignore_names_suppresses_by_pattern() {
    let tmp = fixture();
    let all = tmp.path().join("a.json");
    dead_code_run(tmp.path(), &all, &[]);
    assert!(findings(&all).iter().any(|f| f.ends_with("orphan")));

    let filtered = tmp.path().join("b.json");
    dead_code_run(tmp.path(), &filtered, &["--ignore-names", "orphan$"]);
    assert!(
        !findings(&filtered).iter().any(|f| f.ends_with("orphan")),
        "--ignore-names must drop the matching finding"
    );
}

#[test]
fn ignore_attributes_suppresses_by_decorator() {
    let tmp = tempfile::tempdir().unwrap();
    // No framework import, so nothing marks this live — it is a finding
    // until the attribute pattern suppresses it.
    write(
        tmp.path(),
        "h.py",
        "@somelib.route('/x')\ndef handler():\n    return 1\n",
    );
    let all = tmp.path().join("a.json");
    dead_code_run(tmp.path(), &all, &[]);
    assert!(
        findings(&all).iter().any(|f| f.ends_with("handler")),
        "decorated handler starts out as a finding: {:?}",
        findings(&all)
    );

    let filtered = tmp.path().join("b.json");
    dead_code_run(
        tmp.path(),
        &filtered,
        &["--ignore-attributes", "glob:*route*"],
    );
    assert!(
        !findings(&filtered).iter().any(|f| f.ends_with("handler")),
        "--ignore-attributes must drop findings carrying the attribute"
    );
}

#[test]
fn ignore_file_excludes_matching_paths() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "keep.py", "def keep():\n    return 1\n");
    write(tmp.path(), "drop.py", "def drop():\n    return 2\n");
    write(tmp.path(), "extra.ignore", "drop.py\n");

    let out = tmp.path().join("g.mmd");
    cgg()
        .arg(tmp.path())
        .arg("-o")
        .arg(&out)
        .arg("--ignore-file")
        .arg(tmp.path().join("extra.ignore"))
        .assert()
        .success();
    let g = std::fs::read_to_string(&out).unwrap();
    assert!(
        g.contains("keep"),
        "unignored file must still be analyzed:\n{g}"
    );
    assert!(
        !g.contains("drop"),
        "--ignore-file must exclude the match:\n{g}"
    );
}

// ---------------------------------------------------------------------
// Roots, allow, and --why-live
// ---------------------------------------------------------------------

#[test]
fn roots_confer_liveness_transitively() {
    let tmp = fixture();
    let baseline = tmp.path().join("a.json");
    dead_code_run(tmp.path(), &baseline, &[]);
    let before = findings(&baseline);
    assert!(before.iter().any(|f| f.ends_with("entry")));

    write(tmp.path(), "roots.toml", "roots = [\"^m\\\\.entry$\"]\n");
    let after_path = tmp.path().join("b.json");
    let mut c = cgg();
    c.arg(tmp.path())
        .args(["--dead-code", "--dead-code-confidence", "low"])
        .args(["--dead-code-format", "json"])
        .arg("--dead-code-report")
        .arg(&after_path)
        .arg("--roots")
        .arg(tmp.path().join("roots.toml"));
    c.assert().success();
    let after = findings(&after_path);

    assert!(
        !after.iter().any(|f| f.ends_with("entry")),
        "a declared root is live: {after:?}"
    );
    assert!(
        !after.iter().any(|f| f.ends_with("leaf")),
        "liveness propagates through the root's callees: {after:?}"
    );
    assert!(
        after.iter().any(|f| f.ends_with("orphan")),
        "an unrelated finding must survive — roots are not a mute button: {after:?}"
    );
}

#[test]
fn allow_suppresses_without_conferring_liveness() {
    // The documented distinction: `roots` makes a callable live (and
    // everything it reaches); `[[allow]]` only hides one finding, so
    // whatever it referenced is still reported on its own merits.
    let tmp = fixture();
    write(
        tmp.path(),
        "allow.toml",
        "[[allow]]\nname = \"^m\\\\.entry$\"\nreason = \"reviewed\"\n",
    );
    let path = tmp.path().join("a.json");
    let mut c = cgg();
    c.arg(tmp.path())
        .args(["--dead-code", "--dead-code-confidence", "low"])
        .args(["--dead-code-format", "json"])
        .arg("--dead-code-report")
        .arg(&path)
        .arg("--roots")
        .arg(tmp.path().join("allow.toml"));
    c.assert().success();
    let f = findings(&path);
    assert!(
        !f.iter().any(|x| x.ends_with("entry")),
        "the accepted finding is suppressed: {f:?}"
    );
    assert!(
        f.iter().any(|x| x.ends_with("orphan")),
        "unrelated findings are untouched: {f:?}"
    );
}

#[test]
fn why_live_honours_declared_roots() {
    // REGRESSION: `run_why_live` built its options with
    // `..Default::default()`, leaving `user_roots` empty. A callable
    // the report had just proven live through a declared root answered
    // "NOT REACHED — no path from any known root" when asked why —
    // the one question whose entire purpose is to agree with the
    // report. The `cgg-frameworks` skill tells agents to use exactly
    // this command to check whether their rule worked.
    let tmp = fixture();
    write(tmp.path(), "roots.toml", "roots = [\"^m\\\\.entry$\"]\n");

    // The root itself: live in zero hops.
    cgg()
        .arg(tmp.path())
        .arg("--roots")
        .arg(tmp.path().join("roots.toml"))
        .args(["--why-live", "m.entry$"])
        .assert()
        .success()
        .stdout(predicate::str::contains("LIVE"))
        .stdout(predicate::str::contains("0 hop(s)"));

    // And what the root reaches.
    cgg()
        .arg(tmp.path())
        .arg("--roots")
        .arg(tmp.path().join("roots.toml"))
        .args(["--why-live", "m.leaf$"])
        .assert()
        .success()
        .stdout(predicate::str::contains("LIVE"))
        .stdout(predicate::str::contains("m.entry"));
}

#[test]
fn why_live_still_reports_genuinely_unreached_code() {
    // The fix above must not turn `--why-live` into a yes-machine.
    let tmp = fixture();
    write(tmp.path(), "roots.toml", "roots = [\"^m\\\\.entry$\"]\n");
    cgg()
        .arg(tmp.path())
        .arg("--roots")
        .arg(tmp.path().join("roots.toml"))
        .args(["--why-live", "m.orphan$"])
        .assert()
        .success()
        .stdout(predicate::str::contains("NOT REACHED"));
}

#[test]
fn why_live_proves_a_path_from_a_framework_entry() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        "app.py",
        concat!(
            "from flask import Flask\n",
            "app = Flask(__name__)\n\n",
            "@app.route('/users')\n",
            "def list_users():\n    return _render()\n\n",
            "def _render():\n    return 'x'\n",
        ),
    );
    cgg()
        .arg(tmp.path())
        .args(["--why-live", "_render$"])
        .assert()
        .success()
        .stdout(predicate::str::contains("LIVE"))
        .stdout(predicate::str::contains("list_users"));
}

#[test]
fn why_live_matching_nothing_says_so() {
    let tmp = fixture();
    cgg()
        .arg(tmp.path())
        .args(["--why-live", "no_such_callable_anywhere$"])
        .assert()
        .success()
        .stderr(predicate::str::contains("matched no callables"));
}

// ---------------------------------------------------------------------
// Exit codes
// ---------------------------------------------------------------------

#[test]
fn fail_on_dead_exits_3_when_the_report_is_non_empty() {
    let tmp = fixture();
    cgg()
        .arg(tmp.path())
        .args([
            "--dead-code",
            "--dead-code-confidence",
            "low",
            "--fail-on-dead",
        ])
        .assert()
        .code(3);
}

#[test]
fn fail_on_dead_exits_0_when_the_report_is_empty() {
    let tmp = tempfile::tempdir().unwrap();
    // Every callable reachable from a declared root, so nothing is left
    // to report and the gate must stay green.
    write(
        tmp.path(),
        "m.py",
        "def leaf():\n    return 1\ndef entry():\n    return leaf()\n",
    );
    write(tmp.path(), "roots.toml", "roots = [\"^m\\\\.entry$\"]\n");
    cgg()
        .arg(tmp.path())
        .args([
            "--dead-code",
            "--dead-code-confidence",
            "low",
            "--fail-on-dead",
        ])
        .arg("--roots")
        .arg(tmp.path().join("roots.toml"))
        .assert()
        .success();
}

#[test]
fn dead_code_without_fail_on_dead_stays_green() {
    // Findings alone must never change the exit status.
    let tmp = fixture();
    cgg()
        .arg(tmp.path())
        .args(["--dead-code", "--dead-code-confidence", "low"])
        .assert()
        .success();
}

#[test]
fn invalid_arguments_exit_2() {
    cgg().arg("--not-a-real-flag").assert().code(2);
}

// ---------------------------------------------------------------------
// Framework coverage
// ---------------------------------------------------------------------

#[test]
fn framework_coverage_forces_the_table_when_nothing_matched() {
    let tmp = fixture();
    let out = tmp.path().join("g.mmd");
    cgg()
        .arg(tmp.path())
        .arg("-o")
        .arg(&out)
        .arg("--framework-coverage")
        .assert()
        .success()
        .stderr(predicate::str::contains("framework coverage"))
        .stderr(predicate::str::contains("recognised"));
}

#[test]
fn the_coverage_table_stays_quiet_when_there_is_nothing_to_say() {
    let tmp = fixture();
    let out = tmp.path().join("g.mmd");
    cgg()
        .arg(tmp.path())
        .arg("-o")
        .arg(&out)
        .assert()
        .success()
        .stderr(predicate::str::contains("framework coverage").not());
}

#[test]
fn a_recognised_framework_reports_its_entry_count() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        "app.py",
        concat!(
            "from flask import Flask\n",
            "app = Flask(__name__)\n\n",
            "@app.route('/users')\n",
            "def list_users():\n    return 'x'\n",
        ),
    );
    let out = tmp.path().join("g.mmd");
    cgg()
        .arg(tmp.path())
        .arg("-o")
        .arg(&out)
        .assert()
        .success()
        .stderr(predicate::str::contains("flask (network, 1 entry)"))
        .stderr(predicate::str::contains("INFERRED, not observed"));
    let g = std::fs::read_to_string(&out).unwrap();
    assert!(
        g.contains("framework-entry"),
        "entry node must be in the graph:\n{g}"
    );
    assert!(
        g.contains("|entry|"),
        "entry edge must carry the `entry` tag:\n{g}"
    );
}

#[test]
fn no_entry_nodes_restores_the_plain_call_graph() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        "app.py",
        concat!(
            "from flask import Flask\n",
            "app = Flask(__name__)\n\n",
            "@app.route('/users')\n",
            "def list_users():\n    return 'x'\n",
        ),
    );
    let out = tmp.path().join("g.mmd");
    cgg()
        .arg(tmp.path())
        .arg("-o")
        .arg(&out)
        .arg("--no-entry-nodes")
        .assert()
        .success();
    let g = std::fs::read_to_string(&out).unwrap();
    assert!(
        !g.contains("framework-entry"),
        "--no-entry-nodes must suppress them:\n{g}"
    );
    assert!(g.contains("list_users"), "the handler itself stays:\n{g}");
}

// ---------------------------------------------------------------------
// --since
// ---------------------------------------------------------------------

fn git(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git runs")
        .status
        .success();
    assert!(ok, "git {args:?} failed");
}

#[test]
fn since_seeds_the_filter_from_a_revspec() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    git(d, &["init", "-q", "."]);
    git(d, &["config", "user.email", "t@example.com"]);
    git(d, &["config", "user.name", "t"]);
    write(
        d,
        "m.py",
        "def a():\n    return 1\ndef b():\n    return 2\n",
    );
    git(d, &["add", "-A"]);
    git(d, &["commit", "-qm", "one"]);
    // Only `a` changes.
    write(
        d,
        "m.py",
        "def a():\n    return 99\ndef b():\n    return 2\n",
    );
    git(d, &["add", "-A"]);
    git(d, &["commit", "-qm", "two"]);

    let out = d.join("g.mmd");
    cgg()
        .arg(d)
        .arg("-o")
        .arg(&out)
        .args(["--since", "HEAD~1..HEAD"])
        .assert()
        .success()
        .stderr(predicate::str::contains("1 callable seed(s)"));

    let g = std::fs::read_to_string(&out).unwrap();
    assert!(g.contains("m.a"), "the touched callable is the seed:\n{g}");
    assert!(
        !g.contains("m.b"),
        "an untouched callable must not be seeded:\n{g}"
    );

    let audit: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(d.join("g.mmd.audit.json")).unwrap(),
    )
    .unwrap();
    let ev = audit
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["event"] == "since_resolved")
        .expect("since_resolved event recorded");
    assert_eq!(ev["revspec"], "HEAD~1..HEAD");
    assert_eq!(ev["matched_seeds"][0], "m.a");
}

#[test]
fn since_outside_a_repository_is_a_clear_error() {
    let tmp = fixture();
    cgg()
        .arg(tmp.path())
        .args(["--since", "HEAD~1..HEAD"])
        .assert()
        .failure();
}
