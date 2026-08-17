// Pipeline helpers thread run state explicitly rather than through a
// context struct, which keeps each stage's inputs visible at the call
// site. The arity is the point, not an accident.
#![allow(clippy::too_many_arguments)]
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

use rayon::prelude::*;

use cgg_core::{
    FileFacts,
    audit::{AuditUnresolvedCall, UnresolvedReason},
    graph::{CallEdge, Confidence, Graph, Via},
    ids::{CallableId, FileId, ResolverId},
};

use crate::names::owner_from_qn;

/// Output of the cross-file resolver.
#[derive(Debug, Default)]
pub struct CrossFileOutput {
    pub edges: Vec<CallEdge>,
    /// Sites this pass saw but could not turn into an edge, and that no
    /// earlier pass recorded either. Currently only module-scope value
    /// references — see the `VALUE_REF_HINT` arm in [`resolve`].
    pub unresolved: Vec<AuditUnresolvedCall>,
}

/// What a language calls the method a constructor call lands on.
///
/// `Widget(3)` names a class; the callable it enters is that class's
/// initializer. cgg has no node for a type, so without this mapping
/// "who constructs X?" is unanswerable — in the field report, 107
/// constructors had zero inbound edges out of 1206.
fn constructor_names(lang: &str) -> &'static [&'static str] {
    match lang {
        "python" => &["__init__"],
        "javascript" | "typescript" => &["constructor"],
        "php" => &["__construct"],
        "ruby" => &["initialize"],
        _ => &[],
    }
}

/// The method an instance-call `x(...)` enters when `x` is an object.
fn call_operator_names(lang: &str) -> &'static [&'static str] {
    match lang {
        "python" => &["__call__"],
        "php" => &["__invoke"],
        "ruby" => &["call"],
        _ => &[],
    }
}

