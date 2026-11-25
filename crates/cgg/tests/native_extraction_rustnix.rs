//! Extraction regressions for Nix attribute-path qualified names and
//! Rust function values outside call arguments (0.14.0, packet B5).

use std::fs;
use std::path::Path;

use assert_cmd::prelude::*;
use std::process::Command;
use tempfile::TempDir;

fn write(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, body).unwrap();
}

#[test]
fn nix_nested_bindings_are_named_by_their_attribute_path() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "tests/default.nix",
        "{ pkgs }: pkgs.testers.runNixOSTest {\n\
         \x20 name = \"t\";\n\
         \x20 nodes = {\n\
         \x20   machine = { pkgs, ... }: { };\n\
         \x20 };\n\
         \x20 nodes.other = { ... }: { };\n\
         \x20 testScript = { nodes, ... }: \"start_all()\";\n\
         }\n",
    );
    let out = Command::cargo_bin("cgg")
        .unwrap()
        .args(["-t", "json"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json = String::from_utf8_lossy(&out);
    assert!(json.contains("\"nodes.machine\""), "{json}");
    assert!(json.contains("\"nodes.other\""), "{json}");
    assert!(json.contains("\"testScript\""), "{json}");
}

/// stderr of a run over `dir` with a user rule file.
fn run_with_roots(dir: &Path, toml: &str) -> String {
    let rules = dir.join("rules.toml");
    fs::write(&rules, toml).unwrap();
    let out = Command::cargo_bin("cgg")
        .unwrap()
        .args(["--no-graph", "--roots"])
        .arg(&rules)
        .arg(dir)
        .assert()
        .success()
        .get_output()
        .stderr
        .clone();
    String::from_utf8_lossy(&out).to_string()
}

#[test]
fn nix_a_method_pattern_matches_the_attribute_path() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "tests/default.nix",
        "{ pkgs }: pkgs.testers.runNixOSTest {\n\
         \x20 nodes = {\n\
         \x20   machine = { pkgs, ... }: { };\n\
         \x20   peer = { pkgs, ... }: { };\n\
         \x20 };\n\
         \x20 testScript = { nodes, ... }: \"start_all()\";\n\
         }\n",
    );
    let err = run_with_roots(
        tmp.path(),
        "[[framework]]\nid = \"nixos-test-user\"\nlanguage = \"nix\"\nkind = \"lifecycle\"\n\
         detect_paths = [\"tests/\"]\nmethods = [\"testScript\", \"nodes.*\"]\nnode = true\n",
    );
    assert!(
        err.contains("nixos-test-user (lifecycle, 3 entries)"),
        "{err}"
    );
}
