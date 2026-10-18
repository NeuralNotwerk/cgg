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

fn mermaid_id(g: &str, qn: &str) -> Option<String> {
    g.lines().find_map(|l| {
        let l = l.trim();
        if l.starts_with(['C', 'N']) && l.contains(&format!("[\"{qn}\"]")) {
            Some(l.split('[').next()?.trim().to_string())
        } else {
            None
        }
    })
}

fn dead_code_qns(report: &Path) -> Vec<String> {
    let parsed: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(report).unwrap()).unwrap();
    parsed["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["qualified_name"].as_str().map(str::to_string))
        .collect()
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
            if l.starts_with(['C', 'N']) && l.contains(&format!("[\"{qn}\"]")) {
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
            if l.starts_with(['C', 'N']) && l.contains(&format!("[\"{qn}\"]")) {
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
            if l.starts_with(['C', 'N']) && l.contains(&format!("[\"{qn}\"]")) {
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
            if l.starts_with(['C', 'N']) && l.contains(&format!("[\"{qn}\"]")) {
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
            if l.starts_with(['C', 'N']) && l.contains(&format!("[\"{qn}\"]")) {
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
                if l.starts_with(['C', 'N']) && l.contains(&format!("[\"{qn}\"]")) {
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
        b"#include \"math.hpp\"\nnamespace math {\nint helper(int a, int b) { return a + b; }\nint Calc::add(int a, int b) { return helper(a, b); }\n}\n",
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
            if l.starts_with(['C', 'N']) && l.contains(&format!("[\"{qn}\"]")) {
                Some(l.split('[').next()?.trim().to_string())
            } else {
                None
            }
        })
    };
    // The header declaration and the .cpp body are one function. The
    // edge has to land on the body, which is the node that calls helper.
    let add_nodes: Vec<&str> = g
        .lines()
        .filter(|l| l.contains("[\"math::Calc::add\"]"))
        .collect();
    assert_eq!(
        add_nodes.len(),
        1,
        "prototype and body should be one node:\n{g}"
    );
    let run = node_id("run").expect("run node");
    let add = node_id("math::Calc::add").expect("math::Calc::add node");
    let arrow = format!("{run} --> {add}");
    assert!(g.contains(&arrow), "missing C++ cross-file edge:\n{g}");
    let helper = node_id("math::helper").expect("math::helper node");
    let into_body = format!("{add} --> {helper}");
    assert!(
        g.contains(&into_body),
        "call from add should leave the definition, not an empty prototype:\n{g}"
    );
}

#[test]
fn cpp_typed_local_and_field_chain_resolve() {
    // `Kernel* kernel` types `kernel->add_module`. `streams` is a field
    // declared on the class, so `kernel->streams->printf` types as
    // StreamOutput even though the call and the field are in different
    // files. `this->streams` inside the method does the same.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "kernel.h",
        b"class StreamOutput {\npublic:\n    void printf(const char*);\n};\nclass Kernel {\npublic:\n    StreamOutput* streams;\n    void add_module(int);\n};\n",
    );
    write(
        tmp.path(),
        "kernel.cpp",
        b"#include \"kernel.h\"\nvoid StreamOutput::printf(const char*) {}\nvoid Kernel::add_module(int) { this->streams->printf(\"x\"); }\n",
    );
    write(
        tmp.path(),
        "main.cpp",
        b"#include \"kernel.h\"\nvoid init() {\n    Kernel* kernel = new Kernel();\n    kernel->add_module(1);\n    kernel->streams->printf(\"hi\");\n}\nvoid shout(StreamOutput* stream) { stream->printf(\"z\"); }\n",
    );

    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["-t", "mermaid", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();

    let g = fs::read_to_string(&mmd).unwrap();
    let node_id = |qn: &str| {
        g.lines().find_map(|l| {
            let l = l.trim();
            if l.starts_with(['C', 'N']) && l.contains(&format!("[\"{qn}\"]")) {
                Some(l.split('[').next()?.trim().to_string())
            } else {
                None
            }
        })
    };
    let init = node_id("init").unwrap_or_else(|| panic!("init:\n{g}"));
    let add = node_id("Kernel::add_module").unwrap_or_else(|| panic!("add:\n{g}"));
    let printf =
        node_id("StreamOutput::printf").unwrap_or_else(|| panic!("printf:\n{g}"));
    let shout = node_id("shout").unwrap_or_else(|| panic!("shout:\n{g}"));
    assert!(
        g.contains(&format!("{init} --> {add}")),
        "kernel->add_module did not resolve:\n{g}"
    );
    assert!(
        g.contains(&format!("{init} --> {printf}")),
        "kernel->streams->printf did not resolve:\n{g}"
    );
    assert!(
        g.contains(&format!("{add} --> {printf}")),
        "this->streams->printf did not resolve:\n{g}"
    );
    assert!(
        g.contains(&format!("{shout} --> {printf}")),
        "stream parameter did not resolve:\n{g}"
    );
}

#[test]
fn cpp_same_local_name_in_two_functions_keeps_each_type() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "io.h",
        b"class StreamOutput {\npublic:\n    void printf(const char*);\n};\nclass Kernel {\npublic:\n    void add_module(int);\n};\n",
    );
    write(
        tmp.path(),
        "io.cpp",
        b"#include \"io.h\"\nvoid StreamOutput::printf(const char*) {}\nvoid Kernel::add_module(int) {}\nvoid a() { StreamOutput* stream; stream->printf(\"x\"); }\nvoid b() { Kernel* stream; stream->add_module(1); }\n",
    );
    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["-t", "mermaid", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();
    let g = fs::read_to_string(&mmd).unwrap();
    let node_id = |qn: &str| -> Option<String> {
        g.lines().find_map(|l| {
            let l = l.trim();
            if l.starts_with(['C', 'N']) && l.contains(&format!("[\"{qn}\"]")) {
                Some(l.split('[').next()?.trim().to_string())
            } else {
                None
            }
        })
    };
    let a = node_id("a").unwrap_or_else(|| panic!("a:\n{g}"));
    let b = node_id("b").unwrap_or_else(|| panic!("b:\n{g}"));
    let printf =
        node_id("StreamOutput::printf").unwrap_or_else(|| panic!("printf:\n{g}"));
    let add = node_id("Kernel::add_module").unwrap_or_else(|| panic!("add:\n{g}"));
    assert!(g.contains(&format!("{a} --> {printf}")), "a:\n{g}");
    assert!(g.contains(&format!("{b} --> {add}")), "b:\n{g}");
    assert!(
        !g.contains(&format!("{a} --> {add}")),
        "a must not pick b's stream:\n{g}"
    );
}

#[test]
fn cpp_object_like_macro_receivers_resolve() {
    // `#define THEKERNEL Kernel::instance` and aliases that hop through
    // it. The receiver is uppercase, so without macro typing it would
    // be left alone as a path and never bind.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "kernel.h",
        b"#define THEKERNEL Kernel::instance\n#define THEROBOT THEKERNEL->robot\nclass StreamOutput {\npublic:\n    void printf(const char*);\n};\nclass Robot {\npublic:\n    void on_idle();\n};\nclass Kernel {\npublic:\n    static Kernel* instance;\n    StreamOutput* streams;\n    Robot* robot;\n    void add_module(int);\n};\n",
    );
    write(
        tmp.path(),
        "kernel.cpp",
        b"#include \"kernel.h\"\nKernel* Kernel::instance;\nvoid StreamOutput::printf(const char*) {}\nvoid Robot::on_idle() {}\nvoid Kernel::add_module(int) {}\n",
    );
    write(
        tmp.path(),
        "main.cpp",
        b"#include \"kernel.h\"\nvoid init() {\n    THEKERNEL->add_module(1);\n    THEKERNEL->streams->printf(\"hi\");\n    THEROBOT->on_idle();\n}\n",
    );

    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["-t", "mermaid", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();
    let g = fs::read_to_string(&mmd).unwrap();
    let node_id = |qn: &str| -> Option<String> {
        g.lines().find_map(|l| {
            let l = l.trim();
            if l.starts_with(['C', 'N']) && l.contains(&format!("[\"{qn}\"]")) {
                Some(l.split('[').next()?.trim().to_string())
            } else {
                None
            }
        })
    };
    let init = node_id("init").unwrap_or_else(|| panic!("init:\n{g}"));
    let add = node_id("Kernel::add_module").unwrap_or_else(|| panic!("add:\n{g}"));
    let printf =
        node_id("StreamOutput::printf").unwrap_or_else(|| panic!("printf:\n{g}"));
    let idle = node_id("Robot::on_idle").unwrap_or_else(|| panic!("on_idle:\n{g}"));
    assert!(
        g.contains(&format!("{init} --> {add}")),
        "THEKERNEL->add_module:\n{g}"
    );
    assert!(
        g.contains(&format!("{init} --> {printf}")),
        "THEKERNEL->streams->printf:\n{g}"
    );
    assert!(
        g.contains(&format!("{init} --> {idle}")),
        "THEROBOT->on_idle:\n{g}"
    );
}

/// Edges from `tmp` as (src, dst, confidence), rendered through `-t json`.
fn cpp_edges(tmp: &Path) -> Vec<(String, String, String)> {
    let out = tmp.join("g.json");
    cgg()
        .args(["-t", "json", "-o"])
        .arg(&out)
        .arg(tmp)
        .assert()
        .success();
    let g: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&out).unwrap()).unwrap();
    let c = g["callables"].as_object().unwrap();
    let file = |id: &serde_json::Value| {
        let f = c[id.as_str().unwrap()]["file"].as_str().unwrap();
        let p = g["files"][f]["path"].as_str().unwrap();
        p.rsplit('/').next().unwrap().to_string()
    };
    let qn = |id: &serde_json::Value| {
        c[id.as_str().unwrap()]["qualified_name"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let mut v: Vec<_> = g["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                qn(&e["src"]),
                format!("{}@{}", qn(&e["dst"]), file(&e["dst"])),
                e["confidence"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    v.sort();
    v
}

#[test]
fn cpp_header_declaration_merged_into_a_body_elsewhere_still_binds_through_include() {
    // The caller includes only the header; the body lives in a .cpp it
    // never includes. Merging the prototype away must not lose the call.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "mocks/mock.h",
        b"#pragma once\nvoid mock_set_threads(int n);\n",
    );
    write(
        tmp.path(),
        "mocks/mock.cpp",
        b"#include \"mock.h\"\nvoid mock_set_threads(int n) { (void)n; }\n",
    );
    write(
        tmp.path(),
        "tests/t.cpp",
        b"#include \"mock.h\"\nvoid run_test() { mock_set_threads(3); }\n",
    );
    assert_eq!(
        cpp_edges(tmp.path()),
        vec![(
            "run_test".into(),
            "mock_set_threads@mock.cpp".into(),
            "high".into()
        )]
    );
}

#[test]
fn cpp_free_function_declared_in_several_programs_binds_to_its_own_declaration() {
    // Independent samples each declare and define `cpu_ref`. Merging one
    // sample's declaration into every same-named body made the call
    // ambiguous; it binds to the declaration in its own file instead.
    let tmp = TempDir::new().unwrap();
    for s in ["a", "b"] {
        write(
            tmp.path(),
            &format!("{s}/main.cpp"),
            b"extern \"C\" void cpu_ref(float *out, int n);\nint main() { cpu_ref(0, 1); return 0; }\n",
        );
        write(
            tmp.path(),
            &format!("{s}/gold.cpp"),
            b"extern \"C\" void cpu_ref(float *out, int n) { (void)out; (void)n; }\n",
        );
    }
    let e = cpp_edges(tmp.path());
    let calls: Vec<_> = e.iter().filter(|(s, _, _)| s == "main").collect();
    assert_eq!(calls.len(), 2, "one edge per program: {e:?}");
    assert!(
        calls
            .iter()
            .all(|(_, d, c)| d == "cpu_ref@main.cpp" && c == "high"),
        "{e:?}"
    );
}

#[test]
fn cpp_receiver_typed_through_an_alias_reaches_the_aliased_class() {
    // A `.hpp`, not a `.h`: a bare `.h` without a same-stem `.cpp` is
    // parsed as C, which has no templates (a known limitation).
    // `Rect` names no class; `using Rect = TRect<float>` makes it one.
    // A typed receiver or static call through the alias must land on
    // `TRect`, not on nothing.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "rect.hpp",
        b"template <class T> struct TRect {\n  static TRect MakeXYWH(T x) { return TRect(); }\n  T GetRight() const { return r; }\n  T r;\n};\nusing Rect = TRect<float>;\ntypedef TRect<int> IRect;\n",
    );
    write(
        tmp.path(),
        "use.cpp",
        b"#include \"rect.hpp\"\nfloat use() {\n  Rect rect = Rect::MakeXYWH(1);\n  IRect ir;\n  ir.GetRight();\n  return rect.GetRight();\n}\n",
    );
    let e = cpp_edges(tmp.path());
    for dst in ["TRect::MakeXYWH@rect.hpp", "TRect::GetRight@rect.hpp"] {
        assert!(
            e.iter()
                .any(|(s, d, c)| s == "use" && d == dst && c != "low"),
            "missing use -> {dst}: {e:?}"
        );
    }
}

#[test]
fn cpp_typed_receiver_with_no_known_class_falls_back_to_the_untyped_guess() {
    // `ITextProvider` is a COM interface cgg never sees. Typing the
    // receiver must not drop the call: with no class to look in, it is
    // resolved as the receiver as written would be — a medium guess.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "impl.cc",
        b"class TextProviderWin {\npublic:\n  int GetSelection(int x);\n};\nint TextProviderWin::GetSelection(int x) { return x; }\n",
    );
    write(
        tmp.path(),
        "use.cc",
        b"struct ITextProvider;\nint probe(ITextProvider* document_provider) {\n  return document_provider->GetSelection(1);\n}\n",
    );
    let e = cpp_edges(tmp.path());
    assert_eq!(
        e,
        vec![(
            "probe".into(),
            "TextProviderWin::GetSelection@impl.cc".into(),
            "medium".into()
        )]
    );
}

#[test]
fn cpp_template_member_prototype_unifies_with_its_out_of_line_body() {
    // `Cursor::name` (in-class) and `Cursor<A>::name` (out of line) are one
    // member; left apart, every bare call to it inside the class is
    // ambiguous between the two.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "cursor.hpp",
        b"template <typename A> class Cursor {\npublic:\n  int name(int x);\n  int step();\n};\ntemplate <typename A> int Cursor<A>::name(int x) { return x; }\ntemplate <typename A> int Cursor<A>::step() { return name(1); }\n",
    );
    let e = cpp_edges(tmp.path());
    assert!(
        e.contains(&(
            "Cursor<A>::step".into(),
            "Cursor<A>::name@cursor.hpp".into(),
            "high".into()
        )),
        "{e:?}"
    );
}

