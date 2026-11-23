//! `--module-scope-calls` in every language with top-level code: one row
//! per language, or an N/A reason in `MODULE_SCOPE_NA`. Each row's
//! top-level call must bind from the file's synthetic `<module>` caller,
//! and the flag must only add edges — the default graph's edges all
//! survive it (the synthetic scope spans the whole file, and once made
//! every top-level definition look nested, hiding it from importers).

use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

/// Languages with no top-level statements to bind.
pub const MODULE_SCOPE_NA: &[(&str, &str)] = &[
    ("c", "file scope holds only constant initializers"),
    ("java", "no top-level statements"),
    ("dart", "no top-level statements"),
    ("haskell", "every top-level binding is a definition"),
    ("erlang", "no top-level expressions"),
    ("solidity", "no top-level statements"),
];

/// `(language, files)`; the first file's top-level code calls `f`.
const ROWS: &[(&str, &[(&str, &str)])] = &[
    (
        "python",
        &[
            ("a.py", "from b import f\nf()\n"),
            ("b.py", "def f():\n    pass\n"),
        ],
    ),
    (
        "javascript",
        &[
            ("a.js", "import { f } from './b';\nf();\n"),
            ("b.js", "export function f() {}\n"),
        ],
    ),
    (
        "typescript",
        &[
            ("a.ts", "import { f } from './b';\nf();\n"),
            ("b.ts", "export function f(): void {}\n"),
        ],
    ),
    ("ruby", &[("a.rb", "def f\nend\nf()\n")]),
    ("lua", &[("a.lua", "local function f() end\nf()\n")]),
    ("bash", &[("a.sh", "#!/bin/bash\nf() { echo; }\nf\n")]),
    ("perl", &[("a.pl", "sub f { }\nf();\n")]),
    ("php", &[("a.php", "<?php\nfunction f() {}\nf();\n")]),
    ("r", &[("a.R", "f <- function() {}\nf()\n")]),
    ("julia", &[("a.jl", "function f() end\nf()\n")]),
    ("powershell", &[("a.ps1", "function f { }\nf\n")]),
    ("groovy", &[("a.groovy", "def f() {}\nf()\n")]),
    ("kotlin", &[("a.kts", "fun f() {}\nf()\n")]),
    ("swift", &[("main.swift", "func f() {}\nf()\n")]),
    (
        "go",
        &[(
            "a.go",
            "package p\nfunc f() int { return 1 }\nvar x = f()\n",
        )],
    ),
    ("csharp", &[("a.cs", "static void f() {}\nf();\n")]),
    ("cpp", &[("a.cpp", "int f() { return 1; }\nint x = f();\n")]),
    (
        "rust",
        &[("a.rs", "const fn f() -> u32 { 1 }\nconst X: u32 = f();\n")],
    ),
    (
        "elixir",
        &[("a.exs", "defmodule M do\n  def f, do: 1\nend\nM.f()\n")],
    ),
    ("clojure", &[("a.clj", "(ns a)\n(defn f [] 1)\n(f)\n")]),
    (
        "cmake",
        &[("CMakeLists.txt", "function(f)\nendfunction()\nf()\n")],
    ),
    ("starlark", &[("BUILD", "def f():\n    pass\nf()\n")]),
    ("fsharp", &[("a.fsx", "let f () = 1\nf () |> ignore\n")]),
    (
        "ocaml",
        &[("a.ml", "let f () = 1\nlet () = ignore (f ())\n")],
    ),
    ("scala", &[("a.sc", "def f() = 1\nf()\n")]),
    (
        "zig",
        &[("a.zig", "fn f() i32 { return 1; }\nconst x = f();\n")],
    ),
    ("nix", &[("a.nix", "let f = x: x; in f 1\n")]),
];

fn edges(dir: &std::path::Path, flag: bool) -> Vec<(String, String)> {
    let mut cmd = Command::cargo_bin("cgg").unwrap();
    cmd.arg(dir).args(["-t", "json", "-q", "--no-entry-nodes"]);
    if flag {
        cmd.arg("--module-scope-calls");
    }
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let g: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let qn = |id: &serde_json::Value| {
        g["callables"][id.as_str().unwrap()]["qualified_name"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let file = |id: &serde_json::Value| {
        let f = g["callables"][id.as_str().unwrap()]["file"]
            .as_str()
            .unwrap();
        let p = g["files"][f]["path"].as_str().unwrap();
        p.rsplit(['/', '\\']).next().unwrap().to_string()
    };
    g["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                qn(&e["src"]),
                format!("{}@{}", qn(&e["dst"]), file(&e["dst"])),
            )
        })
        .collect()
}

#[test]
fn a_top_level_call_binds_from_the_module_scope_in_every_language() {
    for (lang, files) in ROWS {
        let tmp = TempDir::new().unwrap();
        for (name, body) in *files {
            fs::write(tmp.path().join(name), body).unwrap();
        }
        let on = edges(tmp.path(), true);
        let first = files[0].0;
        assert!(
            on.iter().any(|(s, d)| s == &format!("<module>:{first}")
                && d.split('@').next().unwrap().ends_with('f')),
            "{lang}: no <module>:{first} -> f edge in {on:?}"
        );
        for e in edges(tmp.path(), false) {
            assert!(on.contains(&e), "{lang}: the flag dropped {e:?}");
        }
    }
}

#[test]
fn an_import_still_names_the_top_level_function_of_a_module_scope_file() {
    // `b` has top-level code, so the flag gives it a `<module>` scope
    // spanning the file. Its `f` is still a top-level definition: the
    // import binds it, and only it — not the same-named `f` in `c`.
    for (lang, a, b, c) in [
        (
            "python",
            ("a.py", "from b import f\ndef g():\n    f()\n"),
            ("b.py", "def f():\n    pass\nf()\n"),
            ("c.py", "def f():\n    pass\n"),
        ),
        (
            "typescript",
            ("a.ts", "import { f } from './b';\nfunction g() { f(); }\n"),
            ("b.ts", "export function f(): void {}\nf();\n"),
            ("c.ts", "export function f(): void {}\n"),
        ),
    ] {
        let tmp = TempDir::new().unwrap();
        for (name, body) in [a, b, c] {
            fs::write(tmp.path().join(name), body).unwrap();
        }
        let from_g: Vec<String> = edges(tmp.path(), true)
            .into_iter()
            .filter(|(s, _)| s.ends_with('g'))
            .map(|(_, d)| d.rsplit('@').next().unwrap().to_string())
            .collect();
        assert_eq!(from_g, vec![b.0.to_string()], "{lang}");
    }
}
