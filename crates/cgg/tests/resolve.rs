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

/// An unambiguous `from x import y` binding resolves at `high`.
///
/// This asserted `medium` until 0.6.6. That was the wrong calibration
/// and it had a concrete cost: same-file resolution scores `high`, so a
/// class method with a name colliding with an imported function
/// *outranked* the correct target. A single-candidate import binding is
/// not a guess — fan-out still scores `medium`, because that is one.
#[test]
fn audit_records_high_confidence_for_an_unambiguous_import() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "lib.py", b"def work():\n    return 1\n");
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
    assert!(
        confidence["high"].as_u64().unwrap_or(0) >= 1,
        "an unambiguous import binding should resolve high: {confidence}"
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
    assert!(
        g.contains("upstream::helper"),
        "missing upstream::helper in:\n{g}"
    );
    assert!(
        g.contains("downstream::caller"),
        "missing downstream::caller in:\n{g}"
    );
    // And a cross-crate edge downstream::caller -> upstream::helper.
    // Find the node ids and verify the arrow.
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
    write(tmp.path(), "facade/src/lib.rs", b"pub use core_::work;\n");

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
    assert!(
        g.contains(&arrow),
        "re-export chain missing edge {arrow}:\n{g}"
    );
}

#[test]
fn go_cross_package_call_resolves() {
    // Two-package Go fixture: `lib` defines `Helper`; `main` imports
    // lib and calls `lib.Helper()`.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "lib/lib.go",
        b"package lib\n\nfunc Helper() int { return 1 }\n",
    );
    write(
        tmp.path(),
        "main.go",
        b"package main\n\nimport \"example.com/lib\"\n\nfunc Run() int { return lib.Helper() }\n",
    );

    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["-t", "mermaid", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();

    let g = fs::read_to_string(&mmd).unwrap();
    assert!(g.contains("main.Run"));
    assert!(g.contains("lib.Helper"));
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
    let run = node_id("main.Run").expect("main.Run node");
    let helper = node_id("lib.Helper").expect("lib.Helper node");
    let arrow = format!("{run} --> {helper}");
    assert!(g.contains(&arrow), "missing cross-package edge:\n{g}");
}

#[test]
fn go_aliased_import_resolves() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "stringz/stringz.go",
        b"package stringz\n\nfunc Upper(s string) string { return s }\n",
    );
    write(
        tmp.path(),
        "main.go",
        b"package main\n\nimport sz \"example.com/stringz\"\n\nfunc Run() string { return sz.Upper(\"hi\") }\n",
    );

    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["-t", "mermaid", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();

    let g = fs::read_to_string(&mmd).unwrap();
    assert!(g.contains("stringz.Upper"), "missing def:\n{g}");
    assert!(g.contains("main.Run"));
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
    let run = node_id("main.Run").expect("main.Run node");
    let upper = node_id("stringz.Upper").expect("stringz.Upper node");
    let arrow = format!("{run} --> {upper}");
    assert!(g.contains(&arrow), "aliased Go import failed:\n{g}");
}

#[test]
fn csharp_cross_file_namespace_call_resolves() {
    // Two C# files in the same namespace; one calls the other's
    // static method via the fully-qualified path.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "helpers.cs",
        b"namespace App {\n    public static class Helpers {\n        public static int Add(int a, int b) { return a + b; }\n    }\n}\n",
    );
    write(
        tmp.path(),
        "main.cs",
        b"namespace App {\n    public class Runner {\n        public int Go() { return App.Helpers.Add(1, 2); }\n    }\n}\n",
    );

    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["-t", "mermaid", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();

    let g = fs::read_to_string(&mmd).unwrap();
    assert!(g.contains("App.Helpers.Add"), "missing def:\n{g}");
    assert!(g.contains("App.Runner.Go"), "missing def:\n{g}");
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
    let gogo = node_id("App.Runner.Go").expect("App.Runner.Go node");
    let add = node_id("App.Helpers.Add").expect("App.Helpers.Add node");
    let arrow = format!("{gogo} --> {add}");
    assert!(g.contains(&arrow), "missing C# cross-file edge:\n{g}");
}

