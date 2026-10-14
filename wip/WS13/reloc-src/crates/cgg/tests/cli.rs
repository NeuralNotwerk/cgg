//! End-to-end CLI tests for Task 1.
//!
//! These exercise the compiled `cgg` binary to guard the invariants:
//!
//! * `--help` prints and exits 0.
//! * No arguments prints help and exits non-zero.
//! * A non-existent path yields a clear error and non-zero exit.
//! * A valid path parses and completes the Task 1 placeholder run.

use assert_cmd::Command;
use predicates::prelude::*;

fn cgg() -> Command {
    Command::cargo_bin("cgg").expect("cgg binary built")
}

#[test]
fn help_succeeds_and_mentions_flags() {
    cgg()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--filter"))
        .stdout(predicate::str::contains("--audit-format"))
        .stdout(predicate::str::contains("-n"))
        .stdout(predicate::str::contains("mermaid"))
        .stdout(predicate::str::contains("graphml"));
}

#[test]
fn missing_positional_is_error() {
    cgg().assert().failure();
}

#[test]
fn nonexistent_path_is_clear_error() {
    cgg()
        .arg("/tmp/definitely-not-a-real-path-9a8b7c6d")
        .assert()
        .failure()
        .stderr(predicate::str::contains("input path does not exist"));
}

#[test]
fn valid_path_runs_placeholder() {
    let tmp = tempfile::tempdir().unwrap();
    cgg()
        .arg(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("cgg:"));
}

#[test]
fn bad_format_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    cgg()
        .args(["-t", "yaml"])
        .arg(tmp.path())
        .assert()
        .failure();
}

/// A tree with one live function, one unreferenced function, and a
/// deep-enough call chain to enumerate more than one path.
fn fixture() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("m.py"),
        concat!(
            "def helper():\n    return 1\n\n",
            "def caller():\n    return helper()\n\n",
            "def other():\n    return helper()\n\n",
            "def genuinely_unused():\n    return 2\n",
        ),
    )
    .unwrap();
    tmp
}

#[test]
fn dead_code_without_o_writes_the_report_to_stderr() {
    // Regression: `cgg <path> --dead-code` is the documented way to get
    // the ranked report, but with the graph on stdout there was no
    // sidecar path to derive, and the report was dropped — leaving only
    // a one-line summary. Silently producing less than advertised.
    let tmp = fixture();
    cgg()
        .arg(tmp.path())
        .args(["--dead-code", "--dead-code-confidence", "low"])
        .assert()
        .success()
        .stdout(predicate::str::contains("flowchart LR"))
        .stderr(predicate::str::contains("cgg dead-code report"))
        .stderr(predicate::str::contains("genuinely_unused"))
        .stderr(predicate::str::contains("EVERY FINDING IS A HYPOTHESIS"));
}

#[test]
fn dead_code_json_without_a_destination_fails_rather_than_discarding_it() {
    // JSON on stderr would interleave with the summary lines and parse
    // as nothing, so this case names the fix instead of half-doing it.
    //
    // It exited 0 until 0.6.6 — a silent-failure trap for scripted use,
    // where the note landed on stderr nobody was reading and the report
    // was simply dropped.
    let tmp = fixture();
    cgg()
        .arg(tmp.path())
        .args(["--dead-code", "--dead-code-format", "json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--dead-code-report"));
}

#[test]
fn no_graph_gives_the_json_report_stdout() {
    // The natural destination once the graph is out of the way, and the
    // reason the failure above is safe to introduce: there is a way to
    // ask for exactly this.
    let tmp = fixture();
    cgg()
        .arg(tmp.path())
        .args(["--dead-code", "--dead-code-format", "json", "--no-graph"])
        .assert()
        .success()
        .stdout(predicate::str::contains("cgg.deadcode.v1"))
        .stdout(predicate::str::contains("flowchart").not());
}

