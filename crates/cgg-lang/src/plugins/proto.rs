//! Protocol Buffers plugin — message/service topology extraction.
//!
//! Maps a `.proto` descriptor onto cgg's callable/edge model:
//!
//! * `message`, `enum`, and `service` declarations become definitions,
//!   qualified by `package` as `package.Name`.
//! * Field types that name another message/enum become references
//!   (`message` → field-type message), so the message dependency graph
//!   shows up. Scalar field types (`string`, `int32`, …) are parsed as a
//!   different node and never produce references.
//! * `rpc` request/response types become references from the enclosing
//!   `service` (`service` → request/response message), giving the gRPC
//!   surface as service→message edges.
//!
//! Reference targets are the grammar's `message_or_enum_type` nodes, which
//! are emitted only for non-scalar types — so no primitive filtering is
//! needed.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct ProtoPlugin;

impl LanguagePlugin for ProtoPlugin {
    fn id(&self) -> &'static str {
        "proto"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".proto"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &[]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_proto::LANGUAGE.into()
    }

    fn extract(
        &self,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "proto");
        let package = find_package(tree.root_node(), source);
        let mut w = ProtoWalker {
            source,
            package,
            facts: &mut facts,
        };
        w.walk(tree.root_node());
        facts
    }
}

fn find_package(root: Node, source: &[u8]) -> String {
    let mut c = root.walk();
    for child in root.children(&mut c) {
        if child.kind() == "package" {
            let mut cc = child.walk();
            for g in child.children(&mut cc) {
                if g.kind() == "full_ident" {
                    return g.utf8_text(source).unwrap_or("").to_string();
                }
            }
        }
    }
    String::new()
}

struct ProtoWalker<'a> {
    source: &'a [u8],
    package: String,
    facts: &'a mut FileFacts,
}

impl<'a> ProtoWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "message" => {
                self.record(node, "message_name", "message");
                return;
            }
            "enum" => {
                self.record(node, "enum_name", "enum");
                return;
            }
            "service" => {
                self.record(node, "service_name", "service");
                // Each `rpc` is a callable in its own right — it is the
                // thing a server implements and a client calls, and the
                // unit any cross-language link has to name. Recording
                // only the service leaves nothing to bind an
                // implementation to.
                let svc = self.name_of(node, "service_name");
                self.record_rpcs(node, &svc);
                return;
            }
            _ => {}
        }
        let mut c = node.walk();
        if c.goto_first_child() {
            loop {
                self.walk(c.node());
                if !c.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    /// Text of a named child, e.g. the `service_name` of a `service`.
    fn name_of(&self, node: Node, name_kind: &str) -> String {
        let mut c = node.walk();
        node.children(&mut c)
            .find(|n| n.kind() == name_kind)
            .map(|n| self.text(n).trim().to_string())
            .unwrap_or_default()
    }

    /// Record every `rpc` inside a service as `Service.Rpc`.
    fn record_rpcs(&mut self, node: Node, service: &str) {
        if node.kind() == "rpc" {
            let name = self.name_of(node, "rpc_name");
            if !name.is_empty() && !service.is_empty() {
                let owner = if self.package.is_empty() {
                    service.to_string()
                } else {
                    format!("{}.{}", self.package, service)
                };
                self.facts.definitions.push(DefRecord {
                    simple_name: name.clone(),
                    qualified_name: format!("{owner}.{name}"),
                    variant: DefVariant::InherentMethod,
                    start_line: (node.start_position().row as u32) + 1,
                    end_line: (node.end_position().row as u32) + 1,
                    start_byte: node.start_byte() as u32,
                    end_byte: node.end_byte() as u32,
                    signature_hint: format!("rpc {name}"),
                    visibility: String::new(),
                    attributes: Vec::new(),
                    ..Default::default()
                });
            }
            return;
        }
        let mut c = node.walk();
        if c.goto_first_child() {
            loop {
                self.record_rpcs(c.node(), service);
                if !c.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    fn record(&mut self, node: Node, name_kind: &str, keyword: &str) {
        // The name node (e.g. `message_name`) wraps an identifier.
        let mut c = node.walk();
        let name_node = node.children(&mut c).find(|n| n.kind() == name_kind);
        let Some(name_node) = name_node else { return };
        let name = self.text(name_node).trim().to_string();
        if name.is_empty() {
            return;
        }

        let qn = if self.package.is_empty() {
            name.clone()
        } else {
            format!("{}.{}", self.package, name)
        };
        let (sl, el) = (
            (node.start_position().row as u32) + 1,
            (node.end_position().row as u32) + 1,
        );
        self.facts.definitions.push(DefRecord {
            simple_name: name.clone(),
            qualified_name: qn,
            variant: DefVariant::FreeFunction,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: format!("{keyword} {name}"),
            visibility: String::new(),
            attributes: Vec::new(),
            ..Default::default()
        });

        // Collect every non-scalar type reference inside the body
        // (field types for messages; rpc request/response for services).
        self.collect_refs(node);
    }

    fn collect_refs(&mut self, node: Node) {
        if node.kind() == "message_or_enum_type" {
            // Reference to another message/enum; may be dotted (pkg.Name).
            let raw = self.text(node);
            let name = raw.rsplit('.').next().unwrap_or(raw).trim();
            if !name.is_empty() {
                self.facts.references.push(RefRecord {
                    name: name.to_string(),
                    receiver_hint: String::new(),
                    site_line: (node.start_position().row as u32) + 1,
                    site_byte: node.start_byte() as u32,
                    ..Default::default()
                });
            }
            return;
        }
        let mut c = node.walk();
        if c.goto_first_child() {
            loop {
                self.collect_refs(c.node());
                if !c.goto_next_sibling() {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_proto::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        ProtoPlugin.extract(
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/a.proto"),
            &tree,
            src.as_bytes(),
        )
    }

    const SVC: &str = r#"syntax = "proto3";
package foo.bar;
message GetReq { string id = 1; Nested n = 2; repeated Item items = 3; }
message Nested { int32 x = 1; }
message Item { string sku = 1; }
enum Color { RED = 0; }
service Svc {
  rpc DoIt(GetReq) returns (Nested);
  rpc Stream(GetReq) returns (stream Item);
}
"#;

    #[test]
    fn plugin_loads() {
        assert_eq!(ProtoPlugin.id(), "proto");
        assert!(ProtoPlugin.extensions().contains(&".proto"));
    }

    #[test]
    fn extracts_package_qualified_defs() {
        let f = extract(SVC);
        let qns: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(qns.contains(&"foo.bar.GetReq"));
        assert!(qns.contains(&"foo.bar.Svc"));
        assert!(qns.contains(&"foo.bar.Color"), "enum def, got {qns:?}");
    }

    #[test]
    fn message_field_and_rpc_references() {
        let f = extract(SVC);
        let refs: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        assert!(refs.contains(&"Nested"), "field type ref, got {refs:?}");
        assert!(refs.contains(&"Item"), "repeated field + rpc return ref");
        assert!(refs.contains(&"GetReq"), "rpc request ref");
        // scalar field types must not appear
        assert!(!refs.contains(&"string"));
        assert!(!refs.contains(&"int32"));
    }
}