#[test]
fn c_include_header_resolves() {
    // C project: header defines `add`, two TUs include it and call it.
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "helpers.h", b"int add(int a, int b);\n");
    write(
        tmp.path(),
        "helpers.c",
        b"#include \"helpers.h\"\nint add(int a, int b) { return a + b; }\n",
    );
    write(
        tmp.path(),
        "main.c",
        b"#include \"helpers.h\"\nint run() { return add(1, 2); }\n",
    );

    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["-t", "mermaid", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();

    let g = fs::read_to_string(&mmd).unwrap();
    assert!(g.contains("run"), "missing run:\n{g}");
    assert!(g.contains("add"), "missing add:\n{g}");
    // Find the run node and any add node, then check an edge exists.
    let node_id = |qn: &str| -> Vec<String> {
        g.lines()
            .filter_map(|l| {
                let l = l.trim();
                if l.starts_with('C') && l.contains(&format!("[\"{qn}\"]")) {
                    Some(l.split('[').next()?.trim().to_string())
                } else {
                    None
                }
            })
            .collect()
    };
    let runs = node_id("run");
    let adds = node_id("add");
    assert!(!runs.is_empty(), "no run node");
    assert!(!adds.is_empty(), "no add node");
    let has_edge = runs
        .iter()
        .any(|r| adds.iter().any(|a| g.contains(&format!("{r} --> {a}"))));
    assert!(has_edge, "missing C include edge:\n{g}");
}

#[test]
fn cpp_namespace_cross_file_resolves() {
    // C++ project: header declares namespace::class::method; impl
    // file defines it; caller includes header and calls it.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "math.hpp",
        b"namespace math {\nclass Calc {\npublic:\n    static int add(int a, int b);\n};\n}\n",
    );
    write(
        tmp.path(),
        "math.cpp",
        b"#include \"math.hpp\"\nnamespace math {\nint Calc::add(int a, int b) { return a + b; }\n}\n",
    );
    write(
        tmp.path(),
        "main.cpp",
        b"#include \"math.hpp\"\nint run() { return math::Calc::add(1, 2); }\n",
    );

    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["-t", "mermaid", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();

    let g = fs::read_to_string(&mmd).unwrap();
    assert!(g.contains("run"), "missing run:\n{g}");
    // The qualified call `math::Calc::add` should resolve.
    assert!(
        g.contains("math::Calc::add"),
        "missing math::Calc::add:\n{g}"
    );
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
    let run = node_id("run").expect("run node");
    let add = node_id("math::Calc::add").expect("math::Calc::add node");
    let arrow = format!("{run} --> {add}");
    assert!(g.contains(&arrow), "missing C++ cross-file edge:\n{g}");
}

#[test]
fn js_esm_import_resolves() {
    // JS project: utils.js exports helper; main.js imports and calls it.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "utils.js",
        b"export function helper() { return 1; }\nexport function scale(x) { return helper() * x; }\n",
    );
    write(
        tmp.path(),
        "main.js",
        b"import { helper, scale } from './utils.js';\nfunction run() { helper(); scale(2); }\n",
    );

    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["-t", "mermaid", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();

    let g = fs::read_to_string(&mmd).unwrap();
    assert!(g.contains("helper"), "missing helper:\n{g}");
    assert!(g.contains("scale"), "missing scale:\n{g}");
    assert!(g.contains("run"), "missing run:\n{g}");
    // scale -> helper is intra-file; run -> helper and run -> scale are cross-file.
    let node_ids = |qn: &str| -> Vec<String> {
        g.lines()
            .filter_map(|l| {
                let l = l.trim();
                if l.starts_with('C') && l.contains(&format!("[\"{qn}\"]")) {
                    Some(l.split('[').next()?.trim().to_string())
                } else {
                    None
                }
            })
            .collect()
    };
    let runs = node_ids("run");
    let helpers = node_ids("helper");
    assert!(!runs.is_empty() && !helpers.is_empty());
    let has_edge = runs
        .iter()
        .any(|r| helpers.iter().any(|h| g.contains(&format!("{r} --> {h}"))));
    assert!(has_edge, "missing JS cross-file edge run->helper:\n{g}");
}