#[test]
fn the_report_sidecar_extension_follows_the_format() {
    // Regression: the ranked *text* report was written to a file named
    // `.deadcode.json`, which breaks every consumer that trusts a
    // suffix.
    let tmp = fixture();
    let out = tmp.path().join("g.mmd");
    cgg()
        .arg(tmp.path())
        .arg("-o")
        .arg(&out)
        .args(["--dead-code", "--dead-code-confidence", "low"])
        .assert()
        .success();
    let text = tmp.path().join("g.mmd.deadcode.txt");
    assert!(text.exists(), "text format must produce a .txt sidecar");
    assert!(
        !tmp.path().join("g.mmd.deadcode.json").exists(),
        "text format must not produce a .json sidecar"
    );
    assert!(
        std::fs::read_to_string(&text)
            .unwrap()
            .contains("cgg dead-code report")
    );

    let out2 = tmp.path().join("h.mmd");
    cgg()
        .arg(tmp.path())
        .arg("-o")
        .arg(&out2)
        .args(["--dead-code", "--dead-code-format", "json"])
        .assert()
        .success();
    let json = tmp.path().join("h.mmd.deadcode.json");
    assert!(json.exists(), "json format must produce a .json sidecar");
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
    assert_eq!(parsed["schema"], "cgg.deadcode.v1");
}

#[test]
fn write_roots_does_not_need_dead_code_spelled_out() {
    // Regression: `--write-roots` alone fell through to the ordinary
    // graph path and emitted mermaid — a silent no-op wearing the
    // costume of a baseline.
    let tmp = fixture();
    cgg()
        .arg(tmp.path())
        .arg("--write-roots")
        .assert()
        .success()
        .stdout(predicate::str::contains("cgg dead-code configuration"))
        .stdout(predicate::str::contains("roots = ["))
        .stdout(predicate::str::contains("flowchart").not());
}

#[test]
fn max_paths_truncation_is_announced() {
    // Regression: `-n 0 --max-paths N` stopped enumerating silently, so
    // a capped path set was indistinguishable from a complete one. Both
    // the stderr note and the audit event have to say so.
    let tmp = fixture();
    let out = tmp.path().join("g.mmd");
    cgg()
        .arg(tmp.path())
        .arg("-o")
        .arg(&out)
        .args(["--filter", "helper", "-n", "0", "--max-paths", "1"])
        .assert()
        .success()
        .stderr(predicate::str::contains("--max-paths 1"));

    let audit = std::fs::read_to_string(tmp.path().join("g.mmd.audit.json")).unwrap();
    assert!(
        audit.contains("paths_truncated"),
        "truncation must be recorded in the audit trail, not just on stderr"
    );
}

#[test]
fn an_uncapped_run_stays_quiet_about_truncation() {
    let tmp = fixture();
    let out = tmp.path().join("g.mmd");
    cgg()
        .arg(tmp.path())
        .arg("-o")
        .arg(&out)
        .args(["--filter", "helper", "-n", "0"])
        .assert()
        .success()
        .stderr(predicate::str::contains("--max-paths").not());
    let audit = std::fs::read_to_string(tmp.path().join("g.mmd.audit.json")).unwrap();
    assert!(
        !audit.contains("paths_truncated"),
        "no cap was hit; nothing to report"
    );
}

#[test]
fn report_unreferenced_lists_what_nothing_points_at() {
    // The case that prompted the mode: a class documented as the
    // contract between two pipeline stages and imported by nothing,
    // where `--dead-code`'s cascade buried the signal under inherited
    // doubt from an unrooted framework handler.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("wire.py"),
        b"class Envelope:\n    def to_dict(self):\n        return {}\n" as &[u8],
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("stage.py"),
        b"def emit(x):\n    return x\ndef run():\n    return emit(1)\n" as &[u8],
    )
    .unwrap();

    cgg()
        .arg(tmp.path())
        .arg("--report-unreferenced")
        .assert()
        .success()
        // The finding.
        .stdout(predicate::str::contains("Envelope.to_dict"))
        // `emit` has a caller, so it is not one.
        .stdout(predicate::str::contains("stage.emit").not())
        // And it replaces the graph rather than adding to it.
        .stdout(predicate::str::contains("flowchart").not());
}
