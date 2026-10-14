//! Every `detect` prefix in the rule table must be able to fire.
//!
//! The detect-only table is deliberately broad — its whole purpose is
//! that an unknown framework produces a *disclosed gap* instead of
//! silence. But breadth creates a failure mode of its own: a rule whose
//! `detect` prefix does not match the way the language actually writes
//! that import is a rule that silently never fires, and silence is
//! exactly what the table exists to prevent. Such a rule is worse than
//! no rule, because the coverage table then implies the framework was
//! considered and absent.
//!
//! No corpus can contain every framework in the table, so most of these
//! prefixes have no real-world evidence behind them. This test supplies
//! the missing evidence synthetically: for each language it writes one
//! file per rule containing nothing but an import of that rule's first
//! `detect` prefix, runs cgg over the directory, and asserts every rule
//! shows up in the coverage table.
//!
//! It checks *detection*, not enumeration. A rule can legitimately be
//! detected and enumerate nothing — that is what a detect-only rule is.
//! What it cannot legitimately do is fail to be detected at all.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use assert_cmd::prelude::*;
use cgg_core::frameworks::rules::SPECS;
use std::process::Command;
use tempfile::TempDir;

/// How each language writes "I use this module", and the file extension
/// to put it in. Only languages that actually have rules need an entry.
fn import_form(language: &str, prefix: &str) -> Option<(String, &'static str)> {
    let (src, ext) = match language {
        "python" => (format!("import {prefix}\n"), "py"),
        "javascript" => (format!("const _x = require('{prefix}');\n"), "js"),
        "typescript" => (format!("import * as _x from '{prefix}';\n"), "ts"),
        "java" => (format!("import {prefix}.Thing;\n"), "java"),
        "kotlin" => (format!("import {prefix}.Thing\n"), "kt"),
        "go" => (format!("package p\n\nimport \"{prefix}\"\n"), "go"),
        "csharp" => (format!("using {prefix};\n"), "cs"),
        "rust" => (format!("use {prefix};\n"), "rs"),
        "ruby" => (format!("require '{prefix}'\n"), "rb"),
        "php" => (format!("<?php\nuse {prefix}\\Thing;\n"), "php"),
        "elixir" => (format!("defmodule M do\n  use {prefix}\nend\n"), "ex"),
        "perl" => (format!("use {prefix};\n1;\n"), "pm"),
        "clojure" => (format!("(ns m (:require [{prefix}]))\n"), "clj"),
        "lua" => (format!("local _x = require(\"{prefix}\")\n"), "lua"),
        "scala" => (format!("import {prefix}.Thing\n"), "scala"),
        "groovy" => (format!("import {prefix}.Thing\n"), "groovy"),
        "swift" => (format!("import {prefix}\n"), "swift"),
        // The objc plugin records the include text verbatim, brackets
        // and all, so a prefix already carries its own `<`.
        "objc" => {
            let bare = prefix.trim_start_matches('<');
            (format!("#import <{bare}/{bare}.h>\n"), "m")
        }
        // Dart prefixes already carry their `package:` scheme.
        "dart" => {
            if let Some(pkg) = prefix.strip_prefix("package:") {
                (format!("import 'package:{pkg}/{pkg}.dart';\n"), "dart")
            } else {
                (format!("import 'dart:{prefix}';\n"), "dart")
            }
        }
        "haskell" => (format!("import {prefix}\n"), "hs"),
        "ocaml" => (format!("open {prefix}\n"), "ml"),
        "fsharp" => (format!("open {prefix}\n"), "fs"),
        "erlang" => (format!("-module(m).\n-behaviour({prefix}).\n"), "erl"),
        "julia" => (format!("using {prefix}\n"), "jl"),
        "r" => (format!("library({prefix})\n"), "R"),
        "powershell" => (format!("Import-Module {prefix}\n"), "ps1"),
        "bash" => (format!("#!/bin/bash\nsource {prefix}\n"), "sh"),
        "zig" => (format!("const _x = @import(\"{prefix}\");\n"), "zig"),
        // A C prefix may already be a full header name (`signal.h`) or a
        // library root (`gtest`); only add `.h` when it is neither.
        "c" | "cpp" => {
            let ext = if language == "c" { "c" } else { "cpp" };
            let hdr = if prefix.contains('.') || prefix.contains('/') {
                prefix.to_string()
            } else {
                format!("{prefix}/{prefix}.h")
            };
            (format!("#include <{hdr}>\n"), ext)
        }
        "fortran" => (format!("program p\n  use {prefix}\nend program p\n"), "f90"),
        "cmake" => (format!("include({prefix})\n"), "cmake"),
        "nix" => (format!("{{ }}: import {prefix}\n"), "nix"),
        "starlark" => (format!("load(\"{prefix}\", \"sym\")\n"), "bzl"),
        "vhdl" => (format!("library {prefix};\nuse {prefix}.all;\n"), "vhd"),
        "verilog" => (format!("`include \"{prefix}.v\"\n"), "v"),
        "asm" => (format!(".global _start\n_start:\n  call {prefix}\n"), "s"),
        // Descriptor languages: an "import" is a schema reference.
        "proto" | "graphql" | "openapi" | "asyncapi" | "smithy" | "hcl" => {
            return None;
        }
        _ => return None,
    };
    Some((src, ext))
}