#[test]
fn ts_namespace_import_resolves() {
    // TS project: math.ts exports add; app.ts imports * as math and calls math.add.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "math.ts",
        b"export function add(a: number, b: number): number { return a + b; }\n",
    );
    write(
        tmp.path(),
        "app.ts",
        b"import * as math from './math';\nexport function run(): number { return math.add(1, 2); }\n",
    );

    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["-t", "mermaid", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();

    let g = fs::read_to_string(&mmd).unwrap();
    assert!(g.contains("add"), "missing add:\n{g}");
    assert!(g.contains("run"), "missing run:\n{g}");
    let node_ids = |qn: &str| -> Vec<String> {
        g.lines()
            .filter_map(|l| {
                let l = l.trim();
                if l.starts_with('C') && l.contains(&format!("[\"{qn}\"]")) {
                    Some(l.split('[').next()?.trim().to_string())
                } else {
                    None
                }
            })
            .collect()
    };
    let runs = node_ids("run");
    let adds = node_ids("add");
    assert!(!runs.is_empty() && !adds.is_empty());
    let has_edge = runs
        .iter()
        .any(|r| adds.iter().any(|a| g.contains(&format!("{r} --> {a}"))));
    assert!(has_edge, "missing TS namespace import edge run->add:\n{g}");
}

#[test]
fn java_cross_file_import_resolves() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "Helper.java", b"package lib;\npublic class Helper {\n  public static int add(int a, int b) { return a + b; }\n}\n");
    write(tmp.path(), "Main.java", b"package app;\nimport lib.Helper;\npublic class Main {\n  public void run() { Helper.add(1, 2); }\n}\n");

    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["-t", "mermaid", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();
    let g = fs::read_to_string(&mmd).unwrap();
    assert!(g.contains("add"), "missing add:\n{g}");
    assert!(g.contains("run"), "missing run:\n{g}");
    // Cross-file edge: run -> add
    let node_ids = |qn: &str| -> Vec<String> {
        g.lines()
            .filter_map(|l| {
                let l = l.trim();
                if l.starts_with('C') && l.contains(&format!("[\"{qn}\"]")) {
                    Some(l.split('[').next()?.trim().to_string())
                } else {
                    None
                }
            })
            .collect()
    };
    let runs = node_ids("app.Main.run");
    let adds = node_ids("lib.Helper.add");
    assert!(!runs.is_empty(), "no run node:\n{g}");
    assert!(!adds.is_empty(), "no add node:\n{g}");
    let has_edge = runs
        .iter()
        .any(|r| adds.iter().any(|a| g.contains(&format!("{r} --> {a}"))));
    assert!(has_edge, "missing Java cross-file edge:\n{g}");
}

#[test]
fn kotlin_cross_file_resolves() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "Helper.kt",
        b"package lib\nfun helper(): Int = 42\n",
    );
    write(
        tmp.path(),
        "Main.kt",
        b"package app\nimport lib.helper\nfun run(): Int = helper()\n",
    );

    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["-t", "mermaid", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();
    let g = fs::read_to_string(&mmd).unwrap();
    assert!(g.contains("helper"), "missing helper:\n{g}");
    assert!(g.contains("run"), "missing run:\n{g}");
}

#[test]
fn bash_source_resolves() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "lib.sh",
        b"#!/bin/bash\nhelper() { echo hi; }\n",
    );
    write(
        tmp.path(),
        "main.sh",
        b"#!/bin/bash\nsource ./lib.sh\nmain() { helper; }\n",
    );

    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["-t", "mermaid", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();
    let g = fs::read_to_string(&mmd).unwrap();
    assert!(g.contains("helper"), "missing helper:\n{g}");
    assert!(g.contains("main"), "missing main:\n{g}");
    let node_ids = |qn: &str| -> Vec<String> {
        g.lines()
            .filter_map(|l| {
                let l = l.trim();
                if l.starts_with('C') && l.contains(&format!("[\"{qn}\"]")) {
                    Some(l.split('[').next()?.trim().to_string())
                } else {
                    None
                }
            })
            .collect()
    };
    let mains = node_ids("main");
    let helpers = node_ids("helper");
    assert!(!mains.is_empty() && !helpers.is_empty());
    let has_edge = mains
        .iter()
        .any(|m| helpers.iter().any(|h| g.contains(&format!("{m} --> {h}"))));
    assert!(has_edge, "missing bash source edge:\n{g}");
}

