//! Haskell plugin — callable extraction.

use std::path::Path;
use cgg_core::{ids::FileId, DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord};
use tree_sitter::{Node, Tree};
use crate::{LanguagePlugin, ResolverKind};

#[derive(Debug)]
pub struct HaskellPlugin;

impl LanguagePlugin for HaskellPlugin {
    fn id(&self) -> &'static str { "haskell" }
    fn extensions(&self) -> &'static [&'static str] { &["hs", "lhs"] }
    fn shebangs(&self) -> &'static [&'static str] { &["runhaskell", "runghc"] }
    fn resolver_kind(&self) -> ResolverKind { ResolverKind::Custom }
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_haskell::LANGUAGE.into() }

    fn extract(&self, file: FileId, path: &Path, tree: &Tree, source: &[u8]) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "haskell");
        let mut w = HaskellWalker { source, facts: &mut facts, module: String::new() };
        w.walk(tree.root_node());
        facts
    }
}

struct HaskellWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    module: String,
}

impl<'a> HaskellWalker<'a> {
    fn text(&self, n: Node) -> &str { n.utf8_text(self.source).unwrap_or("") }
    fn qn(&self, simple: &str) -> String {
        if self.module.is_empty() { simple.to_string() }
        else { format!("{}::{simple}", self.module) }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "module" => {
                self.extract_module(node);
                self.walk_children(node);
            }
            "import_declaration" => {
                self.extract_import(node);
                self.walk_children(node);
            }
            "function" | "bind" => {
                self.extract_function(node);
                self.walk_children(node);
            }
            "exp_apply" => {
                self.record_call(node);
                self.walk_children(node);
            }
            _ => self.walk_children(node),
        }
    }

    fn walk_children(&mut self, node: Node) {
        let mut c = node.walk();
        if c.goto_first_child() { loop { self.walk(c.node()); if !c.goto_next_sibling() { break; } } }
    }

    fn extract_module(&mut self, node: Node) {
        // module Name where
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32) {
                if child.kind() == "module_name" {
                    self.module = self.text(child).to_string();
                    break;
                }
            }
        }
    }

    fn extract_import(&mut self, node: Node) {
        // import Module or import qualified Module as M
        let mut module_name = String::new();
        let mut alias = String::new();
        let mut qualified = false;

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32) {
                match child.kind() {
                    "qualified" => qualified = true,
                    "module_name" => module_name = self.text(child).to_string(),
                    "module_alias" => alias = self.text(child).to_string(),
                    _ => {}
                }
            }
        }

        if !module_name.is_empty() {
            self.facts.imports.push(ImportRecord {
                kind: if qualified { "import qualified" } else { "import" }.to_string(),
                path: module_name,
                alias,
                site_line: (node.start_position().row as u32) + 1,
                site_byte: node.start_byte() as u32,
            });
        }
    }

    fn extract_function(&mut self, node: Node) {
        // function: name = expr or bind: pattern = expr
        let mut name = String::new();
        
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32) {
                if child.kind() == "variable" {
                    name = self.text(child).to_string();
                    break;
                }
            }
        }

        if name.is_empty() { return; }

        let qn = self.qn(&name);
        let (sl, el) = ((node.start_position().row as u32) + 1, (node.end_position().row as u32) + 1);
        self.facts.definitions.push(DefRecord {
            simple_name: name,
            qualified_name: qn,
            variant: DefVariant::FreeFunction,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            attributes: Vec::new(),
        });
    }

    fn record_call(&mut self, node: Node) {
        // exp_apply: function applied to arguments
        if let Some(func_node) = node.child(0) {
            if func_node.kind() == "variable" {
                let name = self.text(func_node).to_string();
                if !name.is_empty() {
                    self.facts.references.push(RefRecord {
                        name,
                        receiver_hint: String::new(),
                        site_line: (node.start_position().row as u32) + 1,
                        site_byte: node.start_byte() as u32,
                    });
                }
            }
        }
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
        p.set_language(&tree_sitter_haskell::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        HaskellPlugin.extract(FileId::new(0), &PathBuf::from("/tmp/__cgg_test__/x.hs"), &tree, src.as_bytes())
    }

    #[test]
    fn plugin_loads() {
        let plugin = HaskellPlugin;
        assert_eq!(plugin.id(), "haskell");
        assert!(plugin.extensions().contains(&"hs"));
        assert!(plugin.extensions().contains(&"lhs"));
    }
}
