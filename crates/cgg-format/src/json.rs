//! JSON formatter — serializes the full graph as a JSON document.

use crate::{GraphFormatter, OutputFormat};
use cgg_core::Graph;
use serde::Serialize;
use std::io;

/// The document schema `-t json` writes, and `--from-graph` reads back.
///
/// Bumped only when an existing field changes meaning or disappears.
/// Adding a field does not bump it: every field on `Graph` is
/// `skip_serializing_if`-guarded or plainly additive, and the reader
/// ignores keys it does not know.
pub const GRAPH_SCHEMA: &str = "cgg.graph.v1";

/// `Graph`, plus the two keys that make it safe to read back later.
///
/// A wrapper rather than fields on `Graph` itself, because they are
/// facts about the *document*, not about the graph: an in-memory
/// `Graph` has no version, and adding one would put a string on every
/// node-carrying structure in the library, in the FFI, and in the Python
/// and Node bindings to serve one file format.
///
/// This exists because `-t json` output was already a valid input to
/// `serde_json::from_str::<Graph>` — it just had no way to say which
/// cgg wrote it. Ids are explicitly not comparable across versions, so a
/// document replayed by a different binary can be silently wrong in
/// exactly the way that is hardest to notice.
#[derive(Serialize)]
struct Document<'a> {
    schema: &'static str,
    cgg_version: &'static str,
    #[serde(flatten)]
    graph: &'a Graph,
}

#[derive(Debug, Default)]
pub struct JsonFormatter;

impl JsonFormatter {
    pub fn new() -> Self {
        Self
    }
}

impl GraphFormatter for JsonFormatter {
    fn format(&self) -> OutputFormat {
        OutputFormat::Json
    }

    fn render(&self, graph: &Graph, out: &mut dyn io::Write) -> io::Result<()> {
        let doc = Document {
            schema: GRAPH_SCHEMA,
            cgg_version: cgg_core::version::CGG_VERSION,
            graph,
        };
        serde_json::to_writer_pretty(out, &doc).map_err(io::Error::other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::graph::{CallableKind, CallableNode, FileRecord, Graph};
    use cgg_core::ids::{CallableId, FileId};
    use std::path::PathBuf;

    #[test]
    fn renders_valid_json() {
        let mut g = Graph::new();
        g.add_file(FileRecord {
            id: FileId::new(0),
            path: PathBuf::from("a.rs"),
            language: "rust".into(),
            detected_via: "ext".into(),
            blake3: "0".repeat(64),
            size_bytes: 10,
            lines: 1,
            parse_ms: 0.1,
            parse_status: "ok".into(),
            ..Default::default()
        });
        g.add_callable(CallableNode {
            id: CallableId::new(0),
            qualified_name: "foo".into(),
            simple_name: "foo".into(),
            kind: CallableKind::Function,
            language: "rust".into(),
            file: FileId::new(0),
            start_line: 1,
            end_line: 1,
            start_byte: 0,
            end_byte: 10,
            signature_hint: String::new(),
            visibility: String::new(),
            attributes: vec![],
            synthetic: false,
            trait_impl_target: None,
            ..Default::default()
        });
        let mut buf = Vec::new();
        JsonFormatter.render(&g, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert!(v["callables"].is_object());
        assert_eq!(v["schema"], GRAPH_SCHEMA);
        assert_eq!(v["cgg_version"], cgg_core::version::CGG_VERSION);
    }

    #[test]
    fn the_document_reads_back_as_a_graph() {
        // The contract `--from-graph` rests on: what `-t json` writes is
        // what `serde_json::from_str::<Graph>` accepts, schema keys and
        // all. `Graph` has no `deny_unknown_fields`, which is what lets
        // the two wrapper keys ride along without a second type.
        let mut g = Graph::new();
        g.add_file(FileRecord {
            id: FileId::new(0),
            path: PathBuf::from("a.rs"),
            language: "rust".into(),
            detected_via: "ext".into(),
            blake3: "0".repeat(64),
            size_bytes: 10,
            lines: 1,
            parse_ms: 0.1,
            parse_status: "ok".into(),
            ..Default::default()
        });
        g.add_callable(CallableNode {
            id: CallableId::new(7),
            qualified_name: "foo".into(),
            simple_name: "foo".into(),
            kind: CallableKind::Function,
            language: "rust".into(),
            file: FileId::new(0),
            ..Default::default()
        });
        let mut buf = Vec::new();
        JsonFormatter.render(&g, &mut buf).unwrap();
        let back: Graph = serde_json::from_slice(&buf).expect("round-trips");
        assert_eq!(back.callables.len(), 1);
        assert_eq!(back.callables[&CallableId::new(7)].qualified_name, "foo");
    }
}
