//! End-to-end fixtures for the 0.14.0 native-language items: Lean
//! attributes, Fortran fixed-form source and assembly extraction.

use std::fs;
use std::path::Path;

use assert_cmd::prelude::*;
use std::process::Command;
use tempfile::TempDir;

fn cgg() -> Command {
    Command::cargo_bin("cgg").expect("cgg binary")
}

fn write(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, body).unwrap();
}

/// Run cgg over `dir` and return (mermaid graph, stderr).
fn run(dir: &Path, extra: &[&str]) -> (String, String) {
    let out = dir.join("g.mmd");
    let mut cmd = cgg();
    cmd.args(["-o"]).arg(&out).args(extra).arg(dir);
    let assert = cmd.assert().success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    (fs::read_to_string(&out).unwrap(), stderr)
}

/// Simple names of the dead-code findings for `dir`.
fn dead(dir: &Path) -> Vec<String> {
    let out = cgg()
        .args([
            "--dead-code",
            "--no-graph",
            "--dead-code-format",
            "json",
            "--jobs",
            "1",
        ])
        .arg(dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let mut names: Vec<String> = doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["simple_name"].as_str().unwrap().to_string())
        .collect();
    names.sort();
    names
}

// ---------------------------------------------------------------------
// Lean attributes
// ---------------------------------------------------------------------

const LEAN: &str = "\
def helper (n : Nat) : Nat := n + 1

def main (args : List String) : IO Unit := do
  IO.println s!\"{helper 1}\"

@[export lean_my_add]
def myAdd (a b : UInt32) : UInt32 := a + b

@[extern \"my_c_fn\"]
opaque myCFn : UInt32 → UInt32

@[command_elab myCmd] def elabMyCmd : CommandElab := fun _ => pure ()
@[tactic myTac] def evalMyTac : Tactic := fun _ => pure ()
@[builtin_term_elab foo] def elabFoo : TermElab := fun _ _ => pure ()
@[init] def initFn : IO Unit := pure ()
@[implemented_by fastImpl] def slowImpl (n : Nat) : Nat := n
def fastImpl (n : Nat) : Nat := n
def unused (n : Nat) : Nat := n
";

// `@[export]` was keyed `[export` (the `@[` prefix half-stripped), so no
// Lean attribute could ever match a rule.
#[test]
fn lean_export_and_extern_are_ffi_entries() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "Main.lean", LEAN);
    let (g, err) = run(tmp.path(), &[]);
    assert!(g.contains("::ffi::lean-ffi::myAdd"), "{g}");
    assert!(g.contains("::ffi::lean-ffi::myCFn"), "{g}");
    assert!(err.contains("lean-ffi"), "{err}");
    // The elaborator attributes root without minting nodes.
    assert!(!g.contains("lean-attributes"), "{g}");
}

#[test]
fn lean_attribute_registered_declarations_are_not_dead() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "Main.lean", LEAN);
    let found = dead(tmp.path());
    for live in [
        "myAdd",
        "myCFn",
        "elabMyCmd",
        "evalMyTac",
        "elabFoo",
        "initFn",
        "slowImpl",
        // Named by `@[implemented_by fastImpl]` only.
        "fastImpl",
    ] {
        assert!(!found.contains(&live.to_string()), "{live}: {found:?}");
    }
    assert!(found.contains(&"unused".to_string()), "{found:?}");
}

// ---------------------------------------------------------------------
// Fortran fixed form
// ---------------------------------------------------------------------

// Column-1 `C` comments sank a fixed-form Abaqus user subroutine file
// into ERROR nodes: the file had no callables at all.
#[test]
fn fortran_fixed_form_file_with_c_comments_has_its_routines() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "umat.for",
        "C*************************************************\n\
         C   USER MATERIAL\n\
         C=================================================\n\
         \x20     SUBROUTINE UMAT(STRESS,STATEV,\n\
         \x20    1 NTENS)\n\
         C     body\n\
         \x20     CALL HELPER(STRESS)\n\
         \x20     RETURN\n\
         \x20     END\n\
         C-------------------------------------------------\n\
         \x20     SUBROUTINE HELPER(S)\n\
         \x20     RETURN\n\
         \x20     END\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    assert!(g.contains("UMAT"), "{g}");
    assert!(g.contains("HELPER"), "{g}");
}
