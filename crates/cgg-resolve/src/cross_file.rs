//! Cross-file scope-aware resolver.
//!
//! This is a lightweight companion to the stack-graphs resolver. It
//! walks each file's declared imports and, for every call-site whose
//! simple name matches an imported symbol (or an imported module's
//! member), emits a cross-file `CallEdge` with `confidence=Medium`
//! and `resolver="cross-file:imports"`.
//!
//! It is deliberately conservative: it emits an edge only when there's
//! an unambiguous imported target. Ambiguous cases are left alone
//! (either the stack-graphs resolver or the intra-file linker will
//! have already made a decision about them).
//!
//! Rules, per file:
//!
//! * Python — `from m import foo` + call `foo(...)` → edge to
//!   `m.foo`. `import m as mod` + call `mod.bar(...)` → edge to
//!   `m.bar`. Matching on the file whose qualified-name chain
//!   begins with `m`.
//! * JS / TS — `import { foo } from "./m.js"` and ESM aliases likewise.
//!
//! For languages where the extractor produces well-formed imports
//! (Task 4 does this for Python and Rust) the resolver is effective.
//! For languages where Task 4 only stubbed extraction, this pass is a
//! no-op.

use std::collections::HashMap;

use cgg_core::{
    graph::{CallEdge, Confidence, Graph, Via},
    ids::{CallableId, FileId, ResolverId},
    FileFacts,
};

/// Output of the cross-file resolver.
#[derive(Debug, Default)]
pub struct CrossFileOutput {
    pub edges: Vec<CallEdge>,
}

