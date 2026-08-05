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

use cgg_core::graph::{CallEdge, Confidence, Graph, Via};
use cgg_core::ids::{CallableId, ResolverId};
use cgg_core::FileFacts;

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
        by_name.entry(c.simple_name.as_str()).or_default().push((c.id, c.language.as_str()));
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

    for f in facts {
        if f.language == "asm" {
            // For each ref in an asm file, locate the enclosing asm
            // label and link the ref to matching C/C++ callables.
            for r in &f.references {
                let candidates = c_family_lookup(&r.name);
                if candidates.is_empty() { continue; }
                let Some(src_id) = enclosing_callable(graph, f, r.site_byte) else { continue };
                for dst in candidates {
                    if dst == src_id { continue; }
                    let dup = graph.edges.iter().any(|e| e.src == src_id && e.dst == dst && e.site_byte == r.site_byte)
                        || out.edges.iter().any(|e| e.src == src_id && e.dst == dst && e.site_byte == r.site_byte);
                    if dup { continue; }
                    out.edges.push(CallEdge {
                        src: src_id, dst,
                        site_line: r.site_line,
                        site_byte: r.site_byte,
                        confidence: Confidence::Medium,
                        via: Via::Ffi("asm-c".into()),
                        resolver: resolver.clone(),
                    });
                }
            }
        } else if matches!(f.language.as_str(), "c" | "cpp" | "objc") {
            // For each ref in a C-family file whose target name (or its
            // `_name` variant) matches an asm label, link C → asm.
            for r in &f.references {
                let stripped = r.name.trim_start_matches('_');
                let names = [r.name.as_str(), stripped];
                let asm_targets: Vec<CallableId> = names.iter()
                    .filter(|n| asm_simple.contains(*n))
                    .flat_map(|n| by_name.get(*n).into_iter().flatten())
                    .filter(|(_, lang)| *lang == "asm")
                    .map(|(cid, _)| *cid)
                    .collect();
                if asm_targets.is_empty() { continue; }
                let Some(src_id) = enclosing_callable(graph, f, r.site_byte) else { continue };
                for dst in asm_targets {
                    if dst == src_id { continue; }
                    let dup = graph.edges.iter().any(|e| e.src == src_id && e.dst == dst && e.site_byte == r.site_byte)
                        || out.edges.iter().any(|e| e.src == src_id && e.dst == dst && e.site_byte == r.site_byte);
                    if dup { continue; }
                    out.edges.push(CallEdge {
                        src: src_id, dst,
                        site_line: r.site_line,
                        site_byte: r.site_byte,
                        confidence: Confidence::Medium,
                        via: Via::Ffi("c-asm".into()),
                        resolver: resolver.clone(),
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
        for &(other_id, other_lang) in candidates {
            if other_lang == c.language.as_str() || other_id == c.id {
                continue;
            }
            // Check if there's already an edge from other_id to c.id.
            let exists = graph.edges.iter().any(|e| e.src == other_id && e.dst == c.id)
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
            });
        }
    }

    out
}

/// Smallest-enclosing-range callable for `(file, byte)`.
fn enclosing_callable(graph: &Graph, f: &FileFacts, byte: u32) -> Option<CallableId> {
    let mut best: Option<(&cgg_core::graph::CallableNode, u32)> = None;
    for c in graph.callables.values() {
        if c.file != f.file { continue; }
        if c.start_byte > byte || c.end_byte < byte { continue; }
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
            start_line: 1, end_line: 3,
            start_byte: 0, end_byte: 50,
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
            start_line: 1, end_line: 2,
            start_byte: 0, end_byte: 30,
            signature_hint: String::new(),
            visibility: String::new(),
            attributes: vec![],
            synthetic: false,
            trait_impl_target: None,
            ..Default::default()
        });
        g
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
        assert_eq!(classify_ffi(&["#[no_mangle]".into()]), Some(("c-abi", Export)));
        assert_eq!(classify_ffi(&["#[pyfunction]".into()]), Some(("pyo3", Export)));
        assert_eq!(classify_ffi(&["extern:C".into()]), Some(("c-abi", Export)));
        // An import is a call *out* of the tree and says nothing about
        // whether anything here is used.
        assert_eq!(classify_ffi(&["[DllImport]".into()]), Some(("pinvoke", Import)));
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

}
