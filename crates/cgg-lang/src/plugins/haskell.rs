//! Haskell plugin — callable extraction.

use std::path::Path;
use cgg_core::{ids::FileId, DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord};
use tree_sitter::{Node, Tree};
use crate::LanguagePlugin;

#[derive(Debug)]
pub struct HaskellPlugin;

impl LanguagePlugin for HaskellPlugin {
    fn id(&self) -> &'static str { "haskell" }
    fn extensions(&self) -> &'static [&'static str] { &[".hs", ".lhs"] }
    fn shebangs(&self) -> &'static [&'static str] { &["runhaskell", "runghc"] }
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
            "import" => {
                self.extract_import(node);
                self.walk_children(node);
            }
            "function" | "bind" => {
                self.extract_function(node);
                self.walk_children(node);
            }
            "apply" => {
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
        // import [qualified] Module [as Alias] [(item, ...)]
        // tree-sitter-haskell layout:
        //   import     -> [import] [qualified?] [module] [as module]? [import_list]?
        let mut modules: Vec<String> = Vec::new();
        let mut alias = String::new();
        let mut qualified = false;
        let mut saw_as = false;
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32) {
                match child.kind() {
                    "qualified" => qualified = true,
                    "as" => saw_as = true,
                    "module" => {
                        let text = self.text(child).to_string();
                        if saw_as { alias = text; } else { modules.push(text); }
                    }
                    _ => {}
                }
            }
        }
        let module_name = modules.into_iter().next().unwrap_or_default();

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
            ..Default::default()
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
        assert!(plugin.extensions().contains(&".hs"));
        assert!(plugin.extensions().contains(&".lhs"));
    }

    #[test]
    fn free_functions_and_call() {
        // No `module ... where` header, so `HaskellWalker::module` stays
        // empty and qualified names are the bare simple names.
        let src = "foo :: Int -> Int\nfoo x = bar x\n\nbar :: Int -> Int\nbar x = x\n";
        let f = extract(src);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(names.contains(&"foo"), "got: {names:?}");
        assert!(names.contains(&"bar"), "got: {names:?}");
        let refs: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        assert!(refs.contains(&"bar"), "got: {refs:?}");
    }

    #[test]
    fn import_is_recorded() {
        let f = extract("import qualified Data.Map as M\n\nmain = pure ()\n");
        let paths: Vec<&str> = f.imports.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"Data.Map"), "got: {paths:?}");
    }
}
