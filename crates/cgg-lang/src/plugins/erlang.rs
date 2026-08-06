//! Erlang plugin — callable extraction.

use std::path::Path;
use cgg_core::{ids::FileId, DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord};
use tree_sitter::{Node, Tree};
use crate::LanguagePlugin;

#[derive(Debug)]
pub struct ErlangPlugin;

impl LanguagePlugin for ErlangPlugin {
    fn id(&self) -> &'static str { "erlang" }
    fn extensions(&self) -> &'static [&'static str] { &[".erl", ".hrl"] }
    fn shebangs(&self) -> &'static [&'static str] { &["escript"] }
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_erlang::LANGUAGE.into() }

    fn extract(&self, file: FileId, path: &Path, tree: &Tree, source: &[u8]) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "erlang");
        let mut w = ErlangWalker { source, facts: &mut facts, module: String::new() };
        w.walk(tree.root_node());
        facts
    }
}

struct ErlangWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    module: String,
}

impl<'a> ErlangWalker<'a> {
    fn text(&self, n: Node) -> &str { n.utf8_text(self.source).unwrap_or("") }
    fn qn(&self, simple: &str) -> String {
        if self.module.is_empty() { simple.to_string() }
        else { format!("{}:{simple}", self.module) }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "module_attribute" => {
                self.extract_module(node);
                self.walk_children(node);
            }
            "import_attribute" => {
                self.record_import(node);
                self.walk_children(node);
            }
            "function_clause" | "function" => {
                self.record_function(node);
                self.walk_children(node);
            }
            "call" => {
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
        // -module(name).
        // Find the atom child
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32) {
                if child.kind() == "atom" {
                    self.module = self.text(child).to_string();
                    break;
                }
            }
        }
    }

    fn record_import(&mut self, node: Node) {
        // -import(module, [func/arity, ...]).
        let mut module_name = String::new();
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32) {
                if child.kind() == "atom" && module_name.is_empty() {
                    module_name = self.text(child).to_string();
                    break;
                }
            }
        }
        if !module_name.is_empty() {
            self.facts.imports.push(ImportRecord {
                kind: "import".to_string(),
                path: module_name,
                alias: String::new(),
                site_line: (node.start_position().row as u32) + 1,
                site_byte: node.start_byte() as u32,
            });
        }
    }

    fn record_function(&mut self, node: Node) {
        // function_clause: atom ( ... ) -> ... ;
        // Extract the atom (function name)
        if let Some(name_node) = node.child(0) {
            if name_node.kind() == "atom" {
                let name = self.text(name_node).to_string();
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
        }
    }

    fn record_call(&mut self, node: Node) {
        // call: atom ( ... ) or module:function ( ... )
        // First child is the function/module reference
        if let Some(func_node) = node.child(0) {
            let name = self.text(func_node).to_string();
            if name.is_empty() { return; }
            
            let receiver_hint = if name.contains(':') {
                let parts: Vec<&str> = name.split(':').collect();
                if parts.len() == 2 {
                    parts[0].to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            
            self.facts.references.push(RefRecord {
                name,
                receiver_hint,
                site_line: (node.start_position().row as u32) + 1,
                site_byte: node.start_byte() as u32,
                ..Default::default()
            });
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
        p.set_language(&tree_sitter_erlang::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        ErlangPlugin.extract(FileId::new(0), &PathBuf::from("/tmp/__cgg_test__/x.erl"), &tree, src.as_bytes())
    }

    #[test]
    fn module_and_functions() {
        let src = "-module(mymod).\n\ngreet(Name) ->\n    io:format(\"Hello, ~s~n\", [Name]).\n";
        let f = extract(src);
        assert!(!f.definitions.is_empty(), "Expected definitions, got none");
        assert_eq!(f.definitions[0].simple_name, "greet");
    }
}