#[test]
fn chained_call_dropped_at_the_cap_stays_in_the_audit_when_the_inner_call_resolves() {
    // `runners.Get()->Post(1)`: both calls start at `runners`, so they
    // share a site byte. `Get` resolves; `Post` has more same-named
    // candidates than the cap and is dropped. The inner call's edge must
    // not erase the outer call's `fanout-cap-exceeded` record — a drop is
    // never silent.
    let tmp = TempDir::new().unwrap();
    let mut lib = String::new();
    for i in 0..7 {
        lib.push_str(&format!(
            "class R{i} {{\npublic:\n  void Post(int t);\n}};\nvoid R{i}::Post(int t) {{}}\n"
        ));
    }
    write(tmp.path(), "lib.cc", lib.as_bytes());
    write(
        tmp.path(),
        "use.cc",
        b"class Runners {\npublic:\n  R0* Get();\n};\nR0* Runners::Get() { return nullptr; }\nvoid go(Runners& runners) {\n  runners.Get()->Post(1);\n}\n",
    );
    let out = tmp.path().join("g.json");
    cgg()
        .args(["-t", "json", "-o"])
        .arg(&out)
        .arg(tmp.path())
        .assert()
        .success();
    let g: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&out).unwrap()).unwrap();
    let posts: Vec<_> = g["unresolved"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|u| u["name"] == "Post" && u["site_line"] == 7)
        .collect();
    assert_eq!(
        posts.len(),
        1,
        "the dropped Post call must be audited: {}",
        g["unresolved"]
    );
    assert_eq!(posts[0]["reason"]["stage"], "fanout-cap-exceeded");
}

#[test]
fn cpp_bare_call_in_a_member_prefers_the_enclosing_class() {
    // Unqualified lookup inside a member function searches the class
    // first: `get(...)` in `Reader::read` is `Reader::get`, not the
    // unrelated free `get` defined in the same file.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "r.cpp",
        b"int get() { return 1; }\nclass Reader {\npublic:\n    int get(int n) { return n; }\n    int read() { return get(2); }\n};\n",
    );
    let e = cpp_edges(tmp.path());
    assert!(
        e.contains(&(
            "Reader::read".into(),
            "Reader::get@r.cpp".into(),
            "high".into()
        )),
        "{e:?}"
    );
}

#[test]
fn cpp_typed_call_reaches_every_definition_not_only_the_last() {
    // Two translation units define Kernel::add_module. by_qn keeps one
    // of them; a typed call must still reach both, or every call lands
    // on whichever file was indexed last.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "kernel.h",
        b"class Kernel {\npublic:\n    Kernel();\n    void add_module(int);\n};\n",
    );
    write(
        tmp.path(),
        "kernel.cpp",
        b"#include \"kernel.h\"\nKernel::Kernel() {}\nvoid helper() {}\nvoid Kernel::add_module(int) { helper(); }\n",
    );
    write(
        tmp.path(),
        "stub.cpp",
        b"#include \"kernel.h\"\nvoid Kernel::add_module(int) {}\n",
    );
    write(
        tmp.path(),
        "main.cpp",
        b"#include \"kernel.h\"\nvoid init() {\n    Kernel* kernel = new Kernel();\n    kernel->add_module(1);\n}\n",
    );
    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["-t", "mermaid", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();
    let g = fs::read_to_string(&mmd).unwrap();
    let ids = |qn: &str| -> Vec<String> {
        g.lines()
            .filter_map(|l| {
                let l = l.trim();
                if l.starts_with(['C', 'N']) && l.contains(&format!("[\"{qn}\"]")) {
                    Some(l.split('[').next()?.trim().to_string())
                } else {
                    None
                }
            })
            .collect()
    };
    let init = ids("init");
    let adds = ids("Kernel::add_module");
    let helpers = ids("helper");
    assert_eq!(init.len(), 1, "init:\n{g}");
    assert_eq!(adds.len(), 2, "both definitions:\n{g}");
    // Either confidence: `-->` (high) or `-->|?|` (not high).
    let arrow = |a: &str, b: &str| {
        g.contains(&format!("{a} --> {b}")) || g.contains(&format!("{a} -->|?| {b}"))
    };
    assert!(
        adds.iter().all(|a| arrow(&init[0], a)),
        "typed call should reach both definitions:\n{g}"
    );
    assert_eq!(helpers.len(), 1, "helper:\n{g}");
    assert!(
        arrow(&adds[0], &helpers[0]) || arrow(&adds[1], &helpers[0]),
        "the real body calls helper:\n{g}"
    );
}

#[test]
fn cpp_virtual_call_reaches_overrides() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "shape.h",
        b"class Shape {\npublic:\n    virtual void draw();\n};\nclass Circle : public Shape {\npublic:\n    void draw();\n};\nclass Square : public Shape {\npublic:\n    void draw();\n};\n",
    );
    write(
        tmp.path(),
        "shape.cpp",
        b"#include \"shape.h\"\nvoid Shape::draw() {}\nvoid Circle::draw() {}\nvoid Square::draw() {}\n",
    );
    write(
        tmp.path(),
        "main.cpp",
        b"#include \"shape.h\"\nvoid render(Shape* s) { s->draw(); }\n",
    );
    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["--dynamic-dispatch", "-t", "mermaid", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();
    let g = fs::read_to_string(&mmd).unwrap();
    let node_id = |qn: &str| -> Option<String> {
        g.lines().find_map(|l| {
            let l = l.trim();
            if l.starts_with(['C', 'N']) && l.contains(&format!("[\"{qn}\"]")) {
                Some(l.split('[').next()?.trim().to_string())
            } else {
                None
            }
        })
    };
    let render = node_id("render").unwrap_or_else(|| panic!("render:\n{g}"));
    let shape = node_id("Shape::draw").unwrap_or_else(|| panic!("Shape::draw:\n{g}"));
    let circle = node_id("Circle::draw").unwrap_or_else(|| panic!("Circle::draw:\n{g}"));
    let square = node_id("Square::draw").unwrap_or_else(|| panic!("Square::draw:\n{g}"));
    assert!(g.contains(&format!("{render} --> {shape}")), "base:\n{g}");
    assert!(
        g.contains(&format!("{render} -->|dyn| {circle}")),
        "Circle override should be dyn:\n{g}"
    );
    assert!(
        g.contains(&format!("{render} -->|dyn| {square}")),
        "Square override should be dyn:\n{g}"
    );
}

#[test]
fn cpp_virtual_overrides_are_not_capped() {
    // Duck-typed fan-out stops at 5. A vtable is the real override set
    // and must not be dropped when a class has more children than that.
    let tmp = TempDir::new().unwrap();
    let mut header =
        String::from("class Shape {\npublic:\n    virtual void draw();\n};\n");
    let mut body = String::from("#include \"shape.h\"\nvoid Shape::draw() {}\n");
    for i in 0..6 {
        header.push_str(&format!(
            "class D{i} : public Shape {{\npublic:\n    void draw();\n}};\n"
        ));
        body.push_str(&format!("void D{i}::draw() {{}}\n"));
    }
    write(tmp.path(), "shape.h", header.as_bytes());
    write(tmp.path(), "shape.cpp", body.as_bytes());
    write(
        tmp.path(),
        "main.cpp",
        b"#include \"shape.h\"\nvoid render(Shape* s) { s->draw(); }\n",
    );
    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["--dynamic-dispatch", "-t", "mermaid", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();
    let g = fs::read_to_string(&mmd).unwrap();
    let node_id = |qn: &str| -> Option<String> {
        g.lines().find_map(|l| {
            let l = l.trim();
            if l.starts_with(['C', 'N']) && l.contains(&format!("[\"{qn}\"]")) {
                Some(l.split('[').next()?.trim().to_string())
            } else {
                None
            }
        })
    };
    let render = node_id("render").unwrap_or_else(|| panic!("render:\n{g}"));
    for i in 0..6 {
        let d =
            node_id(&format!("D{i}::draw")).unwrap_or_else(|| panic!("D{i}::draw:\n{g}"));
        assert!(
            g.contains(&format!("{render} -->|dyn| {d}")),
            "override D{i} missing (cap would drop it):\n{g}"
        );
    }
}

#[test]
fn cpp_non_virtual_call_does_not_fan_out_to_overrides() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "shape.h",
        b"class Shape {\npublic:\n    void draw();\n};\nclass Circle : public Shape {\npublic:\n    void draw();\n};\n",
    );
    write(
        tmp.path(),
        "shape.cpp",
        b"#include \"shape.h\"\nvoid Shape::draw() {}\nvoid Circle::draw() {}\n",
    );
    write(
        tmp.path(),
        "main.cpp",
        b"#include \"shape.h\"\nvoid render(Shape* s) { s->draw(); }\n",
    );
    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["--dynamic-dispatch", "-t", "mermaid", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();
    let g = fs::read_to_string(&mmd).unwrap();
    let node_id = |qn: &str| -> Option<String> {
        g.lines().find_map(|l| {
            let l = l.trim();
            if l.starts_with(['C', 'N']) && l.contains(&format!("[\"{qn}\"]")) {
                Some(l.split('[').next()?.trim().to_string())
            } else {
                None
            }
        })
    };
    let render = node_id("render").unwrap_or_else(|| panic!("render:\n{g}"));
    let shape = node_id("Shape::draw").unwrap_or_else(|| panic!("Shape::draw:\n{g}"));
    let circle = node_id("Circle::draw").unwrap_or_else(|| panic!("Circle::draw:\n{g}"));
    assert!(g.contains(&format!("{render} --> {shape}")), "base:\n{g}");
    assert!(
        !g.contains(&format!("{render} --> {circle}")),
        "non-virtual must not fan out from the call site:\n{g}"
    );
    assert!(
        !g.contains(&format!("{shape} -->|dyn| {circle}")),
        "non-virtual C++ must not grow a declaration→override dyn edge:\n{g}"
    );
    assert_eq!(
        g.matches("-->|dyn|").count(),
        0,
        "this fixture has no virtual slot; dyn edges would be a language leak:\n{g}"
    );
}