#[test]
fn rust_owner_disambiguation_and_constructor_cascade() {
    // Issues 1 + 5: two `new` methods (World::new, Other::new) are an
    // ambiguous name, but `World::new()` names its owner and the bound
    // local `w` types every subsequent `w.method()`. None of these may
    // mis-resolve to `Other`.
    let tmp = TempDir::new().unwrap();
    let src = r#"
struct World { v: u32 }
struct Other { v: u32 }
impl World {
    fn new() -> Self { World { v: 0 } }
    fn load(&self) {}
    fn step(&self) {}
}
impl Other {
    fn new() -> Self { Other { v: 0 } }
}
fn run() {
    let w = World::new();
    w.load();
    w.step();
}
"#;
    write(tmp.path(), "w.rs", src.as_bytes());

    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args([
            "-t",
            "mermaid",
            "--stack-graphs",
            "off",
            "--filter",
            "crate::run$",
            "-n",
            "1",
            "-o",
        ])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();

    let g = fs::read_to_string(&mmd).unwrap();
    // The cascade resolves every call on `w` to World, including the
    // ambiguous `new`.
    assert!(g.contains("crate::World::new"), "missing World::new:\n{g}");
    assert!(
        g.contains("crate::World::load"),
        "missing World::load:\n{g}"
    );
    assert!(
        g.contains("crate::World::step"),
        "missing World::step:\n{g}"
    );
    // The same-named `Other::new` must never appear in run's neighborhood.
    assert!(
        !g.contains("crate::Other::new"),
        "Other::new mis-resolved:\n{g}"
    );
}

#[test]
fn rust_cross_file_receiver_method_resolves() {
    // Issue 2: a method call on a parameter of known type must resolve
    // to that type's method defined in *another* file, via the
    // (owner, method) index.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "reg.rs",
        b"pub struct Registry { v: u32 }\nimpl Registry {\n    pub fn commit(&mut self) {}\n}\n",
    );
    write(
        tmp.path(),
        "flush.rs",
        b"use crate::Registry;\nfn flush(reg: &mut Registry) {\n    reg.commit();\n}\n",
    );

    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["-t", "mermaid", "--stack-graphs", "off", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();

    let g = fs::read_to_string(&mmd).unwrap();
    assert!(g.contains("crate::flush"), "missing flush:\n{g}");
    assert!(
        g.contains("crate::Registry::commit"),
        "missing commit:\n{g}"
    );
    // The flush -> commit edge must exist.
    let flush_id = g
        .lines()
        .find(|l| l.contains("crate::flush"))
        .and_then(|l| l.split('[').next())
        .map(|s| s.trim().to_string());
    let commit_id = g
        .lines()
        .find(|l| l.contains("crate::Registry::commit"))
        .and_then(|l| l.split('[').next())
        .map(|s| s.trim().to_string());
    let (Some(f), Some(c)) = (flush_id, commit_id) else {
        panic!("ids:\n{g}")
    };
    assert!(
        g.contains(&format!("{f} --> {c}")),
        "missing flush->commit edge:\n{g}"
    );
}

#[test]
fn rust_aliased_type_receiver_resolves() {
    // Issue 7: a receiver typed through an import alias
    // (`use ... as Motor`) must canonicalize to the real owner so the
    // method call resolves to the underlying type's method.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "lib.rs",
        b"pub mod engine;\npub use engine::Engine;\n",
    );
    write(
        tmp.path(),
        "engine.rs",
        b"pub struct Engine;\nimpl Engine {\n    pub fn start(&self) {}\n}\n",
    );
    write(
        tmp.path(),
        "drive.rs",
        b"use crate::Engine as Motor;\nfn drive(m: &Motor) {\n    m.start();\n}\n",
    );

    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["-t", "mermaid", "--stack-graphs", "off", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();

    let g = fs::read_to_string(&mmd).unwrap();
    let drive_id = g
        .lines()
        .find(|l| l.contains("crate::drive"))
        .and_then(|l| l.split('[').next())
        .map(|s| s.trim().to_string());
    let start_id = g
        .lines()
        .find(|l| l.contains("crate::Engine::start"))
        .and_then(|l| l.split('[').next())
        .map(|s| s.trim().to_string());
    let (Some(d), Some(s)) = (drive_id, start_id) else {
        panic!("ids:\n{g}")
    };
    assert!(
        g.contains(&format!("{d} --> {s}")),
        "missing drive->Engine::start edge:\n{g}"
    );
}

