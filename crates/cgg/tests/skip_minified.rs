//! `--skip-minified` end-to-end: the flag is off by default, it only ever
//! removes files, and every file it removes is accounted for in the audit.
//!
//! The walker's own unit tests (`cgg-walk`) cover the heuristic — a
//! `name.min.<ext>` filename and the average-line-length probe. These cover
//! what the heuristic *means for the graph*, which is the part a resolver
//! change could quietly break without touching `cgg-walk` at all.
//!
//! Deliberately NOT asserted here: that the edge set is unchanged for the
//! files that survive. It is not, and that is correct rather than a defect —
//! dropping a file drops fan-out candidates with it, so a call site that
//! previously exceeded `--fanout-cap` and emitted nothing can fall under the
//! cap and resolve. Measured on the corpus at 0.8.4: `app-wordpress` gains
//! 55 edges among surviving files and `app-calcom-nextjs` loses one. The
//! *callable* set of the surviving files is the invariant, and that is what
//! `callables_of_surviving_files_are_untouched` pins.

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

/// A tree with one minified file caught by NAME, one caught by the
/// average-line-length probe, and ordinary source that must survive both
/// ways — including a `.js` file, so a failure cannot be explained away as
/// "the whole language was dropped".
fn fixture() -> TempDir {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "assets/bundle.min.js",
        b"function m(){return 1}\n",
    );
    // One line, comfortably over the 2,000-byte average-line threshold.
    let mut long = Vec::new();
    long.extend_from_slice(b"function packed(){return ");
    long.extend(std::iter::repeat_n(b'0', 3000));
    long.extend_from_slice(b"}\n");
    write(tmp.path(), "assets/packed.js", &long);
    write(
        tmp.path(),
        "src/app.js",
        b"function helper() {\n  return 1;\n}\nfunction main() {\n  return helper();\n}\n",
    );
    write(
        tmp.path(),
        "src/lib.py",
        b"def inner():\n    return 1\n\n\ndef outer():\n    return inner()\n",
    );
    tmp
}

fn graph(tmp: &TempDir, extra: &[&str]) -> serde_json::Value {
    let out = cgg()
        .arg(tmp.path())
        .args(["-t", "json", "--jobs", "1"])
        .args(extra)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("graph is JSON")
}

/// Paths the run actually analyzed, relative to the tree root.
fn analyzed(g: &serde_json::Value) -> Vec<String> {
    let mut v: Vec<String> = g["files"]
        .as_object()
        .expect("files map")
        .values()
        .filter_map(|f| f["path"].as_str())
        .map(|p| {
            // Keep the tail, so a temp-dir prefix does not leak into the
            // assertion messages.
            p.rsplit_once("/src/")
                .map(|(_, t)| format!("src/{t}"))
                .or_else(|| {
                    p.rsplit_once("/assets/")
                        .map(|(_, t)| format!("assets/{t}"))
                })
                .unwrap_or_else(|| p.to_string())
        })
        .collect();
    v.sort();
    v
}

#[test]
fn minified_files_are_analyzed_unless_the_flag_is_passed() {
    let tmp = fixture();
    let files = analyzed(&graph(&tmp, &[]));
    for want in ["assets/bundle.min.js", "assets/packed.js"] {
        assert!(
            files.iter().any(|f| f == want),
            "{want} must be analyzed by default — the default walk analyzes \
             every file it can parse; got {files:?}"
        );
    }
}

#[test]
fn the_flag_removes_minified_files_and_nothing_else() {
    let tmp = fixture();
    let before = analyzed(&graph(&tmp, &[]));
    let after = analyzed(&graph(&tmp, &["--skip-minified"]));

    for gone in ["assets/bundle.min.js", "assets/packed.js"] {
        assert!(
            !after.iter().any(|f| f == gone),
            "{gone} should be skipped under --skip-minified; got {after:?}"
        );
    }
    for kept in ["src/app.js", "src/lib.py"] {
        assert!(
            after.iter().any(|f| f == kept),
            "{kept} is ordinary source and must survive --skip-minified; \
             got {after:?}"
        );
    }
    // The flag subtracts. It must never introduce a file the default walk
    // did not already have.
    for f in &after {
        assert!(
            before.contains(f),
            "--skip-minified analyzed {f}, which the default run did not — \
             the flag may only remove files"
        );
    }
}

#[test]
fn callables_of_surviving_files_are_untouched() {
    let tmp = fixture();
    let before = graph(&tmp, &[]);
    let after = graph(&tmp, &["--skip-minified"]);

    // Identify callables by (file path, start byte) so the comparison does
    // not depend on ids, which are content hashes and would differ anyway.
    let names = |g: &serde_json::Value| -> Vec<String> {
        let files = g["files"].as_object().unwrap().clone();
        let mut v: Vec<String> = g["callables"]
            .as_object()
            .unwrap()
            .values()
            .map(|c| {
                let p = files[c["file"].as_str().unwrap()]["path"]
                    .as_str()
                    .unwrap()
                    .to_string();
                format!("{p}#{}", c["start_byte"])
            })
            .collect();
        v.sort();
        v
    };
    let kept: Vec<String> = analyzed(&after);
    let survives = |q: &String| kept.iter().any(|k| q.contains(k.as_str()));

    let a: Vec<String> = names(&before).into_iter().filter(survives).collect();
    let b: Vec<String> = names(&after).into_iter().filter(survives).collect();
    assert_eq!(
        a, b,
        "dropping a minified file must not change which callables are \
         extracted from the files that remain"
    );
    assert!(!a.is_empty(), "fixture should yield callables to compare");
}

#[test]
fn every_skipped_file_is_named_in_the_audit() {
    let tmp = fixture();
    let metrics = tmp.path().join("run.json");
    cgg()
        .arg(tmp.path())
        .args(["--skip-minified", "--jobs", "1"])
        .args(["--metrics", metrics.to_str().unwrap()])
        .assert()
        .success();

    let text = fs::read_to_string(&metrics).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
    let blob = doc.to_string();
    assert!(
        blob.contains("minified"),
        "the audit must record why a file was dropped, not drop it silently"
    );
    for want in ["bundle.min.js", "packed.js"] {
        assert!(
            blob.contains(want),
            "{want} was skipped but is not named anywhere in the audit"
        );
    }
}
