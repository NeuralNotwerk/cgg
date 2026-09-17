//! FFI linker — cross-language edge detection.
//!
//! Scans callable attributes for known FFI markers and emits edges
//! between the foreign-facing callable and any matching call site in
//! another language.
//!
//! Supported families:
//! * `c-abi` — `#[no_mangle]`, `extern "C"`, `__declspec(dllexport)`
//! * `pyo3` — `#[pyfunction]`, `#[pymethods]`, `#[pyclass]`
//! * `wasm-bindgen` — `#[wasm_bindgen]`
//! * `napi` — `#[napi]`, `#[module_exports]`
//! * `jni` — `@JNI`, `native` keyword in Java
//! * `pinvoke` — `[DllImport]` in C#

use std::collections::HashMap;

use cgg_core::FileFacts;
use cgg_core::graph::{CallEdge, CallableKind, CallableNode, Confidence, Graph, Via};
use cgg_core::ids::{CallableId, ResolverId};

use crate::names::owner_from_qn;

#[derive(Debug, Default)]
pub struct FfiOutput {
    pub edges: Vec<CallEdge>,
}

/// Detect FFI boundaries and emit cross-language edges.
pub fn link_ffi(graph: &Graph, facts: &[FileFacts]) -> FfiOutput {
    let mut out = FfiOutput::default();
    let resolver = ResolverId::new("ffi-linker");

    // --- Pass A: asm ↔ C/C++ bridge ---------------------------------
    //
    // Assembly is almost always glued to C. Every `call <name>` site
    // in an asm file that doesn't resolve intra-file should try to
    // link to a C/C++ callable of the same name (or with leading `_`
    // stripped — macOS / MSVC name mangling convention). And every C
    // call to a function that's actually defined in asm should resolve
    // to the asm label. We do both by scanning asm file refs and asm
    // labels in turn.
    let mut by_name: HashMap<&str, Vec<(CallableId, &str)>> = HashMap::new();
    for c in graph.callables.values() {
        by_name
            .entry(c.simple_name.as_str())
            .or_default()
            .push((c.id, c.language.as_str()));
    }
    let asm_simple: std::collections::HashSet<&str> = graph
        .callables
        .values()
        .filter(|c| c.language == "asm")
        .map(|c| c.simple_name.as_str())
        .collect();

    // Helper: candidates with this simple name from C-family languages.
    let c_family_lookup = |name: &str| -> Vec<CallableId> {
        let stripped = name.trim_start_matches('_');
        let mut out: Vec<CallableId> = Vec::new();
        for candidate_name in [name, stripped] {
            if let Some(rows) = by_name.get(candidate_name) {
                for &(cid, lang) in rows {
                    if matches!(lang, "c" | "cpp" | "objc") {
                        out.push(cid);
                    }
                }
            }
        }
        out
    };

    // Every edge already in the graph, plus everything this pass emits.
    // Built once so the duplicate check is a hash probe rather than a
    // scan of the whole edge list per candidate per reference.
    let mut seen_ffi_edges: std::collections::HashSet<(CallableId, CallableId, u32)> =
        graph
            .edges
            .iter()
            .map(|e| (e.src, e.dst, e.site_byte))
            .collect();

    for f in facts {
        if f.language == "asm" {
            // For each ref in an asm file, locate the enclosing asm
            // label and link the ref to matching C/C++ callables.
            for r in &f.references {
                let candidates = c_family_lookup(&r.name);
                if candidates.is_empty() {
                    continue;
                }
                let Some(src_id) = enclosing_callable(graph, f, r.site_byte) else {
                    continue;
                };
                for dst in candidates {
                    if dst == src_id {
                        continue;
                    }
                    // O(references x edges) before this — 24% of the
                    // whole run on Zig's compiler, where almost every
                    // boundary is `extern`.
                    if !seen_ffi_edges.insert((src_id, dst, r.site_byte)) {
                        continue;
                    }
                    out.edges.push(CallEdge {
                        src: src_id,
                        dst,
                        site_line: r.site_line,
                        site_byte: r.site_byte,
                        confidence: Confidence::Medium,
                        via: Via::Ffi("asm-c".into()),
                        resolver: resolver.clone(),
                        weight: 1,
                    });
                }
            }
        } else if matches!(f.language.as_str(), "c" | "cpp" | "objc") {
            // For each ref in a C-family file whose target name (or its
            // `_name` variant) matches an asm label, link C → asm.
            for r in &f.references {
                let stripped = r.name.trim_start_matches('_');
                let names = [r.name.as_str(), stripped];
                let asm_targets: Vec<CallableId> = names
                    .iter()
                    .filter(|n| asm_simple.contains(*n))
                    .flat_map(|n| by_name.get(*n).into_iter().flatten())
                    .filter(|(_, lang)| *lang == "asm")
                    .map(|(cid, _)| *cid)
                    .collect();
                if asm_targets.is_empty() {
                    continue;
                }
                let Some(src_id) = enclosing_callable(graph, f, r.site_byte) else {
                    continue;
                };
                for dst in asm_targets {
                    if dst == src_id {
                        continue;
                    }
                    // O(references x edges) before this — 24% of the
                    // whole run on Zig's compiler, where almost every
                    // boundary is `extern`.
                    if !seen_ffi_edges.insert((src_id, dst, r.site_byte)) {
                        continue;
                    }
                    out.edges.push(CallEdge {
                        src: src_id,
                        dst,
                        site_line: r.site_line,
                        site_byte: r.site_byte,
                        confidence: Confidence::Medium,
                        via: Via::Ffi("c-asm".into()),
                        resolver: resolver.clone(),
                        weight: 1,
                    });
                }
            }
        }
    }
    // --- Pass B: existing attribute-driven FFI ---------------------

    // For each callable with FFI attributes, find matching call sites
    // in other languages.
    for c in graph.callables.values() {
        let family = detect_ffi_family(c);
        if family.is_empty() {
            continue;
        }

        // This callable is exported via FFI. Find callers in other
        // languages that reference the same simple name.
        let Some(candidates) = by_name.get(c.simple_name.as_str()) else {
            continue;
        };

        // Look for unresolved references in other languages that
        // match this callable's simple name. We emit edges from
        // callers in other languages to this FFI-exported callable.
        let own_owner = ffi_owner(c);
        for &(other_id, other_lang) in candidates {
            if other_lang == c.language.as_str() || other_id == c.id {
                continue;
            }
            // The binding technology dictates which language can be on
            // the other side of the boundary: a `napi` export is only
            // ever called from the JS/TS glue `napi` itself generates,
            // a `pyo3` export only from Python, a `c-abi` export only
            // from C/C++/Obj-C. Without this, a Python `.pyi` stub with
            // the same method name as an unrelated napi struct produces
            // a phantom edge (confirmed: 14 of 19 Pass-B edges on
            // cgg-self were exactly this — a `.pyi` stub linked to
            // `cgg_node::Graph::*`).
            if let Some(allowed) = allowed_source_languages(family)
                && !allowed.contains(&other_lang)
            {
                continue;
            }
            // When both the export and the candidate caller are a
            // method/property/constructor/destructor bound to a type,
            // the owning types must match: `Graph.callables` must not
            // bind to `Metrics.callables` just because the method name
            // collides. A free function has no owner and is exempt.
            if let Some(other_c) = graph.callables.get(&other_id)
                && let (Some(a), Some(b)) = (own_owner, ffi_owner(other_c))
                && a != b
            {
                continue;
            }
            // Check if there's already an edge from other_id to c.id.
            let exists = graph
                .edges
                .iter()
                .any(|e| e.src == other_id && e.dst == c.id)
                || out.edges.iter().any(|e| e.src == other_id && e.dst == c.id);
            if exists {
                continue;
            }
            // Emit a cross-language FFI edge: the other-language
            // callable with the same name calls into this FFI export.
            // This is speculative — confidence is Medium.
            out.edges.push(CallEdge {
                src: other_id,
                dst: c.id,
                site_line: 0,
                site_byte: 0,
                confidence: Confidence::Medium,
                via: Via::Ffi(family.to_string()),
                resolver: resolver.clone(),
                weight: 1,
            });
        }
    }

    out
}

