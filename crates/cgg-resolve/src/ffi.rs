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

#[derive(Debug, Default)]
pub struct FfiOutput {
    pub edges: Vec<CallEdge>,
}

/// Detect FFI boundaries and emit cross-language edges.
pub fn link_ffi(graph: &Graph) -> FfiOutput {
    let mut out = FfiOutput::default();
    let resolver = ResolverId::new("ffi-linker");

    // Index: simple_name -> Vec<(CallableId, language)>
    let mut by_name: HashMap<&str, Vec<(CallableId, &str)>> = HashMap::new();
    for c in graph.callables.values() {
        by_name
            .entry(c.simple_name.as_str())
            .or_default()
            .push((c.id, c.language.as_str()));
    }

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

        // Find refs (edges) targeting this name from other languages.
        for edge in &graph.edges {
            if edge.dst != c.id {
                continue;
            }
            let Some(src_node) = graph.callables.get(&edge.src) else {
                continue;
            };
            if src_node.language != c.language {
                // Already a cross-language edge — tag it as FFI.
                // (We don't duplicate; the edge already exists.)
                continue;
            }
        }

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

fn detect_ffi_family(c: &cgg_core::graph::CallableNode) -> &'static str {
    for attr in &c.attributes {
        let a = attr.to_lowercase();
        // PyO3
        if a.contains("pyfunction") || a.contains("pymethods") || a.contains("pyclass") {
            return "pyo3";
        }
        // wasm-bindgen
        if a.contains("wasm_bindgen") {
            return "wasm-bindgen";
        }
        // napi-rs
        if a.contains("napi") || a.contains("module_exports") {
            return "napi";
        }
        // JNI
        if a.contains("jni") || a == "native" {
            return "jni";
        }
        // C# P/Invoke
        if a.contains("dllimport") {
            return "pinvoke";
        }
        // C ABI
        if a.contains("no_mangle") || a.contains("extern \"c\"") || a.contains("dllexport") {
            return "c-abi";
        }
    }
    // Check visibility for extern "C" in Rust (captured as attribute).
    ""
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
            sha256: "0".repeat(64),
            size_bytes: 10,
            lines: 1,
            parse_ms: 0.0,
            parse_status: "ok".into(),
        });
        g.add_file(FileRecord {
            id: FileId::new(1),
            path: PathBuf::from("app.py"),
            language: "python".into(),
            detected_via: "ext".into(),
            sha256: "0".repeat(64),
            size_bytes: 10,
            lines: 1,
            parse_ms: 0.0,
            parse_status: "ok".into(),
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
        });
        g
    }

    #[test]
    fn pyo3_cross_language_edge() {
        let g = mk_graph();
        let out = link_ffi(&g);
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
        let out = link_ffi(&g);
        assert!(out.edges.is_empty());
    }
}
