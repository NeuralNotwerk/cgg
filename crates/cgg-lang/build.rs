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
//! * Groovy: the dekobon/tree-sitter-groovy grammar, from a fork branch
//!   with fixes not yet released upstream; its C symbols are renamed to
//!   `tree_sitter_groovy_dekobon*` (see `vendor/groovy/PROVENANCE.md`).
//!
//! Each grammar is a separate `cc` static lib. The C symbols
//! `tree_sitter_smithy()` / `tree_sitter_kivy()` are bound in the plugins
//! via `tree_sitter_language::LanguageFn`.

use std::path::Path;

/// Compile one vendored grammar (`parser.c`, plus `scanner.c` when the
/// grammar has an external scanner) into the static lib `lib`.
///
/// MSVC needs `-utf-8`. Generated parsers spell non-ASCII token names as
/// `\uXXXX` escapes (Lean 4: `¬`, `·`, `×`, `λ`, `→`, …), and MSVC encodes
/// string literals in the *system code page* unless told otherwise: `·`
/// became the lone byte 0xB7, `Node::kind()` hit invalid UTF-8 and
/// panicked on any Lean file using it, and `λ`/`→` silently became `?`.
/// The published grammar crates (haskell, scala, verilog) pass the same
/// flag. The target is read from `CARGO_CFG_TARGET_ENV`, not `cfg!`, which
/// would describe the build host.
fn grammar(dir: &str, lib: &str, scanner: bool) {
    let dir = Path::new(dir);
    let mut build = cc::Build::new();
    build
        .file(dir.join("parser.c"))
        .include(dir)
        .warnings(false);
    if scanner {
        build.file(dir.join("scanner.c"));
        println!("cargo:rerun-if-changed={}", dir.join("scanner.c").display());
    }
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        build.flag("-utf-8");
    }
    build.compile(lib);
    println!("cargo:rerun-if-changed={}", dir.join("parser.c").display());
    println!(
        "cargo:rerun-if-changed={}",
        dir.join("tree_sitter/parser.h").display()
    );
}

fn main() {
    grammar("vendor/smithy", "tree_sitter_smithy", false);
    grammar("vendor/kivy", "tree_sitter_kivy", true);
    grammar("vendor/lean4", "tree_sitter_lean", true);
    grammar("vendor/groovy", "tree_sitter_groovy_dekobon", true);
}