#[test]
fn cpp_typed_derived_pointer_does_not_reach_sibling_overrides() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "shape.h",
        b"class Shape {\npublic:\n    virtual void draw();\n};\nclass Circle : public Shape {\npublic:\n    void draw();\n};\nclass Square : public Shape {\npublic:\n    void draw();\n};\n",
    );
    write(
        tmp.path(),
        "shape.cpp",
        b"#include \"shape.h\"\nvoid Shape::draw() {}\nvoid Circle::draw() {}\nvoid Square::draw() {}\n",
    );
    write(
        tmp.path(),
        "main.cpp",
        b"#include \"shape.h\"\nvoid render(Circle* c) { c->draw(); }\n",
    );
    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["-t", "mermaid", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();
    let g = fs::read_to_string(&mmd).unwrap();
    let node_id = |qn: &str| -> Option<String> {
        g.lines().find_map(|l| {
            let l = l.trim();
            if l.starts_with(['C', 'N']) && l.contains(&format!("[\"{qn}\"]")) {
                Some(l.split('[').next()?.trim().to_string())
            } else {
                None
            }
        })
    };
    let render = node_id("render").unwrap_or_else(|| panic!("render:\n{g}"));
    let circle = node_id("Circle::draw").unwrap_or_else(|| panic!("Circle::draw:\n{g}"));
    let square = node_id("Square::draw").unwrap_or_else(|| panic!("Square::draw:\n{g}"));
    assert!(
        g.contains(&format!("{render} --> {circle}")),
        "typed Circle* should call Circle::draw:\n{g}"
    );
    assert!(
        !g.contains(&format!("{render} --> {square}"))
            && !g.contains(&format!("{render} -->|dyn| {square}")),
        "sibling Square::draw must not be reached:\n{g}"
    );
}

#[test]
fn cpp_virtual_dyn_edges_are_opt_in_behind_dynamic_dispatch() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "shape.h",
        b"class Shape {\npublic:\n    virtual void draw();\n};\nclass Circle : public Shape {\npublic:\n    void draw();\n};\n",
    );
    write(
        tmp.path(),
        "shape.cpp",
        b"#include \"shape.h\"\nvoid Shape::draw() {}\nvoid Circle::draw() {}\n",
    );
    write(
        tmp.path(),
        "main.cpp",
        b"#include \"shape.h\"\nvoid render(Shape* s) { s->draw(); }\n",
    );
    let run = |name: &str, extra: &[&str]| {
        let out = tmp.path().join(name);
        let mut cmd = cgg();
        cmd.args(["-t", "mermaid", "-o"]).arg(&out);
        cmd.args(extra);
        cmd.arg(tmp.path()).assert().success();
        fs::read_to_string(&out)
            .unwrap()
            .matches("-->|dyn|")
            .count()
    };
    let plain = run("p.mmd", &[]);
    let flagged = run("d.mmd", &["--dynamic-dispatch"]);
    assert_eq!(
        plain, 0,
        "C++ vtable fan-out is opt-in: the default graph carries no dyn edges"
    );
    assert!(
        flagged >= 1,
        "--dynamic-dispatch must add the C++ vtable edges"
    );
}

#[test]
fn cpp_member_pointer_table_reaches_overrides() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "module.h",
        b"class Module {\npublic:\n    virtual void on_idle(void*);\n    virtual void on_tick(void*);\n};\nclass Robot : public Module {\npublic:\n    void on_idle(void*);\n};\ntypedef void (Module::*CB)(void*);\nextern const CB table[2];\n",
    );
    write(
        tmp.path(),
        "module.cpp",
        b"#include \"module.h\"\nconst CB table[2] = { &Module::on_idle, &Module::on_tick };\nvoid Module::on_idle(void*) {}\nvoid Module::on_tick(void*) {}\nvoid Robot::on_idle(void*) {}\n",
    );
    write(
        tmp.path(),
        "kernel.cpp",
        b"#include \"module.h\"\nvoid call_event(Module* m, int i) { (m->*table[i])(0); }\n",
    );
    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["--dynamic-dispatch", "-t", "mermaid", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();
    let g = fs::read_to_string(&mmd).unwrap();
    let node_id = |qn: &str| -> Option<String> {
        g.lines().find_map(|l| {
            let l = l.trim();
            if l.starts_with(['C', 'N']) && l.contains(&format!("[\"{qn}\"]")) {
                Some(l.split('[').next()?.trim().to_string())
            } else {
                None
            }
        })
    };
    let call = node_id("call_event").unwrap_or_else(|| panic!("call_event:\n{g}"));
    let idle = node_id("Module::on_idle").unwrap_or_else(|| panic!("on_idle:\n{g}"));
    let tick = node_id("Module::on_tick").unwrap_or_else(|| panic!("on_tick:\n{g}"));
    let robot =
        node_id("Robot::on_idle").unwrap_or_else(|| panic!("Robot::on_idle:\n{g}"));
    assert!(
        g.contains(&format!("{call} -->|dyn| {idle}")),
        "table on_idle:\n{g}"
    );
    assert!(
        g.contains(&format!("{call} -->|dyn| {tick}")),
        "table on_tick:\n{g}"
    );
    assert!(
        g.contains(&format!("{call} -->|dyn| {robot}")),
        "override of table take:\n{g}"
    );
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
                if l.starts_with(['C', 'N']) && l.contains(&format!("[\"{qn}\"]")) {
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
                if l.starts_with(['C', 'N']) && l.contains(&format!("[\"{qn}\"]")) {
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
                if l.starts_with(['C', 'N']) && l.contains(&format!("[\"{qn}\"]")) {
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
                if l.starts_with(['C', 'N']) && l.contains(&format!("[\"{qn}\"]")) {
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
fn python_inheritance_fanout_is_opt_in() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "ops.py",
        b"class OperationsBase:\n    def generate(self):\n        return 1\n\
class BoreOperation(OperationsBase):\n    def generate(self):\n        return 2\n",
    );

    let plain = tmp.path().join("p.mmd");
    cgg()
        .args(["-o"])
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

    let dyn_out = tmp.path().join("d.mmd");
    cgg()
        .args(["--dynamic-dispatch", "-o"])
        .arg(&dyn_out)
        .arg(tmp.path())
        .assert()
        .success();
    let g = fs::read_to_string(&dyn_out).unwrap();
    let base = mermaid_id(&g, "ops.OperationsBase.generate")
        .unwrap_or_else(|| panic!("ops.OperationsBase.generate:\n{g}"));
    let child = mermaid_id(&g, "ops.BoreOperation.generate")
        .unwrap_or_else(|| panic!("ops.BoreOperation.generate:\n{g}"));
    assert!(
        g.contains(&format!("{base} -->|dyn| {child}")),
        "expected OperationsBase.generate -->|dyn| BoreOperation.generate:\n{g}"
    );
}

#[test]
fn python_bare_call_binds_by_lexical_scope() {
    // A function nested in a method is a function, visible only inside
    // that method; the innermost definition shadows the module's.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "m.py",
        b"def _scan(x):\n    return x\n\
class A:\n    def one(self):\n        def _scan(x):\n            return x\n        return _scan(1)\n    def two(self):\n        return _scan(2)\n\
class _Private:\n    def build(self):\n        def _walk(x):\n            return x\n        return _walk(1)\n",
    );
    let graph = tmp.path().join("g.json");
    cgg()
        .args(["-t", "json", "-o"])
        .arg(&graph)
        .arg(tmp.path())
        .assert()
        .success();
    let g: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&graph).unwrap()).unwrap();
    let c = g["callables"].as_object().unwrap();
    let qn = |id: &serde_json::Value| {
        c[id.as_str().unwrap()]["qualified_name"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let mut edges: Vec<(String, String, String)> = g["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| !qn(&e["src"]).starts_with('<'))
        .map(|e| {
            (
                qn(&e["src"]),
                qn(&e["dst"]),
                e["confidence"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    edges.sort();
    let want: Vec<(String, String, String)> = [
        ("m.A.one", "m.A.one._scan"),
        ("m.A.two", "m._scan"),
        ("m._Private.build", "m._Private.build._walk"),
    ]
    .iter()
    .map(|(a, b)| (a.to_string(), b.to_string(), "high".to_string()))
    .collect();
    assert_eq!(edges, want);
}

#[test]
fn python_override_through_a_bodiless_intermediate_class_is_live() {
    // The template-method shape from a reachability audit: the base
    // calls `self.execute`, the override sits two levels down behind
    // `class Middle(BaseStep): pass`.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "steps.py",
        b"class BaseStep:\n    def run(self, data):\n        return self.execute(data)\n    def execute(self, data):\n        raise NotImplementedError\n\
class Middle(BaseStep):\n    pass\n\
class ScoreStep(Middle):\n    def execute(self, data):\n        return data\n\
def main():\n    s = ScoreStep()\n    return s.run(1)\nmain()\n",
    );

    let graph = tmp.path().join("g.json");
    cgg()
        .args(["--dynamic-dispatch", "-t", "json", "-o"])
        .arg(&graph)
        .arg(tmp.path())
        .assert()
        .success();
    let g: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&graph).unwrap()).unwrap();
    let callables = g["callables"].as_object().unwrap();
    let qn = |id: &serde_json::Value| {
        callables[id.as_str().unwrap()]["qualified_name"]
            .as_str()
            .unwrap()
            .to_string()
    };
    // `s: ScoreStep` inherits `run` through the bodiless class, so the
    // typed-receiver base walk must reach `BaseStep.run` in the default
    // graph — a resolved edge, not a low-confidence guess.
    assert!(
        g["edges"].as_array().unwrap().iter().any(|e| {
            qn(&e["src"]) == "steps.main"
                && qn(&e["dst"]) == "steps.BaseStep.run"
                && e["confidence"] != "low"
                && e["via"]["kind"] == "direct"
        }),
        "main -> BaseStep.run through Middle is missing"
    );
    let dyn_edges: Vec<(String, String, String)> = g["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["via"]["kind"] == "dynamic")
        .map(|e| {
            (
                qn(&e["src"]),
                qn(&e["dst"]),
                e["resolver"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    assert_eq!(
        dyn_edges,
        vec![(
            "steps.BaseStep.execute".to_string(),
            "steps.ScoreStep.execute".to_string(),
            "dispatch:inheritance".to_string()
        )],
        "one inheritance edge through the bodiless class, got {dyn_edges:?}"
    );
    // Inheritance is not a trait impl: Python nodes carry no
    // `trait_impl_target`, so `dispatch:fanout` never double-labels
    // the same edge and the JSON shape for Python is unchanged.
    assert!(
        callables
            .values()
            .all(|c| c.get("trait_impl_target").is_none()),
        "Python nodes must not carry trait_impl_target"
    );

    let report = tmp.path().join("dead.json");
    cgg()
        .args([
            "--dead-code",
            "--no-graph",
            "--dead-code-format",
            "json",
            "--dead-code-report",
        ])
        .arg(&report)
        .arg(tmp.path())
        .assert()
        .success();
    let parsed: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&report).unwrap()).unwrap();
    let qns: Vec<&str> = parsed["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["qualified_name"].as_str())
        .collect();
    assert!(
        !qns.iter().any(|q| q.contains("ScoreStep.execute")),
        "ScoreStep.execute is reached through Middle; findings: {qns:?}"
    );
}

#[test]
fn python_subclass_override_is_live_when_base_method_is() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "ops.py",
        b"class OperationsBase:\n    def generate(self):\n        return 1\n\
class BoreOperation(OperationsBase):\n    def generate(self):\n        return 2\n    def unused(self):\n        return 3\n\
def entry():\n    x = OperationsBase()\n    return x.generate()\nentry()\n",
    );

    let report = tmp.path().join("dead.json");
    cgg()
        .args([
            "--dead-code",
            "--no-graph",
            "--dead-code-format",
            "json",
            "--dead-code-report",
        ])
        .arg(&report)
        .arg(tmp.path())
        .assert()
        .success();

    let parsed: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&report).unwrap()).unwrap();
    let qns: Vec<&str> = parsed["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["qualified_name"].as_str())
        .collect();
    assert!(
        !qns.iter().any(|q| q.contains("BoreOperation.generate")),
        "subclass generate should be live via inheritance fan-out, findings: {qns:?}"
    );
    assert!(
        qns.iter().any(|q| q.contains("BoreOperation.unused")),
        "unused should still be reported, findings: {qns:?}"
    );
}

#[test]
fn python_override_walks_past_a_base_that_does_not_define_the_method() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "ops.py",
        b"class A:\n    def foo(self):\n        return 1\n\
class B(A):\n    def other(self):\n        return 0\n\
class C(B):\n    def foo(self):\n        return 2\n    def unused(self):\n        return 3\n\
def entry():\n    x = A()\n    return x.foo()\nentry()\n",
    );

    let dyn_out = tmp.path().join("d.mmd");
    cgg()
        .args(["--dynamic-dispatch", "-o"])
        .arg(&dyn_out)
        .arg(tmp.path())
        .assert()
        .success();
    let g = fs::read_to_string(&dyn_out).unwrap();
    let a = mermaid_id(&g, "ops.A.foo").unwrap_or_else(|| panic!("ops.A.foo:\n{g}"));
    let c = mermaid_id(&g, "ops.C.foo").unwrap_or_else(|| panic!("ops.C.foo:\n{g}"));
    assert!(
        g.contains(&format!("{a} -->|dyn| {c}")),
        "C.foo overrides A.foo through B, which does not define foo:\n{g}"
    );

    let report = tmp.path().join("dead.json");
    cgg()
        .args([
            "--dead-code",
            "--no-graph",
            "--dead-code-format",
            "json",
            "--dead-code-report",
        ])
        .arg(&report)
        .arg(tmp.path())
        .assert()
        .success();
    let qns = dead_code_qns(&report);
    assert!(
        !qns.iter().any(|q| q.contains(".C.foo")),
        "C.foo should be live via skip-a-generation fan-out, findings: {qns:?}"
    );
    assert!(
        qns.iter().any(|q| q.contains("C.unused")),
        "unused should still be reported, findings: {qns:?}"
    );
}

#[test]
fn python_private_class_override_is_live_when_base_method_is() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "tips.py",
        b"class ToolTipButton:\n    def on_mouse_pos(self):\n        return 1\n\
class _MarkerHoverToolTip(ToolTipButton):\n    def on_mouse_pos(self):\n        return 2\n\
def entry():\n    x = ToolTipButton()\n    return x.on_mouse_pos()\nentry()\n",
    );

    let report = tmp.path().join("dead.json");
    cgg()
        .args([
            "--dead-code",
            "--no-graph",
            "--dead-code-format",
            "json",
            "--dead-code-report",
        ])
        .arg(&report)
        .arg(tmp.path())
        .assert()
        .success();

    let parsed: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&report).unwrap()).unwrap();
    let qns: Vec<&str> = parsed["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["qualified_name"].as_str())
        .collect();
    assert!(
        !qns.iter()
            .any(|q| q.contains("_MarkerHoverToolTip.on_mouse_pos")),
        "private-class override should be live via inheritance fan-out, findings: {qns:?}"
    );
}

