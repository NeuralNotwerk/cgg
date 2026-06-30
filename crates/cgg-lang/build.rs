//! Compile vendored tree-sitter grammars that aren't available as
//! workspace-compatible crates.
//!
//! Currently just Smithy: the published `tree-sitter-smithy` crate pins an
//! old `tree-sitter 0.20` and the deprecated `language()` API, so we vendor
//! its generated `parser.c` (see `vendor/smithy/PROVENANCE.md`) and compile
//! it here. The resulting `tree_sitter_smithy()` C symbol is bound in
//! `plugins/smithy.rs` via `tree_sitter_language::LanguageFn`.

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
}