/// The framework ids the coverage table actually names, parsed out of
/// its two sections rather than looked for as substrings.
///
/// Substring matching would let `spring-batch` satisfy a lookup for
/// `spring`: 41 rule ids in the table are prefixes of another id in the
/// same language (`react`/`react-native`, `plug`/`plug-router`,
/// `blazor`/`blazor-jsinterop`, …), so the shorter one would be reported
/// as detected whenever the longer one fired. That is the exact failure
/// this test exists to catch, so it cannot be allowed to hide in it.
///
/// Two shapes to read, both from `FrameworkCoverage::render`:
///   `  recognised    <id> (<kind>, N entries) · <id> (…)`
///   `  seen, no rules <id> — found in N file(s), …`
/// plus their continuation lines, which carry an item or an id alone.
fn reported_ids(stderr: &str) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for line in stderr.lines() {
        if line.contains(" — found in ") {
            let head = line.split(" — found in ").next().unwrap_or("");
            if let Some(tok) = head.split_whitespace().last() {
                ids.insert(tok.to_string());
            }
        }
        for item in line.split(" · ") {
            let Some((name, rest)) = item.trim().split_once(" (") else {
                continue;
            };
            if !rest.ends_with("entries)") && !rest.ends_with("entry)") {
                continue;
            }
            // The label column shares the line with the first item, and
            // an ambiguous id is rendered as `id/language`.
            let name = name.split_whitespace().last().unwrap_or(name);
            ids.insert(name.split('/').next().unwrap_or(name).to_string());
        }
    }
    ids
}

/// A prefix written for a *deeper* path than the import form can carry.
/// `org.springframework.web.bind.annotation` is a real prefix but a Java
/// import of it plus `.Thing` is still valid, so most work as-is.
fn sanitize(prefix: &str) -> &str {
    prefix.trim()
}

#[test]
fn every_detect_prefix_can_actually_fire() {
    // language -> [(rule id, first detect prefix)]
    let mut by_lang: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();
    for s in SPECS {
        let Some(first) = s.detect.first() else {
            continue;
        };
        by_lang
            .entry(s.language)
            .or_default()
            .push((s.id, sanitize(first)));
    }

    let mut unsupported: Vec<&str> = Vec::new();
    let mut undetected: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for (language, rules) in &by_lang {
        // One directory per language, one file per rule. Detection is
        // per-file but the coverage table is per-run, so a single run
        // covers every rule in the language.
        let tmp = TempDir::new().unwrap();
        let mut wrote = 0usize;
        for (i, (id, prefix)) in rules.iter().enumerate() {
            let Some((src, ext)) = import_form(language, prefix) else {
                // Descriptor and config languages have no import statement
                // to synthesize; their rules are detected by file type or
                // by structure, which this test cannot express.
                if matches!(
                    *language,
                    "proto" | "graphql" | "openapi" | "asyncapi" | "smithy" | "hcl"
                ) {
                    break;
                }
                unsupported.push(language);
                break;
            };
            let name = format!("f{i}_{}.{ext}", id.replace(['-', '.'], "_"));
            fs::write(tmp.path().join(name), src).unwrap();
            wrote += 1;
        }
        if wrote == 0 {
            continue;
        }

        let out = tmp.path().join("g.mmd");
        let assert = Command::cargo_bin("cgg")
            .unwrap()
            .arg(tmp.path())
            .args(["-o"])
            .arg(&out)
            .args(["--framework-coverage", "--no-update-check"])
            .assert()
            .success();
        let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();

        let reported = reported_ids(&stderr);
        for (id, prefix) in rules {
            checked += 1;
            if !reported.contains(*id) {
                undetected.push(format!("{language}/{id} (prefix {prefix:?})"));
            }
        }
    }

    unsupported.sort_unstable();
    unsupported.dedup();
    assert!(
        unsupported.is_empty(),
        "no import form for language(s) {unsupported:?} — add one to \
         import_form() or these rules are untested"
    );
    assert!(
        undetected.is_empty(),
        "{} of {checked} rules were NOT detected from an import of their own \
         first `detect` prefix. Each is a rule that can never fire, which \
         makes the coverage table claim the framework was considered when it \
         was not:\n  {}",
        undetected.len(),
        undetected.join("\n  ")
    );
}