#[test]
fn python_self_field_constructor_resolves_the_method_call() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "controller.py",
        b"class Controller:\n    def open(self):\n        return 1\n    def unused(self):\n        return 0\n",
    );
    write(
        tmp.path(),
        "app.py",
        b"from controller import Controller\n\
class App:\n    def __init__(self):\n        self.controller = Controller()\n    def connect(self):\n        return self.controller.open()\n\
def entry():\n    app = App()\n    return app.connect()\nentry()\n",
    );

    let report = tmp.path().join("dead.json");
    cgg()
        .args([
            "--dead-code",
            "--no-graph",
            "--dead-code-format",
            "json",
            "--dead-code-report",
        ])
        .arg(&report)
        .arg(tmp.path())
        .assert()
        .success();

    let parsed: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&report).unwrap()).unwrap();
    let qns: Vec<&str> = parsed["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["qualified_name"].as_str())
        .collect();
    assert!(
        !qns.iter().any(|q| q.contains("Controller.open")),
        "Controller.open should be live via self.controller, findings: {qns:?}"
    );
    assert!(
        qns.iter().any(|q| q.contains("Controller.unused")),
        "unused should still be reported, findings: {qns:?}"
    );
}

#[test]
fn python_class_annotation_types_self_field_calls() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "ctrl.py",
        b"class Controller:\n    def open(self):\n        return 1\n",
    );
    write(
        tmp.path(),
        "panel.py",
        b"from ctrl import Controller\n\
class Panel:\n    controller: Controller\n    def __init__(self, controller):\n        self.controller = controller\n    def go(self):\n        return self.controller.open()\n\
def entry():\n    p = Panel(Controller())\n    return p.go()\nentry()\n",
    );

    let report = tmp.path().join("dead.json");
    cgg()
        .args([
            "--dead-code",
            "--no-graph",
            "--dead-code-format",
            "json",
            "--dead-code-report",
        ])
        .arg(&report)
        .arg(tmp.path())
        .assert()
        .success();

    let parsed: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&report).unwrap()).unwrap();
    let qns: Vec<&str> = parsed["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["qualified_name"].as_str())
        .collect();
    assert!(
        !qns.iter().any(|q| q.contains("Controller.open")),
        "Controller.open should be live via annotated self.controller, findings: {qns:?}"
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
        fs::read_to_string(&plain).unwrap().matches("ref| ").count(),
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
        g.matches("ref| ").count(),
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
            l.starts_with(['C', 'N'])
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

/// Duck-typed fan-out is narrowed by what a candidate can accept.
///
/// The field report's example: a call passing `data=`/`context=` fanned
/// out to four same-named `evaluate` methods, three of which accept
/// neither keyword and require four others.
#[test]
fn fanout_is_narrowed_by_keyword_compatibility() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "impls.py",
        b"class A:\n    def evaluate(self, data=None, context=None):\n        return data\n\
          class B:\n    def evaluate(self, w, x, y, z):\n        return w\n\
          class C:\n    def evaluate(self, w, x, y, z):\n        return w\n",
    );
    write(
        tmp.path(),
        "use.py",
        b"def go(obj):\n    return obj.evaluate(data=1, context=2)\n",
    );
    let g = graph_of(tmp.path());
    // B and C are still nodes — they are real definitions. What they
    // must not be is targets of this call.
    let targets: Vec<String> = g
        .lines()
        .filter_map(|l| l.split("-->").nth(1))
        .map(|t| {
            // Past any `|label|` (the `?` confidence mark, a count).
            let id = t.trim().rsplit(' ').next().unwrap_or("");
            g.lines()
                .find(|n| n.trim().starts_with(&format!("{id}[")))
                .unwrap_or("")
                .to_string()
        })
        .collect();
    let joined = targets.join("\n");
    assert!(
        joined.contains("A.evaluate"),
        "the compatible one must resolve:\n{g}"
    );
    assert!(
        !joined.contains("B.evaluate"),
        "B accepts neither keyword:\n{g}"
    );
    assert!(
        !joined.contains("C.evaluate"),
        "C accepts neither keyword:\n{g}"
    );
}

/// Narrowing never removes the last candidate.
///
/// One-sided by design: a keyword no candidate accepts is evidence that
/// cgg's picture is incomplete, not licence to drop every edge.
#[test]
fn a_keyword_matching_nothing_does_not_erase_the_fanout() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "impls.py",
        b"class A:\n    def run(self, w):\n        return w\n\
          class B:\n    def run(self, w):\n        return w\n",
    );
    write(
        tmp.path(),
        "use.py",
        b"def go(obj):\n    return obj.run(nonexistent=1)\n",
    );
    let g = graph_of(tmp.path());
    assert!(
        g.contains("A.run") && g.contains("B.run"),
        "both candidates should survive:\n{g}"
    );
}

/// A name imported in a parenthesised `from … import (…)` block — the
/// reporter's shape: first name unaliased, later names aliased,
/// multi-line, trailing comma — must bind through the import, at high
/// confidence, and must not fall to the same-name fan-out. With seven
/// same-named definitions elsewhere the fan-out would exceed the cap and
/// the callee would be reported unreferenced although it is called.
#[test]
fn parenthesised_from_import_binds_at_high_confidence_past_the_fanout_cap() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "client.py",
        b"def create_remediation_request(payload):\n    return payload\n\ndef _create_client(stage):\n    return stage\n",
    );
    for i in 0..7 {
        write(
            tmp.path(),
            &format!("dup{i}.py"),
            b"def create_remediation_request(payload):\n    return -payload\n",
        );
    }
    write(
        tmp.path(),
        "caller.py",
        b"from client import (\n    create_remediation_request,\n    _create_client as _create_local_client,\n)\n\ndef go(payload):\n    return create_remediation_request(payload)\n",
    );

    let out = tmp.path().join("g.json");
    cgg()
        .args(["-t", "json", "-o"])
        .arg(&out)
        .arg(tmp.path())
        .assert()
        .success();
    let g: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&out).unwrap()).unwrap();
    let callables = g["callables"].as_object().unwrap();
    let qn = |id: &str| {
        callables[id]["qualified_name"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let edges: Vec<(String, String, String)> = g["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                qn(e["src"].as_str().unwrap()),
                qn(e["dst"].as_str().unwrap()),
                e["confidence"].as_str().unwrap().to_string(),
            )
        })
        .filter(|(s, _, _)| s == "caller.go")
        .collect();
    assert_eq!(
        edges,
        vec![(
            "caller.go".to_string(),
            "client.create_remediation_request".to_string(),
            "high".to_string()
        )],
        "the parenthesised import must bind exactly the imported definition: {edges:?}"
    );

    let report = cgg()
        .args(["--report-unreferenced"])
        .arg(tmp.path())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&report.stdout).to_string()
        + &String::from_utf8_lossy(&report.stderr);
    assert!(
        !text.contains("client.create_remediation_request"),
        "a called function must not be reported unreferenced:\n{text}"
    );
}

#[test]
fn c_parent_relative_include_resolves_to_the_named_header() {
    // `#include "../common/util.h"` from `src/` names `common/util.h`.
    // Unnormalised, the joined path `src/../common/util.h` matched no
    // file and the `..` defeated the suffix fallback, so the call went
    // unresolved. The decoy shares the suffix and sorts first; the exact
    // path must still win.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "aaa/common/util.h",
        b"int twice(int x) { return x + x; }\n",
    );
    write(
        tmp.path(),
        "common/util.h",
        b"int twice(int x) { return 2 * x; }\n",
    );
    write(
        tmp.path(),
        "src/main.c",
        b"#include \"../common/util.h\"\nint run(void) { return twice(3); }\n",
    );
    let out = tmp.path().join("g.json");
    cgg()
        .args(["-t", "json", "-o"])
        .arg(&out)
        .arg(tmp.path())
        .assert()
        .success();
    let g: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&out).unwrap()).unwrap();
    let path_of = |id: &str| {
        let c = &g["callables"][id];
        g["files"][c["file"].as_str().unwrap()]["path"]
            .as_str()
            .unwrap()
            .replace('\\', "/")
    };
    let targets: Vec<String> = g["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| g["callables"][e["src"].as_str().unwrap()]["simple_name"] == "run")
        .map(|e| path_of(e["dst"].as_str().unwrap()))
        .collect();
    assert_eq!(targets.len(), 1, "{targets:?}");
    assert!(
        targets[0].ends_with("common/util.h") && !targets[0].contains("aaa"),
        "{targets:?}"
    );
}

