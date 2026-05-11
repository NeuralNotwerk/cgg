//! Scala plugin — callable extraction.

use std::path::Path;
use cgg_core::{ids::FileId, DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord};
use tree_sitter::{Node, Tree};
use crate::{LanguagePlugin, ResolverKind};

#[derive(Debug)]
pub struct ScalaPlugin;

impl LanguagePlugin for ScalaPlugin {
    fn id(&self) -> &'static str { "scala" }
    fn extensions(&self) -> &'static [&'static str] { &[".scala", ".sc"] }
    fn shebangs(&self) -> &'static [&'static str] { &["scala"] }
    fn resolver_kind(&self) -> ResolverKind { ResolverKind::Custom }
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_scala::LANGUAGE.into() }

    fn extract(&self, file: FileId, path: &Path, tree: &Tree, source: &[u8]) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "scala");
        let mut w = ScalaWalker { source, facts: &mut facts, scope: Vec::new() };
        w.walk(tree.root_node());
        facts
    }
}

struct ScalaWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
}

impl<'a> ScalaWalker<'a> {
    fn text(&self, n: Node) -> &str { n.utf8_text(self.source).unwrap_or("") }
    fn qn(&self, simple: &str) -> String {
        if self.scope.is_empty() { simple.to_string() }
        else { format!("{}::{simple}", self.scope.join("::")) }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "class_definition" | "object_definition" => {
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
            "import_declaration" => {
                self.record_import(node);
                self.walk_children(node);
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
        if c.goto_first_child() { loop { self.walk(c.node()); if !c.goto_next_sibling() { break; } } }
    }

    fn record_def(&mut self, name: &str, node: Node, variant: DefVariant) {
        let qn = self.qn(name);
        let (sl, el) = ((node.start_position().row as u32) + 1, (node.end_position().row as u32) + 1);
        self.facts.definitions.push(DefRecord {
            simple_name: name.to_string(), qualified_name: qn, variant,
            start_line: sl, end_line: el,
            start_byte: node.start_byte() as u32, end_byte: node.end_byte() as u32,
            signature_hint: self.text(node).lines().next().unwrap_or("").trim().to_string(),
            visibility: String::new(), attributes: Vec::new(),
        });
    }

    fn record_import(&mut self, node: Node) {
        let path = node.child_by_field_name("path")
            .map(|n| self.text(n).to_string()).unwrap_or_default();
        if !path.is_empty() {
            self.facts.imports.push(ImportRecord {
                kind: "import".into(), path, alias: String::new(),
                site_line: (node.start_position().row as u32) + 1,
                site_byte: node.start_byte() as u32,
            });
        }
    }

    fn record_call(&mut self, node: Node) {
        let func = node.child_by_field_name("function")
            .map(|n| self.text(n).to_string()).unwrap_or_default();
        if func.is_empty() { return; }

        self.facts.references.push(RefRecord {
            name: func, receiver_hint: String::new(),
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
        p.set_language(&tree_sitter_scala::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        ScalaPlugin.extract(FileId::new(0), &PathBuf::from("/tmp/__cgg_test__/x.scala"), &tree, src.as_bytes())
    }

    #[test]
    fn plugin_loads() {
        let plugin = ScalaPlugin;
        assert_eq!(plugin.id(), "scala");
        assert!(plugin.extensions().contains(&".scala"));
        assert!(plugin.shebangs().contains(&"scala"));
    }

    #[test]
    fn extracts_definitions() {
        let src = "class Service { def run() {} }\nobject Main { def main() {} }\n";
        let f = extract(src);
        assert!(!f.definitions.is_empty(), "should extract definitions");
    }

    #[test]
    fn extracts_references() {
        let src = "def main() { greet() }\n";
        let f = extract(src);
        assert!(!f.references.is_empty(), "should extract references");
    }
}