/// Smallest-enclosing-range callable for `(file, byte)`.
fn enclosing_callable(graph: &Graph, f: &FileFacts, byte: u32) -> Option<CallableId> {
    let mut best: Option<(&cgg_core::graph::CallableNode, u32)> = None;
    for c in graph.callables.values() {
        if c.file != f.file {
            continue;
        }
        if c.start_byte > byte || c.end_byte < byte {
            continue;
        }
        let span = c.end_byte.saturating_sub(c.start_byte);
        match best {
            Some((_, sp)) if sp <= span => {}
            _ => best = Some((c, span)),
        }
    }
    best.map(|(c, _)| c.id)
}

/// Which side of an FFI boundary a symbol sits on.
///
/// This distinction did not exist before, and it inverts the meaning of
/// the finding: `#[no_mangle] extern "C" fn` is an **export**, called
/// from outside the analyzed tree and therefore unfalsifiably live,
/// whereas `[DllImport]` is an **import**, a call *out* of the tree that
/// says nothing about liveness. Both used to return the same family.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FfiDirection {
    Export,
    Import,
}

/// Classify an FFI symbol by family and direction.
///
/// Compares normalized attribute *keys* for equality rather than
/// substring-matching the raw text: `a.contains("jni")` previously
/// matched any annotation whose text happened to contain those three
/// letters.
pub fn classify_ffi(attrs: &[String]) -> Option<(&'static str, FfiDirection)> {
    use FfiDirection::{Export, Import};
    for attr in attrs {
        let raw = attr.trim();
        let key = raw
            .trim_start_matches("#[")
            .trim_start_matches('[')
            .trim_start_matches('@')
            .trim_end_matches(']');
        let key = key.split('(').next().unwrap_or(key);
        let key = key.split('=').next().unwrap_or(key).trim();
        let lower = key.to_ascii_lowercase();

        let hit = match lower.as_str() {
            "pyfunction" | "pymethods" | "pyclass" => Some(("pyo3", Export)),
            "wasm_bindgen" => Some(("wasm-bindgen", Export)),
            "napi" | "module_exports" => Some(("napi", Export)),
            "no_mangle" | "unsafe(no_mangle)" | "export_name" => Some(("c-abi", Export)),
            "extern:c" => Some(("c-abi", Export)),
            "uniffi::export" => Some(("uniffi", Export)),
            "unmanagedcallersonly" => Some(("c-abi", Export)),
            "jniexport" => Some(("jni", Export)),
            "dllexport" => Some(("c-abi", Export)),
            // Imports: a call leaving the tree.
            "dllimport" => Some(("pinvoke", Import)),
            "native" => Some(("jni", Import)),
            "link" => Some(("c-abi", Import)),
            _ => None,
        };
        if hit.is_some() {
            return hit;
        }
    }
    None
}

