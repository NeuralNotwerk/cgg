//! `--property-edges` in every language with getters: one row per
//! language, or an N/A reason in `PROPERTY_NA`.
//!
//! Each row's fixture declares a getter on `A` and reads it through a
//! receiver whose type is declared (a typed parameter, `this`/`self`, or
//! a construction), and also reads a plain field and an untyped
//! receiver's member, which must bind nothing.

use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

/// Languages where a member read runs no code, or whose paren-less reads
/// are already calls.
pub const PROPERTY_NA: &[(&str, &str)] = &[
    ("ruby", "paren-less member reads are already call sites"),
    ("scala", "paren-less member reads are already call sites"),
    ("java", "no getter syntax: a getter is called as a method"),
    ("go", "no getters: a field read runs no code"),
    ("rust", "no getters: a field read runs no code"),
    ("c", "no getters"),
    ("cpp", "no getters"),
];

/// `(language, file, source, reader qualified-name suffix)`.
const ROWS: &[(&str, &str, &str, &str)] = &[
    (
        "javascript",
        "a.js",
        "class A { get size() { return 1; } }\n\
         function g() { const a = new A(); return a.size + a.plain; }\n\
         function u(x) { return x.size; }\n",
        "g",
    ),
    (
        "typescript",
        "a.ts",
        "class A { plain = 0; get size(): number { return 1; } }\n\
         function g(a: A) { return a.size + a.plain; }\n\
         function u(x) { return x.size; }\n",
        "g",
    ),
    (
        "csharp",
        "a.cs",
        "class A { public int plain; public int Size { get { return 1; } } }\n\
         class B { int G(A a) { return a.Size + a.plain; } int U(dynamic x) { return x.Size; } }\n",
        "B.G",
    ),
    (
        "kotlin",
        "a.kt",
        "package p\nclass A { val plain = 0\n val size: Int get() = 1 }\n\
         fun g(a: A): Int { return a.size + a.plain }\n",
        "g",
    ),
    (
        "swift",
        "a.swift",
        "class A { var plain = 0\n var size: Int { return 1 } }\n\
         func g(a: A) -> Int { return a.size + a.plain }\n",
        "g",
    ),
    (
        "dart",
        "a.dart",
        "class A { int plain = 0; int get size => 1; }\n\
         int g(A a) { return a.size + a.plain; }\n",
        "g",
    ),
    (
        "groovy",
        "a.groovy",
        "class A { int plain = 0\n int getSize() { 1 } }\n\
         class B { int g(A a) { return a.size + a.plain } }\n",
        "B.g",
    ),
    (
        "objc",
        "a.m",
        "@interface A\n- (int)size;\n@end\n@implementation A\n\
         - (int)size { return 1; }\n- (int)g { return self.size; }\n@end\n",
        "g",
    ),
];

fn edges(
    dir: &std::path::Path,
    lang: &str,
    flag: bool,
) -> Vec<(String, String, String, String)> {
    let mut cmd = Command::cargo_bin("cgg").unwrap();
    cmd.arg(dir)
        .args(["--lang", lang, "-t", "json", "-q", "--no-entry-nodes"]);
    if flag {
        cmd.arg("--property-edges");
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
    g["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                qn(&e["src"]),
                qn(&e["dst"]),
                e["via"]["kind"].as_str().unwrap_or("").to_string(),
                e["confidence"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect()
}

#[test]
fn a_getter_read_through_a_declared_type_is_a_property_edge_in_every_language() {
    for (lang, file, src, reader) in ROWS {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join(file), src).unwrap();
        let on = edges(tmp.path(), lang, true);
        let prop: Vec<_> = on.iter().filter(|e| e.2 == "property").collect();
        assert_eq!(
            prop.len(),
            1,
            "{lang}: exactly the getter read binds: {on:?}"
        );
        let (src_qn, dst_qn, _, conf) = prop[0];
        assert!(
            src_qn.ends_with(reader)
                && (dst_qn.contains("size") || dst_qn.contains("Size")),
            "{lang}: {src_qn} -> {dst_qn}"
        );
        assert_eq!(conf, "high", "{lang}: a declared, unique type");
        // Off by default: no property edge, and no getter-read site
        // becomes any other kind of edge.
        let off = edges(tmp.path(), lang, false);
        assert!(off.iter().all(|e| e.2 != "property"), "{lang}: {off:?}");
        let off_set: Vec<_> = off.iter().map(|e| (&e.0, &e.1)).collect();
        let on_rest: Vec<_> = on
            .iter()
            .filter(|e| e.2 != "property")
            .map(|e| (&e.0, &e.1))
            .collect();
        assert_eq!(
            off_set, on_rest,
            "{lang}: the flag adds only property edges"
        );
    }
}

#[test]
fn every_language_has_a_row_or_a_reason() {
    let have: Vec<&str> = ROWS
        .iter()
        .map(|r| r.0)
        .chain(PROPERTY_NA.iter().map(|r| r.0))
        .chain(["python"])
        .collect();
    for lang in cgg_lang::property_reads::PROPERTY_READ_LANGS {
        assert!(
            have.contains(lang),
            "{lang} reads properties but has no row"
        );
    }
}
