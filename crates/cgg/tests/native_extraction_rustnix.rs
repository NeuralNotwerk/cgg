//! Extraction regressions for Rust function values outside call
//! arguments and Nix attribute-path qualified names (0.14.0, packet B5).

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

/// The qualified names of every dead-code finding over `dir`, in every
/// confidence band.
fn dead(dir: &Path) -> Vec<String> {
    let out = Command::cargo_bin("cgg")
        .unwrap()
        .args(["--dead-code", "--dead-code-format", "json", "--no-graph"])
        .arg(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let doc: serde_json::Value = serde_json::from_slice(&out).unwrap();
    doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["qualified_name"].as_str().unwrap().to_string())
        .collect()
}

fn reported(report: &[String], qn: &str) -> bool {
    report.iter().any(|f| f == qn)
}

#[test]
fn rust_function_values_outside_call_arguments_are_references() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "src/main.rs",
        "mod settings { pub fn render_x() {} pub fn render_y() {} }\n\
         struct Entry { render: fn() }\n\
         fn in_struct() {}\nfn in_array() {}\nfn in_tuple() {}\nfn in_let() {}\n\
         fn as_receiver() {}\nfn in_vec_macro() {}\nfn returned() {}\n\
         fn in_static() {}\nstatic HOOK: fn() = in_static;\n\
         static TABLE: &[Entry] = &[Entry { render: settings::render_y }];\n\
         trait Cond { fn run_if(self) -> Self; }\n\
         fn pick() -> fn() { return returned; }\n\
         fn main() {\n\
         \x20   let _e = Entry { render: in_struct };\n\
         \x20   let _p = Entry { render: settings::render_x };\n\
         \x20   let _a = [in_array as fn()];\n\
         \x20   let _t = (in_tuple, 1);\n\
         \x20   let _l: fn() = in_let;\n\
         \x20   let _r = as_receiver.run_if();\n\
         \x20   let _v = vec![in_vec_macro];\n\
         \x20   let _ = (pick(), HOOK, TABLE);\n\
         }\n",
    );
    let report = dead(tmp.path());
    for f in [
        "crate::in_struct",
        "crate::settings::render_x",
        "crate::settings::render_y",
        "crate::in_array",
        "crate::in_tuple",
        "crate::in_let",
        "crate::as_receiver",
        "crate::in_vec_macro",
        "crate::returned",
        "crate::in_static",
    ] {
        assert!(!reported(&report, f), "{f} reported dead: {report:?}");
    }
}

#[test]
fn rust_bevy_system_tuples_and_combinator_receivers_are_references() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "src/main.rs",
        "use bevy::prelude::*;\n\
         fn main() {\n\
         \x20   App::new()\n\
         \x20       .add_systems(Update, (tuple_a, tuple_b))\n\
         \x20       .add_systems(Update, chained_recv.run_if(cond))\n\
         \x20       .add_systems(Update, (chain_a, chain_b).chain())\n\
         \x20       .add_systems(Update, after_a.after(after_b))\n\
         \x20       .run();\n\
         }\n\
         fn tuple_a() {}\nfn tuple_b() {}\nfn chained_recv() {}\n\
         fn cond() -> bool { true }\nfn chain_a() {}\nfn chain_b() {}\n\
         fn after_a() {}\nfn after_b() {}\n",
    );
    let report = dead(tmp.path());
    for f in [
        "tuple_a",
        "tuple_b",
        "chained_recv",
        "cond",
        "chain_a",
        "chain_b",
        "after_a",
        "after_b",
    ] {
        assert!(
            !reported(&report, &format!("crate::{f}")),
            "{f} reported dead: {report:?}"
        );
    }
}

#[test]
fn rust_a_local_in_a_value_position_is_not_the_function() {
    // `helper` is a local here: the tuple, the receiver and the struct
    // field read the variable, never the same-named function.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "src/main.rs",
        "struct S { v: u32 }\n\
         fn helper() {}\n\
         fn main() {\n\
         \x20   let helper = 1u32;\n\
         \x20   let _t = (helper, 2);\n\
         \x20   let _s = S { v: helper };\n\
         \x20   let _c = helper.count_ones();\n\
         }\n",
    );
    let report = dead(tmp.path());
    assert!(reported(&report, "crate::helper"), "{report:?}");
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

#[test]
fn rust_a_local_hides_an_imported_or_module_name_in_a_value_position() {
    // `mod progress;` and `use …::group_by` put the names in the import
    // table; the parameter and the `let` still own the value namespace.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "src/main.rs",
        "mod progress;\nmod grouping { pub fn group_by() {} }\n\
         use grouping::group_by;\n\
         struct P;\nimpl P { fn end(&self) {} }\n\
         fn level(progress: &P) { progress.end(); }\n\
         fn main() {\n\
         \x20   let group_by = P;\n\
         \x20   group_by.end();\n\
         \x20   level(&P);\n\
         }\n",
    );
    write(tmp.path(), "src/progress.rs", "pub fn progress() {}\n");
    let report = dead(tmp.path());
    assert!(reported(&report, "crate::grouping::group_by"), "{report:?}");
    assert!(reported(&report, "crate::progress"), "{report:?}");
}

#[test]
fn rust_a_receiver_names_a_function_only_where_the_file_can_see_one() {
    // `contexts` is a glob-imported diesel table here; the `contexts`
    // getter of another module is not in scope and is not referenced.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "src/main.rs",
        "mod sdk;\nmod schema { pub struct T; }\n\
         use schema::*;\n\
         fn main() { let _r = contexts.filter(1); }\n",
    );
    write(tmp.path(), "src/sdk.rs", "pub fn contexts() {}\n");
    let report = dead(tmp.path());
    assert!(reported(&report, "crate::contexts"), "{report:?}");
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

#[test]
fn rust_a_registrar_reads_values_in_a_tuple_or_a_combinator_receiver() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "src/main.rs",
        "use bevy::prelude::*;\n\
         fn main() {\n\
         \x20   App::new()\n\
         \x20       .add_systems(Update, (tuple_a, tuple_b))\n\
         \x20       .add_systems(Update, chained.run_if(cond))\n\
         \x20       .add_systems(Startup, bare)\n\
         \x20       .run();\n\
         }\n\
         fn tuple_a() {}\nfn tuple_b() {}\nfn chained() {}\n\
         fn cond() -> bool { true }\nfn bare() {}\n",
    );
    let err = run_with_roots(
        tmp.path(),
        "[[framework]]\nid = \"bevy-user\"\nlanguage = \"rust\"\nkind = \"lifecycle\"\n\
         detect = [\"bevy\"]\nregistrars = [\"add_systems\"]\nnode = true\n",
    );
    // tuple_a, tuple_b, chained, cond (a bare argument of `run_if`, not
    // of `add_systems`, so not an entry) and bare.
    assert!(err.contains("bevy-user (lifecycle, 4 entries)"), "{err}");
}
