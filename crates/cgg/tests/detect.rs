//! Task 3 integration tests: language detection + parser pool end-to-end.
//!
//! Builds a mixed-language fixture, runs `cgg` against it with the JSONL
//! audit format, and asserts the per-file `file_analyzed` records carry
//! the right `language` and `detected_via`.

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

/// Iterate every JSONL event.
fn parse_jsonl(text: &str) -> Vec<serde_json::Value> {
    text.lines()
        .map(|l| serde_json::from_str(l).expect("jsonl line must parse"))
        .collect()
}

#[test]
fn nine_language_sample_all_detect() {
    let tmp = TempDir::new().unwrap();
    // Minimal samples per v1 language.
    write(tmp.path(), "r.rs", b"fn main(){}\n");
    write(tmp.path(), "p.py", b"def f():\n    pass\n");
    write(tmp.path(), "j.js", b"function f(){}\n");
    write(tmp.path(), "t.ts", b"function f():void{}\n");
    write(tmp.path(), "g.go", b"package main\nfunc main(){}\n");
    write(tmp.path(), "J.java", b"class A{void m(){}}\n");
    write(tmp.path(), "c1.c", b"int main(){return 0;}\n");
    write(tmp.path(), "cpp1.cpp", b"int main(){return 0;}\n");
    write(tmp.path(), "cs1.cs", b"class A{void M(){}}\n");

    let out = tmp.path().join("run.jsonl");
    cgg()
        .args(["--audit-format", "jsonl", "--metrics"])
        .arg(&out)
        .arg(tmp.path())
        .assert()
        .success();

    let text = fs::read_to_string(&out).unwrap();
    let events = parse_jsonl(&text);

    let analyzed: std::collections::HashMap<String, String> = events
        .iter()
        .filter(|e| e["event"] == "file_analyzed")
        .map(|e| {
            let path = e["path"].as_str().unwrap().to_string();
            let lang = e["language"].as_str().unwrap().to_string();
            (path, lang)
        })
        .collect();

    let expected = [
        ("r.rs", "rust"),
        ("p.py", "python"),
        ("j.js", "javascript"),
        ("t.ts", "typescript"),
        ("g.go", "go"),
        ("J.java", "java"),
        ("c1.c", "c"),
        ("cpp1.cpp", "cpp"),
        ("cs1.cs", "csharp"),
    ];
    for (name, expected_lang) in expected {
        let path_str = tmp
            .path()
            .join(name)
            .to_string_lossy()
            .to_string();
        let actual = analyzed
            .get(&path_str)
            .unwrap_or_else(|| panic!("no file_analyzed for {name}"));
        assert_eq!(actual, expected_lang, "{name} language");
    }
}

#[test]
fn header_disambiguation_prefers_cpp_when_sibling_exists() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "x.h", b"#pragma once\nvoid f();\n");
    write(tmp.path(), "x.cpp", b"#include \"x.h\"\nvoid f(){}\n");
    let out = tmp.path().join("run.jsonl");
    cgg()
        .args(["--audit-format", "jsonl", "--metrics"])
        .arg(&out)
        .arg(tmp.path())
        .assert()
        .success();
    let text = fs::read_to_string(&out).unwrap();
    // Every .h and .cpp in the fixture should be cpp (the .h sibling
    // triggers the header-heuristic:cpp path; .cpp is just cpp).
    for event in parse_jsonl(&text) {
        if event["event"] == "file_analyzed" {
            let path = event["path"].as_str().unwrap();
            if path.ends_with(".h") {
                assert_eq!(event["language"], "cpp");
                assert_eq!(event["detected_via"], "header-heuristic:cpp");
            }
            if path.ends_with(".cpp") {
                assert_eq!(event["language"], "cpp");
                assert_eq!(event["detected_via"], "extension:.cpp");
            }
        }
    }
}

#[test]
fn shebang_beats_extension() {
    let tmp = TempDir::new().unwrap();
    // No extension; shebang forces python.
    write(tmp.path(), "tool", b"#!/usr/bin/env python3\nprint('hi')\n");
    // Unknown extension (.doc); shebang forces node.
    write(
        tmp.path(),
        "thing.doc",
        b"#!/usr/bin/env node\nconsole.log(1)\n",
    );
    let out = tmp.path().join("run.jsonl");
    cgg()
        .args(["--audit-format", "jsonl", "--metrics"])
        .arg(&out)
        .arg(tmp.path())
        .assert()
        .success();
    let text = fs::read_to_string(&out).unwrap();
    let events = parse_jsonl(&text);
    let mut got_python = false;
    let mut got_node = false;
    for e in events {
        if e["event"] == "file_analyzed" {
            if e["path"].as_str().unwrap().ends_with("tool") {
                assert_eq!(e["language"], "python");
                assert_eq!(e["detected_via"], "shebang:python3");
                got_python = true;
            }
            if e["path"].as_str().unwrap().ends_with("thing.doc") {
                assert_eq!(e["language"], "javascript");
                assert_eq!(e["detected_via"], "shebang:node");
                got_node = true;
            }
        }
    }
    assert!(got_python && got_node);
}

#[test]
fn unknown_extension_skip_appears_in_audit() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "notes.txt", b"just notes\n");
    write(tmp.path(), "a.py", b"pass\n");
    let out = tmp.path().join("run.jsonl");
    cgg()
        .args(["--audit-format", "jsonl", "--metrics"])
        .arg(&out)
        .arg(tmp.path())
        .assert()
        .success();
    let text = fs::read_to_string(&out).unwrap();
    assert!(text.contains("\"kind\":\"unknown-extension\""));
    assert!(text.contains("notes.txt"));
}

#[test]
fn lang_filter_excludes_and_reports() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "a.py", b"pass\n");
    write(tmp.path(), "b.rs", b"fn main(){}\n");
    let out = tmp.path().join("run.jsonl");
    cgg()
        .args(["--audit-format", "jsonl", "--lang", "python", "--metrics"])
        .arg(&out)
        .arg(tmp.path())
        .assert()
        .success();
    let text = fs::read_to_string(&out).unwrap();
    let events = parse_jsonl(&text);
    // a.py analyzed, b.rs skipped with lang-filter reason.
    let py_analyzed = events.iter().any(|e| {
        e["event"] == "file_analyzed"
            && e["language"] == "python"
            && e["path"].as_str().unwrap().ends_with("a.py")
    });
    // b.rs is skipped with the dedicated LanguageFilter("rust")
    // variant (kind=language-filter, detail="rust") rather than the
    // old overloaded Builtin("lang-filter:rust").
    let rs_skipped = events.iter().any(|e| {
        e["event"] == "file_skipped"
            && e["path"].as_str().unwrap().ends_with("b.rs")
            && e["reason"]["kind"] == "language-filter"
            && e["reason"]["detail"] == "rust"
    });
    assert!(py_analyzed, "python not analyzed");
    assert!(rs_skipped, "rust not lang-filtered");
}
