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
    assert!(
        adds.iter()
            .all(|a| g.contains(&format!("{} --> {a}", init[0]))),
        "typed call should reach both definitions:\n{g}"
    );
    assert_eq!(helpers.len(), 1, "helper:\n{g}");
    assert!(
        g.contains(&format!("{} --> {}", adds[0], helpers[0]))
            || g.contains(&format!("{} --> {}", adds[1], helpers[0])),
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
            let id = t.trim();
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
