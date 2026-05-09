//! Task 6 integration tests: cross-file resolution for Python.

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
fn python_cross_file_import_resolves() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "helpers.py",
        b"def greet(name):\n    return name\n\ndef compute(x):\n    return x * 2\n",
    );
    write(
        tmp.path(),
        "main.py",
        b"from helpers import greet, compute\n\ndef process(name, x):\n    msg = greet(name)\n    return compute(x)\n",
    );

    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["-t", "mermaid", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();

    let g = fs::read_to_string(&mmd).unwrap();
    // Must include the process -> greet and process -> compute
    // cross-file edges.
    assert!(g.contains("main.process"), "mermaid:\n{g}");
    assert!(g.contains("helpers.greet"), "mermaid:\n{g}");
    assert!(g.contains("helpers.compute"), "mermaid:\n{g}");
    // Three callables minimum; at least two arrows targeting helpers.*.
    let arrow_lines: Vec<&str> = g.lines().filter(|l| l.contains(" --> ")).collect();
    assert!(
        arrow_lines.len() >= 2,
        "expected at least two edges, got {} in:\n{g}",
        arrow_lines.len()
    );
}

#[test]
fn python_module_alias_chain_resolves() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "stats.py",
        b"def average(xs):\n    return sum(xs) / len(xs)\n",
    );
    write(
        tmp.path(),
        "main.py",
        b"import stats as s\n\ndef run(xs):\n    return s.average(xs)\n",
    );

    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["-t", "mermaid", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();

    let g = fs::read_to_string(&mmd).unwrap();
    assert!(g.contains("main.run"));
    assert!(g.contains("stats.average"));
    // Expect at least one directed arrow.
    assert!(g.lines().any(|l| l.contains(" --> ")));
}

#[test]
fn audit_records_medium_confidence_for_cross_file() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "lib.py",
        b"def work():\n    return 1\n",
    );
    write(
        tmp.path(),
        "caller.py",
        b"from lib import work\n\ndef main():\n    return work()\n",
    );

    let audit = tmp.path().join("run.json");
    cgg()
        .args(["--audit-format", "json", "--metrics"])
        .arg(&audit)
        .arg(tmp.path())
        .assert()
        .success();

    let text = fs::read_to_string(&audit).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    let arr = parsed.as_array().unwrap();
    let finished = arr.iter().find(|e| e["event"] == "run_finished").unwrap();
    let confidence = &finished["metrics"]["confidence_histogram"];
    // At least one medium-confidence edge (cross-file).
    assert!(
        confidence["medium"].as_u64().unwrap_or(0) >= 1,
        "expected medium-confidence edge: {confidence}"
    );
}

#[test]
fn rust_cross_crate_use_resolves() {
    // Two-crate workspace; upstream defines `helper`, downstream
    // calls it via `use upstream::helper`.
    let tmp = TempDir::new().unwrap();

    // Workspace root
    write(
        tmp.path(),
        "Cargo.toml",
        br#"[workspace]
members = ["upstream", "downstream"]
"#,
    );

    // Upstream crate
    write(
        tmp.path(),
        "upstream/Cargo.toml",
        br#"[package]
name = "upstream"
version = "0.0.0"
edition = "2021"
"#,
    );
    write(
        tmp.path(),
        "upstream/src/lib.rs",
        b"pub fn helper() -> u32 { 42 }\npub fn unused() {}\n",
    );

    // Downstream crate
    write(
        tmp.path(),
        "downstream/Cargo.toml",
        br#"[package]
name = "downstream"
version = "0.0.0"
edition = "2021"
"#,
    );
    write(
        tmp.path(),
        "downstream/src/lib.rs",
        b"use upstream::helper;\n\npub fn caller() -> u32 { helper() }\n",
    );

    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["-t", "mermaid", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();

    let g = fs::read_to_string(&mmd).unwrap();
    // Both crates must have their callables extracted with the
    // crate-prefixed qualified name.
    assert!(g.contains("upstream::helper"), "missing upstream::helper in:\n{g}");
    assert!(
        g.contains("downstream::caller"),
        "missing downstream::caller in:\n{g}"
    );
    // And a cross-crate edge downstream::caller -> upstream::helper.
    // Find the node ids and verify the arrow.
    let node_id = |qn: &str| {
        g.lines()
            .find_map(|l| {
                let l = l.trim();
                if l.starts_with('C') && l.contains(&format!("[\"{qn}\"]")) {
                    Some(l.split('[').next()?.trim().to_string())
                } else {
                    None
                }
            })
    };
    let caller = node_id("downstream::caller").expect("node");
    let helper = node_id("upstream::helper").expect("node");
    let arrow = format!("{caller} --> {helper}");
    assert!(g.contains(&arrow), "missing edge {arrow} in:\n{g}");
}

#[test]
fn rust_pub_use_reexport_chains() {
    // `facade` re-exports `core_::work` as its own symbol; a caller
    // that imports `facade::work` should still resolve to the
    // original definition in `core_`.
    let tmp = TempDir::new().unwrap();

    write(
        tmp.path(),
        "Cargo.toml",
        br#"[workspace]
members = ["core_", "facade", "caller"]
"#,
    );

    write(
        tmp.path(),
        "core_/Cargo.toml",
        br#"[package]
name = "core_"
version = "0.0.0"
edition = "2021"
"#,
    );
    write(
        tmp.path(),
        "core_/src/lib.rs",
        b"pub fn work() -> i32 { 7 }\n",
    );

    write(
        tmp.path(),
        "facade/Cargo.toml",
        br#"[package]
name = "facade"
version = "0.0.0"
edition = "2021"
"#,
    );
    write(
        tmp.path(),
        "facade/src/lib.rs",
        b"pub use core_::work;\n",
    );

    write(
        tmp.path(),
        "caller/Cargo.toml",
        br#"[package]
name = "caller"
version = "0.0.0"
edition = "2021"
"#,
    );
    write(
        tmp.path(),
        "caller/src/lib.rs",
        b"use facade::work;\n\npub fn run() -> i32 { work() }\n",
    );

    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["-t", "mermaid", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();

    let g = fs::read_to_string(&mmd).unwrap();
    // caller::run -> core_::work  (through the facade re-export).
    assert!(g.contains("core_::work"), "missing core_::work in:\n{g}");
    assert!(g.contains("caller::run"), "missing caller::run in:\n{g}");
    let node_id = |qn: &str| {
        g.lines().find_map(|l| {
            let l = l.trim();
            if l.starts_with('C') && l.contains(&format!("[\"{qn}\"]")) {
                Some(l.split('[').next()?.trim().to_string())
            } else {
                None
            }
        })
    };
    let run = node_id("caller::run").expect("node");
    let work = node_id("core_::work").expect("node");
    let arrow = format!("{run} --> {work}");
    assert!(g.contains(&arrow), "re-export chain missing edge {arrow}:\n{g}");
}
