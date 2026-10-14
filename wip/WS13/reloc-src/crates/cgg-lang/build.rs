//! Compile vendored tree-sitter grammars that aren't available as
//! workspace-compatible crates.
//!
//! * Smithy: the published `tree-sitter-smithy` crate pins an old
//!   `tree-sitter 0.20` and the deprecated `language()` API, so we vendor
//!   its generated `parser.c` (see `vendor/smithy/PROVENANCE.md`).
//! * Kivy KV: no crates.io package; vendor `parser.c` + indent `scanner.c`
//!   (see `vendor/kivy/PROVENANCE.md`).
//! * Lean 4: the only published crate links the `tree-sitter` core crate
//!   directly and conflicts with the workspace runtime; vendor `parser.c`
//!   + `scanner.c` (see `vendor/lean4/PROVENANCE.md`).
//!
//! Each grammar is a separate `cc` static lib. The C symbols
//! `tree_sitter_smithy()` / `tree_sitter_kivy()` are bound in the plugins
//! via `tree_sitter_language::LanguageFn`.

use std::path::Path;

fn main() {
    let smithy = Path::new("vendor/smithy");
    cc::Build::new()
        .file(smithy.join("parser.c"))
        .include(smithy)
        .warnings(false)
        .compile("tree_sitter_smithy");
    println!("cargo:rerun-if-changed=vendor/smithy/parser.c");
    println!("cargo:rerun-if-changed=vendor/smithy/tree_sitter/parser.h");

    let kivy = Path::new("vendor/kivy");
    cc::Build::new()
        .file(kivy.join("parser.c"))
        .file(kivy.join("scanner.c"))
        .include(kivy)
        .warnings(false)
        .compile("tree_sitter_kivy");
    println!("cargo:rerun-if-changed=vendor/kivy/parser.c");
    println!("cargo:rerun-if-changed=vendor/kivy/scanner.c");
    println!("cargo:rerun-if-changed=vendor/kivy/tree_sitter/parser.h");

    let lean = Path::new("vendor/lean4");
    cc::Build::new()
        .file(lean.join("parser.c"))
        .file(lean.join("scanner.c"))
        .include(lean)
        .warnings(false)
        .compile("tree_sitter_lean");
    println!("cargo:rerun-if-changed=vendor/lean4/parser.c");
    println!("cargo:rerun-if-changed=vendor/lean4/scanner.c");
    println!("cargo:rerun-if-changed=vendor/lean4/tree_sitter/parser.h");
}