#[test]
fn rust_dynamic_dispatch_fanout_is_opt_in() {
    let tmp = TempDir::new().unwrap();
    let src = r#"
trait Storage { fn put(&mut self, k: &str); }
struct DiskStorage;
struct MemStorage;
impl Storage for DiskStorage { fn put(&mut self, k: &str) {} }
impl Storage for MemStorage { fn put(&mut self, k: &str) {} }
"#;
    write(tmp.path(), "s.rs", src.as_bytes());

    // Default: no dynamic fan-out edges.
    let plain = tmp.path().join("p.mmd");
    cgg()
        .args(["--stack-graphs", "off", "-o"])
        .arg(&plain)
        .arg(tmp.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(&plain)
            .unwrap()
            .matches("-->|dyn|")
            .count(),
        0
    );

    // With --dynamic-dispatch: Storage::put fans out to both impls.
    let dyn_out = tmp.path().join("d.mmd");
    cgg()
        .args(["--stack-graphs", "off", "--dynamic-dispatch", "-o"])
        .arg(&dyn_out)
        .arg(tmp.path())
        .assert()
        .success();
    let g = fs::read_to_string(&dyn_out).unwrap();
    assert_eq!(
        g.matches("-->|dyn|").count(),
        2,
        "expected 2 dynamic fan-out edges:\n{g}"
    );
}

#[test]
fn rust_reference_edges_are_opt_in() {
    let tmp = TempDir::new().unwrap();
    let src =
        "fn tick(w: u32) {}\nfn boot() { register(tick); }\nfn register(f: fn(u32)) {}\n";
    write(tmp.path(), "r.rs", src.as_bytes());

    // Default: tick has in-degree zero (no reference edge).
    let plain = tmp.path().join("p.mmd");
    cgg()
        .args(["--stack-graphs", "off", "-o"])
        .arg(&plain)
        .arg(tmp.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(&plain)
            .unwrap()
            .matches("-->|ref|")
            .count(),
        0
    );

    // --reference-edges: boot -[ref]-> tick.
    let refs = tmp.path().join("r.mmd");
    cgg()
        .args(["--stack-graphs", "off", "--reference-edges", "-o"])
        .arg(&refs)
        .arg(tmp.path())
        .assert()
        .success();
    let g = fs::read_to_string(&refs).unwrap();
    assert_eq!(
        g.matches("-->|ref|").count(),
        1,
        "expected one reference edge:\n{g}"
    );
    assert!(g.contains("crate::tick"), "{g}");
}

// ---------------------------------------------------------------------
// Resolution gaps reported from a large Python service (0.6.5 audit)
// ---------------------------------------------------------------------

/// `Widget(3)` enters `Widget.__init__`.
///
/// 107 constructors in the audited service had zero inbound edges out of
/// 1206 — "who constructs X?" was unanswerable for every Python class.
#[test]
fn instantiation_links_to_the_constructor() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "widget.py",
        b"class Widget:\n    def __init__(self, a):\n        self.a = a\n",
    );
    write(
        tmp.path(),
        "driver.py",
        b"from widget import Widget\ndef main():\n    return Widget(3)\n",
    );
    let g = graph_of(tmp.path());
    assert!(
        g.contains("Widget.__init__"),
        "expected a constructor edge:\n{g}"
    );
}