#[test]
fn angle_include_follows_an_in_tree_header_but_not_a_vendored_libc() {
    // `#include <helper_cuda.h>` with `-I Common` names a header in the
    // tree; the call binds to it. `<stdio.h>` is the toolchain's even
    // when the tree vendors a copy, so `printf` must not bind there.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "Common/helper_cuda.h",
        b"static inline int checkErrors(int e) { return e; }\n",
    );
    write(
        tmp.path(),
        "vendor/libc/stdio.h",
        b"int printf(const char *f) { return 0; }\n",
    );
    write(
        tmp.path(),
        "samples/main.c",
        b"#include <stdio.h>\n#include <helper_cuda.h>\nint run(void) { printf(\"x\"); return checkErrors(1); }\n",
    );
    let out = tmp.path().join("g.json");
    cgg()
        .args(["-t", "json", "-o"])
        .arg(&out)
        .arg(tmp.path())
        .assert()
        .success();
    let g: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&out).unwrap()).unwrap();
    let targets: Vec<&str> = g["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| g["callables"][e["src"].as_str().unwrap()]["simple_name"] == "run")
        .map(|e| {
            g["callables"][e["dst"].as_str().unwrap()]["simple_name"]
                .as_str()
                .unwrap()
        })
        .collect();
    assert_eq!(targets, vec!["checkErrors"], "{targets:?}");
}

/// Edges out of `src_qn` (by destination qualified name) and the
/// external-bucket records of the run, from `-t json -o` and its audit.
fn edges_and_external(dir: &Path, src_qn: &str) -> (Vec<String>, Vec<(String, String)>) {
    let out = dir.join("g.json");
    cgg()
        .args(["-t", "json", "-o"])
        .arg(&out)
        .arg(dir)
        .assert()
        .success();
    let g: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&out).unwrap()).unwrap();
    let qn = |id: &serde_json::Value| {
        g["callables"][id.as_str().unwrap()]["qualified_name"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let mut dsts: Vec<String> = g["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| qn(&e["src"]) == src_qn)
        .map(|e| qn(&e["dst"]))
        .collect();
    dsts.sort();
    let audit: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("g.json.audit.json")).unwrap())
            .unwrap();
    let mut ext = Vec::new();
    for ev in audit.as_array().unwrap() {
        for u in ev["external_calls"].as_array().into_iter().flatten() {
            if u["reason"]["stage"] == "external-module" {
                ext.push((
                    u["name"].as_str().unwrap().to_string(),
                    u["reason"]["detail"]["module"]
                        .as_str()
                        .unwrap()
                        .to_string(),
                ));
            }
        }
        assert!(
            ev["unresolved_calls"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|u| u["reason"]["stage"] != "external-module"),
            "an external-module site is filed as external, never unresolved"
        );
    }
    ext.sort();
    (dsts, ext)
}

/// FINDINGS A1, end to end: a Go import outside the tree's `go.mod`
/// module is a dependency — `uuid.NewString()` must not bind to a
/// workspace `IDs.NewString` — while a same-module package resolves.
#[test]
fn go_call_into_a_module_outside_go_mod_is_external() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "go.mod", b"module example.com/app\n\ngo 1.21\n");
    write(
        tmp.path(),
        "util/util.go",
        b"package util\n\ntype IDs struct{}\n\nfunc (i IDs) NewString() string { return \"\" }\n",
    );
    write(
        tmp.path(),
        "jobs/jobs.go",
        b"package jobs\n\nfunc Spawn() int { return 1 }\n",
    );
    write(
        tmp.path(),
        "main.go",
        b"package main\n\nimport (\n\t\"example.com/app/jobs\"\n\t\"github.com/google/uuid\"\n)\n\n\
          func Run() string {\n\tjobs.Spawn()\n\treturn uuid.NewString()\n}\n",
    );
    let (dsts, ext) = edges_and_external(tmp.path(), "main.Run");
    assert_eq!(dsts, vec!["jobs.Spawn".to_string()]);
    assert_eq!(
        ext,
        vec![(
            "NewString".to_string(),
            "github.com/google/uuid".to_string()
        )]
    );
}

/// FINDINGS A1, end to end: an npm dependency's default export is not a
/// workspace function of the same name; a monorepo package (named by a
/// `package.json` in the tree) still is the workspace's.
#[test]
fn js_dependency_call_is_external_but_a_workspace_package_resolves() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "packages/net/package.json",
        b"{\"name\": \"@acme/net\", \"version\": \"1.0.0\"}",
    );
    write(
        tmp.path(),
        "packages/net/index.js",
        b"export function fetchAll(u) { return u; }\n",
    );
    write(
        tmp.path(),
        "app/util.js",
        b"export function post(u) { return u; }\n",
    );
    write(
        tmp.path(),
        "app/main.js",
        b"import axios from 'axios';\nimport net from '@acme/net';\n\
          export function run() {\n  axios.post('/u');\n  net.fetchAll('/v');\n}\n",
    );
    let (dsts, ext) = edges_and_external(tmp.path(), "run");
    assert_eq!(dsts, vec!["fetchAll".to_string()]);
    assert_eq!(ext, vec![("post".to_string(), "axios".to_string())]);
}

/// FINDINGS A2, end to end: a generic parameter is typed as the generic,
/// never as one of its type arguments. `Map<String, Handler> h` typed `h`
/// as `Handler` (the piece after the comma) and `h.get()` bound to
/// `Handler.get` at high; `Repo<String, Integer> r` typed `r` as `Integer`.
#[test]
fn a_generic_parameter_is_never_typed_as_its_type_argument() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "p/A.java",
        concat!(
            "package p;\nimport java.util.Map;\n",
            "class Handler { Object get(String k) { return null; } }\n",
            "class Repo<K, V> { int quux() { return 1; } }\n",
            "class Other { int quux() { return 2; } }\n",
            "public class A {\n    void lookup(Map<String, Handler> h) { h.get(\"a\"); }\n",
            "    void typed(Repo<String, Integer> r) { r.quux(); }\n}\n",
        )
        .as_bytes(),
    );
    let (lookup, _) = edges_and_external(tmp.path(), "p.A.lookup");
    assert!(lookup.is_empty(), "Map.get is not Handler.get: {lookup:?}");
    let (typed, _) = edges_and_external(tmp.path(), "p.A.typed");
    assert_eq!(typed, vec!["p.Repo.quux"]);
}

/// FINDINGS A7, end to end: a receiver chain wrapped over several lines
/// resolves exactly like the one-line form — through the self-field type,
/// not by duck typing onto every same-named method.
#[test]
fn a_wrapped_python_self_chain_keeps_its_field_type() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "a.py",
        concat!(
            "class Repo:\n    def quux(self): return 1\n",
            "class Other:\n    def quux(self): return 2\n",
            "class A:\n    def __init__(self):\n        self.repo = Repo()\n",
            "    def multi(self):\n        (self\n            .repo\n            .quux())\n",
        )
        .as_bytes(),
    );
    let (dsts, _) = edges_and_external(tmp.path(), "a.A.multi");
    assert_eq!(dsts, vec!["a.Repo.quux"]);
}

/// FINDINGS A3, end to end: a Kotlin call with a receiver other than a
/// plain identifier is a receiver call, never a bare one. A constructed
/// receiver is its type; `super` is the base; a package-qualified call
/// and a call on a constructed dependency type are external.
#[test]
fn kotlin_receivers_are_never_read_as_bare_calls() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "src/Model.kt",
        concat!(
            "package com.acme.model\nopen class Base { open fun greet() { } }\n",
            "class Widget { fun paint() { } }\n",
            "object Registry {\n    fun lookup(k: String): Widget = Widget()\n}\n",
            "fun runBlocking(b: () -> Unit) { }\n",
            "class Texts { fun readText(): String = \"\" }\n",
        )
        .as_bytes(),
    );
    write(
        tmp.path(),
        "src/App.kt",
        concat!(
            "package com.acme.app\nimport com.acme.model.Base\nimport com.acme.model.Widget\n",
            "import com.acme.model.Registry\nimport java.io.File\nimport kotlinx.coroutines.delay\n",
            "class Screen : Base() {\n    fun paint() { }\n    override fun greet() {\n",
            "        super.greet()\n        Widget().paint()\n",
            "        Registry.lookup(\"b\").paint()\n",
            "        kotlinx.coroutines.runBlocking { }\n        File(\"x\").readText()\n",
            "    }\n}\n",
        )
        .as_bytes(),
    );
    let (dsts, ext) = edges_and_external(tmp.path(), "com.acme.app.Screen.greet");
    assert_eq!(
        dsts,
        vec![
            "com.acme.model.Base.greet",
            "com.acme.model.Registry.lookup",
            "com.acme.model.Widget",
            "com.acme.model.Widget.paint",
        ],
        "no edge to Screen.paint / Screen.greet / the workspace runBlocking or readText"
    );
    assert!(
        ext.contains(&("runBlocking".into(), "kotlinx.coroutines".into())),
        "{ext:?}"
    );
    assert!(
        ext.contains(&("readText".into(), "java.io".into())),
        "{ext:?}"
    );
}

/// `super.m()` resolves through the caller's declared bases — across a
/// bodiless intermediate class — and, in a language whose plugin records
/// every class, a chain that leaves the tree is refused instead of being
/// name-matched onto unrelated workspace methods.
#[test]
fn super_resolves_through_the_declared_base_chain() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "App.kt",
        concat!(
            "package app\nimport android.app.Application\n",
            "class Other { fun onCreate() { } }\n",
            "class MyApp : Application() {\n    override fun onCreate() { super.onCreate() }\n}\n",
            "open class Base { open fun start() { } }\nopen class Mid : Base()\n",
            "class Leaf : Mid() {\n    override fun start() { super.start() }\n}\n",
        )
        .as_bytes(),
    );
    write(
        tmp.path(),
        "net.py",
        concat!(
            "import torch.nn as nn\nclass Other:\n    def forward(self, x): return x\n",
            "class Net(nn.Module):\n    def forward(self, x):\n        return super().forward(x)\n",
        )
        .as_bytes(),
    );
    let (my_app, _) = edges_and_external(tmp.path(), "app.MyApp.onCreate");
    assert!(my_app.is_empty(), "external base: {my_app:?}");
    let (leaf, _) = edges_and_external(tmp.path(), "app.Leaf.start");
    assert_eq!(leaf, vec!["app.Base.start"]);
    let (net, _) = edges_and_external(tmp.path(), "net.Net.forward");
    assert!(net.is_empty(), "external base: {net:?}");
}

/// Java and C# record every class, so `super.m()` / `base.M()` whose
/// chain leaves the tree is refused rather than bound by name onto an
/// unrelated workspace `m`; an in-tree base still binds.
#[test]
fn java_and_csharp_super_into_an_outside_base_is_refused() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "q/E.java",
        b"package q;\nimport org.other.*;\npublic class E extends Widget {\n    @Override public void stop() { super.stop(); }\n}\n",
    );
    write(
        tmp.path(),
        "q/B.java",
        b"package q;\npublic class B { public void stop() { } }\nclass C extends B {\n    @Override public void stop() { super.stop(); }\n}\n",
    );
    write(
        tmp.path(),
        "W.cs",
        b"namespace App {\n  class W : ExternalBase {\n    protected override void Layout() { base.Layout(); }\n  }\n  class Other { internal void Layout() { } }\n}\n",
    );
    let (e, _) = edges_and_external(tmp.path(), "q.E.stop");
    assert!(e.is_empty(), "outside base: {e:?}");
    let (c, _) = edges_and_external(tmp.path(), "q.C.stop");
    assert_eq!(c, vec!["q.B.stop"]);
    let (w, _) = edges_and_external(tmp.path(), "App.W.Layout");
    assert!(w.is_empty(), "outside base: {w:?}");
    // An anonymous subclass's `super` is its own base, not the enclosing
    // class's (`F` extends nothing).
    write(
        tmp.path(),
        "q/F.java",
        b"package q;\npublic class F {\n    void go() {\n        B b = new B() {\n            @Override public void stop() { super.stop(); }\n        };\n    }\n}\n",
    );
    let (f, _) = edges_and_external(tmp.path(), "q.F.stop");
    assert_eq!(f, vec!["q.B.stop"]);
}

