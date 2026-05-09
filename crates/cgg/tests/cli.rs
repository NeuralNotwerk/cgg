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
        .stderr(predicate::str::contains(
            "input path does not exist",
        ));
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