/// Resolve call-site references across files using import tables.
pub fn resolve(graph: &Graph, facts: &[FileFacts]) -> CrossFileOutput {
    let mut out = CrossFileOutput::default();
    let resolver_id = ResolverId::new("cross-file:imports");

    // Index callables by (language, qualified_name) and (language, simple_name).
    let mut by_qn: HashMap<(String, String), CallableId> = HashMap::new();
    let mut by_simple: HashMap<(String, String), Vec<CallableId>> = HashMap::new();
    for c in graph.callables.values() {
        by_qn.insert(
            (c.language.clone(), c.qualified_name.clone()),
            c.id,
        );
        by_simple
            .entry((c.language.clone(), c.simple_name.clone()))
            .or_default()
            .push(c.id);
    }

    // Build a re-export map (Rust only, for now). Every `pub use` in a
    // file that lives under an identifiable crate makes that symbol
    // appear under the re-exporting crate's namespace. Example:
    //   crate cgg_core/src/lib.rs contains `pub use audit::AuditEvent;`
    //   => `cgg_core::AuditEvent` resolves to whatever
    //      `cgg_core::audit::AuditEvent` resolves to.
    let mut reexports: HashMap<(String, String), String> = HashMap::new();
    for f in facts {
        if f.language != "rust" {
            continue;
        }
        // Prefer the explicit crate-root marker emitted by the Rust
        // plugin; fall back to the first definition's crate prefix
        // or the literal "crate" sentinel.
        let crate_root = f
            .imports
            .iter()
            .find(|i| i.kind == "crate-root")
            .map(|i| i.path.clone())
            .or_else(|| {
                f.definitions
                    .first()
                    .and_then(|d| d.qualified_name.split("::").next())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "crate".to_string());
        for imp in &f.imports {
            if imp.kind != "pub-use" {
                continue;
            }
            let target = imp.path.clone();
            let exported_name = if imp.alias.is_empty() {
                target.rsplit("::").next().unwrap_or(&target).to_string()
            } else {
                imp.alias.clone()
            };
            let alias_qn = format!("{crate_root}::{exported_name}");
            reexports.insert(("rust".to_string(), alias_qn), target);
        }
    }

    let facts_by_id: HashMap<FileId, &FileFacts> =
        facts.iter().map(|f| (f.file, f)).collect();

    // Build a `(language, file_path_lowercased)` index so we can map
    // an unqualified import prefix back to the files it covers. Used
    // by the per-file resolution loop below to scope-narrow the
    // by-simple-name fallback for languages whose plugins don't include
    // a module prefix in `qualified_name` (Haskell, OCaml).
    let files_lower: Vec<(FileId, String, String)> = facts
        .iter()
        .map(|f| (f.file, f.language.clone(), f.path.to_string_lossy().to_ascii_lowercase()))
        .collect();

    for facts in facts {
        let lang = facts.language.clone();

        // Normalize imports into lookup tables:
        //   imported_simple_name -> candidate qualified_names.
        // Python: `from helpers import greet` -> map "greet" ->
        //   "helpers.greet".
        //   `import helpers as h` -> map "h" -> "helpers" (module prefix).
        // Rust: `use a::b::c;` -> map "c" -> "a::b::c".
        let mut direct_imports: HashMap<String, Vec<String>> = HashMap::new();
        let mut module_aliases: HashMap<String, String> = HashMap::new();
        // Namespace prefixes that bring symbols into scope unqualified —
        // e.g. Haskell `import Data.Map`, OCaml `open Foo`, Elixir
        // `import Foo`, F# `open System`, PowerShell `using namespace`.
        // Resolution tries `<prefix>.<ref-name>` (and `<prefix>::<name>`
        // for ::-joined languages) for each prefix.
        let mut unqualified_prefixes: Vec<String> = Vec::new();

        for imp in &facts.imports {
            match imp.kind.as_str() {
                "from-import" => {
                    // Python: imp.path is module; imp.alias is the
                    // items list ("greet, compute" or "greet as g").
                    // JS/TS: imp.path is relative path; items are
                    // exported names from that file.
                    let module = imp.path.trim();
                    for item in imp.alias.split(',') {
                        let (src, alias) = match item.split_once(" as ") {
                            Some((s, a)) => (s.trim(), a.trim()),
                            None => (item.trim(), item.trim()),
                        };
                        if src.is_empty() {
                            continue;
                        }
                        let qn = format!("{module}.{src}");
                        direct_imports
                            .entry(alias.to_string())
                            .or_default()
                            .push(qn);
                        // For JS/TS where definitions don't carry a
                        // module prefix, also try the bare name.
                        if module.starts_with('.') || module.starts_with('/') {
                            direct_imports
                                .entry(alias.to_string())
                                .or_default()
                                .push(src.to_string());
                        }
                    }
                }
                "import" if matches!(lang.as_str(), "python" | "go" | "javascript" | "typescript" | "swift" | "zig" | "r" | "perl") => {
                    // Python: `import a.b.c`               (no alias)
                    //         `import a.b.c as d`          (aliased)
                    // Go:     `import "fmt"`               (no alias)
                    //         `import "net/http"`          (no alias)
                    //         `import al "other/lib"`      (aliased)
                    //
                    // The call we want to resolve looks like
                    // `<root>.name()` — `<root>` is the alias if
                    // supplied, else the binding name implied by the
                    // path. That binding name is:
                    //   * Go (path contains '/'): last segment.
                    //   * Python (dotted path):   first segment.
                    //   * bare identifier:        the path itself.
                    // The target "module root" we map to:
                    //   * Go with slashes: the last segment
                    //                      (package name by
                    //                      convention = last dir).
                    //   * Python or bare: the full path.
                    let path = imp.path.trim();
                    let has_slash = path.contains('/');
                    let (binding, target) = if let Some(stripped_alias) =
                        Some(imp.alias.trim()).filter(|a| !a.is_empty() && *a != "_")
                    {
                        // Aliased — user wrote the binding name.
                        let target = if has_slash {
                            path.rsplit('/').next().unwrap_or(path).to_string()
                        } else {
                            path.to_string()
                        };
                        (stripped_alias.to_string(), target)
                    } else if has_slash {
                        let last = path.rsplit('/').next().unwrap_or(path).to_string();
                        (last.clone(), last)
                    } else if path.contains('.') {
                        // Python dotted — bind first segment, target is full.
                        let first = path.split('.').next().unwrap_or(path).to_string();
                        (first, path.to_string())
                    } else {
                        (path.to_string(), path.to_string())
                    };
                    if !binding.is_empty() {
                        module_aliases.insert(binding, target);
                    }
                }
                "use" | "pub-use" if lang == "rust" => {
                    // Rust: `a::b::c` or `a::b::c as d`.
                    let full = imp.path.trim();
                    let alias = if imp.alias.is_empty() {
                        full.rsplit("::").next().unwrap_or(full).to_string()
                    } else {
                        imp.alias.clone()
                    };
                    direct_imports
                        .entry(alias)
                        .or_default()
                        .push(full.to_string());
                }
                "using" if lang == "csharp" => {
                    // C#: `using X.Y.Z;`                 -> module alias Z -> X.Y.Z
                    //     `using Alias = X.Y.Z;`         -> alias Alias -> X.Y.Z
                    let full = imp.path.trim();
                    if !imp.alias.is_empty() {
                        module_aliases
                            .insert(imp.alias.clone(), full.to_string());
                    } else if let Some(last) = full.rsplit('.').next() {
                        module_aliases.insert(last.to_string(), full.to_string());
                    }
                }
                "using-static" => {
                    // C#: `using static X.Y;` — every member of Y is
                    // callable unqualified. We record each definition
                    // by its leaf name once we've walked the graph;
                    // at resolve time we try `X.Y.<name>` directly.
                    let full = imp.path.trim().to_string();
                    direct_imports
                        .entry("__using_static__".into())
                        .or_default()
                        .push(full);
                }
                "include" if matches!(lang.as_str(), "c" | "cpp" | "objc") => {
                    // C/C++: `#include "helpers.h"` — all definitions
                    // from the included file become available in this
                    // TU. We resolve the path relative to the current
                    // file and transitively chase includes up to 8
                    // levels deep.
                    let included_path = imp.path.trim();
                    if !included_path.is_empty() {
                        collect_include_defs(
                            included_path,
                            facts,
                            &facts_by_id,
                            &mut direct_imports,
                            8,
                        );
                    }
                }
                "source" => {
                    // Bash: `source ./lib.sh` — same semantics as
                    // C #include: all definitions from the sourced
                    // file become available.
                    let sourced_path = imp.path.trim();
                    if !sourced_path.is_empty() {
                        collect_include_defs(
                            sourced_path,
                            facts,
                            &facts_by_id,
                            &mut direct_imports,
                            4,
                        );
                    }
                }
                "require" => {
                    // Ruby: `require './helper'`
                    // Lua:  `local m = require('foo.bar')`  (path = "foo.bar")
                    // Clojure: `(:require [foo.bar :as fb])`
                    // Erlang: `-include("x.hrl").`           (kind="include" — handled above)
                    // All three want every definition from the named file
                    // to become reachable. We try a few path resolutions.
                    let req_path = imp.path.trim();
                    if req_path.is_empty() { continue; }

                    // 1) Direct file-include resolution.
                    for try_path in [
                        req_path.to_string(),
                        format!("{req_path}.rb"),
                        format!("{req_path}.lua"),
                        format!("{req_path}.clj"),
                        req_path.replace('.', "/") + ".lua",
                        req_path.replace('.', "/") + ".clj",
                    ] {
                        collect_include_defs(&try_path, facts, &facts_by_id, &mut direct_imports, 4);
                    }

                    // 2) Module-alias / unqualified-prefix.
                    if !imp.alias.is_empty() {
                        module_aliases.insert(imp.alias.clone(), req_path.to_string());
                    } else {
                        let last = req_path
                            .rsplit(|c: char| c == '.' || c == '/' || c == ':')
                            .next()
                            .unwrap_or(req_path);
                        if !last.is_empty() {
                            module_aliases.insert(last.to_string(), req_path.to_string());
                        }
                    }
                    unqualified_prefixes.push(req_path.to_string());
                }
                "load" => {
                    // Starlark: `load("//path:file.bzl", "symbol", aliased="other")`.
                    // path is the .bzl file ref. We treat it as include-like
                    // (every def in the loaded file becomes available) since
                    // tracking the actual symbols list would require a
                    // separate Starlark-aware import representation.
                    let raw = imp.path.trim().trim_start_matches("//").trim_matches('"');
                    let cleaned = raw.replace(':', "/");
                    for try_path in [cleaned.clone(), format!("{cleaned}.bzl")] {
                        collect_include_defs(&try_path, facts, &facts_by_id, &mut direct_imports, 4);
                    }
                }
                "open" => {
                    // OCaml / F#: `open Module` — brings every symbol in
                    // `Module` into scope unqualified.
                    let path = imp.path.trim();
                    if !path.is_empty() {
                        unqualified_prefixes.push(path.to_string());
                        // Last segment also aliases the module.
                        if let Some(last) = path.rsplit('.').next() {
                            module_aliases.insert(last.to_string(), path.to_string());
                        }
                    }
                }
                "alias" => {
                    // Elixir: `alias Foo.Bar` => Bar refers to Foo.Bar.
                    // `alias Foo.Bar, as: B` => B refers to Foo.Bar.
                    let path = imp.path.trim();
                    let alias = if imp.alias.is_empty() {
                        path.rsplit('.').next().unwrap_or(path).to_string()
                    } else {
                        imp.alias.clone()
                    };
                    if !alias.is_empty() {
                        module_aliases.insert(alias, path.to_string());
                    }
                }
                "use" => {
                    // Elixir `use Foo` / Fortran `use module` — typically
                    // brings module contents into scope unqualified.
                    // Rust `use` is handled above; this arm only fires for
                    // other languages because match arms above already
                    // claim the kind for Rust.
                    let path = imp.path.trim();
                    if !path.is_empty() {
                        unqualified_prefixes.push(path.to_string());
                    }
                }
                "import qualified" => {
                    // Haskell: `import qualified Data.Map [as M]`. Without
                    // an alias, the module is referenced by its full name
                    // (`Data.Map.lookup`); with an alias, by `M.lookup`.
                    let path = imp.path.trim();
                    let alias = if imp.alias.is_empty() { path } else { imp.alias.as_str() };
                    if !alias.is_empty() {
                        module_aliases.insert(alias.to_string(), path.to_string());
                    }
                }
                "import" => {
                    // Generic "import" — language-specific dispatch. The
                    // Python/Go/JS variant is handled by the earlier arm
                    // pattern via specific kinds; here we cover the langs
                    // whose plugins emit a bare "import" kind.
                    let path = imp.path.trim();
                    if path.is_empty() { continue; }
                    match lang.as_str() {
                        // Scala / Java-style: `import pkg.{A,B}` or `import pkg.A`.
                        "scala" | "java" | "kotlin" | "groovy" => {
                            if let Some(idx) = path.rfind('.') {
                                let prefix = &path[..idx];
                                let suffix = &path[idx + 1..];
                                let suffix = suffix.trim_matches(|c| c == '{' || c == '}');
                                if suffix == "_" || suffix == "*" {
                                    unqualified_prefixes.push(prefix.to_string());
                                } else {
                                    for name in suffix.split(',') {
                                        let name = name.trim();
                                        if name.is_empty() { continue; }
                                        let (src, alias) = match name.split_once("=>") {
                                            Some((s, a)) => (s.trim(), a.trim()),
                                            None => (name, name),
                                        };
                                        direct_imports
                                            .entry(alias.to_string())
                                            .or_default()
                                            .push(format!("{prefix}.{src}"));
                                    }
                                }
                                if let Some(last) = prefix.rsplit('.').next() {
                                    module_aliases.insert(last.to_string(), prefix.to_string());
                                }
                            }
                        }
                        // Dart / Solidity / Nix: file-relative paths.
                        "dart" | "solidity" | "nix" => {
                            let cleaned = path.trim_matches(|c| c == '\'' || c == '"' || c == '<' || c == '>');
                            for try_path in [cleaned.to_string(), format!("{cleaned}.sol"), format!("{cleaned}.dart"), format!("{cleaned}.nix")] {
                                collect_include_defs(&try_path, facts, &facts_by_id, &mut direct_imports, 4);
                            }
                            if !imp.alias.is_empty() {
                                let derived = cleaned
                                    .rsplit('/')
                                    .next()
                                    .unwrap_or(cleaned)
                                    .trim_end_matches(".dart")
                                    .trim_end_matches(".sol")
                                    .trim_end_matches(".nix")
                                    .to_string();
                                module_aliases.insert(imp.alias.clone(), derived);
                            }
                        }
                        // Haskell / Erlang / Elixir / generic: dotted module name,
                        // unqualified import.
                        "haskell" | "erlang" | "elixir" | "fsharp" | "ocaml" | "julia" => {
                            unqualified_prefixes.push(path.to_string());
                            if let Some(last) = path.rsplit('.').next() {
                                module_aliases.insert(last.to_string(), path.to_string());
                            }
                        }
                        _ => {
                            // Fall back to module-alias on last segment.
                            let last = path
                                .rsplit(|c: char| c == '.' || c == '/' || c == ':')
                                .next()
                                .unwrap_or(path);
                            if !last.is_empty() {
                                module_aliases.insert(last.to_string(), path.to_string());
                            }
                        }
                    }
                }
                "using" if lang == "powershell" => {
                    // PowerShell `using namespace System.IO` — namespace open.
                    let path = imp.path.trim();
                    if !path.is_empty() {
                        unqualified_prefixes.push(path.to_string());
                    }
                }
                k if k.starts_with("using-") && lang == "powershell" => {
                    let path = imp.path.trim();
                    if !path.is_empty() {
                        unqualified_prefixes.push(path.to_string());
                    }
                }
                "import-module" | "dot-source" => {
                    // PowerShell: include-like.
                    let path = imp.path.trim();
                    for try_path in [path.to_string(), format!("{path}.psm1"), format!("{path}.ps1")] {
                        collect_include_defs(&try_path, facts, &facts_by_id, &mut direct_imports, 4);
                    }
                    unqualified_prefixes.push(path.to_string());
                }
                "include" | "add_subdirectory" | "find_package" => {
                    // CMake-style file inclusion (the C/C++ "include" arm
                    // above already claims the kind for those languages).
                    if lang == "cmake" || lang == "verilog" || lang == "vhdl"
                        || lang == "erlang" || lang == "fortran"
                    {
                        let path = imp.path.trim();
                        for try_path in [
                            path.to_string(),
                            format!("{path}.cmake"),
                            format!("{path}.v"),
                            format!("{path}.hrl"),
                            format!("{path}.f90"),
                            format!("{path}.f95"),
                            format!("CMakeLists.txt"),
                        ] {
                            collect_include_defs(&try_path, facts, &facts_by_id, &mut direct_imports, 4);
                        }
                    }
                }
                "using-namespace" => {
                    let path = imp.path.trim();
                    if !path.is_empty() {
                        unqualified_prefixes.push(path.to_string());
                    }
                }
                _ => {}
            }
        }

        // Compute "scoped candidates": for each unqualified_prefix,
        // collect the callables defined in files whose path matches
        // the prefix (e.g. `Text.Pandoc` matches `*/Text/Pandoc.hs`).
        // This is what lets Haskell / OCaml — whose plugins don't put
        // module prefixes on qualified_name — still get cross-file
        // resolution without resorting to global by-simple noise.
        let mut scoped_simple: HashMap<String, Vec<CallableId>> = HashMap::new();
        if !unqualified_prefixes.is_empty()
            || !direct_imports.is_empty()
            || !module_aliases.is_empty()
        {
            let mut path_fragments: Vec<String> = unqualified_prefixes
                .iter()
                .map(|p| p.replace('.', "/").to_ascii_lowercase())
                .collect();
            // Also include single-segment module aliases (Lua `require('foo')`,
            // Scala `import play.Foo` last segment, etc.)
            for (_, target) in &module_aliases {
                path_fragments.push(target.replace('.', "/").to_ascii_lowercase());
            }
            for (fid, flang, fpath) in &files_lower {
                if flang != &lang { continue; }
                if !path_fragments.iter().any(|frag| !frag.is_empty() && fpath.contains(frag.as_str())) {
                    continue;
                }
                if let Some(target) = facts_by_id.get(fid) {
                    for d in &target.definitions {
                        if let Some(cid) = by_qn
                            .get(&(lang.clone(), d.qualified_name.clone()))
                            .copied()
                        {
                            scoped_simple
                                .entry(d.simple_name.clone())
                                .or_default()
                                .push(cid);
                        }
                    }
                }
            }
        }

        for r in &facts.references {
            // Compute enclosing callable up front so we can pass its
            // qualified name into the resolver — needed for the
            // intra-crate qualified-path retry (e.g., `crawl::foo()`
            // inside `nkb_research::ResearchRunner::run` should find
            // `nkb_research::crawl::foo`).
            let enclosing = enclosing_callable_id(graph, facts, r.site_byte);
            let caller_qn = enclosing
                .and_then(|id| graph.callables.get(&id))
                .map(|c| c.qualified_name.as_str());

            if let Some(cids) = try_resolve_ref(
                &lang,
                r,
                &direct_imports,
                &module_aliases,
                &unqualified_prefixes,
                &scoped_simple,
                &by_qn,
                &by_simple,
                &reexports,
                caller_qn,
            ) {
                for cid in cids {
                    // Skip self-edges that coincide with intra-file's
                    // ones (they'd be duplicates with the same resolver).
                    if let Some(src) = enclosing {
                        if src == cid {
                            continue;
                        }
                        // Avoid duplicating intra-file-emitted edges.
                        let dup = graph
                            .edges
                            .iter()
                            .any(|e| e.src == src && e.dst == cid && e.site_byte == r.site_byte);
                        if dup {
                            continue;
                        }
                        out.edges.push(CallEdge {
                            src,
                            dst: cid,
                            site_line: r.site_line,
                            site_byte: r.site_byte,
                            confidence: Confidence::Medium,
                            via: Via::Direct,
                            resolver: resolver_id.clone(),
                        });
                    }
                }
            }
        }

        let _ = &facts_by_id;
    }

    out
}

/// Collect definitions from an included header file and add them as
/// direct imports. Transitively follows `#include` directives in the
/// header up to `depth` levels.
fn collect_include_defs(
    include_path: &str,
    includer_facts: &FileFacts,
    facts_by_id: &HashMap<FileId, &FileFacts>,
    direct_imports: &mut HashMap<String, Vec<String>>,
    depth: u8,
) {
    if depth == 0 {
        return;
    }
    // Resolve the include path relative to the includer's directory.
    let includer_dir = includer_facts
        .path
        .parent()
        .unwrap_or(std::path::Path::new(""));
    let resolved = includer_dir.join(include_path);
    // Find the matching FileFacts by path suffix (handles both
    // absolute and relative paths in the index).
    let target = facts_by_id.values().find(|f| {
        f.path == resolved || f.path.ends_with(include_path)
    });
    let Some(target) = target else { return };
    // Import all definitions from the target.
    for d in &target.definitions {
        direct_imports
            .entry(d.simple_name.clone())
            .or_default()
            .push(d.qualified_name.clone());
    }
    // Transitively follow includes in the target.
    for imp in &target.imports {
        if imp.kind == "include" {
            collect_include_defs(
                imp.path.trim(),
                target,
                facts_by_id,
                direct_imports,
                depth - 1,
            );
        }
    }
}

fn try_resolve_ref(
    lang: &str,
    r: &cgg_core::RefRecord,
    direct_imports: &HashMap<String, Vec<String>>,
    module_aliases: &HashMap<String, String>,
    unqualified_prefixes: &[String],
    scoped_simple: &HashMap<String, Vec<CallableId>>,
    by_qn: &HashMap<(String, String), CallableId>,
    by_simple: &HashMap<(String, String), Vec<CallableId>>,
    reexports: &HashMap<(String, String), String>,
    caller_qn: Option<&str>,
) -> Option<Vec<CallableId>> {
    // Step 1: direct import match — `foo()` where `foo` was
    // imported.
    if r.receiver_hint.is_empty() {
        if let Some(qns) = direct_imports.get(&r.name) {
            let cids: Vec<_> = qns
                .iter()
                .filter_map(|qn| lookup_with_reexports(lang, qn, by_qn, reexports))
                .collect();
            if !cids.is_empty() {
                return Some(cids);
            }
        }
        // Also check module_aliases for bare calls — handles
        // Kotlin/Java `import com.example.Foo` + `Foo()` constructor
        // and `import com.example.helper` + `helper()` top-level fn.
        if let Some(target) = module_aliases.get(&r.name) {
            // Try target.name (the full path IS the callable)
            if let Some(cid) = lookup_with_reexports(lang, target, by_qn, reexports) {
                return Some(vec![cid]);
            }
            // Try just the name itself as a qualified name
            if let Some(cid) = lookup_with_reexports(lang, &r.name, by_qn, reexports) {
                return Some(vec![cid]);
            }
        }
        // Step 1b: bare-name lookup via unqualified-import prefixes
        // (Haskell `import Data.Map` + `lookup`, OCaml `open Foo` + `bar`,
        // Elixir `import Foo` + `bar`, F# `open System.IO` + `File`, …).
        for prefix in unqualified_prefixes {
            for joiner in [".", "::"] {
                let qn = format!("{prefix}{joiner}{}", r.name);
                if let Some(cid) = lookup_with_reexports(lang, &qn, by_qn, reexports) {
                    return Some(vec![cid]);
                }
            }
        }
        // Step 1c: scoped by-simple lookup — restricted to callables
        // defined in files whose path matches one of this file's
        // import prefixes. This carries Haskell/OCaml/Dart over the
        // gap where the plugin omits the module prefix from
        // qualified_name. Cap candidates at 8 to bound noise.
        if let Some(cids) = scoped_simple.get(&r.name) {
            if !cids.is_empty() && cids.len() <= 8 {
                return Some(cids.clone());
            }
        }
        // Step 1d: global by-simple fallback. Last resort — only when
        // the file has at least one import and the simple name is
        // unique-ish (≤3 candidates). Skip stdlib-ish names.
        let has_imports = !direct_imports.is_empty()
            || !module_aliases.is_empty()
            || !unqualified_prefixes.is_empty();
        if has_imports {
            let is_stdlib = cgg_core::stdlib::stdlib_names(lang)
                .map_or(false, |s| s.contains(r.name.as_str()));
            if !is_stdlib {
                if let Some(cids) = by_simple.get(&(lang.to_string(), r.name.clone())) {
                    if !cids.is_empty() && cids.len() <= 3 {
                        return Some(cids.clone());
                    }
                }
            }
        }
    } else {
        // Step 2: attribute call `mod.fn()` where `mod` is aliased.
        // receiver_hint is the full receiver expression (e.g., "mod"
        // or "mod.sub"). Take its first segment to match module alias.
        let first = r
            .receiver_hint
            .split(|c| c == '.' || c == ':')
            .next()
            .unwrap_or("");
        if let Some(module) = module_aliases.get(first) {
            // Rebuild the full target path. For `mod.fn()` with alias
            // `mod=helpers` -> `helpers.fn`. For `mod.sub.fn()` ->
            // `helpers.sub.fn`.
            let rest = r.receiver_hint.strip_prefix(first).unwrap_or("");
            let qn = format!(
                "{module}{rest}.{}",
                r.name
            );
            if let Some(cid) = lookup_with_reexports(lang, &qn, by_qn, reexports) {
                return Some(vec![cid]);
            }
            // Rust path joiner.
            let qn2 = format!("{module}{}::{}", rest.replace('.', "::"), r.name);
            if let Some(cid) = lookup_with_reexports(lang, &qn2, by_qn, reexports) {
                return Some(vec![cid]);
            }
            // For JS/TS: definitions don't carry a module prefix, so
            // try bare name as fallback when the module is a relative
            // path or a short package-like name that doesn't match
            // any qualified name prefix.
            if let Some(cid) = lookup_with_reexports(lang, &r.name, by_qn, reexports) {
                return Some(vec![cid]);
            }
        }

        // Step 3: qualified-path call `foo::bar::baz()` (Rust) or
        // `foo.bar.baz()` (Python / Go / C#). The receiver_hint is
        // already the joined path. Try both the Rust and the dotted
        // form.
        let rh = r.receiver_hint.trim();
        if !rh.is_empty() {
            // Direct paths in both joiners.
            let direct_dot = format!("{rh}.{}", r.name);
            if let Some(cid) = lookup_with_reexports(lang, &direct_dot, by_qn, reexports) {
                return Some(vec![cid]);
            }
            let direct = format!("{rh}::{}", r.name);
            if let Some(cid) = lookup_with_reexports(lang, &direct, by_qn, reexports) {
                return Some(vec![cid]);
            }
            // Step 3b: Rust intra-crate retry — when `mod::fn()` lives
            // inside `crate::other::Type::method`, the qualified name
            // we want is `crate::mod::fn`. Walk every prefix of the
            // caller's qualified name, prepending it to `<rh>::<name>`,
            // until we hit a match. Shortest prefix first (just the
            // crate) is the most common hit. Limit to `::` joiner —
            // dot-joined languages don't have this resolution rule.
            if lang == "rust" {
                if let Some(qn) = caller_qn {
                    let segs: Vec<&str> = qn.split("::").collect();
                    // Try crate-only first, then progressively longer
                    // prefixes. Stop before the last segment (that's
                    // the callable's own name).
                    for i in 1..segs.len() {
                        let prefix = segs[..i].join("::");
                        let candidate = format!("{prefix}::{rh}::{}", r.name);
                        if let Some(cid) =
                            lookup_with_reexports(lang, &candidate, by_qn, reexports)
                        {
                            return Some(vec![cid]);
                        }
                    }
                }
            }
            // If the head segment is imported as something else, rewrite.
            // e.g., `use foo as f; f::bar()` -> receiver=f, name=bar -> foo::bar.
            if let Some(first) = rh.split(|c| c == '.' || c == ':').next() {
                if let Some(qns) = direct_imports.get(first) {
                    for base in qns {
                        let rest = rh.strip_prefix(first).unwrap_or("");
                        let rewritten_colon =
                            format!("{base}{}::{}", rest.replace('.', "::"), r.name);
                        if let Some(cid) = lookup_with_reexports(
                            lang,
                            &rewritten_colon,
                            by_qn,
                            reexports,
                        ) {
                            return Some(vec![cid]);
                        }
                        let rewritten_dot = format!("{base}{rest}.{}", r.name);
                        if let Some(cid) = lookup_with_reexports(
                            lang,
                            &rewritten_dot,
                            by_qn,
                            reexports,
                        ) {
                            return Some(vec![cid]);
                        }
                    }
                }
            }
        }

        // Step 4: Type-qualified method call fallback.
        // When receiver_hint is a type name (e.g. "MermaidFormatter") and
        // name is "new", search all callables for one whose qualified_name
        // ends with "::MermaidFormatter::new" or "::MermaidFormatter::new".
        // This handles cross-crate constructor/method calls without needing
        // explicit import tracking.
        let rh = r.receiver_hint.trim();
        if !rh.is_empty()
            && rh != "self" && rh != "Self" && rh != "cls"
            && rh.chars().next().map_or(false, |c| c.is_uppercase())
        {
            let suffix_colon = format!("::{}::{}", rh, r.name);
            let suffix_dot = format!(".{}.{}", rh, r.name);
            let cids: Vec<_> = by_qn
                .iter()
                .filter(|((l, qn), _)| {
                    l == lang && (qn.ends_with(&suffix_colon) || qn.ends_with(&suffix_dot))
                })
                .map(|(_, &cid)| cid)
                .collect();
            if cids.len() == 1 {
                return Some(cids);
            }
            // If multiple matches, still return them — let the caller
            // pick the best or emit all as medium-confidence.
            if !cids.is_empty() {
                return Some(cids);
            }
        }

        // Step 5: Trait/interface method dispatch.
        // When receiver_hint is a variable name (lowercase) and the method
        // name exists on definitions in other files, find all callables
        // with that simple_name. This handles `formatter.render()` where
        // formatter is a trait object — we emit edges to all implementors.
        // Only applies when the method name is NOT a common stdlib method
        // (to avoid matching `vec.is_empty()` to `WalkOutcome::is_empty`).
        if !rh.is_empty()
            && rh != "self" && rh != "Self" && rh != "cls"
            && rh.chars().next().map_or(false, |c| c.is_lowercase())
        {
            // Skip if the method name is in the stdlib manifest for this language
            let is_stdlib_method = cgg_core::stdlib::stdlib_names(lang)
                .map_or(false, |std| std.contains(r.name.as_str()));
            if !is_stdlib_method {
                if let Some(cids) = by_simple.get(&(lang.to_string(), r.name.clone())) {
                    // Only use this if there's a small number of candidates
                    if cids.len() <= 5 && !cids.is_empty() {
                        return Some(cids.clone());
                    }
                }
            }
        }
    }
    None
}

/// Look up `qn` in the callable index, following Rust `pub use`
/// re-export chains up to a small depth cap so malformed graphs can't
/// loop.
fn lookup_with_reexports(
    lang: &str,
    qn: &str,
    by_qn: &HashMap<(String, String), CallableId>,
    reexports: &HashMap<(String, String), String>,
) -> Option<CallableId> {
    let mut current = qn.to_string();
    for _ in 0..8 {
        if let Some(cid) = by_qn.get(&(lang.to_string(), current.clone())).copied() {
            return Some(cid);
        }
        if let Some(next) = reexports.get(&(lang.to_string(), current.clone())) {
            current = next.clone();
            continue;
        }
        return None;
    }
    None
}

fn enclosing_callable_id(
    graph: &Graph,
    facts: &FileFacts,
    byte: u32,
) -> Option<CallableId> {
    let mut best: Option<(&cgg_core::DefRecord, u32)> = None;
    for d in &facts.definitions {
        if d.start_byte <= byte && byte < d.end_byte {
            let span = d.end_byte - d.start_byte;
            match best {
                None => best = Some((d, span)),
                Some((_, b)) if span < b => best = Some((d, span)),
                _ => {}
            }
        }
    }
    let (d, _) = best?;
    graph
        .callables
        .values()
        .find(|c| {
            c.file == facts.file
                && c.start_byte == d.start_byte
                && c.end_byte == d.end_byte
        })
        .map(|c| c.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::{
        graph::{CallableKind, CallableNode, FileRecord as GraphFileRecord},
        DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord,
    };
    use std::path::PathBuf;

    fn mk_file(id: u32, path: &str, lang: &str) -> GraphFileRecord {
        GraphFileRecord {
            id: FileId::new(id),
            path: PathBuf::from(path),
            language: lang.into(),
            detected_via: "ext:.py".into(),
            blake3: "0".repeat(64),
            size_bytes: 10,
            lines: 1,
            parse_ms: 0.0,
            parse_status: "ok".into(),
        }
    }

    fn mk_callable(
        id: u32,
        simple: &str,
        qn: &str,
        file: u32,
        lang: &str,
        byte_range: (u32, u32),
    ) -> CallableNode {
        CallableNode {
            id: CallableId::new(id),
            qualified_name: qn.into(),
            simple_name: simple.into(),
            kind: CallableKind::Function,
            language: lang.into(),
            file: FileId::new(file),
            start_line: 1,
            end_line: 1,
            start_byte: byte_range.0,
            end_byte: byte_range.1,
            signature_hint: String::new(),
            visibility: String::new(),
            attributes: vec![],
        }
    }

    fn mk_def(
        simple: &str,
        qn: &str,
        variant: DefVariant,
        byte_range: (u32, u32),
    ) -> DefRecord {
        DefRecord {
            simple_name: simple.into(),
            qualified_name: qn.into(),
            variant,
            start_line: 1,
            end_line: 1,
            start_byte: byte_range.0,
            end_byte: byte_range.1,
            signature_hint: String::new(),
            visibility: String::new(),
            attributes: vec![],
        }
    }

    fn facts_for(
        file: u32,
        path: &str,
        lang: &str,
        defs: Vec<DefRecord>,
        refs: Vec<RefRecord>,
        imports: Vec<ImportRecord>,
    ) -> FileFacts {
        FileFacts {
            file: FileId::new(file),
            path: PathBuf::from(path),
            language: lang.into(),
            definitions: defs,
            references: refs,
            imports,
            local_types: Vec::new(),
        }
    }

    #[test]
    fn python_from_import_direct_call() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "helpers.py", "python"));
        g.add_file(mk_file(1, "main.py", "python"));
        g.add_callable(mk_callable(
            0, "greet", "helpers.greet", 0, "python", (0, 40),
        ));
        g.add_callable(mk_callable(
            1, "process", "main.process", 1, "python", (30, 120),
        ));

        let main_facts = facts_for(
            1,
            "main.py",
            "python",
            vec![mk_def("process", "main.process", DefVariant::FreeFunction, (30, 120))],
            vec![RefRecord {
                name: "greet".into(),
                receiver_hint: "".into(),
                site_line: 5,
                site_byte: 60,
            }],
            vec![ImportRecord {
                kind: "from-import".into(),
                path: "helpers".into(),
                alias: "greet, compute".into(),
                site_line: 1,
                site_byte: 0,
            }],
        );
        let helpers_facts = facts_for(
            0,
            "helpers.py",
            "python",
            vec![mk_def("greet", "helpers.greet", DefVariant::FreeFunction, (0, 40))],
            vec![],
            vec![],
        );

        let out = resolve(&g, &[helpers_facts, main_facts]);
        assert_eq!(out.edges.len(), 1, "expected one cross-file edge");
        assert_eq!(out.edges[0].src, CallableId::new(1));
        assert_eq!(out.edges[0].dst, CallableId::new(0));
        assert_eq!(out.edges[0].confidence, Confidence::Medium);
        assert_eq!(out.edges[0].resolver.as_str(), "cross-file:imports");
    }

    #[test]
    fn python_module_alias_attribute_call() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "helpers.py", "python"));
        g.add_file(mk_file(1, "main.py", "python"));
        g.add_callable(mk_callable(
            0, "compute", "helpers.compute", 0, "python", (0, 40),
        ));
        g.add_callable(mk_callable(
            1, "top", "main.top", 1, "python", (30, 120),
        ));

        let main_facts = facts_for(
            1,
            "main.py",
            "python",
            vec![mk_def("top", "main.top", DefVariant::FreeFunction, (30, 120))],
            vec![RefRecord {
                name: "compute".into(),
                receiver_hint: "h".into(),
                site_line: 5,
                site_byte: 60,
            }],
            vec![ImportRecord {
                kind: "import".into(),
                path: "helpers".into(),
                alias: "h".into(),
                site_line: 1,
                site_byte: 0,
            }],
        );
        let helpers_facts = facts_for(
            0,
            "helpers.py",
            "python",
            vec![mk_def(
                "compute",
                "helpers.compute",
                DefVariant::FreeFunction,
                (0, 40),
            )],
            vec![],
            vec![],
        );

        let out = resolve(&g, &[helpers_facts, main_facts]);
        assert_eq!(out.edges.len(), 1);
        assert_eq!(out.edges[0].dst, CallableId::new(0));
    }

    #[test]
    fn rust_use_direct_call() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "lib.rs", "rust"));
        g.add_file(mk_file(1, "main.rs", "rust"));
        g.add_callable(mk_callable(
            0, "helper", "crate::util::helper", 0, "rust", (0, 40),
        ));
        g.add_callable(mk_callable(
            1, "main", "crate::main", 1, "rust", (30, 120),
        ));

        let main_facts = facts_for(
            1,
            "main.rs",
            "rust",
            vec![mk_def(
                "main",
                "crate::main",
                DefVariant::FreeFunction,
                (30, 120),
            )],
            vec![RefRecord {
                name: "helper".into(),
                receiver_hint: "".into(),
                site_line: 5,
                site_byte: 60,
            }],
            vec![ImportRecord {
                kind: "use".into(),
                path: "crate::util::helper".into(),
                alias: "".into(),
                site_line: 1,
                site_byte: 0,
            }],
        );
        let lib_facts = facts_for(
            0,
            "lib.rs",
            "rust",
            vec![mk_def(
                "helper",
                "crate::util::helper",
                DefVariant::FreeFunction,
                (0, 40),
            )],
            vec![],
            vec![],
        );

        let out = resolve(&g, &[lib_facts, main_facts]);
        assert_eq!(out.edges.len(), 1);
        assert_eq!(out.edges[0].dst, CallableId::new(0));
    }
}
