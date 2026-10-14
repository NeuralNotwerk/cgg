//! Task 2 integration tests: end-to-end walker + JSONL audit.

use assert_cmd::Command;
use std::fs;
use std::io::Write;
use std::path::Path;
use tempfile::TempDir;

fn cgg() -> Command {
    Command::cargo_bin("cgg").expect("cgg binary built")
}

fn write(dir: &Path, name: &str, body: &[u8]) {
    let p = dir.join(name);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::File::create(&p).unwrap().write_all(body).unwrap();
}

#[test]
fn jsonl_audit_streams_events() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "src/a.py", b"print('hi')\n");
    write(tmp.path(), "src/b.rs", b"fn main() {}\n");
    write(tmp.path(), "node_modules/c.js", b"module.exports={};\n");
    write(tmp.path(), ".cggignore", b"ignored.py\n");
    write(tmp.path(), "ignored.py", b"pass\n");

    let out = tmp.path().join("run.jsonl");

    cgg()
        .args(["--audit-format", "jsonl"])
        .args(["--metrics", out.to_str().unwrap()])
        .arg(tmp.path())
        .assert()
        .success();

    let text = fs::read_to_string(&out).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    // At minimum: run_started + some file events + run_finished.
    assert!(lines.len() >= 5, "lines: {}", lines.len());
    assert!(lines[0].contains("run_started"));
    assert!(lines.last().unwrap().contains("run_finished"));
    // Built-in deny on node_modules.
    assert!(text.contains("file_skipped"));
    assert!(text.contains("\"kind\":\"builtin\""));
    // .cggignore was honored.
    assert!(!text.contains("\"ignored.py\""));
    // Each line should parse as a JSON object.
    for l in &lines {
        let _: serde_json::Value = serde_json::from_str(l)
            .unwrap_or_else(|e| panic!("not valid JSON on line: {l}\n{e}"));
    }
}

#[test]
fn json_audit_is_single_pretty_document() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "a.py", b"print('hi')\n");
    let out = tmp.path().join("run.json");

    cgg()
        .args(["--audit-format", "json"])
        .args(["--metrics", out.to_str().unwrap()])
        .arg(tmp.path())
        .assert()
        .success();

    let text = fs::read_to_string(&out).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(parsed.is_array());
    let arr = parsed.as_array().unwrap();
    assert!(arr.len() >= 3);
    // First event must be run_started, last must be run_finished.
    assert_eq!(arr[0]["event"], "run_started");
    assert_eq!(arr.last().unwrap()["event"], "run_finished");
}

#[test]
fn skip_reasons_are_audit_visible() {
    let tmp = TempDir::new().unwrap();
    // Binary file.
    write(tmp.path(), "blob.dat", b"head\x00tail");
    // node_modules.
    write(tmp.path(), "node_modules/m.js", b"exports={};");
    // Regular file kept.
    write(tmp.path(), "keep.py", b"pass\n");

    let out = tmp.path().join("audit.jsonl");

    cgg()
        .args(["--audit-format", "jsonl"])
        .args(["--metrics", out.to_str().unwrap()])
        .arg(tmp.path())
        .assert()
        .success();

    let text = fs::read_to_string(&out).unwrap();
    let has_binary_skip = text
        .lines()
        .any(|l| l.contains("file_skipped") && l.contains("\"kind\":\"binary\""));
    let has_builtin_skip = text.lines().any(|l| {
        l.contains("file_skipped")
            && l.contains("\"kind\":\"builtin\"")
            && l.contains("node_modules")
    });
    let has_keep = text
        .lines()
        .any(|l| l.contains("file_discovered") && l.contains("keep.py"));
    assert!(has_binary_skip, "no binary skip in:\n{text}");
    assert!(has_builtin_skip, "no builtin skip in:\n{text}");
    assert!(has_keep, "no keep.py discovery in:\n{text}");
}