fn detect_ffi_family(c: &cgg_core::graph::CallableNode) -> &'static str {
    // Only exports get a speculative cross-language peer edge: an
    // import is a call *out* of the tree and has no in-tree callee.
    match classify_ffi(&c.attributes) {
        Some((family, FfiDirection::Export)) => family,
        _ => "",
    }
}

/// Which source languages can plausibly call an export of this family.
///
/// `None` means the family is not restricted here (unchanged behaviour):
/// only the three families with a concrete, unambiguous binding
/// generator are narrowed, per the verified defect (14 of 19 Pass-B
/// edges on cgg-self were a `.pyi` stub linked to a `napi` export).
fn allowed_source_languages(family: &str) -> Option<&'static [&'static str]> {
    match family {
        "napi" => Some(&["javascript", "typescript"]),
        "pyo3" => Some(&["python"]),
        "c-abi" => Some(&["c", "cpp", "objc"]),
        _ => None,
    }
}

/// The owning type for a method/property/constructor/destructor, or
/// `None` for a free function (which has no owner to disagree about).
///
/// Deliberately narrower than [`owner_from_qn`] alone: that helper
/// returns the enclosing *module* segment even for a free function
/// (`names.rs` documents this — "callers that only want type owners can
/// compare and discard"), which would wrongly reject same-name free
/// functions in differently-named modules/crates across languages.
fn ffi_owner(c: &CallableNode) -> Option<&str> {
    match c.kind {
        CallableKind::Method
        | CallableKind::Constructor
        | CallableKind::Destructor
        | CallableKind::Property => owner_from_qn(&c.qualified_name),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::graph::{CallableKind, CallableNode, FileRecord, Graph};
    use cgg_core::ids::{CallableId, FileId};
    use std::path::PathBuf;

    fn mk_graph() -> Graph {
        let mut g = Graph::new();
        g.add_file(FileRecord {
            id: FileId::new(0),
            path: PathBuf::from("lib.rs"),
            language: "rust".into(),
            detected_via: "ext".into(),
            blake3: "0".repeat(64),
            size_bytes: 10,
            lines: 1,
            parse_ms: 0.0,
            parse_status: "ok".into(),
            ..Default::default()
        });
        g.add_file(FileRecord {
            id: FileId::new(1),
            path: PathBuf::from("app.py"),
            language: "python".into(),
            detected_via: "ext".into(),
            blake3: "0".repeat(64),
            size_bytes: 10,
            lines: 1,
            parse_ms: 0.0,
            parse_status: "ok".into(),
            ..Default::default()
        });
        // Rust FFI export
        g.add_callable(CallableNode {
            id: CallableId::new(0),
            qualified_name: "mylib::add".into(),
            simple_name: "add".into(),
            kind: CallableKind::Function,
            language: "rust".into(),
            file: FileId::new(0),
            start_line: 1,
            end_line: 3,
            start_byte: 0,
            end_byte: 50,
            signature_hint: String::new(),
            visibility: String::new(),
            attributes: vec!["#[pyfunction]".into()],
            synthetic: false,
            trait_impl_target: None,
            ..Default::default()
        });
        // Python caller with same name (it imported the binding)
        g.add_callable(CallableNode {
            id: CallableId::new(1),
            qualified_name: "app.add".into(),
            simple_name: "add".into(),
            kind: CallableKind::Function,
            language: "python".into(),
            file: FileId::new(1),
            start_line: 1,
            end_line: 2,
            start_byte: 0,
            end_byte: 30,
            signature_hint: String::new(),
            visibility: String::new(),
            attributes: vec![],
            synthetic: false,
            trait_impl_target: None,
            ..Default::default()
        });
        g
    }

    /// Every FFI form the README and the `cgg` skill promise to detect,
    /// as `(attribute-as-written, family, direction)`.
    ///
    /// This table is the documented contract — "detect `#[pyfunction]`,
    /// `#[wasm_bindgen]`, `#[napi]`, `@JNI`, `[DllImport]`, `extern "C"`
    /// and link across language boundaries". Only the PyO3 row was
    /// exercised, so a regression in any other family was invisible.
    const DOCUMENTED_FORMS: &[(&str, &str, FfiDirection)] = &[
        ("#[pyfunction]", "pyo3", FfiDirection::Export),
        ("#[pymethods]", "pyo3", FfiDirection::Export),
        ("#[pyclass]", "pyo3", FfiDirection::Export),
        ("#[wasm_bindgen]", "wasm-bindgen", FfiDirection::Export),
        ("#[napi]", "napi", FfiDirection::Export),
        ("#[module_exports]", "napi", FfiDirection::Export),
        ("#[no_mangle]", "c-abi", FfiDirection::Export),
        ("#[export_name = \"x\"]", "c-abi", FfiDirection::Export),
        ("#[uniffi::export]", "uniffi", FfiDirection::Export),
        ("@JNIEXPORT", "jni", FfiDirection::Export),
        ("[UnmanagedCallersOnly]", "c-abi", FfiDirection::Export),
        // Recorded as the bare key; `__declspec(...)` is unwrapped by
        // the plugin, not here.
        ("dllexport", "c-abi", FfiDirection::Export),
        // Imports: a call leaving the tree.
        (
            "[DllImport(\"user32.dll\")]",
            "pinvoke",
            FfiDirection::Import,
        ),
        ("native", "jni", FfiDirection::Import),
        ("#[link(name = \"c\")]", "c-abi", FfiDirection::Import),
    ];

    #[test]
    fn every_documented_ffi_form_classifies() {
        for (attr, family, dir) in DOCUMENTED_FORMS {
            let got = classify_ffi(&[attr.to_string()]);
            let (gf, gd) = got.unwrap_or_else(|| {
                panic!("`{attr}` is documented as detected but classified as nothing")
            });
            assert_eq!(gf, *family, "`{attr}` classified into the wrong family");
            assert_eq!(
                std::mem::discriminant(&gd),
                std::mem::discriminant(dir),
                "`{attr}` classified with the wrong direction"
            );
        }
    }

    #[test]
    fn classification_ignores_attribute_case() {
        // Annotations arrive spelled the way each language spells them;
        // `[DllImport]` and `[dllimport]` are the same thing.
        assert!(classify_ffi(&["[DLLIMPORT]".into()]).is_some());
        assert!(classify_ffi(&["#[No_Mangle]".into()]).is_some());
    }

    #[test]
    fn an_unrelated_attribute_classifies_as_nothing() {
        for attr in [
            "#[derive(Debug)]",
            "@Override",
            "[Serializable]",
            "#[inline]",
        ] {
            assert!(
                classify_ffi(&[attr.to_string()]).is_none(),
                "`{attr}` must not be mistaken for an FFI marker"
            );
        }
        assert!(classify_ffi(&[]).is_none());
    }

    #[test]
    fn only_exports_advertise_a_family_for_peer_edges() {
        // An import is a call *out* of the tree with no in-tree callee,
        // so it must never seed a speculative cross-language edge.
        for (attr, _, dir) in DOCUMENTED_FORMS {
            let mut c = CallableNode {
                id: CallableId::new(0),
                qualified_name: "x".into(),
                simple_name: "x".into(),
                kind: CallableKind::Function,
                language: "rust".into(),
                file: FileId::new(0),
                start_line: 1,
                end_line: 1,
                start_byte: 0,
                end_byte: 1,
                signature_hint: String::new(),
                visibility: String::new(),
                attributes: vec![],
                synthetic: false,
                trait_impl_target: None,
                ..Default::default()
            };
            c.attributes = vec![attr.to_string()];
            let family = detect_ffi_family(&c);
            match dir {
                FfiDirection::Export => {
                    assert!(
                        !family.is_empty(),
                        "`{attr}` is an export and must name a family"
                    )
                }
                FfiDirection::Import => {
                    assert!(
                        family.is_empty(),
                        "`{attr}` is an import and must not seed a peer edge"
                    )
                }
            }
        }
    }

    #[test]
    fn wasm_bindgen_links_rust_to_javascript() {
        // The same shape as the PyO3 case, in a second family, so the
        // linker is exercised beyond one hard-coded key.
        let mut g = mk_graph();
        g.callables.get_mut(&CallableId::new(0)).unwrap().attributes =
            vec!["#[wasm_bindgen]".into()];
        let f = g.files.get_mut(&FileId::new(1)).unwrap();
        f.path = PathBuf::from("app.js");
        f.language = "javascript".into();
        g.callables.get_mut(&CallableId::new(1)).unwrap().language = "javascript".into();

        let out = link_ffi(&g, &[]);
        assert_eq!(out.edges.len(), 1, "one cross-language edge expected");
        assert!(matches!(out.edges[0].via, Via::Ffi(ref fam) if fam == "wasm-bindgen"));
    }

    #[test]
    fn pyo3_cross_language_edge() {
        let g = mk_graph();
        let out = link_ffi(&g, &[]);
        assert_eq!(out.edges.len(), 1);
        assert_eq!(out.edges[0].src, CallableId::new(1));
        assert_eq!(out.edges[0].dst, CallableId::new(0));
        assert!(matches!(out.edges[0].via, Via::Ffi(ref f) if f == "pyo3"));
    }

    #[test]
    fn no_edge_same_language() {
        let mut g = mk_graph();
        // Change python callable to rust — should not emit FFI edge.
        g.callables.get_mut(&CallableId::new(1)).unwrap().language = "rust".into();
        let out = link_ffi(&g, &[]);
        assert!(out.edges.is_empty());
    }

    #[test]
    fn exports_and_imports_are_opposite_directions() {
        use FfiDirection::{Export, Import};
        assert_eq!(
            classify_ffi(&["#[no_mangle]".into()]),
            Some(("c-abi", Export))
        );
        assert_eq!(
            classify_ffi(&["#[pyfunction]".into()]),
            Some(("pyo3", Export))
        );
        assert_eq!(classify_ffi(&["extern:C".into()]), Some(("c-abi", Export)));
        // An import is a call *out* of the tree and says nothing about
        // whether anything here is used.
        assert_eq!(
            classify_ffi(&["[DllImport]".into()]),
            Some(("pinvoke", Import))
        );
        assert_eq!(classify_ffi(&["native".into()]), Some(("jni", Import)));
    }

    #[test]
    fn classification_is_key_exact_not_substring() {
        // `a.contains("jni")` used to match anything containing those
        // three letters.
        assert_eq!(classify_ffi(&["@InjniSomething".into()]), None);
        assert_eq!(classify_ffi(&["#[derive(Debug)]".into()]), None);
        assert_eq!(classify_ffi(&[]), None);
    }

    #[test]
    fn only_exports_get_a_speculative_peer_edge() {
        let mut n = CallableNode {
            id: CallableId::new(0),
            qualified_name: "x".into(),
            simple_name: "x".into(),
            language: "rust".into(),
            ..Default::default()
        };
        n.attributes = vec!["[DllImport]".into()];
        assert_eq!(detect_ffi_family(&n), "", "an import has no in-tree callee");
        n.attributes = vec!["#[no_mangle]".into()];
        assert_eq!(detect_ffi_family(&n), "c-abi");
    }

    /// A Python `.pyi` stub with the same method name AND the same
    /// owning-type name as a `napi`-exported Rust method must not be
    /// linked: `napi` glue is JS/TS, never Python, and a Python stub
    /// with a same-named class is (at best) mirroring an unrelated
    /// `pyo3` binding, not calling into this one.
    ///
    /// This is the confirmed defect (VERIFIED §1j / SPECS c7): before
    /// the language-family + owner checks, Pass B linked every
    /// other-language callable sharing a simple name, so this exact
    /// shape (owners equal, languages incompatible) produced a false
    /// edge.
    #[test]
    fn napi_export_does_not_link_a_python_stub_of_the_same_name() {
        let mut g = Graph::new();
        g.add_file(FileRecord {
            id: FileId::new(0),
            path: PathBuf::from("graph.rs"),
            language: "rust".into(),
            detected_via: "ext".into(),
            blake3: "0".repeat(64),
            size_bytes: 10,
            lines: 1,
            parse_ms: 0.0,
            parse_status: "ok".into(),
            ..Default::default()
        });
        g.add_file(FileRecord {
            id: FileId::new(1),
            path: PathBuf::from("cgg_node.pyi"),
            language: "python".into(),
            detected_via: "ext".into(),
            blake3: "0".repeat(64),
            size_bytes: 10,
            lines: 1,
            parse_ms: 0.0,
            parse_status: "ok".into(),
            ..Default::default()
        });
        // napi-exported Rust method: Graph::callables
        g.add_callable(CallableNode {
            id: CallableId::new(0),
            qualified_name: "cgg_node::Graph::callables".into(),
            simple_name: "callables".into(),
            kind: CallableKind::Method,
            language: "rust".into(),
            file: FileId::new(0),
            start_line: 1,
            end_line: 3,
            start_byte: 0,
            end_byte: 50,
            signature_hint: String::new(),
            visibility: String::new(),
            attributes: vec!["#[napi]".into()],
            synthetic: false,
            trait_impl_target: None,
            ..Default::default()
        });
        // Python stub with the SAME owner name ("Graph") and method
        // name — the case the owner check alone would let through.
        g.add_callable(CallableNode {
            id: CallableId::new(1),
            qualified_name: "cgg._cgg.Graph.callables".into(),
            simple_name: "callables".into(),
            kind: CallableKind::Method,
            language: "python".into(),
            file: FileId::new(1),
            start_line: 1,
            end_line: 2,
            start_byte: 0,
            end_byte: 30,
            signature_hint: String::new(),
            visibility: String::new(),
            attributes: vec![],
            synthetic: false,
            trait_impl_target: None,
            ..Default::default()
        });

        let out = link_ffi(&g, &[]);
        assert!(
            out.edges.is_empty(),
            "a napi export must not link a Python stub, even with a matching owner name: {:?}",
            out.edges
        );
    }

    /// A `pyo3`-exported free function must still link to a Python
    /// caller of the same name — the language-family and owner checks
    /// must not reject the case they exist to keep working.
    #[test]
    fn pyo3_export_links_a_python_caller_of_the_same_name() {
        let mut g = Graph::new();
        g.add_file(FileRecord {
            id: FileId::new(0),
            path: PathBuf::from("lib.rs"),
            language: "rust".into(),
            detected_via: "ext".into(),
            blake3: "0".repeat(64),
            size_bytes: 10,
            lines: 1,
            parse_ms: 0.0,
            parse_status: "ok".into(),
            ..Default::default()
        });
        g.add_file(FileRecord {
            id: FileId::new(1),
            path: PathBuf::from("app.py"),
            language: "python".into(),
            detected_via: "ext".into(),
            blake3: "0".repeat(64),
            size_bytes: 10,
            lines: 1,
            parse_ms: 0.0,
            parse_status: "ok".into(),
            ..Default::default()
        });
        g.add_callable(CallableNode {
            id: CallableId::new(0),
            qualified_name: "mylib::analyze".into(),
            simple_name: "analyze".into(),
            kind: CallableKind::Function,
            language: "rust".into(),
            file: FileId::new(0),
            start_line: 1,
            end_line: 3,
            start_byte: 0,
            end_byte: 50,
            signature_hint: String::new(),
            visibility: String::new(),
            attributes: vec!["#[pyfunction]".into()],
            synthetic: false,
            trait_impl_target: None,
            ..Default::default()
        });
        g.add_callable(CallableNode {
            id: CallableId::new(1),
            qualified_name: "app.analyze".into(),
            simple_name: "analyze".into(),
            kind: CallableKind::Function,
            language: "python".into(),
            file: FileId::new(1),
            start_line: 1,
            end_line: 2,
            start_byte: 0,
            end_byte: 30,
            signature_hint: String::new(),
            visibility: String::new(),
            attributes: vec![],
            synthetic: false,
            trait_impl_target: None,
            ..Default::default()
        });

        let out = link_ffi(&g, &[]);
        assert_eq!(
            out.edges.len(),
            1,
            "a python caller of a pyo3 export must still link"
        );
        assert_eq!(out.edges[0].src, CallableId::new(1));
        assert_eq!(out.edges[0].dst, CallableId::new(0));
    }
}