/// Dead code: an override reached only by dispatch from a called base
/// declaration is live; an override of an outside contract is never in
/// the top band; an unrelated private method still is.
#[test]
fn dispatch_through_declared_bases_keeps_overrides_out_of_the_top_band() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "src/p/V.java",
        concat!(
            "package p;\nimport org.thirdparty.Handler;\n",
            "abstract class Validator { abstract String member(int x); }\n",
            "final class Input extends Validator { @Override String member(int x) { return \"i\"; } }\n",
            "class Ext extends Handler { @Override void handle(int c) { } }\n",
            "public class V {\n    public static void main(String[] a) { Validator v = pick(); v.member(1); }\n",
            "    private static Validator pick() { return null; }\n    private void orphan() { }\n}\n",
        )
        .as_bytes(),
    );
    let report = tmp.path().join("dead.json");
    cgg()
        .args(["--no-graph", "--dead-code", "--dead-code-format", "json"])
        .arg("--dead-code-report")
        .arg(&report)
        .arg(tmp.path())
        .assert()
        .success();
    let parsed: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&report).unwrap()).unwrap();
    let conf = |qn: &str| {
        parsed["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["qualified_name"] == qn)
            .map(|f| f["confidence"].as_str().unwrap().to_string())
    };
    assert_eq!(conf("p.Input.member"), None, "dispatched to");
    assert_eq!(conf("p.Ext.handle").as_deref(), Some("medium"));
    assert_eq!(conf("p.V.orphan").as_deref(), Some("high"));
}

/// A Kotlin extension function in scope — declared in the calling file or
/// imported by name — is what `expr.ext()` calls, whatever the receiver
/// expression; a receiver of a known type that declares the member keeps
/// the member.
#[test]
fn kotlin_extension_calls_resolve_through_their_scope() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "src/Ext.kt",
        concat!(
            "package com.acme.util\nfun String.shout(): String = this\n",
            "class Other { fun shout(): String = \"\" }\n",
        )
        .as_bytes(),
    );
    write(
        tmp.path(),
        "src/Use.kt",
        concat!(
            "package com.acme.app\nimport com.acme.util.shout\n",
            "class Box { fun firstUpper(): String = \"\" }\n",
            "class UseIt {\n    private fun List<String>.firstUpper(): String = \"\"\n",
            "    fun go(items: List<String>, b: Box) {\n",
            "        items.map { it }.firstUpper()\n",
            "        items.first().shout()\n",
            "        b.firstUpper()\n",
            "    }\n}\n",
        )
        .as_bytes(),
    );
    let (dsts, _) = edges_and_external(tmp.path(), "com.acme.app.UseIt.go");
    assert_eq!(
        dsts,
        vec![
            "com.acme.app.Box.firstUpper",
            "com.acme.app.UseIt.firstUpper",
            "com.acme.util.shout",
        ],
        "the import is the extension, not Other.shout; b.firstUpper() is Box's member"
    );
}

/// Duck typing offers a Kotlin extension function only where Kotlin can
/// see it: same file, imported by name, or top-level in the caller's
/// package or a `*`-imported one. `assertThat(x).isEqualTo(y)` (AssertJ)
/// is not a workspace `List<Int>.isEqualTo` from a package never imported.
#[test]
fn kotlin_duck_typing_offers_only_extensions_in_scope() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "src/Utils.kt",
        concat!(
            "package com.acme.utils\nfun List<Int>.isEqualTo(x: Int): Boolean = true\n",
            "fun String.shout(): String = this\n",
        )
        .as_bytes(),
    );
    write(
        tmp.path(),
        "src/NotImported.kt",
        concat!(
            "package com.acme.reader\nimport org.assertj.core.api.Assertions.assertThat\n",
            "fun check() { assertThat(1).isEqualTo(1) }\n",
        )
        .as_bytes(),
    );
    write(
        tmp.path(),
        "src/StarImported.kt",
        concat!(
            "package com.acme.app\nimport com.acme.utils.*\n",
            "fun go(xs: List<String>) { xs.first().shout() }\n",
        )
        .as_bytes(),
    );
    // A top-level function that is not an extension is never reached
    // through a receiver; a member of the same name still may be.
    write(
        tmp.path(),
        "src/Free.kt",
        concat!(
            "package com.acme.dates
private fun zork(a: Int, b: Int) { }
",
            "class Tracker { fun zork(x: Int) { } }
fun make() = listOf(1)
",
            "fun use() { val l = make(); l.zork(1) }
",
        )
        .as_bytes(),
    );
    let (check, _) = edges_and_external(tmp.path(), "com.acme.reader.check");
    assert!(check.is_empty(), "out of scope: {check:?}");
    let (go, _) = edges_and_external(tmp.path(), "com.acme.app.go");
    assert_eq!(go, vec!["com.acme.utils.shout"]);
    let (used, _) = edges_and_external(tmp.path(), "com.acme.dates.use");
    assert_eq!(
        used,
        vec!["com.acme.dates.Tracker.zork", "com.acme.dates.make"]
    );
}

/// A bare call to a member the calling class inherits beats an import of
/// the same name from a dependency (Kotlin/Java resolve members first), and
/// it is that member — not every other callable sharing the name. Kotlin
/// records a superclass written as a constructor call (`: BaseUnitTest()`).
#[test]
fn a_bare_call_to_an_inherited_member_is_that_member() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "src/Base.kt",
        b"package org.acme\nopen class BaseUnitTest { protected fun test(block: () -> Unit) { block() } }\n",
    );
    write(
        tmp.path(),
        "src/Other.kt",
        b"package org.acme.other\nfun test(x: Int) { }\n",
    );
    write(
        tmp.path(),
        "src/MyTest.kt",
        concat!(
            "package org.acme.t\nimport app.cash.turbine.test\nimport org.acme.BaseUnitTest\n",
            "class MyTest : BaseUnitTest() {\n    fun check() { test { } }\n}\n",
        )
        .as_bytes(),
    );
    let (dsts, _) = edges_and_external(tmp.path(), "org.acme.t.MyTest.check");
    assert_eq!(dsts, vec!["org.acme.BaseUnitTest.test"]);
}

/// A Kotlin extension declared in a companion object is imported through
/// it (`HttpUrl.Companion.toHttpUrl`) and resolves to the extension.
#[test]
fn a_companion_imported_kotlin_extension_resolves() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "src/HttpUrl.kt",
        b"package okhttp3\nclass HttpUrl {\n    companion object {\n        fun String.toHttpUrl(): HttpUrl = HttpUrl()\n    }\n}\n",
    );
    write(
        tmp.path(),
        "src/Use.kt",
        b"package app\nimport okhttp3.HttpUrl.Companion.toHttpUrl\nfun go() { \"https://x\".toHttpUrl() }\n",
    );
    let (dsts, _) = edges_and_external(tmp.path(), "app.go");
    assert_eq!(dsts, vec!["okhttp3.HttpUrl.toHttpUrl"]);
}

/// FINDINGS C2: a receiver written as a parenthesised value, a cast or a
/// construction resolves exactly as its bare, typed form does — to the
/// type's own member, never by name to a same-named decoy. A call on what
/// a call returned (`make().m()`) stays the unresolved call it was.
#[test]
fn shaped_java_receivers_resolve_as_their_typed_form() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "Foo.java", b"class Foo { void m() {} }\n");
    write(tmp.path(), "Bar.java", b"class Bar { void m() {} }\n");
    write(
        tmp.path(),
        "Main.java",
        concat!(
            "class Main {\n  Object make() { return null; }\n",
            "  void run(Object o, Foo f) {\n",
            "    (f).m();\n    ((Foo) o).m();\n    new Foo().m();\n    (new Foo()).m();\n",
            "  }\n  void near() { make().m(); }\n}\n",
        )
        .as_bytes(),
    );
    let (dsts, _) = edges_and_external(tmp.path(), "Main.run");
    assert_eq!(dsts, vec!["Foo.m"; 4], "no edge to the Bar.m decoy");
    // Near miss: a call's result has no known type, and keeps the
    // by-name reading it always had.
    let (near, _) = edges_and_external(tmp.path(), "Main.near");
    assert_eq!(near, vec!["Bar.m", "Foo.m", "Main.make"]);
}

/// Go: a composite literal, its address and a type assertion are values
/// of the type they name, lowercase or not; an unexported type is a type,
/// never a variable to fan out over.
#[test]
fn shaped_go_receivers_name_their_type() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "go.mod", b"module ex\n\ngo 1.21\n");
    write(
        tmp.path(),
        "a.go",
        concat!(
            "package main\ntype server struct{ A int }\nfunc (s server) M() {}\n",
            "type other struct{}\nfunc (o other) M() {}\n",
            "func run(v interface{}) {\n\tserver{A: 1}.M()\n\t(&server{}).M()\n\tv.(server).M()\n}\n",
        )
        .as_bytes(),
    );
    let (dsts, _) = edges_and_external(tmp.path(), "main.run");
    assert_eq!(dsts, vec!["main.server.M"; 3], "never other.M");
}

/// TypeScript: a `.ts` file is parsed with the TypeScript grammar, where
/// `<Foo>o` is a type assertion — the TSX grammar read it as an unclosed
/// JSX element and lost every call after it. Casts, `new` and parentheses
/// all reach `Foo.m`.
#[test]
fn ts_type_assertions_parse_and_type_their_receiver() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "a.ts",
        concat!(
            "class Foo { m() {} }\nclass Bar { m() {} }\n",
            "function run(o: any, f: Foo) {\n  (<Foo>o).m();\n  (o as Foo).m();\n",
            "  new Foo().m();\n  (f).m();\n  f!.m();\n}\n",
        )
        .as_bytes(),
    );
    let (dsts, _) = edges_and_external(tmp.path(), "run");
    assert_eq!(dsts, vec!["Foo.m"; 5]);
}

/// Null-conditional calls are calls: Dart `x?.m()` and PHP `$x?->m()`
/// were never recorded at all.
#[test]
fn null_conditional_calls_are_recorded_in_dart_and_php() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "a.dart",
        b"class Foo { void m() {} }\nvoid run(Foo f) {\n  f?.m();\n}\n",
    );
    write(
        tmp.path(),
        "b.php",
        b"<?php\nclass Baz { public function n() {} }\nfunction go(Baz $b) {\n    $b?->n();\n}\n",
    );
    let (dart, _) = edges_and_external(tmp.path(), "run");
    assert_eq!(dart, vec!["Foo::m"]);
    let (php, _) = edges_and_external(tmp.path(), "go");
    assert_eq!(php, vec!["Baz::n"]);
}

/// Solidity `using L for T;`: a call on a value typed `T` that `T` does
/// not declare is library `L`'s function — not a same-named function of
/// another library, and not lost because the receiver is typed.
#[test]
fn solidity_using_for_reaches_the_library_on_a_typed_receiver() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "Lib.sol",
        concat!(
            "pragma solidity ^0.8.0;\nstruct Pos { uint x; }\n",
            "library PosLib { function shift(Pos memory p, uint d) internal pure returns (uint) { return p.x + d; } }\n",
            "library Other { function shift(uint a, uint d) internal pure returns (uint) { return a + d; } }\n",
        )
        .as_bytes(),
    );
    write(
        tmp.path(),
        "Main.sol",
        concat!(
            "pragma solidity ^0.8.0;\nimport \"./Lib.sol\";\n",
            "contract Main {\n    using PosLib for Pos;\n",
            "    function run(Pos memory p) public pure returns (uint) { return p.shift(1); }\n}\n",
        )
        .as_bytes(),
    );
    let (dsts, _) = edges_and_external(tmp.path(), "Main.run");
    assert_eq!(dsts, vec!["PosLib.shift"]);
}

