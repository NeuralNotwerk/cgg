//! JSON formatter — serializes the full graph as a JSON document.

use crate::{GraphFormatter, OutputFormat};
use cgg_core::Graph;
use std::io;

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
        serde_json::to_writer_pretty(out, graph).map_err(io::Error::other)
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
    }
}
