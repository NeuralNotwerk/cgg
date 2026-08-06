//! Objective-C plugin — callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct ObjcPlugin;

impl LanguagePlugin for ObjcPlugin {
    fn id(&self) -> &'static str {
        "objc"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".m", ".mm"]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_objc::LANGUAGE.into()
    }

    fn extract(
        &self,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "objc");
        let mut w = ObjcWalker {
            source,
            facts: &mut facts,
            scope: Vec::new(),
        };
        w.walk(tree.root_node());
        facts
    }
}

struct ObjcWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
}

impl<'a> ObjcWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }
    fn qn(&self, s: &str) -> String {
        if self.scope.is_empty() {
            s.to_string()
        } else {
            format!("{}::{s}", self.scope.join("::"))
        }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "class_implementation" | "class_interface" => {
                let name = node
                    .child(1) // class name is second child (after @implementation/@interface)
                    .filter(|c| c.kind() == "identifier")
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
            "method_definition" | "method_declaration" => {
                self.record_method(node);
                self.walk_children(node);
                return;
            }
            "function_definition" => {
                let name = node
                    .child_by_field_name("declarator")
                    .and_then(|d| d.child_by_field_name("declarator"))
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    let (sl, el) = (
                        (node.start_position().row as u32) + 1,
                        (node.end_position().row as u32) + 1,
                    );
                    self.facts.definitions.push(DefRecord {
                        simple_name: name.clone(),
                        qualified_name: name,
                        variant: DefVariant::FreeFunction,
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
                self.walk_children(node);
                return;
            }
            "message_expression" => {
                let method = node
                    .child_by_field_name("method")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                let recv = node
                    .child_by_field_name("receiver")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !method.is_empty() {
                    self.facts.references.push(RefRecord {
                        name: method,
                        receiver_hint: recv,
                        site_line: (node.start_position().row as u32) + 1,
                        site_byte: node.start_byte() as u32,
                        ..Default::default()
                    });
                }
                self.walk_children(node);
                return;
            }
            "preproc_include" | "preproc_import" => {
                let path_node = node.child_by_field_name("path");
                if let Some(p) = path_node {
                    let path = self.text(p).trim_matches('"').to_string();
                    if !path.is_empty() {
                        self.facts.imports.push(ImportRecord {
                            kind: "import".into(),
                            path,
                            alias: String::new(),
                            site_line: (node.start_position().row as u32) + 1,
                            site_byte: node.start_byte() as u32,
                        });
                    }
                }
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

    fn record_method(&mut self, node: Node) {
        // Method selector is the identifier child
        let name = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "identifier" || c.kind() == "keyword_selector")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if name.is_empty() {
            return;
        }
        // Check if class method (+) or instance method (-)
        let method_text = self.text(node);
        let is_class = method_text.trim_start().starts_with('+');
        let variant = if name == "init" {
            DefVariant::Constructor
        } else if is_class {
            DefVariant::StaticMethod
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_objc::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        ObjcPlugin.extract(
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.m"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn class_methods() {
        let src = "@implementation Service\n- (void)run { [self helper]; }\n+ (instancetype)create { return [[Service alloc] init]; }\n@end\n";
        let f = extract(src);
        assert!(
            f.definitions.iter().any(
                |d| d.simple_name == "run" && d.variant == DefVariant::InherentMethod
            )
        );
        assert!(
            f.definitions
                .iter()
                .any(|d| d.simple_name == "create"
                    && d.variant == DefVariant::StaticMethod)
        );
    }

    #[test]
    fn message_sends() {
        let src = "@implementation C\n- (void)f { [self run]; [Helper create]; }\n@end\n";
        let f = extract(src);
        assert!(
            f.references
                .iter()
                .any(|r| r.name == "run" && r.receiver_hint == "self")
        );
        assert!(
            f.references
                .iter()
                .any(|r| r.name == "create" && r.receiver_hint == "Helper")
        );
    }
}