/// Scala: `math.ceil(x)` is `scala.math.ceil` — every file sees the
/// `scala` package's members without an import — and is never bound by
/// name to a workspace `ceil`; an imported dependency's object is
/// refused the same way. A member `val` of that name (`io`) or a
/// parameter (`math`) is a value, and keeps its reading.
#[test]
fn scala_calls_through_standard_and_dependency_packages_are_external() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "B.scala",
        b"package app.util\nclass Helper { def ceil(x: Double): Double = x; def escapeJava(s: String) = s }\n",
    );
    write(
        tmp.path(),
        "A.scala",
        concat!(
            "package app.core\nimport org.apache.commons.text.StringEscapeUtils\n",
            "import app.util.Helper\n",
            "class A {\n  val io = new Helper\n",
            "  def run(h: Helper): Unit = {\n    math.ceil(1.0)\n    StringEscapeUtils.escapeJava(\"x\")\n",
            "    io.ceil(2.0)\n  }\n",
            "  def param(math: Helper): Unit = { math.ceil(3.0) }\n}\n",
        )
        .as_bytes(),
    );
    let (dsts, ext) = edges_and_external(tmp.path(), "A::run");
    assert_eq!(
        dsts,
        vec!["Helper::ceil"],
        "only the member val's call binds"
    );
    let mut ext = ext;
    ext.sort();
    assert_eq!(
        ext,
        vec![
            ("ceil".to_string(), "scala.math".to_string()),
            (
                "escapeJava".to_string(),
                "org.apache.commons.text".to_string()
            ),
        ]
    );
    let (param, _) = edges_and_external(tmp.path(), "A::param");
    assert_eq!(
        param,
        vec!["Helper::ceil"],
        "a parameter named math is a value"
    );
}

/// Ruby `Foo.new.m` and C++ `Foo{}.m()` / `(*p).m()`: constructions and
/// dereferences are instances of their type.
#[test]
fn ruby_and_cpp_construction_receivers_name_their_type() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "a.rb",
        b"class Foo\n  def m; end\nend\nclass Bar\n  def m; end\nend\nclass Main\n  def run\n    Foo.new.m\n  end\nend\n",
    );
    write(
        tmp.path(),
        "a.cpp",
        concat!(
            "struct Qux { void k() {} };\nstruct Quz { void k() {} };\n",
            "void go(Qux* p, void* o) {\n  Qux{}.k();\n  (*p).k();\n  static_cast<Qux*>(o)->k();\n}\n",
        )
        .as_bytes(),
    );
    let (rb, _) = edges_and_external(tmp.path(), "Main::run");
    assert_eq!(rb, vec!["Foo::m"]);
    let (cpp, _) = edges_and_external(tmp.path(), "go");
    assert_eq!(cpp, vec!["Qux::k"; 3]);
}

/// A class the calling file declares is what a receiver of that name
/// means there: `TestScript().as_form()` in a test that declares its own
/// `TestScript(Script)` reaches the inherited `Script.as_form`, never the
/// `as_form` of another test file's same-named class.
#[test]
fn a_receiver_type_the_file_declares_is_that_files_class() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "scripts.py",
        b"class Script:\n    def as_form(self):\n        pass\n",
    );
    write(
        tmp.path(),
        "test_a.py",
        concat!(
            "from scripts import Script\n",
            "def test_a():\n    class TestScript(Script):\n        def as_form(self):\n            pass\n",
            "    TestScript().as_form()\n",
        )
        .as_bytes(),
    );
    write(
        tmp.path(),
        "test_b.py",
        concat!(
            "from scripts import Script\n",
            "def test_b():\n    class TestScript(Script):\n        pass\n",
            "    TestScript().as_form()\n",
        )
        .as_bytes(),
    );
    let (b, _) = edges_and_external(tmp.path(), "test_b.test_b");
    assert_eq!(b, vec!["scripts.Script.as_form"]);
    let (a, _) = edges_and_external(tmp.path(), "test_a.test_a");
    assert_eq!(a, vec!["test_a.test_a.TestScript.as_form"]);
}

/// Scala: an import that starts at a value (`import spark.implicits._`)
/// does not make `spark` a package root, so `spark.sparkContext.f()` is
/// not refused as a call into a package `spark`.
#[test]
fn a_scala_import_from_a_value_is_not_a_package_root() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "A.scala",
        concat!(
            "package org.acme.bench\nobject Misc extends Base {\n  import spark.implicits._\n",
            "  def f(): Unit = {\n    spark.sparkContext.parallelize(Seq(1))\n  }\n}\n",
        )
        .as_bytes(),
    );
    write(
        tmp.path(),
        "B.scala",
        b"package org.acme.ctx\nclass Ctx { def parallelize(x: Int) = x }\n",
    );
    let (dsts, ext) = edges_and_external(tmp.path(), "Misc::f");
    assert_eq!(dsts, vec!["Ctx::parallelize"]);
    assert!(ext.is_empty(), "{ext:?}");
}

/// Every edge of `dir`'s default graph as `(caller, callee, confidence)`,
/// by qualified name, read from the JSON output.
fn edge_confidences(dir: &Path) -> Vec<(String, String, String)> {
    let out = dir.join("g.json");
    cgg()
        .args(["-t", "json", "-o"])
        .arg(&out)
        .arg(dir)
        .assert()
        .success();
    let g: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&out).unwrap()).unwrap();
    let qn = |id: &serde_json::Value| -> String {
        g["callables"][id.as_str().unwrap()]["qualified_name"]
            .as_str()
            .unwrap()
            .to_string()
    };
    g["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                qn(&e["src"]),
                qn(&e["dst"]),
                e["confidence"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn confidence_of(edges: &[(String, String, String)], src: &str, dst: &str) -> String {
    edges
        .iter()
        .find(|(s, d, _)| s == src && d == dst)
        .map(|(_, _, c)| c.clone())
        .unwrap_or_else(|| panic!("no edge {src} -> {dst} in {edges:?}"))
}

/// WS17: a Rust call through a path a `use` anchors (`use crate::util;
/// util::helper()`), or written relative to the crate (`crate::x::f`,
/// `super::x::f`), binds to the one callable carrying that qualified name:
/// structurally certain, so `high`. A method found only by name through an
/// untyped receiver stays `medium` even with a single candidate.
#[test]
fn rust_paths_anchored_by_use_or_crate_are_high_duck_stays_medium() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "Cargo.toml",
        b"[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
    );
    write(
        tmp.path(),
        "src/lib.rs",
        b"pub mod util;\npub mod run;\npub mod thing;\n",
    );
    write(
        tmp.path(),
        "src/util.rs",
        b"pub fn helper() -> u32 { 1 }\npub fn other() -> u32 { 2 }\n",
    );
    write(
        tmp.path(),
        "src/thing.rs",
        b"pub struct Thing;\nimpl Thing {\n    pub fn only_here(&self) -> u32 { 3 }\n}\n",
    );
    write(
        tmp.path(),
        "src/run.rs",
        b"use crate::util;\n\
          pub fn go() -> u32 { util::helper() }\n\
          pub fn abs() -> u32 { crate::util::other() }\n\
          pub fn duck(xs: &[u8]) {\n    for x in xs {\n        x.only_here();\n    }\n}\n",
    );
    let edges = edge_confidences(tmp.path());
    assert_eq!(
        confidence_of(&edges, "app::run::go", "app::util::helper"),
        "high"
    );
    assert_eq!(
        confidence_of(&edges, "app::run::abs", "app::util::other"),
        "high"
    );
    assert_eq!(
        confidence_of(&edges, "app::run::duck", "app::thing::Thing::only_here"),
        "medium",
        "a by-name match through an untyped receiver is a guess"
    );
}

/// WS17: a Java receiver typed by a declared parameter, its type imported
/// by name, binds to that class's method at `high`. The same call through
/// a variable the calling method never declares — typed only because
/// another method of the file declares a local of that name — stays
/// `medium`: the type is a file-wide guess, not a declaration in scope.
#[test]
fn java_declared_receiver_through_import_is_high_file_wide_guess_is_medium() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "src/com/a/Store.java",
        b"package com.a;\npublic class Store {\n    public void save() {}\n}\n",
    );
    write(
        tmp.path(),
        "src/com/b/User.java",
        b"package com.b;\nimport com.a.Store;\npublic class User {\n\
          void put(Store s) {\n        s.save();\n    }\n\
          void make() {\n        Store cache = new Store();\n    }\n\
          void later() {\n        cache.save();\n    }\n}\n",
    );
    let edges = edge_confidences(tmp.path());
    assert_eq!(
        confidence_of(&edges, "com.b.User.put", "com.a.Store.save"),
        "high"
    );
    assert_eq!(
        confidence_of(&edges, "com.b.User.later", "com.a.Store.save"),
        "medium",
        "a receiver typed only by another method's local is a guess"
    );
}

/// WS17: an overload set the argument count cuts to one member is a known
/// callee (`high`); one it leaves two members of stays `medium`.
#[test]
fn java_overload_cut_to_one_by_arity_is_high() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "src/O.java",
        b"public class O {\n\
          void f(int a) {}\n    void f(int a, int b) {}\n\
          void k(int a) {}\n    void k(String s) {}\n\
          void g() {\n        f(1, 2);\n        k(1);\n    }\n}\n",
    );
    let edges = edge_confidences(tmp.path());
    let fs: Vec<&String> = edges
        .iter()
        .filter(|(s, d, _)| s == "O.g" && d == "O.f")
        .map(|(_, _, c)| c)
        .collect();
    assert_eq!(fs, vec!["high"], "{edges:?}");
    let ks: Vec<&String> = edges
        .iter()
        .filter(|(s, d, _)| s == "O.g" && d == "O.k")
        .map(|(_, _, c)| c)
        .collect();
    assert_eq!(ks, vec!["medium", "medium"], "{edges:?}");
}

/// WS17: a Python receiver whose type is imported by name from the
/// workspace binds `high`; one whose type was read off a call through an
/// outside module (`pd.Series(...)`, recorded as the bare `Series`) stays
/// `medium` even though the workspace declares a `Series` with that
/// method — the type's module is not known. An inherited method two bases
/// declare stays `medium`: the first one found is a guess at the method
/// resolution order.
#[test]
fn python_typed_receivers_are_high_only_when_the_type_is_anchored() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "shop/__init__.py", b"");
    write(
        tmp.path(),
        "shop/store.py",
        b"class Store:\n    def save(self):\n        return 1\n\n\
          class Series:\n    def rename_axis(self):\n        return 2\n\n\
          class A:\n    def ping(self):\n        return 3\n\n\
          class B:\n    def ping(self):\n        return 5\n\n\
          class C(A, B):\n    pass\n",
    );
    write(
        tmp.path(),
        "shop/use.py",
        b"import pandas as pd\nfrom shop.store import Store, C, Series\n\n\
          def keep(s: Store):\n    return s.save()\n\n\
          def frame():\n    x = pd.Series([1])\n    return x.rename_axis()\n\n\
          def built():\n    return pd.Series([1]).rename_axis()\n\n\
          def both(c: C):\n    return c.ping()\n",
    );
    let edges = edge_confidences(tmp.path());
    assert_eq!(
        confidence_of(&edges, "shop.use.keep", "shop.store.Store.save"),
        "high"
    );
    for (s, d, c) in &edges {
        if s == "shop.use.frame" || s == "shop.use.built" || s == "shop.use.both" {
            assert_eq!(c, "medium", "{s} -> {d} must not be high: {edges:?}");
        }
    }
}

