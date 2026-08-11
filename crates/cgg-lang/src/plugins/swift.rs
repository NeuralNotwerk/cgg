//! Swift plugin — callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct SwiftPlugin;

impl LanguagePlugin for SwiftPlugin {
    fn id(&self) -> &'static str {
        "swift"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".swift"]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_swift::LANGUAGE.into()
    }

    fn extract(
        &self,
        _ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "swift");
        let mut w = SwiftWalker {
            source,
            facts: &mut facts,
            scope: Vec::new(),
        };
        w.walk(tree.root_node());
        facts
    }
}

struct SwiftWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
}

impl<'a> SwiftWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }
    fn qn(&self, simple: &str) -> String {
        if self.scope.is_empty() {
            simple.to_string()
        } else {
            format!("{}.{simple}", self.scope.join("."))
        }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "class_declaration"
            | "struct_declaration"
            | "enum_declaration"
            | "protocol_declaration" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    self.scope.push(name);
                    self.walk_children(node);
                    self.scope.pop();
                } else {
                    self.walk_children(node);
                }
                return;
            }
            "function_declaration" => {
                self.record_function(node);
                self.walk_children(node);
                return;
            }
            "init_declaration" => {
                let qn = self.qn("init");
                let (sl, el) = (
                    (node.start_position().row as u32) + 1,
                    (node.end_position().row as u32) + 1,
                );
                self.facts.definitions.push(DefRecord {
                    simple_name: "init".into(),
                    qualified_name: qn,
                    variant: DefVariant::Constructor,
                    start_line: sl,
                    end_line: el,
                    start_byte: node.start_byte() as u32,
                    end_byte: node.end_byte() as u32,
                    signature_hint: super::extract_signature(self.text(node)),
                    visibility: String::new(),
                    attributes: Vec::new(),
                    ..Default::default()
                });
                self.walk_children(node);
                return;
            }
            "import_declaration" => {
                self.record_import(node);
                return;
            }
            "call_expression" => {
                self.record_call(node);
                self.walk_children(node);
                return;
            }
            _ => {}
        }
        self.walk_children(node);
    }

    fn walk_children(&mut self, node: Node) {
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

    fn record_function(&mut self, node: Node) {
        let name = node
            .child_by_field_name("name")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if name.is_empty() {
            return;
        }
        let is_static = node
            .children(&mut node.walk())
            .any(|c| c.kind() == "modifiers" && self.text(c).contains("static"));
        let variant = if is_static {
            DefVariant::StaticMethod
        } else if self.scope.is_empty() {
            DefVariant::FreeFunction
        } else {
            DefVariant::InherentMethod
        };
        let qn = self.qn(&name);
        let (sl, el) = (
            (node.start_position().row as u32) + 1,
            (node.end_position().row as u32) + 1,
        );
        self.facts.definitions.push(DefRecord {
            simple_name: name,
            qualified_name: qn,
            variant,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            attributes: Vec::new(),
            ..Default::default()
        });
    }

    fn record_import(&mut self, node: Node) {
        let path = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "identifier")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if path.is_empty() {
            return;
        }
        self.facts.imports.push(ImportRecord {
            kind: "import".into(),
            path,
            alias: String::new(),
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
        });
    }

    fn record_call(&mut self, node: Node) {
        // call_expression -> first child is callee (simple_identifier or navigation_expression)
        let callee = node.child(0);
        let Some(callee) = callee else { return };
        let (name, recv) = match callee.kind() {
            "simple_identifier" => (self.text(callee).to_string(), String::new()),
            "navigation_expression" => {
                let target = callee
                    .child_by_field_name("target")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                let suffix = callee
                    .children(&mut callee.walk())
                    .find(|c| c.kind() == "navigation_suffix")
                    .and_then(|s| {
                        s.children(&mut s.walk())
                            .find(|c| c.kind() == "simple_identifier")
                    })
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                (suffix, target)
            }
            _ => return,
        };
        if name.is_empty() {
            return;
        }
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: recv,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            ..Default::default()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_swift::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        SwiftPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.swift"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn class_methods() {
        let src = "class Service {\n  init(name: String) {}\n  func run() {}\n  static func create() -> Service { return Service(name: \"\") }\n}\n";
        let f = extract(src);
        let qns: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(qns.contains(&"Service.init"), "got: {qns:?}");
        assert!(qns.contains(&"Service.run"), "got: {qns:?}");
        assert!(qns.contains(&"Service.create"), "got: {qns:?}");
    }

    #[test]
    fn free_function() {
        let src = "func greet(_ name: String) { print(name) }\n";
        let f = extract(src);
        assert!(
            f.definitions.iter().any(
                |d| d.simple_name == "greet" && d.variant == DefVariant::FreeFunction
            )
        );
    }

    #[test]
    fn import_captured() {
        let src = "import Foundation\nfunc f() {}\n";
        let f = extract(src);
        assert!(f.imports.iter().any(|i| i.path == "Foundation"));
    }

    #[test]
    fn call_expressions() {
        let src = "func f() { greet(\"x\"); obj.run() }\n";
        let f = extract(src);
        assert!(
            f.references.iter().any(|r| r.name == "greet"),
            "refs: {:?}",
            f.references
        );
    }
}