/// An *inherited* method resolves through the base chain.
///
/// The contrast is the bug: same receiver, same syntax, same file —
/// resolved when declared on the instantiated class, dropped when
/// inherited.
#[test]
fn an_inherited_method_call_resolves_through_the_base() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "base.py",
        b"class BaseWorker:\n    def apply(self, x):\n        return x\n",
    );
    write(tmp.path(), "child.py", b"from base import BaseWorker\nclass ChildWorker(BaseWorker):\n    def extra(self, x):\n        return x\n");
    write(
        tmp.path(),
        "driver.py",
        b"from child import ChildWorker\ndef main():\n    w = ChildWorker()\n    w.extra(1)\n    w.apply(2)\n",
    );
    let g = graph_of(tmp.path());
    assert!(
        g.contains("ChildWorker.extra"),
        "declared-on-subclass edge missing:\n{g}"
    );
    assert!(
        g.contains("BaseWorker.apply"),
        "inherited edge missing:\n{g}"
    );
}

/// Calling an instance enters `__call__`.
#[test]
fn calling_an_instance_resolves_to_the_call_operator() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "mod.py",
        b"class Agent:\n    def __call__(self, p):\n        return p\n",
    );
    write(
        tmp.path(),
        "use.py",
        b"from mod import Agent\ndef go():\n    a = Agent()\n    a(\"prompt\")\n",
    );
    let g = graph_of(tmp.path());
    assert!(
        g.contains("Agent.__call__"),
        "expected a __call__ edge:\n{g}"
    );
}

/// `super().m()` never targets the calling class's own `m`.
///
/// With the base out of graph this produced an edge back to the
/// subclass, and combined with the real forward edge it formed a
/// phantom cycle that reads as infinite recursion.
#[test]
fn super_does_not_resolve_to_the_calling_class() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "shape.py",
        b"from third_party_not_in_graph import ExternalBase\nclass Sub(ExternalBase):\n    def __call__(self, p):\n        return self._inner(p)\n    def _inner(self, p):\n        return super().__call__(p)\n",
    );
    let g = graph_of(tmp.path());
    assert!(
        g.contains("Sub.__call__"),
        "sanity: the class should be in the graph:\n{g}"
    );
    // The forward edge is real; the back edge is not.
    let back = g.lines().filter(|l| l.contains("-->")).any(|l| {
        l.contains("_inner")
            && l.split("-->")
                .nth(1)
                .is_some_and(|r| r.contains("__call__"))
    });
    assert!(
        !back,
        "super() must not resolve to the subclass's own override:\n{g}"
    );
}

/// A bare identifier bound by `from x import y` resolves only to the
/// import, and at `high`.
#[test]
fn a_bare_name_prefers_its_import_over_a_same_file_method() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "lib.py", b"def helper(x):\n    return x\n");
    write(
        tmp.path(),
        "use.py",
        b"from lib import helper\nclass Holder:\n    def helper(self, x):\n        return x\ndef go():\n    helper(2)\n",
    );
    let g = graph_of(tmp.path());
    // The method is still a node — it is a real definition. What it must
    // not be is the *target* of the bare call.
    let id = |qn: &str| {
        g.lines().find_map(|l| {
            let l = l.trim();
            l.starts_with('C')
                .then(|| {
                    l.contains(&format!("[\"{qn}\"]"))
                        .then(|| l.split('[').next())
                })
                .flatten()
                .flatten()
                .map(str::to_string)
        })
    };
    let lib = id("lib.helper").expect("lib.helper node");
    let method = id("use.Holder.helper").expect("Holder.helper node");
    let targets: Vec<&str> = g
        .lines()
        .filter_map(|l| l.split("-->").nth(1))
        .map(str::trim)
        .collect();
    assert!(
        targets.iter().any(|t| *t == lib),
        "expected an edge to lib.helper:\n{g}"
    );
    assert!(
        !targets.iter().any(|t| *t == method),
        "a method is not in scope for a bare call:\n{g}"
    );
}

/// Render `dir` and return the mermaid graph.
fn graph_of(dir: &Path) -> String {
    let out = dir.join("g.mmd");
    cgg()
        .args(["-t", "mermaid", "-o"])
        .arg(&out)
        .arg(dir)
        .assert()
        .success();
    fs::read_to_string(&out).unwrap()
}