/// WS17: `pub use expand::{run as go_run};` in a crate root names the
/// child module `expand` (Rust 2018 uniform paths). A call through the
/// re-export (`app::go_run()`, `app::run_direct()`) reaches the function
/// it names, at `high`, instead of falling through to a by-name fan-out
/// over every same-named method in the workspace.
#[test]
fn rust_reexport_of_a_child_module_item_is_followed() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "app/Cargo.toml",
        b"[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
    );
    write(
        tmp.path(),
        "app/src/lib.rs",
        b"pub mod expand;\npub use expand::{run as go_run, run_direct};\n",
    );
    write(
        tmp.path(),
        "app/src/expand.rs",
        b"pub fn run() -> u32 { 1 }\npub fn run_direct() -> u32 { 2 }\n",
    );
    write(
        tmp.path(),
        "other/Cargo.toml",
        b"[package]\nname = \"other\"\nversion = \"0.1.0\"\n",
    );
    write(
        tmp.path(),
        "other/src/lib.rs",
        b"pub struct Decoy;\nimpl Decoy {\n    pub fn run_direct(&self) -> u32 { 3 }\n}\n\
          pub fn call() -> u32 {\n    app::go_run() + app::run_direct()\n}\n",
    );
    let edges = edge_confidences(tmp.path());
    assert_eq!(
        confidence_of(&edges, "other::call", "app::expand::run"),
        "high"
    );
    assert_eq!(
        confidence_of(&edges, "other::call", "app::expand::run_direct"),
        "high"
    );
    assert!(
        !edges
            .iter()
            .any(|(_, d, _)| d == "other::Decoy::run_direct"),
        "the re-export, not a same-named method, is the callee: {edges:?}"
    );
}

/// WS17: `super().m()` binds `high` only through bases the calling file
/// anchors. A base imported from a dependency (`from django.test import
/// TestCase`) is not the workspace's own `TestCase`, though bases are
/// recorded by bare name and the walk finds the workspace one.
#[test]
fn python_super_through_a_dependency_base_is_not_high() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "app/__init__.py", b"");
    write(
        tmp.path(),
        "app/base.py",
        b"class TestCase:\n    def setUp(self):\n        return 1\n\n\
          class Base:\n    def ready(self):\n        return 2\n",
    );
    write(
        tmp.path(),
        "app/tests.py",
        b"from django.test import TestCase\nfrom app.base import Base\n\n\
          class Outside(TestCase):\n    def setUp(self):\n        super().setUp()\n\n\
          class Inside(Base):\n    def ready(self):\n        return super().ready()\n",
    );
    let edges = edge_confidences(tmp.path());
    assert_eq!(
        confidence_of(&edges, "app.tests.Inside.ready", "app.base.Base.ready"),
        "high"
    );
    for (s, d, c) in &edges {
        if s == "app.tests.Outside.setUp" {
            assert_eq!(c, "medium", "{s} -> {d}: {edges:?}");
        }
    }
}

/// WS23 gap 10: a Go method's receiver and a parameter of a same-package
/// type written bare are typed, name first, and bind to that package's
/// type at high. A same-named type of another package is not it; a `:=`
/// in an inner block or a function literal's own parameter hides the
/// parameter; an embedded field's methods are promoted.
#[test]
fn go_receivers_and_bare_parameter_types_bind_their_package_type() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "go.mod", b"module ex\n\ngo 1.21\n");
    write(
        tmp.path(),
        "a.go",
        concat!(
            "package main\n",
            "type Base struct{}\nfunc (b *Base) Hello() {}\n",
            "type Server struct{ *Base }\nfunc (s *Server) zip() {}\n",
            "type Third struct{}\nfunc (t *Third) zip() {}\n",
            "type Other struct{}\nfunc (o *Other) Hello() {}\n",
            "func (s *Server) Zap() { s.zip() }\n",
            "func use(q *Server) { q.zip() }\n",
            "func run(s *Server) { s.Hello() }\n",
            "func inner(s *Server) {\n\tif s := (&Third{}); s != nil {\n\t\ts.zip()\n\t}\n}\n",
            "func lit(s *Server) { _ = func(s *Third) { s.zip() } }\n",
        )
        .as_bytes(),
    );
    write(
        tmp.path(),
        "b/b.go",
        b"package b\ntype Server struct{}\nfunc (s *Server) zip() {}\nfunc local(s *Server) { s.zip() }\n",
    );
    let e = cpp_edges(tmp.path());
    let has = |src: &str, dst: &str, conf: &str| {
        e.contains(&(src.to_string(), dst.to_string(), conf.to_string()))
    };
    assert!(
        has("main.Server.Zap", "main.Server.zip@a.go", "high"),
        "{e:?}"
    );
    assert!(has("main.use", "main.Server.zip@a.go", "high"), "{e:?}");
    // Promoted through the embedded `*Base`; medium, because another
    // package also declares a `Server`, which the base chain is keyed by.
    assert!(
        e.iter()
            .any(|(s, d, _)| s == "main.run" && d == "main.Base.Hello@a.go"),
        "{e:?}"
    );
    assert!(has("b.local", "b.Server.zip@b.go", "high"), "{e:?}");
    for src in ["main.inner", "main.lit"] {
        assert!(
            !e.iter()
                .any(|(s, d, _)| s == src && d == "main.Server.zip@a.go"),
            "{src}: the hidden parameter types nothing: {e:?}"
        );
    }
    assert!(
        !e.iter()
            .any(|(s, d, _)| s.starts_with("main.") && d.ends_with("@b.go")),
        "another package's Server is never the callee: {e:?}"
    );
}

/// WS23 gap 26: a bare call's declared return type types a receiver
/// written as the call (`make().work()`) and a local assigned from it
/// (`var x = make()`), each in the function that declares it. A function
/// returning `Object`, or one whose definitions disagree, types nothing.
#[test]
fn declared_return_types_type_call_results_in_type_first_languages() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "A.java",
        concat!(
            "class Tool { void work() {} }\nclass Other { void work() {} }\n",
            "class F {\n  static Tool make() { return null; }\n  Other other() { return null; }\n",
            "  Object any() { return null; }\n",
            "  void a() { var x = make(); x.work(); }\n",
            "  void b() { var x = other(); x.work(); }\n",
            "  void c() { make().work(); }\n",
            "  void d() { any().work(); }\n}\n",
        )
        .as_bytes(),
    );
    write(
        tmp.path(),
        "a.cpp",
        concat!(
            "class Gear { public: void spin() {} };\nclass Cog { public: void spin() {} };\n",
            "Gear* gear() { return nullptr; }\n",
            "void e() { auto g = gear(); g->spin(); }\n",
        )
        .as_bytes(),
    );
    let e = cpp_edges(tmp.path());
    let to = |src: &str| -> Vec<String> {
        e.iter()
            .filter(|(s, d, _)| s == src && (d.contains("work") || d.contains("spin")))
            .map(|(_, d, c)| format!("{d} {c}"))
            .collect()
    };
    assert_eq!(to("F.a"), vec!["Tool.work@A.java high"]);
    assert_eq!(to("F.b"), vec!["Other.work@A.java high"]);
    assert_eq!(to("F.c"), vec!["Tool.work@A.java high"]);
    // Near miss: `Object` says nothing; the by-name reading stays.
    assert_eq!(
        to("F.d"),
        vec!["Other.work@A.java medium", "Tool.work@A.java medium"]
    );
    assert_eq!(to("e"), vec!["Gear::spin@a.cpp high"]);
}

/// WS23 gap 28: a JS/TS local declared from a construction or with a
/// type annotation types its receiver where that declaration is the
/// binding in scope. Another function's local of the name, an inner
/// block's own declaration, and a constructed `let` reassigned later do
/// not.
#[test]
fn ts_locals_type_their_receivers_where_they_are_in_scope() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "a.ts",
        concat!(
            "class Svc { run() {} }\nclass Other { run() {} }\n",
            "function a() { const x = new Svc(); x.run(); }\n",
            "function b() { const x: Other = mk(); x.run(); }\n",
            "function c() { let y = new Svc(); y = mk(); y.run(); }\n",
            "function d() { const z = new Svc(); { const z = new Other(); z.run(); } }\n",
            "function mk(): any { return null; }\n",
        )
        .as_bytes(),
    );
    let e = cpp_edges(tmp.path());
    let to = |src: &str| -> Vec<String> {
        e.iter()
            .filter(|(s, d, _)| s == src && d.contains(".run"))
            .map(|(_, d, c)| format!("{d} {c}"))
            .collect()
    };
    assert_eq!(to("a"), vec!["Svc.run@a.ts high"]);
    assert_eq!(to("b"), vec!["Other.run@a.ts high"]);
    assert_eq!(to("d"), vec!["Other.run@a.ts high"]);
    // Reassigned: the construction no longer types it.
    assert_eq!(
        to("c"),
        vec!["Other.run@a.ts medium", "Svc.run@a.ts medium"]
    );
}

/// WS23 gap 16: Swift and Groovy record a call's argument count, so a
/// typed lookup into another file's overload set binds the overload the
/// count admits, never one arbitrary member.
#[test]
fn swift_and_groovy_typed_overload_lookups_narrow_by_arity() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "A.swift",
        b"class Tool {\n  func work() {}\n  func work(_ n: Int) {}\n  func work(_ n: Int, _ m: Int) {}\n}\n",
    );
    write(
        tmp.path(),
        "B.swift",
        b"func one(t: Tool) { t.work(1) }\nfunc two(t: Tool) { t.work(1) { } }\n",
    );
    write(
        tmp.path(),
        "A.groovy",
        b"class Gear {\n  void spin() {}\n  void spin(int n) {}\n  void spin(int n, int m) {}\n}\n",
    );
    write(
        tmp.path(),
        "B.groovy",
        b"class U {\n  void none(Gear g) { g.spin() }\n  void named(Gear g) { g.spin(a: 1, b: 2) }\n}\n",
    );
    let g = tmp.path().join("g.json");
    cgg()
        .args(["-t", "json", "-o"])
        .arg(&g)
        .arg(tmp.path())
        .assert()
        .success();
    let g: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&g).unwrap()).unwrap();
    let c = &g["callables"];
    let lines = |src: &str| -> Vec<u64> {
        let mut v: Vec<u64> = g["edges"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| c[e["src"].as_str().unwrap()]["qualified_name"] == src)
            .map(|e| {
                c[e["dst"].as_str().unwrap()]["start_line"]
                    .as_u64()
                    .unwrap()
            })
            .collect();
        v.sort();
        v
    };
    assert_eq!(lines("one"), vec![3]);
    // A trailing closure is an argument.
    assert_eq!(lines("two"), vec![4]);
    assert_eq!(lines("U.none"), vec![2]);
    // Near miss: named arguments are one map, so the count says nothing
    // and the whole set stands.
    assert_eq!(lines("U.named"), vec![2, 3, 4]);
}

/// WS23: a typed receiver whose type's bare name several classes nest
/// (`Builder` in each of six classes) is a guess among them, refused over
/// the fan-out cap and recorded, never bound to all of them.
#[test]
fn java_same_named_nested_owner_fanout_is_capped() {
    let tmp = TempDir::new().unwrap();
    // Distinct outer classes, each in its own file: not copies of one
    // class, so the nearest is no more the callee than the others.
    for i in 0..6 {
        write(
            tmp.path(),
            &format!("A{i}.java"),
            format!("class A{i} {{ static class Builder {{ Object build() {{ return null; }} }} }}\n")
                .as_bytes(),
        );
    }
    write(
        tmp.path(),
        "Use.java",
        b"class Use {\n  A0.Builder mk() { return null; }\n  Builder b() { return null; }\n  void run() { b().build(); }\n}\n",
    );
    let e = cpp_edges(tmp.path());
    assert!(
        !e.iter()
            .any(|(s, d, _)| s == "Use.run" && d.contains("build")),
        "{e:?}"
    );
}

/// WS23: over the cap, copies of one same-named class narrow to the copy
/// nearest the caller (as Java/C# bare calls do) before anything is
/// refused; a tie stays refused.
#[test]
fn java_same_named_owner_cap_narrows_to_the_nearest_copy() {
    let tmp = TempDir::new().unwrap();
    for i in 0..6 {
        write(
            tmp.path(),
            &format!("s{i}/Calc.java"),
            b"class Calc { int add() { return 0; } }\n",
        );
    }
    write(
        tmp.path(),
        "s3/Use.java",
        b"class Use {\n  Calc mk() { return null; }\n  void run() { mk().add(); }\n}\n",
    );
    let e = cpp_edges(tmp.path());
    let to: Vec<&String> = e
        .iter()
        .filter(|(s, d, _)| s == "Use.run" && d.contains("add"))
        .map(|(_, d, _)| d)
        .collect();
    assert_eq!(to.len(), 1, "{e:?}");
}
