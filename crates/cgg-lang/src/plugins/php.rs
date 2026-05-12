//! PHP plugin — callable extraction.

use std::path::Path;
use cgg_core::{ids::FileId, DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord};
use tree_sitter::{Node, Tree};
use crate::{LanguagePlugin, ResolverKind};

#[derive(Debug)]
pub struct PhpPlugin;

impl LanguagePlugin for PhpPlugin {
    fn id(&self) -> &'static str { "php" }
    fn extensions(&self) -> &'static [&'static str] { &[".php", ".phtml"] }
    fn shebangs(&self) -> &'static [&'static str] { &["php"] }
    fn resolver_kind(&self) -> ResolverKind { ResolverKind::Custom }
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_php::LANGUAGE_PHP.into() }

    fn extract(&self, file: FileId, path: &Path, tree: &Tree, source: &[u8]) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "php");
        let mut w = PhpWalker { source, facts: &mut facts, scope: Vec::new() };
        w.walk(tree.root_node());
        facts
    }
}

struct PhpWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
}

impl<'a> PhpWalker<'a> {
    fn text(&self, n: Node) -> &str { n.utf8_text(self.source).unwrap_or("") }
    fn qn(&self, simple: &str) -> String {
        if self.scope.is_empty() { simple.to_string() }
        else { format!("{}::{simple}", self.scope.join("::")) }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "class_declaration" => {
                let name = node.child_by_field_name("name")
                    .map(|n| self.text(n).to_string()).unwrap_or_default();
                if !name.is_empty() {
                    self.scope.push(name);
                    self.walk_children(node);
                    self.scope.pop();
                } else { self.walk_children(node); }
                return;
            }
            "function_definition" => {
                let name = node.child_by_field_name("name")
                    .map(|n| self.text(n).to_string()).unwrap_or_default();
                if !name.is_empty() {
                    self.record_def(&name, node, DefVariant::FreeFunction);
                }
                self.walk_children(node);
                return;
            }
            "method_declaration" => {
                let name = node.child_by_field_name("name")
                    .map(|n| self.text(n).to_string()).unwrap_or_default();
                if !name.is_empty() {
                    self.record_def(&name, node, DefVariant::InherentMethod);
                }
                self.walk_children(node);
                return;
            }
            "function_call_expression" | "member_call_expression" => {
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
        if c.goto_first_child() { loop { self.walk(c.node()); if !c.goto_next_sibling() { break; } } }
    }

    fn record_def(&mut self, name: &str, node: Node, variant: DefVariant) {
        let qn = self.qn(name);
        let (sl, el) = ((node.start_position().row as u32) + 1, (node.end_position().row as u32) + 1);
        self.facts.definitions.push(DefRecord {
            simple_name: name.to_string(), qualified_name: qn, variant,
            start_line: sl, end_line: el,
            start_byte: node.start_byte() as u32, end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(), attributes: Vec::new(),
        });
    }

    fn record_call(&mut self, node: Node) {
        let func = node.child_by_field_name("function")
            .map(|n| self.text(n).to_string()).unwrap_or_default();
        if func.is_empty() { return; }

        // require_once/include as import
        if func == "require_once" || func == "include" || func == "include_once" || func == "require" {
            let args = node.child_by_field_name("arguments");
            if let Some(a) = args.and_then(|a| a.child(0)) {
                let path = self.text(a).trim_matches('\'').trim_matches('"').to_string();
                if !path.is_empty() {
                    self.facts.imports.push(ImportRecord {
                        kind: func.clone(), path, alias: String::new(),
                        site_line: (node.start_position().row as u32) + 1,
                        site_byte: node.start_byte() as u32,
                    });
                    return;
                }
            }
        }

        let recv = node.child_by_field_name("object")
            .map(|n| self.text(n).to_string()).unwrap_or_default();
        self.facts.references.push(RefRecord {
            name: func, receiver_hint: recv,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
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
        p.set_language(&tree_sitter_php::LANGUAGE_PHP.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        PhpPlugin.extract(FileId::new(0), &PathBuf::from("/tmp/__cgg_test__/x.php"), &tree, src.as_bytes())
    }

    #[test]
    fn plugin_loads() {
        let plugin = PhpPlugin;
        assert_eq!(plugin.id(), "php");
        assert!(plugin.extensions().contains(&".php"));
        assert!(plugin.shebangs().contains(&"php"));
    }

    #[test]
    fn extracts_definitions() {
        let src = "<?php\nclass Service {\n  public function run() {}\n}\n";
        let f = extract(src);
        assert!(!f.definitions.is_empty(), "should extract definitions");
    }

    #[test]
    fn extracts_references() {
        let src = "<?php\nfunction main() { greet(); }\n";
        let f = extract(src);
        assert!(!f.references.is_empty(), "should extract references");
    }
}