/// Walk a type's declared bases looking for one that owns `method`.
///
/// Python resolves an inherited call through the MRO; cgg matched only
/// the instantiated class, so `w.apply()` on a subclass that inherits
/// `apply` produced no edge while `w.extra()` declared on the subclass
/// did. Bounded and visited-guarded: a base list read from syntax can be
/// cyclic, and depth is not evidence.
fn resolve_via_bases(
    lang: &str,
    owner: &str,
    method: &str,
    by_owner_method: &HashMap<(String, String, String), Vec<CallableId>>,
    bases_by_owner: &HashMap<(String, String), Vec<String>>,
) -> Option<Vec<CallableId>> {
    let _sp = cgg_core::profile::span("xfile::via-bases");
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut frontier: Vec<String> = vec![owner.to_string()];
    for _ in 0..8 {
        let mut next: Vec<String> = Vec::new();
        for t in std::mem::take(&mut frontier) {
            if !seen.insert(t.clone()) {
                continue;
            }
            if t != owner
                && let Some(cids) = by_owner_method.get(&(
                    lang.to_string(),
                    t.clone(),
                    method.to_string(),
                ))
                && !cids.is_empty()
            {
                return Some(cids.clone());
            }
            if let Some(bases) = bases_by_owner.get(&(lang.to_string(), t)) {
                next.extend(bases.iter().cloned());
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    None
}

/// Parameter names a signature accepts, and whether it takes `**kwargs`.
///
/// Parsed from `signature_hint`, which the extractors already record —
/// no new extraction, and no attempt to be a type checker. `None` means
/// "cannot tell", and every caller treats that as "accepts anything".
fn accepted_params(sig: &str) -> Option<(std::collections::HashSet<String>, bool)> {
    let open = sig.find('(')?;
    let rest = &sig[open + 1..];
    let mut depth = 0i32;
    let mut end = rest.len();
    for (i, c) in rest.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' if depth == 0 => {
                end = i;
                break;
            }
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    let mut names = std::collections::HashSet::new();
    let mut star_star = false;
    let mut depth = 0i32;
    let mut cur = String::new();
    fn flush(
        cur: &mut String,
        names: &mut std::collections::HashSet<String>,
        star_star: &mut bool,
    ) {
        let t = cur.trim();
        if t.starts_with("**") {
            *star_star = true;
        } else {
            let name = t
                .trim_start_matches('*')
                .split([':', '='])
                .next()
                .unwrap_or("")
                .trim();
            if !name.is_empty() {
                names.insert(name.to_string());
            }
        }
        cur.clear();
    }
    for c in rest[..end].chars() {
        match c {
            '(' | '[' | '{' => {
                depth += 1;
                cur.push(c);
            }
            ')' | ']' | '}' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth <= 0 => flush(&mut cur, &mut names, &mut star_star),
            _ => cur.push(c),
        }
    }
    flush(&mut cur, &mut names, &mut star_star);
    Some((names, star_star))
}

/// Whether `sig` could accept a call passing these keyword names.
///
/// Deliberately one-sided: it returns `false` only when a keyword is
/// provably not a parameter and the signature has no `**kwargs`. An
/// unparseable signature, or one cgg has no hint for, accepts anything —
/// narrowing fan-out must not become a way to lose real edges.
fn signature_accepts(sig: &str, kwargs: &[String]) -> bool {
    if kwargs.is_empty() || sig.is_empty() {
        return true;
    }
    let Some((params, star_star)) = accepted_params(sig) else {
        return true;
    };
    if star_star || params.is_empty() {
        return true;
    }
    kwargs.iter().all(|k| params.contains(k))
}

/// The default duck-typing fan-out cap.
///
/// When a method call's receiver type is unknown, cgg emits an edge to
/// every same-named method it can see. Past a handful that stops being
/// informative and starts being noise, so the set is dropped — but the
/// drop is recorded (`fanout-cap-exceeded`), never silent.
pub const DEFAULT_FANOUT_CAP: usize = 5;

/// Resolve call-site references across files using import tables.
pub fn resolve(graph: &Graph, facts: &[FileFacts], fanout_cap: usize) -> CrossFileOutput {
    // Edges already emitted by `intra_file`, keyed for O(1) lookup.
    //
    // The de-duplication test below used to scan every edge in the graph
    // per resolved reference. That is O(references x edges), which stayed
    // invisible while PHP resolved almost nothing and became ~4s of a
    // Laravel run the moment it started resolving properly.
    let existing_edges: std::collections::HashSet<(u32, u32, u32)> = graph
        .edges
        .iter()
        .map(|e| (e.src.as_u32(), e.dst.as_u32(), e.site_byte))
        .collect();
    let mut out = CrossFileOutput::default();
    let _sp_idx = cgg_core::profile::span("xfile::index-build");
    let resolver_id = ResolverId::new("cross-file:imports");

    // Index callables by (language, qualified_name) and (language, simple_name).
    // Also build a (language, owner_type, method) index (Issue 2) so a
    // method call on a receiver of known type resolves with an O(1)
    // lookup instead of scanning every qualified name.
    let mut by_qn: HashMap<(String, String), CallableId> = HashMap::new();
    let mut by_simple: HashMap<(String, String), Vec<CallableId>> = HashMap::new();
    let mut by_owner_method: HashMap<(String, String, String), Vec<CallableId>> =
        HashMap::new();
    for c in graph.callables.values() {
        by_qn.insert((c.language.clone(), c.qualified_name.clone()), c.id);
        by_simple
            .entry((c.language.clone(), c.simple_name.clone()))
            .or_default()
            .push(c.id);
        if let Some(owner) = owner_from_qn(&c.qualified_name) {
            by_owner_method
                .entry((c.language.clone(), owner.to_string(), c.simple_name.clone()))
                .or_default()
                .push(c.id);
        }
    }

    // Declarations rather than implementations: a `typing.Protocol`
    // member or an `@abstractmethod`. They carry no body worth entering,
    // so counting them as call targets inflates "how many things does
    // this reach" — three implementations where two exist. Dropped from
    // duck-typed fan-out only when a concrete candidate survives, so a
    // call whose *only* visible target is the declaration still resolves
    // rather than vanishing.
    let stub_ids: std::collections::HashSet<CallableId> = {
        let mut protocol_owners: std::collections::HashSet<(&str, &str)> =
            std::collections::HashSet::new();
        for f in facts {
            for d in &f.definitions {
                if let Some(owner) = owner_from_qn(&d.qualified_name)
                    && d.base_types.iter().any(|b| {
                        let bare = b.split(['<', '[']).next().unwrap_or(b).trim();
                        let bare = bare.rsplit(['.', ':']).next().unwrap_or(bare);
                        matches!(bare, "Protocol" | "ABC" | "ABCMeta")
                    })
                {
                    protocol_owners.insert((f.language.as_str(), owner));
                }
            }
        }
        graph
            .callables
            .values()
            .filter(|c| {
                c.attributes.iter().any(|a| a.contains("abstractmethod"))
                    || owner_from_qn(&c.qualified_name).is_some_and(|o| {
                        protocol_owners.contains(&(c.language.as_str(), o))
                    })
            })
            .map(|c| c.id)
            .collect()
    };

    // Signature text per callable, for narrowing duck-typed fan-out by
    // what a candidate can actually accept.
    // Borrowed, not cloned: the graph outlives this pass, and cloning
    // one string per callable cost ~4% on a 20k-callable tree for
    // nothing.
    let signatures: HashMap<CallableId, &str> = graph
        .callables
        .values()
        .filter(|c| c.signature_hint.contains('('))
        .map(|c| (c.id, c.signature_hint.as_str()))
        .collect();

    // Every type cgg has at least one method for. Distinguishes "this
    // class declares no initializer" from "cgg has never seen this name".
    let known_owners: std::collections::HashSet<(String, String)> = by_owner_method
        .keys()
        .map(|(l, o, _)| (l.clone(), o.clone()))
        .collect();

    // Owner type -> its declared bases, for walking the inheritance
    // chain when a method is inherited rather than declared. Recorded on
    // methods rather than types, because cgg's model has no node for a
    // type — any method of the class carries the same base list.
    let mut bases_by_owner: HashMap<(String, String), Vec<String>> = HashMap::new();
    for f in facts {
        for d in &f.definitions {
            if d.base_types.is_empty() {
                continue;
            }
            let Some(owner) = owner_from_qn(&d.qualified_name) else {
                continue;
            };
            let slot = bases_by_owner
                .entry((f.language.clone(), owner.to_string()))
                .or_default();
            for b in &d.base_types {
                // Store the bare type name: the index is keyed that way,
                // and a base is written as `generic.ObjectListView` or
                // `Handler<T>` as often as plainly.
                let bare = b.split(['<', '[']).next().unwrap_or(b).trim();
                let bare = bare.rsplit(['.', ':', '\\']).next().unwrap_or(bare);
                if !bare.is_empty() && !slot.iter().any(|x| x == bare) {
                    slot.push(bare.to_string());
                }
            }
        }
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

    // `(file, start, end) -> callable`, built once. See
    // `enclosing_callable_id`.
    let mut callables_by_span: HashMap<(FileId, u32, u32), CallableId> = HashMap::new();
    for c in graph.callables.values() {
        callables_by_span
            .entry((c.file, c.start_byte, c.end_byte))
            .or_insert(c.id);
    }

    let facts_by_id: HashMap<FileId, &FileFacts> =
        facts.iter().map(|f| (f.file, f)).collect();

    // Paths grouped by language, built once. The scoped-by-simple-name
    // fallback only ever considers files of the *same* language.
    //
    // An inverted `(language, path fragment) -> files` index was tried
    // here and reverted: the match is `path.contains(fragment)`, which
    // permits a fragment to begin *mid-segment*, and no segment-aligned
    // index reproduces that. The corpus caught it as -9,331 nodes and
    // edges across 34 repos. Any future index has to be proven against
    // the full corpus before it replaces this scan.
    let mut files_by_lang: HashMap<&str, Vec<(FileId, String)>> = HashMap::new();
    for f in facts {
        files_by_lang
            .entry(f.language.as_str())
            .or_default()
            .push((f.file, f.path.to_string_lossy().to_ascii_lowercase()));
    }

    // Include resolution used to scan every file in the tree for every
    // `#include`, of every file — O(files x includes x files). `HashMap`
    // iteration is unordered, so even the exact-match short-circuit read
    // half the map on average, and a miss read all of it. On
    // terraform-provider-aws (12,825 files) that was 82% of the whole
    // run: 341s of 415s CPU inside the import-table build alone.
    //
    // Two indexes built once instead. Exact path is the common case and
    // is now O(1); the suffix case keeps its old meaning — lowest FileId
    // among the matches — by bucketing on the last path segment and
    // verifying the full suffix, which is a handful of candidates rather
    // than the corpus.
    let mut include_by_exact: HashMap<&std::path::Path, &FileFacts> = HashMap::new();
    let mut include_by_last: HashMap<&std::ffi::OsStr, Vec<&FileFacts>> = HashMap::new();
    for f in facts {
        include_by_exact.entry(f.path.as_path()).or_insert(f);
        if let Some(name) = f.path.file_name() {
            include_by_last.entry(name).or_default().push(f);
        }
    }
    for v in include_by_last.values_mut() {
        v.sort_by_key(|f| f.file.as_u32());
    }

    // Per-file and independent: the body reads the shared indexes
    // (`by_qn`, `by_simple`, `by_owner_method`, `reexports`) and writes
    // only into its own output. Collecting in parallel and concatenating
    // in input order keeps the edge sequence identical to the serial
    // form, which the determinism test in crates/cgg/tests pins.
    drop(_sp_idx);
    let _sp_loop = cgg_core::profile::span("xfile::parallel-loop");
    let per_file: Vec<CrossFileOutput> = facts
        .par_iter()
        .map(|facts| {
            let _sp_file = cgg_core::profile::span("xfile::per-file");
            let mut out = CrossFileOutput::default();
            let lang = facts.language.clone();

            // Variable -> its inferred type, for resolving a call *on an
            // instance* (`agent("prompt")` → `Agent.__call__`). A name
            // bound to two different types in one file is dropped rather
            // than guessed at: the whole point of this lookup is that the
            // receiver is known.
            let mut var_types: HashMap<String, String> = HashMap::new();
            {
                let mut conflicted: std::collections::HashSet<&str> =
                    std::collections::HashSet::new();
                for lt in &facts.local_types {
                    if conflicted.contains(lt.var_name.as_str()) {
                        continue;
                    }
                    match var_types.get(&lt.var_name) {
                        Some(prev) if prev != &lt.type_name => {
                            conflicted.insert(lt.var_name.as_str());
                            var_types.remove(&lt.var_name);
                        }
                        Some(_) => {}
                        None => {
                            var_types.insert(lt.var_name.clone(), lt.type_name.clone());
                        }
                    }
                }
            }

            // Normalize imports into lookup tables:
            //   imported_simple_name -> candidate qualified_names.
            // Python: `from helpers import greet` -> map "greet" ->
            //   "helpers.greet".
            //   `import helpers as h` -> map "h" -> "helpers" (module prefix).
            // Rust: `use a::b::c;` -> map "c" -> "a::b::c".
            let mut direct_imports: HashMap<String, Vec<String>> = HashMap::new();
            // Scoped to this file: a header is expanded once per
            // translation unit, which is exactly C's own semantics
            // under include guards.
            let mut include_visited: HashMap<FileId, u8> = HashMap::new();
            let _sp_imports = cgg_core::profile::span("xfile::import-table");
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
                    "import"
                        if matches!(
                            lang.as_str(),
                            "python"
                                | "go"
                                | "javascript"
                                | "typescript"
                                | "swift"
                                | "zig"
                                | "r"
                                | "perl"
                        ) =>
                    {
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
                            let last =
                                path.rsplit('/').next().unwrap_or(path).to_string();
                            (last.clone(), last)
                        } else if path.contains('.') {
                            // Python dotted — bind first segment, target is full.
                            let first =
                                path.split('.').next().unwrap_or(path).to_string();
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
                            module_aliases.insert(imp.alias.clone(), full.to_string());
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
                                &include_by_exact,
                                &include_by_last,
                                &mut direct_imports,
                                8,
                                &mut include_visited,
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
                                &include_by_exact,
                                &include_by_last,
                                &mut direct_imports,
                                4,
                                &mut include_visited,
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
                        if req_path.is_empty() {
                            continue;
                        }

                        // 1) Direct file-include resolution.
                        for try_path in [
                            req_path.to_string(),
                            format!("{req_path}.rb"),
                            format!("{req_path}.lua"),
                            format!("{req_path}.clj"),
                            req_path.replace('.', "/") + ".lua",
                            req_path.replace('.', "/") + ".clj",
                        ] {
                            collect_include_defs(
                                &try_path,
                                facts,
                                &include_by_exact,
                                &include_by_last,
                                &mut direct_imports,
                                4,
                                &mut include_visited,
                            );
                        }

                        // 2) Module-alias / unqualified-prefix.
                        if !imp.alias.is_empty() {
                            module_aliases
                                .insert(imp.alias.clone(), req_path.to_string());
                        } else {
                            let last = req_path
                                .rsplit(['.', '/', ':'])
                                .next()
                                .unwrap_or(req_path);
                            if !last.is_empty() {
                                module_aliases
                                    .insert(last.to_string(), req_path.to_string());
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
                        let raw =
                            imp.path.trim().trim_start_matches("//").trim_matches('"');
                        let cleaned = raw.replace(':', "/");
                        for try_path in [cleaned.clone(), format!("{cleaned}.bzl")] {
                            collect_include_defs(
                                &try_path,
                                facts,
                                &include_by_exact,
                                &include_by_last,
                                &mut direct_imports,
                                4,
                                &mut include_visited,
                            );
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
                        let alias = if imp.alias.is_empty() {
                            path
                        } else {
                            imp.alias.as_str()
                        };
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
                        if path.is_empty() {
                            continue;
                        }
                        match lang.as_str() {
                            // Scala / Java-style: `import pkg.{A,B}` or `import pkg.A`.
                            "scala" | "java" | "kotlin" | "groovy" => {
                                if let Some(idx) = path.rfind('.') {
                                    let prefix = &path[..idx];
                                    let suffix = &path[idx + 1..];
                                    let suffix =
                                        suffix.trim_matches(|c| c == '{' || c == '}');
                                    if suffix == "_" || suffix == "*" {
                                        unqualified_prefixes.push(prefix.to_string());
                                    } else {
                                        for name in suffix.split(',') {
                                            let name = name.trim();
                                            if name.is_empty() {
                                                continue;
                                            }
                                            let (src, alias) = match name.split_once("=>")
                                            {
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
                                        module_aliases
                                            .insert(last.to_string(), prefix.to_string());
                                    }
                                }
                            }
                            // Dart / Solidity / Nix: file-relative paths.
                            "dart" | "solidity" | "nix" => {
                                let cleaned = path.trim_matches(|c| {
                                    c == '\'' || c == '"' || c == '<' || c == '>'
                                });
                                for try_path in [
                                    cleaned.to_string(),
                                    format!("{cleaned}.sol"),
                                    format!("{cleaned}.dart"),
                                    format!("{cleaned}.nix"),
                                ] {
                                    collect_include_defs(
                                        &try_path,
                                        facts,
                                        &include_by_exact,
                                        &include_by_last,
                                        &mut direct_imports,
                                        4,
                                        &mut include_visited,
                                    );
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
                            "haskell" | "erlang" | "elixir" | "fsharp" | "ocaml"
                            | "julia" => {
                                unqualified_prefixes.push(path.to_string());
                                if let Some(last) = path.rsplit('.').next() {
                                    module_aliases
                                        .insert(last.to_string(), path.to_string());
                                }
                            }
                            _ => {
                                // Fall back to module-alias on last segment.
                                let last =
                                    path.rsplit(['.', '/', ':']).next().unwrap_or(path);
                                if !last.is_empty() {
                                    module_aliases
                                        .insert(last.to_string(), path.to_string());
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
                        for try_path in [
                            path.to_string(),
                            format!("{path}.psm1"),
                            format!("{path}.ps1"),
                        ] {
                            collect_include_defs(
                                &try_path,
                                facts,
                                &include_by_exact,
                                &include_by_last,
                                &mut direct_imports,
                                4,
                                &mut include_visited,
                            );
                        }
                        unqualified_prefixes.push(path.to_string());
                    }
                    "include" | "add_subdirectory" | "find_package" => {
                        // CMake-style file inclusion (the C/C++ "include" arm
                        // above already claims the kind for those languages).
                        if lang == "cmake"
                            || lang == "verilog"
                            || lang == "vhdl"
                            || lang == "erlang"
                            || lang == "fortran"
                        {
                            let path = imp.path.trim();
                            for try_path in [
                                path.to_string(),
                                format!("{path}.cmake"),
                                format!("{path}.v"),
                                format!("{path}.hrl"),
                                format!("{path}.f90"),
                                format!("{path}.f95"),
                                "CMakeLists.txt".to_string(),
                            ] {
                                collect_include_defs(
                                    &try_path,
                                    facts,
                                    &include_by_exact,
                                    &include_by_last,
                                    &mut direct_imports,
                                    4,
                                    &mut include_visited,
                                );
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
                for target in module_aliases.values() {
                    path_fragments.push(target.replace('.', "/").to_ascii_lowercase());
                }
                path_fragments.retain(|f| !f.is_empty());
                path_fragments.sort();
                path_fragments.dedup();
                let candidates = files_by_lang
                    .get(lang.as_str())
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                for (fid, fpath) in candidates {
                    if !path_fragments
                        .iter()
                        .any(|frag| fpath.contains(frag.as_str()))
                    {
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

            drop(_sp_imports);
            let _sp_refs = cgg_core::profile::span("xfile::ref-loop");
            for r in &facts.references {
                // A string literal that names a callable is never a call.
                // §8 is explicit: string routing may lower confidence, it
                // must not manufacture an edge.
                if r.receiver_hint == cgg_core::STRING_REF_HINT {
                    continue;
                }

                // Compute enclosing callable up front so we can pass its
                // qualified name into the resolver — needed for the
                // intra-crate qualified-path retry (e.g., `crawl::foo()`
                // inside `nkb_research::ResearchRunner::run` should find
                // `nkb_research::crawl::foo`).
                let enclosing =
                    enclosing_callable_id(&callables_by_span, facts, r.site_byte);

                // Value references (`register(handler)`) resolve by name,
                // not through the import tables, and produce a
                // `Via::Reference` edge gated behind `--reference-edges`.
                //
                // Two gaps closed here at once: `intra_file` could only bind
                // a value ref to a callable in the *same file*, so
                // `app.get('/x', handler)` with the handler in another
                // module resolved to nothing; and letting the record fall
                // through to the generic path below tagged it `Via::Direct`,
                // which claims a call site that does not exist and escapes
                // the flag that is supposed to gate it.
                if r.receiver_hint == cgg_core::VALUE_REF_HINT {
                    // A value reference at module scope has no callable to
                    // hang the edge on: `callback=_validate_key` inside a
                    // click decorator, `event.listen(cls, "...", cls._x)`,
                    // a dispatch-table literal. Dropping it silently made
                    // the *target* look never-referenced, and — because
                    // nothing was on record to doubt it — promoted it to
                    // `High`. Measured on flask, httpie, black, flaskbb and
                    // dispatch, that single omission produced 28 of 45
                    // false positives in the top band.
                    //
                    // The reference is still real, so record it as an
                    // unresolved site. The dead-code pass correlates
                    // unresolved sites by name and will refuse to promote
                    // any callable one of them might target. The hint is
                    // cleared because `VALUE_REF_HINT` is plumbing, not a
                    // receiver type, and the correlation reads that field
                    // as a type.
                    let Some(src) = enclosing else {
                        out.unresolved.push(AuditUnresolvedCall::new(
                            None,
                            facts.file,
                            r.site_line,
                            r.site_byte,
                            r.name.clone(),
                            String::new(),
                            UnresolvedReason::NoEnclosingCallable,
                        ));
                        continue;
                    };
                    let Some(cands) = by_simple.get(&(lang.clone(), r.name.clone()))
                    else {
                        continue;
                    };
                    // Ambiguity is dropped rather than guessed: a reference
                    // edge to the wrong `handler` is worse than none. But
                    // dropping it *silently* is what turned flask's two
                    // `_make_timedelta` definitions into two `High`
                    // findings — the reference names one of them, and
                    // refusing to say which is not the same as there being
                    // no reference. Record the site so the name
                    // correlation can still see it.
                    let [cid] = cands.as_slice() else {
                        out.unresolved.push(AuditUnresolvedCall::new(
                            Some(src),
                            facts.file,
                            r.site_line,
                            r.site_byte,
                            r.name.clone(),
                            String::new(),
                            UnresolvedReason::AmbiguousInFile,
                        ));
                        continue;
                    };
                    if *cid == src {
                        continue;
                    }
                    let dup = existing_edges.contains(&(
                        src.as_u32(),
                        cid.as_u32(),
                        r.site_byte,
                    ));
                    if !dup {
                        out.edges.push(CallEdge {
                            src,
                            dst: *cid,
                            site_line: r.site_line,
                            site_byte: r.site_byte,
                            confidence: Confidence::Medium,
                            via: Via::Reference,
                            resolver: resolver_id.clone(),
                        });
                    }
                    continue;
                }
                let caller_qn = enclosing
                    .and_then(|id| graph.callables.get(&id))
                    .map(|c| c.qualified_name.as_str());

                let _sp_ref = cgg_core::profile::span("xfile::resolve-ref");
                let mut capped = 0u32;
                let mut no_ctor = false;
                let super_recv = is_super_receiver(&r.receiver_hint);
                let resolved = try_resolve_ref(
                    &lang,
                    r,
                    &direct_imports,
                    &module_aliases,
                    &unqualified_prefixes,
                    &scoped_simple,
                    &by_qn,
                    &by_simple,
                    &by_owner_method,
                    &reexports,
                    &bases_by_owner,
                    &known_owners,
                    &stub_ids,
                    &signatures,
                    &var_types,
                    caller_qn,
                    fanout_cap,
                    &mut capped,
                    &mut no_ctor,
                )
                .and_then(|cids| {
                    if super_recv {
                        without_own_class(
                            &lang,
                            &r.name,
                            caller_qn,
                            &by_owner_method,
                            cids,
                        )
                    } else {
                        Some(cids)
                    }
                });
                // An unambiguous binding is not a guess. A bare name
                // bound by `from x import y` in this very file, or a
                // module alias resolving to exactly one callable, is as
                // certain as same-file resolution — and while it scored
                // `medium`, a same-file method with a colliding name
                // could outrank the correct target at `high`. Anything
                // with more than one candidate stays `medium`: that is
                // fan-out, and fan-out is a hypothesis.
                let confidence = match &resolved {
                    Some(cids)
                        if cids.len() == 1
                            && (r.receiver_hint.is_empty()
                                && direct_imports.contains_key(&r.name)
                                || !r.receiver_hint.is_empty()
                                    && module_aliases
                                        .contains_key(r.receiver_hint.as_str())) =>
                    {
                        Confidence::High
                    }
                    _ => Confidence::Medium,
                };
                if let Some(cids) = resolved {
                    for cid in cids {
                        // Skip self-edges that coincide with intra-file's
                        // ones (they'd be duplicates with the same resolver).
                        if let Some(src) = enclosing {
                            if src == cid {
                                continue;
                            }
                            // Avoid duplicating intra-file-emitted edges.
                            if existing_edges.contains(&(
                                src.as_u32(),
                                cid.as_u32(),
                                r.site_byte,
                            )) {
                                continue;
                            }
                            out.edges.push(CallEdge {
                                src,
                                dst: cid,
                                site_line: r.site_line,
                                site_byte: r.site_byte,
                                confidence,
                                via: Via::Direct,
                                resolver: resolver_id.clone(),
                            });
                        }
                    }
                } else if capped > 0 {
                    // A drop is never silent. Without this the call site
                    // is indistinguishable from one that calls nothing —
                    // in the field report a method with 24 grep-visible
                    // call sites showed 2 inbound edges and no signal
                    // that 22 were dropped.
                    out.unresolved.push(AuditUnresolvedCall::new(
                        enclosing,
                        facts.file,
                        r.site_line,
                        r.site_byte,
                        r.name.clone(),
                        r.receiver_hint.clone(),
                        UnresolvedReason::FanoutCapExceeded { candidates: capped },
                    ));
                } else if super_recv {
                    // `super()` with every candidate excluded means the
                    // base is not in the analyzed tree. Saying so beats
                    // both a wrong edge and silence.
                    out.unresolved.push(AuditUnresolvedCall::new(
                        enclosing,
                        facts.file,
                        r.site_line,
                        r.site_byte,
                        r.name.clone(),
                        r.receiver_hint.clone(),
                        UnresolvedReason::SuperBaseOutOfGraph,
                    ));
                } else if no_ctor {
                    out.unresolved.push(AuditUnresolvedCall::new(
                        enclosing,
                        facts.file,
                        r.site_line,
                        r.site_byte,
                        r.name.clone(),
                        r.receiver_hint.clone(),
                        UnresolvedReason::ClassWithoutExplicitInit,
                    ));
                } else if let Some(elsewhere) =
                    by_simple.get(&(lang.clone(), r.name.clone()))
                    && !elsewhere.is_empty()
                {
                    // The name *is* in the graph, just not reachable
                    // from here. `no-candidate-in-file` reads as "this
                    // name does not exist", which was being reported for
                    // names cgg had parsed and indexed — in one case
                    // with nine candidates.
                    out.unresolved.push(AuditUnresolvedCall::new(
                        enclosing,
                        facts.file,
                        r.site_line,
                        r.site_byte,
                        r.name.clone(),
                        r.receiver_hint.clone(),
                        UnresolvedReason::CandidatesInOtherFiles {
                            candidates: elsewhere.len() as u32,
                        },
                    ));
                }
            }

            let _ = &facts_by_id;
            out
        })
        .collect();
    for mut o in per_file {
        out.edges.append(&mut o.edges);
        out.unresolved.append(&mut o.unresolved);
    }

    out
}

/// Collect definitions from an included header file and add them as
/// direct imports. Transitively follows `#include` directives in the
/// header up to `depth` levels.
fn collect_include_defs(
    include_path: &str,
    includer_facts: &FileFacts,
    include_by_exact: &HashMap<&std::path::Path, &FileFacts>,
    include_by_last: &HashMap<&std::ffi::OsStr, Vec<&FileFacts>>,
    direct_imports: &mut HashMap<String, Vec<String>>,
    depth: u8,
    // The best remaining depth each header has already been expanded
    // at, for *this* translation unit.
    //
    // Without any memo the walk is exponential: a C include graph is a
    // diamond, so `a.h` reached by four paths was expanded four times
    // and each of its own includes four times again. On Erlang/OTP's
    // `erts/emulator/beam` — 25 includes in `erl_process.h`, depth 8 —
    // that is 25^8 in the limit, recomputed per file. cgg did not finish
    // the directory in an hour; memoized it takes under a second.
    //
    // Why a depth map and not a plain visited set: `depth` counts down,
    // so a header first reached by a *long* path has little budget left
    // and stops early. A plain set would then refuse to re-expand it
    // when a *short* path arrives with budget to spare, silently losing
    // the deeper definitions — and which ones would depend on include
    // order. Re-expanding only on a strictly larger budget keeps the
    // result identical to the exhaustive walk while bounding the work at
    // O(headers x depth).
    //
    // The dedup is also correct on its own terms: the same definition
    // pushed N times inflated `direct_imports`, and step 1d rejects a
    // name with more than three candidates — so a diamond could push a
    // genuinely unique symbol over the cap and stop it resolving at all.
    visited: &mut HashMap<FileId, u8>,
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
    //
    // The pick must be deterministic. A `HashMap`'s iteration order is
    // randomly seeded per process, so taking the first `.find()` match
    // made the `#include` closure — and therefore the emitted edge set
    // — vary between runs whenever more than one file matched the
    // suffix. That is routine in C/C++, where many directories hold
    // their own `common.h`. Prefer the exactly-resolved path, then the
    // lowest FileId: a total order over the candidates.
    // An exact path match is unique, so it can short-circuit; only the
    // ambiguous suffix case needs the full scan to find the lowest
    // FileId. Scanning unconditionally costs ~6% on include-heavy C/C++
    // trees, and the exact match is the common case.
    // Exact first, exactly as before. Then the suffix fallback, over the
    // few files sharing the include's last segment rather than all of
    // them — pre-sorted by FileId, so `find` yields the same lowest-id
    // winner the old scan did.
    let target: Option<&FileFacts> = include_by_exact
        .get(resolved.as_path())
        .copied()
        .or_else(|| {
            let last = std::path::Path::new(include_path).file_name()?;
            include_by_last
                .get(last)?
                .iter()
                .find(|f| f.path.ends_with(include_path))
                .copied()
        });
    let Some(target) = target else { return };
    // Expand a header again only when this path has more budget left
    // than the one that reached it first.
    match visited.get(&target.file) {
        Some(&best) if best >= depth => return,
        _ => {
            visited.insert(target.file, depth);
        }
    }
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
                include_by_exact,
                include_by_last,
                direct_imports,
                depth - 1,
                visited,
            );
        }
    }
}

/// Whether a receiver is Python's `super()` / Ruby's bare `super`.
///
/// It reaches the resolver verbatim, and `super()` starts with a
/// lowercase letter — so the duck-typing step read it as a variable name
/// and fanned out over every callable with that method name, the calling
/// class's own override included.
fn is_super_receiver(rh: &str) -> bool {
    let rh = rh.trim();
    rh == "super" || rh == "super()" || rh.starts_with("super(")
}

/// Drop the calling class's own methods from a `super()` candidate set.
///
/// `super().m()` means *explicitly not* this class's `m`. Resolving to it
/// produced a false edge and, where the subclass method also called the
/// one containing the `super()` call, a phantom cycle that reads as
/// infinite recursion. When the base is outside the analyzed tree this
/// leaves nothing, which is the correct answer — §8: never manufacture
/// an edge.
fn without_own_class(
    lang: &str,
    method: &str,
    caller_qn: Option<&str>,
    by_owner_method: &HashMap<(String, String, String), Vec<CallableId>>,
    cids: Vec<CallableId>,
) -> Option<Vec<CallableId>> {
    let own = caller_qn.and_then(crate::names::owner_from_qn)?;
    let mine =
        by_owner_method.get(&(lang.to_string(), own.to_string(), method.to_string()));
    let Some(mine) = mine else { return Some(cids) };
    let kept: Vec<CallableId> = cids.into_iter().filter(|c| !mine.contains(c)).collect();
    if kept.is_empty() { None } else { Some(kept) }
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
    by_owner_method: &HashMap<(String, String, String), Vec<CallableId>>,
    reexports: &HashMap<(String, String), String>,
    bases_by_owner: &HashMap<(String, String), Vec<String>>,
    known_owners: &std::collections::HashSet<(String, String)>,
    stub_ids: &std::collections::HashSet<CallableId>,
    signatures: &HashMap<CallableId, &str>,
    var_types: &HashMap<String, String>,
    caller_qn: Option<&str>,
    fanout_cap: usize,
    // Set to the candidate count when the fan-out cap rejected a
    // non-empty set, so the caller can record the drop instead of
    // leaving the site looking uncalled.
    capped: &mut u32,
    // Set when the call names a class cgg knows but that declares no
    // initializer, so the caller can say that rather than "no candidate".
    no_ctor: &mut bool,
) -> Option<Vec<CallableId>> {
    // Descriptor / interface-definition languages (Smithy, Protobuf,
    // GraphQL). Their references are shape/message/type names that are
    // effectively unique identifiers within the model, so a global
    // by-simple-name match is both safe and the right resolution — it
    // links references across files in the same namespace/package (and
    // same-file edges already emitted by the intra-file linker are
    // deduplicated by the caller). Bounded to ≤4 candidates to stay
    // conservative if a name genuinely collides.
    if matches!(
        lang,
        "smithy" | "proto" | "graphql" | "openapi" | "asyncapi"
    ) {
        if let Some(cids) = by_simple.get(&(lang.to_string(), r.name.clone()))
            && !cids.is_empty()
            && cids.len() <= 4
        {
            return Some(cids.clone());
        }
        return None;
    }

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
        if let Some(cids) = scoped_simple.get(&r.name)
            && !cids.is_empty()
            && cids.len() <= 8
        {
            return Some(cids.clone());
        }
        // Step 1d: global by-simple fallback. Last resort — only when
        // the file has at least one import and the simple name is
        // unique-ish (≤3 candidates). Skip stdlib-ish names.
        let has_imports = !direct_imports.is_empty()
            || !module_aliases.is_empty()
            || !unqualified_prefixes.is_empty();
        if has_imports {
            let is_stdlib = cgg_core::stdlib::stdlib_names(lang)
                .is_some_and(|s| s.contains(r.name.as_str()));
            if !is_stdlib
                && let Some(cids) = by_simple.get(&(lang.to_string(), r.name.clone()))
                && !cids.is_empty()
                && cids.len() <= 3
            {
                return Some(cids.clone());
            }
        }
    } else {
        // Step 2: attribute call `mod.fn()` where `mod` is aliased.
        // receiver_hint is the full receiver expression (e.g., "mod"
        // or "mod.sub"). Take its first segment to match module alias.
        let first = r.receiver_hint.split(['.', ':']).next().unwrap_or("");
        if let Some(module) = module_aliases.get(first) {
            // Rebuild the full target path. For `mod.fn()` with alias
            // `mod=helpers` -> `helpers.fn`. For `mod.sub.fn()` ->
            // `helpers.sub.fn`.
            let rest = r.receiver_hint.strip_prefix(first).unwrap_or("");
            let qn = format!("{module}{rest}.{}", r.name);
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
            if let Some(cid) = lookup_with_reexports(lang, &direct_dot, by_qn, reexports)
            {
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
            if lang == "rust"
                && let Some(qn) = caller_qn
            {
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
            // If the head segment is imported as something else, rewrite.
            // e.g., `use foo as f; f::bar()` -> receiver=f, name=bar -> foo::bar.
            if let Some(first) = rh.split(['.', ':']).next()
                && let Some(qns) = direct_imports.get(first)
            {
                for base in qns {
                    let rest = rh.strip_prefix(first).unwrap_or("");
                    let rewritten_colon =
                        format!("{base}{}::{}", rest.replace('.', "::"), r.name);
                    if let Some(cid) =
                        lookup_with_reexports(lang, &rewritten_colon, by_qn, reexports)
                    {
                        return Some(vec![cid]);
                    }
                    let rewritten_dot = format!("{base}{rest}.{}", r.name);
                    if let Some(cid) =
                        lookup_with_reexports(lang, &rewritten_dot, by_qn, reexports)
                    {
                        return Some(vec![cid]);
                    }
                }
            }
        }

        // Step 4: Type-qualified method call (Issue 2).
        // When receiver_hint is a type name (e.g. "MermaidFormatter"),
        // look the owning type up directly in the (owner, method) index —
        // O(1) — instead of scanning every qualified name. `type_hints`
        // rewrites a typed local/param receiver to its type name before
        // this runs, so `reg.commit()` with `reg: Registry` arrives here
        // as receiver_hint = "Registry".
        let rh = r.receiver_hint.trim();
        if !rh.is_empty()
            && rh != "self"
            && rh != "Self"
            && rh != "cls"
            && rh.chars().next().is_some_and(|c| c.is_uppercase())
        {
            // The owner key in the index is the *bare* type name. A
            // receiver can arrive as a multi-segment path (`Utils::Platforms`,
            // `a.b.Thing`), so also try its last segment as the owner —
            // this is what the previous suffix-scan matched and must not
            // regress. Additionally canonicalize an aliased receiver type
            // through the file's import map (Issue 7): `use a::b::Engine as
            // Motor` means a receiver typed `Motor` owns whatever `Engine`
            // owns, so try the alias target's bare type name too.
            let mut owners: Vec<&str> = vec![rh];
            let last_seg = rh.rsplit([':', '.']).next().filter(|s| !s.is_empty());
            if let Some(last) = last_seg
                && last != rh
            {
                owners.push(last);
            }
            if let Some(paths) = direct_imports.get(rh) {
                for p in paths {
                    let last = p.rsplit("::").next().unwrap_or(p);
                    if last != rh {
                        owners.push(last);
                    }
                }
            }
            // An inherited method is declared on a base, not on the
            // class the receiver names. Tried after every direct owner
            // match below, so a subclass override always wins.
            for owner in &owners {
                if by_owner_method
                    .get(&(lang.to_string(), (*owner).to_string(), r.name.clone()))
                    .is_none_or(|c| c.is_empty())
                    && let Some(cids) = resolve_via_bases(
                        lang,
                        owner,
                        &r.name,
                        by_owner_method,
                        bases_by_owner,
                    )
                {
                    return Some(cids);
                }
            }
            for owner in owners {
                if let Some(cids) = by_owner_method.get(&(
                    lang.to_string(),
                    owner.to_string(),
                    r.name.clone(),
                )) && !cids.is_empty()
                {
                    // One match is exact; multiple share owner+method —
                    // return all as medium-confidence and let the
                    // caller decide.
                    return Some(cids.clone());
                }
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
            && rh != "self"
            && rh != "Self"
            && rh != "cls"
            && rh.chars().next().is_some_and(|c| c.is_lowercase())
        {
            // Skip if the method name is in the stdlib manifest for this language
            let _sp_fan = cgg_core::profile::span("xfile::fanout");
            let is_stdlib_method = cgg_core::stdlib::stdlib_names(lang)
                .is_some_and(|std| std.contains(r.name.as_str()));
            if !is_stdlib_method
                && let Some(cids) = by_simple.get(&(lang.to_string(), r.name.clone()))
                && !cids.is_empty()
            {
                // Above the cap the fan-out is too speculative to emit.
                // The caller records *that* it was dropped, with the
                // count — silence here reads as "no call at this site",
                // which understates the caller set rather than widening
                // it, and that is the failure mode impact analysis
                // cannot tolerate.
                // Prefer concrete implementations. A Protocol member
                // or an @abstractmethod is a declaration, not a target.
                let concrete: Vec<CallableId> = cids
                    .iter()
                    .copied()
                    .filter(|c| !stub_ids.contains(c))
                    .collect();
                let cids = if concrete.is_empty() {
                    cids.clone()
                } else {
                    concrete
                };
                // Drop candidates whose signature cannot accept this
                // call's keywords. One-sided and evidence-based: only a
                // keyword that is provably not a parameter eliminates a
                // candidate, so narrowing fan-out can never turn a real
                // edge into a missing one.
                let fits: Vec<CallableId> = cids
                    .iter()
                    .copied()
                    .filter(|c| {
                        signatures
                            .get(c)
                            .is_none_or(|sig| signature_accepts(sig, &r.kwargs))
                    })
                    .collect();
                let cids = if fits.is_empty() { cids } else { fits };
                if cids.len() <= fanout_cap {
                    return Some(cids);
                }
                *capped = cids.len() as u32;
                return None;
            }
        }
    }

    // Step 6: instantiation — `Widget(3)` enters `Widget.__init__`.
    // Last, so a function of the same name always wins. When the
    // class declares no initializer, an inherited one still counts.
    if r.receiver_hint.is_empty() {
        for ctor in constructor_names(lang) {
            if let Some(cids) = by_owner_method.get(&(
                lang.to_string(),
                r.name.clone(),
                (*ctor).to_string(),
            )) && !cids.is_empty()
            {
                return Some(cids.clone());
            }
            if let Some(cids) =
                resolve_via_bases(lang, &r.name, ctor, by_owner_method, bases_by_owner)
            {
                return Some(cids);
            }
        }
        // A known class with no initializer of its own: there is no
        // callable to point at, which is a different fact from "cgg
        // has never heard of this name".
        if !constructor_names(lang).is_empty()
            && known_owners.contains(&(lang.to_string(), r.name.clone()))
        {
            *no_ctor = true;
        }

        // Step 7: calling an instance — `agent("prompt")` where
        // `agent` is an object enters `type(agent).__call__`. In the
        // audited service this was the single most load-bearing edge
        // in the system and it was invisible.
        if let Some(ty) = var_types.get(&r.name) {
            for call_op in call_operator_names(lang) {
                if let Some(cids) = by_owner_method.get(&(
                    lang.to_string(),
                    ty.clone(),
                    (*call_op).to_string(),
                )) && !cids.is_empty()
                {
                    return Some(cids.clone());
                }
                if let Some(cids) =
                    resolve_via_bases(lang, ty, call_op, by_owner_method, bases_by_owner)
                {
                    return Some(cids);
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

/// The innermost callable whose byte range contains `byte`.
///
/// `by_span` maps `(file, start_byte, end_byte)` to the callable id and
/// is built once per run. This used to finish with
/// `graph.callables.values().find(...)` — a scan of *every callable in
/// the graph*, per reference. On Zig's compiler that is 572,840
/// references against 344,808 callables, and it was 449s of a 128s
/// wall-clock run (the span nests across 8 threads). Nothing else in
/// the reference loop came close: the actual resolution was 1.8s.
fn enclosing_callable_id(
    by_span: &HashMap<(FileId, u32, u32), CallableId>,
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
    by_span
        .get(&(facts.file, d.start_byte, d.end_byte))
        .copied()
}

#[cfg(test)]
mod tests {
    /// `resolve` at the default fan-out cap.
    fn resolve_default(g: &Graph, f: &[FileFacts]) -> CrossFileOutput {
        resolve(g, f, DEFAULT_FANOUT_CAP)
    }

    use super::*;
    use cgg_core::{
        DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord,
        graph::{CallableKind, CallableNode, FileRecord as GraphFileRecord},
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
            ..Default::default()
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
            synthetic: false,
            trait_impl_target: None,
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
        }
    }

    /// A value reference sitting at module scope — `callback=handler`
    /// in a decorator, `event.listen(cls, "...", cls.hook)` — has no
    /// enclosing callable, so it cannot become an edge. It must still
    /// be recorded: dropping it silently let the *target* reach the
    /// `High` dead-code band with nothing on record to doubt it.
    #[test]
    fn module_scope_value_ref_is_recorded_not_dropped() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "cli.py", "python"));
        g.add_callable(mk_callable(
            0,
            "_validate_key",
            "cli._validate_key",
            0,
            "python",
            (0, 40),
        ));

        // The ref sits at byte 200, outside `_validate_key`'s (0, 40)
        // span and inside no other callable — i.e. module scope.
        let facts = facts_for(
            0,
            "cli.py",
            "python",
            vec![mk_def(
                "_validate_key",
                "cli._validate_key",
                DefVariant::FreeFunction,
                (0, 40),
            )],
            vec![RefRecord {
                name: "_validate_key".into(),
                receiver_hint: cgg_core::VALUE_REF_HINT.into(),
                site_line: 42,
                site_byte: 200,
                ..Default::default()
            }],
            vec![],
        );

        let out = resolve_default(&g, std::slice::from_ref(&facts));

        assert!(
            out.edges.is_empty(),
            "a module-scope ref has no source callable, so it must not \
             manufacture an edge"
        );
        assert_eq!(out.unresolved.len(), 1, "the ref must still be recorded");
        let u = &out.unresolved[0];
        assert_eq!(u.name, "_validate_key");
        assert_eq!(u.reason, UnresolvedReason::NoEnclosingCallable);
        assert!(
            u.receiver_hint.is_empty(),
            "VALUE_REF_HINT is plumbing, not a receiver type — the \
             dead-code correlation reads this field as a type and would \
             reject the site"
        );
    }

    #[test]
    fn python_from_import_direct_call() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "helpers.py", "python"));
        g.add_file(mk_file(1, "main.py", "python"));
        g.add_callable(mk_callable(
            0,
            "greet",
            "helpers.greet",
            0,
            "python",
            (0, 40),
        ));
        g.add_callable(mk_callable(
            1,
            "process",
            "main.process",
            1,
            "python",
            (30, 120),
        ));

        let main_facts = facts_for(
            1,
            "main.py",
            "python",
            vec![mk_def(
                "process",
                "main.process",
                DefVariant::FreeFunction,
                (30, 120),
            )],
            vec![RefRecord {
                name: "greet".into(),
                receiver_hint: "".into(),
                site_line: 5,
                site_byte: 60,
                ..Default::default()
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
            vec![mk_def(
                "greet",
                "helpers.greet",
                DefVariant::FreeFunction,
                (0, 40),
            )],
            vec![],
            vec![],
        );

        let out = resolve_default(&g, &[helpers_facts, main_facts]);
        assert_eq!(out.edges.len(), 1, "expected one cross-file edge");
        assert_eq!(out.edges[0].src, CallableId::new(1));
        assert_eq!(out.edges[0].dst, CallableId::new(0));
        // An unambiguous `from helpers import greet` binding is not a
        // guess — it scored `medium` until 0.6.6, which let a same-file
        // method with a colliding name outrank it at `high`.
        assert_eq!(out.edges[0].confidence, Confidence::High);
        assert_eq!(out.edges[0].resolver.as_str(), "cross-file:imports");
    }

    #[test]
    fn python_module_alias_attribute_call() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "helpers.py", "python"));
        g.add_file(mk_file(1, "main.py", "python"));
        g.add_callable(mk_callable(
            0,
            "compute",
            "helpers.compute",
            0,
            "python",
            (0, 40),
        ));
        g.add_callable(mk_callable(1, "top", "main.top", 1, "python", (30, 120)));

        let main_facts = facts_for(
            1,
            "main.py",
            "python",
            vec![mk_def(
                "top",
                "main.top",
                DefVariant::FreeFunction,
                (30, 120),
            )],
            vec![RefRecord {
                name: "compute".into(),
                receiver_hint: "h".into(),
                site_line: 5,
                site_byte: 60,
                ..Default::default()
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

        let out = resolve_default(&g, &[helpers_facts, main_facts]);
        assert_eq!(out.edges.len(), 1);
        assert_eq!(out.edges[0].dst, CallableId::new(0));
    }

    #[test]
    fn rust_use_direct_call() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "lib.rs", "rust"));
        g.add_file(mk_file(1, "main.rs", "rust"));
        g.add_callable(mk_callable(
            0,
            "helper",
            "crate::util::helper",
            0,
            "rust",
            (0, 40),
        ));
        g.add_callable(mk_callable(1, "main", "crate::main", 1, "rust", (30, 120)));

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
                ..Default::default()
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

        let out = resolve_default(&g, &[lib_facts, main_facts]);
        assert_eq!(out.edges.len(), 1);
        assert_eq!(out.edges[0].dst, CallableId::new(0));
    }
}
