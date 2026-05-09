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

        for imp in &facts.imports {
            match imp.kind.as_str() {
                "from-import" => {
                    // Python: imp.path is module; imp.alias is the
                    // items list ("greet, compute" or "greet as g").
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
                    }
                }
                "import" => {
                    // Python: `import a.b.c` or `import a as b`.
                    let path = imp.path.trim();
                    if !imp.alias.is_empty() {
                        module_aliases.insert(imp.alias.clone(), path.to_string());
                    } else {
                        // Last segment is the binding name.
                        if let Some(last) = path.split('.').next_back() {
                            module_aliases.insert(last.to_string(), path.to_string());
                        }
                    }
                }
                "use" | "pub-use" => {
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
                _ => {}
            }
        }

        for r in &facts.references {
            // Only try to resolve direct name refs we haven't already
            // emitted — attribute `a.b.c()` receivers are handled by
            // module_aliases when the receiver root is aliased.
            if let Some(cids) = try_resolve_ref(
                &lang,
                r,
                &direct_imports,
                &module_aliases,
                &by_qn,
                &by_simple,
                &reexports,
            ) {
                let enclosing = enclosing_callable_id(graph, facts, r.site_byte);
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

fn try_resolve_ref(
    lang: &str,
    r: &cgg_core::RefRecord,
    direct_imports: &HashMap<String, Vec<String>>,
    module_aliases: &HashMap<String, String>,
    by_qn: &HashMap<(String, String), CallableId>,
    _by_simple: &HashMap<(String, String), Vec<CallableId>>,
    reexports: &HashMap<(String, String), String>,
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
        }

        // Step 3: qualified-path call `foo::bar::baz()`. The
        // receiver_hint is already the dotted/colon-joined path. Try
        // it directly. If the leading segment is an import alias for
        // a longer path, also try that rewrite.
        let rh = r.receiver_hint.trim();
        if !rh.is_empty() {
            // Direct path: `receiver::name`.
            let direct = format!("{rh}::{}", r.name);
            if let Some(cid) = lookup_with_reexports(lang, &direct, by_qn, reexports) {
                return Some(vec![cid]);
            }
            // If the head segment is imported as something else, rewrite.
            // e.g., `use foo as f; f::bar()` -> receiver=f, name=bar -> foo::bar.
            if let Some(first) = rh.split("::").next() {
                if let Some(qns) = direct_imports.get(first) {
                    for base in qns {
                        let rewritten = format!(
                            "{base}{}::{}",
                            rh.strip_prefix(first).unwrap_or(""),
                            r.name
                        );
                        if let Some(cid) =
                            lookup_with_reexports(lang, &rewritten, by_qn, reexports)
                        {
                            return Some(vec![cid]);
                        }
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
            sha256: "0".repeat(64),
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
